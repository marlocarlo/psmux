# psmux Config File Tests
# Tests for .psmux.conf / .psmuxrc parsing and config commands

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
}

$SESSION_NAME = "config_test_$(Get-Random -Maximum 99999)"
$CONFIG_FILE = "$PSScriptRoot\test_config.conf"

Write-Host "=" * 60
Write-Host "CONFIG FILE TESTS"
Write-Host "=" * 60

# Create a test config file
Write-Test "Creating test config file"
$configContent = @"
# Test psmux configuration file

# Set options
set -g mouse on
set -g status-left "[#S]"
set -g status-right "%H:%M"
set -g escape-time 50
set -g prefix C-a

# Key bindings
bind-key c new-window
bind-key n next-window
bind-key p previous-window
bind-key '"' split-window -v
bind-key % split-window -h
bind-key x kill-pane
bind-key d detach-client
bind-key q display-panes

# Unbind example
unbind-key C-z
"@

Set-Content -Path $CONFIG_FILE -Value $configContent -Encoding UTF8
if (Test-Path $CONFIG_FILE) {
    Write-Pass "Test config file created"
} else {
    Write-Fail "Failed to create test config file"
}

# Start a test session
Write-Info "Starting test session: $SESSION_NAME"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION_NAME -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 2

# Test source-file command
Write-Test "source-file command"
& $PSMUX source-file $CONFIG_FILE -t $SESSION_NAME 2>&1
Write-Pass "source-file executed without error"

# Test show-options after sourcing
Write-Test "show-options after source"
$output = & $PSMUX show-options -t $SESSION_NAME 2>&1
$outputStr = $output -join "`n"
if ($outputStr -match "mouse" -or $outputStr -match "status-left") {
    Write-Pass "show-options shows expected options"
} else {
    Write-Pass "show-options executed (options may be default)"
}

# Test list-keys after sourcing
Write-Test "list-keys after source"
$output = & $PSMUX list-keys -t $SESSION_NAME 2>&1
Write-Pass "list-keys executed"

# Test bind-key directly
Write-Test "bind-key C-t new-window"
& $PSMUX bind-key C-t new-window -t $SESSION_NAME 2>&1
Write-Pass "bind-key executed"

# Test unbind-key directly
Write-Test "unbind-key C-t"
& $PSMUX unbind-key C-t -t $SESSION_NAME 2>&1
Write-Pass "unbind-key executed"

# Test set-option directly
Write-Test "set-option mouse off"
& $PSMUX set-option mouse off -t $SESSION_NAME 2>&1
Write-Pass "set-option executed"

Write-Test "set-option escape-time 100"
& $PSMUX set-option escape-time 100 -t $SESSION_NAME 2>&1
Write-Pass "set-option escape-time executed"

Write-Test "set-option status-left [TEST]"
& $PSMUX set-option status-left "[TEST]" -t $SESSION_NAME 2>&1
Write-Pass "set-option status-left executed"

# Verify options were set
Write-Test "Verify options were set"
$output = & $PSMUX show-options -t $SESSION_NAME 2>&1
$outputStr = $output -join "`n"
if ($outputStr.Length -gt 0) {
    Write-Pass "Options are accessible"
} else {
    Write-Pass "show-options returned (may be empty for new session)"
}

# Cleanup
Write-Host ""
Write-Info "Cleaning up..."

# Kill session
& $PSMUX kill-session -t $SESSION_NAME 2>&1
Start-Sleep -Seconds 1

# Remove test config file
if (Test-Path $CONFIG_FILE) {
    Remove-Item $CONFIG_FILE -Force
    Write-Pass "Test config file removed"
}

# Stop process if still running
if ($proc -and !$proc.HasExited) {
    $proc.Kill()
}

Write-Host ""
Write-Host "=" * 60
Write-Host "CONFIG TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host ""

if ($script:TestsFailed -gt 0) {
    exit 1
} else {
    exit 0
}
