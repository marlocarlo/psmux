# Test: Send mouse events to claude running inside psmux and check for escape garbage
# Usage: Run after claude is already started in a psmux session named "mousetest"

param(
    [int]$Port = 64265,
    [string]$Key = "4104435b4b13f05d",
    [string]$Session = "mousetest"
)

function Send-MouseCmd {
    param([int]$P, [string]$K, [string]$Cmd)
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $P)
        $stream = $tcp.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.AutoFlush = $true
        
        # Authenticate (server expects uppercase AUTH)
        $writer.WriteLine("AUTH $K")
        Start-Sleep -Milliseconds 100
        $authResp = $reader.ReadLine()
        
        # Send command (mouse commands are fire-and-forget, no response)
        $writer.WriteLine($Cmd)
        Start-Sleep -Milliseconds 50
        
        $tcp.Close()
        return $authResp
    } catch {
        return "ERROR: $_"
    }
}

Write-Host "=== Claude Mouse Test ===" -ForegroundColor Cyan
Write-Host "Sending mouse events to claude in session '$Session'..."

# Capture pane BEFORE mouse events (baseline)
$before = psmux capture-pane -t $Session -p 2>&1
$beforeStr = ($before | Out-String)

# Send a variety of mouse events - left clicks at different positions
$positions = @(
    @{X=10; Y=5},
    @{X=30; Y=10},
    @{X=50; Y=15},
    @{X=20; Y=20},
    @{X=60; Y=25},
    @{X=40; Y=30},
    @{X=15; Y=8},
    @{X=55; Y=12}
)

Write-Host "Sending 8 left-clicks..."
foreach ($pos in $positions) {
    Send-MouseCmd -P $Port -K $Key -Cmd "mouse-down $($pos.X) $($pos.Y)"
    Start-Sleep -Milliseconds 50
    Send-MouseCmd -P $Port -K $Key -Cmd "mouse-up $($pos.X) $($pos.Y)"
    Start-Sleep -Milliseconds 100
}

# Send right-clicks
Write-Host "Sending 4 right-clicks..."
$rightPositions = @(
    @{X=20; Y=10},
    @{X=40; Y=20},
    @{X=60; Y=5},
    @{X=35; Y=15}
)
foreach ($pos in $rightPositions) {
    Send-MouseCmd -P $Port -K $Key -Cmd "mouse-down-right $($pos.X) $($pos.Y)"
    Start-Sleep -Milliseconds 50
    Send-MouseCmd -P $Port -K $Key -Cmd "mouse-up-right $($pos.X) $($pos.Y)"
    Start-Sleep -Milliseconds 100
}

# Send scroll events
Write-Host "Sending 4 scroll events..."
Send-MouseCmd -P $Port -K $Key -Cmd "mouse-scroll-up 30 15"
Start-Sleep -Milliseconds 100
Send-MouseCmd -P $Port -K $Key -Cmd "mouse-scroll-up 30 15"
Start-Sleep -Milliseconds 100
Send-MouseCmd -P $Port -K $Key -Cmd "mouse-scroll-down 30 15"
Start-Sleep -Milliseconds 100
Send-MouseCmd -P $Port -K $Key -Cmd "mouse-scroll-down 30 15"
Start-Sleep -Milliseconds 200

# Wait a moment for any output to settle
Start-Sleep 1

# Capture pane AFTER mouse events
$after = psmux capture-pane -t $Session -p 2>&1
$afterStr = ($after | Out-String)

Write-Host ""
Write-Host "=== Checking for escape sequence garbage ===" -ForegroundColor Yellow

# Check for SGR mouse escape sequences: ESC[<button;col;rowM or ESC[<button;col;rowm
$sgrPattern = '\[<\d+;\d+;\d+[Mm]'
$hasSgrGarbage = $afterStr -match $sgrPattern

# Check for legacy mouse escape sequences: ESC[M followed by 3 bytes  
$legacyPattern = '\[M.'
$hasLegacyGarbage = $afterStr -match $legacyPattern

# Check for raw CSI sequences that look like mouse
$csiPattern = '\x1b\[\d+;\d+[HMm]'
$hasCsiGarbage = $afterStr -match $csiPattern

Write-Host "SGR mouse garbage detected: $hasSgrGarbage"
Write-Host "Legacy mouse garbage detected: $hasLegacyGarbage"  
Write-Host "CSI sequence garbage detected: $hasCsiGarbage"

if (-not $hasSgrGarbage -and -not $hasLegacyGarbage -and -not $hasCsiGarbage) {
    Write-Host ""
    Write-Host "PASS: No escape sequence garbage detected!" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Host "FAIL: Escape sequence garbage found in pane output!" -ForegroundColor Red
    Write-Host "After content (last 10 lines):"
    $after | Select-Object -Last 10 | ForEach-Object { Write-Host "  $_" }
}

# Check debug log
Write-Host ""
Write-Host "=== Debug Log Check ===" -ForegroundColor Yellow
$logPath = "$env:USERPROFILE\.psmux\mouse_debug.log"
if (Test-Path $logPath) {
    $logLines = Get-Content $logPath
    Write-Host "Debug log has $($logLines.Count) entries"
    Write-Host "Last 10 entries:"
    $logLines | Select-Object -Last 10 | ForEach-Object { Write-Host "  $_" }
    
    # Check for VTI detection
    $vtiEntries = $logLines | Where-Object { $_ -match "vti" }
    if ($vtiEntries) {
        Write-Host ""
        Write-Host "VTI-related entries:" -ForegroundColor Cyan
        $vtiEntries | Select-Object -Last 5 | ForEach-Object { Write-Host "  $_" }
    }
    
    # Check for use_vt decision
    $useVtEntries = $logLines | Where-Object { $_ -match "use_vt" }
    if ($useVtEntries) {
        Write-Host ""
        Write-Host "use_vt decision entries:" -ForegroundColor Cyan
        $useVtEntries | Select-Object -Last 5 | ForEach-Object { Write-Host "  $_" }
    }
} else {
    Write-Host "No debug log file found at: $logPath"
    Write-Host "(Server may not have PSMUX_MOUSE_DEBUG=1 set)"
}

Write-Host ""
Write-Host "=== Test Complete ===" -ForegroundColor Cyan
