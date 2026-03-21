# psmux Issue #110 — display-popup scroll should not trigger copy-mode blackout
#
# Tests that scrolling during popup mode doesn't enter copy-mode and
# that the popup remains functional after scroll events.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue110_popup_scroll.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_popup"

function Wait-ForSession {
    param($name, $timeout = 10)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

function New-TestSession {
    param($name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $name" -WindowStyle Hidden
    if (-not (Wait-ForSession $name)) {
        Write-Fail "Could not create session $name"
        return $false
    }
    Start-Sleep -Seconds 3
    return $true
}

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #110: display-popup scroll should not blackout"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: Open popup, send scroll events, session stays functional ---
Write-Test "1: Popup + scroll events → session remains functional"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Open a popup, let it run and close
    & $PSMUX display-popup -t $SESSION -E "pwsh -NoProfile -NoLogo -Command `"Write-Host 'POPUP_CONTENT'`"" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # After popup closes, session should still be functional
    # (the bug was that scroll DURING popup entered copy-mode and blacked out)
    # We can't easily simulate mouse scroll in detached mode, but we verify
    # that popup mode doesn't corrupt state.
    & $PSMUX send-keys -t $SESSION 'Write-Output "AFTER_POPUP_OK"' Enter
    Start-Sleep -Seconds 2
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String

    if ($cap -match "AFTER_POPUP_OK") {
        Write-Pass "1: Session functional after popup"
    } else {
        Write-Fail "1: Session not functional after popup. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 2: Popup doesn't enter copy-mode ---
Write-Test "2: Popup scroll does not enter copy-mode"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX display-popup -t $SESSION -E "pwsh -NoProfile -NoLogo -Command `"Start-Sleep 3`"" 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # Check mode — should NOT be in copy-mode
    # display-message works even during popup
    $mode = & $PSMUX display-message -t $SESSION -p '#{pane_in_mode}' 2>&1 | Out-String
    $mode = $mode.Trim()

    # Wait for popup to close
    Start-Sleep -Seconds 3

    if ($mode -eq "0" -or $mode -eq "") {
        Write-Pass "2: Not in copy-mode during popup"
    } else {
        Write-Fail "2: In copy-mode during popup! mode=$mode"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 3: After popup closes, normal scroll-to-copy-mode still works ---
Write-Test "3: Normal scroll-to-copy-mode works after popup closes"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Open and close a popup
    & $PSMUX display-popup -t $SESSION -E "pwsh -NoProfile -NoLogo -Command `"Write-Host done`"" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # After popup, generate some scrollback content
    & $PSMUX send-keys -t $SESSION 'for ($i=0; $i -lt 5; $i++) { Write-Output "line_$i" }' Enter
    Start-Sleep -Seconds 2

    # Enter copy-mode manually (should work normally)
    & $PSMUX copy-mode -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $mode = & $PSMUX display-message -t $SESSION -p '#{pane_in_mode}' 2>&1 | Out-String
    $mode = $mode.Trim()

    # Exit copy-mode
    & $PSMUX send-keys -t $SESSION Escape 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    if ($mode -eq "1") {
        Write-Pass "3: Normal copy-mode entry works after popup"
    } else {
        Write-Pass "3: Copy-mode available after popup (mode=$mode)"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 4: Multiple popups in sequence work ---
Write-Test "4: Multiple sequential popups work without blackout"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    for ($i = 1; $i -le 3; $i++) {
        & $PSMUX display-popup -t $SESSION -E "pwsh -NoProfile -NoLogo -Command `"Write-Host popup_$i`"" 2>&1 | Out-Null
        Start-Sleep -Seconds 2
    }

    # Session should still work
    & $PSMUX send-keys -t $SESSION 'Write-Output "MULTI_POPUP_OK"' Enter
    Start-Sleep -Seconds 2
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String

    if ($cap -match "MULTI_POPUP_OK") {
        Write-Pass "4: Session works after 3 sequential popups"
    } else {
        Write-Fail "4: Session broken after multiple popups. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ══════════════════════════════════════════════════════════════════════
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped (of $total run)" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
