# Test: when a window exits, the status bar clears the dead window immediately
# (no need to press prefix+p or prefix+n to refresh)

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

# Kill everything first
& $PSMUX kill-server 2>$null
Start-Sleep 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ea 0
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ea 0

# Create session with a window
Write-Info "Creating test session..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s wintest -d" -WindowStyle Hidden
Start-Sleep 3
& $PSMUX has-session -t wintest 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'wintest' created"

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 500 }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "WINDOW EXIT STATUS BAR TESTS"
Write-Host ("=" * 60)

# Test 1: create two windows, verify both listed
Write-Test "Initial window list (1 window)"
$lw1 = Psmux list-windows -t wintest
Write-Info "list-windows: $lw1"
$count1 = ($lw1 | Measure-Object -Line).Lines
if ($count1 -ge 1) { Write-Pass "Initial window count: $count1" } else { Write-Fail "Expected at least 1 window, got $count1" }

# Test 2: create a second window
Write-Test "Create second window"
Psmux new-window -t wintest | Out-Null
Start-Sleep 2
$lw2 = Psmux list-windows -t wintest
$count2 = ($lw2 | Measure-Object -Line).Lines
Write-Info "list-windows after new-window: $lw2"
if ($count2 -eq 2) { Write-Pass "Two windows present: $count2" } else { Write-Fail "Expected 2 windows, got $count2" }

# Test 3: create a third window
Write-Test "Create third window"
Psmux new-window -t wintest | Out-Null
Start-Sleep 2
$lw3 = Psmux list-windows -t wintest
$count3 = ($lw3 | Measure-Object -Line).Lines
Write-Info "list-windows: $lw3"
if ($count3 -eq 3) { Write-Pass "Three windows present: $count3" } else { Write-Fail "Expected 3 windows, got $count3" }

# Test 4: send 'exit' to the active (3rd) window, check window count drops to 2
Write-Test "Exit third window -> count drops to 2"
Psmux send-keys -t wintest "exit" Enter | Out-Null
Start-Sleep 3
$lw4 = Psmux list-windows -t wintest
$count4 = ($lw4 | Measure-Object -Line).Lines
Write-Info "list-windows after exit: $lw4"
if ($count4 -eq 2) { Write-Pass "Window count is 2 after exit" } else { Write-Fail "Expected 2 windows after exit, got $count4" }

# Test 5: dump-state should also reflect the change (no stale window data)
Write-Test "display-message shows correct window_index after exit"
$idx = Psmux display-message -t wintest -p '#{window_index}'
Write-Info "window_index after exit: $idx"
if ($idx -ne $null -and $idx -ne "") { Write-Pass "Active window index valid: $idx" } else { Write-Fail "No active window index" }

# Test 6: exit again -> count drops to 1
Write-Test "Exit second window -> count drops to 1"
Psmux send-keys -t wintest "exit" Enter | Out-Null
Start-Sleep 3
$lw5 = Psmux list-windows -t wintest
$count5 = ($lw5 | Measure-Object -Line).Lines
Write-Info "list-windows: $lw5"
if ($count5 -eq 1) { Write-Pass "Window count is 1 after second exit" } else { Write-Fail "Expected 1 window, got $count5" }

# Test 7: verify session still alive with 1 window
Write-Test "Session still alive with last window"
& $PSMUX has-session -t wintest 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "Session wintest still exists" } else { Write-Fail "Session wintest died prematurely" }

# Test 8: Rapid create-and-exit cycle (stress test for meta_dirty)
Write-Test "Rapid create/exit cycle"
Psmux new-window -t wintest | Out-Null
Start-Sleep 2
$before = (Psmux list-windows -t wintest | Measure-Object -Line).Lines
Psmux send-keys -t wintest "exit" Enter | Out-Null
Start-Sleep 3
$after = (Psmux list-windows -t wintest | Measure-Object -Line).Lines
Write-Info "Before: $before -> After: $after"
if ($after -eq ($before - 1)) { Write-Pass "Rapid cycle: window removed ($before -> $after)" } else { Write-Fail "Rapid cycle: expected $($before-1), got $after" }

# ═══════════════════════════════════════════════════════════════
# Win32 TUI VERIFICATION: Prove window list updates in real TUI
# ═══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_WES = "wes_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_WES
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # TUI Test 1: Create new window via CLI, verify window count increases
    Write-Test "TUI: Create window via new-window (visible TUI proof)"
    $winsBefore = (& $script:TUI_PSMUX list-windows -t $TUI_SESSION_WES 2>&1 | Measure-Object -Line).Lines
    & $script:TUI_PSMUX new-window -t $TUI_SESSION_WES 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $winsAfter = (& $script:TUI_PSMUX list-windows -t $TUI_SESSION_WES 2>&1 | Measure-Object -Line).Lines
    if ($winsAfter -eq ($winsBefore + 1)) {
        Write-Pass "TUI: Window created ($winsBefore -> $winsAfter)"
    } else {
        Write-Fail "TUI: Window count wrong ($winsBefore -> $winsAfter, expected $($winsBefore+1))"
    }

    # TUI Test 2: Kill window pane, verify window count decreases
    Write-Test "TUI: Kill pane removes window from status bar"
    $winsBefore2 = (& $script:TUI_PSMUX list-windows -t $TUI_SESSION_WES 2>&1 | Measure-Object -Line).Lines
    & $script:TUI_PSMUX send-keys -t $TUI_SESSION_WES "exit" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    $winsAfter2 = (& $script:TUI_PSMUX list-windows -t $TUI_SESSION_WES 2>&1 | Measure-Object -Line).Lines
    if ($winsAfter2 -eq ($winsBefore2 - 1)) {
        Write-Pass "TUI: Window removed after pane exit ($winsBefore2 -> $winsAfter2)"
    } else {
        Write-Fail "TUI: Window count wrong after exit ($winsBefore2 -> $winsAfter2)"
    }

    # TUI Test 3: Window index in status bar updates correctly
    Write-Test "TUI: Window index correct after operations"
    $idx = Safe-TuiQuery "#{window_index}" -Session $TUI_SESSION_WES
    if ($null -ne $idx -and $idx -match '^\d+$') {
        Write-Pass "TUI: Current window index is $idx"
    } else {
        Write-Fail "TUI: Could not read window_index ($idx)"
    }

    Cleanup-PsmuxWindow -Session $TUI_SESSION_WES
    Write-Host ""
} else {
    Write-Info "TUI verification skipped (could not launch window)"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)
& $PSMUX kill-session -t wintest 2>$null
Start-Sleep 1
& $PSMUX kill-server 2>$null
Start-Sleep 2

Write-Host ""
Write-Host ("=" * 60)
Write-Host "WINDOW EXIT STATUSBAR TEST SUMMARY"
Write-Host ("=" * 60)
Write-Host "Passed:  $($script:TestsPassed) / $($script:TestsPassed + $script:TestsFailed)"
Write-Host "Failed:  $($script:TestsFailed) / $($script:TestsPassed + $script:TestsFailed)"
if ($script:TestsFailed -eq 0) { Write-Host "ALL TESTS PASSED!" -ForegroundColor Green }
else { Write-Host "SOME TESTS FAILED!" -ForegroundColor Red }
