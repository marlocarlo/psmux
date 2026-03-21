# Test: set -g default-shell "cmd.exe" works correctly
# Verifies cmd.exe can be used as default-shell via config and runtime set-option.
#
# Tests:
#   1. Full path config (C:\Windows\System32\cmd.exe)
#   2. Bare "cmd.exe" config
#   3. new-window inherits cmd.exe
#   4. split-window inherits cmd.exe
#   5. Env vars (PSMUX_SESSION, TMUX) set in cmd panes
#   6. Runtime set-option changes default-shell to cmd.exe
#   7. Bare "cmd" runtime set
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_default_shell_cmd.ps1

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

$cmdPath = "$env:SystemRoot\System32\cmd.exe"
if (-not (Test-Path $cmdPath)) {
    Write-Info "cmd.exe not found at $cmdPath — skipping tests"
    Write-Host "[SKIP] cmd.exe not found" -ForegroundColor Yellow
    exit 0
}
Write-Info "cmd.exe found: $cmdPath"

$confPath = "$env:USERPROFILE\.psmux.conf"
$confBackup = $null

# ============================================================
# SETUP — backup config, kill servers
# ============================================================
Write-Info "Backing up config and cleaning up..."
if (Test-Path $confPath) {
    $confBackup = Get-Content $confPath -Raw
}
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 3
Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  DEFAULT-SHELL CMD.EXE TESTS"
Write-Host ("=" * 60)

# ============================================================
# Test 1: Full path to cmd.exe
# ============================================================
Write-Host ""
Write-Test "1. default-shell with full path to cmd.exe"

Set-Content -Path $confPath -Value "set -g default-shell `"$cmdPath`""

$session = "cmd_test1"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 5

$hasSession = & $PSMUX has-session -t $session 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Pass "Session '$session' created successfully with cmd.exe default-shell"
} else {
    Write-Fail "Failed to create session with cmd.exe default-shell (full path)"
}

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
$cmd = $cmd.Trim()
Write-Info "  pane_current_command: $cmd"
if ($cmd -match "cmd") {
    Write-Pass "Pane is running cmd.exe"
} else {
    Write-Fail "Pane is NOT running cmd.exe (got: $cmd)"
}

# Send a command and verify it works
& $PSMUX send-keys -t $session 'echo CMD_TEST_WORKS' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
if ($output -match "CMD_TEST_WORKS") {
    Write-Pass "cmd.exe pane executes commands correctly"
} else {
    Write-Fail "cmd.exe pane did not produce expected output"
    Write-Info "  Output: $($output.Trim())"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 2: Bare "cmd.exe" name
# ============================================================
Write-Host ""
Write-Test "2. default-shell with bare name: cmd.exe"

Set-Content -Path $confPath -Value 'set -g default-shell cmd.exe'

$session = "cmd_test2"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$hasSession = & $PSMUX has-session -t $session 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Pass "Session created with bare 'cmd.exe' name"
    $cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
    if ($cmd.Trim() -match "cmd") {
        Write-Pass "Pane runs cmd.exe via bare name"
    } else {
        Write-Fail "Pane not running cmd.exe (got: $($cmd.Trim()))"
    }
} else {
    Write-Fail "Failed to create session with bare 'cmd.exe' name"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 3: new-window inherits cmd.exe
# ============================================================
Write-Host ""
Write-Test "3. new-window inherits default-shell cmd.exe"

Set-Content -Path $confPath -Value "set -g default-shell `"$cmdPath`""

$session = "cmd_test3"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

& $PSMUX new-window -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
Write-Info "  new-window pane_current_command: $($cmd.Trim())"
if ($cmd.Trim() -match "cmd") {
    Write-Pass "New window also runs cmd.exe"
} else {
    # Verify by running a cmd command
    & $PSMUX send-keys -t $session 'echo %COMSPEC%' Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $capOut = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
    if ($capOut -match "cmd\.exe") {
        Write-Pass "New window runs cmd.exe (verified via COMSPEC, pane_current_command=$($cmd.Trim()))"
    } else {
        Write-Fail "New window not running cmd.exe (got: $($cmd.Trim()))"
    }
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 4: split-window inherits cmd.exe
# ============================================================
Write-Host ""
Write-Test "4. split-window inherits default-shell cmd.exe"

Set-Content -Path $confPath -Value "set -g default-shell `"$cmdPath`""

$session = "cmd_test4"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

& $PSMUX split-window -t $session -v 2>&1 | Out-Null
Start-Sleep -Seconds 2

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
Write-Info "  split-window pane_current_command: $($cmd.Trim())"
if ($cmd.Trim() -match "cmd") {
    Write-Pass "Split pane runs cmd.exe"
} else {
    Write-Fail "Split pane not running cmd.exe (got: $($cmd.Trim()))"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 5: PSMUX_SESSION and TMUX env vars set in cmd.exe panes
# ============================================================
Write-Host ""
Write-Test "5. Environment variables set correctly in cmd.exe panes"

Set-Content -Path $confPath -Value "set -g default-shell `"$cmdPath`""

$session = "cmd_test5"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

& $PSMUX send-keys -t $session 'echo PSMUX=%PSMUX_SESSION%' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
if ($output -match "PSMUX=.+") {
    Write-Pass "PSMUX_SESSION is set in cmd.exe pane"
} else {
    Write-Fail "PSMUX_SESSION not set in cmd.exe pane"
    Write-Info "  Output: $($output.Trim())"
}

& $PSMUX send-keys -t $session 'echo TMUX_VAR=%TMUX%' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
if ($output -match "TMUX_VAR=.+") {
    Write-Pass "TMUX env var is set in cmd.exe pane"
} else {
    Write-Fail "TMUX env var not set in cmd.exe pane"
    Write-Info "  Output: $($output.Trim())"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 6: Runtime set-option changes default-shell to cmd.exe
# ============================================================
Write-Host ""
Write-Test "6. Runtime set-option to change default-shell to cmd.exe"

Remove-Item $confPath -Force -ErrorAction SilentlyContinue

$session = "cmd_test6"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
Write-Info "  Initial shell: $($cmd.Trim())"

& $PSMUX set-option -g default-shell "`"$cmdPath`"" -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 1

$shellVal = (& $PSMUX show-options -v default-shell -t $session 2>&1) | Out-String
Write-Info "  default-shell after set-option: $($shellVal.Trim())"
if ($shellVal.Trim() -match "cmd") {
    Write-Pass "default-shell option updated to cmd.exe"
} else {
    Write-Fail "default-shell option NOT updated (got: $($shellVal.Trim()))"
}

& $PSMUX new-window -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
Write-Info "  new-window after runtime set: $($cmd.Trim())"
if ($cmd.Trim() -match "cmd") {
    Write-Pass "New window uses cmd.exe after runtime default-shell change"
} else {
    Write-Fail "New window NOT using cmd.exe after runtime change (got: $($cmd.Trim()))"
}

& $PSMUX split-window -t $session -v 2>&1 | Out-Null
Start-Sleep -Seconds 2

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
Write-Info "  split-window after runtime set: $($cmd.Trim())"
if ($cmd.Trim() -match "cmd") {
    Write-Pass "Split pane uses cmd.exe after runtime default-shell change"
} else {
    Write-Fail "Split pane NOT using cmd.exe after runtime change (got: $($cmd.Trim()))"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 7: Runtime set with bare "cmd" name
# ============================================================
Write-Host ""
Write-Test "7. Runtime set-option with bare cmd name"

$session = "cmd_test7"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

& $PSMUX set-option -g default-shell cmd -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 1

& $PSMUX new-window -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$cmd = (& $PSMUX display-message -t $session -p '#{pane_current_command}' 2>&1) | Out-String
Write-Info "  new-window with bare 'cmd': $($cmd.Trim())"
if ($cmd.Trim() -match "cmd") {
    Write-Pass "Runtime set with bare 'cmd' works"
} else {
    Write-Fail "Runtime set with bare 'cmd' failed (got: $($cmd.Trim()))"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# CLEANUP — restore original config
# ============================================================
Write-Host ""
Write-Info "Cleaning up..."
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 1

if ($confBackup) {
    Set-Content -Path $confPath -Value $confBackup
    Write-Info "Restored original config"
} else {
    Remove-Item $confPath -Force -ErrorAction SilentlyContinue
    Write-Info "Removed test config"
}

# ============================================================
# RESULTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) {
    Write-Host "Some tests FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests PASSED — cmd.exe default-shell works correctly" -ForegroundColor Green
    exit 0
}
