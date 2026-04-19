# psmux bind-key End-to-End Test Suite
# Tests: bind-key server-side storage, binding sync to client via DumpState,
#        list-keys verification, unbind-key, root table bindings, no-session handling
# Run: powershell -ExecutionPolicy Bypass -File tests\test_bind_key.ps1

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
Write-Info "Cleaning up existing sessions..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# ============================================================
# 0. NO-SESSION GRACEFUL HANDLING
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "NO-SESSION GRACEFUL HANDLING"
Write-Host ("=" * 60)

Write-Test "set -g with no session (should warn, not crash)"
$output = & $PSMUX set -g default-shell pwsh 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or "$output" -match "warning.*no active session" -or "$output" -match "no server running") {
    Write-Pass "set -g without session: graceful ($($output.Trim()))"
} else {
    Write-Fail "set -g without session: unexpected error: $output"
}

Write-Test "bind-key with no session (should warn, not crash)"
$output = & $PSMUX bind-key - split-window -v 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or "$output" -match "warning.*no active session" -or "$output" -match "no server running") {
    Write-Pass "bind-key without session: graceful ($($output.Trim()))"
} else {
    Write-Fail "bind-key without session: unexpected error: $output"
}

Write-Test "unbind-key with no session (should warn, not crash)"
$output = & $PSMUX unbind-key x 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or "$output" -match "warning.*no active session" -or "$output" -match "no server running") {
    Write-Pass "unbind-key without session: graceful ($($output.Trim()))"
} else {
    Write-Fail "unbind-key without session: unexpected error: $output"
}

# ============================================================
# Create test session
# ============================================================
Write-Info "Creating test session 'bindtest'..."
New-PsmuxSession -Name "bindtest"
& $PSMUX has-session -t bindtest 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'bindtest' created"

# ============================================================
# 1. BIND-KEY BASIC TESTS (server-side storage)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "BIND-KEY BASIC TESTS"
Write-Host ("=" * 60)

Write-Test "bind-key - split-window -v (key='-')"
Psmux bind-key -t bindtest - split-window -v 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "split-window") {
    Write-Pass "bind-key '-' split-window -v appears in list-keys"
} else {
    Write-Fail "bind-key '-' not found in list-keys. Output: $keys"
}

Write-Test "bind-key _ split-window -h"
Psmux bind-key -t bindtest _ split-window -h 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "split-window -h") {
    Write-Pass "bind-key '_' split-window -h appears in list-keys"
} else {
    Write-Fail "bind-key '_' not found in list-keys. Output: $keys"
}

Write-Test "bind-key z display-message (default prefix table)"
Psmux bind-key -t bindtest z display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "prefix.*z.*display-message") {
    Write-Pass "bind-key z display-message in prefix table"
} else {
    Write-Fail "bind-key z not in prefix table. Output: $keys"
}

# ============================================================
# 2. BIND-KEY WITH TABLE SPECIFIERS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "BIND-KEY TABLE SPECIFIERS"
Write-Host ("=" * 60)

Write-Test "bind-key -T prefix custom binding"
Psmux bind-key -t bindtest -T prefix m display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "prefix.*m.*display-message") {
    Write-Pass "bind-key -T prefix m display-message in list-keys"
} else {
    Write-Fail "bind-key -T prefix m not found. Output: $keys"
}

Write-Test "bind-key -T root (root table binding)"
Psmux bind-key -t bindtest -T root F12 display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "root.*F12") {
    Write-Pass "bind-key -T root F12 in list-keys"
} else {
    Write-Fail "bind-key -T root F12 not found. Output: $keys"
}

Write-Test "bind-key -n creates root binding (shorthand for -T root)"
Psmux bind-key -t bindtest -n F11 display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "root.*F11") {
    Write-Pass "bind-key -n F11 creates root binding"
} else {
    Write-Fail "bind-key -n F11 not in root table. Output: $keys"
}

Write-Test "bind-key -T custom table"
Psmux bind-key -t bindtest -T mytable x display-message 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "mytable") {
    Write-Pass "custom table 'mytable' in list-keys"
} else {
    Write-Fail "custom table not in list-keys. Output: $keys"
}

# ============================================================
# 3. UNBIND-KEY TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "UNBIND-KEY TESTS"
Write-Host ("=" * 60)

Write-Test "unbind-key z"
Psmux unbind-key -t bindtest z 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -notmatch "prefix.*z.*display-message") {
    Write-Pass "unbind z: z removed from list-keys"
} else {
    Write-Fail "unbind z: z still in list-keys"
}

Write-Test "unbind-key -T root F12"
Psmux unbind-key -t bindtest -T root F12 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -notmatch "root.*F12") {
    Write-Pass "unbind F12: removed from list-keys"
} else {
    Write-Fail "unbind F12: still in list-keys"
}

# ============================================================
# 4. BINDINGS IN LIST-KEYS (verify server-side storage & sync)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "BINDINGS VERIFIED VIA LIST-KEYS"
Write-Host ("=" * 60)

# Ensure there's at least one custom binding
Psmux bind-key -t bindtest -T prefix v split-window -v 2>$null | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "Custom prefix binding appears in list-keys"
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "prefix.*v.*split-window") {
    Write-Pass "custom prefix binding v -> split-window in list-keys"
} else {
    Write-Fail "custom prefix binding v not in list-keys"
}

Write-Test "Custom root binding (F11) persists in list-keys"
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "root.*F11") {
    Write-Pass "root F11 binding persists"
} else {
    Write-Fail "root F11 binding missing"
}

Write-Test "Custom table binding persists in list-keys"
if ("$keys" -match "mytable.*x") {
    Write-Pass "mytable x binding persists"
} else {
    Write-Fail "mytable x binding missing"
}

# ============================================================
# 5. REPEATABLE BINDING (-r flag)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "REPEATABLE BINDING TESTS"
Write-Host ("=" * 60)

Write-Test "bind-key -r (repeatable)"
Psmux bind-key -t bindtest -r -T prefix h resize-pane -L 5 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "prefix.*h.*resize-pane") {
    Write-Pass "repeatable binding h -> resize-pane in list-keys"
} else {
    Write-Fail "repeatable binding not in list-keys"
}

# ============================================================
# 6. REBIND EXISTING KEY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "REBIND EXISTING KEY TESTS"
Write-Host ("=" * 60)

Write-Test "rebind key replaces previous binding"
Psmux bind-key -t bindtest -T prefix v new-window 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
# Find lines with 'prefix' and exactly ' v ' (as whole word) for the key
$vLines = ($keys -split "`n") | Where-Object { $_ -match "-T prefix v " }
$hasSplit = "$vLines" -match "split-window"
$hasNew = "$vLines" -match "new-window"
if ($hasNew -and -not $hasSplit) {
    Write-Pass "rebind v: old split-window replaced with new-window"
} else {
    Write-Fail "rebind v: expected only new-window. Got: $vLines"
}

# ============================================================
# 7. CONFIG FILE BIND-KEY TEST
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CONFIG FILE BIND-KEY TESTS"
Write-Host ("=" * 60)

$configPath = "$env:USERPROFILE\.psmux.conf"
$hadConfig = Test-Path $configPath
if ($hadConfig) {
    $origConfig = Get-Content $configPath -Raw
}

Write-Test "bind-key from config file"
# Write a temp config with a custom binding
@"
bind-key -T prefix g display-message
bind-key -n F10 new-window
"@ | Set-Content $configPath -Force

# Kill server (not just session) so config is re-loaded on fresh start
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
New-PsmuxSession -Name "bindtest"
Start-Sleep -Seconds 3

$keys = Psmux list-keys -t bindtest | Out-String
if ("$keys" -match "prefix.*g.*display-message") {
    Write-Pass "config file bind-key: prefix g display-message loaded"
} else {
    Write-Fail "config file bind-key: prefix g not in list-keys"
}

if ("$keys" -match "root.*F10.*new-window") {
    Write-Pass "config file bind-key -n: root F10 new-window loaded"
} else {
    Write-Fail "config file bind-key -n: root F10 not in list-keys"
}

# Restore original config
if ($hadConfig) {
    $origConfig | Set-Content $configPath -Force
} else {
    Remove-Item $configPath -Force -ErrorAction SilentlyContinue
}

# ============================================================
# 8. MULTIPLE BINDINGS AT ONCE
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "MULTIPLE BINDINGS"
Write-Host ("=" * 60)

Write-Test "multiple bind-key commands"
Psmux bind-key -t bindtest -T prefix a new-window 2>$null | Out-Null
Psmux bind-key -t bindtest -T prefix b split-window -h 2>$null | Out-Null
Psmux bind-key -t bindtest -T prefix e kill-pane 2>$null | Out-Null
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t bindtest | Out-String
$hasA = "$keys" -match "prefix.*a.*new-window"
$hasB = "$keys" -match "prefix.*b.*split-window"
$hasE = "$keys" -match "prefix.*e.*kill-pane"
if ($hasA -and $hasB -and $hasE) {
    Write-Pass "all 3 bindings present in list-keys"
} else {
    Write-Fail "missing bindings: a=$hasA b=$hasB e=$hasE"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Cleaning up..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t bindtest" -WindowStyle Hidden
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST SUMMARY" -ForegroundColor White
Write-Host ("=" * 60)
Write-Host "Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "Total:  $($script:TestsPassed + $script:TestsFailed)"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
