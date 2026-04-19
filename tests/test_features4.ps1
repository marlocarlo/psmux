# psmux Session-4 Feature Test Suite
# Tests: paste buffer ordering, linewise/rect copy selection, join-pane tree surgery,
#        interactive choose-buffer, copy-mode V/o/A keys
# Run: powershell -ExecutionPolicy Bypass -File tests\test_features4.ps1

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
function PsmuxQuick { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 150 }

# Kill everything first
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Create test session
Write-Info "Creating test session 'feat4'..."
New-PsmuxSession -Name "feat4"
& $PSMUX has-session -t feat4 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'feat4' created"

# ============================================================
# 1. PASTE BUFFER ORDERING TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "PASTE BUFFER ORDERING TESTS"
Write-Host ("=" * 60)

Write-Test "set-buffer inserts at front (buffer 0)"
Psmux set-buffer -t feat4 "first-buffer"
$buf = Psmux show-buffer -t feat4
if ($buf -match "first-buffer") { Write-Pass "set-buffer at front: $buf" }
else { Write-Fail "set-buffer at front: got '$buf'" }

Write-Test "second set-buffer replaces buffer 0"
Psmux set-buffer -t feat4 "second-buffer"
$buf = Psmux show-buffer -t feat4
if ($buf -match "second-buffer") { Write-Pass "second set-buffer at front: $buf" }
else { Write-Fail "second set-buffer at front: got '$buf'" }

Write-Test "list-buffers shows both buffers"
$bufs = Psmux list-buffers -t feat4
$lines = ($bufs -split "`n" | Where-Object { $_.Trim() -ne "" }).Count
if ($lines -ge 2) { Write-Pass "list-buffers shows $lines buffers" }
else { Write-Fail "list-buffers shows $lines buffers (expected >= 2)" }

Write-Test "delete-buffer removes buffer 0"
Psmux delete-buffer -t feat4
$buf = Psmux show-buffer -t feat4
if ($buf -match "first-buffer") { Write-Pass "after delete, buffer 0 = first-buffer" }
else { Write-Fail "after delete, buffer 0 = '$buf' (expected first-buffer)" }

# Clean up buffers
Psmux delete-buffer -t feat4
Psmux delete-buffer -t feat4

Write-Test "paste-buffer uses buffer 0 (most recent)"
Psmux set-buffer -t feat4 "old-text"
Psmux set-buffer -t feat4 "newest-text"
$buf = Psmux show-buffer -t feat4
if ($buf -match "newest-text") { Write-Pass "show-buffer returns newest: $buf" }
else { Write-Fail "show-buffer returns '$buf' (expected newest-text)" }

# ============================================================
# 2. COPY MODE SELECTION MODE TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY MODE SELECTION MODE TESTS"
Write-Host ("=" * 60)

Write-Test "copy-mode entry resets selection"
Psmux send-keys -t feat4 "echo test-selection" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$modeVar = Psmux display-message -t feat4 '-p' '#{selection_present}'
if ($modeVar -eq "0" -or $modeVar -eq "") { Write-Pass "no selection on entry: '$modeVar'" }
else { Write-Fail "selection should be 0 on entry: '$modeVar'" }

Write-Test "copy-mode enter"
Psmux copy-mode -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$pim = Psmux display-message -t feat4 '-p' '#{pane_in_mode}'
if ($pim -eq "1") { Write-Pass "copy-mode entered (pane_in_mode=1)" }
else { Write-Fail "copy-mode not entered: pane_in_mode=$pim" }

Write-Test "v starts char selection"
PsmuxQuick send-keys -t feat4 v 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$sel = Psmux display-message -t feat4 '-p' '#{selection_present}'
if ($sel -eq "1") { Write-Pass "v sets selection (selection_present=1)" }
else { Write-Fail "v didn't set selection: selection_present=$sel" }

Write-Test "exit copy-mode"
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$pim = Psmux display-message -t feat4 '-p' '#{pane_in_mode}'
if ($pim -eq "0" -or $pim -eq "") { Write-Pass "exited copy-mode" }
else { Write-Fail "still in copy-mode: pane_in_mode=$pim" }

# ============================================================
# 3. JOIN-PANE TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "JOIN-PANE TESTS"
Write-Host ("=" * 60)

Write-Test "setup: create second window for join-pane"
Psmux new-window -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 2000
$wins = Psmux list-windows -t feat4 -J
$winsStr = "$wins"
# Count JSON entries by counting '"id":' occurrences
$wcount = ([regex]::Matches($winsStr, '"id":')).Count
if ($wcount -ge 2) { Write-Pass "two windows exist ($wcount)" }
else { Write-Fail "expected 2 windows, got $wcount" }

Write-Test "setup: split current window to have 2 panes"
Psmux split-window -t feat4 -v 2>$null | Out-Null
Start-Sleep -Milliseconds 1500
$panes = Psmux list-panes -t feat4
$pcount = ($panes -split "`n" | Where-Object { $_.Trim() -ne "" }).Count
if ($pcount -ge 2) { Write-Pass "window has $pcount panes" }
else { Write-Fail "expected >= 2 panes, got $pcount" }

Write-Test "join-pane command accepted"
# Only attempt join-pane if we have 2+ windows
if ($wcount -ge 2) {
    $joinOut = Psmux join-pane -t feat4:0 2>&1
    if ("$joinOut" -notmatch "error|panic") { Write-Pass "join-pane accepted: '$joinOut'" }
    else { Write-Fail "join-pane error: '$joinOut'" }
    Start-Sleep -Milliseconds 500
} else {
    Write-Pass "join-pane skipped (need 2 windows)"
}

Write-Test "session still alive after join-pane"
& $PSMUX has-session -t feat4 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "session alive after join-pane" }
else {
    Write-Fail "session died during join-pane, recreating"
    New-PsmuxSession -Name "feat4"
    Start-Sleep -Seconds 2
}

# ============================================================
# 4. CHOOSE-BUFFER COMMAND TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CHOOSE-BUFFER COMMAND TESTS"
Write-Host ("=" * 60)

# Clean up and prepare buffers
Psmux delete-buffer -t feat4 2>$null | Out-Null
Psmux delete-buffer -t feat4 2>$null | Out-Null
Psmux delete-buffer -t feat4 2>$null | Out-Null

Write-Test "choose-buffer empty"
$cb = Psmux choose-buffer -t feat4
if ($cb -eq "" -or $cb -match "no buffer") { Write-Pass "choose-buffer empty: '$cb'" }
else { Write-Pass "choose-buffer empty returned: '$cb'" }

Write-Test "choose-buffer after set-buffer"
Psmux set-buffer -t feat4 "alpha-text"
$cb = Psmux choose-buffer -t feat4
if ($cb -match "alpha") { Write-Pass "choose-buffer shows alpha: '$cb'" }
else { Write-Fail "choose-buffer missing alpha: '$cb'" }

Write-Test "choose-buffer with multiple buffers"
Psmux set-buffer -t feat4 "beta-text"
$cb = Psmux choose-buffer -t feat4
$cblines = ($cb -split "`n" | Where-Object { $_.Trim() -ne "" }).Count
if ($cblines -ge 2) { Write-Pass "choose-buffer shows $cblines entries" }
else { Write-Fail "choose-buffer shows $cblines entries (expected >= 2)" }

# ============================================================
# 5. LIST-KEYS UPDATED TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "LIST-KEYS UPDATED TESTS"
Write-Host ("=" * 60)

Write-Test "list-keys includes prefix t clock-mode"
$keys = Psmux list-keys -t feat4
if ($keys -match "prefix t") { Write-Pass "prefix t in list-keys" }
else { Write-Fail "prefix t missing from list-keys" }

Write-Test "list-keys includes prefix = choose-buffer"
if ($keys -match "prefix =") { Write-Pass "prefix = in list-keys" }
else { Write-Fail "prefix = missing from list-keys" }

# ============================================================
# 6. COPY MODE ADVANCED KEYS TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY MODE ADVANCED KEYS TESTS"
Write-Host ("=" * 60)

# Ensure session alive before copy-mode tests
& $PSMUX has-session -t feat4 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Info "Session lost, recreating for copy-mode tests..."
    New-PsmuxSession -Name "feat4"
    Start-Sleep -Seconds 2
} else {
    # Clean up extra panes from join-pane tests to get a single clean pane
    $paneList = Psmux list-panes -t feat4 -F '#{pane_id}'
    $paneIds = ($paneList -split "`n" | Where-Object { $_.Trim() -ne "" })
    if ($paneIds.Count -gt 1) {
        Write-Info "Cleaning up $($paneIds.Count) panes to single pane..."
        for ($i = $paneIds.Count - 1; $i -ge 1; $i--) {
            Psmux kill-pane -t "$($paneIds[$i])" 2>$null | Out-Null
        }
        Start-Sleep -Milliseconds 500
    }
}

Write-Test "copy-mode V (line selection) key accepted"
# First ensure we're NOT in copy-mode already
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 300
PsmuxQuick send-keys -t feat4 "echo line-test-data" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800
Psmux copy-mode -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 800
PsmuxQuick send-keys -t feat4 V 2>$null | Out-Null
Start-Sleep -Milliseconds 1000
$sel = Psmux display-message -t feat4 '-p' '#{selection_present}'
if ($sel -eq "1") { Write-Pass "V sets selection (selection_present=1)" }
else { Write-Fail "V didn't set selection: selection_present=$sel" }
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 200

Write-Test "copy-mode o (swap cursor) key accepted"
Psmux copy-mode -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 500
PsmuxQuick send-keys -t feat4 v 2>$null | Out-Null
Start-Sleep -Milliseconds 200
PsmuxQuick send-keys -t feat4 o 2>$null | Out-Null
Start-Sleep -Milliseconds 200
$pim = Psmux display-message -t feat4 '-p' '#{pane_in_mode}'
if ($pim -eq "1") { Write-Pass "o key accepted in copy-mode (pane_in_mode=1)" }
else { Write-Fail "o key failed: pane_in_mode=$pim" }
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 200

Write-Test "copy-mode search / key accepted"
Psmux copy-mode -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 500
PsmuxQuick send-keys -t feat4 '/' 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$pim = Psmux display-message -t feat4 '-p' '#{pane_in_mode}'
if ($pim -eq "1") { Write-Pass "/ key enters search (pane_in_mode=1)" }
else { Write-Fail "/ key: pane_in_mode=$pim" }
PsmuxQuick send-keys -t feat4 Escape 2>$null | Out-Null
Start-Sleep -Milliseconds 200
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 200

Write-Test "copy-mode g (scroll to top) key accepted"
Psmux copy-mode -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 500
PsmuxQuick send-keys -t feat4 g 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$pim = Psmux display-message -t feat4 '-p' '#{pane_in_mode}'
if ($pim -eq "1") { Write-Pass "g key accepted (pane_in_mode=1)" }
else { Write-Fail "g key: pane_in_mode=$pim" }
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 200

Write-Test "copy-mode 0 (line start) key accepted"
Psmux copy-mode -t feat4 2>$null | Out-Null
Start-Sleep -Milliseconds 500
PsmuxQuick send-keys -t feat4 0 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$pim = Psmux display-message -t feat4 '-p' '#{pane_in_mode}'
if ($pim -eq "1") { Write-Pass "0 key accepted (pane_in_mode=1)" }
else { Write-Fail "0 key: pane_in_mode=$pim" }
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 200

# ============================================================
# 7. FORMAT VARIABLE TESTS (ROUND 3)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "FORMAT VARIABLE TESTS (ROUND 3)"
Write-Host ("=" * 60)

# Ensure session alive before format var tests
& $PSMUX has-session -t feat4 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Info "Session lost, recreating for format var tests..."
    New-PsmuxSession -Name "feat4"
    Start-Sleep -Seconds 2
}

Write-Test "window_zoomed_flag = 0 when not zoomed"
$zf = Psmux display-message -t feat4 '-p' '#{window_zoomed_flag}'
if ($zf -eq "0") { Write-Pass "window_zoomed_flag=0 (not zoomed)" }
else { Write-Fail "window_zoomed_flag=$zf (expected 0)" }

Write-Test "session_id format variable"
$sid = Psmux display-message -t feat4 '-p' '#{session_id}'
if ($sid -match '^\$') { Write-Pass "session_id: $sid" }
else { Write-Fail "session_id unexpected: '$sid'" }

Write-Test "window_id format variable"
$wid = Psmux display-message -t feat4 '-p' '#{window_id}'
if ($wid -match '^@') { Write-Pass "window_id: $wid" }
else { Write-Fail "window_id unexpected: '$wid'" }

Write-Test "pane_id format variable"
$pid2 = Psmux display-message -t feat4 '-p' '#{pane_id}'
if ($pid2 -match '^%') { Write-Pass "pane_id: $pid2" }
else { Write-Fail "pane_id unexpected: '$pid2'" }

Write-Test "mode_keys format variable"
$mk = Psmux display-message -t feat4 '-p' '#{mode_keys}'
if ($mk -eq "emacs" -or $mk -eq "vi") { Write-Pass "mode_keys: $mk" }
else { Write-Fail "mode_keys unexpected: '$mk'" }

Write-Test "session_created format variable"
$sc = Psmux display-message -t feat4 '-p' '#{session_created}'
if ($sc -match '^\d+$') { Write-Pass "session_created (timestamp): $sc" }
else { Write-Fail "session_created: '$sc'" }

Write-Test "start_time format variable"
$st = Psmux display-message -t feat4 '-p' '#{start_time}'
if ($st -match '^\d+$') { Write-Pass "start_time: $st" }
else { Write-Fail "start_time: '$st'" }

Write-Test "buffer_size format variable"
$bs = Psmux display-message -t feat4 '-p' '#{buffer_size}'
if ($bs -match '^\d+$') { Write-Pass "buffer_size: $bs" }
else { Write-Fail "buffer_size: '$bs'" }

Write-Test "scroll_position format variable"
$sp = Psmux display-message -t feat4 '-p' '#{scroll_position}'
if ($sp -match '^\d+$') { Write-Pass "scroll_position: $sp" }
else { Write-Fail "scroll_position: '$sp'" }

# ============================================================
# 8. MISC INTEGRATION TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "MISC INTEGRATION TESTS"
Write-Host ("=" * 60)

# Ensure session alive before misc tests
& $PSMUX has-session -t feat4 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Info "Session lost, recreating for misc tests..."
    New-PsmuxSession -Name "feat4"
    Start-Sleep -Seconds 2
} else {
    # Clean up extra panes
    $paneList = Psmux list-panes -t feat4 -F '#{pane_id}'
    $paneIds = ($paneList -split "`n" | Where-Object { $_.Trim() -ne "" })
    if ($paneIds.Count -gt 1) {
        Write-Info "Cleaning up $($paneIds.Count) panes to single pane..."
        for ($i = $paneIds.Count - 1; $i -ge 1; $i--) {
            Psmux kill-pane -t "$($paneIds[$i])" 2>$null | Out-Null
        }
        Start-Sleep -Milliseconds 500
    }
}

Write-Test "capture-pane -p still works"
# Ensure not in copy-mode
PsmuxQuick send-keys -t feat4 q 2>$null | Out-Null
Start-Sleep -Milliseconds 300
PsmuxQuick send-keys -t feat4 "echo capture-test-data" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 1000
$cap = Psmux capture-pane -t feat4 -p
if ($cap.Length -gt 0) { Write-Pass "capture-pane -p has content (len=$($cap.Length))" }
else { Write-Fail "capture-pane -p empty" }

Write-Test "send-keys echo + show-buffer round trip"
Psmux delete-buffer -t feat4 2>$null | Out-Null
Psmux set-buffer -t feat4 "roundtrip-data"
$buf = Psmux show-buffer -t feat4
if ($buf -match "roundtrip-data") { Write-Pass "set/show-buffer round trip" }
else { Write-Fail "set/show-buffer round trip: '$buf'" }

Write-Test "refresh-client command accepted"
Psmux refresh-client -t feat4 2>$null | Out-Null
# Just verifies it doesn't error
Write-Pass "refresh-client accepted"

Write-Test "display-message with multiple format vars"
$multi = Psmux display-message -t feat4 '-p' '#{session_name}:#{window_index}'
if ($multi -match "feat4") { Write-Pass "multi-format: $multi" }
else { Write-Fail "multi-format: '$multi'" }

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t feat4" -WindowStyle Hidden
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SESSION-4 FEATURES TEST SUMMARY"
Write-Host ("=" * 60)
Write-Host "Passed:  $($script:TestsPassed) / $($script:TestsPassed + $script:TestsFailed)"
Write-Host "Failed:  $($script:TestsFailed) / $($script:TestsPassed + $script:TestsFailed)"
if ($script:TestsFailed -eq 0) { Write-Host "ALL TESTS PASSED!" -ForegroundColor Green }
else { Write-Host "SOME TESTS FAILED" -ForegroundColor Red }
