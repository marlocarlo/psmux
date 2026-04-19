# psmux Issue #70 — MRU via select-pane -t (pane index targeting)
#
# Tests that MRU is updated consistently across ALL focus-change paths,
# not just directional navigation.  Specifically covers:
#   - select-pane -t 0:0.N  (explicit pane index targeting)
#   - FocusPaneByIndex path
#
# Repro from spooki44 (comment #4060952854):
#   psmux split-window -h -t 0:0.0   → focus: 1
#   psmux split-window -v -t 0:0.1   → focus: 2
#   psmux select-pane -t 0:0.1       → focus: 1, MRU: [1, 2, 0]
#   psmux select-pane -t 0:0.0       → focus: 0, MRU: [0, 1, 2]
#   psmux select-pane -L              → expected: 1 (MRU), actual: 2 (BUG)
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue70_select_pane_mru.ps1

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

function Get-PaneIndex {
    param($session)
    $info = & $PSMUX display-message -t $session -p '#{pane_index}' 2>&1
    return ($info | Out-String).Trim()
}

function Get-PaneId {
    param($session)
    $info = & $PSMUX display-message -t $session -p '#{pane_id}' 2>&1
    return ($info | Out-String).Trim()
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

$S = "test_70s"

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #70: MRU via select-pane -t (pane index targeting)"
Write-Host ("=" * 70)
# ══════════════════════════════════════════════════════════════════════

# ──────────────────────────────────────────────────────────────
# Test 1: Exact spooki44 repro — select-pane -t 0:0.N should update MRU
#   Layout:  +---+----+
#            |   |  1 |
#            | 0 +----+
#            |   |  2 |
#            +---+----+
#   Focus via select-pane -t: 1, then 0
#   Then select-pane -L → expected pane 1 (MRU), not pane 2
# ──────────────────────────────────────────────────────────────
Write-Test "1: spooki44 repro — select-pane -t updates MRU"
try {
    if (-not (New-TestSession $S)) { throw "skip" }

    # Split: pane 0 (left), pane 1 (right)
    & $PSMUX split-window -h -t "${S}:0.0" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Split right pane: pane 1 (top-right), pane 2 (bottom-right)
    & $PSMUX split-window -v -t "${S}:0.1" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Verify 3 panes exist
    $paneCount = & $PSMUX display-message -t $S -p '#{window_panes}' 2>&1 | Out-String
    $paneCount = $paneCount.Trim()
    Write-Info "  Pane count: $paneCount"

    # Focus pane 1 via explicit targeting (should update MRU)
    & $PSMUX select-pane -t "${S}:0.1" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $idx = Get-PaneIndex $S
    Write-Info "  After select-pane -t 0:0.1: active pane index = $idx"

    # Focus pane 0 via explicit targeting (should update MRU)
    & $PSMUX select-pane -t "${S}:0.0" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $idx = Get-PaneIndex $S
    Write-Info "  After select-pane -t 0:0.0: active pane index = $idx"
    if ($idx -ne "0") {
        Write-Fail "1: Setup — expected pane 0, got $idx"
        throw "skip"
    }

    # MRU should now be: [0, 1, 2]
    # Navigate Left from pane 0 → should wrap to right side
    # With MRU: pane 1 should win (more recent than pane 2)
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $idx = Get-PaneIndex $S
    Write-Info "  After select-pane -L: active pane index = $idx"

    if ($idx -eq "1") {
        Write-Pass "1: select-pane -t updates MRU — Left from 0 → pane 1 (MRU winner)"
    } else {
        Write-Fail "1: Expected pane index 1 (MRU winner), got $idx"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 2: select-pane -t changes MRU order — focus pane 2 last
#   Same layout, but focus: pane 2, then pane 0
#   MRU: [0, 2, 1]
#   Left from 0 → expected pane 2 (MRU winner)
# ──────────────────────────────────────────────────────────────
Write-Test "2: select-pane -t MRU order — focus pane 2 last, then 0 → Left → pane 2"
try {
    if (-not (New-TestSession $S)) { throw "skip" }

    & $PSMUX split-window -h -t "${S}:0.0" 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX split-window -v -t "${S}:0.1" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Focus pane 2 via -t (MRU: [2, ...])
    & $PSMUX select-pane -t "${S}:0.2" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Focus pane 0 via -t (MRU: [0, 2, 1])
    & $PSMUX select-pane -t "${S}:0.0" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Left from pane 0 → should go to pane 2 (MRU rank 1)
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $idx = Get-PaneIndex $S
    if ($idx -eq "2") {
        Write-Pass "2: Left from 0 → pane 2 (MRU winner after -t focus)"
    } else {
        Write-Fail "2: Expected pane index 2, got $idx"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 3: Mixed focus paths — directional + select-pane -t
#   Build MRU with directional nav, then change with -t
#   Verify the -t change overrides directional MRU
# ──────────────────────────────────────────────────────────────
Write-Test "3: Mixed — directional nav then select-pane -t overrides MRU"
try {
    if (-not (New-TestSession $S)) { throw "skip" }

    & $PSMUX split-window -h -t "${S}:0.0" 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX split-window -v -t "${S}:0.1" 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    # Focus is on pane 2 (bottom-right) after split

    # Navigate up to pane 1 (directional, updates MRU)
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    # Navigate left to pane 0 (directional, updates MRU)
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    # Now MRU: [0, 1, 2]

    # Override MRU via -t: focus pane 2, then back to 0
    & $PSMUX select-pane -t "${S}:0.2" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t "${S}:0.0" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    # Now MRU should be: [0, 2, 1] — pane 2 is rank 1

    # Left from pane 0 → should go to pane 2 (MRU)
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $idx = Get-PaneIndex $S
    if ($idx -eq "2") {
        Write-Pass "3: select-pane -t overrides directional MRU — Left → pane 2"
    } else {
        Write-Fail "3: Expected pane index 2, got $idx"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 70)
Write-Host ("Results: {0} passed, {1} failed, {2} skipped" -f $script:TestsPassed, $script:TestsFailed, $script:TestsSkipped)
Write-Host ("=" * 70)
if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
