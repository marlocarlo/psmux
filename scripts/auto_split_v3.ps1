<#
.SYNOPSIS
    v3: Splits Rust modules using brace-depth tracking for accurate definition detection.
    Only detects definitions at brace depth 0 (module level).
    Splits at those boundaries with no brace-counting needed for block ends.
#>

param(
    [string]$BasePath = "C:\Users\uniqu\Documents\workspace\Psmux-Modular\src",
    [int]$MaxChunkLines = 370,
    [switch]$DryRun
)

function Find-ModuleLevelStarts {
    <#
    .SYNOPSIS
        Find all module-level definition start lines using brace-depth tracking.
        A definition is at module level if brace depth is 0 when it starts.
    #>
    param([string[]]$Lines)

    $starts = @()
    $braceDepth = 0

    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $trimmed = $line.TrimStart()

        # At depth 0, check for module-level definitions
        if ($braceDepth -eq 0) {
            $isDef = $false

            if ($trimmed -match "^(pub(\(.*?\))?\s+)?(unsafe\s+)?(async\s+)?(fn|struct|enum|type|trait|const|static)\b") {
                $isDef = $true
            }
            elseif ($trimmed -match "^impl\b") {
                $isDef = $true
            }
            elseif ($trimmed -match "^(pub\s+)?macro_rules!") {
                $isDef = $true
            }
            elseif ($trimmed -match "^#\[cfg\(test\)\]") {
                $isDef = $true
            }

            if ($isDef) {
                # Look backwards for doc comments and attributes
                $start = $i
                while ($start -gt 0) {
                    $prev = $Lines[$start - 1].TrimStart()
                    if ($prev -match "^///" -or $prev -match "^//!" -or
                        $prev -match "^#\[" -or $prev -eq "") {
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

        # Update brace depth for this line
        foreach ($ch in $line.ToCharArray()) {
            if ($ch -eq '{') { $braceDepth++ }
            elseif ($ch -eq '}') { $braceDepth-- }
        }
        if ($braceDepth -lt 0) { $braceDepth = 0 }  # Safety clamp
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
    $chunkStartIdx = 0

    for ($i = 1; $i -lt $Starts.Count; $i++) {
        $chunkSize = $Starts[$i] - $Starts[$chunkStartIdx]
        if ($chunkSize -gt $MaxLines) {
            $cs = $Starts[$chunkStartIdx]
            $ce = $Starts[$i] - 1
            $chunks += [PSCustomObject]@{ StartLine=$cs; EndLine=$ce; Size=($ce - $cs + 1) }
            $chunkStartIdx = $i
        }
    }

    # Last chunk to EOF
    $cs = $Starts[$chunkStartIdx]
    $ce = $TotalLines - 1
    $chunks += [PSCustomObject]@{ StartLine=$cs; EndLine=$ce; Size=($ce - $cs + 1) }

    return $chunks
}

function Get-NameFromLine {
    param([string[]]$Lines, [int]$Start)
    for ($i = $Start; $i -lt [Math]::Min($Start + 15, $Lines.Count); $i++) {
        $line = $Lines[$i].TrimStart()
        if ($line -match "(pub(\(.*?\))?\s+)?(unsafe\s+)?(async\s+)?fn\s+(\w+)") { return $Matches[5] }
        if ($line -match "(pub(\(.*?\))?\s+)?struct\s+(\w+)") { return $Matches[3].ToLower() }
        if ($line -match "(pub(\(.*?\))?\s+)?enum\s+(\w+)") { return $Matches[3].ToLower() }
        if ($line -match "^impl(<.*?>)?\s+(\w+)") { return "impl_$($Matches[2].ToLower())" }
        if ($line -match "(pub(\(.*?\))?\s+)?trait\s+(\w+)") { return $Matches[3].ToLower() }
        if ($line -match "(pub(\(.*?\))?\s+)?const\s+(\w+)") { return $Matches[3].ToLower() }
        if ($line -match "(pub(\(.*?\))?\s+)?type\s+(\w+)") { return $Matches[3].ToLower() }
        if ($line -match "#\[cfg\(test\)\]") { return "tests" }
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
        Write-Host "  SKIP: not found" -ForegroundColor Yellow
        return
    }

    $allLines = Get-Content $modFile -Encoding UTF8
    $totalLines = $allLines.Count

    if ($totalLines -le ($MaxLines + 30)) {
        Write-Host "  SKIP $Module ($totalLines lines)" -ForegroundColor DarkGray
        return
    }

    Write-Host "`n=== $Module ($totalLines lines) ===" -ForegroundColor Cyan

    $defStarts = @(Find-ModuleLevelStarts -Lines $allLines)
    Write-Host "  $($defStarts.Count) module-level definitions"

    if ($defStarts.Count -le 1) {
        Write-Host "  SKIP: too few definitions to split" -ForegroundColor Red
        return
    }

    # Import section
    $importEnd = $defStarts[0]
    $importLines = @()
    if ($importEnd -gt 0) {
        $importLines = $allLines[0..($importEnd - 1)]
        while ($importLines.Count -gt 0 -and $importLines[-1].Trim() -eq "") {
            $importLines = $importLines[0..($importLines.Count - 2)]
        }
    }

    # Split into chunks
    $chunks = @(Split-AtBoundaries -Starts $defStarts -TotalLines $totalLines -MaxLines $MaxLines)

    # If only 1 chunk, try smaller target
    if ($chunks.Count -le 1) {
        $half = [Math]::Max(120, [int]($MaxLines / 2))
        $chunks = @(Split-AtBoundaries -Starts $defStarts -TotalLines $totalLines -MaxLines $half)
        if ($chunks.Count -le 1) {
            Write-Host "  SKIP: single indivisible block" -ForegroundColor Red
            return
        }
    }

    # Name chunks
    $usedNames = @{}
    $namedChunks = @()
    $idx = 0

    foreach ($chunk in $chunks) {
        $name = Get-NameFromLine -Lines $allLines -Start $chunk.StartLine
        if (-not $name) { $name = "part" }
        $name = ($name -replace '[^a-zA-Z0-9_]', '_').ToLower()

        # Deduplicate
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

        $color = if ($chunk.Size -gt 400) { "Red" } elseif ($chunk.Size -gt $MaxLines) { "Yellow" } else { "White" }
        Write-Host ("  [{0}] {1,-30} {2,5}-{3,5} ({4,4} lines)" -f $idx, $name, ($chunk.StartLine+1), ($chunk.EndLine+1), $chunk.Size) -ForegroundColor $color
        $idx++
    }

    if ($DryRun) {
        $over = ($namedChunks | Where-Object { $_.Size -gt 400 }).Count
        Write-Host "  [DRY] $($namedChunks.Count) files, $over over 400 lines" -ForegroundColor Yellow
        return
    }

    # Safety: check for existing files
    foreach ($nc in $namedChunks) {
        $f = Join-Path $modDir "$($nc.Name).rs"
        if (Test-Path $f) {
            Write-Host "  ABORT: $($nc.Name).rs exists" -ForegroundColor Red
            return
        }
    }

    # Create submodule files
    foreach ($nc in $namedChunks) {
        $code = $allLines[$nc.StartLine..$nc.EndLine]

        $content = @()
        $content += "#![allow(unused_imports)]"
        $content += "use super::*;"
        $content += ""

        foreach ($line in $importLines) {
            $t = $line.TrimStart()
            # Skip mod declarations and #![] from submodule copies
            if ($t -match "^mod\s+" -or $t -match "^pub(\(.*?\))?\s+mod\s+" -or $t -match "^#!\[") { continue }
            # Also skip pub use re-exports of existing submodules (they're in mod.rs)
            if ($t -match "^pub\s+use\s+(self|super|crate)::") { continue }
            $content += $line
        }
        $content += ""
        $content += $code

        $f = Join-Path $modDir "$($nc.Name).rs"
        $content | Set-Content $f -Encoding UTF8
        Write-Host "  + $($nc.Name).rs ($($content.Count) lines)" -ForegroundColor Green
    }

    # Rewrite mod.rs
    $mod = @()
    $mod += "#![allow(unused_imports)]"
    $mod += ""

    # Preserve ALL original import/mod lines
    foreach ($line in $importLines) {
        $t = $line.TrimStart()
        if ($t -match "^#!\[") { continue }
        $mod += $line
    }
    $mod += ""

    foreach ($nc in $namedChunks) {
        $mod += "mod $($nc.Name);"
    }
    $mod += ""
    foreach ($nc in $namedChunks) {
        $mod += "pub use $($nc.Name)::*;"
    }
    $mod += ""

    $modFile2 = Join-Path $modDir "mod.rs"
    $mod | Set-Content $modFile2 -Encoding UTF8
    Write-Host "  = mod.rs ($($mod.Count) lines)" -ForegroundColor Green
}

# ============================================================
# MAIN
# ============================================================

$modules = @(
    "types", "cli", "client", "commands", "config", "control",
    "copy_mode", "format", "help", "input", "layout", "pane",
    "platform", "popup", "rendering", "session", "ssh_input",
    "style", "tree", "util", "window_ops", "server"
)

$mode = if ($DryRun) { "DRY RUN" } else { "EXECUTE" }
Write-Host "=== Phase 2: Split Modules ($mode) ===" -ForegroundColor Magenta
Write-Host "Target: max $MaxChunkLines lines per chunk`n"

foreach ($m in $modules) {
    Split-RustModule -Module $m -MaxLines $MaxChunkLines -DryRun:$DryRun
}

Write-Host "`n=== Complete ===" -ForegroundColor Green
