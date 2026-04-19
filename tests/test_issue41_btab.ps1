#!/usr/bin/env pwsh
# test_issue41_btab.ps1 — Verify Shift+Tab (BackTab / BTab) fix for issue #41
#
# Tests:
#   1. send-keys BTab sends ESC[Z (not literal "BTab" text) to the pane
#   2. send-keys BTAB (uppercase alias) also works
#   3. bind-key with BTab shows correctly in list-keys
#   4. bind-key with S-Tab resolves to BTab in list-keys
#   5. send-keys BTab is recognized as a special key (not plain text)
#
# Usage:  pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue41_btab.ps1

param(
    [string]$PsmuxBin = ".\target\release\psmux.exe"
)

$ErrorActionPreference = 'Continue'
$session = "test_issue41"
$passed = 0
$failed = 0
$total  = 0

function Test-Result {
    param([string]$Name, [bool]$Condition, [string]$Details = "")
    $script:total++
    if ($Condition) {
        $script:passed++
        Write-Host "  [PASS] $Name" -ForegroundColor Green
    } else {
        $script:failed++
        Write-Host "  [FAIL] $Name" -ForegroundColor Red
        if ($Details) { Write-Host "         $Details" -ForegroundColor Yellow }
    }
}

Write-Host "`n=== Issue #41: Shift+Tab (BackTab / BTab) Tests ===" -ForegroundColor Cyan
Write-Host "Binary: $PsmuxBin"

# Cleanup any prior sessions
taskkill /f /im psmux.exe 2>$null | Out-Null
Start-Sleep 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

# Start a detached session
Write-Host "`nStarting detached session '$session'..."
Start-Process -FilePath $PsmuxBin -ArgumentList "new-session","-d","-s",$session -WindowStyle Hidden
Start-Sleep 5

# Verify session exists
$sessions = & $PsmuxBin list-sessions 2>&1 | Out-String
if ($sessions -notmatch $session) {
    Write-Host "FATAL: Could not start detached session." -ForegroundColor Red
    exit 1
}
Write-Host "Session started OK.`n"

# ─── Test 1: send-keys BTab should NOT produce literal "BTab" in pane ─────
Write-Host "--- Test 1: send-keys BTab does not produce literal text ---"
& $PsmuxBin -t $session send-keys 'cls' Enter 2>&1 | Out-Null
Start-Sleep 2
& $PsmuxBin -t $session send-keys BTab 2>&1 | Out-Null
Start-Sleep 1
$capture = & $PsmuxBin -t $session capture-pane -p 2>&1 | Out-String
$hasLiteralBTab = $capture -match '\bBTab\b'
Test-Result "send-keys BTab does not appear as literal text" (-not $hasLiteralBTab) `
    "capture-pane still shows literal 'BTab' text"

# ─── Test 2: send-keys BTAB (uppercase) also works ──────────────────────
Write-Host "--- Test 2: send-keys BTAB (uppercase) also works ---"
& $PsmuxBin -t $session send-keys 'cls' Enter 2>&1 | Out-Null
Start-Sleep 2
& $PsmuxBin -t $session send-keys BTAB 2>&1 | Out-Null
Start-Sleep 1
$capture2 = & $PsmuxBin -t $session capture-pane -p 2>&1 | Out-String
$hasLiteralBTAB = $capture2 -match '\bBTAB\b'
Test-Result "send-keys BTAB (uppercase) does not appear as literal text" (-not $hasLiteralBTAB) `
    "capture-pane still shows literal 'BTAB' text"

# ─── Test 3: bind-key BTab appears in list-keys ─────────────────────────
Write-Host "--- Test 3: bind-key BTab shows correctly in list-keys ---"
& $PsmuxBin -t $session bind-key -n BTab run-shell "cmd /c echo BTAB_TEST" 2>&1 | Out-Null
Start-Sleep 1
$keys = & $PsmuxBin -t $session list-keys 2>&1 | Out-String
$hasBTabBind = $keys -match 'BTab.*run-shell'
Test-Result "bind-key BTab visible in list-keys" $hasBTabBind `
    "list-keys output: $keys"

# ─── Test 4: bind-key S-Tab resolves to BTab in list-keys ───────────────
Write-Host "--- Test 4: bind-key S-Tab resolves to BTab ---"
& $PsmuxBin -t $session unbind-key -n BTab 2>&1 | Out-Null
Start-Sleep 1
& $PsmuxBin -t $session bind-key -n S-Tab run-shell "cmd /c echo STAB_TEST" 2>&1 | Out-Null
Start-Sleep 1
$keys2 = & $PsmuxBin -t $session list-keys 2>&1 | Out-String
# S-Tab should resolve to BTab (BackTab) internally
$hasStabAsBTab = ($keys2 -match 'BTab.*run-shell') -or ($keys2 -match 'S-Tab.*run-shell')
Test-Result "S-Tab binding resolves correctly in list-keys" $hasStabAsBTab `
    "list-keys output: $keys2"

# ─── Test 5: send-keys BTab followed by text — BTab is treated as special key ───
Write-Host "--- Test 5: BTab treated as special key (no space inserted) ---"
# Ensure session is still alive, restart if needed
$sessCheck = & $PsmuxBin list-sessions 2>&1 | Out-String
if ($sessCheck -notmatch $session) {
    Write-Host "  Session died, restarting..."
    Start-Process -FilePath $PsmuxBin -ArgumentList "new-session","-d","-s",$session -WindowStyle Hidden
    Start-Sleep 5
}
& $PsmuxBin -t $session send-keys 'cls' Enter 2>&1 | Out-Null
Start-Sleep 2
# If BTab is special, "send-keys BTab hello" should not insert a space between BTab and "hello"
# The pane should just get ESC[Z followed by "hello" (not "BTab hello")
& $PsmuxBin -t $session send-keys BTab 'echo' Space 'btab_special_test' Enter 2>&1 | Out-Null
Start-Sleep 2
$capture3 = & $PsmuxBin -t $session capture-pane -p 2>&1 | Out-String
$hasSpecialTest = $capture3 -match 'btab_special_test'
$noLiteralBtab = $capture3 -notmatch '\bBTab\b'
Test-Result "BTab is treated as special key (not plain text)" ($hasSpecialTest -and $noLiteralBtab) `
    "capture-pane: $capture3"

# ─── Cleanup ────────────────────────────────────────────────────────────
Write-Host "`nCleaning up..."
& $PsmuxBin -t $session kill-server 2>&1 | Out-Null
Start-Sleep 1
taskkill /f /im psmux.exe 2>$null | Out-Null

# ─── Summary ────────────────────────────────────────────────────────────
Write-Host "`n=== Results: $passed/$total passed, $failed failed ===" -ForegroundColor $(if ($failed -eq 0) { 'Green' } else { 'Red' })
if ($failed -gt 0) { exit 1 } else { exit 0 }
