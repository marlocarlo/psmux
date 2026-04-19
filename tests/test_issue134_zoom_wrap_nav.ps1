#!/usr/bin/env pwsh
# test_issue134_zoom_wrap_nav.ps1 — Verify wrapped directional navigation while zoomed
# https://github.com/psmux/psmux/issues/134
#
# When a pane is zoomed, wrapped directional pane navigation (select-pane -L/-R/-U/-D)
# should unzoom and wrap to the opposite edge pane (tmux parity).

$ErrorActionPreference = 'Continue'
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"

$script:TestsPassed = 0
$script:TestsFailed = 0
function Write-Pass($msg) { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  FAIL: $msg" -ForegroundColor Red;   $script:TestsFailed++ }
function Write-Test($msg) { Write-Host "`n[$($script:TestsPassed + $script:TestsFailed + 1)] $msg" -ForegroundColor Cyan }

$SESSION = "issue134_$(Get-Random)"

# Cleanup any leftover
& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep -Seconds 1

# Create detached session
Write-Host "`nCreating session '$SESSION'..." -ForegroundColor Yellow
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$hasSession = & $PSMUX has-session -t $SESSION 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Cannot create session '$SESSION'" -ForegroundColor Red
    exit 1
}

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }
function Fmt { param($f) (& $PSMUX display-message -t $SESSION -p "$f" 2>&1 | Out-String).Trim() }

# Split horizontally => two panes: left (%0, index 0) and right (%1, index 1)
Psmux splitw -h -t $SESSION | Out-Null
Start-Sleep -Seconds 1

# Confirm we have two panes
$paneCount = (Fmt '#{window_panes}')
if ($paneCount -ne "2") {
    Write-Host "ERROR: Expected 2 panes, got '$paneCount'" -ForegroundColor Red
    & $PSMUX kill-session -t $SESSION 2>$null
    exit 1
}

# Active pane should be the right one (index 1) after splitw -h
$activeIndex = (Fmt '#{pane_index}')
Write-Host "Active pane index after split: $activeIndex" -ForegroundColor Gray

# ---------------------------------------------------------------------------
# Test 1: Non-zoomed wrap works (control check)
# From rightmost pane, select-pane -R should wrap to leftmost pane
# ---------------------------------------------------------------------------
Write-Test "Non-zoomed: select-pane -R from rightmost wraps to leftmost"
# Ensure we're on the right pane (index 1)
Psmux select-pane -t "${SESSION}:.1" | Out-Null
$before = Fmt '#{pane_index}'
Psmux select-pane -R -t $SESSION | Out-Null
$after = Fmt '#{pane_index}'
if ($before -eq "1" -and $after -eq "0") {
    Write-Pass "Non-zoomed wrap: pane $before -> $after"
} else {
    Write-Fail "Expected 1->0, got $before->$after"
}

# ---------------------------------------------------------------------------
# Test 2: Zoomed direct neighbor navigation works (control check)
# From right pane zoomed, select-pane -L goes to left pane and unzooms
# ---------------------------------------------------------------------------
Write-Test "Zoomed: select-pane -L from right pane navigates and unzooms"
# Move back to right pane
Psmux select-pane -t "${SESSION}:.1" | Out-Null
# Zoom
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 200
$zoomBefore = Fmt '#{window_zoomed_flag}'
$paneBefore = Fmt '#{pane_index}'
# Navigate left (direct neighbor, no wrap needed)
Psmux select-pane -L -t $SESSION | Out-Null
$paneAfter = Fmt '#{pane_index}'
$zoomAfter = Fmt '#{window_zoomed_flag}'
if ($paneBefore -eq "1" -and $paneAfter -eq "0" -and $zoomBefore -eq "1" -and $zoomAfter -eq "0") {
    Write-Pass "Zoomed -L: pane $paneBefore->$paneAfter, zoom $zoomBefore->$zoomAfter"
} else {
    Write-Fail "Expected pane 1->0 zoom 1->0, got pane $paneBefore->$paneAfter zoom $zoomBefore->$zoomAfter"
}

# ---------------------------------------------------------------------------
# Test 3 (THE BUG): Zoomed wrapped navigation
# From right pane zoomed, select-pane -R should wrap to left pane and unzoom
# ---------------------------------------------------------------------------
Write-Test "Zoomed: select-pane -R from rightmost wraps to leftmost and unzooms (issue #134)"
# Move to right pane and zoom
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 200
$zoomBefore = Fmt '#{window_zoomed_flag}'
$paneBefore = Fmt '#{pane_index}'
# Wrapped navigation: going right from the rightmost pane
Psmux select-pane -R -t $SESSION | Out-Null
$paneAfter = Fmt '#{pane_index}'
$zoomAfter = Fmt '#{window_zoomed_flag}'
if ($paneBefore -eq "1" -and $paneAfter -eq "0" -and $zoomBefore -eq "1" -and $zoomAfter -eq "0") {
    Write-Pass "Zoomed wrap -R: pane $paneBefore->$paneAfter, zoom $zoomBefore->$zoomAfter"
} else {
    Write-Fail "Expected pane 1->0 zoom 1->0, got pane $paneBefore->$paneAfter zoom $zoomBefore->$zoomAfter"
}

# ---------------------------------------------------------------------------
# Test 4: Zoomed wrapped navigation -L from leftmost
# From left pane zoomed, select-pane -L should wrap to right pane and unzoom
# ---------------------------------------------------------------------------
Write-Test "Zoomed: select-pane -L from leftmost wraps to rightmost and unzooms"
# Move to left pane and zoom
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 200
$zoomBefore = Fmt '#{window_zoomed_flag}'
$paneBefore = Fmt '#{pane_index}'
# Wrapped navigation: going left from the leftmost pane
Psmux select-pane -L -t $SESSION | Out-Null
$paneAfter = Fmt '#{pane_index}'
$zoomAfter = Fmt '#{window_zoomed_flag}'
if ($paneBefore -eq "0" -and $paneAfter -eq "1" -and $zoomBefore -eq "1" -and $zoomAfter -eq "0") {
    Write-Pass "Zoomed wrap -L: pane $paneBefore->$paneAfter, zoom $zoomBefore->$zoomAfter"
} else {
    Write-Fail "Expected pane 0->1 zoom 1->0, got pane $paneBefore->$paneAfter zoom $zoomBefore->$zoomAfter"
}

# ---------------------------------------------------------------------------
# Test 5: Vertical layout: zoomed wrap -D from bottom pane
# ---------------------------------------------------------------------------
Write-Test "Zoomed vertical: select-pane -D from bottom wraps to top and unzooms"
# Create a new window with vertical split
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Seconds 1
Psmux splitw -v -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Active pane is bottom (index 1). Zoom it.
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 200
$zoomBefore = Fmt '#{window_zoomed_flag}'
$paneBefore = Fmt '#{pane_index}'
# Wrapped down from bottom → should wrap to top
Psmux select-pane -D -t $SESSION | Out-Null
$paneAfter = Fmt '#{pane_index}'
$zoomAfter = Fmt '#{window_zoomed_flag}'
if ($paneBefore -eq "1" -and $paneAfter -eq "0" -and $zoomBefore -eq "1" -and $zoomAfter -eq "0") {
    Write-Pass "Zoomed wrap -D: pane $paneBefore->$paneAfter, zoom $zoomBefore->$zoomAfter"
} else {
    Write-Fail "Expected pane 1->0 zoom 1->0, got pane $paneBefore->$paneAfter zoom $zoomBefore->$zoomAfter"
}

# ---------------------------------------------------------------------------
# Test 6: Vertical layout: zoomed wrap -U from top pane
# ---------------------------------------------------------------------------
Write-Test "Zoomed vertical: select-pane -U from top wraps to bottom and unzooms"
# Move to top pane and zoom
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 200
$zoomBefore = Fmt '#{window_zoomed_flag}'
$paneBefore = Fmt '#{pane_index}'
# Wrapped up from top → should wrap to bottom
Psmux select-pane -U -t $SESSION | Out-Null
$paneAfter = Fmt '#{pane_index}'
$zoomAfter = Fmt '#{window_zoomed_flag}'
if ($paneBefore -eq "0" -and $paneAfter -eq "1" -and $zoomBefore -eq "1" -and $zoomAfter -eq "0") {
    Write-Pass "Zoomed wrap -U: pane $paneBefore->$paneAfter, zoom $zoomBefore->$zoomAfter"
} else {
    Write-Fail "Expected pane 0->1 zoom 1->0, got pane $paneBefore->$paneAfter zoom $zoomBefore->$zoomAfter"
}

# ═══════════════════════════════════════════════════════════════
# Win32 TUI VERIFICATION: Prove zoom wrap navigation via real keys
# ═══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_Z134 = "z134_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_Z134
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # Create 2-pane horizontal layout
    & $script:TUI_PSMUX split-window -h -t $TUI_SESSION_Z134 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # TUI Test 1: Zoom + navigate right wraps and unzooms (CLI with visible TUI)
    Write-Test "TUI: Zoomed navigation wrap via CLI (visible TUI proof)"
    # Zoom current pane
    & $script:TUI_PSMUX resize-pane -Z -t $TUI_SESSION_Z134 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $zoomBefore = Safe-TuiQuery "#{window_zoomed_flag}" -Session $TUI_SESSION_Z134
    $paneBefore = Safe-TuiQuery "#{pane_index}" -Session $TUI_SESSION_Z134
    Write-Host "    Before nav: pane=$paneBefore, zoom=$zoomBefore" -ForegroundColor DarkGray

    # Navigate right (should wrap and unzoom)
    & $script:TUI_PSMUX select-pane -R -t $TUI_SESSION_Z134 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $paneAfter = Safe-TuiQuery "#{pane_index}" -Session $TUI_SESSION_Z134
    $zoomAfter = Safe-TuiQuery "#{window_zoomed_flag}" -Session $TUI_SESSION_Z134
    Write-Host "    After nav:  pane=$paneAfter, zoom=$zoomAfter" -ForegroundColor DarkGray

    if ($paneAfter -ne $paneBefore) {
        Write-Pass "TUI: Navigation moved pane ($paneBefore -> $paneAfter)"
    } else {
        Write-Fail "TUI: Navigation did not move pane (stayed at $paneBefore)"
    }

    if ($zoomAfter -eq "0") {
        Write-Pass "TUI: Zoom cleared after navigation (zoom=$zoomAfter)"
    } else {
        Write-Fail "TUI: Zoom not cleared (zoom=$zoomAfter)"
    }

    Cleanup-PsmuxWindow -Session $TUI_SESSION_Z134
    Write-Host ""
} else {
    Write-Host "  TUI verification skipped (could not launch window)" -ForegroundColor Yellow
}

# Cleanup
& $PSMUX kill-session -t $SESSION 2>$null

# Summary
Write-Host "`n========================================" -ForegroundColor White
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed" `
    -ForegroundColor $(if ($script:TestsFailed -gt 0) { 'Red' } else { 'Green' })
Write-Host "========================================" -ForegroundColor White
exit $script:TestsFailed
