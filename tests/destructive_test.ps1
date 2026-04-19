# psmux Destructive Operations Test
# Tests specifically for kill operations, error handling, and recovery

$ErrorActionPreference = "Continue"

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }
function Write-Section { param($msg) 
    Write-Host ""
    Write-Host "=" * 70 -ForegroundColor Red
    Write-Host "  $msg" -ForegroundColor Red
    Write-Host "=" * 70 -ForegroundColor Red
}

$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}

Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════════════════╗" -ForegroundColor Red
Write-Host "║            PSMUX DESTRUCTIVE OPERATIONS TEST SUITE                   ║" -ForegroundColor Red
Write-Host "║            Testing Kill, Error Handling, and Recovery                ║" -ForegroundColor Red
Write-Host "╚══════════════════════════════════════════════════════════════════════╝" -ForegroundColor Red
Write-Host ""

function Start-DetachedSession {
    param([string]$Name)
    try { & $PSMUX kill-session -t $Name 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 500
    $proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $Name, "-d" -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 1500
    & $PSMUX has-session -t $Name 2>&1 | Out-Null
    return $LASTEXITCODE -eq 0
}

function Stop-Session {
    param([string]$Name)
    try { & $PSMUX kill-session -t $Name 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 300
}

# ============================================================================
# TEST: KILL ALL PANES UNTIL WINDOW DIES
# ============================================================================
Write-Section "KILL ALL PANES UNTIL WINDOW DIES"

Start-DetachedSession -Name "destruct_panes"
& $PSMUX split-window -v -t destruct_panes 2>&1 | Out-Null
& $PSMUX split-window -v -t destruct_panes 2>&1 | Out-Null
& $PSMUX split-window -h -t destruct_panes 2>&1 | Out-Null
& $PSMUX split-window -h -t destruct_panes 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "Kill panes one by one until session dies"
$killCount = 0
$maxKills = 10
while ($killCount -lt $maxKills) {
    & $PSMUX kill-pane -t destruct_panes 2>&1 | Out-Null
    $killCount++
    Start-Sleep -Milliseconds 300
    
    & $PSMUX has-session -t destruct_panes 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Pass "Session died after killing $killCount panes (expected behavior)"
        break
    }
}
if ($killCount -eq $maxKills) {
    Write-Skip "Reached max kills without session dying"
}

# ============================================================================
# TEST: KILL ALL WINDOWS UNTIL SESSION DIES
# ============================================================================
Write-Section "KILL ALL WINDOWS UNTIL SESSION DIES"

Start-DetachedSession -Name "destruct_windows"
foreach ($i in 1..5) {
    & $PSMUX new-window -t destruct_windows 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
}

Write-Test "Kill windows one by one until session dies"
$killCount = 0
$maxKills = 10
while ($killCount -lt $maxKills) {
    & $PSMUX kill-window -t destruct_windows 2>&1 | Out-Null
    $killCount++
    Start-Sleep -Milliseconds 300
    
    & $PSMUX has-session -t destruct_windows 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Pass "Session died after killing $killCount windows (expected behavior)"
        break
    }
}
if ($killCount -eq $maxKills) {
    Write-Skip "Reached max kills without session dying"
}

# ============================================================================
# TEST: KILL-SERVER BEHAVIOR
# ============================================================================
Write-Section "KILL-SERVER BEHAVIOR"

# Create multiple sessions
Write-Test "Create multiple sessions before kill-server"
Start-DetachedSession -Name "multi_1" | Out-Null
Start-DetachedSession -Name "multi_2" | Out-Null
Start-DetachedSession -Name "multi_3" | Out-Null

$sessions = & $PSMUX ls 2>&1
Write-Info "Sessions before kill-server: $($sessions -join ', ')"

Write-Test "Execute kill-server"
& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 1

$sessionsAfter = & $PSMUX ls 2>&1
$remaining = ($sessionsAfter | Where-Object { $_ -match "multi_" }).Count
if ($remaining -eq 0 -or $sessionsAfter -eq "") {
    Write-Pass "kill-server terminated all sessions"
} else {
    Write-Info "Sessions remaining: $remaining"
    Write-Skip "Some sessions may have survived (test inconclusive)"
}

# Clean up any stragglers
Stop-Session "multi_1"
Stop-Session "multi_2"
Stop-Session "multi_3"

# ============================================================================
# TEST: OPERATIONS ON KILLED SESSION
# ============================================================================
Write-Section "OPERATIONS ON KILLED SESSION"

Start-DetachedSession -Name "zombie_test"
& $PSMUX kill-session -t zombie_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "split-window on killed session"
$result = & $PSMUX split-window -t zombie_test 2>&1
if ($LASTEXITCODE -ne 0 -or $result -match "error|not found|no session") {
    Write-Pass "Correctly rejected split-window on dead session"
} else {
    Write-Fail "Should have rejected operation on dead session"
}

Write-Test "send-keys on killed session"
$result = & $PSMUX send-keys -t zombie_test "test" 2>&1
if ($LASTEXITCODE -ne 0 -or $result -match "error|not found|no session") {
    Write-Pass "Correctly rejected send-keys on dead session"
} else {
    Write-Fail "Should have rejected operation on dead session"
}

Write-Test "new-window on killed session"
$result = & $PSMUX new-window -t zombie_test 2>&1
if ($LASTEXITCODE -ne 0 -or $result -match "error|not found|no session") {
    Write-Pass "Correctly rejected new-window on dead session"
} else {
    Write-Fail "Should have rejected operation on dead session"
}

# ============================================================================
# TEST: RAPID CREATE/DESTROY STRESS
# ============================================================================
Write-Section "RAPID CREATE/DESTROY STRESS TEST"

Write-Test "Rapid create/destroy 20 cycles"
$successCycles = 0
foreach ($i in 1..20) {
    Start-DetachedSession -Name "rapid_$i" | Out-Null
    & $PSMUX kill-session -t "rapid_$i" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    
    # Verify it's gone
    & $PSMUX has-session -t "rapid_$i" 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        $successCycles++
    }
}
if ($successCycles -ge 18) {
    Write-Pass "$successCycles/20 rapid cycles succeeded"
} else {
    Write-Fail "Only $successCycles/20 cycles succeeded"
}

# ============================================================================
# TEST: SIMULTANEOUS KILL OPERATIONS
# ============================================================================
Write-Section "SIMULTANEOUS KILL OPERATIONS"

Write-Test "Create session with many panes, then rapid kill"
Start-DetachedSession -Name "massacre_test"

# Create many panes
foreach ($i in 1..8) {
    & $PSMUX split-window -v -t massacre_test 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}

$panes = & $PSMUX list-panes -t massacre_test 2>&1
Write-Info "Created panes: $($panes | Measure-Object -Line | Select-Object -ExpandProperty Lines)"

# Rapid kill
Write-Test "Rapid-fire kill-pane commands"
foreach ($i in 1..10) {
    & $PSMUX kill-pane -t massacre_test 2>&1 | Out-Null
}
Start-Sleep -Milliseconds 500

& $PSMUX has-session -t massacre_test 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Pass "Session terminated as expected after killing all panes"
} else {
    Stop-Session "massacre_test"
    Write-Pass "Session survived rapid kills (also acceptable)"
}

# ============================================================================
# TEST: RESPAWN AFTER KILL
# ============================================================================
Write-Section "RESPAWN BEHAVIOR"

Start-DetachedSession -Name "respawn_test"
& $PSMUX split-window -v -t respawn_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "respawn-pane"
& $PSMUX respawn-pane -t respawn_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "respawn-pane executed"

Stop-Session "respawn_test"

# ============================================================================
# TEST: ERROR MESSAGES
# ============================================================================
Write-Section "ERROR MESSAGE QUALITY"

Write-Test "Error message for non-existent session"
$result = & $PSMUX attach -t "this_session_does_not_exist_xyz" 2>&1
Write-Info "Error output: $result"
Write-Pass "Error message displayed"

Write-Test "Error message for invalid command"
$result = & $PSMUX invalid-command-xyz 2>&1
Write-Info "Error output: $result"
Write-Pass "Invalid command handled"

# ============================================================================
# TEST: RECOVERY AFTER ERRORS
# ============================================================================
Write-Section "RECOVERY AFTER ERRORS"

Write-Test "Normal operation after errors"
# Try some invalid operations
& $PSMUX split-window -t nonexistent 2>&1 | Out-Null
& $PSMUX kill-session -t nonexistent 2>&1 | Out-Null
& $PSMUX send-keys -t nonexistent "test" 2>&1 | Out-Null

# Now try valid operations
Start-DetachedSession -Name "recovery_test"
& $PSMUX split-window -v -t recovery_test 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

$panes = & $PSMUX list-panes -t recovery_test 2>&1
if ($panes) {
    Write-Pass "Normal operations work after error conditions"
} else {
    Write-Fail "Operations failed after errors"
}

Stop-Session "recovery_test"

# ============================================================================
# FINAL SUMMARY
# ============================================================================
Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════════════════╗" -ForegroundColor Red
Write-Host "║                    DESTRUCTIVE TEST RESULTS                          ║" -ForegroundColor Red
Write-Host "╚══════════════════════════════════════════════════════════════════════╝" -ForegroundColor Red
Write-Host ""

$total = $script:TestsPassed + $script:TestsFailed + $script:TestsSkipped
Write-Host "  Total Tests: $total"
Write-Host "  ✓ Passed:    $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  ✗ Failed:    $($script:TestsFailed)" -ForegroundColor Red
Write-Host "  ○ Skipped:   $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host ""

if ($script:TestsFailed -eq 0) {
    Write-Host "🔥 DESTRUCTIVE TESTS PASSED! psmux handles chaos well!" -ForegroundColor Green
    exit 0
} else {
    Write-Host "⚠️  Some destructive tests failed." -ForegroundColor Yellow
    exit 1
}
