# ============================================================================
# TEMPLATE ONLY - DO NOT PUSH THIS FILE DIRECTLY TO CHOCOLATEY
# ============================================================================
# The real chocolateyinstall.ps1 is generated at publish time with the correct
# SHA256 checksum by either:
#   - GitHub Actions:  .github/workflows/release.yml (publish-chocolatey job)
#   - Local publish:   scripts/publish-choco.ps1
#
# Both download the release zip, compute the hash, and generate this file.
# ============================================================================

$ErrorActionPreference = 'Stop'

$toolsDir = "$(Split-Path -Parent $MyInvocation.MyCommand.Definition)"
$url64 = 'https://github.com/psmux/psmux/releases/download/v__VERSION__/psmux-v__VERSION__-windows-x64.zip'

$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  unzipLocation  = $toolsDir
  url64bit       = $url64
  checksum64     = '__SHA256_COMPUTED_AT_PUBLISH_TIME__'
  checksumType64 = 'sha256'
}

Install-ChocolateyZipPackage @packageArgs

# Create shims for psmux, pmux, and tmux
$psmuxPath = Join-Path $toolsDir "psmux.exe"
$pmuxPath = Join-Path $toolsDir "pmux.exe"
$tmuxPath = Join-Path $toolsDir "tmux.exe"

Install-BinFile -Name "psmux" -Path $psmuxPath
Install-BinFile -Name "pmux" -Path $pmuxPath
Install-BinFile -Name "tmux" -Path $tmuxPath

# Pre-warm: trigger Windows Defender scan cache and spawn a warm server
# so the user's first 'psmux new-session' is instant.
Start-Process -FilePath $psmuxPath -ArgumentList 'warmup' -WindowStyle Hidden
