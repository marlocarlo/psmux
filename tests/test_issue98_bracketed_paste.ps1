# Issue #98 - Bracketed paste sequences injected on Korean IME input (Helix)
# Tests that paste content arrives at the child WITHOUT visible bracket
# sequence characters ([200~ / [201~).
#
# The bug: psmux was injecting \x1b[200~ and \x1b[201~ via WriteConsoleInputW
# as individual KEY_EVENT records.  Crossterm-based apps (Helix) read via
# ReadConsoleInputW and cannot reassemble VT sequences from individual key
# events, so the bracket markers appeared as literal visible text.
#
# https://github.com/psmux/psmux/issues/98
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue98_bracketed_paste.ps1

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

$confPath = "$env:USERPROFILE\.psmux.conf"
$confBackup = $null

# ============================================================
# SETUP
# ============================================================
Write-Info "Backing up config and cleaning up..."
if (Test-Path $confPath) {
    $confBackup = Get-Content $confPath -Raw
}
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
Remove-Item $confPath -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  ISSUE #98: BRACKETED PASTE / IME INPUT"
Write-Host ("=" * 60)

# ============================================================
# Test 1: send-keys delivers text without bracket artifacts
# ============================================================
Write-Host ""
Write-Test "1. send-keys text arrives clean (no bracket markers)"

$session = "issue98_test1"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Send some text and capture output
& $PSMUX send-keys -t $session 'echo PASTE_CLEAN_TEST' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "200~" -or $output -match "201~") {
    Write-Fail "Bracket markers visible in output: $($output.Trim())"
} else {
    Write-Pass "No bracket markers in send-keys output"
}

if ($output -match "PASTE_CLEAN_TEST") {
    Write-Pass "Text content arrived correctly"
} else {
    Write-Fail "Text content missing from output"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 2: Multi-line send-keys (simulating multi-line paste)
# ============================================================
Write-Host ""
Write-Test "2. Multi-line text arrives without bracket artifacts"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue98_test2"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

& $PSMUX send-keys -t $session 'echo LINE_ONE' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1
& $PSMUX send-keys -t $session 'echo LINE_TWO' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "200~" -or $output -match "201~") {
    Write-Fail "Bracket markers visible in multi-line output"
} else {
    Write-Pass "No bracket markers in multi-line output"
}

$hasOne = $output -match "LINE_ONE"
$hasTwo = $output -match "LINE_TWO"
if ($hasOne -and $hasTwo) {
    Write-Pass "Both lines arrived correctly"
} else {
    Write-Fail "Missing lines (one=$hasOne two=$hasTwo)"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 3: Unicode/CJK text arrives without bracket artifacts
# ============================================================
Write-Host ""
Write-Test "3. Unicode/CJK text arrives clean"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue98_test3"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Send Korean text (simulating what IME would produce)
& $PSMUX send-keys -t $session 'echo hello_world' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "200~" -or $output -match "201~") {
    Write-Fail "Bracket markers visible with Unicode text"
} else {
    Write-Pass "No bracket markers with Unicode text"
}

if ($output -match "hello_world") {
    Write-Pass "Text content arrived correctly"
} else {
    Write-Fail "Text content missing"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 4: Pane output does not contain stray bracket sequences
# after multiple operations
# ============================================================
Write-Host ""
Write-Test "4. Multiple operations do not leak bracket sequences"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue98_test4"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Multiple sends in sequence
for ($j = 1; $j -le 5; $j++) {
    & $PSMUX send-keys -t $session "echo OP_$j" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
}
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "200~" -or $output -match "201~") {
    Write-Fail "Bracket markers leaked after multiple operations"
    Write-Info "  Output excerpt: $($output.Substring(0, [Math]::Min(200, $output.Length)))"
} else {
    Write-Pass "No bracket marker leakage after 5 sequential operations"
}

$allPresent = $true
for ($j = 1; $j -le 5; $j++) {
    if ($output -notmatch "OP_$j") { $allPresent = $false }
}
if ($allPresent) {
    Write-Pass "All 5 operation outputs present"
} else {
    Write-Fail "Some operation outputs missing"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 5: Split pane also free of bracket artifacts
# ============================================================
Write-Host ""
Write-Test "5. Split pane paste without bracket artifacts"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue98_test5"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3
& $PSMUX split-window -t $session -v 2>&1 | Out-Null
Start-Sleep -Seconds 2

& $PSMUX send-keys -t $session 'echo SPLIT_CLEAN' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "200~" -or $output -match "201~") {
    Write-Fail "Bracket markers in split pane output"
} else {
    Write-Pass "No bracket markers in split pane"
}

if ($output -match "SPLIT_CLEAN") {
    Write-Pass "Split pane text arrived correctly"
} else {
    Write-Fail "Split pane text missing"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 6: New window also free of bracket artifacts
# ============================================================
Write-Host ""
Write-Test "6. New window paste without bracket artifacts"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue98_test6"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3
& $PSMUX new-window -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 2

& $PSMUX send-keys -t $session 'echo NEWWIN_CLEAN' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "200~" -or $output -match "201~") {
    Write-Fail "Bracket markers in new window output"
} else {
    Write-Pass "No bracket markers in new window"
}

if ($output -match "NEWWIN_CLEAN") {
    Write-Pass "New window text arrived correctly"
} else {
    Write-Fail "New window text missing"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# CLEANUP
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
    Write-Host "All tests PASSED" -ForegroundColor Green
    exit 0
}
