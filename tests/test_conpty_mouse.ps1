#!/usr/bin/env pwsh
# Test: Does ConPTY pass through mouse tracking sequences?
# Run this inside psmux to verify the VT mouse path.

Write-Host "=== ConPTY Mouse Diagnostic Test ===" -ForegroundColor Cyan
Write-Host ""

# Test 1: Check if DECSET 1000 arrives in the output
Write-Host "Test 1: Sending DECSET 1000 (enable mouse tracking) to console..."
Write-Host "If psmux's vt100 parser sees this, mouse_protocol_mode should become non-None."
# Send DECSET 1000h (enable mouse tracking - press/release)
[System.Console]::Write("`e[?1000h")
Start-Sleep -Milliseconds 500

# Test 2: Also enable SGR 1006 encoding
Write-Host "Test 2: Sending DECSET 1006 (SGR mouse encoding)..."
[System.Console]::Write("`e[?1006h")
Start-Sleep -Milliseconds 500

Write-Host ""
Write-Host "If you see this text, DECSET sequences were sent." -ForegroundColor Green
Write-Host "Check psmux list-panes output to see if mouse protocol mode changed." -ForegroundColor Yellow
Write-Host ""
Write-Host "Now testing: click anywhere in this pane with the mouse."
Write-Host "If mouse tracking works, you should see escape sequences printed below."
Write-Host "Press Ctrl+C to exit."
Write-Host ""

# Read raw console input to see if mouse events arrive
try {
    $origMode = [System.Console]::TreatControlCAsInput
    [System.Console]::TreatControlCAsInput = $true
    
    while ($true) {
        if ([System.Console]::KeyAvailable) {
            $key = [System.Console]::ReadKey($true)
            $char = $key.KeyChar
            $code = [int]$char
            if ($code -eq 3) { break }  # Ctrl+C
            if ($code -eq 27) {
                # Escape sequence - read more
                $seq = "`e"
                Start-Sleep -Milliseconds 50
                while ([System.Console]::KeyAvailable) {
                    $next = [System.Console]::ReadKey($true)
                    $seq += $next.KeyChar
                }
                $escaped = $seq -replace "`e", "ESC"
                Write-Host "Got escape sequence: $escaped (length=$($seq.Length))" -ForegroundColor Magenta
            } else {
                Write-Host "Got key: '$char' (code=$code)" -ForegroundColor Gray
            }
        }
        Start-Sleep -Milliseconds 10
    }
} finally {
    [System.Console]::TreatControlCAsInput = $origMode
    # Disable mouse tracking
    [System.Console]::Write("`e[?1000l")
    [System.Console]::Write("`e[?1006l")
    Write-Host ""
    Write-Host "Mouse tracking disabled. Test complete." -ForegroundColor Cyan
}
