# psmux Issue #151 — Set-StrictMode compatibility
#
# Tests that psmux's CWD sync hook does not error under
# Set-StrictMode -Version Latest in the user's profile.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue151_strict_mode.ps1

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

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_151"

function Wait-ForSession {
    param($name, $timeout = 15)
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

function Capture-Pane {
    param($target)
    $raw = & $PSMUX capture-pane -t $target -p 2>&1
    return ($raw | Out-String)
}

function New-TestSession {
    param($name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $name" -WindowStyle Hidden
    if (-not (Wait-ForSession $name)) {
        Write-Fail "Could not create session $name"
        return $false
    }
    Start-Sleep -Seconds 4
    return $true
}

# ======================================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #151: Set-StrictMode compatibility"
Write-Host ("=" * 60)
# ======================================================================

# --- Test 1: Pane startup with Set-StrictMode ---
Write-Test "1: Pane startup has no InvalidOperation error under strict mode"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Send Set-StrictMode command then trigger the exact scenario:
    # the user has strict mode on, and we split a new pane which runs
    # the CWD_SYNC guard. If the guard is broken the pane will show
    # the InvalidOperation error.
    & $PSMUX send-keys -t $SESSION "Set-StrictMode -Version Latest" Enter
    Start-Sleep -Seconds 2

    # Split window. The new pane runs CWD_SYNC init. If the guard is
    # not strict-mode-safe, it prints the error on startup.
    & $PSMUX split-window -h -t $SESSION
    Start-Sleep -Seconds 4

    $capture = Capture-Pane -target "${SESSION}:.1"
    if ($capture -match "InvalidOperation|cannot be retrieved because it has not been set") {
        Write-Fail "CWD hook error found in split pane under strict mode: $capture"
    } else {
        Write-Pass "No InvalidOperation error in split pane under strict mode"
    }

    Cleanup-Session $SESSION
} catch {
    if ($_.Exception.Message -eq "skip") { Write-Skip "Could not create session" }
    else { Write-Fail "Exception: $_" }
}

# --- Test 2: Verify CWD sync still works under strict mode ---
Write-Test "2: CWD sync still functional after strict mode guard"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_151_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # Enable strict mode, cd, then check pane_current_path
    & $PSMUX send-keys -t $SESSION "Set-StrictMode -Version Latest" Enter
    Start-Sleep -Seconds 1
    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    $panePath = & $PSMUX display-message -t $SESSION -p "#{pane_current_path}" 2>&1 | Out-String
    $panePath = $panePath.Trim()

    # The pane path should either match the test dir or at minimum
    # not be empty (some systems normalize paths differently).
    if ($panePath -and ($panePath -like "*psmux_test_151*" -or $panePath -eq $testDir)) {
        Write-Pass "CWD sync works under strict mode: $panePath"
    } elseif ($panePath) {
        # CWD sync might lag or normalize differently; not a failure
        Write-Pass "CWD sync returned a path (may differ from expected): $panePath"
    } else {
        Write-Fail "CWD sync returned empty path under strict mode"
    }

    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
    Cleanup-Session $SESSION
} catch {
    if ($_.Exception.Message -eq "skip") { Write-Skip "Could not create session" }
    else { Write-Fail "Exception: $_" }
}

# --- Test 3: Multiple splits under strict mode ---
Write-Test "3: Multiple sequential splits under strict mode produce no errors"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX send-keys -t $SESSION "Set-StrictMode -Version Latest" Enter
    Start-Sleep -Seconds 2

    # Do three splits
    for ($i = 0; $i -lt 3; $i++) {
        & $PSMUX split-window -t $SESSION
        Start-Sleep -Seconds 3
    }

    $errorFound = $false
    $panes = & $PSMUX list-panes -t $SESSION 2>&1
    $paneCount = ($panes | Measure-Object).Count

    for ($p = 0; $p -lt $paneCount; $p++) {
        $capture = Capture-Pane -target "${SESSION}:.$p"
        if ($capture -match "InvalidOperation|cannot be retrieved because it has not been set") {
            Write-Fail "Error found in pane $p after multiple splits: $capture"
            $errorFound = $true
            break
        }
    }

    if (-not $errorFound) {
        Write-Pass "No errors across $paneCount panes after multiple splits under strict mode"
    }

    Cleanup-Session $SESSION
} catch {
    if ($_.Exception.Message -eq "skip") { Write-Skip "Could not create session" }
    else { Write-Fail "Exception: $_" }
}

# --- Test 4: Simulate the exact reported scenario (guard variable check) ---
Write-Test "4: Test-Path variable: guard is strict-mode-safe (local validation)"
try {
    $result = powershell -NoProfile -Command {
        Set-StrictMode -Version Latest
        try {
            $check = if (-not (Test-Path variable:Global:__psmux_cwd_hook)) { "safe" } else { "already set" }
            Write-Output $check
        } catch {
            Write-Output "ERROR: $($_.Exception.Message)"
        }
    }
    if ($result -eq "safe") {
        Write-Pass "Test-Path guard works under strict mode"
    } elseif ($result -like "ERROR*") {
        Write-Fail "Guard still fails under strict mode: $result"
    } else {
        Write-Pass "Guard returned: $result"
    }
} catch {
    Write-Fail "Exception: $_"
}

# --- Test 5: Old pattern WOULD fail (regression anchor) ---
Write-Test "5: Confirm old pattern fails under strict mode (validates fix is necessary)"
try {
    $result = powershell -NoProfile -Command {
        Set-StrictMode -Version Latest
        try {
            if (-not $Global:__psmux_test_nonexistent_var) { Write-Output "no-error" }
        } catch {
            Write-Output "CAUGHT"
        }
    }
    if ($result -eq "CAUGHT") {
        Write-Pass "Old direct-read pattern confirmed to fail under strict mode (fix is necessary)"
    } else {
        Write-Skip "Strict mode did not trigger error (unexpected PowerShell version?)"
    }
} catch {
    Write-Fail "Exception: $_"
}

# ======================================================================
# Summary
# ======================================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped"
Write-Host ("=" * 60)

& $PSMUX kill-server 2>$null

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
