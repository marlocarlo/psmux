# psmux Zoom Pane Resize Test (GitHub Issue #35)
# Verifies that zooming a pane triggers a PTY resize so child apps
# (neovim, bottom, etc.) re-render at the full terminal size.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_zoom_resize.ps1

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

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

# Cleanup any previous test session
Write-Info "Cleaning up..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t zoom_test" -WindowStyle Hidden -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# Create a detached session
Write-Info "Creating detached session 'zoom_test'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s zoom_test -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$hasSession = & $PSMUX has-session -t zoom_test 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "FATAL: Cannot create test session" -ForegroundColor Red
    exit 1
}
Write-Info "Session 'zoom_test' created"

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  ZOOM PANE RESIZE TEST (Issue #35)"
Write-Host ("=" * 60)
Write-Host ""

# -----------------------------------------------------------------
# Test 1: Vertical split – zoom should expand pane height
# -----------------------------------------------------------------
Write-Test "Vertical split: pane height increases after zoom"

# Create a vertical split (top/bottom)
Psmux split-window -v -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

# Get pre-zoom pane height (active pane = bottom pane after split)
$preH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Pre-zoom pane_height = $preH"

# Zoom the active pane
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

# Get post-zoom pane height
$postH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Post-zoom pane_height = $postH"

if ([int]$postH -gt [int]$preH) {
    Write-Pass "Pane height increased after zoom: $preH -> $postH"
} else {
    Write-Fail "Pane height did NOT increase after zoom: $preH -> $postH (BUG: issue #35)"
}

# Unzoom
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

$restoredH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Restored pane_height = $restoredH"

if ([int]$restoredH -eq [int]$preH) {
    Write-Pass "Pane height restored after unzoom: $restoredH == $preH"
} else {
    Write-Fail "Pane height not restored: expected $preH, got $restoredH"
}

# -----------------------------------------------------------------
# Test 2: Horizontal split – zoom should expand pane width
# -----------------------------------------------------------------
Write-Test "Horizontal split: pane width increases after zoom"

# Start fresh window for this test
Psmux new-window -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

# Create a horizontal split (left/right)
Psmux split-window -h -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

# Get pre-zoom pane width
$preW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
Write-Info "  Pre-zoom pane_width = $preW"

# Zoom
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

$postW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
Write-Info "  Post-zoom pane_width = $postW"

if ([int]$postW -gt [int]$preW) {
    Write-Pass "Pane width increased after zoom: $preW -> $postW"
} else {
    Write-Fail "Pane width did NOT increase after zoom: $preW -> $postW (BUG: issue #35)"
}

# Unzoom
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

$restoredW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
Write-Info "  Restored pane_width = $restoredW"

if ([int]$restoredW -eq [int]$preW) {
    Write-Pass "Pane width restored after unzoom: $restoredW == $preW"
} else {
    Write-Fail "Pane width not restored: expected $preW, got $restoredW"
}

# -----------------------------------------------------------------
# Test 3: Both dimensions in a 4-pane grid layout
# -----------------------------------------------------------------
Write-Test "4-pane grid: zoomed pane gets full window dimensions"

Psmux new-window -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

# Get full-window dimensions before splitting
$fullW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
$fullH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Full-window size: ${fullW}x${fullH}"

# Create a 2x2 grid
Psmux split-window -v -t zoom_test | Out-Null
Start-Sleep -Milliseconds 300
Psmux split-window -h -t zoom_test | Out-Null
Start-Sleep -Milliseconds 300
Psmux select-pane -t zoom_test -U | Out-Null
Start-Sleep -Milliseconds 300
Psmux split-window -h -t zoom_test | Out-Null
Start-Sleep -Milliseconds 300

# Now we're in one of the 4 panes — get its small size
$smallW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
$smallH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Quarter-pane size: ${smallW}x${smallH}"

# Zoom to fill the whole window
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

$zW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
$zH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Zoomed-pane size: ${zW}x${zH}"

if ([int]$zW -ge [int]$fullW -and [int]$zH -ge [int]$fullH) {
    Write-Pass "Zoomed pane fills full window: ${zW}x${zH} >= ${fullW}x${fullH}"
} elseif ([int]$zW -gt [int]$smallW -and [int]$zH -gt [int]$smallH) {
    Write-Pass "Zoomed pane expanded (approximately full): ${zW}x${zH} (full: ${fullW}x${fullH})"
} else {
    Write-Fail "Zoomed pane NOT expanded: ${zW}x${zH}, expected ~${fullW}x${fullH} (BUG: issue #35)"
}

# Unzoom — should return to quarter size
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

$restW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
$restH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Unzoomed size: ${restW}x${restH}"

if ([int]$restW -eq [int]$smallW -and [int]$restH -eq [int]$smallH) {
    Write-Pass "Unzoom restored quarter size: ${restW}x${restH}"
} else {
    Write-Fail "Unzoom size mismatch: got ${restW}x${restH}, expected ${smallW}x${smallH}"
}

# -----------------------------------------------------------------
# Test 4: Double zoom toggle is idempotent
# -----------------------------------------------------------------
Write-Test "Double zoom toggle returns to original size"

Psmux new-window -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500
Psmux split-window -h -t zoom_test | Out-Null
Start-Sleep -Milliseconds 500

$origW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
$origH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  Original: ${origW}x${origH}"

# Zoom + unzoom
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 300
Psmux resize-pane -Z -t zoom_test | Out-Null
Start-Sleep -Milliseconds 300

$afterW = (Psmux display-message -t zoom_test -p '#{pane_width}').Trim()
$afterH = (Psmux display-message -t zoom_test -p '#{pane_height}').Trim()
Write-Info "  After toggle x2: ${afterW}x${afterH}"

if ([int]$afterW -eq [int]$origW -and [int]$afterH -eq [int]$origH) {
    Write-Pass "Double toggle restored size: ${afterW}x${afterH}"
} else {
    Write-Fail "Double toggle size mismatch: got ${afterW}x${afterH}, expected ${origW}x${origH}"
}

# -----------------------------------------------------------------
# Cleanup
# -----------------------------------------------------------------
Write-Host ""
Write-Info "Cleaning up session 'zoom_test'..."
Psmux kill-session -t zoom_test | Out-Null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
    exit 0
}
