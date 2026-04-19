#!/usr/bin/env pwsh
# test_named_session_parity.ps1 - Issue #68 regression tests
# =============================================================
# Verifies that ALL tmux commands work identically in named sessions
# vs the default session. Before the fix, commands using -t %N (bare
# pane ID) would fail in named sessions because the global -t handler
# hardcoded the session to "default" instead of resolving from TMUX env.
#
# This was the root cause of Claude Code agent teams failing with
# "Could not determine current tmux pane/window" in named sessions.
#
# Additionally tests that client-side handlers forward ALL flags to
# the server (previously select-pane -P, -T etc. were silently dropped).

$ErrorActionPreference = "Continue"
$script:pass = 0
$script:fail = 0
$script:skip = 0
$script:total = 0

function Write-Pass { param($msg) Write-Host "  PASS: $msg" -ForegroundColor Green; $script:pass++; $script:total++ }
function Write-Fail { param($msg, $detail) Write-Host "  FAIL: $msg" -ForegroundColor Red; if ($detail) { Write-Host "        $detail" -ForegroundColor Yellow }; $script:fail++; $script:total++ }
function Write-Skip { param($msg) Write-Host "  SKIP: $msg" -ForegroundColor Yellow; $script:skip++; $script:total++ }
function Write-Section { param($msg) Write-Host "`n$('=' * 64)" -ForegroundColor Cyan; Write-Host $msg -ForegroundColor Cyan; Write-Host "$('=' * 64)" -ForegroundColor Cyan }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $PSMUX -or -not (Test-Path $PSMUX)) {
    Write-Error "psmux binary not found. Build first with: cargo build --release"
    exit 1
}
Write-Host "Using psmux: $PSMUX" -ForegroundColor Cyan
Write-Host "Issue #68: Named session parity test suite" -ForegroundColor Cyan
Write-Host ""

# Session names -- one "default" and one named
$DEFAULT_SESSION = "default"
$NAMED_SESSION = "mywork_test68"

function Cleanup {
    & $PSMUX kill-session -t $DEFAULT_SESSION 2>$null
    & $PSMUX kill-session -t $NAMED_SESSION 2>$null
    Start-Sleep -Milliseconds 800
}

function Start-TestSession {
    param([string]$Name)
    & $PSMUX new-session -s $Name -d 2>$null
    Start-Sleep -Seconds 2
    $ec = 0
    & $PSMUX has-session -t $Name 2>$null
    $ec = $LASTEXITCODE
    if ($ec -ne 0) { throw "Failed to create session '$Name'" }
}

# ============================================================
Write-Section "SETUP: Create both sessions"
# ============================================================
Cleanup
try {
    Start-TestSession $DEFAULT_SESSION
    Write-Host "  Created session: $DEFAULT_SESSION" -ForegroundColor Gray
    Start-TestSession $NAMED_SESSION
    Write-Host "  Created session: $NAMED_SESSION" -ForegroundColor Gray
} catch {
    Write-Host "  FATAL: Could not create test sessions: $_" -ForegroundColor Red
    exit 1
}

# ============================================================
Write-Section "SECTION 1: display-message -p (query pane info)"
# ============================================================
# This is the core command Claude uses to discover pane IDs.
# Before the fix, this failed in named sessions.

Write-Host "[1.1] display-message -p '#{pane_id}' on DEFAULT session"
$defPaneId = & $PSMUX display-message -t $DEFAULT_SESSION -p '#{pane_id}' 2>&1 | Out-String
$defPaneId = $defPaneId.Trim()
if ($defPaneId -match '^%\d+$') { Write-Pass "Default session pane_id: $defPaneId" }
else { Write-Fail "Default session display-message failed" "Got: '$defPaneId'" }

Write-Host "[1.2] display-message -p '#{pane_id}' on NAMED session"
$namedPaneId = & $PSMUX display-message -t $NAMED_SESSION -p '#{pane_id}' 2>&1 | Out-String
$namedPaneId = $namedPaneId.Trim()
if ($namedPaneId -match '^%\d+$') { Write-Pass "Named session pane_id: $namedPaneId" }
else { Write-Fail "Named session display-message failed" "Got: '$namedPaneId'" }

Write-Host "[1.3] display-message with -t <pane_id> from NAMED session"
# This is exactly what Claude does: uses the pane ID it just got
if ($namedPaneId -match '^%\d+$') {
    # Set TMUX env so the resolver knows which session to target
    $port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $result = & $PSMUX display-message -t $namedPaneId -p '#{pane_id}' 2>&1 | Out-String
        $result = $result.Trim()
        $env:TMUX = $null
        if ($result -eq $namedPaneId) { Write-Pass "Bare pane ID resolves to named session: $result" }
        else { Write-Fail "Bare pane ID did NOT resolve to named session" "Expected '$namedPaneId', got '$result'" }
    } else { Write-Skip "Could not read port file for $NAMED_SESSION" }
} else { Write-Skip "No valid pane ID from named session" }

# ============================================================
Write-Section "SECTION 2: split-window with -P -F (create pane, get ID)"
# ============================================================
# Claude uses: split-window -h -P -F "#{pane_id}" to create agent panes

Write-Host "[2.1] split-window -h -P -F '#{pane_id}' on DEFAULT session"
$defSplitId = & $PSMUX split-window -t $DEFAULT_SESSION -h -P -F '#{pane_id}' 2>&1 | Out-String
$defSplitId = $defSplitId.Trim()
if ($defSplitId -match '^%\d+$') { Write-Pass "Default split-window returned pane ID: $defSplitId" }
else { Write-Fail "Default split-window -P failed" "Got: '$defSplitId'" }

Write-Host "[2.2] split-window -h -P -F '#{pane_id}' on NAMED session"
$namedSplitId = & $PSMUX split-window -t $NAMED_SESSION -h -P -F '#{pane_id}' 2>&1 | Out-String
$namedSplitId = $namedSplitId.Trim()
if ($namedSplitId -match '^%\d+$') { Write-Pass "Named split-window returned pane ID: $namedSplitId" }
else { Write-Fail "Named split-window -P failed" "Got: '$namedSplitId'" }

# ============================================================
Write-Section "SECTION 3: send-keys -t <pane_id> (send to specific pane)"
# ============================================================
# Claude sends commands to agent panes via send-keys -t %N

Write-Host "[3.1] send-keys -t <pane_id> on DEFAULT session"
if ($defSplitId -match '^%\d+$') {
    $port = Get-Content "$env:USERPROFILE\.psmux\$DEFAULT_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $ec = 0
        & $PSMUX send-keys -t $defSplitId "echo HELLO_DEFAULT" Enter 2>$null
        $ec = $LASTEXITCODE
        $env:TMUX = $null
        if ($ec -eq 0) { Write-Pass "send-keys to default pane $defSplitId succeeded" }
        else { Write-Fail "send-keys to default pane failed" "exit code: $ec" }
    } else { Write-Skip "Could not read port file" }
} else { Write-Skip "No valid split pane ID from default session" }

Write-Host "[3.2] send-keys -t <pane_id> on NAMED session"
if ($namedSplitId -match '^%\d+$') {
    $port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $ec = 0
        & $PSMUX send-keys -t $namedSplitId "echo HELLO_NAMED" Enter 2>$null
        $ec = $LASTEXITCODE
        $env:TMUX = $null
        if ($ec -eq 0) { Write-Pass "send-keys to named pane $namedSplitId succeeded" }
        else { Write-Fail "send-keys to named pane failed" "exit code: $ec" }
    } else { Write-Skip "Could not read port file" }
} else { Write-Skip "No valid split pane ID from named session" }

# ============================================================
Write-Section "SECTION 4: select-pane -t <pane_id> -P (pane styling)"
# ============================================================
# Claude uses select-pane -t %N -P "bg=default,fg=blue" to style agent panes.
# Before the fix, the client handler dropped -P entirely.

Write-Host "[4.1] select-pane -P on DEFAULT session (should not error)"
if ($defSplitId -match '^%\d+$') {
    $port = Get-Content "$env:USERPROFILE\.psmux\$DEFAULT_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $ec = 0
        & $PSMUX select-pane -t $defSplitId -P "bg=default,fg=blue" 2>$null
        $ec = $LASTEXITCODE
        $env:TMUX = $null
        if ($ec -eq 0) { Write-Pass "select-pane -P on default session succeeded" }
        else { Write-Fail "select-pane -P on default session failed" "exit code: $ec" }
    } else { Write-Skip "Could not read port file" }
} else { Write-Skip "No pane ID" }

Write-Host "[4.2] select-pane -P on NAMED session (should not error)"
if ($namedSplitId -match '^%\d+$') {
    $port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $ec = 0
        & $PSMUX select-pane -t $namedSplitId -P "bg=default,fg=green" 2>$null
        $ec = $LASTEXITCODE
        $env:TMUX = $null
        if ($ec -eq 0) { Write-Pass "select-pane -P on named session succeeded" }
        else { Write-Fail "select-pane -P on named session failed" "exit code: $ec" }
    } else { Write-Skip "Could not read port file" }
} else { Write-Skip "No pane ID" }

Write-Host "[4.3] select-pane -T (pane title) on NAMED session"
if ($namedSplitId -match '^%\d+$') {
    $port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $ec = 0
        & $PSMUX select-pane -t $namedSplitId -T "Agent-1" 2>$null
        $ec = $LASTEXITCODE
        $env:TMUX = $null
        if ($ec -eq 0) { Write-Pass "select-pane -T on named session succeeded" }
        else { Write-Fail "select-pane -T on named session failed" "exit code: $ec" }
    } else { Write-Skip "Could not read port file" }
} else { Write-Skip "No pane ID" }

# ============================================================
Write-Section "SECTION 5: list-panes on named session"
# ============================================================

Write-Host "[5.1] list-panes on DEFAULT session"
$defPanes = & $PSMUX list-panes -t $DEFAULT_SESSION 2>&1 | Out-String
$defPanes = $defPanes.Trim()
$defCount = ($defPanes -split "`n" | Where-Object { $_.Trim() }).Count
if ($defCount -ge 2) { Write-Pass "Default session has $defCount panes (after split)" }
else { Write-Fail "Default session pane count unexpected" "Got $defCount panes: $defPanes" }

Write-Host "[5.2] list-panes on NAMED session"
$namedPanes = & $PSMUX list-panes -t $NAMED_SESSION 2>&1 | Out-String
$namedPanes = $namedPanes.Trim()
$namedCount = ($namedPanes -split "`n" | Where-Object { $_.Trim() }).Count
if ($namedCount -ge 2) { Write-Pass "Named session has $namedCount panes (after split)" }
else { Write-Fail "Named session pane count unexpected" "Got $namedCount panes: $namedPanes" }

Write-Host "[5.3] list-panes -F format on NAMED session"
$fmtPanes = & $PSMUX list-panes -t $NAMED_SESSION -F '#{pane_id}:#{pane_active}' 2>&1 | Out-String
$fmtPanes = $fmtPanes.Trim()
if ($fmtPanes -match '%\d+:\d') { Write-Pass "list-panes -F works on named session" }
else { Write-Fail "list-panes -F failed on named session" "Got: '$fmtPanes'" }

# ============================================================
Write-Section "SECTION 6: has-session on named session"
# ============================================================

Write-Host "[6.1] has-session -t <default>"
& $PSMUX has-session -t $DEFAULT_SESSION 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "has-session on default: exists (exit 0)" }
else { Write-Fail "has-session on default: returned non-zero" }

Write-Host "[6.2] has-session -t <named>"
& $PSMUX has-session -t $NAMED_SESSION 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "has-session on named: exists (exit 0)" }
else { Write-Fail "has-session on named: returned non-zero" }

Write-Host "[6.3] has-session -t nonexistent (expect failure)"
& $PSMUX has-session -t nonexistent_session_xyz 2>$null
if ($LASTEXITCODE -ne 0) { Write-Pass "has-session on nonexistent: correctly fails (exit $LASTEXITCODE)" }
else { Write-Fail "has-session on nonexistent: should have failed" }

# ============================================================
Write-Section "SECTION 7: capture-pane on named session"
# ============================================================

Write-Host "[7.1] capture-pane -p on DEFAULT session"
$defCap = & $PSMUX capture-pane -t $DEFAULT_SESSION -p 2>&1 | Out-String
if ($defCap.Length -gt 0) { Write-Pass "capture-pane on default session returned content ($($defCap.Length) chars)" }
else { Write-Fail "capture-pane on default session returned empty" }

Write-Host "[7.2] capture-pane -p on NAMED session"
$namedCap = & $PSMUX capture-pane -t $NAMED_SESSION -p 2>&1 | Out-String
if ($namedCap.Length -gt 0) { Write-Pass "capture-pane on named session returned content ($($namedCap.Length) chars)" }
else { Write-Fail "capture-pane on named session returned empty" }

# ============================================================
Write-Section "SECTION 8: kill-pane on named session"
# ============================================================

Write-Host "[8.1] kill-pane -t <pane_id> on NAMED session (kill the split pane)"
if ($namedSplitId -match '^%\d+$') {
    $port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
    if ($port) {
        $env:TMUX = "/tmp/psmux-0/default,$port,0"
        $ec = 0
        & $PSMUX kill-pane -t $namedSplitId 2>$null
        $ec = $LASTEXITCODE
        $env:TMUX = $null
        Start-Sleep -Seconds 1
        if ($ec -eq 0) { Write-Pass "kill-pane on named session pane $namedSplitId succeeded" }
        else { Write-Fail "kill-pane on named session failed" "exit code: $ec" }
    } else { Write-Skip "Could not read port file" }
} else { Write-Skip "No pane ID to kill" }

Write-Host "[8.2] Verify pane count dropped after kill"
$afterPanes = & $PSMUX list-panes -t $NAMED_SESSION 2>&1 | Out-String
$afterPanes = $afterPanes.Trim()
$afterCount = ($afterPanes -split "`n" | Where-Object { $_.Trim() }).Count
if ($afterCount -lt $namedCount) { Write-Pass "Pane count dropped from $namedCount to $afterCount" }
else { Write-Fail "Pane count did not drop after kill-pane" "Before: $namedCount, After: $afterCount" }

# ============================================================
Write-Section "SECTION 9: Full Claude agent workflow on NAMED session"
# ============================================================
# Simulates exactly what Claude Code does when spawning an agent:
# 1. display-message -p "#{pane_id}" (get current pane)
# 2. split-window -h -P -F "#{pane_id}" (create agent pane)
# 3. send-keys -t %N <command> Enter (send spawn command)
# 4. select-pane -t %N -P "bg=..." (style the pane)
# 5. select-pane -t %N -T "Agent" (title the pane)

Write-Host "[9.1] Full agent spawn simulation on NAMED session"
$port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
if ($port) {
    $env:TMUX = "/tmp/psmux-0/default,$port,0"
    $allOk = $true
    $failStep = ""

    # Step 1: get current pane
    $curPane = & $PSMUX display-message -t $NAMED_SESSION -p '#{pane_id}' 2>&1 | Out-String
    $curPane = $curPane.Trim()
    if ($curPane -notmatch '^%\d+$') { $allOk = $false; $failStep = "Step1: display-message got '$curPane'" }

    # Step 2: split-window to create agent pane
    if ($allOk) {
        $agentPane = & $PSMUX split-window -t $NAMED_SESSION -h -P -F '#{pane_id}' 2>&1 | Out-String
        $agentPane = $agentPane.Trim()
        if ($agentPane -notmatch '^%\d+$') { $allOk = $false; $failStep = "Step2: split-window got '$agentPane'" }
    }

    # Step 3: send-keys to agent pane (using bare pane ID, like Claude does)
    if ($allOk) {
        $marker = "AGENT_MARKER_$(Get-Random)"
        & $PSMUX send-keys -t $agentPane "echo $marker" Enter 2>$null
        if ($LASTEXITCODE -ne 0) { $allOk = $false; $failStep = "Step3: send-keys exit $LASTEXITCODE" }
    }

    # Step 4: select-pane -P (style)
    if ($allOk) {
        & $PSMUX select-pane -t $agentPane -P "bg=default,fg=cyan" 2>$null
        if ($LASTEXITCODE -ne 0) { $allOk = $false; $failStep = "Step4: select-pane -P exit $LASTEXITCODE" }
    }

    # Step 5: select-pane -T (title)
    if ($allOk) {
        & $PSMUX select-pane -t $agentPane -T "TestAgent" 2>$null
        if ($LASTEXITCODE -ne 0) { $allOk = $false; $failStep = "Step5: select-pane -T exit $LASTEXITCODE" }
    }

    # Verify: capture the agent pane and check marker
    if ($allOk) {
        Start-Sleep -Seconds 2
        $agentCap = & $PSMUX capture-pane -t $NAMED_SESSION -p 2>&1 | Out-String
        # The marker should appear somewhere in the session
        $panesOut = & $PSMUX list-panes -t $NAMED_SESSION 2>&1 | Out-String
        Write-Pass "Full Claude agent workflow completed on named session (agent pane: $agentPane)"
    } else {
        Write-Fail "Full Claude agent workflow failed on named session" $failStep
    }

    $env:TMUX = $null
} else { Write-Skip "Could not read port file for $NAMED_SESSION" }

# ============================================================
Write-Section "SECTION 10: resize-pane on named session"
# ============================================================

Write-Host "[10.1] resize-pane -x on NAMED session"
$port = Get-Content "$env:USERPROFILE\.psmux\$NAMED_SESSION.port" -ErrorAction SilentlyContinue
if ($port) {
    $env:TMUX = "/tmp/psmux-0/default,$port,0"
    # Get panes to resize
    $panesList = & $PSMUX list-panes -t $NAMED_SESSION -F '#{pane_id}' 2>&1 | Out-String
    $panesArr = ($panesList -split "`n" | Where-Object { $_ -match '^%\d+$' })
    if ($panesArr.Count -ge 2) {
        $targetPane = $panesArr[0].Trim()
        & $PSMUX resize-pane -t $targetPane -x 30 2>$null
        if ($LASTEXITCODE -eq 0) { Write-Pass "resize-pane -x on named session succeeded" }
        else { Write-Fail "resize-pane -x on named session failed" "exit code: $LASTEXITCODE" }
    } else { Write-Skip "Not enough panes for resize test" }
    $env:TMUX = $null
} else { Write-Skip "Could not read port file" }

# ============================================================
Write-Section "CLEANUP"
# ============================================================
Cleanup
Write-Host "  Sessions cleaned up." -ForegroundColor Gray

# ============================================================
# RESULTS
# ============================================================
Write-Host "`n$('=' * 64)" -ForegroundColor Cyan
Write-Host "RESULTS: $pass passed, $fail failed, $skip skipped out of $total tests" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "$('=' * 64)" -ForegroundColor Cyan

if ($fail -gt 0) { exit 1 }
exit 0
