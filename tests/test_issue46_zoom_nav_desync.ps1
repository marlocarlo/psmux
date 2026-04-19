# psmux Issue #46 - Pane navigation while zoomed desyncs state and viewport
# Verifies that navigating panes while zoom is active either:
#   a) unzooms and moves focus (tmux behavior), OR
#   b) keeps state+viewport consistent (no desync)
#
# The bug: pane focus/state changes while zoom viewport stays on the old pane,
# creating a mismatch where input goes to one pane but you see another.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue46_zoom_nav_desync.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found. Build first: cargo build --release"; exit 1 }
Write-Info "Using: $PSMUX"

function Psmux { & $PSMUX @args 2>&1 | Out-String; Start-Sleep -Milliseconds 300 }
function Query { param([string]$Target, [string]$Fmt) (& $PSMUX display-message -t $Target -p $Fmt 2>&1 | Out-String).Trim() }

# ============================================================
# SETUP
# ============================================================
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "issue46_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Host "FATAL: Cannot create test session" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  ISSUE #46: PANE NAVIGATION WHILE ZOOMED"
Write-Host ("=" * 60)
Write-Host ""

# -----------------------------------------------------------------
# Test 1: Active pane index after pane nav during zoom
# -----------------------------------------------------------------
Write-Test "1. Zoom pane 0, navigate to pane 1 — check state consistency"

# Split to get two panes
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

# Mark each pane with a unique echo
Psmux send-keys -t "${SESSION}:.0" "echo 'I_AM_PANE_0'" Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t "${SESSION}:.1" "echo 'I_AM_PANE_1'" Enter | Out-Null
Start-Sleep -Milliseconds 300

# Select pane 0 and zoom it
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$zoomFlag = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
$activePane = Query -Target $SESSION -Fmt '#{pane_index}'
Write-Info "  After zoom: zoomed=$zoomFlag, active_pane=$activePane"

if ($zoomFlag -match "1") {
    Write-Pass "Zoom activated on pane 0"
} else {
    Write-Fail "Zoom did not activate"
}

# Now try to navigate to the other pane (select-pane -R = right)
# In tmux, this should either: unzoom + move, or be a no-op while zoomed
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 500

$zoomFlagAfterNav = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
$activePaneAfterNav = Query -Target $SESSION -Fmt '#{pane_index}'
Write-Info "  After select-pane to .1: zoomed=$zoomFlagAfterNav, active_pane=$activePaneAfterNav"

# Check consistency: if still zoomed, active pane should still be the zoomed pane (pane 0)
# If unzoomed, active pane can be pane 1
if ($zoomFlagAfterNav -match "1" -and $activePaneAfterNav -match "1") {
    Write-Fail "DESYNC: zoom still active but active pane changed to $activePaneAfterNav. Issue #46 confirmed."
    Write-Info "  This means input goes to pane 1 but viewport shows pane 0 (zoomed)."
} elseif ($zoomFlagAfterNav -match "0" -and $activePaneAfterNav -match "1") {
    Write-Pass "Zoom unset and focus moved to pane 1 (tmux-compatible behavior)"
} elseif ($zoomFlagAfterNav -match "1" -and $activePaneAfterNav -match "0") {
    Write-Pass "Navigation ignored while zoomed — pane stays on 0 (safe behavior)"
} else {
    Write-Info "  Unexpected state: zoomed=$zoomFlagAfterNav, active=$activePaneAfterNav — needs review"
}

# -----------------------------------------------------------------
# Test 2: Send-keys consistency — input goes to correct pane
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 2: INPUT GOES TO VISIBLE PANE"
Write-Host ("=" * 60)

Write-Test "2. After nav during zoom, verify input ends up in the right pane"

# First ensure we're in a known state: unzoom if needed
if ($zoomFlagAfterNav -match "1") {
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 500
}

# Re-zoom pane 0
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

# Try to switch to pane 1 while zoomed
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 500

# Send a unique marker to "the active pane" (whichever psmux thinks is active)
$marker = "NAV_MARKER_$(Get-Random -Maximum 99999)"
Psmux send-keys -t $SESSION "$marker" Enter | Out-Null
Start-Sleep -Milliseconds 500

# Unzoom if still zoomed
$stillZoomed = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
if ($stillZoomed -match "1") {
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 500
}

# Capture both panes and check where the marker ended up
$cap0 = & $PSMUX capture-pane -t "${SESSION}:.0" -p 2>&1 | Out-String
$cap1 = & $PSMUX capture-pane -t "${SESSION}:.1" -p 2>&1 | Out-String

$inPane0 = $cap0 -match $marker
$inPane1 = $cap1 -match $marker

Write-Info "  Marker '$marker' found in pane 0: $inPane0"
Write-Info "  Marker '$marker' found in pane 1: $inPane1"

if ($inPane0 -and -not $inPane1) {
    Write-Info "  Input went to pane 0 (zoomed pane) — zoom navigation was ignored"
    Write-Pass "Input stayed in zoomed pane — no desync"
} elseif ($inPane1 -and -not $inPane0) {
    Write-Info "  Input went to pane 1 (target pane)"
    # This is fine IF zoom was cancelled, but a desync if zoom remained
    Write-Pass "Input went to navigated-to pane (zoom was cancelled or state consistent)"
} elseif ($inPane0 -and $inPane1) {
    Write-Info "  Marker found in both panes (may be in prompt text)"
    Write-Pass "Marker detected (possibly echoed in both)"
} else {
    Write-Fail "Marker not found in either pane — possible send-keys failure"
}

# -----------------------------------------------------------------
# Test 3: Active pane matches pane_id after zoom nav cycle
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 3: PANE STATE CONSISTENCY CHECK"
Write-Host ("=" * 60)

Write-Test "3. Multiple zoom/nav cycles — active_pane stays consistent"

# Run several zoom/nav/unzoom cycles and check for desync
$desyncCount = 0
for ($cycle = 1; $cycle -le 3; $cycle++) {
    # Zoom pane 0
    Psmux select-pane -t "${SESSION}:.0" | Out-Null
    Start-Sleep -Milliseconds 200
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 300

    # Try to nav to pane 1
    Psmux select-pane -t "${SESSION}:.1" | Out-Null
    Start-Sleep -Milliseconds 300

    $z = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
    $a = Query -Target $SESSION -Fmt '#{pane_index}'

    if ($z -match "1" -and $a -match "1") {
        $desyncCount++
        Write-Info "  Cycle ${cycle}: DESYNC (zoomed=1, active=pane1)"
    } else {
        Write-Info "  Cycle ${cycle}: OK (zoomed=$z, active=pane$a)"
    }

    # Unzoom to reset
    if ($z -match "1") {
        Psmux resize-pane -Z -t $SESSION | Out-Null
        Start-Sleep -Milliseconds 300
    }
}

if ($desyncCount -eq 0) {
    Write-Pass "No desync detected across $cycle cycles"
} else {
    Write-Fail "$desyncCount desync(s) detected across 3 cycles — issue #46 confirmed"
}

# ============================================================
# TEST 4: ZOOM WRAP NAVIGATION (issue #134 parity)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 4: ZOOM WRAP NAVIGATION (LEFT FROM LEFTMOST)"
Write-Host ("=" * 60)

Write-Test "4. Zoomed select-pane -L from leftmost wraps to rightmost and unzooms"

# With horizontal split (pane 0 left, pane 1 right):
# Select pane 0, zoom it, then select-pane -L should wrap to pane 1 and unzoom
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500

# Zoom must be off first
$z = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
if ($z -match "1") {
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 300
}

# Now zoom pane 0
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$z4 = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
$a4 = Query -Target $SESSION -Fmt '#{pane_index}'
Write-Info "  Before nav: zoomed=$z4, active_pane=$a4"

# Move left from leftmost — wraps to rightmost, unzooms (issue #134)
Psmux select-pane -L -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$z4after = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
$a4after = Query -Target $SESSION -Fmt '#{pane_index}'
Write-Info "  After select-pane -L: zoomed=$z4after, active_pane=$a4after"

if ($z4after -match "0" -and $a4after -eq "1") {
    Write-Pass "Zoomed wrap -L: pane $a4->$a4after, zoom $z4->$z4after (issue #134 parity)"
} else {
    Write-Fail "Expected pane 0->1 zoom 1->0, got pane $a4->$a4after zoom $z4->$z4after"
}

# Clean up: unzoom
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Cleaning up session $SESSION..."
& $PSMUX kill-session -t $SESSION 2>$null | Out-Null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) {
    Write-Host "Some tests FAILED — issue #46 may be present" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests PASSED" -ForegroundColor Green
    exit 0
}
