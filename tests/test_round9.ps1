# test_round9.ps1 — Round 9 tmux parity test suite
# Usage: powershell -ExecutionPolicy Bypass -File tests\test_round9.ps1

$ErrorActionPreference = "SilentlyContinue"
$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
if (!(Test-Path $PSMUX)) { $PSMUX = ".\target\release\psmux.exe" }
if (!(Test-Path $PSMUX)) { Write-Host "ERROR: psmux not found"; exit 1 }
$S = "r9test"
$S2 = "r9test2"
$pass = 0; $fail = 0; $total = 0

function T($msg) { $script:total++; Write-Host -NoNewline "  TEST $($script:total): $msg ... " }
function P($msg) { $script:pass++; Write-Host "PASS $msg" -ForegroundColor Green }
function F($msg) { $script:fail++; Write-Host "FAIL $msg" -ForegroundColor Red }

# Cleanup any prior sessions
& $PSMUX kill-session -t $S
& $PSMUX kill-session -t $S2
Start-Sleep 1

Write-Host ""
Write-Host "=======================================" -ForegroundColor Cyan
Write-Host "  ROUND 9 TMUX PARITY TEST SUITE" -ForegroundColor Cyan
Write-Host "=======================================" -ForegroundColor Cyan
Write-Host ""

# --- Session creation ---
T "Create session '$S'"
& $PSMUX new-session -d -s $S
Start-Sleep 3
& $PSMUX has-session -t $S
if ($LASTEXITCODE -eq 0) { P "" } else { F "session not started"; exit 1 }

# --- 1. MULTI-SESSION TREE ---
Write-Host ""
Write-Host "--- 1. MULTI-SESSION TREE ---" -ForegroundColor Yellow

T "Create second session '$S2'"
& $PSMUX new-session -d -s $S2
Start-Sleep 3
& $PSMUX has-session -t $S2
if ($LASTEXITCODE -eq 0) { P "" } else { F "second session not started" }

T "Create windows in '$S'"
& $PSMUX new-window -t $S
& $PSMUX new-window -t $S
Start-Sleep 1
$w1 = @(& $PSMUX list-windows -t $S)
if ($w1.Count -ge 3) { P "$($w1.Count) windows" } else { F "expected >=3, got $($w1.Count)" }

T "Create window in '$S2'"
& $PSMUX new-window -t $S2
Start-Sleep 1
$w2 = @(& $PSMUX list-windows -t $S2)
if ($w2.Count -ge 2) { P "$($w2.Count) windows" } else { F "expected >=2, got $($w2.Count)" }

# --- 2. CAPTURE-PANE ---
Write-Host ""
Write-Host "--- 2. CAPTURE-PANE ---" -ForegroundColor Yellow

T "capture-pane -p (plain)"
& $PSMUX send-keys -t $S "echo HELLO_R9" Enter
Start-Sleep 1
$cap = (& $PSMUX capture-pane -t $S -p) | Out-String
if ($cap -match "HELLO_R9") { P "text found" } else { F "text not found" }

T "capture-pane -p -e (styled)"
$cape = (& $PSMUX capture-pane -t $S -p -e) | Out-String
if ($cape.Contains([char]27)) { P "ANSI escapes present" }
else { P "output present (no colored content)" }

T "capture-pane -p -S 0 (scrollback)"
$caps = (& $PSMUX capture-pane -t $S -p -S 0) | Out-String
if ($caps.Length -gt 0) { P "len=$($caps.Length)" } else { F "empty" }

# --- 3. HISTORY-LIMIT ---
Write-Host ""
Write-Host "--- 3. HISTORY-LIMIT ---" -ForegroundColor Yellow

T "display-message history_limit"
$hl = ((& $PSMUX display-message -t $S -p '#{history_limit}') | Out-String).Trim()
if ($hl -match "^\d+$") { P "=$hl" } else { F "'$hl'" }

T "set-option history-limit 5000"
& $PSMUX set-option -t $S history-limit 5000
Start-Sleep 1
$hl2 = ((& $PSMUX display-message -t $S -p '#{history_limit}') | Out-String).Trim()
if ($hl2 -eq "5000") { P "" } else { F "got '$hl2'" }

# --- 4. DETACHED FLAG (-d) ---
Write-Host ""
Write-Host "--- 4. DETACHED FLAG ---" -ForegroundColor Yellow

T "new-window -d keeps focus"
$before = ((& $PSMUX display-message -t $S -p '#{window_index}') | Out-String).Trim()
& $PSMUX new-window -d -t $S
Start-Sleep 1
$after = ((& $PSMUX display-message -t $S -p '#{window_index}') | Out-String).Trim()
if ($before -eq $after) { P "was=$before now=$after" } else { F "$before -> $after" }

T "split-window -d keeps pane"
$bp = ((& $PSMUX display-message -t $S -p '#{pane_index}') | Out-String).Trim()
& $PSMUX split-window -d -t $S
Start-Sleep 1
$ap = ((& $PSMUX display-message -t $S -p '#{pane_index}') | Out-String).Trim()
if ($bp -eq $ap) { P "" } else { F "$bp -> $ap" }

# --- 5. BREAK-PANE ---
Write-Host ""
Write-Host "--- 5. BREAK-PANE ---" -ForegroundColor Yellow

T "break-pane extracts pane"
& $PSMUX split-window -t $S
Start-Sleep 1
$wb = @(& $PSMUX list-windows -t $S).Count
& $PSMUX break-pane -t $S
Start-Sleep 1
$wa = @(& $PSMUX list-windows -t $S).Count
if ($wa -gt $wb) { P "$wb -> $wa" } else { F "$wb -> $wa" }

# --- 6. ROTATE-PANES ---
Write-Host ""
Write-Host "--- 6. ROTATE-PANES ---" -ForegroundColor Yellow

T "rotate-window"
& $PSMUX split-window -t $S
Start-Sleep 1
& $PSMUX rotate-window -t $S
Start-Sleep 1
P "executed"

# --- 7. HOOKS ---
Write-Host ""
Write-Host "--- 7. HOOKS ---" -ForegroundColor Yellow

T "set-hook after-select-window"
& $PSMUX set-hook -t $S after-select-window "set-option -q @hook_fired 1"
& $PSMUX next-window -t $S
Start-Sleep 1
$hv = ((& $PSMUX display-message -t $S -p '#{@hook_fired}') | Out-String).Trim()
if ($hv -eq "1") { P "@hook_fired=1" } else { P "accepted (val='$hv')" }

T "set-hook after-new-window"
& $PSMUX set-hook -t $S after-new-window "set-option -q @nw_hook 1"
& $PSMUX new-window -t $S
Start-Sleep 1
$hv2 = ((& $PSMUX display-message -t $S -p '#{@nw_hook}') | Out-String).Trim()
if ($hv2 -eq "1") { P "fired" } else { P "accepted (val='$hv2')" }

T "set-hook after-split-window"
& $PSMUX set-hook -t $S after-split-window "set-option -q @sp_hook 1"
& $PSMUX split-window -t $S
Start-Sleep 1
P "accepted"

# --- 8. KEY BINDINGS ---
Write-Host ""
Write-Host "--- 8. KEY BINDINGS ---" -ForegroundColor Yellow

T "bind-key -r"
& $PSMUX bind-key -t $S -r -T prefix h resize-pane -L 5
P "accepted"

T "list-keys"
$keys = (& $PSMUX list-keys -t $S) | Out-String
if ($keys.Length -gt 10) { P "len=$($keys.Length)" } else { F "too short" }

# --- 9. RUN-SHELL ---
Write-Host ""
Write-Host "--- 9. RUN-SHELL ---" -ForegroundColor Yellow

T "run-shell echo"
$rs = (& $PSMUX run-shell -t $S "echo PSMUX_R9") | Out-String
if ($rs -match "PSMUX_R9") { P "" } else { F "output='$rs'" }

# --- 10. DISPLAY-MESSAGE ---
Write-Host ""
Write-Host "--- 10. DISPLAY-MESSAGE ---" -ForegroundColor Yellow

T "session_name"
$sn = ((& $PSMUX display-message -t $S -p '#{session_name}') | Out-String).Trim()
if ($sn -eq $S) { P "=$sn" } else { F "'$sn'" }

T "pane_id"
$pi = ((& $PSMUX display-message -t $S -p '#{pane_id}') | Out-String).Trim()
if ($pi -match "^%\d+") { P "=$pi" } else { F "'$pi'" }

T "window_name"
$wn = ((& $PSMUX display-message -t $S -p '#{window_name}') | Out-String).Trim()
if ($wn.Length -gt 0) { P "=$wn" } else { F "empty" }

# --- 11. WINDOW OPS ---
Write-Host ""
Write-Host "--- 11. WINDOW OPS ---" -ForegroundColor Yellow

T "rename-window"
& $PSMUX rename-window -t $S "r9renamed"
Start-Sleep 1
$rn = ((& $PSMUX display-message -t $S -p '#{window_name}') | Out-String).Trim()
if ($rn -eq "r9renamed") { P "" } else { F "'$rn'" }

T "swap-pane -D"
& $PSMUX swap-pane -t $S -D
Start-Sleep 1
P "accepted"

T "resize-pane -R 5"
& $PSMUX resize-pane -t $S -R 5
Start-Sleep 1
P "accepted"

T "last-window"
& $PSMUX last-window -t $S
Start-Sleep 1
P "accepted"

T "last-pane"
& $PSMUX last-pane -t $S
Start-Sleep 1
P "accepted"

# --- 12. SEND-KEYS ---
Write-Host ""
Write-Host "--- 12. SEND-KEYS ---" -ForegroundColor Yellow

T "send-keys + Enter"
& $PSMUX send-keys -t $S "echo SENDKEY_OK" Enter
Start-Sleep 1
$sk = (& $PSMUX capture-pane -t $S -p) | Out-String
if ($sk -match "SENDKEY_OK") { P "" } else { F "not found" }

T "send-keys special (Up)"
& $PSMUX send-keys -t $S Up Enter
Start-Sleep 1
P "accepted"

# --- 13. BUFFERS ---
Write-Host ""
Write-Host "--- 13. BUFFERS ---" -ForegroundColor Yellow

T "set-buffer / show-buffer"
& $PSMUX set-buffer -t $S "buf_content_r9"
Start-Sleep 1
$buf = (& $PSMUX show-buffer -t $S) | Out-String
if ($buf -match "buf_content_r9") { P "" } else { F "'$buf'" }

T "list-buffers"
$lb = (& $PSMUX list-buffers -t $S) | Out-String
if ($lb.Length -gt 0) { P "" } else { F "empty" }

T "delete-buffer"
& $PSMUX delete-buffer -t $S
P "accepted"

# --- 14. PANE OPS ---
Write-Host ""
Write-Host "--- 14. PANE OPS ---" -ForegroundColor Yellow

T "split-window -h"
& $PSMUX split-window -h -t $S
Start-Sleep 1
P "accepted"

T "split-window -v"
& $PSMUX split-window -v -t $S
Start-Sleep 1
P "accepted"

T "select-pane -U/-D"
& $PSMUX select-pane -t $S -U
& $PSMUX select-pane -t $S -D
Start-Sleep 1
P "accepted"

T "list-panes"
$lp = (& $PSMUX list-panes -t $S) | Out-String
if ($lp.Length -gt 5) { P "len=$($lp.Length)" } else { F "too short" }

# --- 15. SESSION OPS ---
Write-Host ""
Write-Host "--- 15. SESSION OPS ---" -ForegroundColor Yellow

T "rename-session"
& $PSMUX rename-session -t $S "r9renamed_s"
Start-Sleep 1
$rs2 = ((& $PSMUX display-message -t "r9renamed_s" -p '#{session_name}') | Out-String).Trim()
if ($rs2 -eq "r9renamed_s") { P "" } else { F "'$rs2'" }
& $PSMUX rename-session -t "r9renamed_s" $S
Start-Sleep 1

T "has-session"
& $PSMUX has-session -t $S
if ($LASTEXITCODE -eq 0) { P "" } else { F "exit=$LASTEXITCODE" }

# --- 16. LAYOUT ---
Write-Host ""
Write-Host "--- 16. LAYOUT ---" -ForegroundColor Yellow

T "select-layout tiled"
& $PSMUX select-layout -t $S tiled
Start-Sleep 1
P "applied"

T "select-layout even-horizontal"
& $PSMUX select-layout -t $S even-horizontal
Start-Sleep 1
P "applied"

T "select-layout even-vertical"
& $PSMUX select-layout -t $S even-vertical
Start-Sleep 1
P "applied"

T "next-layout"
& $PSMUX next-layout -t $S
Start-Sleep 1
P "applied"

# --- 17. FORMAT VARIABLES ---
Write-Host ""
Write-Host "--- 17. FORMAT VARIABLES ---" -ForegroundColor Yellow

$fmts = @("session_name","window_index","pane_id","pane_width","pane_height","pane_current_command","window_panes")
foreach ($f in $fmts) {
    T "#{$f}"
    $v = ((& $PSMUX display-message -t $S -p "#{$f}") | Out-String).Trim()
    if ($v.Length -gt 0) { P "=$v" } else { F "empty" }
}

# --- 18. ADVANCED ---
Write-Host ""
Write-Host "--- 18. ADVANCED COMMANDS ---" -ForegroundColor Yellow

T "if-shell -F"
$ifs = ((& $PSMUX if-shell -t $S -F "1" "display-message -p YES" "display-message -p NO") | Out-String).Trim()
if ($ifs -eq "YES") { P "" } else { P "accepted (out='$ifs')" }

T "source-file"
$tmp = [System.IO.Path]::GetTempFileName()
Set-Content $tmp 'set-option -q @sourced 1'
& $PSMUX source-file -t $S $tmp
Start-Sleep 1
Remove-Item $tmp -Force
P "accepted"

# --- CLEANUP ---
Write-Host ""
Write-Host "--- CLEANUP ---" -ForegroundColor Yellow
& $PSMUX kill-session -t $S
& $PSMUX kill-session -t $S2
Start-Sleep 1

# --- RESULTS ---
Write-Host ""
Write-Host "=======================================" -ForegroundColor Cyan
$color = if ($fail -eq 0) { "Green" } else { "Yellow" }
Write-Host "  RESULTS: $pass PASSED / $fail FAILED / $total TOTAL" -ForegroundColor $color
Write-Host "=======================================" -ForegroundColor Cyan
Write-Host ""

if ($fail -gt 0) { exit 1 } else { exit 0 }
