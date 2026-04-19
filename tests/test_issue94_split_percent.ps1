# psmux Issue #94 - split-window -p percent fix
# Tests that split-window -p <percent> allocates the correct proportion of space
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue94_split_percent.ps1

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

# Kill everything first
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "issue94test"

function New-TestSession {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
    & $PSMUX has-session -t $SESSION 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
}

# Helper: reset to a single-pane window by killing extra panes.
function Reset-ToSinglePane {
    # Kill panes until only one remains (kill-pane always kills the active pane)
    for ($i = 0; $i -lt 10; $i++) {
        $count = (& $PSMUX list-panes -t $SESSION -F "#{pane_index}" 2>&1 | Where-Object { $_.ToString().Trim() -match '^\d+$' }).Count
        if ($count -le 1) { break }
        & $PSMUX kill-pane -t $SESSION 2>$null
        Start-Sleep -Milliseconds 500
    }
    Start-Sleep -Seconds 1
}

# Helper: parse pane dimensions after a split.
# Returns an array of hashtables with Width and Height for each pane, ordered by pane index.
function Get-PaneDimensions {
    $raw = & $PSMUX list-panes -t $SESSION -F "#{pane_index} #{pane_width} #{pane_height}" 2>&1
    $panes = @()
    foreach ($line in $raw) {
        $parts = $line.ToString().Trim() -split '\s+'
        if ($parts.Count -ge 3 -and $parts[0] -match '^\d+$') {
            $panes += @{
                Index  = [int]$parts[0]
                Width  = [int]$parts[1]
                Height = [int]$parts[2]
            }
        }
    }
    return $panes | Sort-Object { $_.Index }
}

# Helper: check that a ratio is within tolerance of the expected value.
# $actual   - the measured percentage (0-100)
# $expected - the target percentage (0-100)
# $tolerance - allowed deviation in percentage points (default 5)
function Assert-Ratio {
    param(
        [double]$actual,
        [double]$expected,
        [double]$tolerance = 5,
        [string]$label
    )
    $diff = [Math]::Abs($actual - $expected)
    if ($diff -le $tolerance) {
        Write-Pass "$label - actual ${actual}% is within ${tolerance}pp of expected ${expected}%"
    } else {
        Write-Fail "$label - actual ${actual}% deviates ${diff}pp from expected ${expected}% (tolerance ${tolerance}pp)"
    }
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #94: split-window -p PERCENT FIX"
Write-Host ("=" * 70)

New-TestSession

# ============================================================
# TEST 1: Vertical split -p 30
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "TEST 1: split-window -v -p 30 (new pane gets ~30% height)"
Write-Host ("=" * 70)

& $PSMUX split-window -v -p 30 -t $SESSION
Start-Sleep -Seconds 1

$panes = Get-PaneDimensions
Write-Test "Vertical split -p 30: checking pane heights"

if ($panes.Count -eq 2) {
    $totalHeight = $panes[0].Height + $panes[1].Height
    # The new pane (pane 1) should be ~30% of total height
    $newPanePct = [Math]::Round(($panes[1].Height / $totalHeight) * 100, 1)
    Write-Info "Pane 0 height: $($panes[0].Height), Pane 1 height: $($panes[1].Height), total: $totalHeight"
    Write-Info "New pane (pane 1) is ${newPanePct}% of total height"
    Assert-Ratio -actual $newPanePct -expected 30 -label "Vertical -p 30: new pane height ratio"
} else {
    Write-Fail "Expected 2 panes after split, got $($panes.Count)"
}

Reset-ToSinglePane

# ============================================================
# TEST 2: Horizontal split -p 70
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "TEST 2: split-window -h -p 70 (new pane gets ~70% width)"
Write-Host ("=" * 70)

& $PSMUX split-window -h -p 70 -t $SESSION
Start-Sleep -Seconds 1

$panes = Get-PaneDimensions
Write-Test "Horizontal split -p 70: checking pane widths"

if ($panes.Count -eq 2) {
    $totalWidth = $panes[0].Width + $panes[1].Width
    # The new pane (pane 1) should be ~70% of total width
    $newPanePct = [Math]::Round(($panes[1].Width / $totalWidth) * 100, 1)
    Write-Info "Pane 0 width: $($panes[0].Width), Pane 1 width: $($panes[1].Width), total: $totalWidth"
    Write-Info "New pane (pane 1) is ${newPanePct}% of total width"
    Assert-Ratio -actual $newPanePct -expected 70 -label "Horizontal -p 70: new pane width ratio"
} else {
    Write-Fail "Expected 2 panes after split, got $($panes.Count)"
}

Reset-ToSinglePane

# ============================================================
# TEST 3: Edge case -p 10 (very small)
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "TEST 3: split-window -v -p 10 (new pane gets ~10% height)"
Write-Host ("=" * 70)

& $PSMUX split-window -v -p 10 -t $SESSION
Start-Sleep -Seconds 1

$panes = Get-PaneDimensions
Write-Test "Vertical split -p 10: checking pane heights"

if ($panes.Count -eq 2) {
    $totalHeight = $panes[0].Height + $panes[1].Height
    $newPanePct = [Math]::Round(($panes[1].Height / $totalHeight) * 100, 1)
    Write-Info "Pane 0 height: $($panes[0].Height), Pane 1 height: $($panes[1].Height), total: $totalHeight"
    Write-Info "New pane (pane 1) is ${newPanePct}% of total height"
    # Use wider tolerance for very small splits - rounding effects are proportionally larger
    Assert-Ratio -actual $newPanePct -expected 10 -tolerance 8 -label "Vertical -p 10: new pane height ratio"
} else {
    Write-Fail "Expected 2 panes after split, got $($panes.Count)"
}

Reset-ToSinglePane

# ============================================================
# TEST 4: Edge case -p 90 (very large)
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "TEST 4: split-window -v -p 90 (new pane gets ~90% height)"
Write-Host ("=" * 70)

& $PSMUX split-window -v -p 90 -t $SESSION
Start-Sleep -Seconds 1

$panes = Get-PaneDimensions
Write-Test "Vertical split -p 90: checking pane heights"

if ($panes.Count -eq 2) {
    $totalHeight = $panes[0].Height + $panes[1].Height
    $newPanePct = [Math]::Round(($panes[1].Height / $totalHeight) * 100, 1)
    Write-Info "Pane 0 height: $($panes[0].Height), Pane 1 height: $($panes[1].Height), total: $totalHeight"
    Write-Info "New pane (pane 1) is ${newPanePct}% of total height"
    # Use wider tolerance for very large splits - rounding effects are proportionally larger
    Assert-Ratio -actual $newPanePct -expected 90 -tolerance 8 -label "Vertical -p 90: new pane height ratio"
} else {
    Write-Fail "Expected 2 panes after split, got $($panes.Count)"
}

Reset-ToSinglePane

# ============================================================
# TEST 5: Default split (no -p) should be 50/50
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "TEST 5: split-window -v (no -p, default 50/50)"
Write-Host ("=" * 70)

& $PSMUX split-window -v -t $SESSION
Start-Sleep -Seconds 1

$panes = Get-PaneDimensions
Write-Test "Default vertical split: checking 50/50 height"

if ($panes.Count -eq 2) {
    $totalHeight = $panes[0].Height + $panes[1].Height
    $newPanePct = [Math]::Round(($panes[1].Height / $totalHeight) * 100, 1)
    Write-Info "Pane 0 height: $($panes[0].Height), Pane 1 height: $($panes[1].Height), total: $totalHeight"
    Write-Info "New pane (pane 1) is ${newPanePct}% of total height"
    Assert-Ratio -actual $newPanePct -expected 50 -label "Default split: new pane height ratio"
} else {
    Write-Fail "Expected 2 panes after split, got $($panes.Count)"
}

Reset-ToSinglePane

# ============================================================
# TEST 6: -l flag with percent (alias for -p)
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "TEST 6: split-window -v -l 25% (alias for -p 25)"
Write-Host ("=" * 70)

& $PSMUX split-window -v -l "25%" -t $SESSION
Start-Sleep -Seconds 1

$panes = Get-PaneDimensions
Write-Test "Vertical split -l 25%: checking pane heights"

if ($panes.Count -eq 2) {
    $totalHeight = $panes[0].Height + $panes[1].Height
    $newPanePct = [Math]::Round(($panes[1].Height / $totalHeight) * 100, 1)
    Write-Info "Pane 0 height: $($panes[0].Height), Pane 1 height: $($panes[1].Height), total: $totalHeight"
    Write-Info "New pane (pane 1) is ${newPanePct}% of total height"
    Assert-Ratio -actual $newPanePct -expected 25 -label "Vertical -l 25%: new pane height ratio"
} else {
    Write-Fail "Expected 2 panes after split, got $($panes.Count)"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Final cleanup..."
& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep -Seconds 1
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #94 SPLIT PERCENT TEST SUMMARY" -ForegroundColor White
Write-Host ("=" * 70)
Write-Host "Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "Total:  $($script:TestsPassed + $script:TestsFailed)"
Write-Host ""
Write-Host "Tests verify:" -ForegroundColor Yellow
Write-Host "  1. split-window -v -p 30  -> new pane gets ~30% height"
Write-Host "  2. split-window -h -p 70  -> new pane gets ~70% width"
Write-Host "  3. split-window -v -p 10  -> very small split works"
Write-Host "  4. split-window -v -p 90  -> very large split works"
Write-Host "  5. split-window -v (no -p) -> default 50/50 split"
Write-Host "  6. split-window -v -l 25% -> -l percent alias works"
Write-Host ("=" * 70)

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
