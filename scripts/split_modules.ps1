<#
.SYNOPSIS
  Splits large Rust module files into submodule files, each under 400 lines.
  
.DESCRIPTION
  For each mod.rs file over $MaxLines:
  1. Extracts the header (use/mod/extern/attribute block)
  2. Finds top-level item boundaries (fn, struct, enum, impl, const, static, type, trait)
  3. Groups consecutive items into chunks of ~$TargetLines (max $MaxLines)
  4. Writes each chunk as a submodule file
  5. Rewrites mod.rs with mod declarations and pub use re-exports
  
  Items > $MaxLines are kept in their own file and flagged for manual splitting.
#>

param(
    [string]$SrcDir = "src",
    [int]$MaxLines = 400,
    [int]$TargetLines = 300,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

function Find-HeaderEnd {
    param([string[]]$Lines)
    
    $headerEnd = 0
    $foundNonHeader = $false
    
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $trimmed = $line.TrimStart()
        
        if ($foundNonHeader) { break }
        
        # Header lines: use, pub use, mod, pub mod, extern, #![...], empty, regular comments
        if ($trimmed -eq '' -or 
            $trimmed -match '^use ' -or 
            $trimmed -match '^pub use ' -or 
            $trimmed -match '^pub\(crate\) use ' -or
            $trimmed -match '^mod ' -or 
            $trimmed -match '^pub mod ' -or
            $trimmed -match '^pub\(crate\) mod ' -or
            $trimmed -match '^extern ' -or 
            $trimmed -match '^#!\[' -or
            $trimmed -match '^// [^/]' -or  # regular comment (not doc comment)
            $trimmed -eq '//' -or
            $trimmed -match '^// Multi-binary' -or  # specific file header comments
            $trimmed -match '^#\[allow' -or
            $trimmed -match '^#\[cfg') {
            $headerEnd = $i + 1
        } else {
            $foundNonHeader = $true
        }
    }
    
    return $headerEnd
}

function Find-ItemBoundaries {
    param([string[]]$Lines, [int]$StartFrom)
    
    $items = @()
    $currentStart = $StartFrom
    
    for ($i = $StartFrom + 1; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        
        # Skip if line starts with whitespace (it's inside a block)
        if ($line.Length -gt 0 -and $line[0] -eq ' ') { continue }
        if ($line.Length -gt 0 -and $line[0] -eq "`t") { continue }
        
        # Check if this is a top-level item definition
        $isItemStart = $false
        if ($line -match '^(pub(\(crate\))?\s+)?(fn |struct |enum |impl |impl<|const |static |type |trait )') {
            $isItemStart = $true
        }
        
        if ($isItemStart) {
            # Walk backward to include doc comments, attributes, and cfg blocks
            $actualStart = $i
            while ($actualStart -gt $currentStart) {
                $prevLine = $Lines[$actualStart - 1].TrimStart()
                if ($prevLine -match '^///' -or 
                    $prevLine -match '^#\[' -or
                    $prevLine -eq '') {
                    $actualStart--
                } else {
                    break
                }
            }
            
            # Trim trailing blank lines from previous item
            $prevEnd = $actualStart - 1
            while ($prevEnd -gt $currentStart -and $Lines[$prevEnd].Trim() -eq '') {
                $prevEnd--
            }
            
            if ($prevEnd -ge $currentStart) {
                # Extract name from the definition
                $name = Extract-ItemName $Lines $currentStart $prevEnd
                $items += [PSCustomObject]@{
                    Start = $currentStart
                    End = $prevEnd
                    Lines = $prevEnd - $currentStart + 1
                    Name = $name
                }
            }
            
            $currentStart = $actualStart
        }
    }
    
    # Last item extends to end of file
    $lastEnd = $Lines.Count - 1
    while ($lastEnd -gt $currentStart -and $Lines[$lastEnd].Trim() -eq '') {
        $lastEnd--
    }
    
    if ($lastEnd -ge $currentStart) {
        $name = Extract-ItemName $Lines $currentStart $lastEnd
        $items += [PSCustomObject]@{
            Start = $currentStart
            End = $lastEnd
            Lines = $lastEnd - $currentStart + 1
            Name = $name
        }
    }
    
    return $items
}

function Extract-ItemName {
    param([string[]]$Lines, [int]$Start, [int]$End)
    
    for ($k = $Start; $k -le $End; $k++) {
        $line = $Lines[$k]
        # Skip doc comments, attributes, blank lines
        if ($line -match '^\s*///' -or $line -match '^\s*#\[' -or $line.Trim() -eq '') { continue }
        
        # Try to extract name
        if ($line -match '(?:pub(?:\(crate\))?\s+)?(?:fn|struct|enum|type|trait|mod|static|const)\s+(\w+)') {
            return $Matches[1]
        }
        if ($line -match 'impl(?:<[^>]*>)?\s+(\w+)') {
            return "impl_$($Matches[1])"
        }
        # Fallback
        return "item_$Start"
    }
    return "item_$Start"
}

function Group-Items {
    param([object[]]$Items, [int]$Target, [int]$Max)
    
    $chunks = @()
    $currentChunk = @()
    $currentLines = 0
    
    foreach ($item in $Items) {
        # If adding this item would exceed max AND we have items already, start new chunk
        if ($currentLines + $item.Lines -gt $Max -and $currentChunk.Count -gt 0) {
            $chunks += ,@($currentChunk)
            $currentChunk = @()
            $currentLines = 0
        }
        
        $currentChunk += $item
        $currentLines += $item.Lines
    }
    
    if ($currentChunk.Count -gt 0) {
        $chunks += ,@($currentChunk)
    }
    
    return $chunks
}

function Make-SafeFileName {
    param([string]$Name)
    # Convert CamelCase to snake_case and clean
    $name = $Name -replace '([a-z])([A-Z])', '$1_$2'
    $name = $name.ToLower()
    $name = $name -replace '[^a-z0-9_]', '_'
    $name = $name -replace '__+', '_'
    $name = $name.Trim('_')
    if ($name -eq 'mod' -or $name -eq 'self' -or $name -eq 'super' -or $name -eq 'crate') {
        $name = "${name}_defs"
    }
    return $name
}

function Get-PublicItems {
    param([string[]]$Lines, [int]$Start, [int]$End)
    
    $exports = @()
    for ($i = $Start; $i -le $End; $i++) {
        $line = $Lines[$i]
        # Match pub items at indent level 0 (not inside impl blocks)
        if ($line -match '^pub(\(crate\))?\s+(fn|struct|enum|const|static|type|trait)\s+(\w+)') {
            $vis = if ($Matches[1]) { "pub(crate)" } else { "pub" }
            $exports += [PSCustomObject]@{ Name = $Matches[3]; Vis = $vis; Kind = $Matches[2] }
        }
        # pub impl doesn't need re-export (methods are accessed through the type)
    }
    return $exports
}

# ====== MAIN LOGIC ======

$moduleFiles = Get-ChildItem $SrcDir -Recurse -Filter "mod.rs" | Where-Object {
    $lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
    $lines -gt $MaxLines
} | Sort-Object { (Get-Content $_.FullName | Measure-Object -Line).Lines } -Descending

Write-Host "Found $($moduleFiles.Count) modules over $MaxLines lines:" -ForegroundColor Cyan
foreach ($f in $moduleFiles) {
    $lines = (Get-Content $f.FullName | Measure-Object -Line).Lines
    $rel = $f.FullName.Replace((Get-Location).Path + "\", "")
    Write-Host "  $rel ($lines lines)"
}
Write-Host ""

$manualItems = @()

foreach ($modFile in $moduleFiles) {
    $modDir = $modFile.Directory.FullName
    $modPath = $modFile.FullName
    $rel = $modPath.Replace((Get-Location).Path + "\", "")
    $allLines = Get-Content $modPath
    $totalLines = $allLines.Count
    
    Write-Host "Processing $rel ($totalLines lines)..." -ForegroundColor Yellow
    
    # Step 1: Find header end
    $headerEnd = Find-HeaderEnd $allLines
    $header = if ($headerEnd -gt 0) { $allLines[0..($headerEnd-1)] } else { @() }
    
    Write-Host "  Header: lines 1-$headerEnd"
    
    # Step 2: Find item boundaries
    $items = Find-ItemBoundaries $allLines $headerEnd
    
    if ($items.Count -eq 0) {
        Write-Host "  No items found, skipping" -ForegroundColor Red
        continue
    }
    
    Write-Host "  Found $($items.Count) top-level items:"
    foreach ($item in $items) {
        $marker = if ($item.Lines -gt $MaxLines) { " [MANUAL]" } else { "" }
        Write-Host "    $($item.Name) (L$($item.Start+1)-L$($item.End+1), $($item.Lines) lines)$marker"
    }
    
    # Flag items that are too large for a single file
    foreach ($item in $items) {
        if ($item.Lines -gt $MaxLines) {
            $manualItems += "$rel :: $($item.Name) ($($item.Lines) lines)"
        }
    }
    
    # Step 3: Group items into chunks
    $chunks = Group-Items $items $TargetLines $MaxLines
    
    if ($chunks.Count -le 1) {
        Write-Host "  Only 1 chunk, skipping (needs manual split)" -ForegroundColor DarkYellow
        continue
    }
    
    Write-Host "  Grouped into $($chunks.Count) chunks"
    
    if ($DryRun) {
        foreach ($chunk in $chunks) {
            $chunkName = Make-SafeFileName $chunk[0].Name
            $chunkLines = ($chunk | Measure-Object -Property Lines -Sum).Sum
            $itemNames = ($chunk | ForEach-Object { $_.Name }) -join ", "
            Write-Host "    $chunkName.rs ($chunkLines lines): $itemNames"
        }
        Write-Host ""
        continue
    }
    
    # Step 4: Write chunk files
    $chunkNames = @()
    $allExports = @()
    $usedNames = @{}
    
    foreach ($chunk in $chunks) {
        $baseName = Make-SafeFileName $chunk[0].Name
        
        # Deduplicate names
        if ($usedNames.ContainsKey($baseName)) {
            $usedNames[$baseName]++
            $baseName = "${baseName}_$($usedNames[$baseName])"
        } else {
            $usedNames[$baseName] = 1
        }
        
        $chunkPath = Join-Path $modDir "$baseName.rs"
        $chunkLines = @()
        
        # Add allow unused imports
        $chunkLines += "#[allow(unused_imports)]"
        
        # Add original use statements (external crates only)
        foreach ($hLine in $header) {
            if ($hLine -match '^\s*use ' -or $hLine -match '^\s*pub(\(crate\))?\s+use ') {
                $chunkLines += $hLine
            }
        }
        
        # Add super::* for cross-module references
        $chunkLines += "use super::*;"
        $chunkLines += ""
        
        # Add the item content
        foreach ($item in $chunk) {
            $itemLines = $allLines[$item.Start..$item.End]
            $chunkLines += $itemLines
            $chunkLines += ""
        }
        
        # Get public items for re-export
        foreach ($item in $chunk) {
            $exports = Get-PublicItems $allLines $item.Start $item.End
            $allExports += $exports | ForEach-Object { [PSCustomObject]@{ Name = $_.Name; Vis = $_.Vis; Kind = $_.Kind; Module = $baseName } }
        }
        
        Set-Content -Path $chunkPath -Value ($chunkLines -join "`n") -NoNewline
        $chunkNames += $baseName
        
        $itemCount = $chunk.Count
        $lineCount = ($chunk | Measure-Object -Property Lines -Sum).Sum
        Write-Host "    Created $baseName.rs ($lineCount lines, $itemCount items)"
    }
    
    # Step 5: Rewrite mod.rs
    $modLines = @()
    
    # Keep file-level attributes
    foreach ($hLine in $header) {
        if ($hLine -match '^#!\[') {
            $modLines += $hLine
        }
    }
    
    # Add mod declarations
    $modLines += ""
    foreach ($name in $chunkNames) {
        $modLines += "mod $name;"
    }
    
    # Add pub use re-exports
    $modLines += ""
    
    # Group exports by module
    $moduleGroups = $allExports | Group-Object Module
    foreach ($group in $moduleGroups) {
        $modName = $group.Name
        $pubItems = $group.Group | Where-Object { $_.Vis -eq "pub" }
        $crateItems = $group.Group | Where-Object { $_.Vis -eq "pub(crate)" }
        
        if ($pubItems.Count -gt 0) {
            $names = ($pubItems | ForEach-Object { $_.Name }) -join ", "
            $modLines += "pub use ${modName}::{$names};"
        }
        if ($crateItems.Count -gt 0) {
            $names = ($crateItems | ForEach-Object { $_.Name }) -join ", "
            $modLines += "pub(crate) use ${modName}::{$names};"
        }
    }
    
    # Also add wildcard re-exports for impl blocks (methods) and non-pub items used internally
    $modLines += ""
    $modLines += "// Re-export everything for backward compatibility"
    foreach ($name in $chunkNames) {
        $modLines += "#[allow(unused_imports)]"
        $modLines += "pub(crate) use ${name}::*;"
    }
    
    $modLines += ""
    
    Set-Content -Path $modPath -Value ($modLines -join "`n") -NoNewline
    Write-Host "    Rewrote mod.rs ($($modLines.Count) lines)" -ForegroundColor Green
    Write-Host ""
}

# Summary
Write-Host "`n===== SUMMARY =====" -ForegroundColor Cyan
if ($manualItems.Count -gt 0) {
    Write-Host "Items over $MaxLines lines (need manual splitting):" -ForegroundColor Yellow
    foreach ($item in $manualItems) {
        Write-Host "  $item"
    }
}

# Count final file sizes
Write-Host "`nFinal file sizes:" -ForegroundColor Cyan
Get-ChildItem $SrcDir -Recurse -Filter "*.rs" | ForEach-Object {
    $lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
    [PSCustomObject]@{File=$_.FullName.Replace((Get-Location).Path + "\",""); Lines=$lines}
} | Where-Object { $_.Lines -gt $MaxLines } | Sort-Object Lines -Descending | ForEach-Object {
    Write-Host "  $($_.File): $($_.Lines) lines [OVER LIMIT]" -ForegroundColor Red
}
