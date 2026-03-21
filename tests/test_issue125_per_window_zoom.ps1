#!/usr/bin/env pwsh
# test_issue125_per_window_zoom.ps1 — Verify window_zoomed_flag is per-window, not global
# https://github.com/psmux/psmux/issues/125 (follow-up: zoom flag follows focus)
#
# Reproduces the exact scenario reported by @maciakl:
#   1. Create session with 2 windows, split window 1 into two panes
#   2. Zoom current pane in window 1 → '+' should appear for window 1
#   3. Switch to window 2 → '+' should stay on window 1, NOT move to window 2
#   4. Switch back to window 1 → '+' still on window 1
#   5. Toggle zoom → '+' disappears immediately

$ErrorActionPreference = 'Continue'
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"

$script:TestsPassed = 0
$script:TestsFailed = 0
function Write-Pass($msg) { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  FAIL: $msg" -ForegroundColor Red;   $script:TestsFailed++ }
function Write-Test($msg) { Write-Host "`n[$($script:TestsPassed + $script:TestsFailed + 1)] $msg" -ForegroundColor Cyan }

$SESSION = "zoom_perwin_$(Get-Random)"

# Cleanup any leftover session
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# Create a detached session (this gives us window 0)
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

# Setup: create window 1 (new-window creates it automatically)
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

# Setup: split window 0 into two panes (go back to window 0 first)
Psmux select-window -t "${SESSION}:0" | Out-Null
Start-Sleep -Milliseconds 300
Psmux split-window -v -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

# ---------------------------------------------------------------------------
# Test 1: Before zooming — both windows show flag=0
# ---------------------------------------------------------------------------
Write-Test "Before zoom: window 0 flag=0"
Psmux select-window -t "${SESSION}:0" | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "0") { Write-Pass "window 0 flag=$val" }
else              { Write-Fail "Expected '0', got '$val'" }

# ---------------------------------------------------------------------------
# Test 2: Zoom pane in window 0 → flag=1 for window 0
# ---------------------------------------------------------------------------
Write-Test "After zoom in window 0: flag=1"
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "1") { Write-Pass "window 0 flag=$val" }
else              { Write-Fail "Expected '1', got '$val'" }

# ---------------------------------------------------------------------------
# Test 3: Conditional format shows ZOOMED for window 0
# ---------------------------------------------------------------------------
Write-Test "Conditional format shows ZOOMED for active zoomed window"
$val = Fmt '#{?window_zoomed_flag,ZOOMED,normal}'
if ($val -eq "ZOOMED") { Write-Pass "conditional=$val" }
else                   { Write-Fail "Expected 'ZOOMED', got '$val'" }

# ---------------------------------------------------------------------------
# Test 4: Switch to window 1 — window 1 should NOT show zoomed
# (BUG in old code: flag followed focus instead of staying per-window)
# ---------------------------------------------------------------------------
Write-Test "After switching to window 1: window 1 flag=0 (not zoomed)"
Psmux select-window -t "${SESSION}:1" | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "0") { Write-Pass "window 1 flag=$val (zoom did NOT follow focus)" }
else              { Write-Fail "Expected '0', got '$val' — BUG: zoom flag followed focus to window 1!" }

# ---------------------------------------------------------------------------
# Test 5: While on window 1, check window 0's zoom flag via list-windows format
# (window 0 should still be zoomed even though it's not the active window)
# ---------------------------------------------------------------------------
Write-Test "Window 0 still shows zoomed (via list-windows while on window 1)"
$listOutput = & $PSMUX list-windows -t $SESSION -F '#{window_index}:#{window_zoomed_flag}' 2>&1
$lines = ($listOutput | Out-String).Trim() -split "`n" | ForEach-Object { $_.Trim() }
$win0Flag = ($lines | Where-Object { $_ -match '^0:' }) -replace '^0:', ''
$win1Flag = ($lines | Where-Object { $_ -match '^1:' }) -replace '^1:', ''
if ($win0Flag -eq "1") { Write-Pass "list-windows: window 0 flag=$win0Flag (still zoomed)" }
else                   { Write-Fail "list-windows: window 0 expected '1', got '$win0Flag' — BUG: zoom lost on switch" }

# ---------------------------------------------------------------------------
# Test 6: Window 1 not zoomed in list-windows
# ---------------------------------------------------------------------------
Write-Test "Window 1 not zoomed in list-windows"
if ($win1Flag -eq "0") { Write-Pass "list-windows: window 1 flag=$win1Flag" }
else                   { Write-Fail "list-windows: window 1 expected '0', got '$win1Flag'" }

# ---------------------------------------------------------------------------
# Test 7: Switch back to window 0 — still zoomed
# ---------------------------------------------------------------------------
Write-Test "Switch back to window 0: flag=1 (still zoomed)"
Psmux select-window -t "${SESSION}:0" | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "1") { Write-Pass "window 0 flag=$val" }
else              { Write-Fail "Expected '1', got '$val'" }

# ---------------------------------------------------------------------------
# Test 8: Unzoom window 0 — flag=0 immediately
# ---------------------------------------------------------------------------
Write-Test "Unzoom window 0: flag=0 immediately"
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$val = Fmt '#{window_zoomed_flag}'
if ($val -eq "0") { Write-Pass "window 0 flag=$val (unzoomed)" }
else              { Write-Fail "Expected '0', got '$val'" }

# ---------------------------------------------------------------------------
# Test 9: window_flags includes 'Z' when zoomed (tmux parity)
# ---------------------------------------------------------------------------
Write-Test "window_flags includes 'Z' when zoomed"
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$flags = Fmt '#{window_flags}'
if ($flags -match 'Z') { Write-Pass "window_flags='$flags' contains Z" }
else                   { Write-Fail "window_flags='$flags' — expected Z flag" }

# Unzoom for next test
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300

# ---------------------------------------------------------------------------
# Test 10: window_flags does NOT include 'Z' when not zoomed
# ---------------------------------------------------------------------------
Write-Test "window_flags does NOT include 'Z' when not zoomed"
$flags = Fmt '#{window_flags}'
if ($flags -notmatch 'Z') { Write-Pass "window_flags='$flags' no Z" }
else                      { Write-Fail "window_flags='$flags' — Z should not be present" }

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------
Psmux kill-session -t $SESSION | Out-Null
Write-Host "`n========================================" -ForegroundColor Yellow
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -gt 0) { 'Red' } else { 'Green' })
exit $script:TestsFailed
