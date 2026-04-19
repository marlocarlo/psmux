# test_kill_tree.ps1 - Verify kill-pane/window/session kill entire process trees
$ErrorActionPreference = "Continue"
$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
$PASS = 0; $FAIL = 0

function Cleanup {
    Get-Process -Name PING -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    & $PSMUX kill-session -t kt1 2>$null
    & $PSMUX kill-session -t kt2 2>$null
    & $PSMUX kill-session -t kt3 2>$null
    & $PSMUX kill-session -t kt4 2>$null
    Start-Sleep -Seconds 1
}

Cleanup

# ── TEST 1: kill-pane ────────────────────────────────────────────────────
Write-Host "[TEST] kill-pane kills subprocess tree"
& $PSMUX new-session -d -s kt1
Start-Sleep -Seconds 2
& $PSMUX send-keys -t kt1 'ping -n 999999 127.0.0.1' Enter
Start-Sleep -Seconds 2
$before = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  before: $before ping(s)"
& $PSMUX kill-pane -t kt1
Start-Sleep -Seconds 3
$after = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  after: $after ping(s)"
if ($after -eq 0) { Write-Host "[PASS] kill-pane kills subprocess tree"; $PASS++ }
else { Write-Host "[FAIL] kill-pane - $after ping(s) still running"; $FAIL++ }
Cleanup

# ── TEST 2: kill-window ─────────────────────────────────────────────────
Write-Host "[TEST] kill-window kills all pane subprocesses"
& $PSMUX new-session -d -s kt2
Start-Sleep -Seconds 2
& $PSMUX send-keys -t kt2 'ping -n 999999 127.0.0.1' Enter
Start-Sleep -Seconds 1
& $PSMUX split-window -v -t kt2
Start-Sleep -Seconds 2
& $PSMUX send-keys -t kt2 'ping -n 999999 127.0.0.2' Enter
Start-Sleep -Seconds 2
$before = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  before: $before ping(s)"
& $PSMUX kill-window -t kt2
Start-Sleep -Seconds 4
$after = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  after: $after ping(s)"
if ($after -eq 0) { Write-Host "[PASS] kill-window kills all pane subprocesses"; $PASS++ }
else { Write-Host "[FAIL] kill-window - $after ping(s) still running"; $FAIL++ }
Cleanup

# ── TEST 3: kill-session ────────────────────────────────────────────────
Write-Host "[TEST] kill-session kills all child processes"
& $PSMUX new-session -d -s kt3
Start-Sleep -Seconds 2
& $PSMUX send-keys -t kt3 'ping -n 999999 127.0.0.3' Enter
Start-Sleep -Seconds 2
$before = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  before: $before ping(s)"
& $PSMUX kill-session -t kt3
Start-Sleep -Seconds 3
$after = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  after: $after ping(s)"
if ($after -eq 0) { Write-Host "[PASS] kill-session kills all child processes"; $PASS++ }
else { Write-Host "[FAIL] kill-session - $after ping(s) still running"; $FAIL++ }
Cleanup

# ── TEST 4: nested tree ─────────────────────────────────────────────────
Write-Host "[TEST] kill-pane with nested cmd-to-ping tree"
& $PSMUX new-session -d -s kt4
Start-Sleep -Seconds 2
& $PSMUX send-keys -t kt4 'cmd /c "ping -n 999999 127.0.0.4"' Enter
Start-Sleep -Seconds 2
$before = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  before: $before ping(s)"
& $PSMUX kill-pane -t kt4
Start-Sleep -Seconds 3
$after = @(Get-Process -Name PING -ErrorAction SilentlyContinue).Count
Write-Host "  after: $after ping(s)"
if ($after -eq 0) { Write-Host "[PASS] kill-pane with nested cmd-to-ping tree"; $PASS++ }
else { Write-Host "[FAIL] nested kill-pane - $after ping(s) still running"; $FAIL++ }
Cleanup

# ── SUMMARY ──────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  PROCESS TREE KILL TEST SUMMARY"
Write-Host ("=" * 60)
Write-Host "  Passed: $PASS"
Write-Host "  Failed: $FAIL"
Write-Host "  Total:  $($PASS + $FAIL)"
Write-Host ("=" * 60)
if ($FAIL -gt 0) { exit 1 }
