# psmux/tmux compatibility test suite
# Run all tests for psmux tmux compatibility

$ErrorActionPreference = "Stop"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

# Colors for output
function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

# Get the psmux binary path
$PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Error "psmux binary not found. Please build the project first."
    exit 1
}

Write-Info "Using psmux binary: $PSMUX"
Write-Info "Starting test suite..."
Write-Host ""

# Test Session Management
Write-Host "=" * 60
Write-Host "SESSION MANAGEMENT TESTS"
Write-Host "=" * 60

# Test: list-sessions with no sessions
Write-Test "list-sessions (no sessions)"
try {
    $output = & $PSMUX ls 2>&1
    if ($LASTEXITCODE -eq 0 -or $output -match "no server" -or $output -match "no session") {
        Write-Pass "list-sessions handles no sessions correctly"
        $script:TestsPassed++
    } else {
        Write-Fail "list-sessions unexpected output: $output"
        $script:TestsFailed++
    }
} catch {
    Write-Pass "list-sessions handles no sessions (exception expected)"
    $script:TestsPassed++
}

# Test: has-session with non-existent session  
Write-Test "has-session (non-existent)"
try {
    & $PSMUX has-session -t nonexistent_session_12345 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Pass "has-session returns error for non-existent session"
        $script:TestsPassed++
    } else {
        Write-Fail "has-session should fail for non-existent session"
        $script:TestsFailed++
    }
} catch {
    Write-Pass "has-session handles non-existent session"
    $script:TestsPassed++
}

# Test: version command
Write-Test "version command"
$output = & $PSMUX -V 2>&1
if ($output -match "psmux" -or $output -match "\d+\.\d+") {
    Write-Pass "version command works: $output"
    $script:TestsPassed++
} else {
    Write-Fail "version command failed: $output"
    $script:TestsFailed++
}

# Test: help command
Write-Test "help command"
$output = & $PSMUX --help 2>&1
if ($output -match "USAGE" -or $output -match "COMMANDS") {
    Write-Pass "help command works"
    $script:TestsPassed++
} else {
    Write-Fail "help command failed"
    $script:TestsFailed++
}

# Test: list-commands
Write-Test "list-commands"
$output = & $PSMUX list-commands 2>&1
if ($output -match "attach-session" -or $output -match "split-window") {
    Write-Pass "list-commands shows commands"
    $script:TestsPassed++
} else {
    Write-Fail "list-commands failed: $output"
    $script:TestsFailed++
}

Write-Host ""
Write-Host "=" * 60
Write-Host "COMMAND PARSING TESTS"
Write-Host "=" * 60

# Test: send-keys parsing
Write-Test "send-keys command exists"
$output = & $PSMUX list-commands 2>&1
if ($output -match "send-keys") {
    Write-Pass "send-keys command is available"
    $script:TestsPassed++
} else {
    Write-Fail "send-keys not found in commands"
    $script:TestsFailed++
}

# Test: bind-key command exists
Write-Test "bind-key command exists"
$output = & $PSMUX list-commands 2>&1
if ($output -match "bind-key") {
    Write-Pass "bind-key command is available"
    $script:TestsPassed++
} else {
    Write-Fail "bind-key not found in commands"
    $script:TestsFailed++
}

# Test: set-option command exists
Write-Test "set-option command exists"  
$output = & $PSMUX list-commands 2>&1
if ($output -match "set-option") {
    Write-Pass "set-option command is available"
    $script:TestsPassed++
} else {
    Write-Fail "set-option not found in commands"
    $script:TestsFailed++
}

Write-Host ""
Write-Host "=" * 60
Write-Host "TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host "Skipped: $script:TestsSkipped" -ForegroundColor Yellow
Write-Host ""

if ($script:TestsFailed -gt 0) {
    Write-Host "Some tests failed!" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests passed!" -ForegroundColor Green
    exit 0
}
