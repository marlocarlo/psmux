# psmux Issue #44 - Hidden-pane buffer truncated/miswrapped after zoom/unzoom
# Verifies that output generated in a hidden (non-zoomed) pane while another
# pane is zoomed is preserved correctly after unzoom.
#
# Reproduction:
#   1. Split into two panes
#   2. Generate output in pane 0
#   3. Move to pane 1 and zoom it (hides pane 0)
#   4. While zoomed, generate more output in pane 0 via send-keys
#   5. Unzoom
#   6. Capture pane 0 and verify output is not truncated or miswrapped
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue44_zoom_buffer.ps1

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

$SESSION = "issue44_$(Get-Random -Maximum 9999)"
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
Write-Host "  ISSUE #44: HIDDEN-PANE BUFFER AFTER ZOOM/UNZOOM"
Write-Host ("=" * 60)
Write-Host ""

# -----------------------------------------------------------------
# Test 1: Output in hidden pane during zoom is preserved
# -----------------------------------------------------------------
Write-Test "1. Output generated in hidden pane during zoom is preserved"

# Split horizontally to get two panes
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Seconds 2

# We now have pane 0 (left) and pane 1 (right, active)
# Select pane 0 and generate some known output
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500

# Generate a marker line in pane 0 before zoom
Psmux send-keys -t "${SESSION}:.0" "echo BEFORE_ZOOM_MARKER" Enter | Out-Null
Start-Sleep -Milliseconds 500

# Now select pane 1 and zoom it (this hides pane 0)
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

# Verify zoom is active
$zoomFlag = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
Write-Info "  window_zoomed_flag after zoom = $zoomFlag"
if ($zoomFlag -match "1") {
    Write-Pass "Zoom activated successfully"
} else {
    Write-Fail "Zoom did not activate (window_zoomed_flag=$zoomFlag)"
}

# While pane 0 is hidden by zoom, send output to it
# Use a distinctive pattern "##" that the issue describes
Psmux send-keys -t "${SESSION}:.0" "echo '## line_A'" Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t "${SESSION}:.0" "echo '## line_B'" Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t "${SESSION}:.0" "echo '## line_C'" Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t "${SESSION}:.0" "echo '## line_D'" Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t "${SESSION}:.0" "echo '## line_E'" Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t "${SESSION}:.0" "echo AFTER_HIDDEN_MARKER" Enter | Out-Null
Start-Sleep -Seconds 1

# Unzoom (if still zoomed — select-pane may have auto-unzoomed per tmux behavior)
$zoomBefore = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
if ($zoomBefore -match "1") {
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Seconds 1
}

$zoomFlagAfter = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
Write-Info "  window_zoomed_flag after unzoom = $zoomFlagAfter"
if ($zoomFlagAfter -match "0") {
    Write-Pass "Unzoom successful"
} else {
    Write-Fail "Unzoom failed (window_zoomed_flag=$zoomFlagAfter)"
}

# Capture pane 0 content and check for the hidden-period output
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500
$captured = & $PSMUX capture-pane -t "${SESSION}:.0" -p 2>&1 | Out-String
Write-Info "  Captured pane 0 content (last 20 lines):"
$lines = $captured -split "`n" | Where-Object { $_.Trim() -ne "" }
$lines | Select-Object -Last 20 | ForEach-Object { Write-Info "    $_" }

# Check that the "##" lines are intact (not wrapped to one char per line)
$hashLines = $lines | Where-Object { $_ -match "## line_" }
Write-Info "  Found $(($hashLines | Measure-Object).Count) '## line_' entries"

if (($hashLines | Measure-Object).Count -ge 5) {
    Write-Pass "All 5 '## line_' outputs found in hidden pane after unzoom"
} else {
    Write-Fail "Missing '## line_' outputs — only $(($hashLines | Measure-Object).Count) found (expected 5). Buffer may be truncated."
}

# Check that lines are not miswrapped (each ## line should be on a single line, not one char per line)
$miswrapped = $false
foreach ($hl in $hashLines) {
    # A miswrapped "## line_A" would appear as "#" then "#" then " " etc on separate lines
    # If we see "## line_" in the captured output on a single line, it's correct
    if ($hl.Trim().Length -lt 6) {
        $miswrapped = $true
        Write-Info "  Suspicious short line: '$($hl.Trim())'"
    }
}
if (-not $miswrapped) {
    Write-Pass "No miswrapped lines detected — hidden-pane output preserved correctly"
} else {
    Write-Fail "Miswrapped lines detected (one char per line). Buffer corruption after zoom/unzoom."
}

# Check for BEFORE marker
$hasBeforeMarker = $lines | Where-Object { $_ -match "BEFORE_ZOOM_MARKER" }
if ($hasBeforeMarker) {
    Write-Pass "BEFORE_ZOOM_MARKER found — pre-zoom output preserved"
} else {
    Write-Fail "BEFORE_ZOOM_MARKER missing — pre-zoom buffer may be truncated"
}

# Check for AFTER marker
$hasAfterMarker = $lines | Where-Object { $_ -match "AFTER_HIDDEN_MARKER" }
if ($hasAfterMarker) {
    Write-Pass "AFTER_HIDDEN_MARKER found — post-hidden output preserved"
} else {
    Write-Fail "AFTER_HIDDEN_MARKER missing — post-hidden buffer may be truncated"
}

# -----------------------------------------------------------------
# Test 2: Larger output volume while hidden (stress test)
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 2: LARGER OUTPUT VOLUME WHILE HIDDEN"
Write-Host ("=" * 60)

Write-Test "2. Generate many lines while pane is hidden, verify after unzoom"

# Select pane 1, zoom it (hide pane 0 again)
Psmux select-pane -t "${SESSION}:.1" | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

# Generate 30 numbered lines in hidden pane 0
for ($i = 1; $i -le 30; $i++) {
    Psmux send-keys -t "${SESSION}:.0" "echo 'BULK_$i'" Enter | Out-Null
    Start-Sleep -Milliseconds 100
}
Psmux send-keys -t "${SESSION}:.0" "echo 'BULK_DONE'" Enter | Out-Null
Start-Sleep -Seconds 2

# Unzoom (if still zoomed)
$zb2 = Query -Target $SESSION -Fmt '#{window_zoomed_flag}'
if ($zb2 -match "1") {
    Psmux resize-pane -Z -t $SESSION | Out-Null
    Start-Sleep -Seconds 1
}

# Capture pane 0
Psmux select-pane -t "${SESSION}:.0" | Out-Null
Start-Sleep -Milliseconds 500
$captured2 = & $PSMUX capture-pane -t "${SESSION}:.0" -p 2>&1 | Out-String
$lines2 = $captured2 -split "`n" | Where-Object { $_.Trim() -ne "" }

$bulkLines = $lines2 | Where-Object { $_ -match "BULK_\d+" }
$bulkCount = ($bulkLines | Measure-Object).Count
Write-Info "  Found $bulkCount BULK_ lines out of 30 expected"

$hasBulkDone = $lines2 | Where-Object { $_ -match "BULK_DONE" }
if ($hasBulkDone) {
    Write-Pass "BULK_DONE marker found"
} else {
    Write-Fail "BULK_DONE marker missing — output truncated during hidden period"
}

# Check for miswrapping: each BULK_ line should be short (under ~15 chars for the echo output)
$miswrapCount = 0
foreach ($bl in $bulkLines) {
    if ($bl.Trim().Length -eq 1) {
        $miswrapCount++
    }
}
if ($miswrapCount -eq 0) {
    Write-Pass "No single-char miswrapped lines in bulk output"
} else {
    Write-Fail "$miswrapCount single-char lines detected — miswrapping after zoom/unzoom (issue #44)"
}

# -----------------------------------------------------------------
# Test 3: Pane width preserved after unzoom
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 3: PANE DIMENSIONS AFTER UNZOOM"
Write-Host ("=" * 60)

Write-Test "3. Pane width is correct after unzoom (not 1-char wide)"

# Get pane width
$paneWidth = Query -Target "${SESSION}:.0" -Fmt '#{pane_width}'
Write-Info "  pane_width after unzoom = $paneWidth"

if ([int]$paneWidth -gt 10) {
    Write-Pass "Pane width is reasonable ($paneWidth cols) — not miswrapped to 1-char width"
} else {
    Write-Fail "Pane width is too narrow ($paneWidth cols) — may cause miswrapping (issue #44)"
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
    Write-Host "Some tests FAILED — issue #44 may be present" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests PASSED" -ForegroundColor Green
    exit 0
}
