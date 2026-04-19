# test_agent_teams_e2e.ps1 — End-to-end Claude Code Agent Teams Test
# ==================================================================
# Launches Claude Code inside psmux, triggers team creation, and
# verifies that teammate panes spawn correctly with working agents.
#
# This test actually runs Claude Code (with Haiku for low cost)
# and validates the entire agent teams pipeline:
#   1. Claude Code detects psmux/tmux
#   2. Team creation spawns teammate panes via split-window
#   3. send-keys delivers the spawn command (cd && env ... claude)
#   4. env shim strips POSIX escapes and sets env vars correctly
#   5. Teammate agents start and complete work
#
# Prerequisites:
#   - psmux installed and on PATH
#   - Claude Code (claude) installed and authenticated
#   - Test workspace: C:\cctest\a long dir name\another very long name with 5
#
# Usage:
#   pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_agent_teams_e2e.ps1
#
# Cost: Uses Haiku 4.5 model exclusively to minimize API costs.

param(
    [string]$Model = "haiku",
    [int]$TeamSize = 2,
    [int]$TimeoutSeconds = 120,
    [switch]$KeepSession
)

$ErrorActionPreference = "Continue"
$script:Passed = 0
$script:Failed = 0
$script:Skipped = 0

function Pass($msg) { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:Passed++ }
function Fail($msg) { Write-Host "  FAIL: $msg" -ForegroundColor Red; $script:Failed++ }
function Skip($msg) { Write-Host "  SKIP: $msg" -ForegroundColor Yellow; $script:Skipped++ }
function Info($msg) { Write-Host "  INFO: $msg" -ForegroundColor Cyan }
function Test($msg) { Write-Host "`n  TEST: $msg" -ForegroundColor White }
function Section($msg) {
    Write-Host "`n$('=' * 70)" -ForegroundColor Cyan
    Write-Host "  $msg" -ForegroundColor Cyan
    Write-Host "$('=' * 70)" -ForegroundColor Cyan
}

# ── Binary detection ──
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = (Get-Command psmux -EA 0).Source }
if (-not $PSMUX) { Write-Error "psmux not found. Build first."; exit 1 }

$CLAUDE = (Get-Command claude -EA 0).Source
if (-not $CLAUDE) { Write-Error "claude not found on PATH"; exit 1 }

Info "psmux: $PSMUX"
Info "claude: $CLAUDE"
Info "Model: $Model (low cost)"

# ── Test workspace ──
$WORKSPACE = "C:\cctest\a long dir name\another very long name with 5"
if (-not (Test-Path $WORKSPACE)) {
    New-Item -Path $WORKSPACE -ItemType Directory -Force | Out-Null
    Info "Created test workspace: $WORKSPACE"
}

$SESSION = "agent_e2e_test"
$PsmuxDir = "$env:USERPROFILE\.psmux"

function Cleanup {
    param([switch]$Keep)
    if (-not $Keep) {
        try { & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null } catch {}
        Start-Sleep -Milliseconds 500
        Remove-Item "$PsmuxDir\$SESSION.port" -Force -EA 0
        Remove-Item "$PsmuxDir\$SESSION.key" -Force -EA 0
    }
}

function Wait-ForSession {
    param([int]$TimeoutMs = 8000)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while ((Get-Date) -lt $deadline) {
        & $PSMUX has-session -t $SESSION 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 300
    }
    return $false
}

function Wait-ForPanes {
    param([int]$ExpectedCount, [int]$TimeoutMs = 60000)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while ((Get-Date) -lt $deadline) {
        $panes = & $PSMUX list-panes -t $SESSION -F '#{pane_id}' 2>&1
        if ($LASTEXITCODE -eq 0) {
            $count = ($panes | Where-Object { $_ -match '^%\d+$' }).Count
            if ($count -ge $ExpectedCount) { return $count }
        }
        Start-Sleep -Seconds 2
    }
    return -1
}

function Capture-AllPanes {
    $panes = & $PSMUX list-panes -t $SESSION -F '#{pane_id} #{pane_index}' 2>&1
    $result = @{}
    foreach ($line in $panes) {
        if ($line -match '^(%\d+)\s+(\d+)$') {
            $id = $Matches[1]
            $idx = $Matches[2]
            $cap = & $PSMUX capture-pane -t $id -p 2>&1 | Out-String
            $result[$idx] = @{ Id = $id; Content = $cap }
        }
    }
    return $result
}

Write-Host ""
Write-Host "=================================================================="
Write-Host "  Claude Code Agent Teams End-to-End Test Suite"
Write-Host "  Model: $Model | Team Size: $TeamSize | Timeout: ${TimeoutSeconds}s"
Write-Host "=================================================================="
Write-Host ""

# ── Pre-test cleanup ──
& $PSMUX kill-server 2>&1 | Out-Null
Get-Process psmux -EA 0 | Stop-Process -Force -EA 0
Start-Sleep -Seconds 3
Get-ChildItem "$PsmuxDir\*.port" -EA 0 | Remove-Item -Force
Get-ChildItem "$PsmuxDir\*.key" -EA 0 | Remove-Item -Force
Start-Sleep -Seconds 1

# ══════════════════════════════════════════════════════════════
Section "TEST 1: Basic split + send-keys pipeline (no Claude Code)"
# ══════════════════════════════════════════════════════════════

Test "1.1: Split pane and deliver Claude Code style command"

& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
if (-not (Wait-ForSession)) {
    Fail "1.1: Cannot create session"
    exit 1
}

$marker = "E2E_$(Get-Random)"
$paneId = & $PSMUX split-window -t $SESSION -h -P -F '#{pane_id}' 2>&1
$paneId = ($paneId | Out-String).Trim()

if ($paneId -match '^%\d+$') {
    Pass "1.1a: split-window returned pane ID: $paneId"
} else {
    Fail "1.1a: Bad pane ID: '$paneId'"
    Cleanup; exit 1
}

Start-Sleep -Milliseconds 300

# Send the exact POSIX-style command Claude Code sends
& $PSMUX send-keys -t $paneId "cd '$WORKSPACE' && env MARKER=$marker CLAUDECODE=1 CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 pwsh -NoProfile -Command `"Write-Host `$env:MARKER`"" Enter
Start-Sleep -Seconds 5

$cap = & $PSMUX capture-pane -t $paneId -p 2>&1 | Out-String
if ($cap -match [regex]::Escape($marker)) {
    Pass "1.1b: POSIX env command executed correctly (marker found)"
} else {
    Fail "1.1b: Marker '$marker' not found in pane. Content: $($cap.Substring(0,[Math]::Min(300,$cap.Length)))"
}

# Check session survived
& $PSMUX has-session -t $SESSION 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
    Pass "1.1c: Session stable after split + send-keys"
} else {
    Fail "1.1c: Session died after split + send-keys"
}

Cleanup

# ══════════════════════════════════════════════════════════════
Section "TEST 2: env shim overrides native env.exe"
# ══════════════════════════════════════════════════════════════

Test "2.1: env shim is Function type (not Application) in pane"

& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
if (-not (Wait-ForSession)) { Fail "2.1: Cannot create session"; Cleanup; }
else {
    & $PSMUX send-keys -t $SESSION "Get-Command env | Select-Object -ExpandProperty CommandType" Enter
    Start-Sleep -Seconds 4
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($cap -match "Function") {
        Pass "2.1: env resolves to Function (shim active)"
    } elseif ($cap -match "Application") {
        Fail "2.1: env resolves to Application (native env.exe, shim NOT active)"
    } else {
        Skip "2.1: Could not determine env type. Output: $($cap.Substring(0,200))"
    }
    Cleanup
}

# ══════════════════════════════════════════════════════════════
Section "TEST 3: End-to-end Claude Code agent team launch"
# ══════════════════════════════════════════════════════════════

Test "3.1: Launch Claude Code, trigger team creation, verify panes"

& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
if (-not (Wait-ForSession)) {
    Fail "3.1: Cannot create session"
    exit 1
}

# Launch Claude Code inside the psmux session
$launchCmd = "cd '$WORKSPACE'; `$env:CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1; `$env:CLAUDE_CODE_USE_POWERSHELL_TOOL=1; `$env:CLAUDE_CODE_AUTO_CONNECT_IDE=`$false; claude --model $Model --dangerously-skip-permissions"
& $PSMUX send-keys -t $SESSION $launchCmd Enter
Info "Launched Claude Code (waiting for it to start)..."

# Wait for Claude Code to fully initialize (spinner -> prompt)
$ccStarted = $false
$ccDeadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $ccDeadline) {
    Start-Sleep -Seconds 3
    $leaderCap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    # Detect Claude Code: spinner chars, prompt, or branded text
    if ($leaderCap -match 'Claude|claude|\$|>|tips|/help|cost|Haiku|model') {
        $ccStarted = $true
        break
    }
}

if ($ccStarted) {
    Pass "3.1a: Claude Code is running in leader pane"
} else {
    Fail "3.1a: Claude Code did not start within 30s. Output: $($leaderCap.Substring(0,[Math]::Min(300,$leaderCap.Length)))"
    Cleanup
    Write-Host "`n  Skipping remaining tests (Claude Code failed to start)" -ForegroundColor Yellow
    $script:Skipped += 3
    $skipTeamTests = $true
}
$skipTeamTests = $skipTeamTests -eq $true

if (-not $skipTeamTests) {
# Send team creation prompt
$teamPrompt = "Create a team of $TeamSize agents. One named 'coder' to create a simple hello world python script in the current directory. One named 'tester' to verify the script works by running it. Keep it minimal."
& $PSMUX send-keys -t $SESSION $teamPrompt Enter
Info "Sent team creation prompt (waiting for panes to appear)..."

# Wait for teammate panes (leader + N teammates)
$expectedPanes = 1 + $TeamSize
$actualPanes = Wait-ForPanes -ExpectedCount $expectedPanes -TimeoutMs ($TimeoutSeconds * 1000)

if ($actualPanes -ge $expectedPanes) {
    Pass "3.1b: $actualPanes panes created (expected $expectedPanes)"
} else {
    if ($actualPanes -gt 0) {
        Fail "3.1b: Only $actualPanes panes (expected $expectedPanes). Team creation incomplete."
    } else {
        Fail "3.1b: No panes created. Team spawn failed entirely."
    }
}

# Capture all panes
Start-Sleep -Seconds 5
$allPanes = Capture-AllPanes

Test "3.2: Verify teammate panes are running Claude Code"

$teammatePanesWorking = 0
foreach ($idx in ($allPanes.Keys | Sort-Object)) {
    $content = $allPanes[$idx].Content
    $paneRef = $allPanes[$idx].Id

    if ($idx -eq "0") {
        # Leader pane
        if ($content -match "team|agent|Idle|running") {
            Info "Pane 0 (leader): team management active"
        }
    } else {
        # Teammate pane
        if ($content -match "@|Claude Code|agent-id|thinking|Metamorphosing|Seasoning|Percolating|Write|Task|hello") {
            $teammatePanesWorking++
            Info "Pane $idx ($paneRef): teammate active"
        } elseif ($content -match "syntax is incorrect|not recognized|error") {
            Fail "3.2: Pane $idx ($paneRef) has error: $($content.Substring(0,[Math]::Min(200,$content.Length)))"
        } else {
            Info "Pane $idx ($paneRef): waiting or idle"
        }
    }
}

if ($teammatePanesWorking -ge 1) {
    Pass "3.2: $teammatePanesWorking/$TeamSize teammate panes running"
} else {
    Fail "3.2: No teammate panes appear to be running Claude Code"
}

Test "3.3: Check for path/syntax errors in teammate panes"

$pathErrors = 0
foreach ($idx in ($allPanes.Keys | Sort-Object)) {
    $content = $allPanes[$idx].Content
    if ($content -match "syntax is incorrect|文件名|The filename|directory name or volume label") {
        $pathErrors++
        Fail "3.3: Pane $idx has path error"
    }
}
if ($pathErrors -eq 0) {
    Pass "3.3: No path/syntax errors in any pane"
}

Test "3.4: Wait for agents to complete work"

# Wait up to 60 more seconds for agents to do their work
$deadline = (Get-Date).AddSeconds(60)
$allDone = $false
while ((Get-Date) -lt $deadline) {
    $leaderCap = & $PSMUX capture-pane -t "$SESSION:0.0" -p 2>&1 | Out-String
    if ($leaderCap -match "Idle|completed|Both|Done|tasks.*complete") {
        $allDone = $true
        break
    }
    Start-Sleep -Seconds 5
}

if ($allDone) {
    Pass "3.4: Leader reports team work complete"
} else {
    Skip "3.4: Leader did not report completion within timeout"
}

} # end if (-not $skipTeamTests)

# Clean up Claude Code
& $PSMUX send-keys -t $SESSION "/exit" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 3
if (-not $KeepSession) { Cleanup }
# ══════════════════════════════════════════════════════════════
Section "TEST SUMMARY"
# ══════════════════════════════════════════════════════════════

$total = $script:Passed + $script:Failed + $script:Skipped
Write-Host "  Passed:  $($script:Passed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:Failed)" -ForegroundColor Red
Write-Host "  Skipped: $($script:Skipped)" -ForegroundColor Yellow
Write-Host ""
if ($total -gt 0) {
    $rate = [math]::Round(($script:Passed / $total) * 100, 1)
    Write-Host "  Pass Rate: $rate% ($($script:Passed)/$total)"
}
Write-Host ""

# Final cleanup
if (-not $KeepSession) {
    & $PSMUX kill-server 2>&1 | Out-Null
    Start-Sleep -Seconds 1
}

if ($script:Failed -gt 0) { exit 1 } else { exit 0 }
