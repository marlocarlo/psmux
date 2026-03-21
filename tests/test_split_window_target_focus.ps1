#!/usr/bin/env pwsh
# Test: split-window -t does not reliably focus the newly created pane
# Issue: https://github.com/psmux/psmux/issues/112
#
# tmux parity: split-window should move focus to the newly created pane
# regardless of whether -t is used to specify a target.

$ErrorActionPreference = "Continue"
$pass = 0
$fail = 0
$total = 0

function Test-Check {
    param([string]$Name, [string]$Expected, [string]$Actual)
    $script:total++
    $trimExpected = $Expected.Trim()
    $trimActual = $Actual.Trim()
    if ($trimExpected -eq $trimActual) {
        $script:pass++
        Write-Host "  PASS: $Name (got '$trimActual')" -ForegroundColor Green
    } else {
        $script:fail++
        Write-Host "  FAIL: $Name - expected '$trimExpected', got '$trimActual'" -ForegroundColor Red
    }
}

# Clean up any leftover sessions
Write-Host "`n=== Cleaning up old sessions ===" -ForegroundColor Cyan
psmux kill-server 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# TEST 1: Basic reproduction from issue #112
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=== TEST 1: Issue #112 exact reproduction ===" -ForegroundColor Cyan

psmux new-session -d -s test112 -x 200 -y 50
Start-Sleep -Milliseconds 1500

# Verify initial state: 1 pane, pane_index = 0
$result = psmux display-message -t test112 -p '#{pane_index}'
Test-Check "Initial active pane is 0" "0" $result

$paneCount = psmux display-message -t test112 -p '#{window_panes}'
Test-Check "Initial pane count is 1" "1" $paneCount

# First targeted split
psmux split-window -h -t test112
Start-Sleep -Milliseconds 1500

$paneCount = psmux display-message -t test112 -p '#{window_panes}'
Test-Check "After first split, pane count is 2" "2" $paneCount

$result = psmux display-message -t test112 -p '#{pane_index}'
Test-Check "After split-window -h -t, focus on pane 1 (new pane)" "1" $result

# Second targeted split
psmux split-window -v -t test112
Start-Sleep -Milliseconds 1500

$paneCount = psmux display-message -t test112 -p '#{window_panes}'
Test-Check "After second split, pane count is 3" "3" $paneCount

$result = psmux display-message -t test112 -p '#{pane_index}'
Test-Check "After split-window -v -t, focus on pane 2 (new pane)" "2" $result

psmux kill-session -t test112 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# TEST 2: Split-window WITHOUT -t (should always work — baseline)
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=== TEST 2: split-window WITHOUT -t (baseline) ===" -ForegroundColor Cyan

psmux new-session -d -s baseline -x 200 -y 50
Start-Sleep -Milliseconds 1500

$result = psmux display-message -t baseline -p '#{pane_index}'
Test-Check "Baseline: initial pane 0" "0" $result

# Split without target (from inside session context)
psmux split-window -h -t baseline
Start-Sleep -Milliseconds 1500

$result = psmux display-message -t baseline -p '#{pane_index}'
Test-Check "Baseline: after horizontal split, pane 1" "1" $result

psmux split-window -v -t baseline
Start-Sleep -Milliseconds 1500

$result = psmux display-message -t baseline -p '#{pane_index}'
Test-Check "Baseline: after vertical split, pane 2" "2" $result

psmux kill-session -t baseline 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# TEST 3: Repeated targeted splits (stress test for race condition)
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=== TEST 3: Repeated targeted splits (race condition stress) ===" -ForegroundColor Cyan

psmux new-session -d -s stress -x 200 -y 50
Start-Sleep -Milliseconds 1500

for ($i = 1; $i -le 5; $i++) {
    # Alternate h/v splits to avoid running out of space in one dimension
    if ($i % 2 -eq 1) { $dir = "-h" } else { $dir = "-v" }
    psmux split-window $dir -t stress
    Start-Sleep -Milliseconds 1000
    $result = psmux display-message -t stress -p '#{pane_index}'
    Test-Check "Stress split ${i}: focus on pane ${i}" "$i" $result
}

psmux kill-session -t stress 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# TEST 4: Split targeting specific pane by index (non-active pane)
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=== TEST 4: Split targeting specific pane ===" -ForegroundColor Cyan

psmux new-session -d -s targetpane -x 200 -y 50
Start-Sleep -Milliseconds 1500

# Create initial split: pane 0 and pane 1, focus on pane 1
psmux split-window -h -t targetpane
Start-Sleep -Milliseconds 1500

$result = psmux display-message -t targetpane -p '#{pane_index}'
Test-Check "Target pane: after first split, active is 1" "1" $result

# Now split pane 0 specifically (non-active pane) using :0.0 target
psmux split-window -v -t targetpane:0.0
Start-Sleep -Milliseconds 1500

$paneCount = psmux display-message -t targetpane -p '#{window_panes}'
Test-Check "Target pane: after targeting pane 0, count is 3" "3" $paneCount

# After splitting pane 0, focus should move to the NEW pane (pane 1 in new layout)
# Tree after: Split(H, [Split(V, [Pane0, Pane2]), Pane1])
# Index order: Pane0=0, Pane2=1, Pane1=2
# The new pane (Pane2) is at index 1, so focus should be on 1
$result = psmux display-message -t targetpane -p '#{pane_index}'
Test-Check "Target pane: after split-window -v -t :0.0, focus moved to new pane (idx 1)" "1" $result

psmux kill-session -t targetpane 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# TEST 5: Rapid successive targeted splits (minimal delays)
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=== TEST 5: Rapid successive targeted splits ===" -ForegroundColor Cyan

psmux new-session -d -s rapid -x 200 -y 50
Start-Sleep -Milliseconds 1500

psmux split-window -h -t rapid
Start-Sleep -Milliseconds 500
psmux split-window -v -t rapid
Start-Sleep -Milliseconds 500
psmux split-window -h -t rapid
Start-Sleep -Milliseconds 500

$paneCount = psmux display-message -t rapid -p '#{window_panes}'
Test-Check "Rapid: after 3 splits, pane count is 4" "4" $paneCount

$result = psmux display-message -t rapid -p '#{pane_index}'
Test-Check "Rapid: after 3 rapid splits, focus on pane 3 (newest)" "3" $result

psmux kill-session -t rapid 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# TEST 6: Split with -d (detached) — focus should NOT move
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=== TEST 6: split-window -d (detached focus) ===" -ForegroundColor Cyan

psmux new-session -d -s detachtest -x 200 -y 50
Start-Sleep -Milliseconds 1500

$result = psmux display-message -t detachtest -p '#{pane_index}'
Test-Check "Detach: initial pane 0" "0" $result

psmux split-window -h -d -t detachtest
Start-Sleep -Milliseconds 1500

$paneCount = psmux display-message -t detachtest -p '#{window_panes}'
Test-Check "Detach: pane count is 2" "2" $paneCount

$result = psmux display-message -t detachtest -p '#{pane_index}'
Test-Check "Detach: after split-window -h -d, focus stays on pane 0" "0" $result

psmux kill-session -t detachtest 2>$null
Start-Sleep -Milliseconds 500

# ─────────────────────────────────────────────────────────────────────────────
# SUMMARY
# ─────────────────────────────────────────────────────────────────────────────
Write-Host "`n=============================" -ForegroundColor Cyan
Write-Host "RESULTS: $pass passed, $fail failed out of $total tests" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "=============================" -ForegroundColor Cyan

# Clean up
psmux kill-server 2>$null

exit $fail
