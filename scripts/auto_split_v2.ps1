<#
.SYNOPSIS
    Splits Rust modules into subfiles at top-level definition boundaries.
    Uses definition START detection only (no brace counting needed).
    Each chunk runs from one definition start to the next chunk boundary.
#>

param(
    [string]$BasePath = "C:\Users\uniqu\Documents\workspace\Psmux-Modular\src",
    [int]$MaxChunkLines = 370,
    [switch]$DryRun
)

function Find-DefinitionStarts {
    param([string[]]$Lines)

    $starts = @()
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $trimmed = $Lines[$i].TrimStart()
        # Match top-level definitions (not indented, or at most 0-1 spaces)
        $indent = $Lines[$i].Length - $Lines[$i].TrimStart().Length
        if ($indent -gt 0) { continue }  # Skip anything indented (inside a block)

        if ($trimmed -match "^(pub(\(.*?\))?\s+)?(unsafe\s+)?(async\s+)?(fn|struct|enum|impl|const|static|type|trait)\b" -or
            $trimmed -match "^(pub\s+)?macro_rules!" -or
            $trimmed -match "^#\[cfg\(test\)\]") {

            # Look backwards for doc comments and attributes
            $start = $i
            while ($start -gt 0) {
                $prev = $Lines[$start - 1].TrimStart()
                if ($prev -match "^///" -or $prev -match "^//!" -or $prev -match "^#\[" -or $prev -eq "") {
                    $start--
                } else {
                    break
                }
            }
            # Skip leading blank lines
            while ($start -lt $i -and $Lines[$start].Trim() -eq "") {
                $start++
            }
            # Don't add duplicate starts
            if ($starts.Count -eq 0 -or $starts[-1] -ne $start) {
                $starts += $start
            }
        }
    }
    return $starts
}

function Split-AtBoundaries {
    param(
        [int[]]$Starts,
        [int]$TotalLines,
        [int]$MaxLines
    )

    if ($Starts.Count -eq 0) { return @() }

    $chunks = @()
    $chunkStartIdx = 0  # Index into $Starts array

    for ($i = 1; $i -lt $Starts.Count; $i++) {
        $chunkSize = $Starts[$i] - $Starts[$chunkStartIdx]
        if ($chunkSize -gt $MaxLines) {
            # End current chunk before this definition
            $cs = $Starts[$chunkStartIdx]
            $ce = $Starts[$i] - 1
            # Trim trailing blank lines
            # Don't trim, just leave as-is
            $chunks += [PSCustomObject]@{
                StartLine = $cs
                EndLine = $ce
                Size = $ce - $cs + 1
            }
            $chunkStartIdx = $i
        }
    }

    # Last chunk goes to EOF
    $cs = $Starts[$chunkStartIdx]
    $ce = $TotalLines - 1
    $chunks += [PSCustomObject]@{
        StartLine = $cs
        EndLine = $ce
        Size = $ce - $cs + 1
    }

    return $chunks
}

function Get-ChunkNameFromLines {
    param([string[]]$Lines, [int]$StartLine)

    # Scan forward from StartLine to find the first definition keyword
    for ($i = $StartLine; $i -lt [Math]::Min($StartLine + 10, $Lines.Count); $i++) {
        $line = $Lines[$i].TrimStart()
        if ($line -match "(pub(\(.*?\))?\s+)?(unsafe\s+)?(async\s+)?fn\s+(\w+)") {
            return $Matches[5]
        }
        if ($line -match "(pub(\(.*?\))?\s+)?struct\s+(\w+)") {
            return $Matches[3].ToLower()
        }
        if ($line -match "(pub(\(.*?\))?\s+)?enum\s+(\w+)") {
            return $Matches[3].ToLower()
        }
        if ($line -match "(pub(\(.*?\))?\s+)?impl(<.*?>)?\s+(\w+)") {
            return "impl_$($Matches[4].ToLower())"
        }
        if ($line -match "(pub(\(.*?\))?\s+)?trait\s+(\w+)") {
            return $Matches[3].ToLower()
        }
        if ($line -match "(pub(\(.*?\))?\s+)?const\s+(\w+)") {
            return $Matches[3].ToLower()
        }
        if ($line -match "(pub(\(.*?\))?\s+)?type\s+(\w+)") {
            return $Matches[3].ToLower()
        }
        if ($line -match "#\[cfg\(test\)\]") {
            return "tests"
        }
    }
    return $null
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

    if ($totalLines -le ($MaxLines + 30)) {
        Write-Host "  SKIP: $Module ($totalLines lines, under limit)" -ForegroundColor DarkGray
        return
    }

    Write-Host "`n=== $Module ($totalLines lines) ===" -ForegroundColor Cyan

    # Find all top-level definition start lines
    $defStarts = @(Find-DefinitionStarts -Lines $allLines)
    Write-Host "  Found $($defStarts.Count) definition starts"

    if ($defStarts.Count -eq 0) {
        Write-Host "  WARN: No definitions found!" -ForegroundColor Red
        return
    }

    # Import section: everything before first definition
    $importEnd = $defStarts[0]
    $importLines = @()
    if ($importEnd -gt 0) {
        $importLines = $allLines[0..($importEnd - 1)]
        # Trim trailing blank lines
        while ($importLines.Count -gt 0 -and $importLines[-1].Trim() -eq "") {
            $importLines = $importLines[0..($importLines.Count - 2)]
        }
    }
    Write-Host "  Import section: $($importLines.Count) lines"

    # Split into chunks at definition boundaries
    $chunks = @(Split-AtBoundaries -Starts $defStarts -TotalLines $totalLines -MaxLines $MaxLines)
    Write-Host "  Split into $($chunks.Count) chunks"

    if ($chunks.Count -le 1) {
        Write-Host "  WARN: Could not split (single chunk). Trying smaller max..." -ForegroundColor Yellow
        $chunks = @(Split-AtBoundaries -Starts $defStarts -TotalLines $totalLines -MaxLines ([Math]::Max(100, [int]($MaxLines / 2))))
        Write-Host "  Retry: $($chunks.Count) chunks"
        if ($chunks.Count -le 1) {
            Write-Host "  SKIP: Module has too few split points" -ForegroundColor Red
            return
        }
    }

    # Name chunks
    $usedNames = @{}
    $namedChunks = @()
    $chunkIdx = 0

    foreach ($chunk in $chunks) {
        $name = Get-ChunkNameFromLines -Lines $allLines -StartLine $chunk.StartLine
        if (-not $name) { $name = "part" }
        $name = $name -replace '[^a-zA-Z0-9_]', '_'
        $name = $name.ToLower()

        if ($usedNames.ContainsKey($name)) {
            $usedNames[$name]++
            $name = "${name}_$($usedNames[$name])"
        } else {
            $usedNames[$name] = 1
        }

        $namedChunks += [PSCustomObject]@{
            Name = $name
            StartLine = $chunk.StartLine
            EndLine = $chunk.EndLine
            Size = $chunk.Size
        }

        $sizeColor = if ($chunk.Size -gt 400) { "Red" } elseif ($chunk.Size -gt $MaxLines) { "Yellow" } else { "White" }
        Write-Host ("  [{0}] {1,-25} lines {2,5}-{3,5} ({4,4} lines)" -f $chunkIdx, $name, ($chunk.StartLine+1), ($chunk.EndLine+1), $chunk.Size) -ForegroundColor $sizeColor
        $chunkIdx++
    }

    if ($DryRun) {
        $oversized = ($namedChunks | Where-Object { $_.Size -gt 400 }).Count
        Write-Host "  [DRY RUN] $($namedChunks.Count) files, $oversized oversized (>400)" -ForegroundColor Yellow
        return
    }

    # Check for existing files
    foreach ($nc in $namedChunks) {
        $subFile = Join-Path $modDir "$($nc.Name).rs"
        if (Test-Path $subFile) {
            Write-Host "  ABORT: $($nc.Name).rs already exists" -ForegroundColor Red
            return
        }
    }

    # Create submodule files
    foreach ($nc in $namedChunks) {
        $codeLines = $allLines[$nc.StartLine..$nc.EndLine]

        $content = @()
        $content += "#![allow(unused_imports)]"
        $content += "use super::*;"
        $content += ""

        # Add original imports (skip mod declarations and inner attributes)
        foreach ($line in $importLines) {
            $trimmed = $line.TrimStart()
            if ($trimmed -match "^mod\s+" -or $trimmed -match "^pub(\(.*?\))?\s+mod\s+" -or $trimmed -match "^#!\[") {
                continue
            }
            $content += $line
        }
        $content += ""
        $content += $codeLines

        $subFile = Join-Path $modDir "$($nc.Name).rs"
        $content | Set-Content $subFile -Encoding UTF8
        Write-Host "  Created $($nc.Name).rs ($($content.Count) lines)" -ForegroundColor Green
    }

    # Rewrite mod.rs
    $modContent = @()
    $modContent += "#![allow(unused_imports)]"
    $modContent += ""

    # Preserve ALL import lines (including existing mod declarations for server)
    foreach ($line in $importLines) {
        $trimmed = $line.TrimStart()
        if ($trimmed -match "^#!\[") { continue }  # Skip existing #![allow]
        $modContent += $line
    }
    $modContent += ""

    # Add new mod declarations
    foreach ($nc in $namedChunks) {
        $modContent += "mod $($nc.Name);"
    }
    $modContent += ""

    # Add pub use re-exports
    foreach ($nc in $namedChunks) {
        $modContent += "pub use $($nc.Name)::*;"
    }
    $modContent += ""

    $modContent | Set-Content $modFile -Encoding UTF8
    Write-Host "  Rewrote mod.rs ($($modContent.Count) lines)" -ForegroundColor Green
}

# ============================================================
# MAIN
# ============================================================

$allModules = @(
    "types", "cli", "client", "commands", "config", "control",
    "copy_mode", "format", "help", "input", "layout", "pane",
    "platform", "popup", "rendering", "session", "ssh_input",
    "style", "tree", "util", "window_ops", "server"
)

$mode = if ($DryRun) { "DRY RUN" } else { "EXECUTE" }
Write-Host "=== Phase 2: Module Splitting ($mode) ===" -ForegroundColor Magenta
Write-Host "Max lines per chunk: $MaxChunkLines"
Write-Host ""

foreach ($mod in $allModules) {
    Split-RustModule -Module $mod -MaxLines $MaxChunkLines -DryRun:$DryRun
}

# Check for server/connection.rs
$connFile = Join-Path $BasePath "server\connection.rs"
if (Test-Path $connFile) {
    $cl = (Get-Content $connFile).Count
    if ($cl -gt $MaxChunkLines) {
        Write-Host "`n=== server/connection.rs ($cl lines) needs conversion to folder ===" -ForegroundColor Yellow
    }
}

Write-Host "`n=== Done ===" -ForegroundColor Green
