# psmux Issue #165: Set-PSReadLineOption -PredictionViewStyle ListView Not Working
#
# Root cause: the early warm pane was spawned BEFORE load_config, so
# allow-predictions on from the config was never applied to the initial pane.
# The fix ensures the warm pane is respawned with the correct PSReadLine
# init string when allow-predictions is enabled by config.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue165_prediction_view_style.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass   { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail   { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red;   $script:TestsFailed++ }
function Write-Skip   { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow;$script:TestsSkipped++ }
function Write-Info   { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test   { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using binary: $PSMUX"

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Get-Process psmux,tmux,pmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

# Save original config
$confPath = "$env:USERPROFILE\.psmux.conf"
$origConf = if (Test-Path $confPath) { Get-Content $confPath -Raw } else { $null }

function Wait-ForSession {
    param($name, $timeout = 12)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Capture-Pane {
    param($target)
    $raw = & $PSMUX capture-pane -t $target -p 2>&1
    return ($raw | Out-String)
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

function Cleanup-All {
    & $PSMUX kill-server 2>$null
    Start-Sleep -Seconds 2
    Get-Process psmux,tmux,pmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

# ==========================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #165: PredictionViewStyle ListView with allow-predictions on"
Write-Host ("=" * 70)
# ==========================================

$SESSION = "test165"

# --- Test 165.1: allow-predictions on restores PredictionSource ---
Write-Test "165.1: allow-predictions on restores PredictionSource"
try {
    Cleanup-All
    "set -g allow-predictions on" | Set-Content $confPath -Force
    Start-Process -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "165.1: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Write-Host "T1_PS=$((Get-PSReadLineOption).PredictionSource)"' Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane $SESSION

    if ($cap -match "T1_PS=HistoryAndPlugin|T1_PS=History") {
        Write-Pass "165.1: PredictionSource restored to $($Matches[0] -replace 'T1_PS=','')"
    } elseif ($cap -match "T1_PS=None") {
        Write-Fail "165.1: PredictionSource is still None (warm pane not respawned). Output:`n$cap"
    } else {
        Write-Skip "165.1: Could not determine PredictionSource. Output:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "165.1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 165.2: PredictionViewStyle can be set to ListView ---
Write-Test "165.2: PredictionViewStyle can be set to ListView inside session"
try {
    Cleanup-All
    "set -g allow-predictions on" | Set-Content $confPath -Force
    Start-Process -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "165.2: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Set-PSReadLineOption -PredictionViewStyle ListView; Write-Host "T2_VS=$((Get-PSReadLineOption).PredictionViewStyle)"' Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane $SESSION

    if ($cap -match "T2_VS=ListView") {
        Write-Pass "165.2: PredictionViewStyle is ListView after set"
    } elseif ($cap -match "T2_VS=InlineView") {
        Write-Fail "165.2: PredictionViewStyle reverted to InlineView (was overridden). Output:`n$cap"
    } else {
        Write-Skip "165.2: Could not determine PredictionViewStyle. Output:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "165.2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 165.3: show-options includes allow-predictions ---
Write-Test "165.3: show-options includes allow-predictions"
try {
    Cleanup-All
    "set -g allow-predictions on" | Set-Content $confPath -Force
    Start-Process -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "165.3: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 2

    $opts = & $PSMUX show-options -g 2>&1 | Out-String
    if ($opts -match "allow-predictions on") {
        Write-Pass "165.3: show-options reports allow-predictions on"
    } elseif ($opts -match "allow-predictions off") {
        Write-Fail "165.3: show-options reports allow-predictions off (config not loaded). Output:`n$opts"
    } else {
        Write-Fail "165.3: allow-predictions not found in show-options. Output:`n$opts"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "165.3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 165.4: Default (no allow-predictions) still disables predictions ---
Write-Test "165.4: Default config keeps PredictionSource None (no regression)"
try {
    Cleanup-All
    "# empty config" | Set-Content $confPath -Force
    Start-Process -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "165.4: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Write-Host "T4_PS=$((Get-PSReadLineOption).PredictionSource)"' Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane $SESSION

    if ($cap -match "T4_PS=None") {
        Write-Pass "165.4: PredictionSource is None (default behavior preserved)"
    } elseif ($cap -match "T4_PS=History") {
        Write-Fail "165.4: PredictionSource is not None, default behavior regressed. Output:`n$cap"
    } else {
        Write-Skip "165.4: Could not determine PredictionSource. Output:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "165.4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ==========================================
# Cleanup: restore original config
Cleanup-All
if ($origConf) {
    Set-Content $confPath -Value $origConf -Force
} else {
    Remove-Item $confPath -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host ("=" * 70)
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped"
Write-Host ("=" * 70)

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
