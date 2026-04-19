# Issue #231: OSC 0/2 escape sequences do not update pane_title
# VERIFICATION TEST - proves the fix works by sending OSC titles and querying them
#
# Root cause: propagate_osc_titles was nested inside auto_rename guard and only
# ran during DumpState (TUI attached). Now it runs independently and before
# all state-query commands (display-message, list-panes, list-windows).
#
# Strategy: We send a command that emits OSC 2 then blocks with Start-Sleep,
# so the pwsh prompt does NOT come back and overwrite our title. This gives us
# a stable window to query pane_title reliably.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue231_osc_title_propagation.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "  [INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "  [TEST] $msg" -ForegroundColor White }

$PSMUX = (Get-Command psmux -EA SilentlyContinue).Source
if (-not $PSMUX) {
    $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -EA SilentlyContinue).Path
}
if (-not $PSMUX) {
    $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -EA SilentlyContinue).Path
}
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_osc231"
$hostname = [System.Net.Dns]::GetHostName()

function Wait-ForSession {
    param($name, $timeout = 15)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>$null
    Start-Sleep -Milliseconds 500
}

function New-Session {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) {
        Write-Fail "Could not create session $SESSION"
        return $false
    }
    Start-Sleep -Seconds 3
    return $true
}

Write-Host ""
Write-Host ("=" * 70)
Write-Host "Issue #231: OSC 0/2 Title Propagation Verification"
Write-Host ("=" * 70)

# -------------------------------------------------------------------------
# TEST 1: OSC 2 updates pane_title via display-message
# -------------------------------------------------------------------------
Write-Test "1: OSC 2 title propagates to pane_title (display-message)"
Cleanup
if (-not (New-Session)) { exit 1 }

$marker1 = "OSC2_MARKER_231_A"
# Send OSC 2 + block so pwsh prompt does not overwrite
& $PSMUX send-keys -t $SESSION "Write-Host -NoNewline ([char]27 + `"]2;$marker1`" + [char]7); Start-Sleep 30" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$title1 = (& $PSMUX display-message -t $SESSION -p '#{pane_title}' 2>&1 | Out-String).Trim()
Write-Info "pane_title after OSC 2: '$title1' (expected: '$marker1')"

if ($title1 -eq $marker1) {
    Write-Pass "OSC 2 correctly propagated to pane_title"
} else {
    Write-Fail "pane_title is '$title1', expected '$marker1'"
}

# Cancel the sleep
& $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# -------------------------------------------------------------------------
# TEST 2: OSC 0 updates pane_title via display-message
# -------------------------------------------------------------------------
Write-Test "2: OSC 0 title propagates to pane_title (display-message)"

$marker2 = "OSC0_MARKER_231_B"
& $PSMUX send-keys -t $SESSION "Write-Host -NoNewline ([char]27 + `"]0;$marker2`" + [char]7); Start-Sleep 30" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$title2 = (& $PSMUX display-message -t $SESSION -p '#{pane_title}' 2>&1 | Out-String).Trim()
Write-Info "pane_title after OSC 0: '$title2' (expected: '$marker2')"

if ($title2 -eq $marker2) {
    Write-Pass "OSC 0 correctly propagated to pane_title"
} else {
    Write-Fail "pane_title is '$title2', expected '$marker2'"
}

& $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# -------------------------------------------------------------------------
# TEST 3: pane_title visible in list-panes output
# -------------------------------------------------------------------------
Write-Test "3: OSC title visible in list-panes -F '#{pane_title}'"

$marker3 = "LISTPANES_231_C"
& $PSMUX send-keys -t $SESSION "Write-Host -NoNewline ([char]27 + `"]2;$marker3`" + [char]7); Start-Sleep 30" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$lpTitle = (& $PSMUX list-panes -t $SESSION -F '#{pane_title}' 2>&1 | Out-String).Trim()
Write-Info "list-panes pane_title: '$lpTitle' (expected: '$marker3')"

if ($lpTitle -eq $marker3) {
    Write-Pass "list-panes shows propagated OSC title"
} else {
    Write-Fail "list-panes pane_title is '$lpTitle', expected '$marker3'"
}

& $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# -------------------------------------------------------------------------
# TEST 4: pane_title visible in list-windows output
# -------------------------------------------------------------------------
Write-Test "4: OSC title visible in list-windows -F '#{pane_title}'"

$marker4 = "LISTWIN_231_D"
& $PSMUX send-keys -t $SESSION "Write-Host -NoNewline ([char]27 + `"]2;$marker4`" + [char]7); Start-Sleep 30" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$lwTitle = (& $PSMUX list-windows -t $SESSION -F '#{pane_title}' 2>&1 | Out-String).Trim()
Write-Info "list-windows pane_title: '$lwTitle' (expected: '$marker4')"

if ($lwTitle -eq $marker4) {
    Write-Pass "list-windows shows propagated OSC title"
} else {
    Write-Fail "list-windows pane_title is '$lwTitle', expected '$marker4'"
}

& $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# -------------------------------------------------------------------------
# TEST 5: #T alias resolves to the OSC title
# -------------------------------------------------------------------------
Write-Test "5: #T alias resolves to OSC title"

$marker5 = "HASH_T_231_E"
& $PSMUX send-keys -t $SESSION "Write-Host -NoNewline ([char]27 + `"]2;$marker5`" + [char]7); Start-Sleep 30" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$hashT = (& $PSMUX display-message -t $SESSION -p '#T' 2>&1 | Out-String).Trim()
Write-Info "#T after OSC 2: '$hashT' (expected: '$marker5')"

if ($hashT -eq $marker5) {
    Write-Pass "#T alias correctly shows OSC title"
} else {
    Write-Fail "#T is '$hashT', expected '$marker5'"
}

& $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# -------------------------------------------------------------------------
# TEST 6: select-pane -T locks title (OSC 2 does not overwrite)
# -------------------------------------------------------------------------
Write-Test "6: select-pane -T locks title against OSC overwrite"

$lockedTitle = "LOCKED_TITLE_231"
& $PSMUX select-pane -t $SESSION -T $lockedTitle 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# Verify lock was set
$titleLocked = (& $PSMUX display-message -t $SESSION -p '#{pane_title}' 2>&1 | Out-String).Trim()
if ($titleLocked -ne $lockedTitle) {
    Write-Fail "select-pane -T did not set title. Got '$titleLocked'"
} else {
    # Now send OSC 2 which should NOT overwrite
    & $PSMUX send-keys -t $SESSION "Write-Host -NoNewline ([char]27 + `"]2;SHOULD_NOT_APPEAR`" + [char]7); Start-Sleep 30" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    $titleAfter = (& $PSMUX display-message -t $SESSION -p '#{pane_title}' 2>&1 | Out-String).Trim()
    Write-Info "pane_title after OSC with lock: '$titleAfter' (expected: '$lockedTitle')"

    if ($titleAfter -eq $lockedTitle) {
        Write-Pass "title_locked prevents OSC from overwriting"
    } else {
        Write-Fail "OSC overwrote locked title. Got '$titleAfter', expected '$lockedTitle'"
    }

    & $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
}

# -------------------------------------------------------------------------
# TEST 7: Rapid successive OSC updates (last one wins)
# -------------------------------------------------------------------------
Write-Test "7: Rapid successive OSC 2 sequences (last one wins)"

# Unlock title first by removing the session and recreating
Cleanup
if (-not (New-Session)) { exit 1 }

$finalMarker = "FINAL_231_G"
# Send multiple OSC titles rapidly, the last should win
$cmd = "Write-Host -NoNewline ([char]27 + `"]2;FIRST`" + [char]7); " +
       "Write-Host -NoNewline ([char]27 + `"]2;SECOND`" + [char]7); " +
       "Write-Host -NoNewline ([char]27 + `"]2;$finalMarker`" + [char]7); " +
       "Start-Sleep 30"
& $PSMUX send-keys -t $SESSION "$cmd" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$rapidTitle = (& $PSMUX display-message -t $SESSION -p '#{pane_title}' 2>&1 | Out-String).Trim()
Write-Info "pane_title after rapid OSC: '$rapidTitle' (expected: '$finalMarker')"

if ($rapidTitle -eq $finalMarker) {
    Write-Pass "Last OSC title wins in rapid succession"
} else {
    Write-Fail "pane_title is '$rapidTitle', expected '$finalMarker'"
}

& $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# -------------------------------------------------------------------------
# CLEANUP AND SUMMARY
# -------------------------------------------------------------------------
Cleanup
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 70)
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 70)

if ($script:TestsFailed -eq 0) {
    Write-Host "  VERDICT: Issue #231 fix VERIFIED. OSC title propagation works." -ForegroundColor Green
} else {
    Write-Host "  VERDICT: $($script:TestsFailed) test(s) failed. Fix may be incomplete." -ForegroundColor Red
}

exit $script:TestsFailed
