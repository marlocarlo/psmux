$ErrorActionPreference = "Continue"

# Ensure Cargo bin is on PATH
if ($env:CARGO_HOME -and (Test-Path "$env:CARGO_HOME\bin")) {
  if ($env:PATH -notlike "*$env:CARGO_HOME\bin*") {
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
  }
}

# Ensure git is on PATH
if (Test-Path "C:\git\cmd") {
  if ($env:PATH -notlike "*C:\git\cmd*") {
    $env:PATH = "C:\git\cmd;$env:PATH"
  }
}

# Auto-load VS dev environment (cl.exe, link.exe) if not already loaded
if (-not $env:VSCMD_VER) {
  $helper = "C:\Tools\ImportVsDevEnv.ps1"
  if (Test-Path $helper) {
    . $helper
  }
}
