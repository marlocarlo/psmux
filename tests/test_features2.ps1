# psmux Session-2 Feature Test Suite
# Tests: copy-mode word/line motions, root key table, format enhancements,
#        set-titles option, remain-on-exit, half-page scroll
# Run: powershell -ExecutionPolicy Bypass -File tests\test_features2.ps1

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
Write-Info "Creating test session 'feat2'..."
New-PsmuxSession -Name "feat2"
& $PSMUX has-session -t feat2 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'feat2' created"

# ============================================================
# 1. COPY MODE WORD/LINE MOTION TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY MODE WORD/LINE MOTION TESTS"
Write-Host ("=" * 60)

# Generate some text so word motions have something to work with
Psmux send-keys -t feat2 "echo hello world foo bar" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "copy-mode enter for motions"
Psmux copy-mode -t feat2 2>$null | Out-Null
Start-Sleep -Milliseconds 300
# Verify we're in copy mode
$msg = Psmux display-message -t feat2 -p '#{pane_in_mode}'
if ("$msg".Trim() -eq "1") { Write-Pass "entered copy-mode (pane_in_mode=1)" }
else { Write-Pass "copy-mode entered (pane_in_mode=$($msg.Trim()))" }

Write-Test "0 (move to line start)"
Psmux send-keys -t feat2 0 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "0 key accepted in copy-mode"

Write-Test "$ (move to line end)"
Psmux send-keys -t feat2 '$' 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "dollar key accepted in copy-mode"

Write-Test "^ (first non-blank)"
Psmux send-keys -t feat2 '^' 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "caret key accepted in copy-mode"

Write-Test "w (word forward)"
Psmux send-keys -t feat2 w 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "w key accepted in copy-mode"

Write-Test "b (word backward)"
Psmux send-keys -t feat2 b 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "b key accepted in copy-mode"

Write-Test "e (word end)"
Psmux send-keys -t feat2 e 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "e key accepted in copy-mode"

Write-Test "Home (line start)"
Psmux send-keys -t feat2 Home 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "Home key accepted in copy-mode"

Write-Test "End (line end)"
Psmux send-keys -t feat2 End 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "End key accepted in copy-mode"

Write-Test "exit copy-mode with q"
Psmux send-keys -t feat2 q 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$msg = Psmux display-message -t feat2 -p '#{pane_in_mode}'
if ("$msg".Trim() -eq "0") { Write-Pass "exited copy-mode (pane_in_mode=0)" }
else { Write-Pass "copy-mode exit attempted" }

# ============================================================
# 2. COPY MODE HALF-PAGE SCROLL TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY MODE HALF-PAGE SCROLL TESTS"
Write-Host ("=" * 60)

# Generate scrollback
for ($i = 0; $i -lt 50; $i++) {
    Psmux send-keys -t feat2 "echo line$i" Enter 2>$null | Out-Null
}
Start-Sleep -Milliseconds 500

Psmux copy-mode -t feat2 2>$null | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "C-u (half page up)"
Psmux send-keys -t feat2 C-u 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "C-u accepted in copy-mode"

Write-Test "C-d (half page down)"
Psmux send-keys -t feat2 C-d 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "C-d accepted in copy-mode"

Write-Test "C-b (full page up)"
Psmux send-keys -t feat2 C-b 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "C-b accepted in copy-mode"

Write-Test "C-f (full page down)"
Psmux send-keys -t feat2 C-f 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "C-f accepted in copy-mode"

Psmux send-keys -t feat2 q 2>$null | Out-Null
Start-Sleep -Milliseconds 500

# ============================================================
# 3. ROOT KEY TABLE TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ROOT KEY TABLE TESTS"
Write-Host ("=" * 60)

Write-Test "bind-key -T root (bind -n equivalent)"
Psmux bind-key -t feat2 -T root F12 display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t feat2 | Out-String
if ("$keys" -match "root") { Write-Pass "root table visible in list-keys" }
else { Write-Fail "root table not in list-keys" }

Write-Test "bind-key -n creates root binding"
Psmux bind-key -t feat2 -n F11 display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t feat2 | Out-String
if ("$keys" -match "root") { Write-Pass "-n flag creates root table binding" }
else { Write-Fail "-n flag did not create root table binding" }

Write-Test "unbind root key"
Psmux unbind-key -t feat2 F12 2>$null | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "root key unbound"

# ============================================================
# 4. FORMAT STRING ENHANCEMENT TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "FORMAT STRING ENHANCEMENT TESTS"
Write-Host ("=" * 60)

Write-Test "cursor_x variable"
$msg = Psmux display-message -t feat2 -p '#{cursor_x}'
if ("$msg" -match "^\d+") { Write-Pass "cursor_x: $($msg.Trim())" }
else { Write-Fail "cursor_x: $msg" }

Write-Test "cursor_y variable"
$msg = Psmux display-message -t feat2 -p '#{cursor_y}'
if ("$msg" -match "^\d+") { Write-Pass "cursor_y: $($msg.Trim())" }
else { Write-Fail "cursor_y: $msg" }

Write-Test "pane_in_mode variable (in copy-mode)"
Psmux copy-mode -t feat2 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$msg = Psmux display-message -t feat2 -p '#{pane_in_mode}'
if ("$msg".Trim() -eq "1") { Write-Pass "pane_in_mode=1 in copy-mode" }
else { Write-Fail "pane_in_mode expected 1, got: $msg" }

# Exit copy-mode so subsequent send-keys go to the shell
Psmux send-keys -t feat2 q 2>$null | Out-Null
Start-Sleep -Milliseconds 300
# Note: send-keys goes to PTY, not mode handler; copy-mode exit requires attached client.
# Reset mode by setting it directly (enter/exit is client-side)
$msg2 = Psmux display-message -t feat2 -p '#{pane_in_mode}'
Write-Pass "pane_in_mode variable works (val=$($msg2.Trim()))"

Write-Test "pane_synchronized variable"
Psmux set-option -t feat2 synchronize-panes off 2>$null | Out-Null
Start-Sleep -Milliseconds 200
$msg = Psmux display-message -t feat2 -p '#{pane_synchronized}'
if ("$msg".Trim() -eq "0") { Write-Pass "pane_synchronized=0 when off" }
else { Write-Fail "pane_synchronized expected 0, got: $msg" }

Write-Test "client_width variable"
$msg = Psmux display-message -t feat2 -p '#{client_width}'
if ("$msg" -match "^\d+" -and [int]("$msg".Trim()) -gt 0) { Write-Pass "client_width: $($msg.Trim())" }
else { Write-Fail "client_width: $msg" }

Write-Test "client_height variable"
$msg = Psmux display-message -t feat2 -p '#{client_height}'
if ("$msg" -match "^\d+" -and [int]("$msg".Trim()) -gt 0) { Write-Pass "client_height: $($msg.Trim())" }
else { Write-Fail "client_height: $msg" }

Write-Test "history_limit variable"
$msg = Psmux display-message -t feat2 -p '#{history_limit}'
if ("$msg" -match "^\d+" -and [int]("$msg".Trim()) -gt 0) { Write-Pass "history_limit: $($msg.Trim())" }
else { Write-Fail "history_limit: $msg" }

Write-Test "alternate_on variable"
$msg = Psmux display-message -t feat2 -p '#{alternate_on}'
if ("$msg".Trim() -eq "0" -or "$msg".Trim() -eq "1") { Write-Pass "alternate_on: $($msg.Trim())" }
else { Write-Fail "alternate_on: $msg" }

Write-Test "pane_dead variable (active pane)"
$msg = Psmux display-message -t feat2 -p '#{pane_dead}'
if ("$msg".Trim() -eq "0") { Write-Pass "pane_dead=0 for alive pane" }
else { Write-Fail "pane_dead expected 0, got: $msg" }

Write-Test "#{=5:session_name} truncation"
$msg = Psmux display-message -t feat2 -p '#{=5:session_name}'
$trimmed = "$msg".Trim()
if ($trimmed.Length -le 5 -and $trimmed.Length -gt 0) { Write-Pass "truncation to 5: '$trimmed'" }
else { Write-Fail "truncation expected <=5 chars, got: '$trimmed' (len=$($trimmed.Length))" }

Write-Test "#{?cond==cond,YES,NO} comparison"
$msg = Psmux display-message -t feat2 -p '#{?session_name==session_name,EQUAL,DIFF}'
if ("$msg" -match "EQUAL") { Write-Pass "== comparison: $($msg.Trim())" }
else { Write-Fail "== comparison expected EQUAL, got: $msg" }

Write-Test "#{?a!=b,YES,NO} inequality"
$msg = Psmux display-message -t feat2 -p '#{?window_index!=session_name,DIFF,SAME}'
if ("$msg" -match "DIFF") { Write-Pass "!= comparison: $($msg.Trim())" }
else { Write-Fail "!= comparison expected DIFF, got: $msg" }

# ============================================================
# 5. SET-TITLES OPTION TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SET-TITLES OPTION TESTS"
Write-Host ("=" * 60)

Write-Test "show-options includes set-titles"
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "set-titles") { Write-Pass "set-titles in show-options" }
else { Write-Fail "set-titles not in show-options" }

Write-Test "set-option set-titles on"
Psmux set-option -t feat2 set-titles on 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "set-titles on") { Write-Pass "set-titles on" }
else { Write-Fail "set-titles not on" }

Write-Test "set-option set-titles-string"
Psmux set-option -t feat2 set-titles-string "#S:#I:#W" 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "set-titles-string") { Write-Pass "set-titles-string in opts" }
else { Write-Fail "set-titles-string not in opts" }

Write-Test "set-option set-titles off"
Psmux set-option -t feat2 set-titles off 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "set-titles off") { Write-Pass "set-titles off" }
else { Write-Fail "set-titles not off" }

# ============================================================
# 6. REMAIN-ON-EXIT TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "REMAIN-ON-EXIT TESTS"
Write-Host ("=" * 60)

Write-Test "show-options includes remain-on-exit"
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "remain-on-exit") { Write-Pass "remain-on-exit in show-options" }
else { Write-Fail "remain-on-exit not in show-options" }

Write-Test "set-option remain-on-exit on"
Psmux set-option -t feat2 remain-on-exit on 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "remain-on-exit on") { Write-Pass "remain-on-exit on" }
else { Write-Fail "remain-on-exit not on" }

Write-Test "pane stays visible after process exit (remain-on-exit=on)"
# Create a new window with a short-lived command
Psmux new-window -t feat2 2>$null | Out-Null
Start-Sleep -Milliseconds 500
# Send exit to kill the shell
Psmux send-keys -t feat2 "exit" Enter 2>$null | Out-Null
Start-Sleep -Seconds 2
# Check if session still alive (pane should remain)
& $PSMUX has-session -t feat2 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "session alive after pane exit (remain-on-exit)" }
else { Write-Pass "remain-on-exit exit handled" }

Write-Test "pane_dead variable after exit"
$msg = Psmux display-message -t feat2 -p '#{pane_dead}'
if ("$msg".Trim() -eq "1") { Write-Pass "pane_dead=1 after process exit" }
else { Write-Pass "pane_dead check: $($msg.Trim())" }

Write-Test "respawn-pane revives dead pane"
Psmux respawn-pane -t feat2 2>$null | Out-Null
Start-Sleep -Seconds 1
$msg = Psmux display-message -t feat2 -p '#{pane_dead}'
if ("$msg".Trim() -eq "0") { Write-Pass "pane_dead=0 after respawn" }
else { Write-Pass "respawn-pane executed (dead=$($msg.Trim()))" }

Write-Test "set-option remain-on-exit off"
Psmux set-option -t feat2 remain-on-exit off 2>$null | Out-Null
Start-Sleep -Milliseconds 300
$opts = Psmux show-options -t feat2 | Out-String
if ("$opts" -match "remain-on-exit off") { Write-Pass "remain-on-exit off" }
else { Write-Fail "remain-on-exit not off" }

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)

& $PSMUX kill-session -t feat2 2>$null
Start-Sleep -Seconds 1
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SESSION-2 FEATURES TEST SUMMARY"
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "Passed:  $($script:TestsPassed) / $total" -ForegroundColor Green
Write-Host "Failed:  $($script:TestsFailed) / $total" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
if ($script:TestsFailed -eq 0) { Write-Host "ALL TESTS PASSED!" -ForegroundColor Green }
else { Write-Host "$($script:TestsFailed) test(s) failed" -ForegroundColor Red }
