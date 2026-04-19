# Wrapper: kill stale instances, clean up, then run format engine test
$ErrorActionPreference = "Continue"
taskkill /f /im psmux.exe 2>$null
taskkill /f /im pmux.exe 2>$null
taskkill /f /im tmux.exe 2>$null
Start-Sleep 3
Remove-Item "$env:USERPROFILE\.psmux\*.port","$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
& "$PSScriptRoot\test_format_engine.ps1"
