#!/usr/bin/env pwsh
###############################################################################
# test_issue70_mouse_mru_and_detached.ps1
#
# Tests for issue #70 remaining divergences:
#   1. Mouse-click focus not updating MRU for directional nav
#   2. split-window -d tie-break when multiple candidates were never focused
#
# These tests validate tmux-parity for MRU-based directional navigation
# across different focus-change paths.
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
Write-Host " Issue #70: Mouse MRU + Detached Split Tie-Break" -ForegroundColor Cyan
Write-Host "================================================================`n" -ForegroundColor Cyan

###############################################################################
# TEST 1: select-pane -t 0:0.N updates MRU for directional nav
#
# This is spooki44's exact repro from the issue comment.
# Layout: left(0) | top-right(1) / bottom-right(2)
# Focus sequence: pane 1, then pane 0
# Navigate Left from pane 0 → should pick pane 1 (MRU), not pane 2
###############################################################################
Write-Host "--- TEST 1: select-pane -t index updates MRU ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "t1" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create 3-pane layout: left(0) | top-right(1) / bottom-right(2)
psmux split-window -h -t "t1:0.0" 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -v -t "t1:0.1" 2>$null
Start-Sleep -Milliseconds 800

$cnt1 = Get-PaneCount "t1"
Report "Test1: 3 panes created" ($cnt1 -eq 3) "count=$cnt1"

# Focus pane 1 (top-right) by index
psmux select-pane -t "t1:0.1" 2>$null
Start-Sleep -Milliseconds 500
# Focus pane 0 (left) by index
psmux select-pane -t "t1:0.0" 2>$null
Start-Sleep -Milliseconds 500

# MRU should be: [0, 1, 2] (0 most recent, 1 second, 2 least)
# Navigate Right from pane 0: both pane 1 and 2 overlap, MRU picks pane 1
psmux select-pane -R -t "t1:0" 2>$null
Start-Sleep -Milliseconds 500

$result1 = Get-ActivePaneIndex "t1"
Report "Test1: Right from 0 picks MRU pane 1 (not 2)" ($result1 -eq 1) "expected=1 got=$result1"

psmux kill-session -t "t1" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 2: split-window -d tie-break — tmux uses pane_index for unvisited panes
#
# This is spooki44's exact repro for the detached split divergence.
# Create layout with detached splits, kill active, then navigate.
# When no candidate was ever focused, tmux picks lowest pane_index.
###############################################################################
Write-Host "`n--- TEST 2: split-window -d tie-break by pane_index ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "t2" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create the exact layout from the issue:
# split-window -h creates %2 to the right of %1
psmux split-window -h -t "t2:0.0" 2>$null
Start-Sleep -Milliseconds 800

# split-window -v -d: detached vertical splits of pane at index 1
psmux split-window -v -d -t "t2:0.1" 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -v -d -t "t2:0.1" 2>$null
Start-Sleep -Milliseconds 800

$cnt2 = Get-PaneCount "t2"
Report "Test2: 4 panes created" ($cnt2 -eq 4) "count=$cnt2"

# Layout should be:
# +------------------+------------------+
# | 0 (%1)           | 1 (%2)           |
# |                  +------------------+
# |                  | 2 (%4)           |
# |                  +------------------+
# |                  | 3 (%3)           |
# +------------------+------------------+

# Kill pane at index 1 (the one that was focused via split-window -h)
psmux kill-pane -t "t2:0.1" 2>$null
Start-Sleep -Milliseconds 800

$cnt2after = Get-PaneCount "t2"
Report "Test2: 3 panes after kill" ($cnt2after -eq 3) "count=$cnt2after"

# Now layout:
# +------------------+------------------+
# | 0 (%1)           | 1 (%4)           |
# |                  +------------------+
# |                  | 2 (%3)           |
# +------------------+------------------+
# Pane 1 (%4) and Pane 2 (%3) were never focused (created -d).
# tmux tie-break picks lowest pane_index → pane 1

# Navigate Right from pane 0
psmux select-pane -t "t2:0.0" 2>$null
Start-Sleep -Milliseconds 500
psmux select-pane -R -t "t2:0" 2>$null
Start-Sleep -Milliseconds 500

$result2 = Get-ActivePaneIndex "t2"
Report "Test2: Right from 0 picks pane_index 1 (not 2)" ($result2 -eq 1) "expected=1 got=$result2"

psmux kill-session -t "t2" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 3: Directional nav MRU works with select-pane -t (by-index focus)
#
# Verify MRU is properly updated when focus changes via select-pane -t.
# Layout: left(0) | top-right(1) / bottom-right(2)
# Focus 2, then 0. Navigate Right → should pick 2 (MRU), not 1.
###############################################################################
Write-Host "`n--- TEST 3: MRU via select-pane -t picks correct pane ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "t3" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

psmux split-window -h -t "t3:0.0" 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -v -t "t3:0.1" 2>$null
Start-Sleep -Milliseconds 800

# Focus bottom-right (2), then left (0)
psmux select-pane -t "t3:0.2" 2>$null
Start-Sleep -Milliseconds 500
psmux select-pane -t "t3:0.0" 2>$null
Start-Sleep -Milliseconds 500

# MRU: [0, 2, 1]
# Navigate Right from 0 → should pick 2 (MRU winner)
psmux select-pane -R -t "t3:0" 2>$null
Start-Sleep -Milliseconds 500

$result3 = Get-ActivePaneIndex "t3"
Report "Test3: Right picks MRU pane 2 (not 1)" ($result3 -eq 2) "expected=2 got=$result3"

psmux kill-session -t "t3" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 4: Detached panes with 5-pane layout (3 stacked right)
#
# Layout: left(0) | three stacked right panes (1, 2, 3)
# All right panes created with -d (never focused).
# Navigate Right from 0 → should pick lowest pane_index (1).
###############################################################################
Write-Host "`n--- TEST 4: 5-pane detached stacked right ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "t4" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create right pane (focused, not detached)
psmux split-window -h -t "t4:0.0" 2>$null
Start-Sleep -Milliseconds 800

# Create two more detached vertical splits of the right pane
psmux split-window -v -d -t "t4:0.1" 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -v -d -t "t4:0.1" 2>$null
Start-Sleep -Milliseconds 800

$cnt4 = Get-PaneCount "t4"
Report "Test4: 4 panes created" ($cnt4 -eq 4) "count=$cnt4"

# Kill the originally-focused right pane (index 1)
psmux kill-pane -t "t4:0.1" 2>$null
Start-Sleep -Milliseconds 800

$cnt4after = Get-PaneCount "t4"
# Navigate to pane 0 first, then Right
psmux select-pane -t "t4:0.0" 2>$null
Start-Sleep -Milliseconds 500
psmux select-pane -R -t "t4:0" 2>$null
Start-Sleep -Milliseconds 500

$result4 = Get-ActivePaneIndex "t4"
Report "Test4: Right picks lowest pane_index among unvisited" ($result4 -eq 1) "expected=1 got=$result4"

psmux kill-session -t "t4" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 5: MRU via directional nav still works (regression check)
#
# Layout: left(0) | top-right(1) / bottom-right(2)
# Use ONLY directional navigation to build MRU.
# Verify the MRU-based tie-break works correctly.
###############################################################################
Write-Host "`n--- TEST 5: MRU via directional nav (regression) ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "t5" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

psmux split-window -h -t "t5:0.0" 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -v -t "t5:0.1" 2>$null
Start-Sleep -Milliseconds 800

# Active = bottom-right (2). MRU: [2, 1, 0]
# Navigate Right wraps to left
psmux select-pane -R -t "t5:0" 2>$null
Start-Sleep -Milliseconds 500
# Now on left (0). MRU: [0, 2, 1]

# Navigate Right → should go to bottom-right (2, MRU winner)
psmux select-pane -R -t "t5:0" 2>$null
Start-Sleep -Milliseconds 500

$result5 = Get-ActivePaneIndex "t5"
Report "Test5: Directional MRU still works (original issue)" ($result5 -eq 2) "expected=2 got=$result5"

psmux kill-session -t "t5" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 6: Focused pane wins over detached panes in MRU
#
# Layout: left(0) | 3 right panes
# Focus one right pane, then navigate away and back.
# Should pick the focused one, not the lower pane_index.
###############################################################################
Write-Host "`n--- TEST 6: Focused pane wins over detached ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "t6" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

# Create right pane
psmux split-window -h -t "t6:0.0" 2>$null
Start-Sleep -Milliseconds 800

# Detached split twice
psmux split-window -v -d -t "t6:0.1" 2>$null
Start-Sleep -Milliseconds 800
psmux split-window -v -d -t "t6:0.1" 2>$null
Start-Sleep -Milliseconds 800

$cnt6 = Get-PaneCount "t6"
Report "Test6: 4 panes" ($cnt6 -eq 4) "count=$cnt6"

# Layout: 0(left) | 1(top-right) / 2(mid-right) / 3(bottom-right)
# Pane 1 was the original split target (focused via split-window -h).
# Panes 2, 3 are detached (never focused).

# Focus pane 3 (bottom-right) using select-pane -t
psmux select-pane -t "t6:0.3" 2>$null
Start-Sleep -Milliseconds 500

# Go to left pane
psmux select-pane -t "t6:0.0" 2>$null
Start-Sleep -Milliseconds 500

# Navigate Right → should pick pane 3 (MRU winner, actually focused)
psmux select-pane -R -t "t6:0" 2>$null
Start-Sleep -Milliseconds 500

$result6 = Get-ActivePaneIndex "t6"
Report "Test6: Focused pane 3 wins over unfocused 1,2" ($result6 -eq 3) "expected=3 got=$result6"

psmux kill-session -t "t6" 2>$null
Kill-All

###############################################################################
# SUMMARY
###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "================================================================`n" -ForegroundColor Cyan

if ($fail -gt 0) { exit 1 }
exit 0
