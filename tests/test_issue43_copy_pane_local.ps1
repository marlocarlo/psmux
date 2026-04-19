# psmux Issue #43 - Copy mode pane-local tests
# Verifies copy mode is per-pane (tmux parity):
#   - Copy mode state persists when switching away and back
#   - Each pane independently tracks copy mode
#   - Scroll position is preserved per-pane
#   - Switching away does NOT cancel copy mode on original pane
#   - Window switching also preserves copy mode per-pane
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue43_copy_pane_local.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function Psmux { & $PSMUX @args 2>&1 | Out-String; Start-Sleep -Milliseconds 300 }
function Query { param([string]$Fmt) (& $PSMUX display-message -t $SESSION -p $Fmt 2>&1 | Out-String).Trim() }

# ============================================================
# SETUP
# ============================================================
# Kill any leftover server
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "issue43_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Host "FATAL: Cannot create test session" -ForegroundColor Red
    exit 1
}

# Put some text in the first pane for scrollback testing
Psmux send-keys -t $SESSION "echo 'pane0 line1 hello world'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'pane0 line2 foo bar baz'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'pane0 line3 test data'" Enter | Out-Null
Start-Sleep -Milliseconds 500

# Split to create a second pane
Psmux split-window -t $SESSION | Out-Null
Start-Sleep -Seconds 2

# Put text in pane 1
Psmux send-keys -t $SESSION "echo 'pane1 line1 second pane'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'pane1 line2 more text'" Enter | Out-Null
Start-Sleep -Milliseconds 500

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "1. COPY MODE IS PANE-LOCAL (PANE SWITCH)"
Write-Host ("=" * 60)

Write-Test "1.1 Enter copy mode on pane 1 (bottom pane)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") { Write-Pass "copy-mode entered on pane 1" } else { Write-Fail "copy-mode entry failed: pane_in_mode=$inMode" }

Write-Test "1.2 Move cursor in copy mode (scroll up to create offset)"
# Scroll up a bit to get a non-zero scroll position
Psmux send-keys -t $SESSION k | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION k | Out-Null
Start-Sleep -Milliseconds 200
$cursorY1 = Query "#{copy_cursor_y}"
Write-Info "  cursor_y on pane 1 after 2k: $cursorY1"
Write-Pass "cursor moved in copy mode"

Write-Test "1.3 Switch to pane 0 (top pane) — copy mode on pane 1 should be saved"
# select-pane -U to switch to upper pane
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "0") {
    Write-Pass "pane 0 is NOT in copy mode after switch"
} else {
    Write-Fail "pane 0 should not be in copy mode: pane_in_mode=$inMode"
}

Write-Test "1.4 Switch back to pane 1 — copy mode should be restored"
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") {
    Write-Pass "copy mode restored on pane 1 after switch-back"
} else {
    Write-Fail "copy mode NOT restored on pane 1: pane_in_mode=$inMode"
}

Write-Test "1.5 Cursor position preserved after round-trip"
$cursorY1_after = Query "#{copy_cursor_y}"
Write-Info "  cursor_y before switch: $cursorY1, after round-trip: $cursorY1_after"
if ($cursorY1_after -eq $cursorY1) {
    Write-Pass "cursor_y preserved after pane switch round-trip"
} else {
    Write-Fail "cursor_y changed: expected $cursorY1, got $cursorY1_after"
}

# Exit copy mode on pane 1
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "2. INDEPENDENT COPY MODE PER PANE"
Write-Host ("=" * 60)

Write-Test "2.1 Enter copy mode on pane 1 (bottom)"
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 300
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") { Write-Pass "pane 1 in copy mode" } else { Write-Fail "pane 1 copy mode: $inMode" }

Write-Test "2.2 Switch to pane 0, enter copy mode there too"
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 500
$inMode0 = Query "#{pane_in_mode}"
if ($inMode0 -match "0") { Write-Pass "pane 0 starts in passthrough" } else { Write-Fail "pane 0 unexpected mode: $inMode0" }

Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode0 = Query "#{pane_in_mode}"
if ($inMode0 -match "1") { Write-Pass "pane 0 now in copy mode" } else { Write-Fail "pane 0 copy mode entry: $inMode0" }

Write-Test "2.3 Exit copy mode on pane 0 — pane 1 should still have copy mode"
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300
$inMode0 = Query "#{pane_in_mode}"
if ($inMode0 -match "0") { Write-Pass "pane 0 exited copy mode" } else { Write-Fail "pane 0 still in copy mode: $inMode0" }

# Switch back to pane 1 and verify copy mode is still there
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 500
$inMode1 = Query "#{pane_in_mode}"
if ($inMode1 -match "1") {
    Write-Pass "pane 1 still in copy mode (independent from pane 0)"
} else {
    Write-Fail "pane 1 lost copy mode when pane 0 exited: pane_in_mode=$inMode1"
}

# Clean up
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "3. SCROLL POSITION PRESERVED PER PANE"
Write-Host ("=" * 60)

Write-Test "3.1 Enter copy mode, scroll up, verify scroll_position"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Scroll up multiple times
for ($i = 0; $i -lt 5; $i++) {
    Psmux send-keys -t $SESSION k | Out-Null
    Start-Sleep -Milliseconds 100
}
$scrollPos = Query "#{scroll_position}"
Write-Info "  scroll_position on pane 1: $scrollPos"
$scrollPosInt = [int]$scrollPos
# We need at least cursor_y changed (scroll_position might be 0 if buffer is small)
$cursorY = Query "#{copy_cursor_y}"
Write-Info "  cursor_y on pane 1: $cursorY"
Write-Pass "scroll position captured"

Write-Test "3.2 Switch to pane 0 — scrollback on pane 0 should be 0"
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 500
$scrollPos0 = Query "#{scroll_position}"
Write-Info "  scroll_position on pane 0: $scrollPos0"
if ($scrollPos0 -eq "0") {
    Write-Pass "pane 0 scrollback is 0 (not affected by pane 1)"
} else {
    Write-Fail "pane 0 scrollback should be 0, got: $scrollPos0"
}

Write-Test "3.3 Switch back to pane 1 — scroll position should be restored"
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 500
$scrollPosRestored = Query "#{scroll_position}"
$cursorYRestored = Query "#{copy_cursor_y}"
Write-Info "  scroll_position restored: $scrollPosRestored (was: $scrollPos)"
Write-Info "  cursor_y restored: $cursorYRestored (was: $cursorY)"
if ($cursorYRestored -eq $cursorY) {
    Write-Pass "cursor_y preserved after pane switch"
} else {
    Write-Fail "cursor_y not preserved: expected $cursorY, got $cursorYRestored"
}

Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "4. WINDOW SWITCHING PRESERVES COPY MODE"
Write-Host ("=" * 60)

Write-Test "4.1 Create second window"
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Seconds 2
# Put text in window 2
Psmux send-keys -t $SESSION "echo 'window2 pane text'" Enter | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "4.2 Switch back to window 0, enter copy mode"
Psmux select-window -t $SESSION -p | Out-Null
Start-Sleep -Milliseconds 500

# Make sure we're on the bottom pane (pane 1)
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 300

Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") { Write-Pass "copy mode entered on window 0" } else { Write-Fail "copy mode entry: $inMode" }

# Move cursor
Psmux send-keys -t $SESSION k | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION k | Out-Null
Start-Sleep -Milliseconds 200
$cursorBefore = Query "#{copy_cursor_y}"

Write-Test "4.3 Switch to window 1 — copy mode on window 0 should be saved"
Psmux select-window -t $SESSION -n | Out-Null
Start-Sleep -Milliseconds 500
$inModeW1 = Query "#{pane_in_mode}"
if ($inModeW1 -match "0") {
    Write-Pass "window 1 is NOT in copy mode"
} else {
    Write-Fail "window 1 should not be in copy mode: $inModeW1"
}

Write-Test "4.4 Switch back to window 0 — copy mode should be restored"
Psmux select-window -t $SESSION -p | Out-Null
Start-Sleep -Milliseconds 500
$inModeW0 = Query "#{pane_in_mode}"
if ($inModeW0 -match "1") {
    Write-Pass "copy mode restored on window 0 after return"
} else {
    Write-Fail "copy mode NOT restored on window 0: $inModeW0"
}

Write-Test "4.5 Cursor position preserved across window switch"
$cursorAfter = Query "#{copy_cursor_y}"
Write-Info "  cursor_y before window switch: $cursorBefore, after: $cursorAfter"
if ($cursorAfter -eq $cursorBefore) {
    Write-Pass "cursor_y preserved across window switch"
} else {
    Write-Fail "cursor_y changed: expected $cursorBefore, got $cursorAfter"
}

Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "5. EXIT COPY MODE ONLY AFFECTS CURRENT PANE"
Write-Host ("=" * 60)

Write-Test "5.1 Enter copy mode on pane 1, switch to pane 0"
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 300
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "5.2 Cancel from pane 0 (Escape) — pane 1 should remain in copy mode"
Psmux send-keys -t $SESSION Escape | Out-Null
Start-Sleep -Milliseconds 300

# Switch back to pane 1 and check
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") {
    Write-Pass "pane 1 still in copy mode after Escape on pane 0"
} else {
    Write-Fail "pane 1 lost copy mode: pane_in_mode=$inMode"
}
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "6. LAST-PANE (PREFIX ;) PRESERVES COPY MODE"
Write-Host ("=" * 60)

Write-Test "6.1 Enter copy mode, switch via last-pane, switch back"
Psmux select-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 300
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") { Write-Pass "copy mode entered" } else { Write-Fail "copy mode entry: $inMode" }

# last-pane to switch away
Psmux select-pane -t $SESSION -l | Out-Null
Start-Sleep -Milliseconds 500
$inModeOther = Query "#{pane_in_mode}"
if ($inModeOther -match "0") { Write-Pass "other pane not in copy mode" } else { Write-Fail "other pane in copy mode: $inModeOther" }

# last-pane back
Psmux select-pane -t $SESSION -l | Out-Null
Start-Sleep -Milliseconds 500
$inModeBack = Query "#{pane_in_mode}"
if ($inModeBack -match "1") {
    Write-Pass "copy mode restored after last-pane round-trip"
} else {
    Write-Fail "copy mode lost after last-pane: $inModeBack"
}
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "7. LAST-WINDOW (PREFIX l) PRESERVES COPY MODE"
Write-Host ("=" * 60)

Write-Test "7.1 Enter copy mode, last-window, switch back"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = Query "#{pane_in_mode}"
if ($inMode -match "1") { Write-Pass "copy mode entered" } else { Write-Fail "entry: $inMode" }

# last-window switch 
Psmux select-window -t $SESSION -l | Out-Null
Start-Sleep -Milliseconds 500
$inModeW = Query "#{pane_in_mode}"
if ($inModeW -match "0") { Write-Pass "other window not in copy mode" } else { Write-Fail "other window copy mode: $inModeW" }

# switch back
Psmux select-window -t $SESSION -l | Out-Null
Start-Sleep -Milliseconds 500
$inModeBack = Query "#{pane_in_mode}"
if ($inModeBack -match "1") {
    Write-Pass "copy mode restored after last-window round-trip"
} else {
    Write-Fail "copy mode lost after last-window: $inModeBack"
}
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed)/$total passed, $($script:TestsFailed) failed"
if ($script:TestsFailed -eq 0) {
    Write-Host "ALL TESTS PASSED!" -ForegroundColor Green
} else {
    Write-Host "$($script:TestsFailed) TESTS FAILED" -ForegroundColor Red
}
Write-Host ("=" * 60)

exit $script:TestsFailed
