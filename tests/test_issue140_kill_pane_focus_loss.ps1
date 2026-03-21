# psmux Issue #140 — Pane removal can lose UI focus and misreport active pane
#
# Tests that:
# 1. After killing a pane by ID, focus moves to the MRU pane (not a random one)
# 2. After a pane process exits, focus moves to the MRU pane
# 3. The active pane is correctly reported by list-panes after removal
# 4. The exact 5-pane layout from the issue reproduces correctly
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue140_kill_pane_focus_loss.ps1

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

$SESSION = "test_140"

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

function Get-ListPanesActive {
    param($session)
    $panes = & $PSMUX list-panes -t $session 2>&1
    $text = ($panes | Out-String)
    if ($text -match '%(\d+)\s+\(active\)') {
        return "%$($Matches[1])"
    }
    return $null
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

Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #140: Pane removal focus loss and misreport"
Write-Host ("=" * 60)

# ============================================================
# Test 1: Exact reproduction from issue #140
#   Create 5 panes, select %1, select %3, kill %3 by ID
#   Expected: %1 becomes active (MRU pane)
# ============================================================
Write-Test "1: Exact issue #140 reproduction (kill-pane -t by ID)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Get initial pane ID (%1)
    $p1 = Get-ActivePaneId $SESSION
    Write-Info "  Created pane $p1"

    # split-window -h -t 0:0 -> creates %2
    & $PSMUX split-window -h -t "${SESSION}:0" 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p2 = Get-ActivePaneId $SESSION
    Write-Info "  Split -> $p2"

    # split-window -h -d -t 0:0 -> creates %3 (no focus due to -d)
    & $PSMUX split-window -h -d -t "${SESSION}:0" 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    # split-window -v -t 0:0 -> creates %4
    & $PSMUX split-window -v -t "${SESSION}:0" 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p4 = Get-ActivePaneId $SESSION
    Write-Info "  Split -> $p4"

    # split-window -v -t 0:0 -> creates %5
    & $PSMUX split-window -v -t "${SESSION}:0" 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p5 = Get-ActivePaneId $SESSION
    Write-Info "  Split -> $p5"

    # List all panes to find %3 (pane created with -d that we never focused)
    $listOutput = & $PSMUX list-panes -t $SESSION 2>&1
    $listText = ($listOutput | Out-String)
    Write-Info "  Layout after setup:"
    $listText.Split("`n") | ForEach-Object { if ($_.Trim()) { Write-Info "    $_" } }

    # Extract all pane IDs from list-panes
    $allIds = [regex]::Matches($listText, '%(\d+)') | ForEach-Object { "%$($_.Groups[1].Value)" }
    Write-Info "  All panes: $($allIds -join ', ')"

    # Find %3 (the pane we haven't identified yet)
    $p3 = $allIds | Where-Object { $_ -ne $p1 -and $_ -ne $p2 -and $_ -ne $p4 -and $_ -ne $p5 } | Select-Object -First 1
    if (-not $p3) {
        Write-Fail "1: Could not identify the 5th pane (expected %3 equivalent)"
        throw "skip"
    }
    Write-Info "  Identified detached pane: $p3"
    Write-Info "  Panes: p1=${p1} p2=${p2} p3=${p3} p4=${p4} p5=${p5}"

    # select-pane -t %1
    & $PSMUX select-pane -t "${SESSION}:${p1}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $SESSION
    Write-Info "  After select ${p1}: active=$active"

    # select-pane -t %3
    & $PSMUX select-pane -t "${SESSION}:${p3}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $SESSION
    Write-Info "  After select ${p3}: active=$active"

    # Now MRU should be: p3, p1, ...
    # Kill %3 by pane ID
    & $PSMUX kill-pane -t "${SESSION}:${p3}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $activeAfter = Get-ActivePaneId $SESSION
    $listedActive = Get-ListPanesActive $SESSION
    $paneCount = Get-PaneCount $SESSION
    Write-Info "  After kill ${p3}: active=$activeAfter listed=$listedActive count=$paneCount"

    if ($activeAfter -eq $p1) {
        Write-Pass "1: Kill $p3 by ID -> focus correctly moved to MRU pane $p1"
    } else {
        Write-Fail "1: Kill $p3 by ID -> focus=$activeAfter, expected MRU=$p1"
    }

    # Also verify list-panes agrees
    if ($listedActive -eq $p1) {
        Write-Pass "1b: list-panes correctly reports $p1 as active"
    } elseif ($listedActive -eq $activeAfter) {
        Write-Pass "1b: list-panes and display-message agree (both=$activeAfter)"
    } else {
        Write-Fail "1b: list-panes reports $listedActive, display-message says $activeAfter"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ============================================================
# Test 2: Kill active pane by ID with 3 panes and MRU history
#   Create 3 panes, navigate p1 -> p2 -> p3, kill p3
#   Expected: p2 becomes active (MRU)
# ============================================================
Write-Test "2: Kill active pane by ID, 3 panes, MRU selects p2"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $p1 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p2 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p3 = Get-ActivePaneId $SESSION

    Write-Info "  Panes: p1=$p1 p2=$p2 p3=$p3"

    # Navigate: focus p1, focus p2, focus p3 -> MRU: p3, p2, p1
    & $PSMUX select-pane -t "${SESSION}:${p1}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t "${SESSION}:${p2}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t "${SESSION}:${p3}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Kill p3 by ID -> MRU should pick p2
    & $PSMUX kill-pane -t "${SESSION}:${p3}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $activeAfter = Get-ActivePaneId $SESSION
    if ($activeAfter -eq $p2) {
        Write-Pass "2: Kill $p3 by ID -> focus correctly moved to MRU pane $p2"
    } else {
        Write-Fail "2: Kill $p3 by ID -> focus=$activeAfter, expected MRU=$p2"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ============================================================
# Test 3: Kill non-active pane by ID preserves current focus
#   Create 3 panes p1,p2,p3. Focus p1, kill p3
#   Expected: p1 stays active
# ============================================================
Write-Test "3: Kill non-active pane by ID preserves focus"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $p1 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p2 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p3 = Get-ActivePaneId $SESSION

    Write-Info "  Panes: p1=$p1 p2=$p2 p3=$p3"

    # Focus p1
    & $PSMUX select-pane -t "${SESSION}:${p1}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $activeBefore = Get-ActivePaneId $SESSION
    Write-Info "  Active before kill: $activeBefore (should be ${p1})"

    # Kill p3 (not active) by ID
    & $PSMUX kill-pane -t "${SESSION}:${p3}" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $activeAfter = Get-ActivePaneId $SESSION
    if ($activeAfter -eq $p1) {
        Write-Pass "3: Kill non-active $p3 -> focus correctly stayed on $p1"
    } else {
        Write-Fail "3: Kill non-active $p3 -> focus=$activeAfter, expected $p1"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ============================================================
# Test 4: Kill pane via exit (process death) with MRU navigation
#   Create 3 panes, navigate p1->p2->p3, exit p3
#   Expected: p2 becomes active (MRU)
# ============================================================
Write-Test "4: Pane exit via 'exit' command, MRU focus"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }
    $p1 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p2 = Get-ActivePaneId $SESSION

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $p3 = Get-ActivePaneId $SESSION

    Write-Info "  Panes: p1=$p1 p2=$p2 p3=$p3"

    # Navigate: p1 -> p2 -> p3 -> MRU: p3, p2, p1
    & $PSMUX select-pane -t "${SESSION}:${p1}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t "${SESSION}:${p2}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t "${SESSION}:${p3}" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Send 'exit' to p3 to trigger process death path
    & $PSMUX send-keys -t "${SESSION}:${p3}" 'exit' Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    $activeAfter = Get-ActivePaneId $SESSION
    $paneCount = Get-PaneCount $SESSION
    Write-Info "  After exit: active=${activeAfter} count=${paneCount}"

    if ($activeAfter -eq $p2) {
        Write-Pass "4: Pane exit -> focus correctly moved to MRU pane $p2"
    } elseif ($activeAfter -eq $p1) {
        Write-Fail "4: Pane exit -> focus went to $p1 instead of MRU $p2"
    } else {
        Write-Fail "4: Pane exit -> focus=$activeAfter, expected MRU=$p2"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ============================================================
# Cleanup and summary
# ============================================================
Write-Host ""
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped"
Write-Host ("=" * 60)

exit $script:TestsFailed
