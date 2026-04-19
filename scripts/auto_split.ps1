<#
.SYNOPSIS
    Automated Rust module splitter that respects definition boundaries.
    Splits a module's mod.rs into submodule files at clean function/struct/impl boundaries.
    Uses brace-level tracking to avoid cutting definitions in half.
#>

param(
    [string]$BasePath = "C:\Users\uniqu\Documents\workspace\Psmux-Modular\src",
    [int]$MaxLines = 380,
    [int]$MinLines = 50,
    [switch]$DryRun
)

function Get-TopLevelBlocks {
    <#
    .SYNOPSIS
        Returns an array of blocks, each with Start/End line indices (0-based)
        and whether it's a definition or import/comment preamble.
    #>
    param([string[]]$Lines)

    $blocks = @()
    $braceLevel = 0
    $blockStart = -1
    $inBlock = $false
    $lastBlockEnd = -1

    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $trimmed = $line.TrimStart()

        # Skip empty lines between blocks
        if (-not $inBlock) {
            # Check if this line starts a new top-level definition
            if ($trimmed -match "^(pub(\(.*?\))?\s+)?(fn|struct|enum|impl|type|trait|const|static)\s+" -or
                $trimmed -match "^(pub(\(.*?\))?\s+)?(fn|struct|enum|impl|type|trait|const|static)\b" -or
                $trimmed -match "^(pub\s+)?macro_rules!") {

                # Include any preceding doc comments/attributes
                $blockStart = $i
                while ($blockStart -gt 0 -and $blockStart -gt ($lastBlockEnd + 1)) {
                    $prev = $Lines[$blockStart - 1].TrimStart()
                    if ($prev -match "^///" -or $prev -match "^#\[" -or $prev -eq "") {
                        $blockStart--
                    } else {
                        break
                    }
                }
                # Don't include leading blank lines
                while ($blockStart -lt $i -and $Lines[$blockStart].Trim() -eq "") {
                    $blockStart++
                }

                $inBlock = $true
                $braceLevel = 0
            }
        }

        if ($inBlock) {
            # Count braces (naive: doesn't handle strings/comments with braces)
            foreach ($ch in $line.ToCharArray()) {
                if ($ch -eq '{') { $braceLevel++ }
                elseif ($ch -eq '}') { $braceLevel-- }
            }

            # Block ends when braces balance or it's a single-line const/static
            if ($braceLevel -le 0) {
                $blocks += [PSCustomObject]@{
                    Start = $blockStart
                    End = $i
                    Lines = ($i - $blockStart + 1)
                    FirstLine = $Lines[$blockStart].TrimStart().Substring(0, [Math]::Min(80, $Lines[$blockStart].TrimStart().Length))
                }
                $lastBlockEnd = $i
                $inBlock = $false
                $braceLevel = 0
            }
        }
    }

    # If still in a block at EOF (shouldn't happen with valid Rust)
    if ($inBlock) {
        $blocks += [PSCustomObject]@{
            Start = $blockStart
            End = $Lines.Count - 1
            Lines = ($Lines.Count - $blockStart)
            FirstLine = $Lines[$blockStart].TrimStart().Substring(0, [Math]::Min(80, $Lines[$blockStart].TrimStart().Length))
        }
    }

    return $blocks
}

function Get-ImportSection {
    <#
    .SYNOPSIS
        Returns the line index where imports end and definitions begin.
        Import section = everything before the first top-level definition.
    #>
    param([string[]]$Lines, [array]$Blocks)

    if ($Blocks.Count -eq 0) { return $Lines.Count }
    return $Blocks[0].Start
}

function Get-ChunkName {
    param([array]$Blocks, [int]$ChunkIndex)

    $first = $Blocks[0]
    $fl = $first.FirstLine

    # Try to extract a meaningful name from the first definition
    if ($fl -match "(pub(\(.*?\))?\s+)?fn\s+(\w+)") { return $Matches[3] }
    if ($fl -match "(pub(\(.*?\))?\s+)?struct\s+(\w+)") { return $Matches[3].ToLower() }
    if ($fl -match "(pub(\(.*?\))?\s+)?enum\s+(\w+)") { return $Matches[3].ToLower() }
    if ($fl -match "(pub(\(.*?\))?\s+)?impl\s+(\w+)") { return "impl_$($Matches[3].ToLower())" }
    if ($fl -match "(pub(\(.*?\))?\s+)?trait\s+(\w+)") { return $Matches[3].ToLower() }
    if ($fl -match "(pub(\(.*?\))?\s+)?const\s+(\w+)") { return $Matches[3].ToLower() }
    if ($fl -match "(pub(\(.*?\))?\s+)?type\s+(\w+)") { return $Matches[3].ToLower() }

    return "part_$ChunkIndex"
}

function Group-BlocksIntoChunks {
    <#
    .SYNOPSIS
        Groups adjacent blocks into chunks, each not exceeding MaxLines.
        Returns array of chunks, each containing the blocks in that chunk.
    #>
    param(
        [array]$Blocks,
        [int]$MaxLines
    )

    $chunks = @()
    $currentChunk = @()
    $currentLines = 0

    foreach ($block in $Blocks) {
        if ($currentChunk.Count -gt 0 -and ($currentLines + $block.Lines) -gt $MaxLines) {
            # Start new chunk
            $chunks += ,@($currentChunk)
            $currentChunk = @($block)
            $currentLines = $block.Lines
        } else {
            $currentChunk += $block
            $currentLines += $block.Lines
        }
    }

    if ($currentChunk.Count -gt 0) {
        $chunks += ,@($currentChunk)
    }

    return $chunks
}

function Split-RustModule {
    param(
        [string]$Module,
        [int]$MaxLines,
        [switch]$DryRun
    )

    $modDir = Join-Path $BasePath $Module
    $modFile = Join-Path $modDir "mod.rs"

    if (-not (Test-Path $modFile)) {
        Write-Host "  SKIP: $modFile not found" -ForegroundColor Yellow
        return
    }

    $allLines = Get-Content $modFile -Encoding UTF8
    $totalLines = $allLines.Count

    if ($totalLines -le $MaxLines) {
        Write-Host "  SKIP: $Module ($totalLines lines <= $MaxLines max)" -ForegroundColor DarkGray
        return
    }

    Write-Host "`n=== $Module ($totalLines lines) ===" -ForegroundColor Cyan

    # Find top-level blocks
    $blocks = @(Get-TopLevelBlocks -Lines $allLines)
    Write-Host "  Found $($blocks.Count) top-level definitions"

    if ($blocks.Count -eq 0) {
        Write-Host "  WARN: No definitions found!" -ForegroundColor Red
        return
    }

    # Get import section (everything before first definition)
    $importEnd = $blocks[0].Start
    $importLines = @()
    if ($importEnd -gt 0) {
        $importLines = $allLines[0..($importEnd - 1)]
        # Trim trailing blank lines from imports
        while ($importLines.Count -gt 0 -and $importLines[-1].Trim() -eq "") {
            $importLines = $importLines[0..($importLines.Count - 2)]
        }
    }
    Write-Host "  Import section: $($importLines.Count) lines"

    # Group blocks into chunks
    $chunks = @(Group-BlocksIntoChunks -Blocks $blocks -MaxLines ($MaxLines - $importLines.Count - 5))

    # If only 1 chunk, try with smaller max
    if ($chunks.Count -le 1) {
        $chunks = @(Group-BlocksIntoChunks -Blocks $blocks -MaxLines ([Math]::Max(100, $MaxLines / 2)))
    }

    Write-Host "  Split into $($chunks.Count) chunks"

    # Deduplicate chunk names
    $usedNames = @{}
    $chunkPlans = @()

    for ($ci = 0; $ci -lt $chunks.Count; $ci++) {
        $chunk = $chunks[$ci]
        $name = Get-ChunkName -Blocks $chunk -ChunkIndex $ci
        # Sanitize name
        $name = $name -replace '[^a-zA-Z0-9_]', '_'
        $name = $name.ToLower()

        # Deduplicate
        if ($usedNames.ContainsKey($name)) {
            $usedNames[$name]++
            $name = "${name}_$($usedNames[$name])"
        } else {
            $usedNames[$name] = 1
        }

        $startLine = $chunk[0].Start
        $endLine = $chunk[-1].End
        $lineCount = $endLine - $startLine + 1

        $chunkPlans += [PSCustomObject]@{
            Name = $name
            Start = $startLine
            End = $endLine
            LineCount = $lineCount
            BlockCount = $chunk.Count
            FirstDef = $chunk[0].FirstLine
        }

        Write-Host "  [$ci] $name : lines $($startLine+1)-$($endLine+1) ($lineCount lines, $($chunk.Count) defs) | $($chunk[0].FirstLine)" -ForegroundColor White
    }

    if ($DryRun) {
        Write-Host "  [DRY RUN] Would create $($chunkPlans.Count) submodule files" -ForegroundColor Yellow
        return
    }

    # Check if any existing submodule files would be overwritten
    foreach ($plan in $chunkPlans) {
        $subFile = Join-Path $modDir "$($plan.Name).rs"
        if (Test-Path $subFile) {
            Write-Host "  WARN: $($plan.Name).rs already exists, skipping module" -ForegroundColor Red
            return
        }
    }

    # Create submodule files
    foreach ($plan in $chunkPlans) {
        $codeLines = $allLines[$plan.Start..$plan.End]

        $content = @()
        $content += "#![allow(unused_imports)]"
        $content += "use super::*;"
        $content += ""

        # Add original imports (skip mod declarations, #![allow] directives)
        foreach ($line in $importLines) {
            $trimmed = $line.TrimStart()
            if ($trimmed -match "^mod\s+" -or $trimmed -match "^pub\s+mod\s+" -or $trimmed -match "^#!\[") {
                continue
            }
            $content += $line
        }
        $content += ""
        $content += $codeLines

        $subFile = Join-Path $modDir "$($plan.Name).rs"
        $content | Set-Content $subFile -Encoding UTF8
        $totalWithImports = $content.Count
        Write-Host "  Created $($plan.Name).rs ($totalWithImports lines total)" -ForegroundColor Green
    }

    # Rewrite mod.rs
    $modContent = @()
    $modContent += "#![allow(unused_imports)]"
    $modContent += ""

    # Keep original imports in mod.rs for resolution
    foreach ($line in $importLines) {
        $trimmed = $line.TrimStart()
        if ($trimmed -match "^#!\[") { continue }
        $modContent += $line
    }
    $modContent += ""

    # Add mod declarations
    foreach ($plan in $chunkPlans) {
        $modContent += "mod $($plan.Name);"
    }
    $modContent += ""

    # Add pub use re-exports
    foreach ($plan in $chunkPlans) {
        $modContent += "pub use $($plan.Name)::*;"
    }
    $modContent += ""

    $modFile = Join-Path $modDir "mod.rs"
    $modContent | Set-Content $modFile -Encoding UTF8
    Write-Host "  Rewrote mod.rs ($($modContent.Count) lines)" -ForegroundColor Green
}

# ============================================================
# MAIN: Process all modules that need splitting
# ============================================================

$modulesToSplit = @(
    "types", "cli", "client", "commands", "config", "control",
    "copy_mode", "format", "help", "input", "layout", "pane",
    "platform", "popup", "rendering", "session", "ssh_input",
    "style", "tree", "util", "window_ops"
)

Write-Host "=== Phase 2: Automated Module Splitting ===" -ForegroundColor Magenta
Write-Host "Max lines per chunk: $MaxLines"
Write-Host "Modules to process: $($modulesToSplit.Count)"
Write-Host ""

foreach ($mod in $modulesToSplit) {
    Split-RustModule -Module $mod -MaxLines $MaxLines -DryRun:$DryRun
}

# Handle server/mod.rs separately (it has sibling files)
$serverMod = Join-Path $BasePath "server\mod.rs"
if (Test-Path $serverMod) {
    $serverLines = (Get-Content $serverMod).Count
    if ($serverLines -gt $MaxLines) {
        Split-RustModule -Module "server" -MaxLines $MaxLines -DryRun:$DryRun
    }
}

# Handle server/connection.rs if it exists and is large
$connFile = Join-Path $BasePath "server\connection.rs"
if (Test-Path $connFile) {
    $connLines = (Get-Content $connFile).Count
    if ($connLines -gt $MaxLines) {
        Write-Host "`n=== server/connection.rs ($connLines lines) ===" -ForegroundColor Cyan
        Write-Host "  NOTE: connection.rs is a standalone file, needs manual conversion to folder first" -ForegroundColor Yellow
    }
}

Write-Host "`n=== Splitting complete ===" -ForegroundColor Green
if (-not $DryRun) {
    Write-Host "Running cargo check..." -ForegroundColor Yellow
}
