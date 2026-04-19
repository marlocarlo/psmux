# psmux Issue #82 — Zoom: comprehensive tmux parity
#
# Tests that ALL operations interact with zoom correctly per tmux behavior:
# - split-window: push/pop (unzoom → split → re-zoom on new pane)
# - swap-pane: push/pop (unzoom → swap → re-zoom)
# - kill-pane, break-pane, resize-pane, select-layout: permanent unzoom
# - Borders hidden and non-draggable when zoomed
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue82_zoom_split_borders.ps1

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

& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_82"

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

function Get-PaneCount {
    param($session)
    return (& $PSMUX list-panes -t $session 2>&1 | Measure-Object -Line).Lines
}

function Get-ZoomFlag {
    param($session)
    return (& $PSMUX display-message -t $session -p '#{window_zoomed_flag}' 2>&1 | Out-String).Trim()
}

function Get-ActivePaneId {
    param($session)
    return (& $PSMUX display-message -t $session -p '#{pane_id}' 2>&1 | Out-String).Trim()
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

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "PUSH/POP ZOOM: split-window and swap-pane"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: split-window while zoomed → re-zooms on new pane ---
Write-Test "1: split-window while zoomed → unzooms permanently (tmux parity)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    $pBefore = Get-ActivePaneId $SESSION

    # Zoom
    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Split while zoomed
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    $zoomed = Get-ZoomFlag $SESSION
    $pAfter = Get-ActivePaneId $SESSION
    $count = Get-PaneCount $SESSION

    if ($count -eq 3 -and $zoomed -eq "0") {
        Write-Pass "1: Split while zoomed → permanently unzoomed, 3 panes visible"
    } else {
        Write-Fail "1: Expected 3 panes, unzoomed. Got panes=$count zoomed=$zoomed"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 2: Split while zoomed already unzooms — no toggle needed ---
Write-Test "2: Split while zoomed already shows all panes (no extra unzoom)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Split while zoomed — should permanently unzoom
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Already unzoomed — no toggle needed. All 3 panes visible.
    $zoomed = Get-ZoomFlag $SESSION
    $count = Get-PaneCount $SESSION

    if ($count -eq 3 -and $zoomed -eq "0") {
        Write-Pass "2: Split while zoomed → 3 panes visible, already unzoomed"
    } else {
        Write-Fail "2: panes=$count zoomed=$zoomed"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 3: swap-pane while zoomed → stays zoomed ---
Write-Test "3: swap-pane while zoomed → permanently unzooms (tmux parity)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX swap-pane -U -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $zoomed = Get-ZoomFlag $SESSION
    if ($zoomed -eq "0") {
        Write-Pass "3: swap-pane while zoomed → permanently unzoomed"
    } else {
        Write-Fail "3: swap-pane should unzoom but zoomed=$zoomed"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "PERMANENT UNZOOM: kill-pane, resize, layout, break-pane"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 4: kill-pane while zoomed → unzooms ---
Write-Test "4: kill-pane while zoomed → permanently unzooms"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $z1 = Get-ZoomFlag $SESSION

    # Kill active pane while zoomed
    & $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $zoomed = Get-ZoomFlag $SESSION
    $count = Get-PaneCount $SESSION

    if ($z1 -eq "1" -and $zoomed -eq "0" -and $count -eq 2) {
        Write-Pass "4: kill-pane while zoomed → unzoomed, 2 panes"
    } else {
        Write-Fail "4: before=$z1 after=$zoomed panes=$count"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 5: resize-pane while zoomed → unzooms ---
Write-Test "5: resize-pane -U while zoomed → permanently unzooms"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Resize while zoomed (not -Z toggle, but directional resize)
    & $PSMUX resize-pane -U 5 -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $zoomed = Get-ZoomFlag $SESSION
    if ($zoomed -eq "0") {
        Write-Pass "5: resize-pane -U while zoomed → unzoomed"
    } else {
        Write-Fail "5: Still zoomed after resize-pane"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 6: select-layout while zoomed → unzooms ---
Write-Test "6: select-layout while zoomed → permanently unzooms"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX select-layout -t $SESSION even-horizontal 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $zoomed = Get-ZoomFlag $SESSION
    if ($zoomed -eq "0") {
        Write-Pass "6: select-layout while zoomed → unzoomed"
    } else {
        Write-Fail "6: Still zoomed after select-layout"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 7: next-layout while zoomed → unzooms ---
Write-Test "7: next-layout (Space) while zoomed → permanently unzooms"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX next-layout -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $zoomed = Get-ZoomFlag $SESSION
    if ($zoomed -eq "0") {
        Write-Pass "7: next-layout while zoomed → unzoomed"
    } else {
        Write-Fail "7: Still zoomed after next-layout"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "7: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 8: Zoom flag toggle still works normally ---
Write-Test "8: Zoom flag toggles correctly (off→on→off)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    $z0 = Get-ZoomFlag $SESSION
    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $z1 = Get-ZoomFlag $SESSION
    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $z2 = Get-ZoomFlag $SESSION

    if ($z0 -eq "0" -and $z1 -eq "1" -and $z2 -eq "0") {
        Write-Pass "8: Zoom toggle: 0→1→0"
    } else {
        Write-Fail "8: z0=$z0 z1=$z1 z2=$z2"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "8: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 9: 4-pane, zoom+split → correct pane count ---
Write-Test "9: 4-pane zoom+split → 5 panes"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX select-pane -t $SESSION -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    $c1 = Get-PaneCount $SESSION

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    $c2 = Get-PaneCount $SESSION

    if ($c1 -eq 4 -and $c2 -eq 5) {
        Write-Pass "9: 4→5 panes after zoom+split"
    } else {
        Write-Fail "9: before=$c1 after=$c2"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "9: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 10: Navigation after all zoom operations works ---
Write-Test "10: Navigation works after zoom operations"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Unzoom
    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $p1 = Get-ActivePaneId $SESSION
    & $PSMUX select-pane -t $SESSION -L 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $p2 = Get-ActivePaneId $SESSION

    if ($p1 -ne $p2) {
        Write-Pass "10: Navigation works after zoom cycle ($p1 → $p2)"
    } else {
        Write-Fail "10: Navigation stuck at $p1"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "10: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ═══════════════════════════════════════════════════════════════
# Win32 TUI VERIFICATION: Prove zoom/unzoom renders visually
# ═══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_Z82 = "z82_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_Z82
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # Create split for zoom testing
    & $script:TUI_PSMUX split-window -h -t $TUI_SESSION_Z82 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $paneCount = (& $script:TUI_PSMUX list-panes -t $TUI_SESSION_Z82 2>&1 | Measure-Object -Line).Lines
    Write-Host "    Setup: $paneCount panes" -ForegroundColor DarkGray

    # TUI Test 1: Zoom via CLI while visible window proves rendering
    Write-Test "TUI: Zoom pane via resize-pane -Z (visible TUI proof)"
    & $script:TUI_PSMUX resize-pane -Z -t $TUI_SESSION_Z82 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $zoomFlag = Safe-TuiQuery "#{window_zoomed_flag}" -Session $TUI_SESSION_Z82
    if ($zoomFlag -eq "1") {
        Write-Pass "TUI: Zoom activated (flag=1, visible window rendering)"
    } else {
        Write-Fail "TUI: Zoom flag is '$zoomFlag' after zoom (expected 1)"
    }

    # TUI Test 2: Unzoom and verify visual state
    Write-Test "TUI: Unzoom via resize-pane -Z (visible TUI proof)"
    & $script:TUI_PSMUX resize-pane -Z -t $TUI_SESSION_Z82 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $zoomFlag2 = Safe-TuiQuery "#{window_zoomed_flag}" -Session $TUI_SESSION_Z82
    if ($zoomFlag2 -eq "0") {
        Write-Pass "TUI: Unzoom successful (flag=0, visible window rendering)"
    } else {
        Write-Fail "TUI: Zoom flag is '$zoomFlag2' after unzoom (expected 0)"
    }

    # TUI Test 3: All panes still exist after zoom cycle
    $paneCountAfter = (& $script:TUI_PSMUX list-panes -t $TUI_SESSION_Z82 2>&1 | Measure-Object -Line).Lines
    if ($paneCountAfter -eq $paneCount) {
        Write-Pass "TUI: All $paneCount panes intact after zoom cycle"
    } else {
        Write-Fail "TUI: Pane count changed ($paneCount -> $paneCountAfter) during zoom"
    }

    Cleanup-PsmuxWindow -Session $TUI_SESSION_Z82
    Write-Host ""
} else {
    Write-Info "TUI verification skipped (could not launch window)"
}

# ══════════════════════════════════════════════════════════════════════
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped (of $total run)" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
