# Diagnostic: Launch psmux with input debug enabled, wait for user to press
# Shift+Enter, then read and display the enter-diag lines from the log.
# Usage: Run this script in the target terminal (Windows Terminal / WezTerm).

param(
    [string]$Terminal = "current"
)

$env:PSMUX_INPUT_DEBUG = "1"
$logFile = "$env:USERPROFILE\.psmux\input_debug.log"

# Remove old log
if (Test-Path $logFile) { Remove-Item $logFile -Force }

Write-Host "=== Shift+Enter Diagnostic ===" -ForegroundColor Cyan
Write-Host "Starting psmux with PSMUX_INPUT_DEBUG=1"
Write-Host ""
Write-Host "Instructions:" -ForegroundColor Yellow
Write-Host "  1. Once psmux starts, press Shift+Enter 3 times"
Write-Host "  2. Then press plain Enter 1 time"
Write-Host "  3. Then type 'exit' and press Enter to quit"
Write-Host "  4. Then press Ctrl+B, then : , then type 'kill-server' and press Enter"
Write-Host ""
Write-Host "Press any key to start psmux..."
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")

# Start psmux (will block until exit)
psmux

# Now read the log
Write-Host ""
Write-Host "=== input_debug.log (enter-diag lines) ===" -ForegroundColor Cyan
if (Test-Path $logFile) {
    Get-Content $logFile | Where-Object { $_ -match "enter-diag|Enter" } | ForEach-Object {
        Write-Host $_
    }
    Write-Host ""
    Write-Host "=== Full log path: $logFile ===" -ForegroundColor Green
} else {
    Write-Host "ERROR: Log file not found at $logFile" -ForegroundColor Red
}
