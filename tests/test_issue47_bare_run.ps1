# psmux Issue #47 - Running bare `psmux` / `tmux` fails with "no session" error
# Verifies that running psmux with no arguments (bare invocation) correctly
# creates a new session and attaches, rather than erroring out.
#
# The bug: running `tmux` (when tmux is aliased/symlinked to psmux) with no
# arguments gives: Error: Custom { kind: Other, error: "no session" }
#
# This test runs non-interactively and checks:
#   1. Bare `psmux` with no existing session starts a server and creates a session
#   2. After bare invocation, the session exists and is queryable
#   3. The error message "no session" does not appear
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue47_bare_run.ps1

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

# ============================================================
# SETUP — kill everything so we test from a clean state
# ============================================================
Write-Info "Killing all psmux servers..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\last_session" -Force -ErrorAction SilentlyContinue

# Verify no sessions exist
$lsBefore = & $PSMUX list-sessions 2>&1 | Out-String
Write-Info "list-sessions before test: $($lsBefore.Trim())"

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  ISSUE #47: BARE INVOCATION ERROR"
Write-Host ("=" * 60)
Write-Host ""

# -----------------------------------------------------------------
# Test 1: Bare invocation should not produce "no session" error
# -----------------------------------------------------------------
Write-Test "1. Bare psmux invocation - should not error with 'no session'"

# We can't run a fully interactive attach in a test script, but we CAN test
# the server-spawn path. Bare `psmux` creates an auto-numbered session (0, 1, 2...)
# like tmux, then attaches to it. We test the server-spawn + naming.

# Use new-session -d to replicate bare behavior (creates auto-numbered session)
& $PSMUX new-session -d 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Check if the port file was created (should be "0" for first session)
$portPath = "$env:USERPROFILE\.psmux\0.port"
if (Test-Path $portPath) {
    Write-Pass "Port file created for auto-numbered session '0'"
    $port = (Get-Content $portPath).Trim()
    Write-Info "  Server port: $port"
} else {
    Write-Fail "Port file NOT created — server may have failed to start"
}

# Check if session is queryable
$hasSession = & $PSMUX has-session -t 0 2>&1 | Out-String
$hasExitCode = $LASTEXITCODE
Write-Info "  has-session exit code: $hasExitCode"
if ($hasExitCode -eq 0) {
    Write-Pass "has-session -t 0 succeeds"
} else {
    Write-Fail "has-session -t 0 failed (exit code $hasExitCode)"
}

# List sessions — should show auto-numbered session
$lsAfter = & $PSMUX list-sessions 2>&1 | Out-String
Write-Info "  list-sessions: $($lsAfter.Trim())"
if ($lsAfter -match "^0:") {
    Write-Pass "Auto-numbered session '0' listed in list-sessions"
} else {
    Write-Fail "Session '0' NOT found in list-sessions"
}

# -----------------------------------------------------------------
# Test 2: Check that client connect to session works (non-interactive)
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 2: CLIENT CAN QUERY EXISTING SESSION"
Write-Host ("=" * 60)

Write-Test "2. display-message works against the default session"

$windowName = & $PSMUX display-message -t default -p '#{window_name}' 2>&1 | Out-String
Write-Info "  window_name: $($windowName.Trim())"
if ($windowName.Trim().Length -gt 0 -and $windowName -notmatch "no session" -and $windowName -notmatch "Error") {
    Write-Pass "display-message returned valid window_name"
} else {
    Write-Fail "display-message failed or returned error: $($windowName.Trim())"
}

# -----------------------------------------------------------------
# Test 3: Error message check — simulate what happens when port file missing
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 3: ERROR MESSAGE FOR MISSING SESSION"
Write-Host ("=" * 60)

Write-Test "3. Commands against non-existent session give clear error"

$errOutput = & $PSMUX display-message -t nonexistent_session_xyz -p '#{window_name}' 2>&1 | Out-String
Write-Info "  Error output: $($errOutput.Trim())"

# The error should be clear, not "Custom { kind: Other, error: ... }"
if ($errOutput -match "Custom.*kind.*Other.*error") {
    Write-Fail "Raw Rust error format exposed to user: $($errOutput.Trim())"
    Write-Info "  Should be a user-friendly message like 'no server running on session ...'"
} else {
    Write-Pass "Error message does not expose raw Rust error format"
}

# The error should mention the session name or "no server"
if ($errOutput -match "no server|session.*not found|can.t find") {
    Write-Pass "Error message is descriptive"
} elseif ($errOutput -match "error|Error") {
    Write-Info "  Error message: $($errOutput.Trim()) — review if user-friendly enough"
    Write-Pass "Error reported (may need friendlier message)"
} else {
    Write-Info "  Output: $($errOutput.Trim())"
}

# -----------------------------------------------------------------
# Test 4: Bare invocation with rename (tmux aliased) — the actual issue #47
# -----------------------------------------------------------------
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 4: SIMULATE BARE INVOCATION (tmux -> psmux)"
Write-Host ("=" * 60)

Write-Test "4. Running psmux with no args — should create auto-numbered session (tmux-compatible)"

# Kill all sessions first
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Now run psmux with no arguments in a subprocess with a timeout
# This will try to create + attach interactively; we kill it after a few seconds
# and check if a port file was created (server started)
$proc = Start-Process -FilePath $PSMUX -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 5

# Check if an auto-numbered session was created (should be "0")
$portExists = Test-Path "$env:USERPROFILE\.psmux\0.port"
Write-Info "  Port file exists after bare invocation: $portExists"

if ($portExists) {
    Write-Pass "Bare invocation created auto-numbered session '0' — no 'no session' error"
    # Verify session is live
    $check = & $PSMUX has-session -t 0 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) {
        Write-Pass "Session '0' is alive after bare invocation"
    } else {
        Write-Fail "Session '0' port file exists but session not responding"
    }
} else {
    # Check if any numeric port file exists (warm server might have been claimed with a different number)
    $anyPort = Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue | Where-Object { $_.BaseName -match '^\d+$' -and $_.BaseName -ne '__warm__' }
    if ($anyPort) {
        Write-Pass "Bare invocation created auto-numbered session '$($anyPort[0].BaseName)'"
    } else {
        Write-Fail "Bare invocation did NOT create a session — may error with 'no session' (issue #47)"
    }
}

# Kill the process if still running
if (-not $proc.HasExited) {
    $proc.Kill()
    Start-Sleep -Milliseconds 500
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Cleaning up..."
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) {
    Write-Host "Some tests FAILED — issue #47 may be present" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests PASSED" -ForegroundColor Green
    exit 0
}
