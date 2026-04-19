# Issue #36 - Comprehensive config command tests
# Tests all commands from the reported config file:
#   set -g mouse off, base-index, status-left, status-right, status-style,
#   cursor-style, cursor-blink, history-limit, prediction-dimming,
#   bind-key -T prefix h/v split-window
#
# https://github.com/psmux/psmux/issues/36

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

# Wait for an option to match a pattern (polls show-options)
function Wait-ForOption {
    param($Session, $Binary, $Pattern, $TimeoutSec = 5)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $opts = & $Binary show-options -t $Session 2>&1
        if ($opts -match $Pattern) { return $true }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

# Wait for a single-value option query (-v) to match exact value
function Wait-ForOptionValue {
    param($Session, $Binary, $Name, $Expected, $TimeoutSec = 5)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $val = (& $Binary show-options -v $Name -t $Session 2>&1) | Out-String
        # Only strip line endings, not trailing spaces which may be significant
        $val = $val -replace '[\r\n]+$', ''
        if ($val -eq $Expected) { return $true }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "[FATAL] psmux binary not found" -ForegroundColor Red
    exit 1
}

$SESSION_NAME = "issue36_test_$(Get-Random)"
Write-Info "Using psmux binary: $PSMUX"

# ─── Cleanup stale sessions ──────────────────────────────────
Write-Info "Cleaning up stale sessions..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

# ─── Start test session ──────────────────────────────────────
Write-Info "Starting test session: $SESSION_NAME"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION_NAME -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3

$sessions = (& $PSMUX ls 2>&1) -join "`n"
if ($sessions -notmatch [regex]::Escape($SESSION_NAME)) {
    Write-Host "[FATAL] Could not start test session. Output: $sessions" -ForegroundColor Red
    exit 1
}
Write-Info "Session started successfully"
Write-Host ""

# ═══════════════════════════════════════════════════════════════
Write-Host "=" * 60
Write-Host "ISSUE #36 - CONFIG COMMAND TESTS"
Write-Host "=" * 60

# ─── 1. set -g mouse off ─────────────────────────────────────
Write-Host ""
Write-Host "--- mouse option ---"

Write-Test "Default mouse is on"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "mouse" -Expected "on") {
    Write-Pass "Default mouse is on"
} else {
    $v = (& $PSMUX show-options -v mouse -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected mouse=on, got: '$($v.Trim())'"
}

Write-Test "set -g mouse off"
& $PSMUX set-option -t $SESSION_NAME mouse off 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "mouse" -Expected "off") {
    Write-Pass "mouse set to off"
} else {
    $v = (& $PSMUX show-options -v mouse -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected mouse=off, got: '$($v.Trim())'"
}

Write-Test "show-options reflects mouse off"
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "mouse off") {
    Write-Pass "show-options shows mouse off"
} else {
    Write-Fail "show-options does not show mouse off"
}

Write-Test "set -g mouse on (restore)"
& $PSMUX set-option -t $SESSION_NAME mouse on 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "mouse" -Expected "on") {
    Write-Pass "mouse restored to on"
} else {
    Write-Fail "Failed to restore mouse to on"
}

# ─── 2. set -g base-index 1 ──────────────────────────────────
Write-Host ""
Write-Host "--- base-index option ---"

Write-Test "Default base-index is 0"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "base-index" -Expected "0") {
    Write-Pass "Default base-index is 0"
} else {
    $v = (& $PSMUX show-options -v base-index -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected base-index=0, got: '$($v.Trim())'"
}

Write-Test "set -g base-index 0"
& $PSMUX set-option -t $SESSION_NAME base-index 0 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "base-index" -Expected "0") {
    Write-Pass "base-index set to 0"
} else {
    Write-Fail "Failed to set base-index to 0"
}

Write-Test "set -g base-index 1 (restore)"
& $PSMUX set-option -t $SESSION_NAME base-index 1 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "base-index" -Expected "1") {
    Write-Pass "base-index restored to 1"
} else {
    Write-Fail "Failed to restore base-index to 1"
}

# ─── 3. set -g status-left ───────────────────────────────────
Write-Host ""
Write-Host "--- status-left option ---"

Write-Test "set -g status-left '[#S] '"
& $PSMUX set-option -t $SESSION_NAME status-left "[#S] " 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-left" -Expected "[#S] ") {
    Write-Pass "status-left set to '[#S] '"
} else {
    $v = (& $PSMUX show-options -v status-left -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-left='[#S] ', got: '$($v.Trim())'"
}

Write-Test "show-options reflects status-left"
$opts = (& $PSMUX show-options -t $SESSION_NAME 2>&1) -join "`n"
if ($opts -match 'status-left "\[#S\] "') {
    Write-Pass "show-options shows status-left"
} else {
    Write-Fail "show-options does not show status-left correctly: $opts"
}

# ─── 4. set -g status-right ──────────────────────────────────
Write-Host ""
Write-Host "--- status-right option ---"

Write-Test "set -g status-right '%H:%M %d-%b-%y'"
& $PSMUX set-option -t $SESSION_NAME status-right "%H:%M %d-%b-%y" 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-right" -Expected "%H:%M %d-%b-%y") {
    Write-Pass "status-right set to '%H:%M %d-%b-%y'"
} else {
    $v = (& $PSMUX show-options -v status-right -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-right='%H:%M %d-%b-%y', got: '$($v.Trim())'"
}

# ─── 5. set -g status-style ──────────────────────────────────
Write-Host ""
Write-Host "--- status-style option ---"

Write-Test "set -g status-style 'bg=green,fg=black'"
& $PSMUX set-option -t $SESSION_NAME status-style "bg=green,fg=black" 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-style" -Expected "bg=green,fg=black") {
    Write-Pass "status-style set correctly"
} else {
    $v = (& $PSMUX show-options -v status-style -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-style='bg=green,fg=black', got: '$($v.Trim())'"
}

Write-Test "set -g status-style with different colors"
& $PSMUX set-option -t $SESSION_NAME status-style "bg=blue,fg=white" 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-style" -Expected "bg=blue,fg=white") {
    Write-Pass "status-style changed to bg=blue,fg=white"
} else {
    $v = (& $PSMUX show-options -v status-style -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-style='bg=blue,fg=white', got: '$($v.Trim())'"
}

# ─── 6. set -g cursor-style ──────────────────────────────────
Write-Host ""
Write-Host "--- cursor-style option ---"

Write-Test "set -g cursor-style bar"
& $PSMUX set-option -t $SESSION_NAME cursor-style bar 2>&1
Start-Sleep -Milliseconds 500
$v = (& $PSMUX show-options -v cursor-style -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "bar") {
    Write-Pass "cursor-style set to bar (visible via show-options -v)"
} else {
    Write-Fail "Expected cursor-style=bar, got: '$v'"
}

Write-Test "set -g cursor-style block"
& $PSMUX set-option -t $SESSION_NAME cursor-style block 2>&1
Start-Sleep -Milliseconds 500
$v = (& $PSMUX show-options -v cursor-style -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "block") {
    Write-Pass "cursor-style set to block"
} else {
    Write-Fail "Expected cursor-style=block, got: '$v'"
}

Write-Test "set -g cursor-style underline"
& $PSMUX set-option -t $SESSION_NAME cursor-style underline 2>&1
Start-Sleep -Milliseconds 500
$v = (& $PSMUX show-options -v cursor-style -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "underline") {
    Write-Pass "cursor-style set to underline"
} else {
    Write-Fail "Expected cursor-style=underline, got: '$v'"
}

Write-Test "cursor-style appears in show-options full dump"
$opts = (& $PSMUX show-options -t $SESSION_NAME 2>&1) -join "`n"
if ($opts -match "cursor-style") {
    Write-Pass "cursor-style visible in show-options"
} else {
    Write-Fail "cursor-style not visible in show-options"
}

# ─── 7. set -g cursor-blink ──────────────────────────────────
Write-Host ""
Write-Host "--- cursor-blink option ---"

Write-Test "set -g cursor-blink on"
& $PSMUX set-option -t $SESSION_NAME cursor-blink on 2>&1
Start-Sleep -Milliseconds 500
$v = (& $PSMUX show-options -v cursor-blink -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "on") {
    Write-Pass "cursor-blink set to on"
} else {
    Write-Fail "Expected cursor-blink=on, got: '$v'"
}

Write-Test "set -g cursor-blink off"
& $PSMUX set-option -t $SESSION_NAME cursor-blink off 2>&1
Start-Sleep -Milliseconds 500
$v = (& $PSMUX show-options -v cursor-blink -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "off") {
    Write-Pass "cursor-blink set to off"
} else {
    Write-Fail "Expected cursor-blink=off, got: '$v'"
}

Write-Test "cursor-blink appears in show-options full dump"
$opts = (& $PSMUX show-options -t $SESSION_NAME 2>&1) -join "`n"
if ($opts -match "cursor-blink") {
    Write-Pass "cursor-blink visible in show-options"
} else {
    Write-Fail "cursor-blink not visible in show-options"
}

# ─── 8. set -g history-limit ─────────────────────────────────
Write-Host ""
Write-Host "--- history-limit option ---"

Write-Test "Default history-limit is 2000"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "history-limit" -Expected "2000") {
    Write-Pass "Default history-limit is 2000"
} else {
    $v = (& $PSMUX show-options -v history-limit -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected history-limit=2000, got: '$($v.Trim())'"
}

Write-Test "set -g history-limit 9999"
& $PSMUX set-option -t $SESSION_NAME history-limit 9999 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "history-limit" -Expected "9999") {
    Write-Pass "history-limit set to 9999"
} else {
    $v = (& $PSMUX show-options -v history-limit -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected history-limit=9999, got: '$($v.Trim())'"
}

Write-Test "set -g history-limit 2000 (restore)"
& $PSMUX set-option -t $SESSION_NAME history-limit 2000 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "history-limit" -Expected "2000") {
    Write-Pass "history-limit restored to 2000"
} else {
    Write-Fail "Failed to restore history-limit to 2000"
}

# ─── 9. set -g prediction-dimming ────────────────────────────
Write-Host ""
Write-Host "--- prediction-dimming option ---"

Write-Test "set -g prediction-dimming off"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming off 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "prediction-dimming" -Expected "off") {
    Write-Pass "prediction-dimming set to off"
} else {
    $v = (& $PSMUX show-options -v prediction-dimming -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected prediction-dimming=off, got: '$($v.Trim())'"
}

Write-Test "set -g prediction-dimming on"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming on 2>&1
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "prediction-dimming" -Expected "on") {
    Write-Pass "prediction-dimming restored to on"
} else {
    Write-Fail "Failed to restore prediction-dimming to on"
}

# ─── 10. bind-key -T prefix h split-window -h ────────────────
Write-Host ""
Write-Host "--- bind-key tests ---"

Write-Test "bind-key -T prefix h split-window -h"
& $PSMUX bind-key -T prefix h split-window -h -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$keys = (& $PSMUX list-keys -t $SESSION_NAME 2>&1) -join "`n"
if ($keys -match "prefix.*h.*split.*-h") {
    Write-Pass "bind-key -T prefix h split-window -h registered"
} else {
    Write-Fail "bind-key -T prefix h not found in list-keys: $keys"
}

Write-Test "bind-key -T prefix v split-window -v"
& $PSMUX bind-key -T prefix v split-window -v -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$keys = (& $PSMUX list-keys -t $SESSION_NAME 2>&1) -join "`n"
if ($keys -match "prefix.*v.*split.*-v") {
    Write-Pass "bind-key -T prefix v split-window -v registered"
} else {
    Write-Fail "bind-key -T prefix v not found in list-keys: $keys"
}

# ─── 11. source-file test with full issue config ─────────────
Write-Host ""
Write-Host "--- source-file with full issue #36 config ---"

$CONFIG_FILE = "$PSScriptRoot\test_issue36.conf"
$configContent = @"
set -g mouse off
set -g base-index 1

# Customize status bar
set -g status-left "[#S] "
set -g status-right "%H:%M %d-%b-%y"
set -g status-style "bg=green,fg=black"

# Cursor style: block, underline, or bar
set -g cursor-style bar
set -g cursor-blink on

# Scrollback history
set -g history-limit 9999

# Prediction dimming (disable for apps like Neovim)
set -g prediction-dimming off

# Key bindings
bind-key -T prefix h split-window -h
bind-key -T prefix v split-window -v
"@

Set-Content -Path $CONFIG_FILE -Value $configContent -Encoding UTF8
Write-Test "source-file with full issue #36 config"
& $PSMUX source-file $CONFIG_FILE -t $SESSION_NAME 2>&1
Start-Sleep -Seconds 2

# Verify each option from the config file
Write-Test "Verify mouse off after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "mouse" -Expected "off") {
    Write-Pass "mouse=off after source-file"
} else {
    $v = (& $PSMUX show-options -v mouse -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected mouse=off after source-file, got: '$($v.Trim())'"
}

Write-Test "Verify base-index 1 after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "base-index" -Expected "1") {
    Write-Pass "base-index=1 after source-file"
} else {
    Write-Fail "base-index not 1 after source-file"
}

Write-Test "Verify status-left after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-left" -Expected "[#S] ") {
    Write-Pass "status-left='[#S] ' after source-file"
} else {
    $v = (& $PSMUX show-options -v status-left -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-left='[#S] ', got: '$($v.Trim())'"
}

Write-Test "Verify status-right after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-right" -Expected "%H:%M %d-%b-%y") {
    Write-Pass "status-right='%H:%M %d-%b-%y' after source-file"
} else {
    $v = (& $PSMUX show-options -v status-right -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-right='%H:%M %d-%b-%y', got: '$($v.Trim())'"
}

Write-Test "Verify status-style after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "status-style" -Expected "bg=green,fg=black") {
    Write-Pass "status-style='bg=green,fg=black' after source-file"
} else {
    $v = (& $PSMUX show-options -v status-style -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected status-style='bg=green,fg=black', got: '$($v.Trim())'"
}

Write-Test "Verify cursor-style after source-file"
$v = (& $PSMUX show-options -v cursor-style -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "bar") {
    Write-Pass "cursor-style=bar after source-file"
} else {
    Write-Fail "Expected cursor-style=bar after source-file, got: '$v'"
}

Write-Test "Verify cursor-blink after source-file"
$v = (& $PSMUX show-options -v cursor-blink -t $SESSION_NAME 2>&1) | Out-String
$v = $v.Trim()
if ($v -eq "on") {
    Write-Pass "cursor-blink=on after source-file"
} else {
    Write-Fail "Expected cursor-blink=on after source-file, got: '$v'"
}

Write-Test "Verify history-limit after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "history-limit" -Expected "9999") {
    Write-Pass "history-limit=9999 after source-file"
} else {
    $v = (& $PSMUX show-options -v history-limit -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected history-limit=9999, got: '$($v.Trim())'"
}

Write-Test "Verify prediction-dimming after source-file"
if (Wait-ForOptionValue -Session $SESSION_NAME -Binary $PSMUX -Name "prediction-dimming" -Expected "off") {
    Write-Pass "prediction-dimming=off after source-file"
} else {
    $v = (& $PSMUX show-options -v prediction-dimming -t $SESSION_NAME 2>&1) | Out-String
    Write-Fail "Expected prediction-dimming=off, got: '$($v.Trim())'"
}

Write-Test "Verify bind h split-window -h after source-file"
$keys = (& $PSMUX list-keys -t $SESSION_NAME 2>&1) -join "`n"
if ($keys -match "prefix.*h.*split.*-h") {
    Write-Pass "bind h split-window -h present after source-file"
} else {
    Write-Fail "bind h split-window -h NOT found after source-file"
}

Write-Test "Verify bind v split-window -v after source-file"
if ($keys -match "prefix.*v.*split.*-v") {
    Write-Pass "bind v split-window -v present after source-file"
} else {
    Write-Fail "bind v split-window -v NOT found after source-file"
}

# ─── Cleanup ──────────────────────────────────────────────────
Write-Host ""
Write-Info "Cleaning up..."

# Kill session
& $PSMUX kill-session -t $SESSION_NAME 2>&1
Start-Sleep -Seconds 1

# Remove test config file
if (Test-Path $CONFIG_FILE) {
    Remove-Item $CONFIG_FILE -Force
    Write-Info "Removed test config file"
}

Write-Host ""
Write-Host "=" * 60
Write-Host "ISSUE #36 TEST SUMMARY"
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
