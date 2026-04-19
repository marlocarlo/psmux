#!/usr/bin/env pwsh
# mouse_diag.ps1 — Comprehensive mouse hover diagnostic
#
# Tests every link in the chain:
#   1. Client receives MouseEventKind::Moved
#   2. Client checks alternate_screen from layout JSON
#   3. Client sends mouse-move to server
#   4. Server receives mouse-move, calls remote_mouse_motion
#   5. Server checks screen_has_tui_content
#   6. Server injects SGR mouse via write_mouse_to_pty
#   7. Child reads SGR mouse from stdin
#
# Usage: pwsh tests/mouse_diag.ps1
#
# Prerequisites: psmux must be installed (cargo install --path .)

$ErrorActionPreference = "Continue"
$psmux = "psmux"
$session = "mouse_diag_$$"
$logDir = "$env:USERPROFILE\.psmux"
$mouseLog = "$logDir\mouse_debug.log"

Write-Host "=== Mouse Hover Diagnostic ===" -ForegroundColor Cyan
Write-Host ""

# Kill any existing test session
& $psmux kill-session -t $session 2>$null
Start-Sleep -Milliseconds 500

# Clean old logs
Remove-Item $mouseLog -ErrorAction SilentlyContinue

# Start server with debug enabled
$env:PSMUX_MOUSE_DEBUG = "1"
& $psmux new-session -d -s $session
Start-Sleep -Milliseconds 2000

# Step 1: Check if server is running
$ls = & $psmux ls 2>&1 | Out-String
if ($ls -notmatch $session) {
    Write-Host "[FAIL] Server session '$session' not found" -ForegroundColor Red
    exit 1
}
Write-Host "[OK] Server session running" -ForegroundColor Green

# Step 2: Start a simple python/bun script that reads stdin and logs mouse events
# For now, use a PowerShell script that enables VT input and logs raw bytes
$testScript = @'
# Enable VT input mode on console stdin
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class ConsoleVT {
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern IntPtr GetStdHandle(int nStdHandle);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool GetConsoleMode(IntPtr hConsoleHandle, out uint lpMode);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool SetConsoleMode(IntPtr hConsoleHandle, uint dwMode);
}
"@
$STD_INPUT = -10
$h = [ConsoleVT]::GetStdHandle($STD_INPUT)
$mode = 0
[ConsoleVT]::GetConsoleMode($h, [ref]$mode) | Out-Null
$origMode = $mode
Write-Host "Original console mode: 0x$($mode.ToString('X4'))"
$ENABLE_VIRTUAL_TERMINAL_INPUT = 0x0200
$ENABLE_MOUSE_INPUT = 0x0010  
$newMode = ($mode -bor $ENABLE_VIRTUAL_TERMINAL_INPUT -bor $ENABLE_MOUSE_INPUT) -band (-bnot 0x0004) -band (-bnot 0x0002) -band (-bnot 0x0001)
[ConsoleVT]::SetConsoleMode($h, $newMode) | Out-Null
$checkMode = 0
[ConsoleVT]::GetConsoleMode($h, [ref]$checkMode) | Out-Null
Write-Host "New console mode: 0x$($checkMode.ToString('X4'))"
$vtSet = ($checkMode -band $ENABLE_VIRTUAL_TERMINAL_INPUT) -ne 0
Write-Host "ENABLE_VIRTUAL_TERMINAL_INPUT: $vtSet"
# Enable mouse tracking via VT
Write-Host "`e[?1003h`e[?1006h" -NoNewline
Write-Host "Waiting for mouse events (10 seconds)..."
$logPath = "$env:USERPROFILE\.psmux\mouse_child_recv.log"
"START $(Get-Date -Format 'HH:mm:ss.fff')" | Out-File $logPath
$deadline = (Get-Date).AddSeconds(10)
$count = 0
while ((Get-Date) -lt $deadline) {
    if ([Console]::KeyAvailable) {
        $key = [Console]::ReadKey($true)
        $ch = [int]$key.KeyChar
        "$count ch=$ch ($($key.KeyChar)) key=$($key.Key) mod=$($key.Modifiers)" | Out-File $logPath -Append
        $count++
    }
    Start-Sleep -Milliseconds 10
}
# Disable mouse tracking
Write-Host "`e[?1003l`e[?1006l" -NoNewline
# Restore console mode
[ConsoleVT]::SetConsoleMode($h, $origMode) | Out-Null
"END count=$count $(Get-Date -Format 'HH:mm:ss.fff')" | Out-File $logPath -Append
Write-Host "Done. Received $count inputs. Log: $logPath"
'@

# Write test script to temp
$testScriptPath = "$env:TEMP\psmux_mouse_test.ps1"
$testScript | Out-File $testScriptPath -Encoding utf8

# Send test script to the session pane
& $psmux send-keys -t $session "pwsh -NoProfile -ExecutionPolicy Bypass -File `"$testScriptPath`"" Enter
Start-Sleep -Milliseconds 3000

# Step 3: Capture pane to verify the test script is running
$capture = & $psmux capture-pane -t $session -p 2>&1 | Out-String
Write-Host ""
Write-Host "=== Pane capture ===" -ForegroundColor Yellow
Write-Host $capture
Write-Host "=== End capture ===" -ForegroundColor Yellow
Write-Host ""

# Step 4: Check layout JSON for alternate_screen
$dump = & $psmux display-message -t $session -p '#{alternate_on}' 2>&1 | Out-String
Write-Host "alternate_screen flag: [$($dump.Trim())]"

# Step 5: Inject mouse-move commands directly via TCP
$portFile = "$logDir\$session.port"
if (Test-Path $portFile) {
    $port = (Get-Content $portFile -Raw).Trim()
    $keyFile = "$logDir\$session.key"
    $key = if (Test-Path $keyFile) { (Get-Content $keyFile -Raw).Trim() } else { "" }
    
    Write-Host ""
    Write-Host "=== Injecting mouse-move events via TCP (port $port) ===" -ForegroundColor Yellow
    
    # Send 5 mouse-move events at different coordinates
    for ($i = 1; $i -le 5; $i++) {
        try {
            $client = New-Object System.Net.Sockets.TcpClient
            $client.Connect("127.0.0.1", [int]$port)
            $stream = $client.GetStream()
            $writer = New-Object System.IO.StreamWriter($stream)
            if ($key) { $writer.Write("auth $key`n"); $writer.Flush() }
            $writer.Write("mouse-move $($i * 5) 5`n")
            $writer.Flush()
            Start-Sleep -Milliseconds 100
            $writer.Close()
            $client.Close()
            Write-Host "  Sent mouse-move $($i * 5) 5" -ForegroundColor Gray
        } catch {
            Write-Host "  [ERR] Failed to send mouse-move: $_" -ForegroundColor Red
        }
    }
    Start-Sleep -Milliseconds 2000
} else {
    Write-Host "[WARN] Port file not found: $portFile" -ForegroundColor Yellow
}

# Step 6: Check mouse debug log
Write-Host ""
Write-Host "=== Mouse debug log ===" -ForegroundColor Yellow
if (Test-Path $mouseLog) {
    $logContent = Get-Content $mouseLog
    Write-Host "  Log entries: $($logContent.Count)"
    $logContent | ForEach-Object { Write-Host "  $_" -ForegroundColor Gray }
} else {
    Write-Host "  [WARN] No mouse_debug.log found (PSMUX_MOUSE_DEBUG not active?)" -ForegroundColor Yellow
}

# Step 7: Check child receive log
Write-Host ""
Write-Host "=== Child receive log ===" -ForegroundColor Yellow
$childLog = "$logDir\mouse_child_recv.log"
if (Test-Path $childLog) {
    $childContent = Get-Content $childLog
    Write-Host "  Log entries: $($childContent.Count)"
    $childContent | ForEach-Object { Write-Host "  $_" -ForegroundColor Gray }
} else {
    Write-Host "  [WARN] No mouse_child_recv.log found" -ForegroundColor Yellow
}

# Cleanup
Write-Host ""
Write-Host "=== Cleanup ===" -ForegroundColor Yellow
& $psmux kill-session -t $session 2>$null
Write-Host "Done."
