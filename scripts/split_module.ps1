<#
.SYNOPSIS
    Splits a Rust module's mod.rs into submodule files based on a split plan.
    Each split is defined by a start line and end line.
    After splitting, mod.rs is rewritten with mod declarations and pub use re-exports.

.DESCRIPTION
    Usage: .\split_module.ps1 -ModulePath <path> -Splits <array of hashtables>

    Each split hashtable has:
      - Name: submodule file name (without .rs)
      - StartLine: 1-based start line (inclusive)
      - EndLine: 1-based end line (inclusive)
      - Exports: array of public names to re-export

    Lines not covered by any split remain in mod.rs (typically use/imports at the top).
#>
param(
    [string]$ModulePath,
    [array]$Splits,
    [string[]]$ExtraModRsContent = @()
)

$modFile = Join-Path $ModulePath "mod.rs"
$lines = Get-Content $modFile

foreach ($split in $Splits) {
    $name = $split.Name
    $start = $split.StartLine - 1  # Convert to 0-based
    $end = $split.EndLine - 1

    # Extract lines for this submodule
    $subLines = $lines[$start..$end]

    # Write submodule file
    $subFile = Join-Path $ModulePath "$name.rs"
    $subLines | Set-Content $subFile -Encoding UTF8
    Write-Host "  Created $name.rs ($($subLines.Count) lines)"
}

# Build new mod.rs with mod declarations and pub use re-exports
$modContent = @()

# Add any extra content (e.g., imports that stay in mod.rs)
foreach ($extra in $ExtraModRsContent) {
    $modContent += $extra
}

# Add mod declarations and re-exports
foreach ($split in $Splits) {
    $name = $split.Name
    $modContent += "mod $name;"
}
$modContent += ""

foreach ($split in $Splits) {
    $name = $split.Name
    if ($split.Exports -and $split.Exports.Count -gt 0) {
        $exports = $split.Exports -join ", "
        $modContent += "pub use ${name}::{$exports};"
    } else {
        $modContent += "pub use ${name}::*;"
    }
}

$modContent | Set-Content $modFile -Encoding UTF8
Write-Host "  Rewrote mod.rs ($($modContent.Count) lines)"
