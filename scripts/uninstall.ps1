# psmux uninstall script for Windows

param(
    [string]$InstallDir = "$env:LOCALAPPDATA\psmux"
)

$ErrorActionPreference = 'Stop'

Write-Host "psmux uninstaller" -ForegroundColor Cyan
Write-Host "=================" -ForegroundColor Cyan

# Kill any running sessions first
Write-Host "Stopping any running sessions..."
$psmuxPath = Join-Path $InstallDir "psmux.exe"
if (Test-Path $psmuxPath) {
    try {
        & $psmuxPath kill-server 2>$null
    } catch {}
}

# Also try to stop by process name
Get-Process -Name psmux,pmux,tmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

Start-Sleep -Seconds 1

# Remove install directory
if (Test-Path $InstallDir) {
    Write-Host "Removing $InstallDir..."
    Remove-Item -Recurse -Force $InstallDir
    Write-Host "  Removed install directory" -ForegroundColor Green
} else {
    Write-Host "Install directory not found: $InstallDir" -ForegroundColor Yellow
}

# Remove from PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -like "*$InstallDir*") {
    Write-Host "Removing from PATH..."
    $NewPath = ($UserPath -split ';' | Where-Object { $_ -ne $InstallDir }) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    Write-Host "  Removed from user PATH" -ForegroundColor Green
}

# Clean up psmux data directory
$DataDir = "$env:USERPROFILE\.psmux"
if (Test-Path $DataDir) {
    $response = Read-Host "Remove psmux data directory ($DataDir)? [y/N]"
    if ($response -eq 'y' -or $response -eq 'Y') {
        Remove-Item -Recurse -Force $DataDir
        Write-Host "  Removed data directory" -ForegroundColor Green
    } else {
        Write-Host "  Kept data directory" -ForegroundColor Yellow
    }
}

Write-Host ""
Write-Host "Uninstall complete!" -ForegroundColor Green
Write-Host "Restart your terminal to apply PATH changes."
