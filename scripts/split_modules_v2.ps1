<#
.SYNOPSIS
  Splits large Rust mod.rs files into submodules under 400 lines.
#>
param(
    [string]$SrcDir = "src",
    [int]$MaxLines = 400,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

function Find-HeaderEnd {
    param([string[]]$Lines)
    $inMultiLineUse = $false
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $t = $Lines[$i].TrimStart()
        
        # If we're inside a multi-line use statement, keep going until we find the closing ;
        if ($inMultiLineUse) {
            if ($t -match ';') { $inMultiLineUse = $false }
            continue
        }
        
        # Check if this line starts a multi-line use (has { but no ;)
        if ($t -match '^(pub(\(crate\))?\s+)?use ' -and $t -match '\{' -and $t -notmatch ';') {
            $inMultiLineUse = $true
            continue
        }
        
        if ($t -ne '' -and
            $t -notmatch '^use ' -and $t -notmatch '^pub use ' -and $t -notmatch '^pub\(crate\) use ' -and
            $t -notmatch '^#!\[' -and $t -notmatch '^//' -and
            $t -notmatch '^extern ' -and $t -notmatch '^#\[' -and
            $t -notmatch '^(pub(\(crate\))?\s+)?mod ') {
            return $i
        }
    }
    return $Lines.Count
}

function Find-TopLevelStarts {
    param([string[]]$Lines, [int]$From)
    $starts = @()
    for ($i = $From; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        if ($line.Length -eq 0 -or $line[0] -eq ' ' -or $line[0] -eq "`t") { continue }
        if ($line -match '^(pub(\(crate\))?\s+)?(fn |struct |enum |impl[ <]|const |static |type |trait |mod \w+ \{|unsafe )' -or
            $line -match '^#\[cfg\(' -or
            $line -match '^(thread_local|lazy_static|macro_rules)!') {
            # Walk back to include doc comments and attributes
            $start = $i
            while ($start -gt $From -and ($Lines[$start-1].TrimStart() -match '^///' -or $Lines[$start-1].TrimStart() -match '^#\[')) {
                $start--
            }
            # Also skip blank lines between doc comment and previous item
            # (but keep the doc comment with this item)
            $starts += $start
        }
    }
    return ($starts | Sort-Object -Unique)
}

function Get-ItemName {
    param([string[]]$Lines, [int]$Start)
    for ($k = $Start; $k -lt [Math]::Min($Start + 10, $Lines.Count); $k++) {
        $line = $Lines[$k]
        if ($line -match '^\s*///' -or $line -match '^\s*#\[' -or $line.Trim() -eq '') { continue }
        if ($line -match '(?:pub(?:\(crate\))?\s+)?(?:fn|struct|enum|type|trait|mod|static|const)\s+(\w+)') {
            return $Matches[1]
        }
        if ($line -match 'impl(?:<[^>]*>)?\s+(\w+)') {
            return "impl_$($Matches[1])"
        }
        if ($line -match '^(thread_local|lazy_static|macro_rules)!\s*') {
            return "macros"
        }
        return "block_$Start"
    }
    return "block_$Start"
}

function Safe-FileName {
    param([string]$Name)
    # Just lowercase and clean, names are already snake_case from Rust
    $n = $Name.ToLower()
    $n = $n -replace '[^a-z0-9_]', '_'
    $n = $n -replace '_+', '_'
    $n = $n.Trim('_')
    if ($n -match '^(mod|self|super|crate|type|use)$') { $n = "${n}_def" }
    return $n
}

# ====== MAIN ======

$moduleFiles = Get-ChildItem $SrcDir -Recurse -Filter "mod.rs" | ForEach-Object {
    $lc = [System.IO.File]::ReadAllLines($_.FullName, [System.Text.UTF8Encoding]::new($false)).Count
    [PSCustomObject]@{ Path = $_.FullName; Dir = $_.Directory.FullName; LineCount = $lc }
} | Where-Object { $_.LineCount -gt $MaxLines } | Sort-Object LineCount -Descending

Write-Host "`nModules to split ($($moduleFiles.Count)):" -ForegroundColor Cyan
$moduleFiles | ForEach-Object { Write-Host "  $($_.Path.Replace($PWD.Path+'\',''))  ($($_.LineCount) lines)" }

$manual = @()

foreach ($mod in $moduleFiles) {
    $lines = [System.IO.File]::ReadAllLines($mod.Path, [System.Text.UTF8Encoding]::new($false))
    $total = $lines.Count
    $rel = $mod.Path.Replace($PWD.Path + '\', '')
    
    Write-Host "`n=== $rel ($total lines) ===" -ForegroundColor Yellow
    
    $headerEnd = Find-HeaderEnd $lines
    $header = if ($headerEnd -gt 0) { $lines[0..($headerEnd-1)] } else { @() }
    
    $starts = Find-TopLevelStarts $lines $headerEnd
    if ($starts.Count -eq 0) {
        Write-Host "  No items found, skipping" -ForegroundColor Red
        continue
    }
    
    # Build items: each item spans from its start to the line before the next item
    $items = @()
    for ($j = 0; $j -lt $starts.Count; $j++) {
        $s = $starts[$j]
        $e = if ($j -lt $starts.Count - 1) { $starts[$j+1] - 1 } else { $total - 1 }
        # Trim trailing blank lines
        while ($e -gt $s -and $lines[$e].Trim() -eq '') { $e-- }
        $name = Get-ItemName $lines $s
        $size = $e - $s + 1
        $items += [PSCustomObject]@{ Start=$s; End=$e; Name=$name; Size=$size }
        $flag = if ($size -gt $MaxLines) { " [>$MaxLines, NEEDS MANUAL]" } else { "" }
        Write-Host "  $name (L$($s+1)-L$($e+1), $size lines)$flag"
        if ($size -gt $MaxLines) { $manual += "$rel :: $name ($size lines)" }
    }
    
    # Group items into chunks under MaxLines
    $chunks = @()
    $cur = @()
    $curSize = 0
    foreach ($item in $items) {
        if ($curSize + $item.Size -gt $MaxLines -and $cur.Count -gt 0) {
            $chunks += ,@($cur)
            $cur = @()
            $curSize = 0
        }
        $cur += $item
        $curSize += $item.Size
    }
    if ($cur.Count -gt 0) { $chunks += ,@($cur) }
    
    if ($chunks.Count -le 1) {
        Write-Host "  -> Only 1 chunk possible (giant function?), skipping" -ForegroundColor DarkYellow
        continue
    }
    
    Write-Host "  -> $($chunks.Count) chunks:" -ForegroundColor Green
    
    # Generate unique file names
    $usedNames = @{}
    $chunkFiles = @()
    foreach ($chunk in $chunks) {
        $baseName = Safe-FileName $chunk[0].Name
        if ($usedNames.ContainsKey($baseName)) {
            $usedNames[$baseName]++
            $baseName = "${baseName}$($usedNames[$baseName])"
        } else { $usedNames[$baseName] = 1 }
        
        $totalSize = ($chunk | Measure-Object -Property Size -Sum).Sum
        $names = ($chunk | ForEach-Object { $_.Name }) -join ", "
        Write-Host "     $baseName.rs ($totalSize lines) = $names"
        
        $chunkFiles += [PSCustomObject]@{ 
            FileName = $baseName
            Items = $chunk
            TotalSize = $totalSize
        }
    }
    
    if ($DryRun) { continue }
    
    # Collect sibling module names from mod declarations in the header
    $siblingModNames = @()
    foreach ($h in $header) {
        if ($h -match '^\s*(pub(\(crate\))?\s+)?mod\s+(\w+)\s*;') {
            $siblingModNames += $Matches[3]
        }
    }
    
    # Write submodule files
    foreach ($cf in $chunkFiles) {
        $outPath = Join-Path $mod.Dir "$($cf.FileName).rs"
        $content = @()
        $content += "#[allow(unused_imports)]"
        # Copy ALL header lines (preserves multi-line use statements, comments, etc.)
        # but skip mod/pub mod declarations, #![...] crate-level attributes, and //! doc comments
        foreach ($h in $header) {
            $ht = $h.TrimStart()
            if ($ht -match '^(pub(\(crate\))?\s+)?mod ' -or $ht -match '^#!\[' -or $ht -match '^//!') { continue }
            # Convert sibling module use (e.g. "use helpers::..." -> "use super::helpers::...")
            $converted = $h
            foreach ($sib in $siblingModNames) {
                $converted = $converted -replace "^(\s*(?:pub(?:\(crate\))?\s+)?use\s+)${sib}::", "`$1super::${sib}::"
            }
            $content += $converted
        }
        $content += "use super::*;"
        $content += ""
        
        foreach ($item in $cf.Items) {
            $content += $lines[$item.Start..$item.End]
            $content += ""
        }
        
        # Convert sibling module references in body code
        # (e.g. "helpers::some_fn()" -> "super::helpers::some_fn()")
        if ($siblingModNames.Count -gt 0) {
            for ($ci = 0; $ci -lt $content.Count; $ci++) {
                foreach ($sib in $siblingModNames) {
                    # Only convert bare references, not already-qualified ones (super::, self::, crate::)
                    $content[$ci] = $content[$ci] -replace "(?<!super::)(?<!self::)(?<!crate::server::)(?<![a-zA-Z_])${sib}::", "super::${sib}::"
                }
            }
        }
        
        # Promote private top-level items to pub(crate) so they're visible through pub use *
        # Only modify lines at indent 0 that start a definition without pub
        for ($ci = 0; $ci -lt $content.Count; $ci++) {
            $cline = $content[$ci]
            # Skip indented lines (inside impl/fn blocks)
            if ($cline.Length -gt 0 -and ($cline[0] -eq ' ' -or $cline[0] -eq "`t")) { continue }
            # Promote private fn/struct/enum/const/static/type/trait to pub(crate)
            if ($cline -match '^(fn |struct |enum |const |static |type |trait |unsafe fn )') {
                $content[$ci] = "pub(crate) $cline"
            }
        }
        
        # Promote private methods inside impl blocks to pub(crate)
        # This allows cross-submodule access to struct methods
        $inImpl = $false
        $implBraceDepth = 0
        for ($ci = 0; $ci -lt $content.Count; $ci++) {
            $cline = $content[$ci]
            if (-not $inImpl) {
                if ($cline -match '^impl[ <]' -and $cline -match '\{') {
                    $inImpl = $true
                    $implBraceDepth = 1
                }
                continue
            }
            # Promote private fn inside impl BEFORE counting braces on this line
            # (so fn foo() { doesn't bump depth before we check)
            if ($implBraceDepth -eq 1 -and $cline -match '^\s+(fn |unsafe fn )' -and $cline -notmatch '^\s*pub') {
                $content[$ci] = $cline -replace '^(\s+)(fn |unsafe fn )', '$1pub(crate) $2'
                $cline = $content[$ci]  # update for brace counting below
            }
            # Track brace depth
            foreach ($char in $cline.ToCharArray()) {
                if ($char -eq '{') { $implBraceDepth++ }
                elseif ($char -eq '}') { $implBraceDepth-- }
            }
            if ($implBraceDepth -le 0) { $inImpl = $false; continue }
        }
        
        # Promote statics inside thread_local! blocks to pub(crate)
        for ($ci = 0; $ci -lt $content.Count; $ci++) {
            $cline = $content[$ci]
            if ($cline -match '^\s+static\s+\w+' -and $cline -notmatch '^\s*pub') {
                # Check if we're inside a thread_local! block (look backwards for thread_local!)
                for ($back = $ci - 1; $back -ge 0; $back--) {
                    $bl = $content[$back].TrimStart()
                    if ($bl -match '^thread_local') { 
                        $content[$ci] = $cline -replace '^(\s+)static ', '$1pub(crate) static '
                        break
                    }
                    if ($bl -eq '}' -or $bl -match '^(pub|fn |struct |enum |impl |const |type |trait )') { break }
                }
            }
        }
        
        # Promote struct/enum fields to pub(crate) for cross-submodule access
        # Promote struct fields to pub(crate) for cross-submodule access
        # Only apply to struct definitions, NOT enum variants (which can't have visibility)
        $inStruct = $false
        $braceDepth = 0
        for ($ci = 0; $ci -lt $content.Count; $ci++) {
            $cline = $content[$ci]
            # Detect start of struct definition (not enum!)
            if ($cline -match '^(pub(\(crate\))?\s+)?struct\s+\w+' -and $cline -match '\{') {
                $inStruct = $true
                $braceDepth = 1
                continue
            }
            # Detect start of enum (set flag to skip)
            if ($cline -match '^(pub(\(crate\))?\s+)?enum\s+\w+' -and $cline -match '\{') {
                $braceDepth = 1
                continue
            }
            if ($inStruct) {
                # Track brace depth
                foreach ($char in $cline.ToCharArray()) {
                    if ($char -eq '{') { $braceDepth++ }
                    elseif ($char -eq '}') { $braceDepth-- }
                }
                if ($braceDepth -le 0) { $inStruct = $false; continue }
                # Only promote fields at depth 1 (direct struct fields, not nested)
                if ($braceDepth -eq 1 -and $cline -match '^\s+\w+\s*:' -and $cline -notmatch '^\s*pub' -and $cline -notmatch '^\s*//' -and $cline -notmatch '^\s*#') {
                    $content[$ci] = $cline -replace '^(\s+)', '$1pub(crate) '
                }
            } elseif ($braceDepth -gt 0) {
                # Inside an enum or other non-struct braced item, just track depth
                foreach ($char in $cline.ToCharArray()) {
                    if ($char -eq '{') { $braceDepth++ }
                    elseif ($char -eq '}') { $braceDepth-- }
                }
            }
        }
        
        [System.IO.File]::WriteAllText($outPath, ($content -join "`n"), [System.Text.UTF8Encoding]::new($false))
    }
    
    # Unwrap inline modules: if a file X.rs contains "pub mod X { ... }" where X matches filename,
    # unwrap the module contents (remove the wrapping mod declaration and dedent)
    foreach ($cf in $chunkFiles) {
        $filePath = Join-Path $mod.Dir "$($cf.FileName).rs"
        $fileLines = [System.IO.File]::ReadAllLines($filePath, [System.Text.UTF8Encoding]::new($false))
        $modName = $cf.FileName
        $unwrapped = $false
        
        for ($li = 0; $li -lt $fileLines.Count; $li++) {
            $fl = $fileLines[$li]
            # Match: pub mod <name> { or #[cfg(...)] \n pub mod <name> {
            if ($fl -match "^(pub(\(crate\))?\s+)?mod\s+${modName}\s*\{" -or 
                ($fl -match "^#\[cfg\(" -and $li + 1 -lt $fileLines.Count -and $fileLines[$li+1] -match "^(pub(\(crate\))?\s+)?mod\s+${modName}\s*\{")) {
                
                # Find the cfg attribute if present
                $cfgAttr = $null
                $modStartLine = $li
                if ($fl -match "^#\[cfg\(") {
                    $cfgAttr = $fl
                    $modStartLine = $li + 1
                }
                
                # Find matching closing brace
                $depth = 0
                $modEndLine = -1
                for ($mi = $modStartLine; $mi -lt $fileLines.Count; $mi++) {
                    foreach ($char in $fileLines[$mi].ToCharArray()) {
                        if ($char -eq '{') { $depth++ }
                        elseif ($char -eq '}') { $depth-- }
                    }
                    if ($depth -eq 0) { $modEndLine = $mi; break }
                }
                
                if ($modEndLine -gt $modStartLine) {
                    Write-Host "    Unwrapping inline module '$modName' in $($cf.FileName).rs" -ForegroundColor Magenta
                    $newContent = @()
                    # Keep everything before the mod declaration
                    if ($li -gt 0) { $newContent += $fileLines[0..($li-1)] }
                    # Add cfg attribute if present
                    if ($cfgAttr) { $newContent += $cfgAttr }
                    # Dedent the inner content (skip the opening line and closing brace)
                    for ($inner = $modStartLine + 1; $inner -lt $modEndLine; $inner++) {
                        $iline = $fileLines[$inner]
                        # Remove one level of indentation (4 spaces or 1 tab)
                        if ($iline.StartsWith("    ")) { $iline = $iline.Substring(4) }
                        elseif ($iline.StartsWith("`t")) { $iline = $iline.Substring(1) }
                        $newContent += $iline
                    }
                    # Keep everything after the closing brace
                    if ($modEndLine + 1 -lt $fileLines.Count) { $newContent += $fileLines[($modEndLine+1)..($fileLines.Count-1)] }
                    
                    [System.IO.File]::WriteAllText($filePath, ($newContent -join "`n"), [System.Text.UTF8Encoding]::new($false))
                    $unwrapped = $true
                    # Re-read for any additional inline modules with same name (cfg variants)
                    $fileLines = [System.IO.File]::ReadAllLines($filePath, [System.Text.UTF8Encoding]::new($false))
                    $li = -1  # restart scan
                }
            }
        }
    }
    
    # Rewrite mod.rs
    $modContent = @()
    # Keep #![...] attributes and //! doc comments from header
    foreach ($h in $header) {
        $ht = $h.TrimStart()
        if ($ht -match '^#!\[' -or $ht -match '^//!') { $modContent += $h }
    }
    $modContent += ""
    
    # Keep pub use re-exports from original header (e.g. pub use crate::style::{...})
    # These are cross-module re-exports that must stay in mod.rs
    $inPubUseBlock = $false
    foreach ($h in $header) {
        $ht = $h.TrimStart()
        if ($inPubUseBlock) {
            $modContent += $h
            if ($ht -match ';') { $inPubUseBlock = $false }
            continue
        }
        if ($ht -match '^pub use ' -or $ht -match '^pub\(crate\) use ') {
            $modContent += $h
            if ($ht -match '\{' -and $ht -notmatch ';') { $inPubUseBlock = $true }
        }
    }
    
    # Keep private use imports as pub(crate) so submodules can access them via use super::*
    $inUseBlock = $false
    foreach ($h in $header) {
        $ht = $h.TrimStart()
        if ($inUseBlock) {
            $modContent += $h
            if ($ht -match ';') { $inUseBlock = $false }
            continue
        }
        # Match private use (not pub, not pub(crate)) and promote to pub(crate)
        if ($ht -match '^use ' -and $ht -notmatch '^pub') {
            $promoted = $h -replace '^(\s*)use ', '$1pub(crate) use '
            $modContent += $promoted
            if ($ht -match '\{' -and $ht -notmatch ';') { $inUseBlock = $true }
        }
    }
    $modContent += ""
    
    # Existing submodule declarations (server has connection.rs, helpers.rs, etc.)
    $existingSubs = Get-ChildItem $mod.Dir -Filter "*.rs" | Where-Object { $_.Name -ne "mod.rs" -and $_.Name -notin ($chunkFiles | ForEach-Object { "$($_.FileName).rs" }) }
    foreach ($sub in $existingSubs) {
        $subName = $sub.BaseName
        # Preserve original visibility from the old mod.rs
        $origDecl = $header | Where-Object { $_ -match "^\s*(pub(\(crate\))?\s+)?mod\s+$subName\s*;" }
        if ($origDecl) {
            $modContent += $origDecl[0]
        } else {
            $modContent += "pub(crate) mod $subName;"
        }
    }
    
    # New submodule declarations - use pub(crate) to preserve module-qualified paths
    foreach ($cf in $chunkFiles) {
        $modContent += "pub(crate) mod $($cf.FileName);"
    }
    $modContent += ""
    
    # Re-export everything from new submodules
    foreach ($cf in $chunkFiles) {
        $modContent += "pub use $($cf.FileName)::*;"
    }
    $modContent += ""
    
    [System.IO.File]::WriteAllText($mod.Path, ($modContent -join "`n"), [System.Text.UTF8Encoding]::new($false))
    Write-Host "  mod.rs rewritten ($($modContent.Count) lines)" -ForegroundColor Green
}

# Summary
Write-Host "`n===== MANUAL SPLITS NEEDED =====" -ForegroundColor Yellow
$manual | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }

Write-Host "`n===== FILES STILL OVER $MaxLines LINES =====" -ForegroundColor Yellow
Get-ChildItem $SrcDir -Recurse -Filter "*.rs" | ForEach-Object {
    $lc = [System.IO.File]::ReadAllLines($_.FullName, [System.Text.UTF8Encoding]::new($false)).Count
    if ($lc -gt $MaxLines) {
        Write-Host "  $($_.FullName.Replace($PWD.Path+'\',''))  ($lc lines)" -ForegroundColor Red
    }
}
