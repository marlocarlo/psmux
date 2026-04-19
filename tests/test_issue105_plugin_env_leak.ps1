# psmux Issue #105 — @plugin option leaks into child shell environment
#
# Tests that set -g @plugin and other @-prefixed user options do NOT
# appear as environment variables in child panes, while still being
# accessible via show-options and format strings.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue105_plugin_env_leak.ps1

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

$SESSION = "test_105"

function Wait-ForSession {
    param($name, $timeout = 10)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Capture-Pane {
    param($target)
    $raw = & $PSMUX capture-pane -t $target -p 2>&1
    return ($raw | Out-String)
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #105: @plugin must NOT leak into child shell env"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: @plugin does NOT appear in child pane environment ---
Write-Test "1: set -g @plugin does NOT leak to child pane env"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "1: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 3

    # Set @plugin option (mimics config: set -g @plugin 'psmux-plugins/psmux-sensible')
    & $PSMUX set-option -g -t $SESSION "@plugin" "psmux-plugins/psmux-sensible" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Split a new pane — it should NOT have @plugin as an env var
    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    # Try to read $env:@plugin — should be empty/error
    # PowerShell can't even reference $env:@plugin without ${} syntax,
    # so we check if the variable exists via Get-ChildItem
    & $PSMUX send-keys -t $SESSION 'Get-ChildItem env: | Where-Object { $_.Name -match "plugin" } | ForEach-Object { Write-Output "LEAKED=$($_.Name)=$($_.Value)" }; Write-Output "CHECK_DONE"' Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane $SESSION

    if ($cap -match "LEAKED=.*plugin") {
        Write-Fail "1: @plugin leaked into child env! Got:`n$cap"
    } elseif ($cap -match "CHECK_DONE") {
        Write-Pass "1: @plugin does NOT leak into child pane environment"
    } else {
        Write-Fail "1: Could not verify (CHECK_DONE not found). Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 2: @custom_option does NOT leak ---
Write-Test "2: set -g @custom_option does NOT leak to child env"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "2: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 3

    & $PSMUX set-option -g -t $SESSION "@my_custom_opt" "test_value_123" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Get-ChildItem env: | Where-Object { $_.Name -match "custom" } | ForEach-Object { Write-Output "LEAKED=$($_.Name)=$($_.Value)" }; Write-Output "CHECK_DONE"' Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane $SESSION

    if ($cap -match "LEAKED=.*custom") {
        Write-Fail "2: @my_custom_opt leaked! Got:`n$cap"
    } elseif ($cap -match "CHECK_DONE") {
        Write-Pass "2: @custom_option does NOT leak"
    } else {
        Write-Fail "2: Could not verify. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 3: show-options still shows @plugin ---
Write-Test "3: show-options still displays @plugin value"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "3: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 2

    & $PSMUX set-option -g -t $SESSION "@plugin" "psmux-plugins/psmux-sensible" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $output = & $PSMUX show-options -t $SESSION 2>&1 | Out-String
    if ($output -match "@plugin.*psmux-sensible") {
        Write-Pass "3: show-options displays @plugin correctly"
    } else {
        Write-Fail "3: @plugin not in show-options. Got:`n$output"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 4: show-option -v @plugin returns value ---
Write-Test "4: show-option -v @plugin returns the value"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "4: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 2

    & $PSMUX set-option -g -t $SESSION "@plugin" "psmux-plugins/psmux-sensible" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $val = & $PSMUX show-options -v -t $SESSION "@plugin" 2>&1 | Out-String
    if ($val -match "psmux-sensible") {
        Write-Pass "4: show-option -v @plugin returns correct value"
    } else {
        Write-Fail "4: show-option -v @plugin wrong. Got: $val"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 5: set-environment (real env vars) still works ---
Write-Test "5: set-environment (real env vars) still propagates to child"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "5: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 3

    & $PSMUX set-environment -t $SESSION PSMUX_REAL_ENVVAR "real_value_ok" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Write-Output "ENVVAL=$env:PSMUX_REAL_ENVVAR"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION

    if ($cap -match "ENVVAL=real_value_ok") {
        Write-Pass "5: Real env vars (set-environment) still propagate"
    } else {
        Write-Fail "5: Real env var not propagated. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 6: No ParserError on startup with @plugin in config ---
Write-Test "6: Config with @plugin does NOT cause ParserError on startup"
try {
    $configDir = Join-Path $env:TEMP "psmux_test_105_cfg_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir ".psmux.conf"
    Set-Content -Path $configFile -Value @"
set -g @plugin 'psmux-plugins/psmux-sensible'
set -g @my_theme 'catppuccin'
"@

    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "6: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 2

    # Source the config that sets @plugin
    & $PSMUX source-file -t $SESSION $configFile 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Split and check for ParserError
    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    $cap = Capture-Pane $SESSION
    if ($cap -match "ParserError|not followed by a valid variable") {
        Write-Fail "6: ParserError in child pane! @plugin leaked. Got:`n$cap"
    } else {
        Write-Pass "6: No ParserError — @plugin not leaking to child shells"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 7: @option -u (unset) removes from user_options ---
Write-Test "7: set -u @option removes it from show-options"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "7: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 2

    & $PSMUX set-option -g -t $SESSION "@removable" "temp_value" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    # Verify it's there
    $before = & $PSMUX show-options -t $SESSION 2>&1 | Out-String
    if ($before -notmatch "@removable") { Write-Fail "7: Pre-condition: @removable not set"; throw "skip" }

    # Unset
    & $PSMUX set-option -u -t $SESSION "@removable" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    $after = & $PSMUX show-options -t $SESSION 2>&1 | Out-String
    if ($after -match "@removable") {
        Write-Fail "7: @removable still present after -u. Got:`n$after"
    } else {
        Write-Pass "7: @option -u correctly removes from user_options"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "7: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 8: Multiple @plugin values (append) don't leak ---
Write-Test "8: Multiple @plugin set -a (append) don't leak"
try {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "8: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 3

    & $PSMUX set-option -g -t $SESSION "@plugin" "psmux-plugins/psmux-sensible" 2>&1 | Out-Null
    & $PSMUX set-option -ga -t $SESSION "@plugin" ",psmux-plugins/psmux-cpu" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Get-ChildItem env: | Where-Object { $_.Name -match "plugin" } | ForEach-Object { Write-Output "LEAKED=$($_.Name)" }; Write-Output "APPEND_CHECK"' Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane $SESSION

    if ($cap -match "LEAKED=.*plugin") {
        Write-Fail "8: Appended @plugin leaked! Got:`n$cap"
    } elseif ($cap -match "APPEND_CHECK") {
        Write-Pass "8: Appended @plugin values don't leak"
    } else {
        Write-Fail "8: Could not verify. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "8: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 9: Real plugin config (psmux-sensible) loads without errors ---
Write-Test "9: Real plugin psmux-sensible loads without errors"
try {
    $pluginConf = "$env:USERPROFILE\.psmux\plugins\psmux-plugins\psmux-sensible\plugin.conf"
    if (-not (Test-Path $pluginConf)) {
        Write-Skip "9: psmux-sensible plugin not installed at $pluginConf"
        throw "skip"
    }

    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION)) { Write-Fail "9: Session did not start"; throw "skip" }
    Start-Sleep -Seconds 2

    # Set the plugin (triggers auto-source)
    & $PSMUX set-option -g -t $SESSION "@plugin" "psmux-plugins/psmux-sensible" 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    # Source the plugin conf directly
    & $PSMUX source-file -t $SESSION $pluginConf 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    # Split and verify no errors
    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    $cap = Capture-Pane $SESSION
    if ($cap -match "ParserError|Error|not valid") {
        Write-Fail "9: Plugin config caused errors in child pane. Got:`n$cap"
    } else {
        Write-Pass "9: psmux-sensible loads cleanly, no env leak"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "9: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# ══════════════════════════════════════════════════════════════════════
# Cleanup & summary
# ══════════════════════════════════════════════════════════════════════
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped (of $total run)" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
