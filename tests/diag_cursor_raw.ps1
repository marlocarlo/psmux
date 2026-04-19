###############################################################################
# diag_cursor_raw.ps1 – Raw dump of ALL escape sequences from Claude TUI
#
# Captures raw ConPTY output and scans for ANY cursor-related sequences:
# - DECSCUSR (\e[N q)
# - DECTCEM show/hide cursor (\e[?25h / \e[?25l)
# - Private mode sets/resets
# - Any other CSI sequences
###############################################################################
$ErrorActionPreference = 'Stop'

$rawLog = "$env:TEMP\psmux_raw_cursor_dump.log"

# Clean up
Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
Start-Sleep -Milliseconds 500
Remove-Item $rawLog -ErrorAction SilentlyContinue

# Set raw debug mode
$env:PSMUX_DEBUG_CURSOR = "1"
$env:PSMUX_DEBUG_RAW_ESC = "1"

Write-Host "=== Raw Escape Sequence Diagnostic ===" -ForegroundColor Cyan
Write-Host "Log: $rawLog"

# Start psmux session
Write-Host "Starting psmux session..." -ForegroundColor Yellow
$proc = Start-Process psmux -ArgumentList "new-session","-d" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

$end = (Get-Date).AddSeconds(10)
while ((Get-Date) -lt $end) {
    $r = psmux list-sessions 2>$null
    if ($LASTEXITCODE -eq 0) { break }
    Start-Sleep -Milliseconds 300
}

Write-Host "Launching Claude..." -ForegroundColor Yellow
psmux send-keys "cd C:\ccintelmac Enter" 2>$null
Start-Sleep -Milliseconds 500
psmux send-keys "claude Enter" 2>$null
Start-Sleep -Seconds 8

# Type something to trigger input cursor
psmux send-keys "hi" 2>$null
Start-Sleep -Seconds 3

# Exit Claude
psmux send-keys "Escape" 2>$null
Start-Sleep -Milliseconds 500
psmux send-keys "/exit Enter" 2>$null
Start-Sleep -Seconds 2

# Check for raw log
Write-Host "`nAnalysis:" -ForegroundColor Yellow
if (Test-Path $rawLog) {
    $data = Get-Content $rawLog
    Write-Host "  Found $($data.Count) raw escape events"
    $data | Select-Object -First 100 | ForEach-Object { Write-Host "    $_" }
} else {
    Write-Host "  No raw log (expected - feature not implemented yet)" -ForegroundColor Gray
}

# Check DECSCUSR log too
$debugLog = "$env:TEMP\psmux_cursor_debug.log"
if (Test-Path $debugLog) {
    $events = Get-Content $debugLog
    Write-Host "  DECSCUSR events: $($events.Count)"
    $events | ForEach-Object { Write-Host "    $_" }
} else {
    Write-Host "  No DECSCUSR events" -ForegroundColor Gray
}

Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
$env:PSMUX_DEBUG_CURSOR = $null
$env:PSMUX_DEBUG_RAW_ESC = $null

Write-Host "`n=== Done ===" -ForegroundColor Cyan
