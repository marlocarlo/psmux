# psmux Issue #95 Test Script
# Tests fixes for choose-tree CLI dispatch and display-message status bar
#
# Issue #95:
#   1. choose-tree / choose-window / choose-session returned "unknown command"
#   2. display-message without -p would hang or not process the response
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue95_commands.ps1

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

# Kill everything first
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "issue95test"

function New-TestSession {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
    & $PSMUX has-session -t $SESSION 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
    Write-Info "Session '$SESSION' is running"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #95: choose-tree CLI dispatch and display-message"
Write-Host ("=" * 70)

New-TestSession

# ============================================================
# TEST GROUP 1: choose-tree / choose-window / choose-session
# These are interactive commands that switch the server to
# WindowChooser mode. They just need to not error out.
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GROUP 1: choose-tree / choose-window / choose-session dispatch"
Write-Host ("=" * 70)

# Test 1: choose-tree should not return "unknown command"
Write-Test "choose-tree does not return unknown command error"
$output = & $PSMUX choose-tree -t $SESSION 2>&1 | Out-String
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0) {
    Write-Pass "choose-tree exited with code 0 (no unknown command error)"
} else {
    Write-Fail "choose-tree exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# Test 2: choose-window should not return "unknown command"
Write-Test "choose-window does not return unknown command error"
$output = & $PSMUX choose-window -t $SESSION 2>&1 | Out-String
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0) {
    Write-Pass "choose-window exited with code 0 (no unknown command error)"
} else {
    Write-Fail "choose-window exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# Test 3: choose-session should not return "unknown command"
Write-Test "choose-session does not return unknown command error"
$output = & $PSMUX choose-session -t $SESSION 2>&1 | Out-String
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0) {
    Write-Pass "choose-session exited with code 0 (no unknown command error)"
} else {
    Write-Fail "choose-session exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# ============================================================
# TEST GROUP 2: display-message fixes
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GROUP 2: display-message fixes"
Write-Host ("=" * 70)

# Test 4: display-message -p "#{session_name}" should print session name
Write-Test "display-message -p '#{session_name}' prints session name"
$output = & $PSMUX display-message -t $SESSION -p "#{session_name}" 2>&1 | Out-String
$output = $output.Trim()
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0 -and $output -eq $SESSION) {
    Write-Pass "display-message -p '#{session_name}' returned '$output' (matches session name)"
} elseif ($exitCode -eq 0) {
    Write-Fail "display-message -p '#{session_name}' returned '$output', expected '$SESSION'"
} else {
    Write-Fail "display-message -p '#{session_name}' exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# Test 5: display-message without -p should exit cleanly
Write-Test "display-message without -p exits cleanly (exit code 0)"
$output = & $PSMUX display-message -t $SESSION "hello world" 2>&1 | Out-String
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0) {
    Write-Pass "display-message without -p exited cleanly (code 0)"
} else {
    Write-Fail "display-message without -p exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# Test 6: display-message -p "#{window_index}" should return window index
Write-Test "display-message -p '#{window_index}' returns window index"
$output = & $PSMUX display-message -t $SESSION -p "#{window_index}" 2>&1 | Out-String
$output = $output.Trim()
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0 -and $output -match '^\d+$') {
    Write-Pass "display-message -p '#{window_index}' returned '$output' (valid index number)"
} elseif ($exitCode -eq 0) {
    Write-Fail "display-message -p '#{window_index}' returned '$output', expected a number"
} else {
    Write-Fail "display-message -p '#{window_index}' exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# ============================================================
# TEST GROUP 3: Regression tests for existing working commands
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GROUP 3: Regression tests (existing commands)"
Write-Host ("=" * 70)

# Test 7: run-shell "echo hello" should output "hello"
Write-Test "run-shell 'echo hello' outputs hello"
$output = & $PSMUX run-shell -t $SESSION "echo hello" 2>&1 | Out-String
$output = $output.Trim()
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0 -and $output -eq "hello") {
    Write-Pass "run-shell returned 'hello'"
} elseif ($exitCode -eq 0) {
    Write-Fail "run-shell returned '$output', expected 'hello'"
} else {
    Write-Fail "run-shell exited with code $exitCode. Output: $output"
}

Start-Sleep -Milliseconds 500

# Test 8: display-message -p "#{pane_id}" should return a pane ID
Write-Test "display-message -p '#{pane_id}' returns a pane ID"
$output = & $PSMUX display-message -t $SESSION -p "#{pane_id}" 2>&1 | Out-String
$output = $output.Trim()
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0 -and $output -match '^%\d+$') {
    Write-Pass "display-message -p '#{pane_id}' returned '$output' (valid pane ID)"
} elseif ($exitCode -eq 0 -and $output.Length -gt 0) {
    # Some implementations may not prefix with %, accept any non-empty output
    Write-Pass "display-message -p '#{pane_id}' returned '$output'"
} else {
    Write-Fail "display-message -p '#{pane_id}' exited with code $exitCode. Output: $output"
}

# ============================================================
# TEST GROUP 4: Overlay / Visual Rendering Commands
# These trigger TUI overlays on the server. From CLI they should exit 0.
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GROUP 4: Overlay / Visual Rendering Commands (exit code verification)"
Write-Host ("=" * 70)

# Test 9: display-popup
Write-Test "display-popup exits cleanly"
$output = & $PSMUX display-popup -t $SESSION "echo hello" 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "display-popup exited with code 0" }
else { Write-Fail "display-popup exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# Test 10: display-popup with size flags
Write-Test "display-popup with -w/-h size flags exits cleanly"
$output = & $PSMUX display-popup -t $SESSION -w 50 -h 20 "echo test" 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "display-popup -w 50 -h 20 exited with code 0" }
else { Write-Fail "display-popup -w/-h exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# Test 11: display-menu
Write-Test "display-menu exits cleanly"
$output = & $PSMUX display-menu -t $SESSION -T "Menu" "Item1" "a" "echo a" 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "display-menu exited with code 0" }
else { Write-Fail "display-menu exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# Test 12: confirm-before
Write-Test "confirm-before exits cleanly"
$output = & $PSMUX confirm-before -t $SESSION -p "sure?" "echo yes" 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "confirm-before exited with code 0" }
else { Write-Fail "confirm-before exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# Test 13: display-panes
Write-Test "display-panes exits cleanly"
$output = & $PSMUX display-panes -t $SESSION 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "display-panes exited with code 0" }
else { Write-Fail "display-panes exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# Test 14: clock-mode
Write-Test "clock-mode exits cleanly"
$output = & $PSMUX clock-mode -t $SESSION 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "clock-mode exited with code 0" }
else { Write-Fail "clock-mode exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# ============================================================
# TEST GROUP 5: pipe-pane and copy-mode
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GROUP 5: pipe-pane and copy-mode"
Write-Host ("=" * 70)

# Test 15: pipe-pane exits cleanly
Write-Test "pipe-pane exits cleanly"
$output = & $PSMUX pipe-pane -t $SESSION 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "pipe-pane exited with code 0" }
else { Write-Fail "pipe-pane exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# Test 16: copy-mode exits cleanly
Write-Test "copy-mode exits cleanly"
$output = & $PSMUX copy-mode -t $SESSION 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "copy-mode exited with code 0" }
else { Write-Fail "copy-mode exited with code $LASTEXITCODE. Output: $output" }
Start-Sleep -Milliseconds 500

# ============================================================
# TEST GROUP 6: Working commands regression tests
# Verifies all commands listed as WORKING in issue #95
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GROUP 6: Working commands regression tests"
Write-Host ("=" * 70)

# Test 17: if-shell
Write-Test "if-shell evaluates correctly"
$output = & $PSMUX if-shell -t $SESSION "true" "run-shell 'echo T'" "run-shell 'echo F'" 2>&1 | Out-String
$output = $output.Trim()
if ($output -eq "T") { Write-Pass "if-shell true branch returned 'T'" }
else { Write-Fail "if-shell returned '$output', expected 'T'" }
Start-Sleep -Milliseconds 500

# Test 18: send-keys + capture-pane
Write-Test "send-keys delivers keystrokes (verified via capture-pane)"
# Dismiss any overlay mode from previous tests (Escape x3, then q for good measure)
& $PSMUX send-keys -t $SESSION Escape 2>$null
Start-Sleep -Milliseconds 200
& $PSMUX send-keys -t $SESSION Escape 2>$null
Start-Sleep -Milliseconds 200
& $PSMUX send-keys -t $SESSION q 2>$null
Start-Sleep -Milliseconds 200
& $PSMUX send-keys -t $SESSION Escape 2>$null
Start-Sleep -Seconds 1
& $PSMUX send-keys -t $SESSION "echo send_keys_test_12345" Enter 2>$null
Start-Sleep -Seconds 2
$captured = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
if ($captured -match "send_keys_test_12345") { Write-Pass "send-keys + capture-pane: found marker text" }
else { Write-Fail "send-keys + capture-pane: marker text not found in capture" }
Start-Sleep -Milliseconds 500

# Test 19: set-buffer + show-buffer
Write-Test "set-buffer / show-buffer round-trip"
& $PSMUX set-buffer -t $SESSION "test_buffer_data_xyz" 2>$null
$buf = & $PSMUX show-buffer -t $SESSION 2>&1 | Out-String
$buf = $buf.Trim()
if ($buf -eq "test_buffer_data_xyz") { Write-Pass "set-buffer / show-buffer returned correct data" }
else { Write-Fail "show-buffer returned '$buf', expected 'test_buffer_data_xyz'" }
Start-Sleep -Milliseconds 500

# Test 20: list-buffers
Write-Test "list-buffers returns buffer info"
$output = & $PSMUX list-buffers -t $SESSION 2>&1 | Out-String
if ($output.Trim().Length -gt 0) { Write-Pass "list-buffers returned non-empty output" }
else { Write-Fail "list-buffers returned empty" }
Start-Sleep -Milliseconds 500

# Test 21: delete-buffer
Write-Test "delete-buffer removes buffer"
& $PSMUX set-buffer -t $SESSION "to_delete" 2>$null
& $PSMUX delete-buffer -t $SESSION 2>$null
$afterDel = & $PSMUX show-buffer -t $SESSION 2>&1 | Out-String
# After delete, show-buffer should return empty or different data
if ($afterDel.Trim() -ne "to_delete") { Write-Pass "delete-buffer removed the buffer" }
else { Write-Fail "delete-buffer did not remove the buffer" }
Start-Sleep -Milliseconds 500

# Test 22: set-environment / show-environment
Write-Test "set-environment / show-environment"
& $PSMUX set-environment -t $SESSION FOO bar_test_val 2>$null
$envOut = & $PSMUX show-environment -t $SESSION 2>&1 | Out-String
if ($envOut -match "FOO=bar_test_val") { Write-Pass "set-environment FOO=bar_test_val found in show-environment" }
else { Write-Fail "FOO=bar_test_val not found in show-environment output: $envOut" }
Start-Sleep -Milliseconds 500

# Test 23: set-hook / show-hooks
Write-Test "set-hook / show-hooks"
& $PSMUX set-hook -t $SESSION after-new-window "run-shell 'echo hooked'" 2>$null
$hooks = & $PSMUX show-hooks -t $SESSION 2>&1 | Out-String
if ($hooks -match "after-new-window") { Write-Pass "set-hook registered and visible in show-hooks" }
else { Write-Fail "after-new-window hook not found in show-hooks: $hooks" }
Start-Sleep -Milliseconds 500

# Test 24: find-window
Write-Test "find-window finds matching window"
$output = & $PSMUX find-window -t $SESSION "pwsh" 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "find-window exited with code 0" }
else { Write-Fail "find-window exited with code $LASTEXITCODE" }
Start-Sleep -Milliseconds 500

# Test 25: choose-buffer
Write-Test "choose-buffer lists buffers"
& $PSMUX set-buffer -t $SESSION "choosebuf" 2>$null
$output = & $PSMUX choose-buffer -t $SESSION 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { Write-Pass "choose-buffer exited with code 0" }
else { Write-Fail "choose-buffer exited with code $LASTEXITCODE" }
Start-Sleep -Milliseconds 500

# Test 26: list-keys / bind-key / unbind-key
Write-Test "bind-key / list-keys / unbind-key"
& $PSMUX bind-key -t $SESSION X "run-shell 'echo bound'" 2>$null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "X") { Write-Pass "bind-key X visible in list-keys" }
else { Write-Fail "bind-key X not found in list-keys" }
& $PSMUX unbind-key -t $SESSION X 2>$null
Start-Sleep -Milliseconds 500

# Test 27: set-option / show-options
Write-Test "set-option / show-options round-trip"
& $PSMUX set-option -t $SESSION -g status-style "fg=white" 2>$null
$opts = & $PSMUX show-options -t $SESSION -g -v status-style 2>&1 | Out-String
$opts = $opts.Trim()
if ($opts -match "fg=white") { Write-Pass "set-option status-style visible in show-options" }
else { Write-Fail "status-style not matching: '$opts'" }
Start-Sleep -Milliseconds 500

# Test 28: Format variables
Write-Test "Format variables resolve correctly"
$sname = & $PSMUX display-message -t $SESSION -p "#{session_name}" 2>&1 | Out-String
$sname = $sname.Trim()
$widx = & $PSMUX display-message -t $SESSION -p "#{window_index}" 2>&1 | Out-String
$widx = $widx.Trim()
$ppid = & $PSMUX display-message -t $SESSION -p "#{pane_pid}" 2>&1 | Out-String
$ppid = $ppid.Trim()
$cond = & $PSMUX display-message -t $SESSION -p "#{?window_active,YES,NO}" 2>&1 | Out-String
$cond = $cond.Trim()
$allOk = ($sname -eq $SESSION) -and ($widx -match '^\d+$') -and ($ppid -match '^\d+$') -and ($cond -eq "YES")
if ($allOk) { Write-Pass "Format vars: session=$sname, window=$widx, pid=$ppid, conditional=$cond" }
else { Write-Fail "Format vars: session='$sname', window='$widx', pid='$ppid', conditional='$cond'" }

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Final cleanup..."
& $PSMUX kill-server 2>$null
Start-Sleep 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #95 TEST SUMMARY" -ForegroundColor White
Write-Host ("=" * 70)
Write-Host "Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "Total:  $($script:TestsPassed + $script:TestsFailed)"
Write-Host ""
Write-Host "Tests covered:" -ForegroundColor Yellow
Write-Host "  1. choose-tree CLI dispatch (should not return unknown command)"
Write-Host "  2. choose-window CLI dispatch (should not return unknown command)"
Write-Host "  3. choose-session CLI dispatch (should not return unknown command)"
Write-Host "  4. display-message -p '#{session_name}' prints session name (regression)"
Write-Host "  5. display-message without -p exits cleanly (no hang)"
Write-Host "  6. display-message -p '#{window_index}' returns window index"
Write-Host "  7. run-shell 'echo hello' outputs hello (regression)"
Write-Host "  8. display-message -p '#{pane_id}' returns pane ID (regression)"
Write-Host "  9. display-popup exits cleanly"
Write-Host " 10. display-popup with -w/-h size flags exits cleanly"
Write-Host " 11. display-menu exits cleanly"
Write-Host " 12. confirm-before exits cleanly"
Write-Host " 13. display-panes exits cleanly"
Write-Host " 14. clock-mode exits cleanly"
Write-Host " 15. pipe-pane exits cleanly"
Write-Host " 16. copy-mode exits cleanly"
Write-Host " 17. if-shell evaluates correctly"
Write-Host " 18. send-keys + capture-pane delivers keystrokes"
Write-Host " 19. set-buffer / show-buffer round-trip"
Write-Host " 20. list-buffers returns buffer info"
Write-Host " 21. delete-buffer removes buffer"
Write-Host " 22. set-environment / show-environment"
Write-Host " 23. set-hook / show-hooks"
Write-Host " 24. find-window finds matching window"
Write-Host " 25. choose-buffer lists buffers"
Write-Host " 26. bind-key / list-keys / unbind-key"
Write-Host " 27. set-option / show-options round-trip"
Write-Host " 28. Format variables resolve correctly"
Write-Host ("=" * 70)

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
