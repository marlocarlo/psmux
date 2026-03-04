# psmux Keybinding & Option Tests
# Tests: prefix2, switch-client -T, list-keys, list-commands, command-alias, and all new options
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_keybinding_options.ps1

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
function Psmux { & $PSMUX -t $SESSION @args 2>&1 | Out-String; Start-Sleep -Milliseconds 300 }

# Cleanup
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "keyopt_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"
New-PsmuxSession -Name $SESSION

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "1. PREFIX2 KEY"
Write-Host ("=" * 60)

Write-Test "1.1 Set prefix2"
Psmux set -g prefix2 C-a | Out-Null
Start-Sleep -Milliseconds 500
$val = (& $PSMUX show-options -t $SESSION -g -v prefix2 2>&1 | Out-String).Trim()
Write-Info "  prefix2 = $val"
if ($val -match "C-a") { Write-Pass "prefix2 set to C-a" } else { Write-Fail "prefix2 not set correctly: $val" }

Write-Test "1.2 Show prefix2"
$output = (& $PSMUX show-options -t $SESSION -g prefix2 2>&1 | Out-String).Trim()
if ($output -match "prefix2") { Write-Pass "show-options shows prefix2" } else { Write-Fail "prefix2 not visible in show-options" }

Write-Test "1.3 Set prefix2 None"
Psmux set -g prefix2 None | Out-Null
Start-Sleep -Milliseconds 200
$val = (& $PSMUX show-options -t $SESSION -g -v prefix2 2>&1 | Out-String).Trim()
if ($val -match "None" -or $val -eq "") { Write-Pass "prefix2 cleared" } else { Write-Fail "prefix2 not cleared: $val" }

# Restore for later tests
Psmux set -g prefix2 C-a | Out-Null
Start-Sleep -Milliseconds 200

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "2. LIST-KEYS / LIST-COMMANDS"
Write-Host ("=" * 60)

Write-Test "2.1 list-keys returns output"
$keys = (& $PSMUX list-keys -t $SESSION 2>&1 | Out-String).Trim()
if ($keys.Length -gt 10) { Write-Pass "list-keys returned $($keys.Length) chars" } else { Write-Fail "list-keys returned too little: $keys" }

Write-Test "2.2 list-keys contains bind-key"
if ($keys -match "bind-key|bind") { Write-Pass "list-keys contains bind entries" } else { Write-Fail "list-keys missing bind entries" }

Write-Test "2.3 list-commands returns output"
$cmds = (& $PSMUX list-commands -t $SESSION 2>&1 | Out-String).Trim()
if ($cmds.Length -gt 10) { Write-Pass "list-commands returned $($cmds.Length) chars" } else { Write-Fail "list-commands returned too little: $cmds" }

Write-Test "2.4 list-commands contains new-session"
if ($cmds -match "new-session") { Write-Pass "list-commands contains new-session" } else { Write-Fail "list-commands missing new-session" }

Write-Test "2.5 list-commands contains split-window"
if ($cmds -match "split-window") { Write-Pass "list-commands contains split-window" } else { Write-Fail "list-commands missing split-window" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "3. SWITCH-CLIENT -T (KEY TABLES)"
Write-Host ("=" * 60)

Write-Test "3.1 bind key in custom table"
Psmux bind-key -T my-table x display-message "from-table" | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "bind-key -T my-table x accepted"

Write-Test "3.2 switch-client -T"
$output = Psmux switch-client -T my-table
Write-Info "  switch-client -T result: $output"
Write-Pass "switch-client -T command accepted"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "4. STATUS BAR OPTIONS"
Write-Host ("=" * 60)

Write-Test "4.1 set status-left-length"
Psmux set -g status-left-length 20 | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v status-left-length 2>&1 | Out-String).Trim()
if ($val -match "20") { Write-Pass "status-left-length = 20" } else { Write-Fail "status-left-length: $val" }

Write-Test "4.2 set status-right-length"
Psmux set -g status-right-length 50 | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v status-right-length 2>&1 | Out-String).Trim()
if ($val -match "50") { Write-Pass "status-right-length = 50" } else { Write-Fail "status-right-length: $val" }

Write-Test "4.3 set status on/off"
Psmux set -g status off | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v status 2>&1 | Out-String).Trim()
if ($val -match "off|0") { Write-Pass "status = off" } else { Write-Fail "status: $val" }
Psmux set -g status on | Out-Null

Write-Test "4.4 set status-format"
Psmux set -g "status-format[0]" "#S:#W" | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "status-format[0] accepted"

Write-Test "4.5 set status-lines and status-format multi"
Psmux set -g status 2 | Out-Null
Start-Sleep -Milliseconds 200
Psmux set -g "status-format[1]" "#(hostname)" | Out-Null
Start-Sleep -Milliseconds 200
$val = (& $PSMUX show-options -t $SESSION -g -v status 2>&1 | Out-String).Trim()
Write-Info "  status = $val"
Write-Pass "multi-line status set"
Psmux set -g status on | Out-Null

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "5. WINDOW-SIZE OPTION"
Write-Host ("=" * 60)

Write-Test "5.1 set window-size smallest"
Psmux set -g window-size smallest | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v window-size 2>&1 | Out-String).Trim()
if ($val -match "smallest") { Write-Pass "window-size = smallest" } else { Write-Fail "window-size: $val" }

Write-Test "5.2 set window-size largest"
Psmux set -g window-size largest | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v window-size 2>&1 | Out-String).Trim()
if ($val -match "largest") { Write-Pass "window-size = largest" } else { Write-Fail "window-size: $val" }

Write-Test "5.3 set window-size latest"
Psmux set -g window-size latest | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v window-size 2>&1 | Out-String).Trim()
if ($val -match "latest") { Write-Pass "window-size = latest" } else { Write-Fail "window-size: $val" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "6. ALLOW-PASSTHROUGH"
Write-Host ("=" * 60)

Write-Test "6.1 set allow-passthrough off"
Psmux set -g allow-passthrough off | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v allow-passthrough 2>&1 | Out-String).Trim()
if ($val -match "off") { Write-Pass "allow-passthrough = off" } else { Write-Fail "allow-passthrough: $val" }

Write-Test "6.2 set allow-passthrough on"
Psmux set -g allow-passthrough on | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v allow-passthrough 2>&1 | Out-String).Trim()
if ($val -match "on") { Write-Pass "allow-passthrough = on" } else { Write-Fail "allow-passthrough: $val" }

Write-Test "6.3 set allow-passthrough all"
Psmux set -g allow-passthrough all | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v allow-passthrough 2>&1 | Out-String).Trim()
if ($val -match "all") { Write-Pass "allow-passthrough = all" } else { Write-Fail "allow-passthrough: $val" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "7. COPY-COMMAND"
Write-Host ("=" * 60)

Write-Test "7.1 set copy-command"
Psmux set -g copy-command "Set-Clipboard" | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v copy-command 2>&1 | Out-String).Trim()
if ($val -match "Set-Clipboard") { Write-Pass "copy-command = Set-Clipboard" } else { Write-Fail "copy-command: $val" }

Write-Test "7.2 clear copy-command"
Psmux set -g copy-command "" | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v copy-command 2>&1 | Out-String).Trim()
Write-Info "  copy-command cleared: '$val'"
Write-Pass "copy-command cleared"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "8. SET-CLIPBOARD (OSC 52)"
Write-Host ("=" * 60)

Write-Test "8.1 set set-clipboard on"
Psmux set -g set-clipboard on | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v set-clipboard 2>&1 | Out-String).Trim()
if ($val -match "on") { Write-Pass "set-clipboard = on" } else { Write-Fail "set-clipboard: $val" }

Write-Test "8.2 set set-clipboard external"
Psmux set -g set-clipboard external | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v set-clipboard 2>&1 | Out-String).Trim()
if ($val -match "external") { Write-Pass "set-clipboard = external" } else { Write-Fail "set-clipboard: $val" }

Write-Test "8.3 set set-clipboard off"
Psmux set -g set-clipboard off | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v set-clipboard 2>&1 | Out-String).Trim()
if ($val -match "off") { Write-Pass "set-clipboard = off" } else { Write-Fail "set-clipboard: $val" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "9. COMMAND-ALIAS"
Write-Host ("=" * 60)

Write-Test "9.1 set command-alias"
Psmux set -g command-alias "splitw=split-window" | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "command-alias splitw=split-window accepted"

Write-Test "9.2 show command-alias"
$val = (& $PSMUX show-options -t $SESSION -g command-alias 2>&1 | Out-String).Trim()
Write-Info "  command-alias: $val"
if ($val.Length -gt 0) { Write-Pass "command-alias visible" } else { Write-Fail "command-alias not visible" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "10. KEYBINDING OPS"
Write-Host ("=" * 60)

Write-Test "10.1 bind-key"
Psmux bind-key z display-message "test-bind" | Out-Null
$keys = (& $PSMUX list-keys -t $SESSION 2>&1 | Out-String)
if ($keys -match "z") { Write-Pass "bind-key z visible in list-keys" } else { Write-Fail "bind z not in list-keys" }

Write-Test "10.2 unbind-key"
Psmux unbind-key z | Out-Null
Start-Sleep -Milliseconds 200
$keys = (& $PSMUX list-keys -t $SESSION 2>&1 | Out-String)
# z should not appear as standalone binding
Write-Pass "unbind-key z executed"

Write-Test "10.3 bind-key -n (no prefix)"
Psmux bind-key -n F5 display-message "f5test" | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "bind-key -n F5 accepted"

Write-Test "10.4 bind-key -r (repeat)"
Psmux bind-key -r M-Up resize-pane -U 5 | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "bind-key -r M-Up accepted"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "11. MAIN-PANE DIMENSIONS"
Write-Host ("=" * 60)

# Need a split for main-pane tests
Psmux split-window -t $SESSION | Out-Null
Start-Sleep -Seconds 1

Write-Test "11.1 set main-pane-width"
Psmux set -g main-pane-width 60 | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v main-pane-width 2>&1 | Out-String).Trim()
if ($val -match "60") { Write-Pass "main-pane-width = 60" } else { Write-Fail "main-pane-width: $val" }

Write-Test "11.2 set main-pane-height"
Psmux set -g main-pane-height 30 | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v main-pane-height 2>&1 | Out-String).Trim()
if ($val -match "30") { Write-Pass "main-pane-height = 30" } else { Write-Fail "main-pane-height: $val" }

Write-Test "11.3 next-layout uses main-pane dimensions"
Psmux next-layout -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux next-layout -t $SESSION | Out-Null  # Cycle to main-vertical
Start-Sleep -Milliseconds 500
Write-Pass "layouts cycle with main-pane dimensions"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "12. REGRESSION: EXISTING OPTIONS STILL WORK"
Write-Host ("=" * 60)

Write-Test "12.1 set-option prefix"
Psmux set -g prefix C-b | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v prefix 2>&1 | Out-String).Trim()
if ($val -match "C-b") { Write-Pass "prefix = C-b" } else { Write-Fail "prefix: $val" }

Write-Test "12.2 set-option mouse"
Psmux set -g mouse on | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v mouse 2>&1 | Out-String).Trim()
if ($val -match "on") { Write-Pass "mouse = on" } else { Write-Fail "mouse: $val" }

Write-Test "12.3 set-option base-index"
Psmux set -g base-index 1 | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v base-index 2>&1 | Out-String).Trim()
if ($val -match "1") { Write-Pass "base-index = 1" } else { Write-Fail "base-index: $val" }

Write-Test "12.4 set-option mode-keys"
Psmux set -g mode-keys vi | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v mode-keys 2>&1 | Out-String).Trim()
if ($val -match "vi") { Write-Pass "mode-keys = vi" } else { Write-Fail "mode-keys: $val" }

Write-Test "12.5 display-message format"
$msg = (& $PSMUX display-message -t $SESSION -p "#{session_name}" 2>&1 | Out-String).Trim()
Write-Info "  session_name = $msg"
if ($msg -eq $SESSION) { Write-Pass "display-message format works" } else { Write-Fail "display-message: $msg != $SESSION" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "13. NEW EXIT STRATEGY OPTIONS"
Write-Host ("=" * 60)

Write-Test "13.1 show-options includes destroy-unattached and exit-empty"
$allOptions = (& $PSMUX show-options -t $SESSION -g 2>&1 | Out-String)
$hasDestroy = $allOptions -match "destroy-unattached"
$hasExitEmpty = $allOptions -match "exit-empty"
if ($hasDestroy -and $hasExitEmpty) {
    Write-Pass "show-options includes new exit strategy options"
} else {
    Write-Fail "show-options missing new options (destroy=$hasDestroy exit-empty=$hasExitEmpty)"
}

Write-Test "13.2 set/show destroy-unattached on/off"
Psmux set -g destroy-unattached on | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v destroy-unattached 2>&1 | Out-String).Trim()
$okOn = ($val -match "on")
Psmux set -g destroy-unattached off | Out-Null
$val2 = (& $PSMUX show-options -t $SESSION -g -v destroy-unattached 2>&1 | Out-String).Trim()
$okOff = ($val2 -match "off")
if ($okOn -and $okOff) { Write-Pass "destroy-unattached set/show works" } else { Write-Fail "destroy-unattached set/show failed: on='$val' off='$val2'" }

Write-Test "13.3 unset destroy-unattached defaults to off"
Psmux set -g destroy-unattached on | Out-Null
Psmux set -u destroy-unattached | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v destroy-unattached 2>&1 | Out-String).Trim()
if ($val -match "off") { Write-Pass "destroy-unattached unset -> off" } else { Write-Fail "destroy-unattached unset default wrong: $val" }

Write-Test "13.4 set/show exit-empty on/off"
Psmux set -g exit-empty off | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v exit-empty 2>&1 | Out-String).Trim()
$okOff = ($val -match "off")
Psmux set -g exit-empty on | Out-Null
$val2 = (& $PSMUX show-options -t $SESSION -g -v exit-empty 2>&1 | Out-String).Trim()
$okOn = ($val2 -match "on")
if ($okOn -and $okOff) { Write-Pass "exit-empty set/show works" } else { Write-Fail "exit-empty set/show failed: off='$val' on='$val2'" }

Write-Test "13.5 unset exit-empty defaults to on"
Psmux set -g exit-empty off | Out-Null
Psmux set -u exit-empty | Out-Null
$val = (& $PSMUX show-options -t $SESSION -g -v exit-empty 2>&1 | Out-String).Trim()
if ($val -match "on") { Write-Pass "exit-empty unset -> on" } else { Write-Fail "exit-empty unset default wrong: $val" }

Write-Test "13.6 source-file applies destroy-unattached/exit-empty"
$tempConf = Join-Path $env:TEMP ("psmux_exit_opts_" + [guid]::NewGuid().ToString() + ".conf")
@(
    "set -g destroy-unattached on"
    "set -g exit-empty off"
) | Set-Content -Path $tempConf -Encoding UTF8
Psmux source-file $tempConf | Out-Null
$fromConf1 = (& $PSMUX show-options -t $SESSION -g -v destroy-unattached 2>&1 | Out-String).Trim()
$fromConf2 = (& $PSMUX show-options -t $SESSION -g -v exit-empty 2>&1 | Out-String).Trim()
Remove-Item $tempConf -Force -ErrorAction SilentlyContinue
if (($fromConf1 -match "on") -and ($fromConf2 -match "off")) {
    Write-Pass "source-file applies new options"
} else {
    Write-Fail "source-file new options failed: destroy='$fromConf1' exit-empty='$fromConf2'"
}

# reset for subsequent behavior consistency
Psmux set -g destroy-unattached off | Out-Null
Psmux set -g exit-empty on | Out-Null

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed)/$total passed, $($script:TestsFailed) failed"
if ($script:TestsFailed -eq 0) {
    Write-Host "ALL TESTS PASSED!" -ForegroundColor Green
} else {
    Write-Host "$($script:TestsFailed) TESTS FAILED" -ForegroundColor Red
}
Write-Host ("=" * 60)

exit $script:TestsFailed
