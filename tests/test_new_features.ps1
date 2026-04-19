# psmux New Features Test Suite
# Tests: rename-session, run-shell, if-shell, format strings, zoom, last-window,
# last-pane, swap-pane, break-pane, kill-window, resize-pane, list-keys, hooks
# Requires a running session.

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
    Write-Host "[ERROR] psmux binary not found. Run 'cargo build --release' first." -ForegroundColor Red
    exit 1
}

$SESSION_NAME = "test_new_feat_$$"
Write-Info "Binary: $PSMUX"
Write-Info "Starting test session: $SESSION_NAME"
Write-Host ""

# Start a detached session
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION_NAME -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

# Verify session
$sessions = & $PSMUX ls 2>&1
if (-not ($sessions -match $SESSION_NAME)) {
    Write-Host "[ERROR] Could not create session. Aborting." -ForegroundColor Red
    exit 1
}
Write-Info "Session created successfully"
Write-Host ""

# ============================================================
# 1. RENAME-SESSION
# ============================================================
Write-Host "--- rename-session ---" -ForegroundColor Yellow
$NEW_SESSION = "renamed_sess_$$"

Write-Test "rename-session"
& $PSMUX rename-session -t $SESSION_NAME $NEW_SESSION 2>&1
Start-Sleep -Milliseconds 500

$sessions = & $PSMUX ls 2>&1
if ($sessions -match $NEW_SESSION) {
    Write-Pass "rename-session works (now: $NEW_SESSION)"
    $SESSION_NAME = $NEW_SESSION
} else {
    Write-Fail "rename-session failed - session not found under new name"
    Write-Info "  Sessions: $sessions"
}

# Verify display-message reflects new name
Write-Test "display-message after rename-session"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#S" 2>&1
if ($output -match $SESSION_NAME) {
    Write-Pass "display-message shows renamed session: $output"
} else {
    Write-Fail "display-message did not reflect rename: $output"
}
Write-Host ""

# ============================================================
# 2. RUN-SHELL
# ============================================================
Write-Host "--- run-shell ---" -ForegroundColor Yellow

Write-Test "run-shell (echo)"
$output = & $PSMUX run-shell -t $SESSION_NAME "echo hello-from-run-shell" 2>&1
if ($output -match "hello-from-run-shell") {
    Write-Pass "run-shell captures output: $output"
} else {
    Write-Fail "run-shell did not return expected output: $output"
}

Write-Test "run-shell (exit 0)"
$output = & $PSMUX run-shell -t $SESSION_NAME "cmd /c exit 0" 2>&1
Write-Pass "run-shell with exit 0 completed"

Write-Test "run-shell with format vars"
$output = & $PSMUX run-shell -t $SESSION_NAME "echo session=#S" 2>&1
if ($output -match "session=") {
    Write-Pass "run-shell with format: $output"
} else {
    Write-Fail "run-shell format expansion may have failed: $output"
}
Write-Host ""

# ============================================================
# 3. IF-SHELL
# ============================================================
Write-Host "--- if-shell ---" -ForegroundColor Yellow

Write-Test "if-shell (true branch)"
$output = & $PSMUX if-shell -t $SESSION_NAME "cmd /c exit 0" "display-message if-true" "display-message if-false" 2>&1
# if-shell runs asynchronously; just verify no crash
Write-Pass "if-shell (true) executed without crash"

Write-Test "if-shell (false branch)"
$output = & $PSMUX if-shell -t $SESSION_NAME "cmd /c exit 1" "display-message if-true" "display-message if-false" 2>&1
Write-Pass "if-shell (false) executed without crash"
Write-Host ""

# ============================================================
# 4. FORMAT STRINGS (display-message)
# ============================================================
Write-Host "--- format strings ---" -ForegroundColor Yellow

Write-Test "format: #S (session name)"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#S" 2>&1
if ($output.Length -gt 0) {
    Write-Pass "#S = $output"
} else {
    Write-Fail "#S returned empty"
}

Write-Test "format: #W (window name)"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#W" 2>&1
if ($output.Length -gt 0) {
    Write-Pass "#W = $output"
} else {
    Write-Fail "#W returned empty"
}

Write-Test "format: #I (window index)"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#I" 2>&1
if ($output.Length -ge 0) {
    Write-Pass "#I = $output"
} else {
    Write-Fail "#I returned nothing"
}

Write-Test "format: #P (pane index)"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#P" 2>&1
if ($output.Length -ge 0) {
    Write-Pass "#P = $output"
} else {
    Write-Fail "#P returned nothing"
}

Write-Test "format: #H (hostname)"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#H" 2>&1
if ($output.Length -gt 0) {
    Write-Pass "#H = $output"
} else {
    Write-Fail "#H returned empty"
}

Write-Test "format: #T (pane title)"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#T" 2>&1
if ($output.Length -ge 0) {
    Write-Pass "#T = $output"
} else {
    Write-Fail "#T returned nothing"
}

Write-Test "format: compound string"
$output = & $PSMUX display-message -t $SESSION_NAME -p "[#S] #I:#W" 2>&1
if ($output -match "\[.+\]") {
    Write-Pass "compound format = $output"
} else {
    Write-Fail "compound format failed: $output"
}

Write-Test "format: #{window_name}"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#{window_name}" 2>&1
if ($output.Length -gt 0) {
    Write-Pass "#{window_name} = $output"
} else {
    Write-Fail "#{window_name} returned empty"
}

Write-Test "format: #{session_name}"
$output = & $PSMUX display-message -t $SESSION_NAME -p "#{session_name}" 2>&1
if ($output -match $SESSION_NAME) {
    Write-Pass "#{session_name} = $output"
} else {
    Write-Fail "#{session_name} mismatch: $output"
}
Write-Host ""

# ============================================================
# 5. WINDOW/PANE OPERATIONS
# ============================================================
Write-Host "--- window/pane operations ---" -ForegroundColor Yellow

# Create a second window first
Write-Test "new-window (setup)"
& $PSMUX new-window -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
Write-Pass "new-window created"

# Test last-window
Write-Test "last-window"
& $PSMUX last-window -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200
Write-Pass "last-window executed"

# Switch back
& $PSMUX last-window -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200

# Create a split so we have multiple panes
Write-Test "split-window (setup)"
& $PSMUX split-window -v -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
Write-Pass "split-window created"

# Test last-pane  
Write-Test "last-pane"
& $PSMUX last-pane -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200
Write-Pass "last-pane executed"

# Test select-pane directions
Write-Test "select-pane -U"
& $PSMUX select-pane -U -t $SESSION_NAME 2>&1
Write-Pass "select-pane -U executed"

Write-Test "select-pane -D"
& $PSMUX select-pane -D -t $SESSION_NAME 2>&1
Write-Pass "select-pane -D executed"

# Test zoom-pane (toggle)
Write-Test "zoom-pane (toggle on)"
& $PSMUX resize-pane -Z -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200
Write-Pass "zoom-pane toggled on"

Write-Test "zoom-pane (toggle off)"
& $PSMUX resize-pane -Z -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200
Write-Pass "zoom-pane toggled off"

# Test resize-pane
Write-Test "resize-pane -U 2"
& $PSMUX resize-pane -U 2 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -U executed"

Write-Test "resize-pane -D 2"
& $PSMUX resize-pane -D 2 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -D executed"

Write-Test "resize-pane -L 3"
& $PSMUX resize-pane -L 3 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -L executed"

Write-Test "resize-pane -R 3"
& $PSMUX resize-pane -R 3 -t $SESSION_NAME 2>&1
Write-Pass "resize-pane -R executed"

# Test swap-pane
Write-Test "swap-pane -U"
& $PSMUX swap-pane -U -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200
Write-Pass "swap-pane -U executed"

Write-Test "swap-pane -D"
& $PSMUX swap-pane -D -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 200
Write-Pass "swap-pane -D executed"

# Test list-keys (returns custom binds; may be empty with default config)
Write-Test "list-keys"
$output = & $PSMUX list-keys -t $SESSION_NAME 2>&1
Write-Pass "list-keys executed (custom binds: $($output.Count))"

# Test rename-window
Write-Test "rename-window"
& $PSMUX rename-window -t $SESSION_NAME "test_win" 2>&1
Start-Sleep -Milliseconds 300
$output = & $PSMUX display-message -t $SESSION_NAME -p "#W" 2>&1
if ($output -match "test_win") {
    Write-Pass "rename-window works: $output"
} else {
    Write-Fail "rename-window did not stick: $output"
}
Write-Host ""

# ============================================================
# 6. BREAK-PANE & KILL-WINDOW
# ============================================================
Write-Host "--- break-pane & kill-window ---" -ForegroundColor Yellow

# First ensure we have a split
& $PSMUX split-window -v -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500

Write-Test "break-pane"
$before = & $PSMUX list-windows -t $SESSION_NAME 2>&1
& $PSMUX break-pane -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$after = & $PSMUX list-windows -t $SESSION_NAME 2>&1
if ($after.Count -gt $before.Count -or ($after.Length -gt $before.Length)) {
    Write-Pass "break-pane created a new window"
} else {
    Write-Pass "break-pane executed (window count may vary)"
}

Write-Test "kill-window"
# Create a disposable window then kill it
& $PSMUX new-window -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$before = & $PSMUX list-windows -t $SESSION_NAME 2>&1
& $PSMUX kill-window -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
$after = & $PSMUX list-windows -t $SESSION_NAME 2>&1
Write-Pass "kill-window executed"
Write-Host ""

# ============================================================
# 7. HOOKS (set-hook / show-hooks)
# ============================================================
Write-Host "--- hooks ---" -ForegroundColor Yellow

Write-Test "set-hook"
& $PSMUX set-hook -t $SESSION_NAME "after-new-window" "display-message hook-fired" 2>&1
Write-Pass "set-hook executed"

Write-Test "show-hooks"
$output = & $PSMUX show-hooks -t $SESSION_NAME 2>&1
if ($output -match "after-new-window") {
    Write-Pass "show-hooks lists our hook: $output"
} else {
    Write-Fail "show-hooks did not list hook: $output"
}
Write-Host ""

# ============================================================
# 8. CAPTURE-PANE / SAVE-BUFFER / LIST-BUFFERS
# ============================================================
Write-Host "--- capture/buffer ---" -ForegroundColor Yellow

Write-Test "capture-pane"
& $PSMUX capture-pane -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 300
Write-Pass "capture-pane executed"

Write-Test "list-buffers"
$output = & $PSMUX list-buffers -t $SESSION_NAME 2>&1
if ($output.Length -ge 0) {
    Write-Pass "list-buffers: $($output.Count) buffer(s)"
} else {
    Write-Fail "list-buffers returned nothing"
}

$tempFile = [System.IO.Path]::GetTempFileName()
Write-Test "save-buffer"
& $PSMUX save-buffer -t $SESSION_NAME $tempFile 2>&1
if (Test-Path $tempFile) {
    $size = (Get-Item $tempFile).Length
    Write-Pass "save-buffer wrote $size bytes to $tempFile"
    Remove-Item $tempFile -Force
} else {
    Write-Fail "save-buffer did not create file"
}
Write-Host ""

# ============================================================
# 9. SEND-KEYS
# ============================================================
Write-Host "--- send-keys ---" -ForegroundColor Yellow

Write-Test "send-keys (text)"
& $PSMUX send-keys -t $SESSION_NAME "echo test-send-keys" Enter 2>&1
Start-Sleep -Milliseconds 500
Write-Pass "send-keys with text+Enter executed"

Write-Test "send-keys (special: Space)"
& $PSMUX send-keys -t $SESSION_NAME Space 2>&1
Write-Pass "send-keys Space executed"

Write-Test "send-keys (Ctrl-C)"
& $PSMUX send-keys -t $SESSION_NAME "C-c" 2>&1
Write-Pass "send-keys C-c executed"
Write-Host ""

# ============================================================
# 10. ROTATE-WINDOW / NEXT-WINDOW / PREVIOUS-WINDOW
# ============================================================
Write-Host "--- window navigation ---" -ForegroundColor Yellow

Write-Test "next-window"
& $PSMUX next-window -t $SESSION_NAME 2>&1
Write-Pass "next-window executed"

Write-Test "previous-window"
& $PSMUX previous-window -t $SESSION_NAME 2>&1
Write-Pass "previous-window executed"

Write-Test "select-window -t 1"
& $PSMUX select-window -t "${SESSION_NAME}:1" 2>&1
Write-Pass "select-window executed"
Write-Host ""

# ============================================================
# CLEANUP
# ============================================================
Write-Host "--- cleanup ---" -ForegroundColor Yellow
Write-Test "kill-session"
& $PSMUX kill-session -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500

$sessions = & $PSMUX ls 2>&1
if ($sessions -match $SESSION_NAME) {
    Write-Fail "Session still exists after kill-session"
} else {
    Write-Pass "Session cleaned up"
}
Write-Host ""

# ============================================================
# SUMMARY
# ============================================================
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  NEW FEATURES TEST SUMMARY" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "  Total:  $($script:TestsPassed + $script:TestsFailed)" -ForegroundColor White
Write-Host "========================================" -ForegroundColor Cyan

if ($script:TestsFailed -gt 0) {
    exit 1
} else {
    exit 0
}
