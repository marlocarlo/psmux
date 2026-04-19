# psmux Issue #45 - Restored pane shows cursor row at top after unzoom
# Verifies that after zooming pane 1 (hiding pane 0) and then unzooming,
# pane 0's cursor position is preserved where it was before zoom.
#
# Reproduction:
#   1. Split into two panes
#   2. In pane 0, move the cursor down (press Enter several times)
#   3. Switch to pane 1 and zoom it (hides pane 0)
#   4. Unzoom
#   5. Check that pane 0's cursor_y is NOT at row 0
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue45_unzoom_cursor.ps1

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

$SESSION = "issue45_$(Get-Random -Maximum 9999)"
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
Write-Host "  ISSUE #45: CURSOR POSITION AFTER UNZOOM"
Write-Host ("=" * 60)
Write-Host ""

# -----------------------------------------------------------------
# Test 1: Cursor Y preserved after zoom/unzoom
# -----------------------------------------------------------------
Write-Test "1. Cursor position preserved in hidden pane after zoom/unzoom"

# Split horizontally
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

# Select pane 0
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500

# Move cursor down by pressing Enter several times (creates blank lines, moves prompt down)
for ($i = 0; $i -lt 8; $i++) {
    Psmux send-keys -t "${SESSION}:.0" "" Enter | Out-Null
    Start-Sleep -Milliseconds 100
}
Start-Sleep -Milliseconds 500

# Record pane 0 cursor_y before zoom
$cursorYBefore = Query -Target "${SESSION}:.0" -Fmt '#{cursor_y}'
Write-Info "  cursor_y in pane 0 BEFORE zoom = $cursorYBefore"

# Now switch to pane 1 and zoom it (hiding pane 0)
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$zoomFlag = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
Write-Info "  window_zoomed_flag = $zoomFlag"
if ($zoomFlag -match "1") {
    Write-Pass "Zoom activated"
} else {
    Write-Fail "Zoom not activated"
}

# Wait a moment while pane 0 is hidden
Start-Sleep -Seconds 1

# Unzoom
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Seconds 1

# Check pane 0 cursor_y after unzoom
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500
$cursorYAfter = Query -Target "${SESSION}:.0" -Fmt '#{cursor_y}'
Write-Info "  cursor_y in pane 0 AFTER unzoom = $cursorYAfter"

# The cursor should NOT be at row 0 — it should be near where it was before zoom
if ([int]$cursorYAfter -eq 0 -and [int]$cursorYBefore -gt 2) {
    Write-Fail "cursor_y reset to 0 after unzoom (was $cursorYBefore before zoom). Issue #45 confirmed."
} elseif ([Math]::Abs([int]$cursorYAfter - [int]$cursorYBefore) -le 2) {
    Write-Pass "cursor_y preserved after unzoom (before=$cursorYBefore, after=$cursorYAfter)"
} else {
    Write-Info "  cursor_y shifted (before=$cursorYBefore, after=$cursorYAfter) — may be acceptable"
    # A small shift is tolerable if the pane was resized during zoom, but large shifts indicate a bug
    if ([int]$cursorYAfter -eq 0) {
        Write-Fail "cursor_y snapped to top of pane (0) — issue #45"
    } else {
        Write-Pass "cursor_y did not snap to 0 (before=$cursorYBefore, after=$cursorYAfter)"
    }
}

# -----------------------------------------------------------------
# Test 2: Cursor position with output (not just blank lines)
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 2: CURSOR POSITION WITH COMMAND OUTPUT"
Write-Host ("=" * 60)

Write-Test "2. Cursor position preserved when pane has command output"

# Generate output in pane 0 to push cursor further down
for ($i = 1; $i -le 5; $i++) {
    Psmux send-keys -t "${SESSION}:.0" "echo 'output line $i'" Enter | Out-Null
    Start-Sleep -Milliseconds 200
}
Start-Sleep -Milliseconds 500

$cursorYBefore2 = Query -Target "${SESSION}:.0" -Fmt '#{cursor_y}'
Write-Info "  cursor_y in pane 0 BEFORE zoom = $cursorYBefore2"

# Zoom pane 1 again
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Seconds 1

# Unzoom
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Seconds 1

# Check cursor
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500
$cursorYAfter2 = Query -Target "${SESSION}:.0" -Fmt '#{cursor_y}'
Write-Info "  cursor_y in pane 0 AFTER unzoom = $cursorYAfter2"

if ([int]$cursorYAfter2 -eq 0 -and [int]$cursorYBefore2 -gt 2) {
    Write-Fail "cursor_y reset to 0 after second zoom cycle (was $cursorYBefore2). Issue #45 confirmed."
} else {
    Write-Pass "cursor_y not at 0 after second zoom cycle (before=$cursorYBefore2, after=$cursorYAfter2)"
}

# -----------------------------------------------------------------
# Test 3: Visual content check — prompt should NOT be at top
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 3: VISUAL CONTENT CHECK AFTER UNZOOM"
Write-Host ("=" * 60)

Write-Test "3. Content check — prompt area should not be at top of pane"

# Put a clear marker at the current position
Psmux send-keys -t "${SESSION}:.0" "echo 'CURSOR_POS_MARKER'" Enter | Out-Null
Start-Sleep -Milliseconds 500

# Zoom and unzoom again
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Seconds 1
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Seconds 1

# Capture pane 0 and verify the marker is present
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500
$captured = & $PSMUX capture-pane -t "${SESSION}:.0" -p 2>&1 | Out-String
$lines = $captured -split "`n"

# Find where CURSOR_POS_MARKER appears
$markerLine = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match "CURSOR_POS_MARKER") {
        $markerLine = $i
        break
    }
}
Write-Info "  CURSOR_POS_MARKER found at line index $markerLine (out of $($lines.Count) lines)"

if ($markerLine -gt 0) {
    Write-Pass "Marker is not at top of pane (line $markerLine)"
} elseif ($markerLine -eq 0) {
    Write-Fail "Marker at line 0 — pane content shifted to top after unzoom (issue #45)"
} else {
    Write-Fail "CURSOR_POS_MARKER not found in capture"
}

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
    Write-Host "Some tests FAILED — issue #45 may be present" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests PASSED" -ForegroundColor Green
    exit 0
}
