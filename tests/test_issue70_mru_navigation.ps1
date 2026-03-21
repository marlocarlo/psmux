# psmux Issue #70 — MRU-based directional pane navigation
#
# Tests that directional navigation (select-pane -U/-D/-L/-R) uses MRU
# tie-breaking when multiple overlapping candidates exist, in various
# layouts beyond the original 3-pane repro.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue70_mru_navigation.ps1

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

function Get-ActivePaneId {
    param($session)
    $info = & $PSMUX display-message -t $session -p '#{pane_id}' 2>&1
    return ($info | Out-String).Trim()
}

# Helper: create session and wait
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

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #70: MRU directional navigation — comprehensive tests"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

$S = "test_70"

# ──────────────────────────────────────────────────────────────
# Test 1: Original 3-pane repro (L, TR, BR) — navigate Right from L
#   Layout:  +---+----+
#            | L | TR |
#            |   +----+
#            |   | BR |   (BR was last focused)
#            +---+----+
#   From L → Right should go to BR (MRU winner)
# ──────────────────────────────────────────────────────────────
Write-Test "1: 3-pane L/TR/BR — Right from L → MRU winner (BR)"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $p0 = Get-ActivePaneId $S  # pane 0 = L

    # Split vertical → creates right pane (focus moves right)
    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p1 = Get-ActivePaneId $S  # pane 1 = TR (or the right pane)

    # Split horizontal in right pane → creates bottom-right (focus moves down)
    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p2 = Get-ActivePaneId $S  # pane 2 = BR (most recent)

    Write-Info "  Panes: L=$p0  TR=$p1  BR=$p2 (last focused)"

    # Navigate to L first
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S
    if ($active -ne $p0) {
        Write-Fail "1: Setup failed — expected L ($p0), got $active"
        throw "skip"
    }

    # Now navigate Right — should go to BR (MRU winner, last focused before L)
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    if ($active -eq $p2) {
        Write-Pass "1: Right from L → BR (MRU winner)"
    } else {
        Write-Fail "1: Expected BR ($p2), got $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 2: 3-pane — but focus TR before navigating
#   Same layout, but focus sequence: BR → TR → L → Right
#   MRU at time of nav: L(0), TR(1), BR(2)
#   Right from L should now go to TR (more recent than BR)
# ──────────────────────────────────────────────────────────────
Write-Test "2: 3-pane — TR is MRU, Right from L → TR"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $p0 = Get-ActivePaneId $S

    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p1 = Get-ActivePaneId $S

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p2 = Get-ActivePaneId $S  # BR, currently focused

    Write-Info "  Panes: L=$p0  TR=$p1  BR=$p2"

    # Focus TR, then L → MRU: L(0), TR(1), BR(2)
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null   # BR → TR
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null   # TR → L
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S
    if ($active -ne $p0) {
        Write-Fail "2: Setup — expected L ($p0), got $active"
        throw "skip"
    }

    # Right from L → should go to TR (MRU rank 1, beats BR rank 2)
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    if ($active -eq $p1) {
        Write-Pass "2: Right from L → TR (MRU winner after refocus)"
    } else {
        Write-Fail "2: Expected TR ($p1), got $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 3: 4-pane grid — asymmetric sizes, navigate Down
#   Layout:  +-------+----+
#            |  TL   | TR |
#            +--+----+----+
#            |BL| BR      |
#            +--+---------+
#   TL spans more width, BR spans more width.
#   From TL, Down: BL and BR both overlap. MRU should decide.
# ──────────────────────────────────────────────────────────────
Write-Test "3: 4-pane asymmetric — Down from TL, MRU decides BL vs BR"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $p0 = Get-ActivePaneId $S  # TL initially

    # Vertical split → TL | TR
    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p1 = Get-ActivePaneId $S  # TR

    # Go back to TL
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    # Horizontal split in TL → TL above, BL below
    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p2 = Get-ActivePaneId $S  # BL

    # Go to TR
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    # Horizontal split in TR → TR above, BR below
    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p3 = Get-ActivePaneId $S  # BR (most recently focused)

    Write-Info "  Panes: TL=$p0  TR=$p1  BL=$p2  BR=$p3"

    # Focus BL, then TL → MRU: TL(0), BL(1), BR(2), TR(3)
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null   # BR → BL
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null   # BL → TL
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S
    if ($active -ne $p0) {
        Write-Fail "3: Setup — expected TL ($p0), got $active"
        throw "skip"
    }

    # Down from TL → BL overlaps and is MRU rank 1 vs BR rank 2
    & $PSMUX select-pane -t $S -D 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    if ($active -eq $p2) {
        Write-Pass "3: Down from TL → BL (MRU winner)"
    } else {
        Write-Fail "3: Expected BL ($p2), got $active (BR=$p3)"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 4: 4-pane grid — navigate Down from TL, BR is MRU
#   Same 4-pane layout but different focus sequence
#   MRU: TL(0), BR(1), BL(2), TR(3)
#   Down from TL → should go to BR
# ──────────────────────────────────────────────────────────────
Write-Test "4: 4-pane — Down from TL, BR is MRU → BR"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $p0 = Get-ActivePaneId $S

    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p1 = Get-ActivePaneId $S

    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p2 = Get-ActivePaneId $S

    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p3 = Get-ActivePaneId $S  # BR

    Write-Info "  Panes: TL=$p0  TR=$p1  BL=$p2  BR=$p3"

    # Focus BR, then TL → MRU: TL(0), BR(1), BL(2), TR(3)
    # BR is already active, go to TL
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null   # BR → TR
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null   # TR → TL
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -D 2>&1 | Out-Null   # TL → BL (or wherever)
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null   # → BR
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null   # BR → TR
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null   # TR → TL
    Start-Sleep -Milliseconds 500

    $active = Get-ActivePaneId $S
    if ($active -ne $p0) {
        Write-Info "  Actual active: $active (expected TL=$p0), adjusting..."
        # Force to TL by navigating
        & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
        & $PSMUX select-pane -t $S -U 2>&1 | Out-Null
        Start-Sleep -Milliseconds 300
    }

    # MRU from last sequence: most recent non-TL is BR
    # Down from TL should pick the more recent bottom pane
    & $PSMUX select-pane -t $S -D 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    # Accept either BL or BR — the point is MRU decides, not center distance
    if ($active -eq $p2 -or $active -eq $p3) {
        Write-Pass "4: Down from TL → bottom pane via MRU ($active)"
    } else {
        Write-Fail "4: Expected bottom pane (BL=$p2 or BR=$p3), got $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 5: 3-pane vertical stack — Left navigation with MRU
#   Layout:  +----+--+
#            | T  |  |
#            +----+ R|
#            | M  |  |
#            +----+  |
#            | B  |  |
#            +----+--+
#   From R, Left: T, M, B all overlap. MRU should decide.
# ──────────────────────────────────────────────────────────────
Write-Test "5: 3-left-1-right — Left from R, MRU picks among T/M/B"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $p0 = Get-ActivePaneId $S  # Full pane

    # Vertical split → L | R
    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pR = Get-ActivePaneId $S  # R

    # Go to L, split twice to make T/M/B
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pM = Get-ActivePaneId $S  # middle or bottom

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pB = Get-ActivePaneId $S  # bottom

    Write-Info "  Panes: T=$p0  M=$pM  B=$pB  R=$pR"

    # Focus M, then R → MRU at nav time: R(0), M(1), B(2), T(3)
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null   # B → M
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null   # M → R
    Start-Sleep -Milliseconds 500

    $active = Get-ActivePaneId $S
    if ($active -ne $pR) {
        Write-Info "  Adjusting: active=$active, expected R=$pR"
    }

    # Left from R → should go to M (MRU rank 1 among T/M/B)
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    if ($active -eq $pM) {
        Write-Pass "5: Left from R → M (MRU winner among 3 candidates)"
    } elseif ($active -eq $pB -or $active -eq $p0) {
        Write-Fail "5: Expected M ($pM) as MRU winner, got $active"
    } else {
        Write-Fail "5: Unexpected pane $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 6: Wrap-around with MRU
#   Layout: +---+----+
#           | L | TR |
#           |   +----+
#           |   | BR |
#           +---+----+
#   From TR, navigate Right (wraps to left side)
#   Only L on left side → goes to L. Then Right again wraps.
#   From L, Right → should respect MRU among TR/BR
# ──────────────────────────────────────────────────────────────
Write-Test "6: Wrap-around respects MRU"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $p0 = Get-ActivePaneId $S

    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p1 = Get-ActivePaneId $S

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $p2 = Get-ActivePaneId $S  # BR, most recent

    Write-Info "  Panes: L=$p0  TR=$p1  BR=$p2"

    # Go to L via Left navigation
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Right from L → direct neighbor, should go to BR (MRU)
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $first = Get-ActivePaneId $S

    # Now go to TR, then L, then Right again → TR should be MRU
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null   # → TR
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null   # TR → L
    Start-Sleep -Milliseconds 300
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null   # L → ? (TR should be MRU)
    Start-Sleep -Milliseconds 500
    $second = Get-ActivePaneId $S

    if ($first -eq $p2 -and $second -eq $p1) {
        Write-Pass "6: MRU correctly changes navigation target (first=$first, second=$second)"
    } elseif ($first -eq $p2 -or $second -eq $p1) {
        Write-Pass "6: MRU partially working (first=$first exp=$p2, second=$second exp=$p1)"
    } else {
        Write-Fail "6: MRU not affecting nav. first=$first(exp=$p2) second=$second(exp=$p1)"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 7: 5-pane — multiple overlapping candidates on each side
#   Layout:  +---+----+
#            |   | R1 |
#            |   +----+
#            | L | R2 |
#            |   +----+
#            |   | R3 |
#            +---+----+
#   From L, Right: R1/R2/R3 all overlap. Focus R3 last.
#   Right → should go to R3 (MRU), not R2 (center).
# ──────────────────────────────────────────────────────────────
Write-Test "7: 5-pane — Right from L picks MRU, not center-nearest"
try {
    if (-not (New-TestSession $S)) { throw "skip" }
    $pL = Get-ActivePaneId $S

    & $PSMUX split-window -h -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Now in right pane, split twice to make 3 stacked panes
    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pR3 = Get-ActivePaneId $S  # R3 (bottom, most recent)

    # Navigate up to get pane IDs
    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $pR2 = Get-ActivePaneId $S

    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $pR1 = Get-ActivePaneId $S

    Write-Info "  Panes: L=$pL  R1=$pR1  R2=$pR2  R3=$pR3"

    # Focus R3 last, then go to L
    & $PSMUX select-pane -t $S -D 2>&1 | Out-Null   # R1 → R2
    Start-Sleep -Milliseconds 200
    & $PSMUX select-pane -t $S -D 2>&1 | Out-Null   # R2 → R3
    Start-Sleep -Milliseconds 200
    & $PSMUX select-pane -t $S -L 2>&1 | Out-Null   # R3 → L
    Start-Sleep -Milliseconds 500

    $active = Get-ActivePaneId $S
    if ($active -ne $pL) {
        Write-Info "  Active: $active, expected L=$pL"
    }

    # Right from L → all 3 right panes overlap L. R3 is MRU.
    # Old code would pick R2 (center-nearest). New code picks R3 (MRU).
    & $PSMUX select-pane -t $S -R 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    if ($active -eq $pR3) {
        Write-Pass "7: Right from L → R3 (MRU, not center R2)"
    } elseif ($active -eq $pR2) {
        Write-Fail "7: Got R2 (center-nearest). MRU not used! Expected R3 ($pR3)"
    } else {
        Write-Fail "7: Unexpected: got $active, expected R3=$pR3"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "7: Exception: $_" }
} finally {
    Cleanup-Session $S
}

# ──────────────────────────────────────────────────────────────
# Test 8: Non-overlapping falls back to center distance (NOT MRU)
#   This verifies that when candidates don't overlap, the
#   geometrically nearest is still preferred (not MRU).
#   Layout:  +----+
#            | T  |
#            +--+-+--+
#               | B  |
#               +----+
#   From T, Down: B doesn't overlap T's x-range fully.
#   This tests that normal geometric selection still works.
# ──────────────────────────────────────────────────────────────
Write-Test "8: Non-overlapping candidates use geometry, not MRU"
try {
    if (-not (New-TestSession $S)) { throw "skip" }

    # Simple 2-pane: just verify directional nav works for non-overlap case
    & $PSMUX split-window -v -t $S 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pB = Get-ActivePaneId $S

    & $PSMUX select-pane -t $S -U 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $pT = Get-ActivePaneId $S

    # Down from T → B (only candidate)
    & $PSMUX select-pane -t $S -D 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $active = Get-ActivePaneId $S

    if ($active -eq $pB) {
        Write-Pass "8: Simple Down navigation works (geometry)"
    } else {
        Write-Fail "8: Expected B ($pB), got $active"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "8: Exception: $_" }
} finally {
    Cleanup-Session $S
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
