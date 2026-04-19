# PTY Handle Stability Tests
# Tests that split-window and new-window don't crash with "The handle is invalid"
# Regression test for ConPTY slave handle leak
#
# This test creates multiple windows and splits, verifying that:
# 1. All panes stay alive (no premature exit)
# 2. PowerShell prompt appears in each pane
# 3. Panes can execute commands
# 4. Deeply nested splits work

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "[FATAL] psmux binary not found" -ForegroundColor Red
    exit 1
}

$SESSION = "pty_stability_$(Get-Random)"
Write-Info "Using psmux binary: $PSMUX"

# ─── Cleanup ──────────────────────────────────────────────────
Write-Info "Cleaning up stale sessions..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

# ─── Helper: count panes ─────────────────────────────────────
function Get-PaneCount {
    param($Session)
    $info = (& $PSMUX list-panes -t $Session 2>&1) | Out-String
    if ([string]::IsNullOrWhiteSpace($info)) { return 0 }
    return ($info.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }).Count
}

# ─── Helper: count windows ───────────────────────────────────
function Get-WindowCount {
    param($Session)
    $info = (& $PSMUX list-windows -t $Session 2>&1) | Out-String
    if ([string]::IsNullOrWhiteSpace($info)) { return 0 }
    return ($info.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }).Count
}

# ─── Helper: check panes are alive ───────────────────────────
function Test-PanesAlive {
    param($Session, $Expected, $Label, $WaitSec = 5)
    $deadline = (Get-Date).AddSeconds($WaitSec)
    $count = 0
    while ((Get-Date) -lt $deadline) {
        $count = Get-PaneCount -Session $Session
        if ($count -ge $Expected) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

# ═══════════════════════════════════════════════════════════════
Write-Host "=" * 60
Write-Host "PTY HANDLE STABILITY TESTS"
Write-Host "=" * 60

# ─── Start session ────────────────────────────────────────────
Write-Info "Starting test session: $SESSION"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3

$sessions = (& $PSMUX ls 2>&1) -join "`n"
if ($sessions -notmatch [regex]::Escape($SESSION)) {
    Write-Host "[FATAL] Could not start test session. Output: $sessions" -ForegroundColor Red
    exit 1
}
Write-Info "Session started successfully"
Write-Host ""

# ─── Test 1: Initial window has 1 pane ───────────────────────
Write-Test "Initial window has 1 pane"
$count = Get-PaneCount -Session $SESSION
if ($count -ge 1) {
    Write-Pass "Initial pane count: $count"
} else {
    Write-Fail "Expected at least 1 pane, got: $count"
}

# ─── Test 2: Split vertical once ─────────────────────────────
Write-Test 'Split vertical (Prefix + ")'
& $PSMUX split-window -v -t $SESSION 2>&1
Start-Sleep -Seconds 3
if (Test-PanesAlive -Session $SESSION -Expected 2 -Label "after split-v") {
    Write-Pass "2 panes after vertical split"
} else {
    $count = Get-PaneCount -Session $SESSION
    Write-Fail "Expected 2 panes after vertical split, got: $count"
}

# ─── Test 3: Split horizontal ────────────────────────────────
Write-Test "Split horizontal (Prefix + %)"
& $PSMUX split-window -h -t $SESSION 2>&1
Start-Sleep -Seconds 3
if (Test-PanesAlive -Session $SESSION -Expected 3 -Label "after split-h") {
    Write-Pass "3 panes after horizontal split"
} else {
    $count = Get-PaneCount -Session $SESSION
    Write-Fail "Expected 3 panes after horizontal split, got: $count"
}

# ─── Test 4: Wait and verify panes are still alive ───────────
Write-Test "Panes survive for 5 seconds (no premature exit)"
Start-Sleep -Seconds 5
$count = Get-PaneCount -Session $SESSION
if ($count -ge 3) {
    Write-Pass "All 3 panes still alive after 5 seconds"
} else {
    Write-Fail "Panes died! Expected 3, got: $count"
}

# ─── Test 5: Split the split (deeply nested) ─────────────────
Write-Test "Split an already-split pane (nested split)"
& $PSMUX split-window -v -t $SESSION 2>&1
Start-Sleep -Seconds 3
if (Test-PanesAlive -Session $SESSION -Expected 4 -Label "nested split") {
    Write-Pass "4 panes after nested split"
} else {
    $count = Get-PaneCount -Session $SESSION
    Write-Fail "Expected 4 panes after nested split, got: $count"
}

# ─── Test 6: Create new window ───────────────────────────────
Write-Test "Create new window"
& $PSMUX new-window -t $SESSION 2>&1
Start-Sleep -Seconds 3
$winCount = Get-WindowCount -Session $SESSION
if ($winCount -ge 2) {
    Write-Pass "2 windows after new-window (got $winCount)"
} else {
    Write-Fail "Expected 2 windows, got: $winCount"
}

# ─── Test 7: New window pane alive ───────────────────────────
Write-Test "New window pane is alive"
Start-Sleep -Seconds 3
$panes = Get-PaneCount -Session $SESSION
if ($panes -ge 1) {
    Write-Pass "New window pane alive (current window panes: $panes)"
} else {
    Write-Fail "New window pane not alive"
}

# ─── Test 8: Split new window multiple times ─────────────────
Write-Test "Multi-split new window (3 rapid splits)"
& $PSMUX split-window -v -t $SESSION 2>&1
Start-Sleep -Seconds 2
& $PSMUX split-window -h -t $SESSION 2>&1
Start-Sleep -Seconds 2
& $PSMUX split-window -v -t $SESSION 2>&1
Start-Sleep -Seconds 3

$panes = Get-PaneCount -Session $SESSION
if ($panes -ge 4) {
    Write-Pass "Multi-split: $panes panes in new window"
} else {
    Write-Fail "Expected at least 4 panes in new window, got: $panes"
}

# ─── Test 9: Stability check - all panes survive 5 more sec ──
Write-Test "All panes survive 5 more seconds after multi-split"
Start-Sleep -Seconds 5
$panes2 = Get-PaneCount -Session $SESSION
if ($panes2 -ge $panes) {
    Write-Pass "All panes still alive ($panes2 panes)"
} else {
    Write-Fail "Some panes died! Was $panes, now $panes2"
}

# ─── Test 10: Send a command to active pane ───────────────────
Write-Test "Send command to pane (send-keys echo hello)"
& $PSMUX send-keys -t $SESSION "echo psmux-pty-ok" Enter 2>&1
Start-Sleep -Seconds 2
$capture = (& $PSMUX capture-pane -p -t $SESSION 2>&1) | Out-String
if ($capture -match "psmux-pty-ok") {
    Write-Pass "Command executed successfully in pane"
} else {
    Write-Fail "Command output not found in capture. Output: $($capture.Substring(0, [Math]::Min(200, $capture.Length)))"
}

# ─── Test 11: Create 3 more windows rapidly ──────────────────
Write-Test "Create 3 more windows rapidly"
& $PSMUX new-window -t $SESSION 2>&1
Start-Sleep -Milliseconds 500
& $PSMUX new-window -t $SESSION 2>&1
Start-Sleep -Milliseconds 500
& $PSMUX new-window -t $SESSION 2>&1
Start-Sleep -Seconds 3

$winCount = Get-WindowCount -Session $SESSION
if ($winCount -ge 5) {
    Write-Pass "5 windows after rapid creation (got $winCount)"
} else {
    Write-Fail "Expected 5 windows, got: $winCount"
}

# ─── Test 12: Verify windows survive ─────────────────────────
Write-Test "All windows survive for 5 seconds"
Start-Sleep -Seconds 5
$winCount2 = Get-WindowCount -Session $SESSION
if ($winCount2 -ge $winCount) {
    Write-Pass "All windows still alive ($winCount2 windows)"
} else {
    Write-Fail "Some windows died! Was $winCount, now $winCount2"
}

# ─── Test 13: Go back to window 1 and verify panes ───────────
Write-Test "Switch to first window and verify its panes"
& $PSMUX select-window -t "$SESSION`:1" 2>&1
Start-Sleep -Seconds 1
$panes = Get-PaneCount -Session $SESSION
if ($panes -ge 4) {
    Write-Pass "First window still has $panes panes"
} else {
    Write-Fail "First window panes shrunk, expected >=4, got: $panes"
}

# ─── Cleanup ──────────────────────────────────────────────────
Write-Host ""
Write-Info "Cleaning up..."
& $PSMUX kill-session -t $SESSION 2>&1
Start-Sleep -Seconds 2

Write-Host ""
Write-Host "=" * 60
Write-Host "PTY STABILITY TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host ""

if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
    exit 0
}
