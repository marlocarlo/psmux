$ErrorActionPreference = 'Stop'

$toolsDir = "$(Split-Path -Parent $MyInvocation.MyCommand.Definition)"
$url64 = 'https://github.com/marlocarlo/psmux/releases/download/v0.1.0/psmux-windows-x86_64.zip'

$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  unzipLocation  = $toolsDir
  url64bit       = $url64
  checksum64     = 'f3305fb9cd4ba44e516898de0b18a06b8b32e90b21f2047f05a9123eaa37ac79'
  checksumType64 = 'sha256'
}

Install-ChocolateyZipPackage @packageArgs

# Create shims for psmux, pmux, and tmux
$psmuxPath = Join-Path $toolsDir "psmux-windows-x86_64\psmux.exe"
$pmuxPath = Join-Path $toolsDir "psmux-windows-x86_64\pmux.exe"
$tmuxPath = Join-Path $toolsDir "psmux-windows-x86_64\tmux.exe"

Install-BinFile -Name "psmux" -Path $psmuxPath
Install-BinFile -Name "pmux" -Path $pmuxPath
Install-BinFile -Name "tmux" -Path $tmuxPath
