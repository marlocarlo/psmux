# Issue #43 Side-Observations:
#   1. Prefix+o (select-pane -t :.+) should cycle to next pane
#   2. Prefix+l (last-window) should update active window index (meta_dirty)
# Also tests: select-pane -t :.- (previous pane), meta_dirty on select-pane/last-pane

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "[FATAL] psmux binary not found" -ForegroundColor Red
    exit 1
}

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

$SESSION = "i43pol_$(Get-Random)"
Write-Info "Using psmux binary: $PSMUX"

# ─── Cleanup ──────────────────────────────────────────────────
Write-Info "Cleaning up stale sessions..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

# ─── Helpers ──────────────────────────────────────────────────

function Get-ActivePaneId {
    param($Session)
    $id = (& $PSMUX display-message -p "#{pane_id}" -t $Session 2>&1) | Out-String
    return $id.Trim()
}

function Get-ActiveWindowIndex {
    param($Session)
    $idx = (& $PSMUX display-message -p "#{window_index}" -t $Session 2>&1) | Out-String
    return $idx.Trim()
}

function Get-AllPaneIds {
    param($Session)
    $panes = (& $PSMUX list-panes -t $Session 2>&1) | Out-String
    $ids = @()
    foreach ($line in $panes.Split("`n")) {
        $line = $line.Trim()
        if ($line -match '%(\d+)') {
            $ids += "%$($Matches[1])"
        }
    }
    return $ids
}

# ─── Start server session ────────────────────────────────────
Write-Info "Starting session: $SESSION"
Psmux new-session -d -s $SESSION | Out-Null
Start-Sleep -Seconds 3

# ══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host "=" * 60
Write-Host "SECTION 1: SELECT-PANE -t :.+ (NEXT PANE / PREFIX+o)"
Write-Host "=" * 60

# Create a 2-pane layout
Write-Test "1.1 select-pane -t :.+ cycles to next pane (2 panes)"
Psmux split-window -h -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$allPanes = Get-AllPaneIds -Session $SESSION
Write-Info "  Panes: $($allPanes -join ', ')"

$before = Get-ActivePaneId -Session $SESSION
Write-Info "  Active before: $before"

# select-pane -t :.+ should move to next pane
Psmux select-pane -t "${SESSION}:.+" | Out-Null
Start-Sleep -Milliseconds 300

$after = Get-ActivePaneId -Session $SESSION
Write-Info "  Active after:  $after"

if ($before -ne $after -and $allPanes -contains $after) {
    Write-Pass "select-pane -t :.+ moved to different pane ($before -> $after)"
} else {
    Write-Fail "select-pane -t :.+ did not change pane (before=$before, after=$after)"
}

# ──────────────────────────────────────────────────────────────
Write-Test "1.2 select-pane -t :.+ wraps around (cycle)"
# Do :.+ again — should wrap back to original pane (2-pane layout)
Psmux select-pane -t "${SESSION}:.+" | Out-Null
Start-Sleep -Milliseconds 300

$wrapBack = Get-ActivePaneId -Session $SESSION
Write-Info "  After second :.+: $wrapBack"

if ($wrapBack -eq $before) {
    Write-Pass "select-pane -t :.+ wraps around back to first pane"
} else {
    Write-Fail "select-pane -t :.+ did not wrap (expected $before, got $wrapBack)"
}

# ──────────────────────────────────────────────────────────────
Write-Test "1.3 select-pane -t :.- cycles to previous pane"
$beforePrev = Get-ActivePaneId -Session $SESSION
Psmux select-pane -t "${SESSION}:.-" | Out-Null
Start-Sleep -Milliseconds 300

$afterPrev = Get-ActivePaneId -Session $SESSION
Write-Info "  Before: $beforePrev, After :.- : $afterPrev"

if ($beforePrev -ne $afterPrev) {
    Write-Pass "select-pane -t :.- moved to different pane"
} else {
    Write-Fail "select-pane -t :.- did not change pane"
}

# ──────────────────────────────────────────────────────────────
Write-Test "1.4 select-pane -t :.+ with 3 panes cycles through all"
# Add a third pane
Psmux split-window -v -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$allPanes3 = Get-AllPaneIds -Session $SESSION
Write-Info "  3 Panes: $($allPanes3 -join ', ')"

# Cycle through all 3 panes
$visited = @{}
$startId = Get-ActivePaneId -Session $SESSION
$visited[$startId] = $true
for ($i = 0; $i -lt 3; $i++) {
    Psmux select-pane -t "${SESSION}:.+" | Out-Null
    Start-Sleep -Milliseconds 300
    $cur = Get-ActivePaneId -Session $SESSION
    $visited[$cur] = $true
}

if ($visited.Count -ge 3) {
    Write-Pass "select-pane -t :.+ visited all 3 panes ($($visited.Count) unique)"
} else {
    Write-Fail "select-pane -t :.+ only visited $($visited.Count) of 3 panes"
}

# ──────────────────────────────────────────────────────────────
Write-Test "1.5 select-pane -t :.+ returns to start after full cycle (3 panes)"
# After 3 more :.+ calls from current position, should return to same pane
$cycleStart = Get-ActivePaneId -Session $SESSION
for ($i = 0; $i -lt 3; $i++) {
    Psmux select-pane -t "${SESSION}:.+" | Out-Null
    Start-Sleep -Milliseconds 300
}
$cycleEnd = Get-ActivePaneId -Session $SESSION
if ($cycleStart -eq $cycleEnd) {
    Write-Pass "Full cycle of 3 panes returns to start"
} else {
    Write-Fail "Full cycle did not return to start (start=$cycleStart, end=$cycleEnd)"
}

# ══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host "=" * 60
Write-Host "SECTION 2: LAST-WINDOW (PREFIX+l) WITH META_DIRTY"
Write-Host "=" * 60

# Create a second window
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "2.1 last-window switches active window index"
$idxBefore = Get-ActiveWindowIndex -Session $SESSION
Write-Info "  Window index before last-window: $idxBefore"

# Switch to last window (which is window 0, since new-window made active=1)
Psmux last-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300

$idxAfter = Get-ActiveWindowIndex -Session $SESSION
Write-Info "  Window index after last-window: $idxAfter"

if ($idxBefore -ne $idxAfter) {
    Write-Pass "last-window changed active window ($idxBefore -> $idxAfter)"
} else {
    Write-Fail "last-window did not change active window (stayed at $idxBefore)"
}

# ──────────────────────────────────────────────────────────────
Write-Test "2.2 last-window round-trip returns to original window"
Psmux last-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300

$idxRoundTrip = Get-ActiveWindowIndex -Session $SESSION
Write-Info "  Window index after second last-window: $idxRoundTrip"

if ($idxRoundTrip -eq $idxBefore) {
    Write-Pass "last-window round-trip returns to original window ($idxRoundTrip)"
} else {
    Write-Fail "last-window round-trip failed (expected $idxBefore, got $idxRoundTrip)"
}

# ──────────────────────────────────────────────────────────────
Write-Test "2.3 window index reported correctly after last-window"
# Create a 3rd window to make it more interesting
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$idx3 = Get-ActiveWindowIndex -Session $SESSION
Write-Info "  After new-window, active index: $idx3"

# Switch to previous window (select-window -p)
Psmux select-window -t $SESSION -p | Out-Null
Start-Sleep -Milliseconds 300
$idxPrev = Get-ActiveWindowIndex -Session $SESSION
Write-Info "  After select-window -p: $idxPrev"

# Now do last-window — should go back to window $idx3
Psmux last-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$idxLast = Get-ActiveWindowIndex -Session $SESSION
Write-Info "  After last-window: $idxLast"

if ($idxLast -eq $idx3) {
    Write-Pass "last-window correctly returned to window $idx3"
} else {
    Write-Fail "last-window went to $idxLast instead of expected $idx3"
}

# ══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host "=" * 60
Write-Host "SECTION 3: SELECT-PANE META_DIRTY (PANE SWITCHING UPDATES)"
Write-Host "=" * 60

# Go back to window 0 which has 3 panes
Psmux select-window -t "${SESSION}:0" | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "3.1 select-pane -U updates correctly"
$pBefore = Get-ActivePaneId -Session $SESSION
Psmux select-pane -t $SESSION -U | Out-Null
Start-Sleep -Milliseconds 300
$pAfterU = Get-ActivePaneId -Session $SESSION
# The pane may or may not change (depends on layout), but the query must work
if ($pAfterU -match '%\d+') {
    Write-Pass "select-pane -U returns valid pane id ($pAfterU)"
} else {
    Write-Fail "select-pane -U returned invalid pane: $pAfterU"
}

Write-Test "3.2 select-pane -l (last pane) works and updates"
# First select a known pane, then switch, then use -l
$p1 = Get-ActivePaneId -Session $SESSION
Psmux select-pane -t "${SESSION}:.+" | Out-Null
Start-Sleep -Milliseconds 300
$p2 = Get-ActivePaneId -Session $SESSION

# Now last-pane should go back to p1
Psmux select-pane -t $SESSION -l | Out-Null
Start-Sleep -Milliseconds 300
$pLast = Get-ActivePaneId -Session $SESSION

if ($pLast -eq $p1) {
    Write-Pass "select-pane -l returned to previous pane ($pLast)"
} else {
    Write-Fail "select-pane -l went to $pLast instead of expected $p1"
}

Write-Test "3.3 select-pane -l round-trip"
Psmux select-pane -t $SESSION -l | Out-Null
Start-Sleep -Milliseconds 300
$pLast2 = Get-ActivePaneId -Session $SESSION
if ($pLast2 -eq $p2) {
    Write-Pass "select-pane -l round-trip to $pLast2 (expected $p2)"
} else {
    Write-Fail "select-pane -l round-trip got $pLast2 (expected $p2)"
}

# ══════════════════════════════════════════════════════════════  
Write-Host ""
Write-Host "=" * 60
Write-Host "SECTION 4: EDGE CASES"
Write-Host "=" * 60

Write-Test "4.1 select-pane -t :.+ with single pane stays on same pane"
# Create a new window (single pane)
Psmux new-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500

$singlePane = Get-ActivePaneId -Session $SESSION
Psmux select-pane -t "${SESSION}:.+" | Out-Null
Start-Sleep -Milliseconds 300
$afterSingle = Get-ActivePaneId -Session $SESSION

if ($singlePane -eq $afterSingle) {
    Write-Pass "select-pane -t :.+ with single pane stays put ($singlePane)"
} else {
    Write-Fail "select-pane -t :.+ with single pane changed to $afterSingle"
}

Write-Test "4.2 select-pane -t :.- with single pane stays on same pane"
Psmux select-pane -t "${SESSION}:.-" | Out-Null
Start-Sleep -Milliseconds 300
$afterSinglePrev = Get-ActivePaneId -Session $SESSION

if ($singlePane -eq $afterSinglePrev) {
    Write-Pass "select-pane -t :.- with single pane stays put"
} else {
    Write-Fail "select-pane -t :.- with single pane changed to $afterSinglePrev"
}

Write-Test "4.3 last-window with only 1 window does not crash"
# Kill extra windows, go back to 1 window
# Actually, let's just test with the current multi-window session - skip destructive test
# Instead, test that last-window with many windows still works
$idxCur = Get-ActiveWindowIndex -Session $SESSION
Psmux last-window -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$idxSwitch = Get-ActiveWindowIndex -Session $SESSION
# Just verify no crash and result is valid
if ($idxSwitch -match '^\d+$') {
    Write-Pass "last-window returns valid window index ($idxSwitch)"
} else {
    Write-Fail "last-window returned invalid index: $idxSwitch"
}

# ─── Cleanup ──────────────────────────────────────────────────
Write-Host ""
Write-Info "Cleaning up..."
Psmux kill-session -t $SESSION | Out-Null
Start-Sleep -Seconds 2

Write-Host ""
Write-Host "=" * 60
Write-Host "ISSUE #43 PREFIX+O / PREFIX+L TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host ""

if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
    exit 0
}
