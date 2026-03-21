#!/usr/bin/env pwsh
# test_issue33_remaining.ps1
# Tests for the 4 remaining issues from GitHub Issue #33 comment:
# https://github.com/psmux/psmux/issues/33#issuecomment-3912890080
#
# Issue 1: -L <socket> flag not supported (High)
# Issue 2: new-session -P -F '#{pane_id}' returns empty (Medium)
# Issue 3: new-window -P -F '#{pane_id}' returns empty (Medium)
# Issue 4: Commands without -t default to session "default" (Medium)

$ErrorActionPreference = "Continue"
$exe = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $exe)) { $exe = "$PSScriptRoot\..\target\debug\psmux.exe" }
if (-not (Test-Path $exe)) { $exe = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $exe -or -not (Test-Path $exe)) { Write-Error "psmux binary not found"; exit 1 }

# Helper: cleanup sessions
function Cleanup-Sessions {
    & $exe kill-session -t test-issue33 2>$null
    & $exe -L test-L-socket kill-session -t default 2>$null
    & $exe -L test-L-socket kill-session -t nstest 2>$null
    & $exe -L test-L-socket kill-session -t test-L-socket 2>$null
    & $exe kill-session -t test-pf 2>$null
    & $exe kill-session -t test-newwin 2>$null
    & $exe kill-session -t test-implicit 2>$null
    Start-Sleep -Milliseconds 500
}

$pass = 0
$fail = 0
$total = 0

function Test-Assert {
    param(
        [string]$Name,
        [bool]$Condition,
        [string]$Detail = ""
    )
    $script:total++
    if ($Condition) {
        $script:pass++
        Write-Host "  PASS: $Name" -ForegroundColor Green
    } else {
        $script:fail++
        Write-Host "  FAIL: $Name" -ForegroundColor Red
        if ($Detail) { Write-Host "        Detail: $Detail" -ForegroundColor Yellow }
    }
}

Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "Issue #33 Remaining Issues Test Suite" -ForegroundColor Cyan
Write-Host "========================================`n" -ForegroundColor Cyan

# --- Cleanup before tests ---
Cleanup-Sessions

# ============================================================
# TEST GROUP 1: -L <socket> flag support
# ============================================================
Write-Host "[Test Group 1] -L <socket> flag support" -ForegroundColor Magenta
Write-Host "  tmux uses -L to name the server socket. psmux uses -L as namespace prefix." -ForegroundColor Gray

# Test 1.1: new-session with -L should not return "unknown command"
$output = & $exe new-session -d -L test-L-socket -s test-L-socket 2>&1
$exitCode = $LASTEXITCODE
$errorStr = ($output | Out-String)
$hasUnknown = $errorStr -match "unknown"
Test-Assert "new-session with -L flag does not error" (-not $hasUnknown) "Output: $errorStr"

# Cleanup
& $exe -L test-L-socket kill-session -t test-L-socket 2>$null
Start-Sleep -Milliseconds 300

# Test 1.2: -L as a namespace with explicit session name
# In tmux: tmux -L mysocket new-session -d creates a server named "mysocket"
# For psmux, -L creates a namespace: port file = "test-L-socket__nstest.port"
# NOTE: We use -s to give an explicit name because psmux auto-numbers sessions
# (0, 1, 2, ...) when no -s is given, matching tmux behavior.
$output2 = & $exe -L test-L-socket new-session -d -s nstest 2>&1
$exitCode2 = $LASTEXITCODE
$errorStr2 = ($output2 | Out-String)
$hasUnknown2 = $errorStr2 -match "unknown"
Test-Assert "-L <name> new-session -d works (creates namespaced session)" (-not $hasUnknown2) "Output: $errorStr2"

# Verify session exists using -L namespace
Start-Sleep -Milliseconds 500
& $exe -L test-L-socket has-session -t nstest 2>$null
$hasExit = $LASTEXITCODE
Test-Assert "-L created session findable via -L has-session" ($hasExit -eq 0) "Exit code: $hasExit"

# Cleanup
& $exe -L test-L-socket kill-session -t nstest 2>$null
Start-Sleep -Milliseconds 300

# ============================================================
# TEST GROUP 2: new-session -P -F '#{pane_id}' returns pane ID
# ============================================================
Write-Host "`n[Test Group 2] new-session -P -F '#{pane_id}'" -ForegroundColor Magenta

# Test 2.1: new-session -d -P -F '#{pane_id}' should print pane ID
$paneId = & $exe new-session -d -s test-pf -P -F '#{pane_id}' 2>&1
$paneIdStr = ($paneId | Out-String).Trim()
Test-Assert "new-session -P -F '#{pane_id}' returns non-empty" ($paneIdStr.Length -gt 0) "Got: '$paneIdStr'"
Test-Assert "new-session -P -F '#{pane_id}' returns %N format" ($paneIdStr -match '^%\d+$') "Got: '$paneIdStr'"

# Test 2.2: new-session -d -P (no -F) should print "session_name:" (tmux default)
& $exe kill-session -t test-pf2 2>$null
Start-Sleep -Milliseconds 300
$defaultInfo = & $exe new-session -d -s test-pf2 -P 2>&1
$defaultInfoStr = ($defaultInfo | Out-String).Trim()
Test-Assert "new-session -P (no -F) returns session info" ($defaultInfoStr.Length -gt 0) "Got: '$defaultInfoStr'"
Test-Assert "new-session -P default format is 'session:'" ($defaultInfoStr -eq "test-pf2:") "Got: '$defaultInfoStr'"

# Cleanup
& $exe kill-session -t test-pf 2>$null
& $exe kill-session -t test-pf2 2>$null
Start-Sleep -Milliseconds 300

# ============================================================
# TEST GROUP 3: new-window -P -F '#{pane_id}' returns pane ID
# ============================================================
Write-Host "`n[Test Group 3] new-window -P -F '#{pane_id}'" -ForegroundColor Magenta

# Create a session first
& $exe new-session -d -s test-newwin 2>$null
Start-Sleep -Milliseconds 500

# Test 3.1: new-window -P -F '#{pane_id}' should print pane ID  
$newWinPaneId = & $exe new-window -t test-newwin -P -F '#{pane_id}' 2>&1
$newWinPaneIdStr = ($newWinPaneId | Out-String).Trim()
Test-Assert "new-window -P -F '#{pane_id}' returns non-empty" ($newWinPaneIdStr.Length -gt 0) "Got: '$newWinPaneIdStr'"
Test-Assert "new-window -P -F '#{pane_id}' returns %N format" ($newWinPaneIdStr -match '^%\d+$') "Got: '$newWinPaneIdStr'"

# Test 3.2: new-window -P (no -F) should print session:window format (tmux default)
$newWinDefault = & $exe new-window -t test-newwin -P 2>&1
$newWinDefaultStr = ($newWinDefault | Out-String).Trim()
Test-Assert "new-window -P (no -F) returns session info" ($newWinDefaultStr.Length -gt 0) "Got: '$newWinDefaultStr'"
Test-Assert "new-window -P default format is 'session:window'" ($newWinDefaultStr -match '^test-newwin:\d+$') "Got: '$newWinDefaultStr'"

# Cleanup
& $exe kill-session -t test-newwin 2>$null
Start-Sleep -Milliseconds 300

# ============================================================
# TEST GROUP 4: Commands without -t resolve from TMUX env var
# ============================================================
Write-Host "`n[Test Group 4] Implicit session from TMUX env var" -ForegroundColor Magenta

# Create a specifically named session (not "default")
& $exe new-session -d -s test-implicit 2>$null
Start-Sleep -Milliseconds 500

# Test 4.1: Verify the session exists first
& $exe has-session -t test-implicit 2>$null
Test-Assert "test-implicit session exists" ($LASTEXITCODE -eq 0)

# Test 4.2: display-message without -t, but with TMUX env var pointing to test-implicit
# Read the port file for the session
$homeDir = $env:USERPROFILE
$portFile = "$homeDir\.psmux\test-implicit.port"
if (Test-Path $portFile) {
    $port = (Get-Content $portFile).Trim()
    # Set TMUX env var like a real psmux pane would have
    $serverPid = (Get-Process -Name psmux -ErrorAction SilentlyContinue | Select-Object -First 1).Id
    if (-not $serverPid) { $serverPid = 0 }
    
    # Simulate being inside a psmux session by setting TMUX env var
    $env:TMUX = "/tmp/psmux-$serverPid/default,$port,0"
    $env:PSMUX_TARGET_SESSION = $null
    
    $displayOut = & $exe display-message -p '#{session_name}' 2>&1
    $displayOutStr = ($displayOut | Out-String).Trim()
    Test-Assert "display-message resolves session from TMUX env (no -t)" ($displayOutStr -eq "test-implicit") "Got: '$displayOutStr', Expected: 'test-implicit'"
    
    # Test 4.3: send-keys without -t should resolve from TMUX env
    $sendResult = & $exe send-keys "echo hello" Enter 2>&1
    $sendResultStr = ($sendResult | Out-String).Trim()
    $sendErr = $sendResultStr -match "no session|error"
    Test-Assert "send-keys without -t resolves from TMUX env" (-not $sendErr) "Output: '$sendResultStr'"
    
    # Clean up env
    $env:TMUX = $null
} else {
    Write-Host "  SKIP: Could not find port file for test-implicit session" -ForegroundColor Yellow
    $script:total += 2
}

# Cleanup
& $exe kill-session -t test-implicit 2>$null
Start-Sleep -Milliseconds 300

# ============================================================
# SUMMARY
# ============================================================
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "Results: $pass/$total passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "========================================`n" -ForegroundColor Cyan

if ($fail -gt 0) {
    exit 1
} else {
    exit 0
}
