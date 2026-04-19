## Self-contained Shift+Enter diagnostic.
## Launches psmux, then uses psmux send-keys from WITHIN the psmux session 
## to deliver S-Enter through the send-keys path, then reads the server debug log.
##
## ALSO: launches the raw crossterm enter_diag tool in psmux to capture
## events as they come through psmux's ConPTY.

$env:PSMUX_INPUT_DEBUG = "1"
$env:PSMUX_SERVER_DEBUG = "1"
$logDir = "$env:USERPROFILE\.psmux"
$inputLog = "$logDir\input_debug.log"
$serverLog = "$logDir\server_debug.log"
$resultsFile = "$logDir\enter_diag_results.txt"

# Clean old logs
Remove-Item $inputLog -Force -ErrorAction SilentlyContinue
Remove-Item $serverLog -Force -ErrorAction SilentlyContinue
Remove-Item $resultsFile -Force -ErrorAction SilentlyContinue

Write-Host "=== Shift+Enter / Modified Enter Diagnostic ===" -ForegroundColor Cyan
Write-Host "Terminal: $($Host.Name)" -ForegroundColor Yellow
Write-Host ""

# Start psmux server in detached mode
Write-Host "Starting psmux server with debug logging..."
$null = Start-Process psmux -ArgumentList "new-session","-d" -PassThru
Start-Sleep -Seconds 3

# Use send-keys to inject modified Enter through the CLI path
Write-Host "Sending S-Enter via psmux send-keys..."
psmux send-keys "S-Enter"
Start-Sleep -Milliseconds 500

Write-Host "Sending C-Enter via psmux send-keys..."
psmux send-keys "C-Enter"
Start-Sleep -Milliseconds 500

Write-Host "Sending M-Enter via psmux send-keys..."
psmux send-keys "M-Enter" 
Start-Sleep -Milliseconds 500

Write-Host "Sending plain Enter via psmux send-keys..."
psmux send-keys "Enter"
Start-Sleep -Milliseconds 500

# Capture pane output
Write-Host "Capturing pane content..."
$paneContent = psmux capture-pane -p -t 0 2>&1
Write-Host "Pane content: $paneContent"
Start-Sleep -Seconds 1

# Kill server
psmux kill-server
Start-Sleep -Seconds 2

# Read and display logs
Write-Host ""
Write-Host "=== Results ===" -ForegroundColor Cyan

$output = @()
$output += "=== Terminal: $($env:WT_SESSION ? 'Windows Terminal' : ($env:WEZTERM_EXECUTABLE ? 'WezTerm' : 'Unknown')) ==="
$output += "Date: $(Get-Date)"
$output += ""

if (Test-Path $inputLog) {
    $lines = [System.IO.File]::ReadAllLines($inputLog)
    $enterLines = $lines | Where-Object { $_ -match "[Ee]nter|enter-diag" }
    $output += "=== Input Debug Log (Enter events) ==="
    $output += $enterLines
    $enterLines | ForEach-Object { Write-Host $_ }
} else {
    $output += "No input_debug.log found"
    Write-Host "No input_debug.log found" -ForegroundColor Red
}

$output += ""
if (Test-Path $serverLog) {
    $slines = [System.IO.File]::ReadAllLines($serverLog)
    $senterLines = $slines | Where-Object { $_ -match "[Ee]nter|send-key" }
    $output += "=== Server Debug Log (Enter events) ==="
    $output += $senterLines
    $senterLines | ForEach-Object { Write-Host $_ -ForegroundColor DarkGray }
}

$output | Set-Content $resultsFile
Write-Host ""
Write-Host "Results saved to: $resultsFile" -ForegroundColor Green
Write-Host "Press any key to close..."
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
