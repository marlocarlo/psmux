###############################################################################
# diag_cursor_claude.ps1 – Diagnose cursor shape sequences from Claude TUI
#
# Starts psmux with PSMUX_DEBUG_CURSOR=1, runs Claude TUI briefly inside it,
# then analyzes the debug log to see what DECSCUSR sequences ConPTY emits.
###############################################################################
$ErrorActionPreference = 'Stop'

$debugLog = "$env:TEMP\psmux_cursor_debug.log"

# Clean up
Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
Start-Sleep -Milliseconds 500
Remove-Item $debugLog -ErrorAction SilentlyContinue

# Set debug env var
$env:PSMUX_DEBUG_CURSOR = "1"

Write-Host "=== Cursor Shape Diagnostic ===" -ForegroundColor Cyan
Write-Host "Debug log: $debugLog"
Write-Host ""

# Start psmux session
Write-Host "Starting psmux session..." -ForegroundColor Yellow
$proc = Start-Process psmux -ArgumentList "new-session","-d" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

# Wait for session
$end = (Get-Date).AddSeconds(10)
while ((Get-Date) -lt $end) {
    $r = psmux list-sessions 2>$null
    if ($LASTEXITCODE -eq 0) { break }
    Start-Sleep -Milliseconds 300
}

Write-Host "Phase 1: Baseline - just idle shell" -ForegroundColor Yellow
Start-Sleep -Seconds 2

# Check what cursor shapes come from just the shell
if (Test-Path $debugLog) {
    $baseline = Get-Content $debugLog
    Write-Host "  Baseline cursor events: $($baseline.Count)"
    $baseline | Group-Object | ForEach-Object { Write-Host "    $($_.Count)x $($_.Name)" }
} else {
    Write-Host "  No cursor events from idle shell"
}

# Now launch Claude
Write-Host "`nPhase 2: Running Claude TUI..." -ForegroundColor Yellow
Remove-Item $debugLog -ErrorAction SilentlyContinue

# Check if claude is available
$claudePath = "C:\ccintelmac\claude.exe"
if (-not (Test-Path $claudePath)) {
    $claudePath = (Get-Command claude -ErrorAction SilentlyContinue).Source
}
if (-not $claudePath) {
    Write-Host "  Claude not found, using cursor-changing Write-Host test instead" -ForegroundColor Red
    # Simulate: set to blinking bar, wait, check
    psmux send-keys "Write-Host `"`e[5 q`"; Start-Sleep 2; Write-Host `"`e[0 q`" Enter" 2>$null
    Start-Sleep -Seconds 4
} else {
    Write-Host "  Claude found at: $claudePath"
    # Send claude command, wait for it to start, then quit
    psmux send-keys "cd C:\ccintelmac Enter" 2>$null
    Start-Sleep -Milliseconds 500
    psmux send-keys "claude Enter" 2>$null
    Start-Sleep -Seconds 5

    # Let it sit for a moment with focus in the input box
    # Then type something to trigger input cursor
    psmux send-keys "hello" 2>$null
    Start-Sleep -Seconds 3

    # Exit claude
    psmux send-keys "Escape" 2>$null
    Start-Sleep -Milliseconds 500
    psmux send-keys "/exit Enter" 2>$null
    Start-Sleep -Seconds 2
}

# Analyze results
Write-Host "`nPhase 3: Analysis" -ForegroundColor Yellow
if (Test-Path $debugLog) {
    $events = Get-Content $debugLog
    Write-Host "  Total cursor events during Claude: $($events.Count)"
    Write-Host ""
    Write-Host "  Events by type:" -ForegroundColor Cyan
    $events | Group-Object | Sort-Object Count -Descending | ForEach-Object {
        Write-Host "    $($_.Count)x  $($_.Name)"
    }
    Write-Host ""
    
    # Extract param values
    $params = $events | ForEach-Object {
        if ($_ -match 'param=(\d+)') { $Matches[1] }
    }
    Write-Host "  Unique param values seen: $($params | Sort-Object -Unique | Join-String -Separator ', ')" -ForegroundColor Green
    
    # Check for param=5 or param=6 (bar cursors)
    $barCursors = $params | Where-Object { $_ -eq '5' -or $_ -eq '6' }
    if ($barCursors.Count -gt 0) {
        Write-Host "  Bar cursor (5/6) events: $($barCursors.Count)" -ForegroundColor Green
    } else {
        Write-Host "  NO bar cursor (5/6) events detected!" -ForegroundColor Red
    }
    
    # Show first 50 events chronologically
    Write-Host ""
    Write-Host "  Chronological events (first 50):" -ForegroundColor Cyan
    $events | Select-Object -First 50 | ForEach-Object { Write-Host "    $_" }
} else {
    Write-Host "  NO debug log was created - no cursor sequences detected at all!" -ForegroundColor Red
}

# Cleanup
Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
$env:PSMUX_DEBUG_CURSOR = $null

Write-Host "`n=== Done ===" -ForegroundColor Cyan
