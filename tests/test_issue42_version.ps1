# Issue #42 Tests - tmux -V / -v version flag, $TMUX env var, format variables
# https://github.com/psmux/psmux/issues/42
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
    Write-Host "[FATAL] psmux binary not found. Run 'cargo build --release' first." -ForegroundColor Red
    exit 1
}

# Also test the tmux alias binary
$TMUX = "$PSScriptRoot\..\target\release\tmux.exe"
if (-not (Test-Path $TMUX)) {
    $TMUX = "$PSScriptRoot\..\target\debug\tmux.exe"
}

Write-Info "Using psmux binary: $PSMUX"
Write-Info "Using tmux binary: $TMUX"

# ============================================================
# Test 1: psmux -V (capital) prints version and exits
# ============================================================
Write-Test "Test 1: psmux -V prints version"
$output = & $PSMUX -V 2>&1
if ($LASTEXITCODE -eq 0 -and $output -match "psmux \d+\.\d+") {
    Write-Pass "psmux -V prints version: $output"
} else {
    Write-Fail "psmux -V did not print version (exit=$LASTEXITCODE): $output"
}

# ============================================================
# Test 2: psmux -v (lowercase) prints version and exits
# ============================================================
Write-Test "Test 2: psmux -v prints version (not hang/TUI)"
$job = Start-Job -ScriptBlock {
    param($bin)
    & $bin -v 2>&1
} -ArgumentList $PSMUX
$completed = $job | Wait-Job -Timeout 5
if ($completed) {
    $output = Receive-Job $job
    $exitCode = $job.ChildJobs[0].JobStateInfo.Reason
    if ($output -match "psmux \d+\.\d+") {
        Write-Pass "psmux -v prints version: $output"
    } else {
        Write-Fail "psmux -v unexpected output: $output"
    }
} else {
    Write-Fail "psmux -v hung (launched TUI instead of printing version)"
    Stop-Job $job
}
Remove-Job $job -Force

# ============================================================
# Test 3: tmux -V (capital) prints version and exits
# ============================================================
Write-Test "Test 3: tmux -V prints version"
if (Test-Path $TMUX) {
    $output = & $TMUX -V 2>&1
    if ($LASTEXITCODE -eq 0 -and $output -match "tmux \d+\.\d+") {
        Write-Pass "tmux -V prints version: $output"
    } else {
        Write-Fail "tmux -V did not print version (exit=$LASTEXITCODE): $output"
    }
} else {
    Write-Info "Skipped: tmux binary not found"
}

# ============================================================
# Test 4: tmux -v (lowercase) prints version and exits
# ============================================================
Write-Test "Test 4: tmux -v prints version (not hang/TUI)"
if (Test-Path $TMUX) {
    $job = Start-Job -ScriptBlock {
        param($bin)
        & $bin -v 2>&1
    } -ArgumentList $TMUX
    $completed = $job | Wait-Job -Timeout 5
    if ($completed) {
        $output = Receive-Job $job
        if ($output -match "tmux \d+\.\d+") {
            Write-Pass "tmux -v prints version: $output"
        } else {
            Write-Fail "tmux -v unexpected output: $output"
        }
    } else {
        Write-Fail "tmux -v hung (launched TUI instead of printing version)"
        Stop-Job $job
    }
    Remove-Job $job -Force
} else {
    Write-Info "Skipped: tmux binary not found"
}

# ============================================================
# Test 5: psmux --version prints version
# ============================================================
Write-Test "Test 5: psmux --version prints version"
$output = & $PSMUX --version 2>&1
if ($LASTEXITCODE -eq 0 -and $output -match "psmux \d+\.\d+") {
    Write-Pass "psmux --version prints version: $output"
} else {
    Write-Fail "psmux --version did not print version (exit=$LASTEXITCODE): $output"
}

# ============================================================
# Test 6: $TMUX env var inside pane has correct port (not 0)
# ============================================================
Write-Test "Test 6: TMUX env var inside initial pane has non-zero port"
$SESSION_NAME = "issue42_test_$(Get-Random)"
# Kill any lingering sessions
& $PSMUX kill-server -t $SESSION_NAME 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION_NAME -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3

# Verify session started
$sessions = (& $PSMUX ls 2>&1) -join "`n"
if ($sessions -notmatch [regex]::Escape($SESSION_NAME)) {
    Write-Fail "Could not start test session for TMUX env test"
} else {
    # Send a command to echo $TMUX inside the pane
    & $PSMUX send-keys -t $SESSION_NAME "echo TMUX_VAL=`$env:TMUX" Enter
    Start-Sleep -Seconds 2

    $paneContent = & $PSMUX capture-pane -t $SESSION_NAME -p 2>&1
    $paneText = ($paneContent | Out-String)

    # Look for TMUX_VAL= line and check the port is not 0
    if ($paneText -match "TMUX_VAL=/tmp/psmux-\d+/[^,]+,(\d+),\d+") {
        $portVal = $Matches[1]
        if ($portVal -ne "0") {
            Write-Pass "TMUX env has valid port: $portVal"
        } else {
            Write-Fail "TMUX env port is 0 (session lookup will fail)"
        }
    } else {
        Write-Fail "Could not find TMUX env var in pane output. Content: $($paneText.Substring(0, [Math]::Min(200, $paneText.Length)))"
    }
}

# ============================================================
# Test 7: Format variables work without -t from inside pane
# ============================================================
Write-Test "Test 7: Format variables resolve correctly from inside pane context"
# Use the session from Test 6 — send a command that runs psmux display-message
& $PSMUX send-keys -t $SESSION_NAME "& '$PSMUX' display-message -p '#{session_name}'" Enter
Start-Sleep -Seconds 2

$paneContent = & $PSMUX capture-pane -t $SESSION_NAME -p 2>&1
$paneText = ($paneContent | Out-String)

if ($paneText -match [regex]::Escape($SESSION_NAME)) {
    Write-Pass "Format variable #{session_name} resolved to '$SESSION_NAME' inside pane"
} else {
    Write-Fail "Format variable did not resolve inside pane. Content: $($paneText.Substring(0, [Math]::Min(300, $paneText.Length)))"
}

# Cleanup
Write-Info "Cleaning up session: $SESSION_NAME"
& $PSMUX kill-server -t $SESSION_NAME 2>&1 | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Summary
# ============================================================
Write-Host ""
Write-Host "========================================" -ForegroundColor White
Write-Host "  Issue #42 Test Results" -ForegroundColor White
Write-Host "========================================" -ForegroundColor White
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "========================================" -ForegroundColor White

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
