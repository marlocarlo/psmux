param(
  [string]$VsInstallPath = $env:VS_INSTALL_PATH
)

$ErrorActionPreference = "Stop"

$vsDevCmd = Join-Path $VsInstallPath "Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $vsDevCmd)) {
  throw "VsDevCmd.bat not found: $vsDevCmd"
}

cmd /c "`"$vsDevCmd`" -arch=x64 -host_arch=x64 && set" | ForEach-Object {
  if ($_ -match "^(.*?)=(.*)$") {
    Set-Item -Path "Env:\$($matches[1])" -Value $matches[2]
  }
}
