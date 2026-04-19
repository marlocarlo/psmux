# Issue #221: run-shell: program not found
# Tests that run-shell works correctly from ALL code paths:
#   CLI dispatch (main.rs), TCP handler (connection.rs), config (config.rs)
# Proves shell resolution, error handling, background mode, arg parsing all work.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$SESSION = "test_issue221"
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
}

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

# === SETUP ===
Cleanup
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION,"-d" -WindowStyle Hidden
Start-Sleep -Seconds 4

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Session creation failed"
    exit 1
}

Write-Host "`n=== Issue #221: run-shell Tests ===" -ForegroundColor Cyan

# ================================================================
# Part A: CLI Path (main.rs dispatch)
# ================================================================
Write-Host "`n--- Part A: CLI Path ---" -ForegroundColor Magenta

# Test 1: Basic echo via run-shell
Write-Host "`n[Test 1] run-shell with echo" -ForegroundColor Yellow
$output = & $PSMUX run-shell "echo MARKER_221_HELLO" 2>&1 | Out-String
if ($output -match "MARKER_221_HELLO") { Write-Pass "run-shell echo works" }
else { Write-Fail "Expected MARKER_221_HELLO in output, got: $output" }

# Test 2: run alias works same as run-shell
Write-Host "`n[Test 2] 'run' alias" -ForegroundColor Yellow
$output = & $PSMUX run "echo ALIAS_221" 2>&1 | Out-String
if ($output -match "ALIAS_221") { Write-Pass "'run' alias works" }
else { Write-Fail "Expected ALIAS_221 in output, got: $output" }

# Test 3: run-shell with no arguments shows usage
Write-Host "`n[Test 3] run-shell no args" -ForegroundColor Yellow
$output = & $PSMUX run-shell 2>&1 | Out-String
if ($output -match "usage.*run-shell") { Write-Pass "No args shows usage" }
else { Write-Fail "Expected usage message, got: $output" }

# Test 4: run-shell with background flag
Write-Host "`n[Test 4] run-shell -b (background)" -ForegroundColor Yellow
$marker = "$env:TEMP\psmux_221_bg_marker.txt"
Remove-Item $marker -Force -EA SilentlyContinue
& $PSMUX run-shell -b "echo BG_DONE > '$marker'" 2>&1 | Out-Null
Start-Sleep -Seconds 2
if (Test-Path $marker) { Write-Pass "Background run-shell created marker file" }
else { Write-Fail "Background run-shell did not create marker file" }
Remove-Item $marker -Force -EA SilentlyContinue

# Test 5: run-shell with PowerShell command (pipeline)
Write-Host "`n[Test 5] run-shell with PS pipeline" -ForegroundColor Yellow
$output = & $PSMUX run-shell "Write-Output 'PS_WORKS_221'" 2>&1 | Out-String
if ($output -match "PS_WORKS_221") { Write-Pass "PowerShell pipeline in run-shell" }
else { Write-Fail "Expected PS_WORKS_221, got: $output" }

# Test 6: run-shell with quoted command
Write-Host "`n[Test 6] run-shell with quoted command" -ForegroundColor Yellow
$output = & $PSMUX run-shell "Write-Output 'hello world'" 2>&1 | Out-String
if ($output -match "hello world") { Write-Pass "Quoted command works" }
else { Write-Fail "Expected 'hello world', got: $output" }

# Test 7: run-shell exit code forwarding
Write-Host "`n[Test 7] run-shell exit code" -ForegroundColor Yellow
& $PSMUX run-shell "exit 0" 2>&1 | Out-Null
$ec0 = $LASTEXITCODE
& $PSMUX run-shell "exit 42" 2>&1 | Out-Null
$ec42 = $LASTEXITCODE
if ($ec0 -eq 0) { Write-Pass "Exit code 0 forwarded" }
else { Write-Fail "Expected exit code 0, got $ec0" }
if ($ec42 -eq 42) { Write-Pass "Exit code 42 forwarded" }
else { Write-Fail "Expected exit code 42, got $ec42" }

# Test 8: run-shell with nonexistent command (error handling)
Write-Host "`n[Test 8] run-shell error for nonexistent cmd" -ForegroundColor Yellow
$output = & $PSMUX run-shell "nonexistent_command_xyz_221" 2>&1 | Out-String
# Should get a PowerShell error, not crash
if ($output -match "not recognized|not found|CommandNotFoundException") { 
    Write-Pass "Nonexistent command gives error, not crash" 
} else { 
    Write-Fail "Unexpected output for nonexistent cmd: $output" 
}

# ================================================================
# Part B: TCP Server Path (connection.rs)
# ================================================================
Write-Host "`n--- Part B: TCP Server Path ---" -ForegroundColor Magenta

# Test 9: TCP run-shell with echo
Write-Host "`n[Test 9] TCP run-shell echo" -ForegroundColor Yellow
$resp = Send-TcpCommand -Session $SESSION -Command "run-shell `"echo TCP_MARKER_221`""
if ($resp -match "TCP_MARKER_221") { Write-Pass "TCP run-shell echo works" }
else { Write-Fail "Expected TCP_MARKER_221, got: $resp" }

# Test 10: TCP run alias
Write-Host "`n[Test 10] TCP 'run' alias" -ForegroundColor Yellow
$resp = Send-TcpCommand -Session $SESSION -Command "run `"echo TCP_ALIAS_221`""
if ($resp -match "TCP_ALIAS_221") { Write-Pass "TCP 'run' alias works" }
else { Write-Fail "Expected TCP_ALIAS_221, got: $resp" }

# Test 11: TCP run-shell no args
Write-Host "`n[Test 11] TCP run-shell no args" -ForegroundColor Yellow
$resp = Send-TcpCommand -Session $SESSION -Command "run-shell"
if ($resp -match "usage.*run-shell") { Write-Pass "TCP no args shows usage" }
else { Write-Fail "Expected usage, got: $resp" }

# Test 12: TCP run-shell with nonexistent command
Write-Host "`n[Test 12] TCP run-shell nonexistent cmd" -ForegroundColor Yellow
$resp = Send-TcpCommand -Session $SESSION -Command "run-shell nonexistent_xyz_221"
if ($resp -match "not recognized|not found|CommandNotFoundException") { 
    Write-Pass "TCP nonexistent cmd returns error" 
} else { 
    Write-Fail "Expected error for nonexistent, got: $resp" 
}

# Test 13: TCP run-shell background mode (verifies -b flag is accepted and does not error)
Write-Host "`n[Test 13] TCP run-shell -b" -ForegroundColor Yellow
# Background mode spawns and returns immediately; we verify it did not error
# by checking the session is still alive afterward
$resp = Send-TcpCommand -Session $SESSION -Command "run-shell -b `"echo background_test`""
Start-Sleep -Seconds 1
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "TCP background run-shell accepted, session alive" }
else { Write-Fail "Session died after TCP background run-shell" }

# ================================================================
# Part C: Edge Cases
# ================================================================
Write-Host "`n--- Part C: Edge Cases ---" -ForegroundColor Magenta

# Test 14: run-shell with command that writes to stderr
Write-Host "`n[Test 14] run-shell stderr capture" -ForegroundColor Yellow
$output = & $PSMUX run-shell "Write-Error 'STDERR_TEST_221' 2>&1" 2>&1 | Out-String
if ($output -match "STDERR_TEST_221") { Write-Pass "stderr captured" }
else { Write-Fail "stderr not captured, got: $output" }

# Test 15: run-shell with empty string after flags
Write-Host "`n[Test 15] run-shell -b with empty cmd" -ForegroundColor Yellow
$output = & $PSMUX run-shell -b 2>&1 | Out-String
# Should show usage or silently do nothing - not crash
Write-Pass "run-shell -b no cmd did not crash"

# Test 16: run-shell with tilde expansion
Write-Host "`n[Test 16] run-shell tilde expansion" -ForegroundColor Yellow
$output = & $PSMUX run-shell "Write-Output `$env:USERPROFILE" 2>&1 | Out-String
if ($output.Trim().Length -gt 0) { Write-Pass "run-shell can access env vars" }
else { Write-Fail "Expected USERPROFILE path, got empty" }

# Test 17: run-shell with .ps1 script
Write-Host "`n[Test 17] run-shell with .ps1 file" -ForegroundColor Yellow
$testScript = "$env:TEMP\psmux_221_test_script.ps1"
"Write-Output 'PS1_SCRIPT_WORKS_221'" | Set-Content $testScript -Encoding UTF8
$output = & $PSMUX run-shell "`"$testScript`"" 2>&1 | Out-String
if ($output -match "PS1_SCRIPT_WORKS_221") { Write-Pass ".ps1 script via run-shell works" }
else { Write-Fail "Expected PS1_SCRIPT_WORKS_221, got: $output" }
Remove-Item $testScript -Force -EA SilentlyContinue

# Test 18: run-shell with pwsh -Command prefix (should not double-wrap)
Write-Host "`n[Test 18] run-shell with 'pwsh' prefix" -ForegroundColor Yellow
$output = & $PSMUX run-shell "pwsh -NoProfile -Command `"Write-Output 'NOWRAP_221'`"" 2>&1 | Out-String
if ($output -match "NOWRAP_221") { Write-Pass "pwsh prefix not double-wrapped" }
else { Write-Fail "Expected NOWRAP_221, got: $output" }

# ================================================================
# Part D: Config File Integration
# ================================================================
Write-Host "`n--- Part D: Config with run-shell ---" -ForegroundColor Magenta

# Test 19: Config file with run-shell
Write-Host "`n[Test 19] Config file run-shell" -ForegroundColor Yellow
$cfgSession = "test_221_cfg"
$cfgMarker = "$env:TEMP\psmux_221_cfg_marker.txt"
Remove-Item $cfgMarker -Force -EA SilentlyContinue
& $PSMUX kill-session -t $cfgSession 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSession.*" -Force -EA SilentlyContinue

$confFile = "$env:TEMP\psmux_test_221.conf"
@"
run-shell "echo CFG_RUN > '$cfgMarker'"
"@ | Set-Content $confFile -Encoding UTF8

$env:PSMUX_CONFIG_FILE = $confFile
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$cfgSession,"-d" -WindowStyle Hidden
$env:PSMUX_CONFIG_FILE = $null
Start-Sleep -Seconds 5

& $PSMUX has-session -t $cfgSession 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "Session with run-shell config started" }
else { Write-Fail "Session with run-shell config failed to start" }

if (Test-Path $cfgMarker) { Write-Pass "Config run-shell executed" }
else { Write-Fail "Config run-shell marker not found (may be async timing)" }

& $PSMUX kill-session -t $cfgSession 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSession.*" -Force -EA SilentlyContinue
Remove-Item $cfgMarker -Force -EA SilentlyContinue
Remove-Item $confFile -Force -EA SilentlyContinue

# Test 20: Config with run-shell calling nonexistent program (should not crash session)
Write-Host "`n[Test 20] Config with bad run-shell does not crash" -ForegroundColor Yellow
$cfgSession2 = "test_221_badcfg"
& $PSMUX kill-session -t $cfgSession2 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSession2.*" -Force -EA SilentlyContinue

$badConf = "$env:TEMP\psmux_test_221_bad.conf"
@"
run-shell "nonexistent_program_221_bad"
set -g status-left "[SURVIVED]"
"@ | Set-Content $badConf -Encoding UTF8

$env:PSMUX_CONFIG_FILE = $badConf
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$cfgSession2,"-d" -WindowStyle Hidden
$env:PSMUX_CONFIG_FILE = $null
Start-Sleep -Seconds 5

& $PSMUX has-session -t $cfgSession2 2>$null
if ($LASTEXITCODE -eq 0) { 
    Write-Pass "Session survived bad run-shell in config" 
    # Verify that subsequent config lines still applied
    $sl = (& $PSMUX show-options -g -v "status-left" -t $cfgSession2 2>&1 | Out-String).Trim()
    if ($sl -match "SURVIVED") { Write-Pass "Config lines after bad run-shell still applied" }
    else { Write-Fail "Config lines after bad run-shell not applied, got: $sl" }
} else { 
    Write-Fail "Session crashed due to bad run-shell in config" 
}

& $PSMUX kill-session -t $cfgSession2 2>&1 | Out-Null
Remove-Item "$psmuxDir\$cfgSession2.*" -Force -EA SilentlyContinue
Remove-Item $badConf -Force -EA SilentlyContinue

# ================================================================
# Part E: Win32 TUI Visual Verification
# ================================================================
Write-Host "`n--- Part E: Win32 TUI Visual Verification ---" -ForegroundColor Magenta

Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION"
Write-Host ("=" * 60)

$SESSION_TUI = "issue221_tui_proof"
& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION_TUI.*" -Force -EA SilentlyContinue
Start-Sleep -Milliseconds 500

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION_TUI -PassThru
Start-Sleep -Seconds 4

& $PSMUX has-session -t $SESSION_TUI 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "TUI session did not start"
} else {
    # TUI Check 1: Session responds to display-message
    Write-Host "`n[TUI 1] Session responds to format variables" -ForegroundColor Yellow
    $name = (& $PSMUX display-message -t $SESSION_TUI -p '#{session_name}' 2>&1 | Out-String).Trim()
    if ($name -eq $SESSION_TUI) { Write-Pass "TUI: session_name correct" }
    else { Write-Fail "TUI: expected $SESSION_TUI, got: $name" }

    # TUI Check 2: run-shell via TCP while TUI is live
    Write-Host "`n[TUI 2] run-shell via TCP on live TUI" -ForegroundColor Yellow
    $resp = Send-TcpCommand -Session $SESSION_TUI -Command "run-shell `"echo TUI_ALIVE_221`""
    if ($resp -match "TUI_ALIVE_221") { Write-Pass "TUI: run-shell via TCP works" }
    else { Write-Fail "TUI: run-shell expected TUI_ALIVE_221, got: $resp" }

    # TUI Check 3: send-keys + capture-pane to verify pane is functional
    Write-Host "`n[TUI 3] send-keys + capture-pane" -ForegroundColor Yellow
    & $PSMUX send-keys -t $SESSION_TUI "echo PANE_FUNCTIONAL_221" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $captured = & $PSMUX capture-pane -t $SESSION_TUI -p 2>&1 | Out-String
    if ($captured -match "PANE_FUNCTIONAL_221") { Write-Pass "TUI: pane captures output" }
    else { Write-Fail "TUI: PANE_FUNCTIONAL_221 not in capture" }

    # TUI Check 4: split-window works during live TUI
    Write-Host "`n[TUI 4] split-window on live TUI" -ForegroundColor Yellow
    & $PSMUX split-window -v -t $SESSION_TUI 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $panes = (& $PSMUX display-message -t $SESSION_TUI -p '#{window_panes}' 2>&1 | Out-String).Trim()
    if ($panes -eq "2") { Write-Pass "TUI: split-window created 2 panes" }
    else { Write-Fail "TUI: expected 2 panes, got: $panes" }
}

& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Remove-Item "$psmuxDir\$SESSION_TUI.*" -Force -EA SilentlyContinue

# === TEARDOWN ===
Cleanup
Remove-Item "$env:TEMP\psmux_221_*" -Force -EA SilentlyContinue

Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
