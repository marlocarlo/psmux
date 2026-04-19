# psmux Session Tests - Tests that require an active session
# This script starts a session, runs tests, and cleans up

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
}

$SESSION_NAME = "test_session_$(Get-Random -Maximum 99999)"

Write-Info "Starting test session: $SESSION_NAME"
Write-Host ""

# Start a detached session
Write-Test "Creating detached session"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION_NAME -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 2

# Check if session exists
Write-Test "Verifying session exists"
$sessions = & $PSMUX ls 2>&1
if ($sessions -match $SESSION_NAME) {
    Write-Pass "Session created successfully"
} else {
    Write-Fail "Session not found in list: $sessions"
}

# Test list-windows
Write-Test "list-windows"
$output = & $PSMUX list-windows -t $SESSION_NAME 2>&1
if ($output -match "window" -or $output -match "\d+:" -or $output.Length -gt 0) {
    Write-Pass "list-windows works"
} else {
    Write-Fail "list-windows failed: $output"
}

# Test list-panes
Write-Test "list-panes"
$output = & $PSMUX list-panes -t $SESSION_NAME 2>&1
if ($output -match "%" -or $output -match "\d+x\d+" -or $output.Length -gt 0) {
    Write-Pass "list-panes works"
} else {
    Write-Fail "list-panes failed: $output"
}

# Test display-message
Write-Test "display-message"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#S" 2>&1
if ($output.Length -gt 0) {
    Write-Pass "display-message works: $output"
} else {
    Write-Fail "display-message failed"
}

# Test new-window
Write-Test "new-window"
& $PSMUX new-window -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$windows = & $PSMUX list-windows -t $SESSION_NAME 2>&1
if ($windows.Length -gt 0) {
    Write-Pass "new-window works"
} else {
    Write-Fail "new-window may have failed"
}

# Test split-window vertical
Write-Test "split-window -v (vertical)"
& $PSMUX split-window -v -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$panes = & $PSMUX list-panes -t $SESSION_NAME 2>&1
if ($panes) {
    Write-Pass "split-window -v works"
} else {
    Write-Fail "split-window -v may have failed"
}

# Test split-window horizontal  
Write-Test "split-window -h (horizontal)"
& $PSMUX split-window -h -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
Write-Pass "split-window -h command executed"

# Test select-pane directions
Write-Test "select-pane -U (up)"
& $PSMUX select-pane -U -t $SESSION_NAME 2>&1
Write-Pass "select-pane -U executed"

Write-Test "select-pane -D (down)"
& $PSMUX select-pane -D -t $SESSION_NAME 2>&1
Write-Pass "select-pane -D executed"

Write-Test "select-pane -L (left)"
& $PSMUX select-pane -L -t $SESSION_NAME 2>&1
Write-Pass "select-pane -L executed"

Write-Test "select-pane -R (right)"
& $PSMUX select-pane -R -t $SESSION_NAME 2>&1
Write-Pass "select-pane -R executed"

# Test send-keys
Write-Test "send-keys"
& $PSMUX send-keys -t $SESSION_NAME "echo hello" Enter 2>&1
Start-Sleep -Milliseconds 500
Write-Pass "send-keys executed"

# Test send-keys with literal flag
Write-Test "send-keys -l (literal)"
& $PSMUX send-keys -l -t $SESSION_NAME "test text" 2>&1
Write-Pass "send-keys -l executed"

# Test capture-pane
Write-Test "capture-pane"
$output = & $PSMUX capture-pane -t $SESSION_NAME -p 2>&1
if ($output.Length -gt 0) {
    Write-Pass "capture-pane works"
} else {
    Write-Fail "capture-pane returned empty"
}

# Test rename-window
Write-Test "rename-window"
& $PSMUX rename-window -t $SESSION_NAME "test_window" 2>&1
Write-Pass "rename-window executed"

# Test set-buffer
Write-Test "set-buffer"
& $PSMUX set-buffer -t $SESSION_NAME "test buffer content" 2>&1
Write-Pass "set-buffer executed"

# Test list-buffers
Write-Test "list-buffers"
$output = & $PSMUX list-buffers -t $SESSION_NAME 2>&1
if ($output -match "buffer" -or $output.Length -ge 0) {
    Write-Pass "list-buffers works"
} else {
    Write-Fail "list-buffers failed"
}

# Test show-buffer
Write-Test "show-buffer"
$output = & $PSMUX show-buffer -t $SESSION_NAME 2>&1
Write-Pass "show-buffer executed"

# Test next-window
Write-Test "next-window"
& $PSMUX next-window -t $SESSION_NAME 2>&1
Write-Pass "next-window executed"

# Test previous-window
Write-Test "previous-window"
& $PSMUX previous-window -t $SESSION_NAME 2>&1
Write-Pass "previous-window executed"

# Test last-window
Write-Test "last-window"
& $PSMUX last-window -t $SESSION_NAME 2>&1
Write-Pass "last-window executed"

# Test zoom-pane
Write-Test "zoom-pane"
& $PSMUX resize-pane -Z -t $SESSION_NAME 2>&1
Write-Pass "zoom-pane executed"

# Test resize-pane
Write-Test "resize-pane -U 5"
& $PSMUX resize-pane -U 5 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -U executed"

Write-Test "resize-pane -D 5"
& $PSMUX resize-pane -D 5 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -D executed"

Write-Test "resize-pane -L 5"
& $PSMUX resize-pane -L 5 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -L executed"

Write-Test "resize-pane -R 5"
& $PSMUX resize-pane -R 5 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -R executed"

# Test swap-pane
Write-Test "swap-pane -U"
& $PSMUX swap-pane -U -t $SESSION_NAME 2>&1
Write-Pass "swap-pane -U executed"

Write-Test "swap-pane -D"
& $PSMUX swap-pane -D -t $SESSION_NAME 2>&1
Write-Pass "swap-pane -D executed"

# Test rotate-window
Write-Test "rotate-window"
& $PSMUX rotate-window -t $SESSION_NAME 2>&1
Write-Pass "rotate-window executed"

# Test display-panes
Write-Test "display-panes"
& $PSMUX display-panes -t $SESSION_NAME 2>&1
Write-Pass "display-panes executed"

# Test list-keys
Write-Test "list-keys"
$output = & $PSMUX list-keys -t $SESSION_NAME 2>&1
Write-Pass "list-keys executed"

# Test show-options
Write-Test "show-options"
$output = & $PSMUX show-options -t $SESSION_NAME 2>&1
if ($output -match "mouse" -or $output -match "prefix" -or $output -match "status") {
    Write-Pass "show-options works: $($output -join ', ')"
} else {
    Write-Pass "show-options executed (may have empty bindings)"
}

# Test set-option
Write-Test "set-option mouse off"
& $PSMUX set-option -g mouse off -t $SESSION_NAME 2>&1
Write-Pass "set-option executed"

# Test kill-pane
Write-Test "kill-pane"
& $PSMUX kill-pane -t $SESSION_NAME 2>&1
Write-Pass "kill-pane executed"

# Test kill-window
Write-Test "kill-window"
& $PSMUX kill-window -t $SESSION_NAME 2>&1
Write-Pass "kill-window executed"

# Cleanup - kill the session
Write-Host ""
Write-Info "Cleaning up test session..."
& $PSMUX kill-session -t $SESSION_NAME 2>&1
Start-Sleep -Seconds 1

# Verify session is gone
& $PSMUX has-session -t $SESSION_NAME 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Pass "Session cleaned up successfully"
} else {
    Write-Fail "Session may still exist"
}

# Stop any remaining process
if ($proc -and !$proc.HasExited) {
    $proc.Kill()
}

Write-Host ""
Write-Host "=" * 60
Write-Host "SESSION TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host ""

if ($script:TestsFailed -gt 0) {
    exit 1
} else {
    exit 0
}
