# PR #222 & #223: run-shell path handling and set-hook escaping
# Tests that:
#   1. run-shell with forward-slash tilde paths works (PR #222 claims it breaks)
#   2. run-shell with backslash tilde paths works
#   3. run-shell from config files with tilde paths works
#   4. set-hook with escaped quotes and paths with spaces works (PR #223 claims it breaks)
#   5. PPM-style plugin paths work from config
#   6. All paths work via CLI, TCP, and config code paths

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$SESSION = "test_pr222_223"
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Send-TcpCommand {
    param([string]$Session, [string]$Command)
    $port = (Get-Content "$psmuxDir\$Session.port" -Raw).Trim()
    $key = (Get-Content "$psmuxDir\$Session.key" -Raw).Trim()
    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
    $tcp.NoDelay = $true
    $stream = $tcp.GetStream()
    $writer = [System.IO.StreamWriter]::new($stream)
    $reader = [System.IO.StreamReader]::new($stream)
    $writer.Write("AUTH $key`n"); $writer.Flush()
    $authResp = $reader.ReadLine()
    if ($authResp -ne "OK") { $tcp.Close(); return "AUTH_FAILED" }
    $writer.Write("$Command`n"); $writer.Flush()
    $stream.ReadTimeout = 10000
    try { $resp = $reader.ReadLine() } catch { $resp = "TIMEOUT" }
    $tcp.Close()
    return $resp
}

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
}

# === SETUP: Create test scripts at plugin paths ===
$pluginDir = "$env:USERPROFILE\.psmux\plugins\ppm\scripts"
New-Item -ItemType Directory -Path $pluginDir -Force | Out-Null

$spaceDir = "$env:USERPROFILE\.psmux\plugins\test path with spaces"
New-Item -ItemType Directory -Path $spaceDir -Force | Out-Null

# Script that outputs a marker
"Write-Output 'FWDSLASH_CLI_OK'" | Set-Content "$pluginDir\test_fwdslash.ps1" -Encoding UTF8
"Write-Output 'BKSLASH_CLI_OK'" | Set-Content "$pluginDir\test_bkslash.ps1" -Encoding UTF8

# Script that writes marker file (for async config/hook testing)
"'CONFIG_FWDSLASH_OK' | Out-File '$env:TEMP\pr222_cfg_fwd.txt' -Encoding UTF8" | Set-Content "$pluginDir\test_cfg_fwd.ps1" -Encoding UTF8
"'CONFIG_BKSLASH_OK' | Out-File '$env:TEMP\pr222_cfg_bk.txt' -Encoding UTF8" | Set-Content "$pluginDir\test_cfg_bk.ps1" -Encoding UTF8
"'HOOK_SPACE_PATH_OK' | Out-File '$env:TEMP\pr223_hook.txt' -Encoding UTF8" | Set-Content "$spaceDir\hook_test.ps1" -Encoding UTF8
"'PPM_STYLE_OK' | Out-File '$env:TEMP\pr222_ppm.txt' -Encoding UTF8" | Set-Content "$pluginDir\test_ppm.ps1" -Encoding UTF8

# Create session
Cleanup
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION,"-d" -WindowStyle Hidden
Start-Sleep -Seconds 4

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Session creation failed"
    exit 1
}

Write-Host "`n=== PR #222 & #223: Path Handling Tests ===" -ForegroundColor Cyan

# ================================================================
# Part A: CLI Path (main.rs) - PR #222 claims these break
# ================================================================
Write-Host "`n--- Part A: CLI run-shell with tilde paths ---" -ForegroundColor Magenta

# Test 1: Forward-slash tilde path via CLI
Write-Host "`n[Test 1] CLI: run-shell with forward-slash tilde (~/.psmux/...)" -ForegroundColor Yellow
$out = & $PSMUX run-shell "~/.psmux/plugins/ppm/scripts/test_fwdslash.ps1" 2>&1 | Out-String
if ($out -match "FWDSLASH_CLI_OK") { Write-Pass "Forward-slash tilde path works via CLI" }
else { Write-Fail "Forward-slash tilde path BROKEN: $out" }

# Test 2: Backslash tilde path via CLI
Write-Host "`n[Test 2] CLI: run-shell with backslash tilde (~\.psmux\...)" -ForegroundColor Yellow
$out = & $PSMUX run-shell "~\.psmux\plugins\ppm\scripts\test_bkslash.ps1" 2>&1 | Out-String
if ($out -match "BKSLASH_CLI_OK") { Write-Pass "Backslash tilde path works via CLI" }
else { Write-Fail "Backslash tilde path BROKEN: $out" }

# Test 3: Forward-slash tilde with single quotes
Write-Host "`n[Test 3] CLI: run-shell with single-quoted forward-slash path" -ForegroundColor Yellow
$out = & $PSMUX run-shell "'~/.psmux/plugins/ppm/scripts/test_fwdslash.ps1'" 2>&1 | Out-String
if ($out -match "FWDSLASH_CLI_OK") { Write-Pass "Single-quoted forward-slash works" }
else { Write-Fail "Single-quoted forward-slash BROKEN: $out" }

# Test 4: Forward-slash tilde with double quotes
Write-Host "`n[Test 4] CLI: run-shell with double-quoted forward-slash path" -ForegroundColor Yellow
$out = & $PSMUX run-shell "`"~/.psmux/plugins/ppm/scripts/test_fwdslash.ps1`"" 2>&1 | Out-String
if ($out -match "FWDSLASH_CLI_OK") { Write-Pass "Double-quoted forward-slash works" }
else { Write-Fail "Double-quoted forward-slash BROKEN: $out" }

# ================================================================
# Part B: TCP Path (connection.rs) - PR #222 claims these break
# ================================================================
Write-Host "`n--- Part B: TCP run-shell with tilde paths ---" -ForegroundColor Magenta

# Test 5: Forward-slash tilde via TCP
Write-Host "`n[Test 5] TCP: run-shell with forward-slash tilde" -ForegroundColor Yellow
$resp = Send-TcpCommand -Session $SESSION -Command "run-shell ~/.psmux/plugins/ppm/scripts/test_fwdslash.ps1"
if ($resp -match "FWDSLASH_CLI_OK") { Write-Pass "Forward-slash tilde works via TCP" }
else { Write-Fail "Forward-slash tilde via TCP BROKEN: $resp" }

# Test 6: Backslash tilde via TCP
Write-Host "`n[Test 6] TCP: run-shell with backslash tilde" -ForegroundColor Yellow
$resp = Send-TcpCommand -Session $SESSION -Command "run-shell ~\.psmux\plugins\ppm\scripts\test_bkslash.ps1"
if ($resp -match "BKSLASH_CLI_OK") { Write-Pass "Backslash tilde works via TCP" }
else { Write-Fail "Backslash tilde via TCP BROKEN: $resp" }

# ================================================================
# Part C: Config File Path (config.rs) - The real plugin scenario
# ================================================================
Write-Host "`n--- Part C: Config file run-shell with tilde paths ---" -ForegroundColor Magenta

# Test 7: Forward-slash tilde from config (PPM style)
Write-Host "`n[Test 7] Config: forward-slash tilde run-shell" -ForegroundColor Yellow
$cfgSess = "pr222_cfg_fwd"
Remove-Item "$env:TEMP\pr222_cfg_fwd.txt" -Force -EA SilentlyContinue
& $PSMUX kill-session -t $cfgSess 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSess.*" -Force -EA SilentlyContinue
$conf = "$env:TEMP\pr222_test_fwd.conf"
"run-shell '~/.psmux/plugins/ppm/scripts/test_cfg_fwd.ps1'" | Set-Content $conf -Encoding UTF8
$env:PSMUX_CONFIG_FILE = $conf
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$cfgSess,"-d" -WindowStyle Hidden
$env:PSMUX_CONFIG_FILE = $null
Start-Sleep -Seconds 5
& $PSMUX has-session -t $cfgSess 2>$null
if ($LASTEXITCODE -eq 0 -and (Test-Path "$env:TEMP\pr222_cfg_fwd.txt")) {
    Write-Pass "Config forward-slash tilde run-shell executed"
} else {
    Write-Fail "Config forward-slash tilde run-shell DID NOT execute"
}
& $PSMUX kill-session -t $cfgSess 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSess.*" -Force -EA SilentlyContinue

# Test 8: Backslash tilde from config
Write-Host "`n[Test 8] Config: backslash tilde run-shell" -ForegroundColor Yellow
$cfgSess2 = "pr222_cfg_bk"
Remove-Item "$env:TEMP\pr222_cfg_bk.txt" -Force -EA SilentlyContinue
& $PSMUX kill-session -t $cfgSess2 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSess2.*" -Force -EA SilentlyContinue
$conf2 = "$env:TEMP\pr222_test_bk.conf"
"run-shell '~\.psmux\plugins\ppm\scripts\test_cfg_bk.ps1'" | Set-Content $conf2 -Encoding UTF8
$env:PSMUX_CONFIG_FILE = $conf2
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$cfgSess2,"-d" -WindowStyle Hidden
$env:PSMUX_CONFIG_FILE = $null
Start-Sleep -Seconds 5
& $PSMUX has-session -t $cfgSess2 2>$null
if ($LASTEXITCODE -eq 0 -and (Test-Path "$env:TEMP\pr222_cfg_bk.txt")) {
    Write-Pass "Config backslash tilde run-shell executed"
} else {
    Write-Fail "Config backslash tilde run-shell DID NOT execute"
}
& $PSMUX kill-session -t $cfgSess2 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSess2.*" -Force -EA SilentlyContinue

# Test 9: PPM-style plugin path from config
Write-Host "`n[Test 9] Config: PPM-style plugin run-shell" -ForegroundColor Yellow
$cfgSessPpm = "pr222_ppm"
Remove-Item "$env:TEMP\pr222_ppm.txt" -Force -EA SilentlyContinue
& $PSMUX kill-session -t $cfgSessPpm 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSessPpm.*" -Force -EA SilentlyContinue
$confPpm = "$env:TEMP\pr222_test_ppm.conf"
"run-shell '~/.psmux/plugins/ppm/scripts/test_ppm.ps1'" | Set-Content $confPpm -Encoding UTF8
$env:PSMUX_CONFIG_FILE = $confPpm
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$cfgSessPpm,"-d" -WindowStyle Hidden
$env:PSMUX_CONFIG_FILE = $null
Start-Sleep -Seconds 5
& $PSMUX has-session -t $cfgSessPpm 2>$null
if ($LASTEXITCODE -eq 0 -and (Test-Path "$env:TEMP\pr222_ppm.txt")) {
    Write-Pass "PPM-style run-shell from config executed"
} else {
    Write-Fail "PPM-style run-shell from config DID NOT execute"
}
& $PSMUX kill-session -t $cfgSessPpm 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSessPpm.*" -Force -EA SilentlyContinue

# ================================================================
# Part D: set-hook with escaped quotes (PR #223 claim)
# ================================================================
Write-Host "`n--- Part D: set-hook with escaped quotes ---" -ForegroundColor Magenta

# Test 10: set-hook from config with escaped quotes and path with spaces
Write-Host "`n[Test 10] Config: set-hook with escaped quotes, path with spaces" -ForegroundColor Yellow
$cfgHook = "pr223_hook"
Remove-Item "$env:TEMP\pr223_hook.txt" -Force -EA SilentlyContinue
& $PSMUX kill-session -t $cfgHook 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgHook.*" -Force -EA SilentlyContinue
$confHook = "$env:TEMP\pr223_test_hook.conf"
@'
set-hook -g after-new-window 'run-shell "pwsh -NoProfile -File \"~/.psmux/plugins/test path with spaces/hook_test.ps1\""'
'@ | Set-Content $confHook -Encoding UTF8
$env:PSMUX_CONFIG_FILE = $confHook
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$cfgHook,"-d" -WindowStyle Hidden
$env:PSMUX_CONFIG_FILE = $null
Start-Sleep -Seconds 5
& $PSMUX has-session -t $cfgHook 2>$null
if ($LASTEXITCODE -eq 0) {
    # Trigger the hook by creating a new window
    & $PSMUX new-window -t $cfgHook 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    if (Test-Path "$env:TEMP\pr223_hook.txt") {
        Write-Pass "set-hook with escaped quotes and spaces path WORKS"
    } else {
        Write-Fail "set-hook with escaped quotes did NOT fire (marker missing)"
    }
} else {
    Write-Fail "Session with hook config failed to start"
}
& $PSMUX kill-session -t $cfgHook 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgHook.*" -Force -EA SilentlyContinue

# Test 11: set-hook via CLI (no spaces in path, tests CLI set-hook dispatch)
Write-Host "`n[Test 11] CLI: set-hook dispatches run-shell correctly" -ForegroundColor Yellow
$hookSess = "pr223_cli_hook"
Remove-Item "$env:TEMP\pr222_ppm.txt" -Force -EA SilentlyContinue
& $PSMUX kill-session -t $hookSess 2>&1 | Out-Null
Remove-Item "$psmuxDir\$hookSess.*" -Force -EA SilentlyContinue
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$hookSess,"-d" -WindowStyle Hidden
Start-Sleep -Seconds 4
& $PSMUX has-session -t $hookSess 2>$null
if ($LASTEXITCODE -eq 0) {
    & $PSMUX set-hook -g -t $hookSess after-new-window "run-shell `"~/.psmux/plugins/ppm/scripts/test_ppm.ps1`"" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    & $PSMUX new-window -t $hookSess 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    if (Test-Path "$env:TEMP\pr222_ppm.txt") {
        Write-Pass "CLI set-hook fires run-shell with tilde path"
    } else {
        Write-Fail "CLI set-hook did not fire (marker missing)"
    }
} else {
    Write-Fail "Session for CLI hook test failed to start"
}
& $PSMUX kill-session -t $hookSess 2>&1 | Out-Null
Remove-Item "$psmuxDir\$hookSess.*" -Force -EA SilentlyContinue

# ================================================================
# Part E: Edge Cases
# ================================================================
Write-Host "`n--- Part E: Edge Cases ---" -ForegroundColor Magenta

# Test 12: run-shell with URL (forward slashes must NOT be converted to backslashes)
Write-Host "`n[Test 12] run-shell with URL (forward slashes preserved)" -ForegroundColor Yellow
$out = & $PSMUX run-shell "echo https://example.com/api/v1" 2>&1 | Out-String
if ($out -match "https://example.com/api/v1") { Write-Pass "URL forward slashes preserved" }
else { Write-Fail "URL forward slashes BROKEN: $out" }

# Test 13: run-shell with absolute Windows path (no tilde)
Write-Host "`n[Test 13] run-shell with absolute Windows path" -ForegroundColor Yellow
$absScript = "$env:TEMP\pr222_abs_test.ps1"
"Write-Output 'ABSOLUTE_PATH_OK'" | Set-Content $absScript -Encoding UTF8
$out = & $PSMUX run-shell "$absScript" 2>&1 | Out-String
if ($out -match "ABSOLUTE_PATH_OK") { Write-Pass "Absolute path works" }
else { Write-Fail "Absolute path BROKEN: $out" }
Remove-Item $absScript -Force -EA SilentlyContinue

# Test 14: run-shell with mixed forward/backslash in path (no tilde)
Write-Host "`n[Test 14] run-shell preserves command structure" -ForegroundColor Yellow
$out = & $PSMUX run-shell "echo test/value\other" 2>&1 | Out-String
if ($out.Trim().Length -gt 0) { Write-Pass "Mixed slash echo did not crash" }
else { Write-Fail "Mixed slash echo produced no output" }

# ================================================================
# Part F: Win32 TUI Visual Verification
# ================================================================
Write-Host "`n--- Part F: Win32 TUI Visual Verification ---" -ForegroundColor Magenta
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION"
Write-Host ("=" * 60)

$SESSION_TUI = "pr222_223_tui"
& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION_TUI.*" -Force -EA SilentlyContinue
Start-Sleep -Milliseconds 500

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION_TUI -PassThru
Start-Sleep -Seconds 4

& $PSMUX has-session -t $SESSION_TUI 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "TUI session did not start"
} else {
    # TUI Check 1: Session responds
    Write-Host "`n[TUI 1] Session responds" -ForegroundColor Yellow
    $name = (& $PSMUX display-message -t $SESSION_TUI -p '#{session_name}' 2>&1 | Out-String).Trim()
    if ($name -eq $SESSION_TUI) { Write-Pass "TUI: session_name correct" }
    else { Write-Fail "TUI: expected $SESSION_TUI, got: $name" }

    # TUI Check 2: run-shell via TCP on live TUI with tilde path
    Write-Host "`n[TUI 2] TCP run-shell with tilde path on live TUI" -ForegroundColor Yellow
    $resp = Send-TcpCommand -Session $SESSION_TUI -Command "run-shell ~/.psmux/plugins/ppm/scripts/test_fwdslash.ps1"
    if ($resp -match "FWDSLASH_CLI_OK") { Write-Pass "TUI: tilde path run-shell via TCP works" }
    else { Write-Fail "TUI: tilde path run-shell failed: $resp" }

    # TUI Check 3: send-keys + capture-pane
    Write-Host "`n[TUI 3] Pane functional (send-keys + capture-pane)" -ForegroundColor Yellow
    & $PSMUX send-keys -t $SESSION_TUI "echo TUI_ALIVE_PR222" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $captured = & $PSMUX capture-pane -t $SESSION_TUI -p 2>&1 | Out-String
    if ($captured -match "TUI_ALIVE_PR222") { Write-Pass "TUI: pane captures output" }
    else { Write-Fail "TUI: output not in capture" }

    # TUI Check 4: set-hook on live TUI triggers correctly (no spaces in path)
    Write-Host "`n[TUI 4] set-hook fires on live TUI" -ForegroundColor Yellow
    Remove-Item "$env:TEMP\pr222_ppm.txt" -Force -EA SilentlyContinue
    & $PSMUX set-hook -g -t $SESSION_TUI after-new-window "run-shell `"~/.psmux/plugins/ppm/scripts/test_ppm.ps1`"" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    & $PSMUX new-window -t $SESSION_TUI 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    if (Test-Path "$env:TEMP\pr222_ppm.txt") { Write-Pass "TUI: set-hook fires on new-window" }
    else { Write-Fail "TUI: set-hook did not fire" }
}

& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Remove-Item "$psmuxDir\$SESSION_TUI.*" -Force -EA SilentlyContinue

# === TEARDOWN ===
Cleanup
Remove-Item "$env:TEMP\pr222_*" -Force -EA SilentlyContinue
Remove-Item "$env:TEMP\pr223_*" -Force -EA SilentlyContinue
Remove-Item "$env:TEMP\psmux_test_222_*" -Force -EA SilentlyContinue
Remove-Item "$env:TEMP\psmux_test_223_*" -Force -EA SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\plugins\ppm\scripts\test_*" -Force -EA SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\plugins\test path with spaces" -Recurse -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\pr222_*" -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\pr223_*" -Force -EA SilentlyContinue

Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
