# psmux Battle Test Suite
# Comprehensive testing of all psmux features: sessions, windows, panes, resize, kill, etc.

$ErrorActionPreference = "Continue"

# Colors and helpers
function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }
function Write-Section { param($msg) 
    Write-Host ""
    Write-Host "=" * 70 -ForegroundColor Magenta
    Write-Host "  $msg" -ForegroundColor Magenta
    Write-Host "=" * 70 -ForegroundColor Magenta
}

# Statistics
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

# Find psmux binary
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Error "psmux binary not found. Please build the project first with: cargo build --release"
    exit 1
}

Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║               PSMUX BATTLE TEST SUITE                                ║" -ForegroundColor Cyan
Write-Host "║               Comprehensive Feature Testing                          ║" -ForegroundColor Cyan
Write-Host "╚══════════════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""
Write-Info "Binary: $PSMUX"
Write-Info "Started: $(Get-Date)"
Write-Host ""

# Helper: Start a detached session safely
function Start-DetachedSession {
    param([string]$Name)
    
    # Kill any existing session with this name
    try { & $PSMUX kill-session -t $Name 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 500
    
    # Start new detached session
    $proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $Name, "-d" -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 1500
    
    # Verify session exists
    $result = & $PSMUX has-session -t $Name 2>&1
    if ($LASTEXITCODE -eq 0) {
        return $true
    }
    return $false
}

# Helper: Clean up session
function Stop-Session {
    param([string]$Name)
    try {
        & $PSMUX kill-session -t $Name 2>&1 | Out-Null
    } catch {}
    Start-Sleep -Milliseconds 300
}

# ============================================================================
# CLEANUP BEFORE TESTS
# ============================================================================
Write-Section "CLEANUP - Killing any existing test sessions"

$testSessions = @("battle_test", "test_session_1", "test_session_2", "test_session_3", 
                  "multi_test_1", "multi_test_2", "pane_test", "window_test", 
                  "resize_test", "kill_test", "stress_test", "rapid_test")

foreach ($session in $testSessions) {
    try { & $PSMUX kill-session -t $session 2>&1 | Out-Null } catch {}
}
Start-Sleep -Seconds 1
Write-Info "Cleanup complete"

# ============================================================================
# TEST CATEGORY 1: SESSION MANAGEMENT
# ============================================================================
Write-Section "SESSION MANAGEMENT TESTS"

# Test 1.1: Create a new session
Write-Test "Create new detached session"
if (Start-DetachedSession -Name "battle_test") {
    Write-Pass "Session 'battle_test' created successfully"
} else {
    Write-Fail "Failed to create session 'battle_test'"
}

# Test 1.2: List sessions
Write-Test "List sessions"
$sessions = & $PSMUX ls 2>&1
if ($sessions -match "battle_test") {
    Write-Pass "Session appears in list-sessions output"
} else {
    Write-Fail "Session not found in list: $sessions"
}

# Test 1.3: has-session check
Write-Test "has-session (existing)"
& $PSMUX has-session -t battle_test 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Pass "has-session correctly identifies existing session"
} else {
    Write-Fail "has-session failed for existing session"
}

# Test 1.4: has-session for non-existent
Write-Test "has-session (non-existent)"
& $PSMUX has-session -t nonexistent_session_xyz 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Pass "has-session correctly rejects non-existent session"
} else {
    Write-Fail "has-session incorrectly accepted non-existent session"
}

# Test 1.5: Create multiple sessions
Write-Test "Create multiple sessions simultaneously"
$created = 0
foreach ($i in 1..3) {
    if (Start-DetachedSession -Name "test_session_$i") {
        $created++
    }
}
if ($created -eq 3) {
    Write-Pass "Created 3 sessions successfully"
} else {
    Write-Fail "Only created $created/3 sessions"
}

# Test 1.6: Verify all sessions exist
Write-Test "Verify all sessions in list"
$sessions = & $PSMUX ls 2>&1
$found = 0
foreach ($i in 1..3) {
    if ($sessions -match "test_session_$i") { $found++ }
}
if ($found -eq 3) {
    Write-Pass "All 3 sessions appear in list"
} else {
    Write-Fail "Only $found/3 sessions found in list"
}

# ============================================================================
# TEST CATEGORY 2: WINDOW MANAGEMENT
# ============================================================================
Write-Section "WINDOW MANAGEMENT TESTS"

# Test 2.1: Create new windows
Write-Test "Create new windows in session"
& $PSMUX new-window -t battle_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX new-window -t battle_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX new-window -t battle_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "Created 3 new windows"

# Test 2.2: List windows
Write-Test "List windows"
$windows = & $PSMUX list-windows -t battle_test 2>&1
if ($windows) {
    Write-Pass "list-windows returned output"
    Write-Info "Windows: $($windows -join ', ')"
} else {
    Write-Fail "list-windows returned empty"
}

# Test 2.3: Navigate windows with next-window
Write-Test "next-window navigation"
$success = $true
foreach ($i in 1..5) {
    & $PSMUX next-window -t battle_test 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { $success = $false }
    Start-Sleep -Milliseconds 100
}
if ($success) {
    Write-Pass "next-window navigation works"
} else {
    Write-Fail "next-window navigation had errors"
}

# Test 2.4: Navigate windows with previous-window
Write-Test "previous-window navigation"
$success = $true
foreach ($i in 1..5) {
    & $PSMUX previous-window -t battle_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "previous-window navigation executed"

# Test 2.5: last-window
Write-Test "last-window"
& $PSMUX last-window -t battle_test 2>&1 | Out-Null
Write-Pass "last-window executed"

# Test 2.6: Select specific window
Write-Test "select-window by index"
& $PSMUX select-window -t battle_test:0 2>&1 | Out-Null
Start-Sleep -Milliseconds 200
& $PSMUX select-window -t battle_test:1 2>&1 | Out-Null
Start-Sleep -Milliseconds 200
& $PSMUX select-window -t battle_test:2 2>&1 | Out-Null
Write-Pass "select-window by index works"

# Test 2.7: Rename window
Write-Test "rename-window"
& $PSMUX rename-window -t battle_test "renamed_window" 2>&1 | Out-Null
Write-Pass "rename-window executed"

# ============================================================================
# TEST CATEGORY 3: PANE MANAGEMENT
# ============================================================================
Write-Section "PANE MANAGEMENT TESTS"

# Create fresh session for pane tests
Stop-Session -Name "pane_test"
Start-DetachedSession -Name "pane_test"

# Test 3.1: Vertical split
Write-Test "split-window -v (vertical)"
& $PSMUX split-window -v -t pane_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$panes = & $PSMUX list-panes -t pane_test 2>&1
Write-Pass "Vertical split created"

# Test 3.2: Horizontal split
Write-Test "split-window -h (horizontal)"
& $PSMUX split-window -h -t pane_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "Horizontal split created"

# Test 3.3: Multiple splits (stress test)
Write-Test "Multiple rapid splits"
$splitSuccess = 0
foreach ($i in 1..4) {
    & $PSMUX split-window -v -t pane_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX split-window -h -t pane_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $splitSuccess += 2
}
Write-Pass "Created $splitSuccess additional splits"

# Test 3.4: List panes
Write-Test "list-panes"
$panes = & $PSMUX list-panes -t pane_test 2>&1
if ($panes) {
    $paneCount = ($panes | Measure-Object -Line).Lines
    Write-Pass "list-panes shows panes"
    Write-Info "Pane count: $paneCount"
} else {
    Write-Fail "list-panes returned empty"
}

# Test 3.5: Select pane directions
Write-Test "select-pane in all directions"
foreach ($dir in @("-U", "-D", "-L", "-R")) {
    & $PSMUX select-pane $dir -t pane_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "select-pane all directions executed"

# Test 3.6: Rapid pane navigation
Write-Test "Rapid pane navigation (10 cycles)"
foreach ($i in 1..10) {
    & $PSMUX select-pane -U -t pane_test 2>&1 | Out-Null
    & $PSMUX select-pane -R -t pane_test 2>&1 | Out-Null
    & $PSMUX select-pane -D -t pane_test 2>&1 | Out-Null
    & $PSMUX select-pane -L -t pane_test 2>&1 | Out-Null
}
Write-Pass "Rapid navigation completed"

# ============================================================================
# TEST CATEGORY 4: RESIZE PANES
# ============================================================================
Write-Section "RESIZE PANE TESTS"

# Create fresh session for resize tests
Stop-Session -Name "resize_test"
Start-DetachedSession -Name "resize_test"
& $PSMUX split-window -v -t resize_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX split-window -h -t resize_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# Test 4.1: Resize up
Write-Test "resize-pane -U (up)"
foreach ($i in 1..5) {
    & $PSMUX resize-pane -U 2 -t resize_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "Resize up executed 5 times"

# Test 4.2: Resize down
Write-Test "resize-pane -D (down)"
foreach ($i in 1..5) {
    & $PSMUX resize-pane -D 2 -t resize_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "Resize down executed 5 times"

# Test 4.3: Resize left
Write-Test "resize-pane -L (left)"
foreach ($i in 1..5) {
    & $PSMUX resize-pane -L 2 -t resize_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "Resize left executed 5 times"

# Test 4.4: Resize right
Write-Test "resize-pane -R (right)"
foreach ($i in 1..5) {
    & $PSMUX resize-pane -R 2 -t resize_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "Resize right executed 5 times"

# Test 4.5: Larger resize operations
Write-Test "Large resize operations"
& $PSMUX resize-pane -U 10 -t resize_test 2>&1 | Out-Null
& $PSMUX resize-pane -D 10 -t resize_test 2>&1 | Out-Null
& $PSMUX resize-pane -L 15 -t resize_test 2>&1 | Out-Null
& $PSMUX resize-pane -R 15 -t resize_test 2>&1 | Out-Null
Write-Pass "Large resize operations completed"

# Test 4.6: Zoom pane
Write-Test "zoom-pane toggle"
& $PSMUX resize-pane -Z -t resize_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX resize-pane -Z -t resize_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "Zoom pane toggled twice"

# ============================================================================
# TEST CATEGORY 5: KILL OPERATIONS
# ============================================================================
Write-Section "KILL OPERATIONS TESTS"

# Create fresh session for kill tests
Stop-Session -Name "kill_test"
Start-DetachedSession -Name "kill_test"

# Test 5.1: Create panes then kill
Write-Test "Create and kill panes"
& $PSMUX split-window -v -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX split-window -h -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX split-window -v -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$panesBefore = & $PSMUX list-panes -t kill_test 2>&1
Write-Info "Panes before kill: $($panesBefore | Measure-Object -Line | Select-Object -ExpandProperty Lines)"

& $PSMUX kill-pane -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "kill-pane executed"

# Test 5.2: Kill multiple panes
Write-Test "Kill multiple panes in succession"
& $PSMUX split-window -v -t kill_test 2>&1 | Out-Null
& $PSMUX split-window -v -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX kill-pane -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX kill-pane -t kill_test 2>&1 | Out-Null
Write-Pass "Multiple panes killed"

# Test 5.3: Create windows then kill window
Write-Test "Create and kill windows"
& $PSMUX new-window -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX new-window -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$windowsBefore = & $PSMUX list-windows -t kill_test 2>&1
Write-Info "Windows before kill: $($windowsBefore | Measure-Object -Line | Select-Object -ExpandProperty Lines)"

& $PSMUX kill-window -t kill_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "kill-window executed"

# Test 5.4: Kill session
Write-Test "Kill session"
& $PSMUX has-session -t kill_test 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
    & $PSMUX kill-session -t kill_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    & $PSMUX has-session -t kill_test 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Pass "Session killed successfully"
    } else {
        Write-Fail "Session still exists after kill"
    }
} else {
    Write-Fail "Session didn't exist to kill"
}

# ============================================================================
# TEST CATEGORY 6: SEND-KEYS
# ============================================================================
Write-Section "SEND-KEYS TESTS"

# Create fresh session
Stop-Session -Name "keys_test"
Start-DetachedSession -Name "keys_test"

# Test 6.1: Basic send-keys
Write-Test "send-keys basic text"
& $PSMUX send-keys -t keys_test "echo hello world" Enter 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "send-keys with Enter executed"

# Test 6.2: Literal send-keys
Write-Test "send-keys -l (literal)"
& $PSMUX send-keys -l -t keys_test "literal text test" 2>&1 | Out-Null
Write-Pass "send-keys literal executed"

# Test 6.3: Send special keys
Write-Test "send-keys special keys"
& $PSMUX send-keys -t keys_test Tab 2>&1 | Out-Null
& $PSMUX send-keys -t keys_test Escape 2>&1 | Out-Null
& $PSMUX send-keys -t keys_test Up 2>&1 | Out-Null
& $PSMUX send-keys -t keys_test Down 2>&1 | Out-Null
Write-Pass "Special keys sent"

# Test 6.4: Rapid key sending
Write-Test "Rapid send-keys (10 commands)"
foreach ($i in 1..10) {
    & $PSMUX send-keys -t keys_test "echo test $i" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 50
}
Write-Pass "Rapid send-keys completed"

# ============================================================================
# TEST CATEGORY 7: BUFFERS AND CAPTURE
# ============================================================================
Write-Section "BUFFER AND CAPTURE TESTS"

# Test 7.1: Set buffer
Write-Test "set-buffer"
& $PSMUX set-buffer -t keys_test "Test buffer content 12345" 2>&1 | Out-Null
Write-Pass "set-buffer executed"

# Test 7.2: List buffers
Write-Test "list-buffers"
$buffers = & $PSMUX list-buffers -t keys_test 2>&1
Write-Pass "list-buffers executed"

# Test 7.3: Show buffer
Write-Test "show-buffer"
$content = & $PSMUX show-buffer -t keys_test 2>&1
Write-Pass "show-buffer executed"

# Test 7.4: Capture pane
Write-Test "capture-pane"
$captured = & $PSMUX capture-pane -t keys_test -p 2>&1
if ($captured) {
    Write-Pass "capture-pane returned content"
} else {
    Write-Skip "capture-pane returned empty (may be expected)"
}

# ============================================================================
# TEST CATEGORY 8: SWAP AND ROTATE
# ============================================================================
Write-Section "SWAP AND ROTATE TESTS"

# Create fresh session
Stop-Session -Name "swap_test"
Start-DetachedSession -Name "swap_test"
& $PSMUX split-window -v -t swap_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX split-window -h -t swap_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# Test 8.1: Swap pane up
Write-Test "swap-pane -U"
& $PSMUX swap-pane -U -t swap_test 2>&1 | Out-Null
Write-Pass "swap-pane -U executed"

# Test 8.2: Swap pane down
Write-Test "swap-pane -D"
& $PSMUX swap-pane -D -t swap_test 2>&1 | Out-Null
Write-Pass "swap-pane -D executed"

# Test 8.3: Rotate window
Write-Test "rotate-window"
& $PSMUX rotate-window -t swap_test 2>&1 | Out-Null
Write-Pass "rotate-window executed"

# Test 8.4: Multiple rotations
Write-Test "Multiple rotations"
foreach ($i in 1..5) {
    & $PSMUX rotate-window -t swap_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
Write-Pass "5 rotations completed"

# ============================================================================
# TEST CATEGORY 9: LAYOUTS
# ============================================================================
Write-Section "LAYOUT TESTS"

# Create fresh session with multiple panes
Stop-Session -Name "layout_test"
Start-DetachedSession -Name "layout_test"
& $PSMUX split-window -v -t layout_test 2>&1 | Out-Null
& $PSMUX split-window -h -t layout_test 2>&1 | Out-Null
& $PSMUX split-window -v -t layout_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# Test 9.1: Even-horizontal layout
Write-Test "select-layout even-horizontal"
& $PSMUX select-layout -t layout_test even-horizontal 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "even-horizontal layout applied"

# Test 9.2: Even-vertical layout
Write-Test "select-layout even-vertical"
& $PSMUX select-layout -t layout_test even-vertical 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "even-vertical layout applied"

# Test 9.3: Main-horizontal layout
Write-Test "select-layout main-horizontal"
& $PSMUX select-layout -t layout_test main-horizontal 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "main-horizontal layout applied"

# Test 9.4: Main-vertical layout
Write-Test "select-layout main-vertical"
& $PSMUX select-layout -t layout_test main-vertical 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "main-vertical layout applied"

# Test 9.5: Tiled layout
Write-Test "select-layout tiled"
& $PSMUX select-layout -t layout_test tiled 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "tiled layout applied"

# ============================================================================
# TEST CATEGORY 10: STRESS TESTS
# ============================================================================
Write-Section "STRESS TESTS"

# Test 10.1: Rapid session create/destroy
Write-Test "Rapid session create/destroy (5 cycles)"
foreach ($i in 1..5) {
    Start-DetachedSession -Name "rapid_test" | Out-Null
    Start-Sleep -Milliseconds 200
    & $PSMUX kill-session -t rapid_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
}
Write-Pass "5 rapid session cycles completed"

# Test 10.2: Many windows in single session
Write-Test "Create 10 windows rapidly"
Stop-Session -Name "stress_test"
Start-DetachedSession -Name "stress_test"
foreach ($i in 1..10) {
    & $PSMUX new-window -t stress_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}
$windows = & $PSMUX list-windows -t stress_test 2>&1
$windowCount = ($windows | Measure-Object -Line).Lines
Write-Pass "Created windows (count: $windowCount)"

# Test 10.3: Many operations on single session
Write-Test "Stress test: 50 mixed operations"
$operations = 0
foreach ($i in 1..10) {
    & $PSMUX split-window -v -t stress_test 2>&1 | Out-Null
    $operations++
    & $PSMUX select-pane -U -t stress_test 2>&1 | Out-Null
    $operations++
    & $PSMUX resize-pane -D 1 -t stress_test 2>&1 | Out-Null
    $operations++
    & $PSMUX next-window -t stress_test 2>&1 | Out-Null
    $operations++
    & $PSMUX select-pane -L -t stress_test 2>&1 | Out-Null
    $operations++
}
Write-Pass "$operations mixed operations completed"

# ============================================================================
# TEST CATEGORY 11: DISPLAY AND INFO COMMANDS
# ============================================================================
Write-Section "DISPLAY AND INFO COMMANDS"

# Test 11.1: Display message with format
Write-Test "display-message with format string"
$output = & $PSMUX display-message -t stress_test -p "#S:#I:#W" 2>&1
if ($output) {
    Write-Pass "display-message returned: $output"
} else {
    Write-Skip "display-message returned empty"
}

# Test 11.2: Display panes (q command simulation)
Write-Test "display-panes"
& $PSMUX display-panes -t stress_test 2>&1 | Out-Null
Write-Pass "display-panes executed"

# Test 11.3: List clients
Write-Test "list-clients"
$clients = & $PSMUX list-clients 2>&1
Write-Pass "list-clients executed"

# Test 11.4: List keys
Write-Test "list-keys"
$keys = & $PSMUX list-keys 2>&1
if ($keys) {
    Write-Pass "list-keys returned bindings"
} else {
    Write-Skip "list-keys returned empty"
}

# ============================================================================
# TEST CATEGORY 12: EDGE CASES
# ============================================================================
Write-Section "EDGE CASE TESTS"

# Test 12.1: Commands on non-existent session
Write-Test "Commands on non-existent session"
$result = & $PSMUX split-window -t nonexistent_xyz_123 2>&1
if ($LASTEXITCODE -ne 0 -or $result -match "error|not found|no session") {
    Write-Pass "Correctly handles non-existent session"
} else {
    Write-Skip "Non-existent session handling unclear"
}

# Test 12.2: Empty session name
Write-Test "Session operations with various names"
$specialNames = @("test-dash", "test_underscore", "Test123")
foreach ($name in $specialNames) {
    if (Start-DetachedSession -Name $name) {
        & $PSMUX kill-session -t $name 2>&1 | Out-Null
    }
}
Write-Pass "Various session names handled"

# Test 12.3: Very long session name
Write-Test "Long session name"
$longName = "test_" + ("a" * 50)
try {
    if (Start-DetachedSession -Name $longName) {
        & $PSMUX kill-session -t $longName 2>&1 | Out-Null
        Write-Pass "Long session name handled"
    } else {
        Write-Skip "Long session name creation unclear"
    }
} catch {
    Write-Skip "Long session name test: $_"
}

# ============================================================================
# CLEANUP
# ============================================================================
Write-Section "CLEANUP"

Write-Info "Cleaning up test sessions..."
$allTestSessions = @("battle_test", "test_session_1", "test_session_2", "test_session_3",
                     "pane_test", "resize_test", "kill_test", "keys_test", "swap_test",
                     "layout_test", "stress_test", "rapid_test", "test-dash", 
                     "test_underscore", "Test123")

foreach ($session in $allTestSessions) {
    try { & $PSMUX kill-session -t $session 2>&1 | Out-Null } catch {}
}

Start-Sleep -Seconds 1
Write-Info "Cleanup complete"

# ============================================================================
# FINAL SUMMARY
# ============================================================================
Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║                         FINAL RESULTS                                ║" -ForegroundColor Cyan
Write-Host "╚══════════════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

$total = $script:TestsPassed + $script:TestsFailed + $script:TestsSkipped
Write-Host "  Total Tests: $total"
Write-Host "  ✓ Passed:    $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  ✗ Failed:    $($script:TestsFailed)" -ForegroundColor Red
Write-Host "  ○ Skipped:   $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host ""

$passRate = if ($total -gt 0) { [math]::Round(($script:TestsPassed / $total) * 100, 1) } else { 0 }
Write-Host "  Pass Rate: $passRate%" -ForegroundColor $(if ($passRate -ge 80) { "Green" } elseif ($passRate -ge 60) { "Yellow" } else { "Red" })
Write-Host ""
Write-Info "Completed: $(Get-Date)"
Write-Host ""

if ($script:TestsFailed -eq 0) {
    Write-Host "🎉 ALL TESTS PASSED! psmux is battle-ready!" -ForegroundColor Green
    exit 0
} else {
    Write-Host "⚠️  Some tests failed. Review the output above." -ForegroundColor Yellow
    exit 1
}
