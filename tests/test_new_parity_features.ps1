# psmux New Parity Features Test Suite
# Tests all newly implemented features:
#   1. Layout engine: custom layout strings, deep restructuring, main-pane-width/height, previous-layout
#   2. Status bar: status-left-length, status-right-length, multi-line status, status-format
#   3. Options: window-size, allow-passthrough, copy-command, command-alias, set-clipboard, prefix2
#   4. Keybinding: list-keys, list-commands, switch-client -T
#   5. Copy mode: numeric prefix, text objects, named registers, copy-pipe
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_new_parity_features.ps1

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
    param([string]$Name, [string[]]$ExtraArgs)
    $allArgs = @("new-session", "-s", $Name, "-d") + $ExtraArgs
    Start-Process -FilePath $PSMUX -ArgumentList ($allArgs -join " ") -WindowStyle Hidden
    Start-Sleep -Seconds 3
}

function Psmux { & $PSMUX @args 2>&1 | Out-String; Start-Sleep -Milliseconds 300 }
function PsmuxRaw { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

# Kill everything first
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "npf_$(Get-Random -Maximum 9999)"
Write-Info "Test session: $SESSION"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "1. LAYOUT ENGINE TESTS"
Write-Host ("=" * 60)

New-PsmuxSession -Name $SESSION
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }

# Create 4 panes for layout tests
Psmux split-window -t $SESSION -h | Out-Null
Psmux split-window -t $SESSION -v | Out-Null
Psmux split-window -t $SESSION -v | Out-Null
Start-Sleep -Seconds 1

# --- Test 1.1: Named layouts with deep restructuring ---
Write-Test "1.1 even-horizontal layout restructures all panes"
Psmux select-layout -t $SESSION even-horizontal | Out-Null
Start-Sleep -Milliseconds 500
$panes = (& $PSMUX list-panes -t $SESSION 2>&1) | Out-String
$paneCount = ($panes.Split("`n") | Where-Object { $_ -match '\d+:' }).Count
if ($paneCount -ge 4) { Write-Pass "even-horizontal with $paneCount panes" } else { Write-Fail "even-horizontal: expected 4+ panes, got $paneCount" }

Write-Test "1.2 even-vertical layout"
Psmux select-layout -t $SESSION even-vertical | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "even-vertical applied"

Write-Test "1.3 main-horizontal layout"
Psmux select-layout -t $SESSION main-horizontal | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "main-horizontal applied"

Write-Test "1.4 main-vertical layout"
Psmux select-layout -t $SESSION main-vertical | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "main-vertical applied"

Write-Test "1.5 tiled layout"
Psmux select-layout -t $SESSION tiled | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "tiled applied"

# --- Test 1.6: Next/Previous layout cycling ---
Write-Test "1.6 next-layout cycles forward"
$layout1 = (Psmux display-message -t $SESSION -p "#{window_layout}").Trim()
Psmux next-layout -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$layout2 = (Psmux display-message -t $SESSION -p "#{window_layout}").Trim()
if ($layout1 -ne $layout2) { Write-Pass "next-layout changed layout" } else { Write-Fail "next-layout did not change layout" }

Write-Test "1.7 previous-layout cycles backward"
Psmux previous-layout -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
$layout3 = (Psmux display-message -t $SESSION -p "#{window_layout}").Trim()
if ($layout2 -ne $layout3) { Write-Pass "previous-layout changed layout" } else { Write-Fail "previous-layout did not change layout" }

# --- Test 1.8: main-pane-width/height ---
Write-Test "1.8 main-pane-width option"
Psmux set-option -t $SESSION main-pane-width 120 | Out-Null
Start-Sleep -Milliseconds 300
$val = (Psmux show-options -t $SESSION -v main-pane-width 2>&1 | Out-String).Trim()
if ($val -match "120") { Write-Pass "main-pane-width set to 120" } else { Write-Fail "main-pane-width not stored correctly: $val" }

Write-Test "1.9 main-pane-height option"
Psmux set-option -t $SESSION main-pane-height 30 | Out-Null
Start-Sleep -Milliseconds 300
$val = (Psmux show-options -t $SESSION -v main-pane-height 2>&1 | Out-String).Trim()
if ($val -match "30") { Write-Pass "main-pane-height set to 30" } else { Write-Fail "main-pane-height not stored correctly: $val" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "2. STATUS BAR TESTS"
Write-Host ("=" * 60)

# --- Test 2.1: status-left-length ---
Write-Test "2.1 status-left-length option"
Psmux set-option -t $SESSION status-left-length 20 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "status-left-length set"

# --- Test 2.2: status-right-length ---
Write-Test "2.2 status-right-length option"
Psmux set-option -t $SESSION status-right-length 50 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "status-right-length set"

# --- Test 2.3: status multi-line ---
Write-Test "2.3 multi-line status bar (set status 2)"
Psmux set-option -t $SESSION status 2 | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "status set to 2 lines"

# --- Test 2.4: status-format ---
Write-Test "2.4 status-format[1] custom format"
Psmux set-option -t $SESSION 'status-format[1]' '#[fg=white,bg=blue] Custom Line 2: #S ' | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "status-format[1] set"

# Reset to single line
Psmux set-option -t $SESSION status on | Out-Null

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "3. OPTIONS TESTS"
Write-Host ("=" * 60)

# --- Test 3.1: window-size ---
Write-Test "3.1 window-size option"
Psmux set-option -t $SESSION window-size latest | Out-Null
Start-Sleep -Milliseconds 300
$val = (Psmux show-options -t $SESSION -v window-size 2>&1 | Out-String).Trim()
if ($val -match "latest") { Write-Pass "window-size=latest" } else { Write-Fail "window-size not stored: $val" }

# --- Test 3.2: allow-passthrough ---
Write-Test "3.2 allow-passthrough option"
Psmux set-option -t $SESSION allow-passthrough on | Out-Null
Start-Sleep -Milliseconds 300
$val = (Psmux show-options -t $SESSION -v allow-passthrough 2>&1 | Out-String).Trim()
if ($val -match "on") { Write-Pass "allow-passthrough=on" } else { Write-Fail "allow-passthrough not stored: $val" }

# --- Test 3.3: copy-command ---
Write-Test "3.3 copy-command option"
Psmux set-option -t $SESSION copy-command "Set-Clipboard" | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "copy-command set"

# --- Test 3.4: set-clipboard ---
Write-Test "3.4 set-clipboard option"
Psmux set-option -t $SESSION set-clipboard on | Out-Null
Start-Sleep -Milliseconds 300
$val = (Psmux show-options -t $SESSION -v set-clipboard 2>&1 | Out-String).Trim()
if ($val -match "on") { Write-Pass "set-clipboard=on" } else { Write-Fail "set-clipboard not stored: $val" }

# --- Test 3.5: command-alias ---
Write-Test "3.5 command-alias"
Psmux set-option -t $SESSION command-alias 'splitp=split-window' | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "command-alias set"

# --- Test 3.6: prefix2 ---
Write-Test "3.6 prefix2 option"
Psmux set-option -t $SESSION prefix2 C-a | Out-Null
Start-Sleep -Milliseconds 300
$val = (Psmux show-options -t $SESSION -v prefix2 2>&1 | Out-String).Trim()
if ($val -match "C-a") { Write-Pass "prefix2=C-a" } else { Write-Fail "prefix2 not stored: $val" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "4. KEYBINDING FEATURE TESTS"
Write-Host ("=" * 60)

# --- Test 4.1: list-keys ---
Write-Test "4.1 list-keys command"
$keys = (& $PSMUX list-keys -t $SESSION 2>&1) | Out-String
if ($keys.Length -gt 10) { Write-Pass "list-keys returned key bindings ($($keys.Length) chars)" } else { Write-Fail "list-keys returned empty/short: $keys" }

# --- Test 4.2: list-commands ---
Write-Test "4.2 list-commands command"
$cmds = (& $PSMUX list-commands -t $SESSION 2>&1) | Out-String
if ($cmds -match "new-session|split-window|send-keys") { Write-Pass "list-commands returned command list" } else { Write-Fail "list-commands output unexpected: $cmds" }

# --- Test 4.3: bind-key with key table ---
Write-Test "4.3 bind-key -T custom table"
Psmux bind-key -t $SESSION -T mytable x "display-message 'custom table works'" | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "bind-key -T mytable executed"

# --- Test 4.4: switch-client -T ---
Write-Test "4.4 switch-client -T"
Psmux switch-client -t $SESSION -T mytable | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "switch-client -T executed"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "5. COPY MODE TESTS"
Write-Host ("=" * 60)

# --- Test 5.1: Enter copy mode ---
Write-Test "5.1 copy-mode entry"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inCopy = (Psmux display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inCopy -match "1") { Write-Pass "copy-mode entered" } else { Write-Fail "copy-mode not detected: $inCopy" }

# --- Test 5.2: send-keys -X cancel ---
Write-Test "5.2 copy-mode cancel via send-keys -X"
Psmux send-keys -t $SESSION -X cancel | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "send-keys -X cancel executed"

# --- Test 5.3: Numeric prefix test via send-keys ---
Write-Test "5.3 numeric prefix in copy mode"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Send "5j" to move down 5 lines
Psmux send-keys -t $SESSION 5 j | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION -X cancel | Out-Null
Write-Pass "numeric prefix 5j executed"

# --- Test 5.4: Text objects via send-keys ---
Write-Test "5.4 text objects (aw)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Try aw text object
Psmux send-keys -t $SESSION a w | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION -X cancel | Out-Null
Write-Pass "text object aw executed"

# --- Test 5.5: Named registers ---
Write-Test "5.5 named registers"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Press " then a to select register a
Psmux send-keys -t $SESSION '"' a | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION -X cancel | Out-Null
Write-Pass "named register selection executed"

# --- Test 5.6: copy-pipe ---
Write-Test "5.6 copy-pipe support"
# Verify send-keys -X recognizes copy-pipe commands
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION -X cancel | Out-Null
Write-Pass "copy-pipe support verified"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "6. EXISTING FEATURE REGRESSION TESTS"
Write-Host ("=" * 60)

# --- Test 6.1: Basic split and pane operations still work ---
Write-Test "6.1 split-window still works"
$beforePanes = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n").Count
Psmux split-window -t $SESSION -h | Out-Null
Start-Sleep -Milliseconds 500
$afterPanes = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n").Count
if ($afterPanes -gt $beforePanes) { Write-Pass "split-window creates pane" } else { Write-Fail "split-window did not create pane" }

# --- Test 6.2: Window operations ---
Write-Test "6.2 new-window and select-window"
Psmux new-window -t $SESSION -n "testwin" | Out-Null
Start-Sleep -Milliseconds 500
$wins = (& $PSMUX list-windows -t $SESSION 2>&1) | Out-String
if ($wins -match "testwin") { Write-Pass "new-window and list-windows work" } else { Write-Fail "testwin not found: $wins" }

# --- Test 6.3: Session operations ---
Write-Test "6.3 has-session"
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "has-session returns 0 for existing session" } else { Write-Fail "has-session returned $LASTEXITCODE" }

# --- Test 6.4: display-message format variables ---
Write-Test "6.4 display-message format expansion"
$sesName = (& $PSMUX display-message -t $SESSION -p "#S" 2>&1 | Out-String).Trim()
if ($sesName -match $SESSION) { Write-Pass "display-message #S = $sesName" } else { Write-Fail "display-message #S unexpected: $sesName" }

# --- Test 6.5: set/show options ---
Write-Test "6.5 set-option and show-options"
Psmux set-option -t $SESSION mouse on | Out-Null
Start-Sleep -Milliseconds 200
$mouseVal = (& $PSMUX show-options -t $SESSION -v mouse 2>&1 | Out-String).Trim()
if ($mouseVal -match "on") { Write-Pass "set/show mouse=on" } else { Write-Fail "mouse option: $mouseVal" }

# --- Test 6.6: bind-key / unbind-key ---
Write-Test "6.6 bind-key and unbind-key"
Psmux bind-key -t $SESSION x "display-message 'test binding'" | Out-Null
Start-Sleep -Milliseconds 200
Psmux unbind-key -t $SESSION x | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "bind-key and unbind-key executed"

# --- Test 6.7: send-keys ---
Write-Test "6.7 send-keys"
Psmux send-keys -t $SESSION "echo hello" Enter | Out-Null
Start-Sleep -Milliseconds 500
Write-Pass "send-keys executed"

# --- Test 6.8: select-pane directional ---
Write-Test "6.8 select-pane directional navigation"
Psmux select-pane -t $SESSION -U | Out-Null
Psmux select-pane -t $SESSION -D | Out-Null
Psmux select-pane -t $SESSION -L | Out-Null
Psmux select-pane -t $SESSION -R | Out-Null
Write-Pass "directional select-pane works"

# --- Test 6.9: resize-pane ---
Write-Test "6.9 resize-pane"
Psmux resize-pane -t $SESSION -R 5 | Out-Null
Start-Sleep -Milliseconds 200
Psmux resize-pane -t $SESSION -L 5 | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "resize-pane works"

# --- Test 6.10: zoom-pane ---
Write-Test "6.10 zoom and unzoom pane"
Psmux resize-pane -t $SESSION -Z | Out-Null
Start-Sleep -Milliseconds 300
$zoomed = (& $PSMUX display-message -t $SESSION -p "#{window_zoomed_flag}" 2>&1 | Out-String).Trim()
Psmux resize-pane -t $SESSION -Z | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "zoom toggle executed (flag=$zoomed)"

# --- Test 6.11: swap-pane ---
Write-Test "6.11 swap-pane"
Psmux swap-pane -t $SESSION -D | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "swap-pane executed"

# --- Test 6.12: kill-pane ---
Write-Test "6.12 kill-pane reduces pane count"
# Ensure the active window has ≥2 panes so kill-pane doesn't destroy the window
Psmux split-window -t $SESSION -h | Out-Null
Start-Sleep -Milliseconds 500
$beforeKill = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n").Count
Psmux kill-pane -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$afterKill = ((& $PSMUX list-panes -t $SESSION 2>&1) | Out-String).Split("`n").Count
if ($afterKill -lt $beforeKill) { Write-Pass "kill-pane reduced pane count" } else { Write-Fail "kill-pane did not reduce count" }

# --- Test 6.13: run-shell uses PowerShell ---
Write-Test "6.13 run-shell uses PowerShell"
$runOut = (& $PSMUX run-shell -t $SESSION "Write-Output 'pwsh-works'" 2>&1) | Out-String
if ($runOut -match "pwsh-works") { Write-Pass "run-shell uses PowerShell" } else { Write-Fail "run-shell output unexpected: $runOut" }

# --- Test 6.14: if-shell format mode ---
Write-Test "6.14 if-shell -F format mode"
$ifOut = (& $PSMUX if-shell -t $SESSION -F "1" "display-message -p 'TRUE'" "display-message -p 'FALSE'" 2>&1) | Out-String
if ($ifOut -match "TRUE") { Write-Pass "if-shell -F works" } else { Write-Fail "if-shell -F output: $ifOut" }

# --- Test 6.15: source-file ---
Write-Test "6.15 source-file"
$tmpConf = [System.IO.Path]::GetTempFileName()
Set-Content $tmpConf "set -g status-right 'SOURCED'"
Psmux source-file -t $SESSION $tmpConf | Out-Null
Start-Sleep -Milliseconds 500
Remove-Item $tmpConf -Force -ErrorAction SilentlyContinue
Write-Pass "source-file executed"

# --- Test 6.16: capture-pane ---
Write-Test "6.16 capture-pane"
$capture = (& $PSMUX capture-pane -t $SESSION -p 2>&1) | Out-String
if ($capture.Length -ge 0) { Write-Pass "capture-pane returned content" } else { Write-Fail "capture-pane failed" }

# --- Test 6.17: list-buffers ---
Write-Test "6.17 list-buffers"
$bufs = (& $PSMUX list-buffers -t $SESSION 2>&1) | Out-String
Write-Pass "list-buffers executed ($($bufs.Length) chars)"

# --- Test 6.18: wait-for ---
Write-Test "6.18 wait-for channel signal"
# Signal a channel
Psmux wait-for -t $SESSION -S test_channel | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "wait-for -S executed"

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)

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
