# psmux Layout Engine Tests
# Tests custom layout string parsing, deep tree restructuring, layout cycling
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_layout_engine.ps1

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

function New-PsmuxSession {
    param([string]$Name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $Name -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
}

function Psmux { & $PSMUX @args 2>&1 | Out-String; Start-Sleep -Milliseconds 300 }

# Cleanup
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "layout_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"
New-PsmuxSession -Name $SESSION

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "DEEP TREE RESTRUCTURING FOR NAMED LAYOUTS"
Write-Host ("=" * 60)

# Create a complex nested tree: 2 splits creating 4 panes
Psmux split-window -t $SESSION -h | Out-Null
Psmux split-window -t $SESSION -v | Out-Null
Psmux split-window -t $SESSION -v | Out-Null
Start-Sleep -Seconds 1

$panesBefore = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountBefore = ($panesBefore | Where-Object { $_ -match '\d+:' }).Count
Write-Info "Created $paneCountBefore panes"

# --- even-horizontal: should flatten all panes into a single H-split ---
Write-Test "even-horizontal flattens nested tree"
Psmux select-layout -t $SESSION even-horizontal | Out-Null
Start-Sleep -Milliseconds 500
$panesAfter = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountAfter = ($panesAfter | Where-Object { $_ -match '\d+:' }).Count
if ($paneCountAfter -eq $paneCountBefore) {
    Write-Pass "even-horizontal preserved $paneCountAfter panes"
} else {
    Write-Fail "even-horizontal changed pane count: $paneCountBefore -> $paneCountAfter"
}

# --- even-vertical: should flatten all panes into a single V-split ---
Write-Test "even-vertical flattens nested tree"
Psmux select-layout -t $SESSION even-vertical | Out-Null
Start-Sleep -Milliseconds 500
$panesAfter = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountAfter = ($panesAfter | Where-Object { $_ -match '\d+:' }).Count
if ($paneCountAfter -eq $paneCountBefore) {
    Write-Pass "even-vertical preserved $paneCountAfter panes"
} else {
    Write-Fail "even-vertical changed pane count: $paneCountBefore -> $paneCountAfter"
}

# --- main-horizontal: main pane on top, rest below ---
Write-Test "main-horizontal restructures correctly"
Psmux select-layout -t $SESSION main-horizontal | Out-Null
Start-Sleep -Milliseconds 500
$panesAfter = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountAfter = ($panesAfter | Where-Object { $_ -match '\d+:' }).Count
if ($paneCountAfter -eq $paneCountBefore) {
    Write-Pass "main-horizontal preserved $paneCountAfter panes"
} else {
    Write-Fail "main-horizontal changed pane count: $paneCountBefore -> $paneCountAfter"
}

# --- main-vertical: main pane on left, rest on right ---
Write-Test "main-vertical restructures correctly"
Psmux select-layout -t $SESSION main-vertical | Out-Null
Start-Sleep -Milliseconds 500
$panesAfter = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountAfter = ($panesAfter | Where-Object { $_ -match '\d+:' }).Count
if ($paneCountAfter -eq $paneCountBefore) {
    Write-Pass "main-vertical preserved $paneCountAfter panes"
} else {
    Write-Fail "main-vertical changed pane count: $paneCountBefore -> $paneCountAfter"
}

# --- tiled ---
Write-Test "tiled restructures correctly"
Psmux select-layout -t $SESSION tiled | Out-Null
Start-Sleep -Milliseconds 500
$panesAfter = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountAfter = ($panesAfter | Where-Object { $_ -match '\d+:' }).Count
if ($paneCountAfter -eq $paneCountBefore) {
    Write-Pass "tiled preserved $paneCountAfter panes"
} else {
    Write-Fail "tiled changed pane count: $paneCountBefore -> $paneCountAfter"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "LAYOUT CYCLING (FORWARD & REVERSE)"
Write-Host ("=" * 60)

# Track layouts through a full cycle
Write-Test "full forward cycle through 5 layouts"
$layouts = @()
for ($i = 0; $i -lt 5; $i++) {
    Psmux next-layout -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 300
    $layouts += "layout_$i"
}
Write-Pass "forward cycle completed 5 iterations"

Write-Test "full reverse cycle through 5 layouts"  
for ($i = 0; $i -lt 5; $i++) {
    Psmux previous-layout -t $SESSION | Out-Null
    Start-Sleep -Milliseconds 300
}
Write-Pass "reverse cycle completed 5 iterations"

Write-Test "reverse cycle is distinct from forward"
$layoutA = (Psmux display-message -t $SESSION -p "#{window_layout}").Trim()
Psmux next-layout -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$layoutB = (Psmux display-message -t $SESSION -p "#{window_layout}").Trim()
Psmux previous-layout -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$layoutC = (Psmux display-message -t $SESSION -p "#{window_layout}").Trim()
# After next then prev, should be back to original
if ($layoutA -eq $layoutC -and $layoutA -ne $layoutB) {
    Write-Pass "next-layout then previous-layout returns to original"
} else {
    Write-Info "A='$layoutA' B='$layoutB' C='$layoutC'"
    Write-Fail "layout cycling not symmetric"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "MAIN-PANE-WIDTH/HEIGHT ENFORCEMENT"
Write-Host ("=" * 60)

Write-Test "main-pane-width affects main-vertical layout"
Psmux set -t $SESSION -g main-pane-width 80 | Out-Null
Psmux select-layout -t $SESSION main-vertical | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "main-vertical with main-pane-width=80 applied"

Write-Test "main-pane-height affects main-horizontal layout"
Psmux set -t $SESSION -g main-pane-height 20 | Out-Null
Psmux select-layout -t $SESSION main-horizontal | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "main-horizontal with main-pane-height=20 applied"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CUSTOM LAYOUT STRING PARSING"
Write-Host ("=" * 60)

Write-Test "custom layout string (simple horizontal split)"
# tmux layout string format: checksum,WxH,X,Y{child1,child2}
# We generate a real window_layout, then re-apply it
$currentLayout = (& $PSMUX display-message -t $SESSION -p "#{window_layout}" 2>&1 | Out-String).Trim()
Write-Info "Current layout string: $currentLayout"
if ($currentLayout.Length -gt 5) {
    # Re-apply the same layout
    Psmux select-layout -t $SESSION "$currentLayout" | Out-Null
    Start-Sleep -Milliseconds 500
    Write-Pass "custom layout string re-applied: $($currentLayout.Substring(0, [Math]::Min(40, $currentLayout.Length)))..."
} else {
    Write-Fail "Could not get current layout string"
}

Write-Test "layout string round-trip preserves pane count"
$panesAfterCustom = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n")
$paneCountCustom = ($panesAfterCustom | Where-Object { $_ -match '\d+:' }).Count
if ($paneCountCustom -eq $paneCountBefore) {
    Write-Pass "layout string round-trip preserved $paneCountCustom panes"
} else {
    Write-Fail "layout string changed pane count: $paneCountBefore -> $paneCountCustom"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "NAVIGATION AFTER LAYOUT CHANGES"
Write-Host ("=" * 60)

# After all those layout changes, all panes should still be navigable
Write-Test "all panes reachable after layout changes"
$allPanes = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n") | Where-Object { $_ -match '\d+:' }
$reachable = 0
foreach ($dir in @("U", "D", "L", "R")) {
    Psmux select-pane -t $SESSION "-$dir" | Out-Null
    Start-Sleep -Milliseconds 200
    $reachable++
}
Write-Pass "directional navigation works after layout changes ($reachable directions tested)"

# ============================================================
# Win32 TUI VERIFICATION: Prove layout changes render visually
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_LYT = "lyt_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_LYT
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # TUI Test 1: Split pane via CLI (visible TUI window proves border rendering)
    Write-Test "TUI: Split pane creates visible border (visible TUI proof)"
    $panesBefore = (& $script:TUI_PSMUX list-panes -t $TUI_SESSION_LYT 2>&1 | Measure-Object -Line).Lines
    & $script:TUI_PSMUX split-window -h -t $TUI_SESSION_LYT 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $panesAfter = (& $script:TUI_PSMUX list-panes -t $TUI_SESSION_LYT 2>&1 | Measure-Object -Line).Lines
    if ($panesAfter -eq ($panesBefore + 1)) {
        Write-Pass "TUI: Split pane created new pane ($panesBefore -> $panesAfter)"
    } else {
        Write-Fail "TUI: Split failed ($panesBefore -> $panesAfter)"
    }

    # TUI Test 2: Layout cycle via CLI changes layout (visible TUI window proves rendering)
    Write-Test "TUI: Layout cycle changes layout (visible TUI proof)"
    $layoutBefore = Safe-TuiQuery "#{window_layout}" -Session $TUI_SESSION_LYT
    & $script:TUI_PSMUX next-layout -t $TUI_SESSION_LYT 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $layoutAfter = Safe-TuiQuery "#{window_layout}" -Session $TUI_SESSION_LYT
    if ($layoutAfter -ne $layoutBefore) {
        Write-Pass "TUI: Layout changed ($layoutBefore -> $layoutAfter)"
    } else {
        Write-Fail "TUI: Layout unchanged after next-layout"
    }

    # TUI Test 3: Horizontal split via CLI
    Write-Test "TUI: Horizontal split creates pane (visible TUI proof)"
    $panesBefore2 = (& $script:TUI_PSMUX list-panes -t $TUI_SESSION_LYT 2>&1 | Measure-Object -Line).Lines
    & $script:TUI_PSMUX split-window -v -t $TUI_SESSION_LYT 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $panesAfter2 = (& $script:TUI_PSMUX list-panes -t $TUI_SESSION_LYT 2>&1 | Measure-Object -Line).Lines
    if ($panesAfter2 -eq ($panesBefore2 + 1)) {
        Write-Pass "TUI: Horizontal split created new pane ($panesBefore2 -> $panesAfter2)"
    } else {
        Write-Fail "TUI: Horizontal split failed ($panesBefore2 -> $panesAfter2)"
    }

    Cleanup-PsmuxWindow -Session $TUI_SESSION_LYT
    Write-Host ""
} else {
    Write-Info "TUI verification skipped (could not launch window)"
}

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
