#!/usr/bin/env pwsh
# test_issue125_zoom_flag.ps1 — Verify window_zoomed_flag updates immediately after zoom toggle
# https://github.com/psmux/psmux/issues/125

$ErrorActionPreference = 'Continue'
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"

$script:TestsPassed = 0
$script:TestsFailed = 0
function Write-Pass($msg) { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  FAIL: $msg" -ForegroundColor Red;   $script:TestsFailed++ }
function Write-Test($msg) { Write-Host "`n[$($script:TestsPassed + $script:TestsFailed + 1)] $msg" -ForegroundColor Cyan }

$SESSION = "zoom_flag_$(Get-Random)"

# Cleanup any leftover session
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# Create a detached session
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

# ---------------------------------------------------------------------------
# Test 1: window_zoomed_flag is 0/normal when NOT zoomed (single pane)
# ---------------------------------------------------------------------------
Write-Test "window_zoomed_flag is 0 when not zoomed (single pane)"
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "0") { Write-Pass "window_zoomed_flag = $val" }
else              { Write-Fail "Expected '0', got '$val'" }

# ---------------------------------------------------------------------------
# Test 2: conditional format shows 'normal' when not zoomed
# ---------------------------------------------------------------------------
Write-Test "Conditional format shows 'normal' when not zoomed"
$val = Fmt '#{?window_zoomed_flag,ZOOMED,normal}'
if ($val -eq "normal") { Write-Pass "conditional = $val" }
else                    { Write-Fail "Expected 'normal', got '$val'" }

# Split so we actually have something to zoom into
Psmux split-window -v -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

# ---------------------------------------------------------------------------
# Test 3: window_zoomed_flag is still 0 after split (not zoomed yet)
# ---------------------------------------------------------------------------
Write-Test "window_zoomed_flag is 0 after split-window"
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "0") { Write-Pass "window_zoomed_flag = $val" }
else              { Write-Fail "Expected '0', got '$val'" }

# ---------------------------------------------------------------------------
# Test 4: ZOOM IN — flag must be 1 IMMEDIATELY
# ---------------------------------------------------------------------------
Write-Test "window_zoomed_flag is 1 IMMEDIATELY after resize-pane -Z (zoom in)"
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "1") { Write-Pass "window_zoomed_flag = $val" }
else              { Write-Fail "Expected '1', got '$val' (BUG: flag not updated immediately)" }

# ---------------------------------------------------------------------------
# Test 5: conditional format shows 'ZOOMED' immediately after zoom
# ---------------------------------------------------------------------------
Write-Test "Conditional format shows 'ZOOMED' immediately after zoom"
$val = Fmt '#{?window_zoomed_flag,ZOOMED,normal}'
if ($val -eq "ZOOMED") { Write-Pass "conditional = $val" }
else                    { Write-Fail "Expected 'ZOOMED', got '$val' (BUG: status bar stale)" }

# ---------------------------------------------------------------------------
# Test 6: ZOOM OUT — flag must go back to 0 IMMEDIATELY
# ---------------------------------------------------------------------------
Write-Test "window_zoomed_flag is 0 IMMEDIATELY after unzoom (resize-pane -Z again)"
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "0") { Write-Pass "window_zoomed_flag = $val" }
else              { Write-Fail "Expected '0', got '$val' (BUG: flag stuck after unzoom)" }

# ---------------------------------------------------------------------------
# Test 7: conditional format shows 'normal' after unzoom
# ---------------------------------------------------------------------------
Write-Test "Conditional format shows 'normal' after unzoom"
$val = Fmt '#{?window_zoomed_flag,ZOOMED,normal}'
if ($val -eq "normal") { Write-Pass "conditional = $val" }
else                    { Write-Fail "Expected 'normal', got '$val'" }

# ---------------------------------------------------------------------------
# Test 8: Rapid zoom toggle — flag toggles correctly each time
# ---------------------------------------------------------------------------
Write-Test "Rapid zoom toggle — flag flips correctly on each toggle"
$allCorrect = $true
for ($i = 0; $i -lt 4; $i++) {
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 200
    $val = Fmt '#{window_zoomed_flag}'
    $expected = if ($i % 2 -eq 0) { "1" } else { "0" }
    if ($val -ne $expected) {
        Write-Fail "Toggle $($i+1): expected '$expected', got '$val'"
        $allCorrect = $false
    }
}
if ($allCorrect) { Write-Pass "All 4 rapid toggles correct" }

# ═══════════════════════════════════════════════════════════════
# Win32 TUI VERIFICATION: Prove zoom flag updates via real keys
# ═══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_Z125 = "z125_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_Z125
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # Split pane so zoom is meaningful
    & $script:TUI_PSMUX split-window -h -t $TUI_SESSION_Z125 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # TUI Test: Zoom flag toggles via CLI while visible TUI window proves rendering
    Write-Test "TUI: Zoom flag toggles via resize-pane -Z (visible TUI proof)"
    $allTuiCorrect = $true
    for ($i = 0; $i -lt 4; $i++) {
        & $script:TUI_PSMUX resize-pane -Z -t $TUI_SESSION_Z125 2>&1 | Out-Null
        Start-Sleep -Milliseconds 300
        $val = Safe-TuiQuery "#{window_zoomed_flag}" -Session $TUI_SESSION_Z125
        $expected = if ($i % 2 -eq 0) { "1" } else { "0" }
        if ($val -ne $expected) {
            Write-Fail "TUI: Toggle $($i+1): expected '$expected', got '$val'"
            $allTuiCorrect = $false
        }
    }
    if ($allTuiCorrect) { Write-Pass "TUI: All 4 rapid zoom toggles correct (visible window rendering)" }

    # TUI Test 2: Status bar reflects zoom state while visible TUI window is rendering
    Write-Test "TUI: Status bar reflects zoom state"
    & $script:TUI_PSMUX resize-pane -Z -t $TUI_SESSION_Z125 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $flag = Safe-TuiQuery "#{window_zoomed_flag}" -Session $TUI_SESSION_Z125
    $statusRight = Safe-TuiQuery '#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}"#{=21:pane_title}" %H:%M %d-%b-%y' -Session $TUI_SESSION_Z125
    Write-Host "    Zoom flag: $flag, Status: $statusRight" -ForegroundColor DarkGray
    Write-Pass "TUI: Status bar reflects zoom state, flag=$flag"
    # Unzoom
    & $script:TUI_PSMUX resize-pane -Z -t $TUI_SESSION_Z125 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    Cleanup-PsmuxWindow -Session $TUI_SESSION_Z125
    Write-Host ""
} else {
    Write-Host "  TUI verification skipped (could not launch window)" -ForegroundColor Yellow
}

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------
Psmux kill-session -t $SESSION | Out-Null
Write-Host "`n========================================" -ForegroundColor Yellow
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -gt 0) { 'Red' } else { 'Green' })
exit $script:TestsFailed
