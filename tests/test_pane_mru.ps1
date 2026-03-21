#!/usr/bin/env pwsh
###############################################################################
# test_pane_mru.ps1 — Regression tests for pane MRU focus ordering
#
# Issue #70: Directional navigation tie-break uses MRU recency
# Issue #71: Kill-pane focuses MRU pane, not DFS/leftmost
###############################################################################
$ErrorActionPreference = "Continue"

$pass = 0
$fail = 0

function Report {
    param([string]$Name, [bool]$Ok, [string]$Detail = "")
    if ($Ok) { $script:pass++; Write-Host "  [PASS] $Name  $Detail" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

function Kill-All {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Start-Sleep -Milliseconds 500
    Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue | Remove-Item -Force
    Get-ChildItem "$env:USERPROFILE\.psmux\*.key" -ErrorAction SilentlyContinue | Remove-Item -Force
    Start-Sleep -Milliseconds 300
}

function Get-ActivePaneIndex {
    param([string]$Session)
    $info = psmux display-message -t $Session -p '#{pane_index}' 2>$null
    if ($LASTEXITCODE -eq 0 -and $info -match '^\d+$') { return [int]$info }
    return -1
}

function Get-ActivePaneId {
    param([string]$Session)
    $info = psmux display-message -t $Session -p '#{pane_id}' 2>$null
    if ($LASTEXITCODE -eq 0) { return $info.Trim() }
    return ""
}

function Get-PaneCount {
    param([string]$Session)
    $info = psmux display-message -t $Session -p '#{window_panes}' 2>$null
    if ($LASTEXITCODE -eq 0 -and $info -match '^\d+$') { return [int]$info }
    return 0
}

Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Issues #70 & #71: Pane MRU focus ordering" -ForegroundColor Cyan
Write-Host "================================================================`n" -ForegroundColor Cyan

###############################################################################
# TEST 1: Kill active pane → focus goes to MRU pane (issue #71)
###############################################################################
Write-Host "--- TEST 1: Kill active pane → MRU focus ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "mru1" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create 3-pane layout: left | top-right / bottom-right
# Step 1: Split vertically → left + right (right is active)
psmux split-window -t "mru1" -h 2>$null
Start-Sleep -Milliseconds 1500

# Step 2: Split right horizontally → top-right + bottom-right (bottom-right active)
psmux split-window -t "mru1" -v 2>$null
Start-Sleep -Milliseconds 1500

# Now we have 3 panes: left(0), top-right(1), bottom-right(2)
# MRU order should be: bottom-right, top-right, left
# (because: left was created first, then right split to create top-right,
#  then bottom-right was split from top-right and got focus)

$paneCount = Get-PaneCount "mru1"
Report "3 panes created" ($paneCount -eq 3) "count=$paneCount"

# Navigate to top-right pane to make it MRU #1 (bottom-right becomes #2)
psmux select-pane -t "mru1" -U 2>$null
Start-Sleep -Milliseconds 500

$topRightId = Get-ActivePaneId "mru1"
Write-Host "  Top-right pane ID: $topRightId" -ForegroundColor Gray

# Navigate to left pane (now MRU: left, top-right, bottom-right)
psmux select-pane -t "mru1" -L 2>$null
Start-Sleep -Milliseconds 500

$leftId = Get-ActivePaneId "mru1"
Write-Host "  Left pane ID: $leftId" -ForegroundColor Gray

# Kill active (left) pane → should focus top-right (MRU #2, not bottom-right)
psmux kill-pane -t "mru1" 2>$null
Start-Sleep -Milliseconds 800

$afterKillId = Get-ActivePaneId "mru1"
Write-Host "  After kill, active pane ID: $afterKillId" -ForegroundColor Gray
Report "Kill active → MRU pane gets focus" ($afterKillId -eq $topRightId) "expected=$topRightId got=$afterKillId"

psmux kill-session -t "mru1" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 2: Kill non-active pane → focus stays on current pane (issue #71)
###############################################################################
Write-Host "`n--- TEST 2: Kill non-active pane → focus unchanged ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "mru2" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create 3-pane layout
psmux split-window -t "mru2" -h 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -t "mru2" -v 2>$null
Start-Sleep -Milliseconds 800

# Navigate to left pane
psmux select-pane -t "mru2" -L 2>$null
Start-Sleep -Milliseconds 500

$leftId2 = Get-ActivePaneId "mru2"
Write-Host "  Active (left) pane: $leftId2" -ForegroundColor Gray

# Kill pane 1 (top-right) while left is active
psmux kill-pane -t "mru2:.1" 2>$null
Start-Sleep -Milliseconds 800

$afterId2 = Get-ActivePaneId "mru2"
Write-Host "  After kill, active pane: $afterId2" -ForegroundColor Gray
Report "Kill non-active → focus unchanged" ($afterId2 -eq $leftId2) "expected=$leftId2 got=$afterId2"

psmux kill-session -t "mru2" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 3: Directional navigation MRU tie-break (issue #70)
#
# Layout: left | top-right / bottom-right
# After creating this layout, bottom-right is active.
# Navigate right (from left pane) — both right panes overlap.
# tmux picks the MRU winner among overlapping candidates.
###############################################################################
Write-Host "`n--- TEST 3: Directional nav MRU tie-break ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "mru3" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create layout: left(0) | top-right(1) / bottom-right(2)
psmux split-window -t "mru3" -h 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -t "mru3" -v 2>$null
Start-Sleep -Milliseconds 800

# bottom-right is active (MRU #1)
$brId = Get-ActivePaneId "mru3"
Write-Host "  Bottom-right pane ID: $brId" -ForegroundColor Gray

# Navigate to left: Ctrl+b Left wraps, press Right from left
# Actually, navigate right → wraps to left pane
psmux select-pane -t "mru3" -R 2>$null
Start-Sleep -Milliseconds 500
# Now on left pane
$leftId3 = Get-ActivePaneId "mru3"
Write-Host "  Left pane ID: $leftId3" -ForegroundColor Gray

# Navigate right again — should go to bottom-right (MRU winner)
psmux select-pane -t "mru3" -R 2>$null
Start-Sleep -Milliseconds 500

$rightId3 = Get-ActivePaneId "mru3"
Write-Host "  After Right from left, landed on: $rightId3" -ForegroundColor Gray
Report "Directional nav picks MRU pane" ($rightId3 -eq $brId) "expected=$brId got=$rightId3"

psmux kill-session -t "mru3" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 4: MRU tracks across multiple focus changes
###############################################################################
Write-Host "`n--- TEST 4: MRU tracks across multiple focus changes ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "mru4" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create 3 panes
psmux split-window -t "mru4" -h 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -t "mru4" -v 2>$null
Start-Sleep -Milliseconds 800

# bottom-right is active
# Navigate to top-right
psmux select-pane -t "mru4" -U 2>$null
Start-Sleep -Milliseconds 500
$trId4 = Get-ActivePaneId "mru4"
Write-Host "  Step1 -U → top-right: $trId4" -ForegroundColor Gray

# Navigate to left
psmux select-pane -t "mru4" -L 2>$null
Start-Sleep -Milliseconds 500
$lId4 = Get-ActivePaneId "mru4"
Write-Host "  Step2 -L → left: $lId4" -ForegroundColor Gray

# Navigate to top-right again
psmux select-pane -t "mru4" -R 2>$null
Start-Sleep -Milliseconds 500
$step3 = Get-ActivePaneId "mru4"
Write-Host "  Step3 -R → should be top-right: $step3" -ForegroundColor Gray

# Navigate to bottom-right
psmux select-pane -t "mru4" -D 2>$null
Start-Sleep -Milliseconds 500
$brId4 = Get-ActivePaneId "mru4"
Write-Host "  Step4 -D → bottom-right: $brId4" -ForegroundColor Gray

$countBefore = Get-PaneCount "mru4"
Write-Host "  Pane count before kill: $countBefore" -ForegroundColor Gray

# Kill bottom-right (active) → should go to top-right (MRU #2, most recently visited before bottom-right)
psmux kill-pane -t "mru4" 2>$null
Start-Sleep -Milliseconds 800

$countAfter = Get-PaneCount "mru4"
$afterId4 = Get-ActivePaneId "mru4"
Write-Host "  Pane count after kill: $countAfter" -ForegroundColor Gray
Write-Host "  After killing bottom-right, active: $afterId4" -ForegroundColor Gray
# Expected: step3 (top-right, the pane we visited right before bottom-right)
Report "Kill after multiple focus changes → correct MRU" ($afterId4 -eq $step3) "expected=$step3 got=$afterId4"

psmux kill-session -t "mru4" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 5: Original issue #70 exact repro
#
# 1. Start fresh session
# 2. Ctrl+b % (vertical split) → left + right, right active
# 3. Ctrl+b " (horizontal split of right) → left, top-right, bottom-right active
# 4. Ctrl+b Right (wraps to left)
# 5. Ctrl+b Right again
# Expected: bottom-right (MRU winner)
###############################################################################
Write-Host "`n--- TEST 5: Issue #70 exact reproduction ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "mru5" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Step 2: vertical split (%) → left + right, right is active
psmux split-window -t "mru5" -h 2>$null
Start-Sleep -Milliseconds 800

# Step 3: horizontal split (") of right → top-right + bottom-right, bottom-right active
psmux split-window -t "mru5" -v 2>$null
Start-Sleep -Milliseconds 800

$brId5 = Get-ActivePaneId "mru5"
Write-Host "  After splits, active (bottom-right): $brId5" -ForegroundColor Gray

# Step 4: navigate right (wraps to left)
psmux select-pane -t "mru5" -R 2>$null
Start-Sleep -Milliseconds 500
$leftCheck = Get-ActivePaneId "mru5"
Write-Host "  After Right (wrap), active (left): $leftCheck" -ForegroundColor Gray

# Step 5: navigate right again → should go to bottom-right (MRU)
psmux select-pane -t "mru5" -R 2>$null
Start-Sleep -Milliseconds 500

$result5 = Get-ActivePaneId "mru5"
Write-Host "  After Right again, active: $result5" -ForegroundColor Gray
Report "Issue #70 exact repro: Right picks bottom-right (MRU)" ($result5 -eq $brId5) "expected=$brId5 got=$result5"

psmux kill-session -t "mru5" 2>$null
Kill-All

###############################################################################
# SUMMARY
###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "================================================================`n" -ForegroundColor Cyan

if ($fail -gt 0) { exit 1 }
exit 0
