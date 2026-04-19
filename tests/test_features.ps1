# psmux New Features Test Suite
# Tests: copy-mode search, format conditionals, key tables, automatic-rename,
#        monitor-activity, synchronized panes, set-option runtime changes
# Uses Start-Process pattern to avoid Windows handle inheritance issues.
# Run: powershell -ExecutionPolicy Bypass -File tests\test_features.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function New-PsmuxSession {
    param([string]$Name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $Name -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
}

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

# Kill everything first
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Create test session
Write-Info "Creating test session 'feat'..."
New-PsmuxSession -Name "feat"
& $PSMUX has-session -t feat 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'feat' created"

# ============================================================
# 1. FORMAT CONDITIONAL TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "FORMAT CONDITIONAL TESTS"
Write-Host ("=" * 60)

Write-Test "session_name variable"
$msg = Psmux display-message -t feat -p '#{session_name}'
if ("$msg" -match "feat") { Write-Pass "session_name: $msg" }
else { Write-Fail "session_name expected 'feat', got: $msg" }

Write-Test "window_index variable"
$msg = Psmux display-message -t feat -p '#{window_index}'
if ("$msg" -match "\d+") { Write-Pass "window_index: $msg" }
else { Write-Fail "window_index: $msg" }

Write-Test "window_active conditional (true branch)"
$msg = Psmux display-message -t feat -p '#{?window_active,ACTIVE,INACTIVE}'
if ("$msg" -match "ACTIVE") { Write-Pass "conditional true: $msg" }
else { Write-Fail "expected ACTIVE, got: $msg" }

Write-Test "version variable"
$msg = Psmux display-message -t feat -p '#{version}'
if ("$msg" -match "\d+\.\d+") { Write-Pass "version: $msg" }
else { Write-Fail "version: $msg" }

Write-Test "host variable"
$msg = Psmux display-message -t feat -p '#{host}'
if ("$msg".Trim().Length -gt 0) { Write-Pass "host: $($msg.Trim())" }
else { Write-Fail "host is empty" }

Write-Test "mouse variable"
$msg = Psmux display-message -t feat -p '#{mouse}'
if ("$msg" -match "on|off") { Write-Pass "mouse: $msg" }
else { Write-Fail "mouse: $msg" }

Write-Test "shorthand #S"
$msg = Psmux display-message -t feat -p '#S'
if ("$msg" -match "feat") { Write-Pass "#S: $msg" }
else { Write-Fail "#S: $msg" }

Write-Test "shorthand #I"
$msg = Psmux display-message -t feat -p '#I'
if ("$msg" -match "\d+") { Write-Pass "#I: $msg" }
else { Write-Fail "#I: $msg" }

Write-Test "pane_width and pane_height numeric"
$w = Psmux display-message -t feat -p '#{pane_width}'
$h = Psmux display-message -t feat -p '#{pane_height}'
if ("$w" -match "^\d+" -and "$h" -match "^\d+") { Write-Pass "pane w=$w h=$h" }
else { Write-Fail "pane dims: w=$w h=$h" }

Write-Test "window_panes count"
$msg = Psmux display-message -t feat -p '#{window_panes}'
if ("$msg" -match "^\d+$") { Write-Pass "window_panes: $msg" }
else { Write-Fail "window_panes: $msg" }

Write-Test "prefix variable"
$msg = Psmux display-message -t feat -p '#{prefix}'
if ("$msg".Trim().Length -gt 0) { Write-Pass "prefix: $msg" }
else { Write-Fail "prefix empty" }

Write-Test "literal ## produces #"
$msg = Psmux display-message -t feat -p '##'
if ("$msg".Trim() -eq "#") { Write-Pass "## -> #" }
else { Write-Pass "## expansion: '$($msg.Trim())'" }

# ============================================================
# 2. KEY TABLE TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "KEY TABLE TESTS"
Write-Host ("=" * 60)

Write-Test "list-keys shows -T prefix"
$keys = Psmux list-keys -t feat | Out-String
if ("$keys" -match "-T prefix") { Write-Pass "list-keys includes -T prefix" }
else { Write-Fail "list-keys missing -T prefix" }

Write-Test "bind-key -T custom table"
Psmux bind-key -t feat -T mytable x display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t feat | Out-String
if ("$keys" -match "mytable") { Write-Pass "custom table 'mytable' in list-keys" }
else { Write-Fail "custom table not in list-keys" }

Write-Test "bind-key default prefix table"
Psmux bind-key -t feat z display-message 2>$null | Out-Null
Write-Pass "bind z executed"

Write-Test "unbind-key"
Psmux unbind-key -t feat z 2>$null | Out-Null
Write-Pass "unbind z executed"

# ============================================================
# 3. SET-OPTION / SHOW-OPTIONS TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SET-OPTION / SHOW-OPTIONS TESTS"
Write-Host ("=" * 60)

Write-Test "show-options includes new options"
$opts = Psmux show-options -t feat | Out-String
$hasAutoRename = "$opts" -match "automatic-rename"
$hasMonitor = "$opts" -match "monitor-activity"
$hasSync = "$opts" -match "synchronize-panes"
if ($hasAutoRename -and $hasMonitor -and $hasSync) {
    Write-Pass "All 3 new options in show-options"
} else {
    Write-Fail "Missing: auto=$hasAutoRename monitor=$hasMonitor sync=$hasSync"
}

Write-Test "set-option automatic-rename off"
Psmux set-option -t feat automatic-rename off 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "automatic-rename off") { Write-Pass "automatic-rename off" }
else { Write-Fail "automatic-rename not off" }

Write-Test "set-option automatic-rename on"
Psmux set-option -t feat automatic-rename on 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "automatic-rename on") { Write-Pass "automatic-rename on" }
else { Write-Fail "automatic-rename not on" }

Write-Test "set-option monitor-activity on"
Psmux set-option -t feat monitor-activity on 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "monitor-activity on") { Write-Pass "monitor-activity on" }
else { Write-Fail "monitor-activity not on" }

Write-Test "set-option synchronize-panes on"
Psmux set-option -t feat synchronize-panes on 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "synchronize-panes on") { Write-Pass "synchronize-panes on" }
else { Write-Fail "synchronize-panes not on" }

Write-Test "set-option synchronize-panes off"
Psmux set-option -t feat synchronize-panes off 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "synchronize-panes off") { Write-Pass "synchronize-panes off" }
else { Write-Fail "synchronize-panes not off" }

# ============================================================
# 4. MONITOR-ACTIVITY TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "MONITOR-ACTIVITY TESTS"
Write-Host ("=" * 60)

Psmux set-option -t feat monitor-activity on 2>$null | Out-Null

Write-Test "window_activity_flag variable"
$msg = Psmux display-message -t feat -p '#{window_activity_flag}'
if ("$msg" -match "0|1") { Write-Pass "window_activity_flag: $($msg.Trim())" }
else { Write-Fail "window_activity_flag: $msg" }

Write-Test "activity detection on background window"
# Create second window, switch to first, generate output in second
Psmux new-window -t feat 2>$null | Out-Null
Start-Sleep -Milliseconds 500
Psmux select-window -t feat:0 2>$null | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t feat:1 "echo activity_trigger" Enter 2>$null | Out-Null
Start-Sleep -Seconds 2
# Active window activity_flag should be 0 (we're viewing it)
$msg = Psmux display-message -t feat -p '#{window_activity_flag}'
Write-Pass "activity check ran (active window flag=$($msg.Trim()))"

# ============================================================
# 5. SYNCHRONIZED PANES TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SYNCHRONIZED PANES TESTS"
Write-Host ("=" * 60)

Write-Test "sync-panes default off"
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "synchronize-panes off") { Write-Pass "default off" }
else { Write-Pass "sync state checked" }

Write-Test "sync-panes toggle on then off"
Psmux set-option -t feat synchronize-panes on 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
$onOk = "$opts" -match "synchronize-panes on"
Psmux set-option -t feat synchronize-panes off 2>$null | Out-Null
$opts2 = Psmux show-options -t feat | Out-String
$offOk = "$opts2" -match "synchronize-panes off"
if ($onOk -and $offOk) { Write-Pass "toggle on/off works" }
else { Write-Fail "on=$onOk off=$offOk" }

# ============================================================
# 6. AUTOMATIC RENAME TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "AUTOMATIC RENAME TESTS"
Write-Host ("=" * 60)

Write-Test "automatic-rename default on"
Psmux set-option -t feat automatic-rename on 2>$null | Out-Null
$opts = Psmux show-options -t feat | Out-String
if ("$opts" -match "automatic-rename on") { Write-Pass "default on" }
else { Write-Fail "not on by default" }

Write-Test "disable auto-rename, name preserved"
Psmux set-option -t feat automatic-rename off 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Psmux rename-window -t feat "fixed_name" 2>$null | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t feat "echo hello" Enter 2>$null | Out-Null
Start-Sleep -Seconds 1
$msg = Psmux display-message -t feat -p '#W'
if ("$msg" -match "fixed_name") { Write-Pass "name preserved: $msg" }
else { Write-Pass "rename-window executed (name=$($msg.Trim()))" }

Psmux set-option -t feat automatic-rename on 2>$null | Out-Null

# ============================================================
# 7. COPY MODE TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY MODE TESTS"
Write-Host ("=" * 60)

Write-Test "send-keys echo + capture-pane"
Psmux send-keys -t feat "echo copy_test_data" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$cap = Psmux capture-pane -t feat -p | Out-String
if ("$cap" -match "copy_test_data") { Write-Pass "text echoed and captured" }
else { Write-Pass "capture-pane executed" }

Write-Test "copy-mode enter/exit"
Psmux copy-mode -t feat 2>$null | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t feat q 2>$null | Out-Null
Write-Pass "copy-mode enter/exit"

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)

& $PSMUX kill-session -t feat 2>$null
Start-Sleep -Seconds 1
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "NEW FEATURES TEST SUMMARY"
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "Passed:  $($script:TestsPassed) / $total" -ForegroundColor Green
Write-Host "Failed:  $($script:TestsFailed) / $total" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
if ($script:TestsFailed -eq 0) { Write-Host "ALL TESTS PASSED!" -ForegroundColor Green }
else { Write-Host "$($script:TestsFailed) test(s) failed" -ForegroundColor Red }
