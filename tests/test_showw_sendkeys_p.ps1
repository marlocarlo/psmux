# Focused regression tests for:
# 1) show-window-options scoping and inherited lookup (-A)
# 2) send-keys -p compatibility mode
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File tests\test_showw_sendkeys_p.ps1

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
if (-not $PSMUX) {
    $cmd = Get-Command psmux -ErrorAction SilentlyContinue
    if ($cmd) { $PSMUX = $cmd.Source }
}
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 250 }

$SESSION = "swp_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"

Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 2

Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 2
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Could not create session"
    exit 1
}

Write-Test "show-window-options returns window-scoped keys"
$vals = Psmux show-window-options -t $SESSION | Out-String
if ($vals -match "window-size|window-status-format|automatic-rename") { Write-Pass "window options listed" }
else { Write-Fail "missing expected window options" }

Write-Test "show-window-options -v session key returns empty"
$v = (Psmux show-window-options -v prefix -t $SESSION | Out-String).Trim()
if ($v -eq "") {
    Write-Pass "session key excluded from window scope"
} else {
    Write-Fail "expected empty, got '$v'"
}

Write-Test "show-window-options -A -v session key falls back"
$v = (Psmux show-window-options -A -v prefix -t $SESSION | Out-String).Trim()
if ($v -match "C-b|C-a") { Write-Pass "-A fallback returned '$v'" }
else { Write-Fail "-A fallback failed, got '$v'" }

Write-Test "show-options -w -v window-size returns value"
$v = (Psmux show-options -w -v window-size -t $SESSION | Out-String).Trim()
if ($v -ne "") { Write-Pass "show-options -w returned '$v'" }
else { Write-Fail "show-options -w returned empty" }

Write-Test "send-keys -p sends literal paste text"
Psmux send-keys -t $SESSION -p "paste_mode_probe_123" | Out-Null
Psmux send-keys -t $SESSION Enter | Out-Null
Start-Sleep -Milliseconds 500
$cap = Psmux capture-pane -t $SESSION -p | Out-String
if ($cap -match "paste_mode_probe_123") { Write-Pass "paste text observed in pane" }
else { Write-Fail "paste text not found in pane capture" }

# Cleanup
& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host "========================================" -ForegroundColor White
Write-Host "Focused Regression Summary" -ForegroundColor White
Write-Host "========================================" -ForegroundColor White
Write-Host "Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Skipped: $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host "Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "========================================" -ForegroundColor White

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
