#!/usr/bin/env pwsh
# Debug test to narrow down split-window -t focus issue
$ErrorActionPreference = "Continue"

function Test-Check {
    param([string]$Name, [string]$Expected, [string]$Actual)
    $trimExpected = $Expected.Trim()
    $trimActual = $Actual.Trim()
    if ($trimExpected -eq $trimActual) {
        Write-Host "  PASS: $Name (got '$trimActual')" -ForegroundColor Green
    } else {
        Write-Host "  FAIL: $Name - expected '$trimExpected', got '$trimActual'" -ForegroundColor Red
    }
}

psmux kill-server 2>$null
Start-Sleep -Milliseconds 500

# ─── Test A: Does select-pane -t work to change focus? ───
Write-Host "`n=== Test A: Does select-pane -t :0.0 properly change focus? ===" -ForegroundColor Cyan
psmux new-session -d -s testA -x 200 -y 50
Start-Sleep -Milliseconds 1500
psmux split-window -h -t testA
Start-Sleep -Milliseconds 1500

$r = psmux display-message -t testA -p '#{pane_index}'
Test-Check "After split, active is pane 1" "1" $r

# Now explicitly select pane 0
psmux select-pane -t testA:0.0
Start-Sleep -Milliseconds 500

$r = psmux display-message -t testA -p '#{pane_index}'
Test-Check "After select-pane -t :0.0, active is pane 0" "0" $r

# Select pane 1 back
psmux select-pane -t testA:0.1
Start-Sleep -Milliseconds 500

$r = psmux display-message -t testA -p '#{pane_index}'
Test-Check "After select-pane -t :0.1, active is pane 1" "1" $r

psmux kill-session -t testA 2>$null
Start-Sleep -Milliseconds 500

# ─── Test B: Does the -t on display-message itself change focus? ───
Write-Host "`n=== Test B: Does display-message -t with pane spec change focus? ===" -ForegroundColor Cyan
psmux new-session -d -s testB -x 200 -y 50
Start-Sleep -Milliseconds 1500
psmux split-window -h -t testB
Start-Sleep -Milliseconds 1500

# Active should be pane 1 after split
$r = psmux display-message -t testB -p '#{pane_index}'
Test-Check "Active is pane 1" "1" $r

# Now use display-message targeting pane 0 to see if IT changes focus
$r = psmux display-message -t testB:0.0 -p '#{pane_index}'
Write-Host "  INFO: display-message -t testB:0.0 returned: '$($r.Trim())'" -ForegroundColor Yellow

# Check if focus changed persistently
$r2 = psmux display-message -t testB -p '#{pane_index}'
Write-Host "  INFO: After display-message -t :0.0, current active is: '$($r2.Trim())'" -ForegroundColor Yellow

psmux kill-session -t testB 2>$null
Start-Sleep -Milliseconds 500

# ─── Test C: Split targeting non-active pane — step by step ───
Write-Host "`n=== Test C: Step-by-step targeting pane 0 for split ===" -ForegroundColor Cyan
psmux new-session -d -s testC -x 200 -y 50
Start-Sleep -Milliseconds 1500
psmux split-window -h -t testC
Start-Sleep -Milliseconds 1500

$r = psmux display-message -t testC -p '#{pane_index}'
Test-Check "Step C1: Active pane is 1" "1" $r

# MANUALLY select pane 0, then split
psmux select-pane -t testC:0.0
Start-Sleep -Milliseconds 500

$r = psmux display-message -t testC -p '#{pane_index}'
Test-Check "Step C2: After select, active pane is 0" "0" $r

psmux split-window -v -t testC
Start-Sleep -Milliseconds 1500

$count = psmux display-message -t testC -p '#{window_panes}'
Test-Check "Step C3: Pane count is 3" "3" $count

$r = psmux display-message -t testC -p '#{pane_index}'
Test-Check "Step C4: After split on selected pane 0, active is 1 (new pane)" "1" $r

psmux kill-session -t testC 2>$null
Start-Sleep -Milliseconds 500

# ─── Test D: Single command split-window -t :0.0 ───
Write-Host "`n=== Test D: Direct split-window -v -t testD:0.0 ===" -ForegroundColor Cyan
psmux new-session -d -s testD -x 200 -y 50
Start-Sleep -Milliseconds 1500
psmux split-window -h -t testD
Start-Sleep -Milliseconds 1500

$r = psmux display-message -t testD -p '#{pane_index}'
Test-Check "Step D1: Active pane is 1" "1" $r

# Split targeting pane 0 in one command
psmux split-window -v -t testD:0.0
Start-Sleep -Milliseconds 1500

$count = psmux display-message -t testD -p '#{window_panes}'
Test-Check "Step D2: Pane count is 3" "3" $count

$r = psmux display-message -t testD -p '#{pane_index}'
Write-Host "  INFO: After split-window -v -t testD:0.0, active pane index: '$($r.Trim())'" -ForegroundColor Yellow
Test-Check "Step D3: Active should be 1 (new pane from splitting pane 0)" "1" $r

# Now list all panes to see the tree structure
$panes = psmux list-panes -t testD
Write-Host "  INFO: Pane layout:" -ForegroundColor Yellow
Write-Host "$panes" -ForegroundColor Gray

psmux kill-session -t testD 2>$null
Start-Sleep -Milliseconds 500

# ─── Test E: Without specific pane target, split goes to active pane ───
Write-Host "`n=== Test E: Verify split always goes to currently active pane ===" -ForegroundColor Cyan
psmux new-session -d -s testE -x 200 -y 50
Start-Sleep -Milliseconds 1500
psmux split-window -h -t testE
Start-Sleep -Milliseconds 1500

# Active should be pane 1
$r = psmux display-message -t testE -p '#{pane_index}'
Test-Check "Step E1: Active is 1" "1" $r

# Split WITHOUT pane target — should split pane 1 (the active pane)
psmux split-window -v -t testE
Start-Sleep -Milliseconds 1500

$r = psmux display-message -t testE -p '#{pane_index}'
Test-Check "Step E2: After splitting active pane 1, focus on 2 (new)" "2" $r

psmux kill-session -t testE 2>$null

psmux kill-server 2>$null
