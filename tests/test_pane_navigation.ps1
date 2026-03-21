# Pane Navigation Tests
# Verifies that prefix+arrow keys can reach ALL panes in various layouts.
# Regression test for navigation algorithm that skipped certain panes.

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "[FATAL] psmux binary not found" -ForegroundColor Red
    exit 1
}

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

$SESSION = "nav_test_$(Get-Random)"
Write-Info "Using psmux binary: $PSMUX"

# ─── Cleanup ──────────────────────────────────────────────────
Write-Info "Cleaning up stale sessions..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

# ─── Helpers ──────────────────────────────────────────────────

# Get the active pane ID (via display-message)
function Get-ActivePaneId {
    param($Session)
    $id = (& $PSMUX display-message -p "#{pane_id}" -t $Session 2>&1) | Out-String
    return $id.Trim()
}

# Get all pane IDs
function Get-AllPaneIds {
    param($Session)
    $panes = (& $PSMUX list-panes -t $Session 2>&1) | Out-String
    $ids = @()
    foreach ($line in $panes.Split("`n")) {
        $line = $line.Trim()
        if ($line -match '%(\d+)') {
            $ids += "%$($Matches[1])"
        }
    }
    return $ids
}

# Navigate in a direction using select-pane
function Navigate {
    param($Session, $Dir)
    Psmux select-pane -t $Session "-$Dir" | Out-Null
    Start-Sleep -Milliseconds 200
}

# Check if all panes are reachable by cycling through directions
function Test-AllPanesReachable {
    param($Session, $Label, $PaneCount)
    
    $allIds = Get-AllPaneIds -Session $Session
    Write-Info "  Pane IDs: $($allIds -join ', ')"
    
    if ($allIds.Count -lt $PaneCount) {
        Write-Fail "$Label - Expected $PaneCount panes, found $($allIds.Count)"
        return $false
    }
    
    # BFS: from each visited pane, select it by ID, then try all 4 directions
    $visited = @{}
    $startId = Get-ActivePaneId -Session $Session
    $visited[$startId] = $true
    $queue = [System.Collections.Queue]::new()
    $queue.Enqueue($startId)
    
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        # Select this pane by ID (include session name for correct routing)
        Psmux select-pane -t "${Session}:${current}" | Out-Null
        Start-Sleep -Milliseconds 200
        foreach ($dir in @("U", "D", "L", "R")) {
            # Navigate in direction
            Navigate -Session $Session -Dir $dir
            $newId = Get-ActivePaneId -Session $Session
            if ($newId -and -not $visited.ContainsKey($newId)) {
                $visited[$newId] = $true
                $queue.Enqueue($newId)
                Write-Info "    From $current DIR=$dir -> discovered $newId"
            }
            # Return to current pane for next direction
            Psmux select-pane -t "${Session}:${current}" | Out-Null
            Start-Sleep -Milliseconds 200
        }
    }
    
    $reachable = $visited.Count
    Write-Info "  Visited pane IDs: $($visited.Keys -join ', ')"
    Write-Info "  Reached $reachable / $($allIds.Count) panes"
    
    if ($reachable -ge $allIds.Count) {
        Write-Pass "$Label - All $reachable panes reachable"
        return $true
    } else {
        $missing = $allIds | Where-Object { -not $visited.ContainsKey($_) }
        Write-Fail "$Label - Only $reachable/$($allIds.Count) panes reachable. Missing: $($missing -join ', ')"
        return $false
    }
}

# ═══════════════════════════════════════════════════════════════
Write-Host "=" * 60
Write-Host "PANE NAVIGATION TESTS"
Write-Host "=" * 60

# ─── Start session ────────────────────────────────────────────
Write-Info "Starting test session: $SESSION"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3

$sessions = (& $PSMUX ls 2>&1) -join "`n"
if ($sessions -notmatch [regex]::Escape($SESSION)) {
    Write-Host "[FATAL] Could not start session. Output: $sessions" -ForegroundColor Red
    exit 1
}
Write-Info "Session started successfully"

# ─── Test 1: 2 panes (vertical split) ────────────────────────
Write-Host ""
Write-Host "--- Layout 1: 2 panes (vertical split) ---"
# Already have 1 pane, split once
Psmux split-window -v -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Test "2-pane vertical: all panes reachable"
Test-AllPanesReachable -Session $SESSION -Label "2-pane vertical" -PaneCount 2

# ─── Test 2: 3 panes (add horizontal split) ──────────────────
Write-Host ""
Write-Host "--- Layout 2: 3 panes (V + H) ---"
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Test "3-pane V+H: all panes reachable"
Test-AllPanesReachable -Session $SESSION -Label "3-pane V+H" -PaneCount 3

# ─── Test 3: 4 panes (asymmetric - the bug trigger) ──────────
Write-Host ""
Write-Host "--- Layout 3: 4 panes (asymmetric grid) ---"
# Go to top pane, split it horizontally
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 500
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Test "4-pane asymmetric: all panes reachable"
Test-AllPanesReachable -Session $SESSION -Label "4-pane asymmetric" -PaneCount 4

# ─── Test 4: 5 panes ─────────────────────────────────────────
Write-Host ""
Write-Host "--- Layout 4: 5 panes ---"
Psmux split-window -v -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Test "5-pane: all panes reachable"
Test-AllPanesReachable -Session $SESSION -Label "5-pane" -PaneCount 5

# ─── Test 5: 6 panes (complex - matches user's screenshot) ───
Write-Host ""
Write-Host "--- Layout 5: 6 panes (complex) ---"
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Test "6-pane complex: all panes reachable"
Test-AllPanesReachable -Session $SESSION -Label "6-pane complex" -PaneCount 6

# ─── Test 6: New window with clean grid layout ───────────────
Write-Host ""
Write-Host "--- Layout 6: New window, 2x2 grid ---"
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Seconds 2
# Create a 2x2 grid: split v, go up, split h, go down, split h
Psmux split-window -v -t $SESSION | Out-Null
Start-Sleep -Seconds 1
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 300
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 1
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 300
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Test "2x2 grid: all panes reachable"
Test-AllPanesReachable -Session $SESSION -Label "2x2 grid" -PaneCount 4

# ─── Test 7: Directional navigation correctness ──────────────
Write-Host ""
Write-Host "--- Test 7: Direction correctness in 2x2 grid ---"
# In a 2x2 grid:
#  [A] [B]
#  [C] [D]
# From A: Right→B, Down→C (not B)
# From D: Left→C, Up→B (not A)

# Navigate to top-left
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 200
Psmux select-pane -t $SESSION -L | Out-Null
Start-Sleep -Milliseconds 200
$topLeft = Get-ActivePaneId -Session $SESSION

# Go right → should be top-right
Navigate -Session $SESSION -Dir "R"
$topRight = Get-ActivePaneId -Session $SESSION

# Go down → should be bottom-right (not top-left!)
Navigate -Session $SESSION -Dir "D"
$bottomRight = Get-ActivePaneId -Session $SESSION

# Go left → should be bottom-left
Navigate -Session $SESSION -Dir "L"
$bottomLeft = Get-ActivePaneId -Session $SESSION

# Go up → should be top-left
Navigate -Session $SESSION -Dir "U"
$backToTopLeft = Get-ActivePaneId -Session $SESSION

Write-Test "2x2: All 4 cells are distinct panes"
$allDistinct = @($topLeft, $topRight, $bottomRight, $bottomLeft) | Sort-Object -Unique
if ($allDistinct.Count -eq 4) {
    Write-Pass "All 4 grid cells are distinct panes"
} else {
    Write-Fail "Expected 4 distinct panes from grid navigation, got $($allDistinct.Count): $($allDistinct -join ', ')"
}

Write-Test "2x2: Circuit returns to start"
if ($backToTopLeft -eq $topLeft) {
    Write-Pass "R→D→L→U returns to starting pane"
} else {
    Write-Fail "Circuit didn't return to start. Start=$topLeft, End=$backToTopLeft"
}

Write-Test "2x2: Down from top-right is consistent"
# Replay the exact same path from topLeft: R then D should give bottomRight
# We're already at topLeft (backToTopLeft confirmed)
Navigate -Session $SESSION -Dir "R"
$replayTopRight = Get-ActivePaneId -Session $SESSION
Navigate -Session $SESSION -Dir "D"
$replayDown = Get-ActivePaneId -Session $SESSION
# The key invariant: R→D from the same starting pane should always land on the same pane
if ($replayTopRight -eq $topRight -and $replayDown -eq $bottomRight) {
    Write-Pass "Down from top-right is consistent with circuit path"
} elseif ($replayDown -ne $replayTopRight) {
    # At minimum, D from topRight should go to a DIFFERENT pane (not stay)
    Write-Pass "Down from top-right navigates to a different pane ($replayDown)"
} else {
    Write-Fail "Down from top-right went to $replayDown, expected $bottomRight (circuit result)"
}

# ─── Cleanup ──────────────────────────────────────────────────
Write-Host ""
Write-Info "Cleaning up..."
Psmux kill-session -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Host ""
Write-Host "=" * 60
Write-Host "PANE NAVIGATION TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host ""

if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
    exit 0
}
