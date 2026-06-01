$ErrorActionPreference = "Continue"
$PSMUX = $env:PSMUX_EXE
if (-not $PSMUX) {
    $cmd = Get-Command psmux -EA Stop
    $PSMUX = if ($cmd.Path) { $cmd.Path } elseif ($cmd.Source) { $cmd.Source } else { $cmd.Definition }
}
if (-not $PSMUX) {
    Write-Host "FATAL: could not resolve psmux executable path" -ForegroundColor Red
    exit 1
}
$SESSION = "smoke_pr"
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
}

function Wait-Session([string]$Name, [int]$TimeoutMs = 10000) {
    $pf = "$psmuxDir\$Name.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw -EA SilentlyContinue).Trim()
            if ($port -match '^\d+$') { return $true }
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Wait-PaneContent([string]$Target, [string]$Pattern, [int]$TimeoutMs = 8000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
        if ($cap -match $Pattern) { return $true }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

Write-Host "`n=== PR Smoke Tests ===" -ForegroundColor Cyan

# Static safety-gate guard for the destructive comprehensive runner (#342).
# It only reads run_all_tests.ps1 — no psmux binary or session needed — so a
# removed or moved PSMUX_TEST_SANDBOX gate fails the PR build here too. Run it
# first; surface its per-check output only when it fails.
$gateGuard = Join-Path $PSScriptRoot "test_runner_safety_gate.ps1"
$guardOut = & pwsh -NoProfile -ExecutionPolicy Bypass -File $gateGuard 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Pass "run_all_tests.ps1 sandbox gate intact (static guard)"
} else {
    Write-Fail "run_all_tests.ps1 sandbox gate guard failed (exit $LASTEXITCODE)"
    $guardOut | ForEach-Object { Write-Host "    $_" }
}

Cleanup
& $PSMUX new-session -d -s $SESSION -x 120 -y 30 | Out-Null
if (-not (Wait-Session $SESSION)) {
    Write-Host "FATAL: failed to create smoke session" -ForegroundColor Red
    exit 1
}

# Test 1: basic session liveness
$ls = (& $PSMUX ls 2>&1) -join "`n"
if ($ls -match [regex]::Escape($SESSION)) {
    Write-Pass "Session created and visible in list-sessions"
} else {
    Write-Fail "Session missing from list-sessions"
}

# Test 2: basic pane ops
& $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$panes = & $PSMUX list-panes -t $SESSION 2>&1
$paneCount = ($panes | Measure-Object -Line).Lines
if ($paneCount -ge 2) {
    Write-Pass "split-window created a second pane"
} else {
    Write-Fail "split-window did not create expected pane count"
}

# Test 3: Ctrl+C line cancel behavior (#338)
$markerFile = Join-Path $env:TEMP "psmux_smoke_marker_$([guid]::NewGuid().ToString('N')).txt"
if (Test-Path $markerFile) { Remove-Item $markerFile -Force -EA SilentlyContinue }
$pendingCmd = "New-Item -ItemType File -Path '$markerFile' -Force | Out-Null"
& $PSMUX send-keys -t "$SESSION:0.0" $pendingCmd 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t "$SESSION:0.0" C-c 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t "$SESSION:0.0" Enter 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
if (-not (Test-Path $markerFile)) {
    Write-Pass "Ctrl+C cancels current prompt line"
} else {
    Write-Fail "Canceled prompt line executed unexpectedly"
    Remove-Item $markerFile -Force -EA SilentlyContinue
}

# Test 4: Ctrl+C interrupts running command
& $PSMUX send-keys -t "$SESSION:0.0" "ping -t 127.0.0.1" Enter 2>&1 | Out-Null
if (Wait-PaneContent "$SESSION:0.0" "Reply from" 10000) {
    & $PSMUX send-keys -t "$SESSION:0.0" C-c 2>&1 | Out-Null
    if (Wait-PaneContent "$SESSION:0.0" "Control-C|Ping statistics|PS [A-Z]:\\|[A-Z]:\\.*>" 8000) {
        Write-Pass "Ctrl+C interrupts running process"
    } else {
        Write-Fail "Ctrl+C did not interrupt running process"
    }
} else {
    Write-Fail "Could not start ping for interrupt smoke test"
}

Cleanup

Write-Host "`n=== Smoke Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
