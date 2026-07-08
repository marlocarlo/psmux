# Regression test: swap-window / swapw must actually reorder windows.
#
# The bug: swap-window's -t was hijacked into a transient FocusWindowTemp,
# the dispatch then ignored the resolved -t target, and (once dispatch was
# fixed) the handler used the raw display index without base-index
# conversion. Net effect: swap-window never swapped anything -- it briefly
# focused the target window and then snapped back, and meta/state were never
# marked dirty so nothing persisted.
#
# The unit test suite missed this entirely because every server_forwarded_*
# test drives a mock App with control_port = None (the local/offline path).
# This bug lived in the control_port = Some(..) path (src/server/connection.rs
# dispatch -> CtrlReq::SwapWindow -> src/server/mod.rs handler), which only a
# real TCP-backed server exercises. This script drives a real server the same
# way tests/test_issue400_swap_pane_index_order.ps1 does for swap-pane.
#
# Assertions:
#   R1 - swap-window -t :N reorders to the correct window (the core bug).
#   R2 - the new order persists after select-window / next-window (no revert
#        on window switch -- proves meta_dirty/state_dirty are set).
#   R3 - swapping with the current window, or an out-of-range target, is a
#        silent no-op: no crash, no reorder.

$ErrorActionPreference = "Continue"

$PSMUX = $env:PSMUX_EXE
if (-not $PSMUX) {
    $cmd = Get-Command psmux -EA Stop
    $PSMUX = if ($cmd.Path) { $cmd.Path } elseif ($cmd.Source) { $cmd.Source } else { $cmd.Definition }
}
if (-not $PSMUX) {
    Write-Host "FATAL: could not resolve psmux executable path" -ForegroundColor Red
    exit 1
}

# A stale/pre-fix warm-server can otherwise serve this test a cached binary.
$env:PSMUX_NO_WARM = "1"

$psmuxDir = "$env:USERPROFILE\.psmux"
# Unique per-PID session name -- never collide with (or clobber) a real session.
$SESSION = "swapwtest_$PID"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    # Kill ONLY this test's session -- never a global Stop-Process/taskkill,
    # which would nuke the user's real live sessions.
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 400
    Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
}

function Wait-ServerReady {
    param([int]$MaxAttempts = 20, [int]$DelayMs = 400)
    # Server-startup race: require list-windows to succeed twice in a row
    # before trusting the session is actually ready.
    $consecutiveOk = 0
    for ($i = 0; $i -lt $MaxAttempts; $i++) {
        & $PSMUX list-windows -t $SESSION 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $consecutiveOk++
            if ($consecutiveOk -ge 2) { return $true }
        } else {
            $consecutiveOk = 0
        }
        Start-Sleep -Milliseconds $DelayMs
    }
    return $false
}

function Get-WindowLines {
    $out = & $PSMUX list-windows -t $SESSION -F '#{window_index}:#{window_name}:#{window_active}' 2>&1
    return @($out | Where-Object { $_ -match '^\d+:[^:]*:\d$' })
}

function Get-OrderString {
    $sorted = Get-WindowLines | Sort-Object { [int]($_ -split ':')[0] }
    return ($sorted | ForEach-Object { $p = $_ -split ':'; "$($p[0]):$($p[1])" }) -join ' '
}

function Get-ActiveIndex {
    foreach ($l in (Get-WindowLines)) {
        $p = $l -split ':'
        if ($p[-1] -eq '1') { return [int]$p[0] }
    }
    return -1
}

function Add-WindowRetry {
    param([string]$Name, [int]$MaxCreateAttempts = 3, [int]$PollAttempts = 15, [int]$DelayMs = 300)
    for ($c = 0; $c -lt $MaxCreateAttempts; $c++) {
        if ((Get-WindowLines) -match ":${Name}:") { return $true }
        & $PSMUX neww -n $Name -d -t $SESSION 2>&1 | Out-Null
        for ($p = 0; $p -lt $PollAttempts; $p++) {
            Start-Sleep -Milliseconds $DelayMs
            if ((Get-WindowLines) -match ":${Name}:") { return $true }
        }
    }
    return $false
}

Write-Host "`n=== swap-window revert regression test (session: $SESSION) ===" -ForegroundColor Cyan

# === SETUP ===
Cleanup
& $PSMUX new-session -d -s $SESSION -n alpha 2>&1 | Out-Null
if (-not (Wait-ServerReady)) {
    Write-Host "FATAL: server for $SESSION did not become ready" -ForegroundColor Red
    Cleanup
    exit 1
}

$baseline = Get-WindowLines
if ($baseline.Count -ne 1 -or -not ($baseline -match ':alpha:')) {
    Write-Host "FATAL: expected single window 'alpha' after new-session, got: $($baseline -join ' | ')" -ForegroundColor Red
    Cleanup
    exit 1
}
# Discover the real base-index dynamically instead of assuming a fixed value.
# base-index is a configurable session option (src/server/option_catalog.rs)
# and this machine's effective default may differ from a fresh checkout's --
# hardcoding a number here would make the test fragile to config drift.
$base = [int](($baseline[0] -split ':')[0])
Write-Host "  Discovered base-index: $base" -ForegroundColor DarkGray

if (-not (Add-WindowRetry -Name "beta")) {
    Write-Host "FATAL: window 'beta' never appeared" -ForegroundColor Red
    Cleanup
    exit 1
}
if (-not (Add-WindowRetry -Name "gamma")) {
    Write-Host "FATAL: window 'gamma' never appeared" -ForegroundColor Red
    Cleanup
    exit 1
}

$idxAlpha = $base
$idxBeta  = $base + 1
$idxGamma = $base + 2

$expectedBaseline = "${idxAlpha}:alpha ${idxBeta}:beta ${idxGamma}:gamma"
$actualBaseline = Get-OrderString
if ($actualBaseline -eq $expectedBaseline) { Write-Pass "baseline order is $actualBaseline" }
else { Write-Fail "baseline order expected '$expectedBaseline', got '$actualBaseline'" }

# === R1: swap-window -t reorders to the correct window (the core bug) ===
Write-Host "`n[R1] swap-window -t reorders to the correct window" -ForegroundColor Yellow
& $PSMUX select-window -t "${SESSION}:${idxBeta}" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
if ((Get-ActiveIndex) -ne $idxBeta) { Write-Fail "setup: expected beta ($idxBeta) active before swap, got $(Get-ActiveIndex)" }

& $PSMUX swap-window -t "${SESSION}:${idxGamma}" 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
# swap-window exchanges the window OBJECTS at the active slot and the target
# slot; the slot numbers (positions) stay put, but their contents (names)
# trade places. Active was at idxBeta (beta), target was idxGamma (gamma), so
# after the swap: idxBeta's slot now shows gamma, idxGamma's slot shows beta.
$expectedSwapped = "${idxAlpha}:alpha ${idxBeta}:gamma ${idxGamma}:beta"
$afterSwap = Get-OrderString
if ($afterSwap -eq $expectedSwapped) { Write-Pass "R1: swap-window -t :${idxGamma} swapped beta<->gamma -> $afterSwap" }
else { Write-Fail "R1: expected '$expectedSwapped', got '$afterSwap'" }

# === R2: order persists after select-window / next-window ===
Write-Host "`n[R2] swap persists across window switches" -ForegroundColor Yellow
& $PSMUX select-window -t "${SESSION}:${idxAlpha}" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
$afterSelect = Get-OrderString
if ($afterSelect -eq $expectedSwapped) { Write-Pass "R2: order persists after select-window -> $afterSelect" }
else { Write-Fail "R2: order reverted after select-window: expected '$expectedSwapped', got '$afterSelect'" }

& $PSMUX next-window -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
$afterNext = Get-OrderString
if ($afterNext -eq $expectedSwapped) { Write-Pass "R2: order persists after next-window -> $afterNext" }
else { Write-Fail "R2: order reverted after next-window: expected '$expectedSwapped', got '$afterNext'" }

# === R3: edge cases are silent no-ops, no crash ===
Write-Host "`n[R3] current/out-of-range swap targets no-op without crashing" -ForegroundColor Yellow
$currentActive = Get-ActiveIndex
& $PSMUX swap-window -t "${SESSION}:${currentActive}" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
$afterSelfSwap = Get-OrderString
if ($afterSelfSwap -eq $expectedSwapped) { Write-Pass "R3: swap-window -t :<current> is a no-op ($afterSelfSwap)" }
else { Write-Fail "R3: current-target swap should be a no-op: expected '$expectedSwapped', got '$afterSelfSwap'" }

& $PSMUX swap-window -t "${SESSION}:99" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
if (-not (Wait-ServerReady -MaxAttempts 5 -DelayMs 200)) {
    Write-Fail "R3: server did not survive an out-of-range swap-window target"
} else {
    $afterOob = Get-OrderString
    if ($afterOob -eq $expectedSwapped) { Write-Pass "R3: swap-window -t :99 (out-of-range) is a no-op, server alive ($afterOob)" }
    else { Write-Fail "R3: out-of-range target should be a no-op: expected '$expectedSwapped', got '$afterOob'" }
}

# === CLEANUP ===
Cleanup

Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
