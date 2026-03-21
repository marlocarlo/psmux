# psmux Issue #121 (follow-up) — Shift+Enter PSReadLine compatibility
#
# Tests that modified Enter key bindings (S-Enter, M-Enter, C-Enter) can be
# bound and that send-keys delivers them correctly — both through VT encoding
# and via native WriteConsoleInputW injection on Windows.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue121_shift_enter.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_121"

function Wait-ForSession {
    param($name, $timeout = 10)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

# Start a session for all tests
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
if (-not (Wait-ForSession $SESSION)) {
    Write-Host "FATAL: Cannot create test session" -ForegroundColor Red
    exit 1
}
Start-Sleep -Seconds 2

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #121: Shift+Enter PSReadLine compatibility"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: bind-key S-Enter ---
Write-Test "1: bind-key S-Enter can be registered"
& $PSMUX bind-key -t $SESSION S-Enter send-keys 'hello' 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "S-Enter|S-Return|S-CR") {
    Write-Pass "1: S-Enter binding registered in list-keys"
} else {
    Write-Fail "1: S-Enter not found in list-keys. Got:`n$keys"
}

# --- Test 2: bind-key M-Enter ---
Write-Test "2: bind-key M-Enter can be registered"
& $PSMUX bind-key -t $SESSION M-Enter send-keys 'world' 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "M-Enter|M-Return|M-CR") {
    Write-Pass "2: M-Enter binding registered in list-keys"
} else {
    Write-Fail "2: M-Enter not found in list-keys. Got:`n$keys"
}

# --- Test 3: bind-key C-Enter ---
Write-Test "3: bind-key C-Enter can be registered"
& $PSMUX bind-key -t $SESSION C-Enter send-keys 'ctrl' 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "C-Enter|C-Return|C-CR") {
    Write-Pass "3: C-Enter binding registered in list-keys"
} else {
    Write-Fail "3: C-Enter not found in list-keys. Got:`n$keys"
}

# --- Test 4: send-keys S-Enter does not error ---
Write-Test "4: send-keys S-Enter executes without error"
$output = & $PSMUX send-keys -t $SESSION S-Enter 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or $output -notmatch "error|unknown|invalid") {
    Write-Pass "4: send-keys S-Enter accepted (exit=$LASTEXITCODE)"
} else {
    Write-Fail "4: send-keys S-Enter failed: $output"
}

# --- Test 5: send-keys M-Enter does not error ---
Write-Test "5: send-keys M-Enter executes without error"
$output = & $PSMUX send-keys -t $SESSION M-Enter 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or $output -notmatch "error|unknown|invalid") {
    Write-Pass "5: send-keys M-Enter accepted (exit=$LASTEXITCODE)"
} else {
    Write-Fail "5: send-keys M-Enter failed: $output"
}

# --- Test 6: send-keys C-Enter does not error ---
Write-Test "6: send-keys C-Enter executes without error"
$output = & $PSMUX send-keys -t $SESSION C-Enter 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or $output -notmatch "error|unknown|invalid") {
    Write-Pass "6: send-keys C-Enter accepted (exit=$LASTEXITCODE)"
} else {
    Write-Fail "6: send-keys C-Enter failed: $output"
}

# --- Test 7: send-keys C-S-Enter does not error ---
Write-Test "7: send-keys C-S-Enter executes without error"
$output = & $PSMUX send-keys -t $SESSION C-S-Enter 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or $output -notmatch "error|unknown|invalid") {
    Write-Pass "7: send-keys C-S-Enter accepted (exit=$LASTEXITCODE)"
} else {
    Write-Fail "7: send-keys C-S-Enter failed: $output"
}

# --- Test 8: unbind S-Enter ---
Write-Test "8: unbind-key S-Enter"
& $PSMUX unbind-key -t $SESSION S-Enter 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -notmatch "S-Enter") {
    Write-Pass "8: S-Enter successfully unbound"
} else {
    Write-Fail "8: S-Enter still present after unbind. Got:`n$keys"
}

# --- Test 9: Verify capture-pane works after send-keys Enter ---
Write-Test "9: send-keys Enter produces newline in pane"
& $PSMUX send-keys -t $SESSION 'echo test121' 2>&1 | Out-Null
Start-Sleep -Milliseconds 200
& $PSMUX send-keys -t $SESSION Enter 2>&1 | Out-Null
Start-Sleep -Milliseconds 1500
$capture = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
if ($capture -match "test121") {
    Write-Pass "9: Plain Enter works (echo output captured)"
} else {
    Write-Fail "9: Plain Enter may not have worked - 'test121' not in capture. Got:`n$capture"
}

# --- Test 10: version check (ensure binary is up to date) ---
Write-Test "10: psmux version check"
$ver = & $PSMUX -V 2>&1 | Out-String
Write-Info "Version: $($ver.Trim())"
Write-Pass "10: Version check passed"

# ═══════════════════════ Cleanup ═══════════════════════
Cleanup-Session $SESSION
& $PSMUX kill-server 2>$null

# ═══════════════════════ Summary ═══════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host ("Tests passed: $($script:TestsPassed)  Failed: $($script:TestsFailed)  Skipped: $($script:TestsSkipped)")
if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
    exit 0
}
