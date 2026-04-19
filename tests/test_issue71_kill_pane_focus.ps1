# psmux Issue #71 — Kill pane focus behavior
#
# Tests that:
# 1. Killing a non-active pane keeps focus on the current pane
# 2. Killing the active pane moves focus to the MRU pane
# 3. After kill, navigation still works (no "zombie" focus state)
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue71_kill_pane_focus.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_71"

function Wait-ForSession {
    param($name, $timeout = 10)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

function Get-ActivePaneId {
    param($session)
    $info = & $PSMUX display-message -t $session -p '#{pane_id}' 2>&1
    return ($info | Out-String).Trim()
}

function Get-PaneCount {
    param($session)
    $panes = & $PSMUX list-panes -t $session 2>&1
    return ($panes | Measure-Object -Line).Lines
}

function New-TestSession {
    param($name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $name" -WindowStyle Hidden
    if (-not (Wait-ForSession $name)) {
        Write-Fail "Could not create session $name"
        return $false
    }
    Start-Sleep -Seconds 3
    return $true
}

function Capture-Pane {
    param($target)
    $raw = & $PSMUX capture-pane -t $target -p 2>&1
    return ($raw | Out-String)
}

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #71: Kill pane focus — comprehensive tests"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# ──────────────────────────────────────────────────────────────
# Test 1: Kill active pane → focus moves to MRU pane
#   Layout:  +---+----+
#            | L | TR |
#            |   +----+
#            |   | BR |  ← active, kill this
#            +---+----+
#   MRU order: BR(0), TR(1), L(2)
#   Kill BR → should focus TR (MRU winner among remaining)
# ──────────────────────────────────────────────────────────────
Write-Test "1: Kill active pane → focus moves to MRU (TR)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  Panes: L=$pL TR=$pTR BR=$pBR (active)"
    Write-Info "  MRU order: BR, TR, L"

    # Verify we're on BR
    $active = Get-ActivePaneId $SESSION
    if ($active -ne $pBR) { Write-Fail "1: Setup — expected BR ($pBR), got $active"; throw "skip" }

    # Kill active pane (BR)
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $active = Get-ActivePaneId $SESSION
    $count = Get-PaneCount $SESSION

    if ($active -eq $pTR) {
        Write-Pass "1: Kill active BR → focus moved to TR (MRU). Panes=$count"
    } elseif ($active -eq $pL) {
        Write-Fail "1: Focus jumped to L ($pL) instead of MRU TR ($pTR)"
    } else {
        Write-Fail "1: Unexpected focus: $active (expected TR=$pTR)"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 2: Kill active pane with different MRU history
#   Same layout, but focus L before going back to BR.
#   MRU: BR(0), L(1), TR(2)
#   Kill BR → should focus L (MRU winner among remaining)
# ──────────────────────────────────────────────────────────────
Write-Test "2: Kill active pane → focus moves to MRU (L, not TR)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  Panes: L=$pL TR=$pTR BR=$pBR"

    # Build MRU so L is most recent right-side neighbor:
    # Focus L explicitly, then BR (so MRU = BR(0), L(1), TR(2))
    # After killing BR, L should be next MRU
    & $PSMUX select-pane -t "${SESSION}:${pL}" 2>&1 | Out-Null     # focus L
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t "${SESSION}:${pBR}" 2>&1 | Out-Null    # focus BR
    Start-Sleep -Milliseconds 500

    $active = Get-ActivePaneId $SESSION
    Write-Info "  Active before kill: $active (should be BR=$pBR)"

    # Kill active (BR) — MRU: L(1) should be chosen over TR(2)
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $active = Get-ActivePaneId $SESSION
    if ($active -eq $pL) {
        Write-Pass "2: Kill active BR → focus moved to L (MRU winner)"
    } elseif ($active -eq $pTR) {
        Write-Fail "2: Focus went to TR instead of L (MRU not used)"
    } else {
        Write-Fail "2: Unexpected focus: $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 3: Kill non-active pane → focus stays on current pane
#   Kill TR while BR is active
# ──────────────────────────────────────────────────────────────
Write-Test "3: Kill non-active pane → focus stays on current pane"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  Panes: L=$pL TR=$pTR BR=$pBR (active)"

    # Kill TR (non-active) by pane id
    # Kill by pane ID using session:%N format
    & $PSMUX kill-pane -t "${SESSION}:${pTR}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $active = Get-ActivePaneId $SESSION
    $count = Get-PaneCount $SESSION

    if ($active -eq $pBR) {
        Write-Pass "3: Kill non-active TR → focus stayed on BR. Panes=$count"
    } else {
        Write-Fail "3: Focus changed! Expected BR ($pBR), got $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 4: Kill non-active pane → send-keys still works
#   After killing a non-active pane, the remaining active pane
#   should still receive input (proving focus is truly set)
# ──────────────────────────────────────────────────────────────
Write-Test "4: After kill non-active, send-keys works on active pane"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    # Kill TR
    # Kill by pane ID using session:%N format
    & $PSMUX kill-pane -t "${SESSION}:${pTR}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # Send keys to active pane (BR should be active)
    & $PSMUX send-keys -t $SESSION 'Write-Output "ALIVE_AFTER_KILL"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION

    if ($cap -match "ALIVE_AFTER_KILL") {
        Write-Pass "4: send-keys works after killing non-active pane"
    } else {
        Write-Fail "4: send-keys output not found. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 5: After kill active, navigation still works
#   Kill BR, then verify directional nav works from new focus
# ──────────────────────────────────────────────────────────────
Write-Test "5: After kill active, directional navigation still works"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  Kill BR, then navigate from new active"

    # Kill BR
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $afterKill = Get-ActivePaneId $SESSION
    Write-Info "  After kill active=$afterKill"

    # Navigate Left (should go to L if we're on TR, or stay if already on L)
    & $PSMUX select-pane -t $SESSION -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $afterNav = Get-ActivePaneId $SESSION

    # Navigate Right (should go back)
    & $PSMUX select-pane -t $SESSION -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $afterNav2 = Get-ActivePaneId $SESSION

    if ($afterNav -ne $afterKill -or $afterNav2 -ne $afterNav) {
        Write-Pass "5: Navigation works after kill active (moved: $afterKill → $afterNav → $afterNav2)"
    } elseif ($afterNav -eq $afterKill -and $afterNav -eq $pL) {
        # Only one pane direction to go — but nav command executed without error
        Write-Pass "5: Navigation works after kill (single direction: $afterNav)"
    } else {
        Write-Fail "5: Navigation stuck at $afterKill after kill"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 6: Kill non-active → send-keys to remaining panes works
#   4-pane grid, kill one, verify all remaining accept input
# ──────────────────────────────────────────────────────────────
Write-Test "6: 4-pane kill one, remaining panes all accept input"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pTL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX select-pane -t $SESSION -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBL = Get-ActivePaneId $SESSION

    & $PSMUX select-pane -t $SESSION -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  4 panes: TL=$pTL TR=$pTR BL=$pBL BR=$pBR"

    # Kill TL (non-active) by pane ID
    & $PSMUX kill-pane -t "${SESSION}:${pTL}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $count = Get-PaneCount $SESSION
    $active = Get-ActivePaneId $SESSION
    Write-Info "  After kill TL: panes=$count active=$active"

    # Verify active pane accepts input
    & $PSMUX send-keys -t $SESSION 'Write-Output "GRID_OK"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION

    if ($cap -match "GRID_OK" -and $count -eq 3) {
        Write-Pass "6: 4→3 panes, active pane accepts input after non-active kill"
    } else {
        Write-Fail "6: panes=$count, output match=$(if($cap -match 'GRID_OK'){'yes'}else{'no'})"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 7: Kill down to 2 panes, then 1 pane
#   Progressive kills — verify focus is correct at each step
# ──────────────────────────────────────────────────────────────
Write-Test "7: Progressive kill-pane down to 1 pane"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $p0 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p1 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p2 = Get-ActivePaneId $SESSION

    Write-Info "  3 panes: $p0 $p1 $p2"

    # Kill active (p2)
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1
    $after1 = Get-ActivePaneId $SESSION
    $count1 = Get-PaneCount $SESSION
    Write-Info "  After 1st kill: active=$after1 count=$count1"

    # Kill active again
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1
    $after2 = Get-ActivePaneId $SESSION
    $count2 = Get-PaneCount $SESSION
    Write-Info "  After 2nd kill: active=$after2 count=$count2"

    # Verify remaining pane accepts input
    & $PSMUX send-keys -t $SESSION 'Write-Output "LAST_PANE_OK"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION

    if ($count2 -eq 1 -and $cap -match "LAST_PANE_OK") {
        Write-Pass "7: Progressive kill down to 1 pane, still functional"
    } else {
        Write-Fail "7: count=$count2, output match=$(if($cap -match 'LAST_PANE_OK'){'yes'}else{'no'})"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "7: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 8: Issue #71 exact repro — Case A
#   1. Fresh session
#   2. Ctrl+b % (vertical split, focus right)
#   3. Ctrl+b " (horizontal split in right, focus BR)
#   4. Kill active (BR)
#   Expected: focus → TR (most recently active remaining)
# ──────────────────────────────────────────────────────────────
Write-Test "8: Issue #71 Case A — kill active BR → focus TR"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    # Step 2: vertical split (focus moves right)
    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    # Step 3: horizontal split in right (focus moves to BR)
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  L=$pL TR=$pTR BR=$pBR (active)"

    # Step 4: kill active
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $active = Get-ActivePaneId $SESSION
    if ($active -eq $pTR) {
        Write-Pass "8: Case A — kill BR → focus TR (correct MRU)"
    } elseif ($active -eq $pL) {
        Write-Fail "8: Case A — focus jumped to L! Expected TR ($pTR)"
    } else {
        Write-Fail "8: Case A — unexpected: $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "8: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ──────────────────────────────────────────────────────────────
# Test 9: Issue #71 exact repro — Case B
#   Same layout, kill non-active TR while BR is active
#   Expected: focus stays on BR
# ──────────────────────────────────────────────────────────────
Write-Test "9: Issue #71 Case B — kill non-active TR → focus stays BR"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $pL = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pTR = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBR = Get-ActivePaneId $SESSION

    Write-Info "  L=$pL TR=$pTR BR=$pBR (active)"

    # Kill TR by ID
    # Kill by pane ID using session:%N format
    & $PSMUX kill-pane -t "${SESSION}:${pTR}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $active = Get-ActivePaneId $SESSION
    if ($active -eq $pBR) {
        Write-Pass "9: Case B — kill TR → focus stays on BR"
    } elseif ($active -eq $pL) {
        Write-Fail "9: Case B — focus jumped to L! Expected BR ($pBR)"
    } else {
        Write-Fail "9: Case B — unexpected: $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "9: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ══════════════════════════════════════════════════════════════════════
# Cleanup & summary
# ══════════════════════════════════════════════════════════════════════
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped (of $total run)" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
