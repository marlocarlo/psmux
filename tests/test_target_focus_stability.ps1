# psmux Regression Test: Target-Flag Focus Stability
# Bug: Commands with `-t` flag (like display-message, list-panes, capture-pane)
#      were permanently changing the active window, causing window focus to bounce
#      when plugins (e.g. psmux-resurrect) periodically query all windows.
# Fix: Only select-window and select-pane commands permanently change focus via -t.
#      All other commands use temporary focus that auto-restores.
#
# Run: powershell -ExecutionPolicy Bypass -File tests\test_target_focus_stability.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function New-PsmuxSession {
    param([string]$Name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $Name -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
}

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

# ── Cleanup ──
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# ── Setup: session 'tfs' with 3 windows ──
Write-Info "Creating test session 'tfs' with 3 windows..."
New-PsmuxSession -Name "tfs"
& $PSMUX has-session -t tfs 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }

Psmux new-window -t tfs            # second window
Psmux new-window -t tfs            # third window
Start-Sleep -Milliseconds 500

# Dynamically discover window indices (handles any base-index)
$winLines = (& $PSMUX list-windows -t tfs -F "#{window_index}" 2>&1 | Out-String).Trim() -split "`n"
$W = @()
foreach ($line in $winLines) { $W += $line.Trim() }
if ($W.Count -lt 3) { Write-Host "FATAL: Need 3 windows, got $($W.Count)" -ForegroundColor Red; exit 1 }
$W0 = $W[0]  # first window index
$W1 = $W[1]  # second window index
$W2 = $W[2]  # third window index
Write-Info "Session tfs has $($W.Count) windows: indices $W0, $W1, $W2"

# Helper: get active window index for session tfs
function Get-ActiveWindow {
    (& $PSMUX display-message -t tfs -p "#{window_index}" 2>&1 | Out-String).Trim()
}

# ════════════════════════════════════════════════════════════
# TEST GROUP 1: DISPLAY-MESSAGE WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 1: display-message -t focus stability"
Write-Host ("=" * 60)

# Start on first window
Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

$active0 = Get-ActiveWindow
Write-Test "initial active window"
if ($active0 -eq $W0) { Write-Pass "active window is $W0" } else { Write-Fail "expected $W0, got '$active0'" }

# Query second window via display-message -t (should NOT switch)
Write-Test "display-message -t tfs:$W1 should NOT change active window"
Psmux display-message -t "tfs:$W1" -p "#{pane_current_path}" | Out-Null
$afterDm = Get-ActiveWindow
if ($afterDm -eq $W0) { Write-Pass "active window still $W0 after display-message -t tfs:$W1" } else { Write-Fail "active window changed to '$afterDm' (expected $W0)" }

# Query third window via display-message -t
Write-Test "display-message -t tfs:$W2 should NOT change active window"
Psmux display-message -t "tfs:$W2" -p "#{pane_current_path}" | Out-Null
$afterDm2 = Get-ActiveWindow
if ($afterDm2 -eq $W0) { Write-Pass "active window still $W0 after display-message -t tfs:$W2" } else { Write-Fail "active window changed to '$afterDm2' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 2: LIST-PANES WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 2: list-panes -t focus stability"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "list-panes -t tfs:$W1 should NOT change active window"
Psmux list-panes -t "tfs:$W1" | Out-Null
$afterLp = Get-ActiveWindow
if ($afterLp -eq $W0) { Write-Pass "active window still $W0 after list-panes -t tfs:$W1" } else { Write-Fail "active window changed to '$afterLp' (expected $W0)" }

Write-Test "list-panes -t tfs:$W2 should NOT change active window"
Psmux list-panes -t "tfs:$W2" | Out-Null
$afterLp2 = Get-ActiveWindow
if ($afterLp2 -eq $W0) { Write-Pass "active window still $W0 after list-panes -t tfs:$W2" } else { Write-Fail "active window changed to '$afterLp2' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 3: CAPTURE-PANE WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 3: capture-pane -t focus stability"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "capture-pane -t tfs:$W1 should NOT change active window"
Psmux capture-pane -t "tfs:$W1" -p | Out-Null
$afterCp = Get-ActiveWindow
if ($afterCp -eq $W0) { Write-Pass "active window still $W0 after capture-pane -t tfs:$W1" } else { Write-Fail "active window changed to '$afterCp' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 4: SEND-KEYS WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 4: send-keys -t focus stability"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "send-keys -t tfs:$W1 should NOT change active window"
Psmux send-keys -t "tfs:$W1" "echo hello" Enter
$afterSk = Get-ActiveWindow
if ($afterSk -eq $W0) { Write-Pass "active window still $W0 after send-keys -t tfs:$W1" } else { Write-Fail "active window changed to '$afterSk' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 5: SELECT-WINDOW WITH -t SHOULD SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 5: select-window -t SHOULD switch focus"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "select-window -t tfs:$W1 SHOULD change active window"
Psmux select-window -t "tfs:$W1"
Start-Sleep -Milliseconds 300
$afterSw = Get-ActiveWindow
if ($afterSw -eq $W1) { Write-Pass "active window changed to $W1" } else { Write-Fail "expected $W1, got '$afterSw'" }

Write-Test "select-window -t tfs:$W2 SHOULD change active window"
Psmux select-window -t "tfs:$W2"
Start-Sleep -Milliseconds 300
$afterSw2 = Get-ActiveWindow
if ($afterSw2 -eq $W2) { Write-Pass "active window changed to $W2" } else { Write-Fail "expected $W2, got '$afterSw2'" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 6: RAPID -t QUERIES (SIMULATING PLUGIN BEHAVIOR)
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 6: rapid -t queries (plugin simulation)"
Write-Host ("=" * 60)

# Go back to first window
Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 500

Write-Test "rapid alternating display-message -t queries"
# Simulate what psmux-resurrect does: query all windows rapidly
for ($i = 0; $i -lt 10; $i++) {
    Psmux display-message -t "tfs:$W1" -p "#{pane_current_path}" | Out-Null
    Psmux list-panes -t "tfs:$W2" | Out-Null
    Psmux display-message -t "tfs:$W0" -p "#{window_layout}" | Out-Null
}
Start-Sleep -Milliseconds 300
$afterRapid = Get-ActiveWindow
if ($afterRapid -eq $W0) {
    Write-Pass "active window still $W0 after 30 rapid -t queries across 3 windows"
} else {
    Write-Fail "active window changed to '$afterRapid' after rapid queries (expected $W0)"
}

# ════════════════════════════════════════════════════════════
# TEST GROUP 7: MIXED -t AND SELECT-WINDOW
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 7: mixed -t queries and select-window"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "select-window then display-message -t another window"
Psmux select-window -t "tfs:$W1"
Start-Sleep -Milliseconds 300
Psmux display-message -t "tfs:$W0" -p "#{pane_current_path}" | Out-Null
Psmux display-message -t "tfs:$W2" -p "#{pane_current_path}" | Out-Null
$afterMixed = Get-ActiveWindow
if ($afterMixed -eq $W1) {
    Write-Pass "active window stayed at $W1 (select-window target) despite -t queries"
} else {
    Write-Fail "expected $W1, got '$afterMixed'"
}

# ════════════════════════════════════════════════════════════
# TEST GROUP 8: SET-OPTION WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 8: set-option -t focus stability"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "set-option -t tfs:$W1 should NOT change active window"
Psmux set-option -t "tfs:$W1" automatic-rename off | Out-Null
$afterSo = Get-ActiveWindow
if ($afterSo -eq $W0) { Write-Pass "active window still $W0 after set-option -t tfs:$W1" } else { Write-Fail "active window changed to '$afterSo' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 9: SHOW-OPTIONS WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 9: show-options -t focus stability"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "show-options -t tfs:$W2 should NOT change active window"
Psmux show-options -t "tfs:$W2" | Out-Null
$afterShow = Get-ActiveWindow
if ($afterShow -eq $W0) { Write-Pass "active window still $W0 after show-options -t tfs:$W2" } else { Write-Fail "active window changed to '$afterShow' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# TEST GROUP 10: RENAME-WINDOW WITH -t SHOULD NOT SWITCH FOCUS
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST GROUP 10: rename-window -t focus stability"
Write-Host ("=" * 60)

Psmux select-window -t "tfs:$W0"
Start-Sleep -Milliseconds 300

Write-Test "rename-window -t tfs:$W2 should NOT change active window"
Psmux rename-window -t "tfs:$W2" "renamed_win" | Out-Null
$afterRn = Get-ActiveWindow
if ($afterRn -eq $W0) { Write-Pass "active window still $W0 after rename-window -t tfs:$W2" } else { Write-Fail "active window changed to '$afterRn' (expected $W0)" }

# ════════════════════════════════════════════════════════════
# CLEANUP
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)
Psmux kill-session -t tfs

# ════════════════════════════════════════════════════════════
# SUMMARY
# ════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TARGET FOCUS STABILITY TEST RESULTS"
Write-Host ("=" * 60)
Write-Host "Passed: $($script:TestsPassed) / $($script:TestsPassed + $script:TestsFailed)"
Write-Host "Failed: $($script:TestsFailed) / $($script:TestsPassed + $script:TestsFailed)"
if ($script:TestsFailed -eq 0) {
    Write-Host "ALL TESTS PASSED!" -ForegroundColor Green
} else {
    Write-Host "SOME TESTS FAILED!" -ForegroundColor Red
    exit 1
}
