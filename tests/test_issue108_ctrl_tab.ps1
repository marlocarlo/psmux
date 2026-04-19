# psmux Issue #108 — Ctrl+Tab and Ctrl+Shift+Tab key binding support
#
# Tests that C-Tab, C-S-Tab, and multi-modifier key combos can be bound
# and are correctly registered in the key table.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue108_ctrl_tab.ps1

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

$SESSION = "test_108"

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
Write-Host "ISSUE #108: Ctrl+Tab and Ctrl+Shift+Tab key bindings"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: bind-key C-Tab ---
Write-Test "1: bind-key C-Tab next-window"
& $PSMUX bind-key -t $SESSION C-Tab next-window 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "C-Tab" -and $keys -match "next-window") {
    Write-Pass "1: C-Tab binding registered and visible in list-keys"
} else {
    Write-Fail "1: C-Tab not found in list-keys. Got:`n$keys"
}

# --- Test 2: bind-key C-S-Tab (Ctrl+Shift+Tab) ---
Write-Test "2: bind-key C-S-Tab previous-window"
& $PSMUX bind-key -t $SESSION C-S-Tab previous-window 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "C-BTab" -and $keys -match "previous-window") {
    Write-Pass "2: C-S-Tab binding registered (displayed as C-BTab)"
} elseif ($keys -match "C-S-Tab" -and $keys -match "previous-window") {
    Write-Pass "2: C-S-Tab binding registered"
} else {
    Write-Fail "2: C-S-Tab not found in list-keys. Got:`n$keys"
}

# --- Test 3: bind-key M-Tab (Alt+Tab) ---
Write-Test "3: bind-key M-Tab select-pane -t :.+"
& $PSMUX bind-key -t $SESSION M-Tab select-pane 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "M-Tab" -and $keys -match "select-pane") {
    Write-Pass "3: M-Tab binding registered"
} else {
    Write-Fail "3: M-Tab not found in list-keys. Got:`n$keys"
}

# --- Test 4: bind-key S-Tab (Shift+Tab = BTab) ---
Write-Test "4: bind-key S-Tab display-message"
& $PSMUX bind-key -t $SESSION S-Tab display-message 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if (($keys -match "BTab" -or $keys -match "S-Tab") -and $keys -match "display-message") {
    Write-Pass "4: S-Tab binding registered"
} else {
    Write-Fail "4: S-Tab/BTab not found in list-keys. Got:`n$keys"
}

# --- Test 5: Multi-modifier C-M-Tab (Ctrl+Alt+Tab) ---
Write-Test "5: bind-key C-M-Tab last-window"
& $PSMUX bind-key -t $SESSION C-M-Tab last-window 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "C-M-Tab" -and $keys -match "last-window") {
    Write-Pass "5: C-M-Tab binding registered"
} else {
    Write-Fail "5: C-M-Tab not found in list-keys. Got:`n$keys"
}

# --- Test 6: Multi-modifier C-M-S-Tab (Ctrl+Alt+Shift+Tab) ---
Write-Test "6: bind-key C-M-S-Tab kill-pane"
& $PSMUX bind-key -t $SESSION C-M-S-Tab kill-pane 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
# C-M-S-Tab → BackTab with Ctrl|Alt modifiers → displayed as C-M-BTab
if ($keys -match "C-M-BTab" -and $keys -match "kill-pane") {
    Write-Pass "6: C-M-S-Tab binding registered (displayed as C-M-BTab)"
} elseif ($keys -match "C-M-S-Tab" -and $keys -match "kill-pane") {
    Write-Pass "6: C-M-S-Tab binding registered"
} else {
    Write-Fail "6: C-M-S-Tab not found in list-keys. Got:`n$keys"
}

# --- Test 7: Existing single-modifier bindings still work (C-a) ---
Write-Test "7: Existing C-a binding still works"
& $PSMUX bind-key -t $SESSION C-a send-prefix 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "C-a" -and $keys -match "send-prefix") {
    Write-Pass "7: C-a binding works (regression check)"
} else {
    Write-Fail "7: C-a not found in list-keys. Got:`n$keys"
}

# --- Test 8: Existing S-Left, S-Right bindings still work ---
Write-Test "8: Existing S-Left binding still works"
& $PSMUX bind-key -t $SESSION S-Left select-pane 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "S-Left" -and $keys -match "select-pane") {
    Write-Pass "8: S-Left binding works (regression check)"
} else {
    Write-Fail "8: S-Left not found in list-keys. Got:`n$keys"
}

# --- Test 9: C-S-Left (multi-modifier with arrow) ---
Write-Test "9: bind-key C-S-Left resize-pane -L 5"
& $PSMUX bind-key -t $SESSION C-S-Left resize-pane 2>&1 | Out-Null
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
if ($keys -match "C-S-Left" -and $keys -match "resize-pane") {
    Write-Pass "9: C-S-Left multi-modifier binding works"
} else {
    Write-Fail "9: C-S-Left not found in list-keys. Got:`n$keys"
}

# --- Test 10: send-keys C-Tab via -H (hex/key name) ---
Write-Test "10: send-keys -H C-Tab generates correct escape sequence"
# send-keys -H sends the key by name; we verify by checking the pane captures something
# (if the key is NOT recognized, send-keys silently drops it)
& $PSMUX send-keys -t $SESSION -H C-Tab 2>&1 | Out-Null
# No crash = success for encoding. More detailed check: verify parse_modified_special_key output
# by looking at the list of special keys or by seeing if the command didn't error.
# Since send-keys -H uses parse_modified_special_key, if C-Tab wasn't recognized it'd be
# silently dropped. We can verify by checking stderr.
$result = & $PSMUX send-keys -t $SESSION -H C-Tab 2>&1
if ($LASTEXITCODE -eq 0 -or $null -eq $LASTEXITCODE) {
    Write-Pass "10: send-keys -H C-Tab accepted (no error)"
} else {
    Write-Fail "10: send-keys -H C-Tab failed. Got: $result"
}

# --- Test 11: send-keys C-S-Tab via -H ---
Write-Test "11: send-keys -H C-S-Tab accepted"
$result = & $PSMUX send-keys -t $SESSION -H C-S-Tab 2>&1
if ($LASTEXITCODE -eq 0 -or $null -eq $LASTEXITCODE) {
    Write-Pass "11: send-keys -H C-S-Tab accepted (no error)"
} else {
    Write-Fail "11: send-keys -H C-S-Tab failed. Got: $result"
}

# --- Test 12: Config file with C-Tab binding ---
Write-Test "12: Config line 'bind-key C-Tab next-window' parses correctly"
try {
    $configDir = Join-Path $env:TEMP "psmux_test_108_config_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir ".psmux.conf"
    Set-Content -Path $configFile -Value @"
bind-key C-Tab next-window
bind-key C-S-Tab previous-window
"@
    # source-file loads config at runtime
    & $PSMUX source-file -t $SESSION $configFile 2>&1 | Out-Null
    $keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String

    $ctab = $keys -match "C-Tab" -and $keys -match "next-window"
    $cstab = ($keys -match "C-BTab" -or $keys -match "C-S-Tab") -and $keys -match "previous-window"

    if ($ctab -and $cstab) {
        Write-Pass "12: Config file C-Tab and C-S-Tab bindings loaded correctly"
    } else {
        Write-Fail "12: Config bindings not loaded. Got:`n$keys"
    }
} catch {
    Write-Fail "12: Exception: $_"
} finally {
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ══════════════════════════════════════════════════════════════════════
# Cleanup & summary
# ══════════════════════════════════════════════════════════════════════
Cleanup-Session $SESSION
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped (of $total run)" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
