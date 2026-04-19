#!/usr/bin/env pwsh
# Tests for discussion #210: gastown integration failures against psmux
# https://github.com/psmux/psmux/discussions/210
#
# Covers three psmux bugs identified from pbolduc's failing gastown test suite:
#
#   Bug 1: new-session duplicate exits 0 with wrong message
#          "psmux: session '...' already exists"   (WRONG - gastown needs "duplicate session:")
#          Fix: emits "duplicate session: NAME" and exits 1
#
#   Bug 2: list-sessions -f "#{==:#{session_name},NAME}" not evaluated
#          Was doing raw substring match; now evaluates equality expression
#          Fix: GetSessionInfo()-compatible exact-match filtering
#
#   Bug 3: list-keys -T prefix KEY fails without a running server
#          Fix: falls back to built-in default bindings offline
#
# Each test proves the fix with REAL CLI output and REAL exit codes.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:pass = 0
$script:fail = 0

function Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green;  $script:pass++ }
function Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red;    $script:fail++ }

function KillSession($name) {
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 600
    Remove-Item "$psmuxDir\$name.*" -Force -EA SilentlyContinue
}

function WaitAlive($name, $timeoutMs = 8000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $pf = "$psmuxDir\$name.port"
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw).Trim()
            try {
                $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
                $tcp.Close()
                return $true
            } catch {}
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

# ════════════════════════════════════════════════════════════════════════
Write-Host "`n════ Discussion #210: Gastown Fix Verification ═════════" -ForegroundColor Cyan
Write-Host "  psmux: $PSMUX" -ForegroundColor DarkGray

# ════════════════════════════════════════════════════════════════════════
# BUG 1: Duplicate session detection
# gastown TestDuplicateSession / TestNewSessionWithCommand_Duplicate
# wrapError() looks for "duplicate session" in stderr and maps to ErrSessionExists
# ════════════════════════════════════════════════════════════════════════
Write-Host "`n──── Bug 1: Duplicate session error (exit code + message) ────" -ForegroundColor Yellow

# Part 1a: Exit code MUST be 1 (not 0)
Write-Host ""
Write-Host "  [Test 1a] Exit code is 1 when session already exists"
$sess1 = "d210_dup_a"
KillSession $sess1
& $PSMUX new-session -d -s $sess1 2>&1 | Out-Null
if (-not (WaitAlive $sess1)) { Fail "Could not create initial session $sess1"; exit 1 }

$dupOut = & $PSMUX new-session -d -s $sess1 2>&1
$dupExit = $LASTEXITCODE

if ($dupExit -ne 0) { Pass "exit code $dupExit (non-zero) on duplicate" }
else                 { Fail "exit code 0 on duplicate - gastown gets nil error instead of ErrSessionExists" }

# Part 1b: Stderr MUST contain "duplicate session"  (gastown's wrapError key phrase)
Write-Host "  [Test 1b] Stderr contains 'duplicate session'"
$errMsg = $dupOut | Out-String
if ($errMsg -match "duplicate session") {
    Pass "message contains 'duplicate session': $($errMsg.Trim())"
} else {
    Fail "message does NOT contain 'duplicate session', got: $($errMsg.Trim())"
}

# Part 1c: Session name appears in error message
Write-Host "  [Test 1c] Session name appears in error message"
if ($errMsg -match [regex]::Escape($sess1)) {
    Pass "found session name '$sess1' in: $($errMsg.Trim())"
} else {
    Fail "session name '$sess1' missing from: $($errMsg.Trim())"
}

KillSession $sess1

# Part 1d: -A flag (attach-if-exists) must NOT trigger error
Write-Host "  [Test 1d] -A flag: attach to existing session does not error"
$sess1a = "d210_dup_attach"
KillSession $sess1a
& $PSMUX new-session -d -s $sess1a 2>&1 | Out-Null
WaitAlive $sess1a | Out-Null
$attOut = & $PSMUX new-session -d -s $sess1a -A -X 2>&1  # -X = do not attach, just check
$attExit = $LASTEXITCODE
# -A should NOT return error (exit 0 or silently succeed / attach)
# We just make sure it doesn't say "duplicate session"
if (-not ($attOut -match "duplicate session")) {
    Pass "-A flag does not trigger duplicate session error (exit $attExit)"
} else {
    Fail "-A flag still reports duplicate session error: $attOut"
}
KillSession $sess1a

# Part 1e: Two different sessions do NOT trigger duplicate error
Write-Host "  [Test 1e] Different session names do not trigger duplicate"
$sessA = "d210_unique_x1"
$sessB = "d210_unique_x2"
KillSession $sessA; KillSession $sessB
& $PSMUX new-session -d -s $sessA 2>&1 | Out-Null
WaitAlive $sessA | Out-Null
$newOut = & $PSMUX new-session -d -s $sessB 2>&1
$newExit = $LASTEXITCODE
if ($newExit -eq 0) { Pass "different names: exit 0 (no false duplicate)" }
else                 { Fail "different names: exit $newExit, output: $newOut" }
KillSession $sessA; KillSession $sessB

# ════════════════════════════════════════════════════════════════════════
# BUG 2: list-sessions -f filter with #{==:#{session_name},NAME}
# gastown TestGetSessionInfo uses:
#   list-sessions -F "fmt" -f "#{==:#{session_name},NAME}"
# and expects exactly ONE row for the named session
# ════════════════════════════════════════════════════════════════════════
Write-Host "`n──── Bug 2: list-sessions -f #{==:...} filter ────" -ForegroundColor Yellow

$sessAlpha = "d210_alpha"
$sessBeta  = "d210_beta"
$sessGamma = "d210_gamma"
KillSession $sessAlpha; KillSession $sessBeta; KillSession $sessGamma

& $PSMUX new-session -d -s $sessAlpha 2>&1 | Out-Null
& $PSMUX new-session -d -s $sessBeta  2>&1 | Out-Null
& $PSMUX new-session -d -s $sessGamma 2>&1 | Out-Null

# Wait for all three to come up
foreach ($s in @($sessAlpha, $sessBeta, $sessGamma)) {
    if (-not (WaitAlive $s)) { Fail "Could not create session $s"; exit 1 }
}
Start-Sleep -Milliseconds 300

# Part 2a: Filter returns only the named session
Write-Host ""
Write-Host "  [Test 2a] Filter #{==:#{session_name},d210_beta} returns only d210_beta"
$rows = @(& $PSMUX list-sessions -F "#{session_name}" -f "#{==:#{session_name},$sessBeta}" 2>&1)
$rowStr = ($rows | Out-String).Trim()
$rowArr = @($rows | Where-Object { $_ -and "$_".Trim() })
if ($rowArr.Count -eq 1 -and "$($rowArr[0])".Trim() -eq $sessBeta) {
    Pass "exactly 1 row: '$($rowArr[0].Trim())'"
} else {
    Fail "expected 1 row '$sessBeta', got $($rowArr.Count) rows: $rowStr"
}

# Part 2b: Filter for alpha returns only alpha (not beta or gamma)
Write-Host "  [Test 2b] Filter for d210_alpha excludes d210_beta and d210_gamma"
$rows2 = @(& $PSMUX list-sessions -F "#{session_name}" -f "#{==:#{session_name},$sessAlpha}" 2>&1 | Where-Object { $_ -and "$_".Trim() })
if ($rows2.Count -eq 1 -and "$($rows2[0])".Trim() -eq $sessAlpha) {
    Pass "only '$sessAlpha' returned"
} else {
    Fail "expected only '$sessAlpha', got: $($rows2 -join ', ')"
}

# Part 2c: Exact gastown GetSessionInfo format string
Write-Host "  [Test 2c] Full gastown GetSessionInfo format works"
$fmt = "#{session_name}|#{session_windows}|#{session_created}|#{session_attached}|#{session_activity}|#{session_last_attached}"
$info = & $PSMUX list-sessions -F $fmt -f "#{==:#{session_name},$sessGamma}" 2>&1
$infoStr = ($info | Out-String).Trim()
# Should have exactly one pipe-delimited row with 6 fields
$fields = $infoStr -split '\|'
if ($fields.Count -eq 6 -and $fields[0].Trim() -eq $sessGamma) {
    Pass "GetSessionInfo format: 6 fields, name='$($fields[0].Trim())'"
} else {
    Fail "GetSessionInfo format failed: fields=$($fields.Count), raw='$infoStr'"
}

# Part 2d: Filter for nonexistent session returns empty (not an error)
Write-Host "  [Test 2d] Filter for nonexistent session returns empty"
$noRows = & $PSMUX list-sessions -F "#{session_name}" -f "#{==:#{session_name},d210_nonexistent_xyz}" 2>&1 | Where-Object { $_ -and $_.Trim() }
if ($noRows.Count -eq 0) {
    Pass "empty result for nonexistent session (no crash)"
} else {
    Fail "expected empty, got: $($noRows -join ', ')"
}

# Part 2e: No -f filter returns all three sessions
Write-Host "  [Test 2e] No -f filter returns all running sessions"
$allRows = & $PSMUX list-sessions -F "#{session_name}" 2>&1 | Where-Object { $_ -and $_.Trim() }
$hasAll = ($allRows -contains $sessAlpha) -and ($allRows -contains $sessBeta) -and ($allRows -contains $sessGamma)
if ($hasAll) {
    Pass "all 3 sessions visible without filter ($($allRows.Count) total)"
} else {
    Fail "missing sessions - got: $($allRows -join ', ')"
}

KillSession $sessAlpha; KillSession $sessBeta; KillSession $sessGamma

# ════════════════════════════════════════════════════════════════════════
# BUG 3: list-keys offline fallback
# gastown TestGetKeyBinding_CapturesDefaultBinding  (n => next-window)
# gastown TestGetKeyBinding_CapturesDefaultBindingWithArgs (w => choose-tree)
# Real tmux works without a server for built-in defaults
# ════════════════════════════════════════════════════════════════════════
Write-Host "`n──── Bug 3: list-keys offline built-in fallback ────" -ForegroundColor Yellow

# Ensure no running sessions for a clean offline test
# (kill-server if any, then test; we re-start after)
Write-Host ""
Write-Host "  [Test 3a] list-keys -T prefix n returns next-window (offline)"
$lkN = & $PSMUX list-keys -T prefix n 2>&1 | Out-String
if ($lkN -match "bind-key.*-T\s+prefix\s+n\s+next-window") {
    Pass "n => next-window: $($lkN.Trim())"
} else {
    Fail "n did not map to next-window, got: $($lkN.Trim())"
}

Write-Host "  [Test 3b] list-keys -T prefix w returns choose-tree (offline)"
$lkW = & $PSMUX list-keys -T prefix w 2>&1 | Out-String
if ($lkW -match "bind-key.*-T\s+prefix\s+w\s+choose-tree") {
    Pass "w => choose-tree: $($lkW.Trim())"
} else {
    Fail "w did not map to choose-tree, got: $($lkW.Trim())"
}

Write-Host "  [Test 3c] list-keys -T prefix p returns previous-window (offline)"
$lkP = & $PSMUX list-keys -T prefix p 2>&1 | Out-String
if ($lkP -match "bind-key.*-T\s+prefix\s+p\s+previous-window") {
    Pass "p => previous-window: $($lkP.Trim())"
} else {
    Fail "p did not map to previous-window, got: $($lkP.Trim())"
}

Write-Host "  [Test 3d] list-keys -T prefix d returns detach-client (offline)"
$lkD = & $PSMUX list-keys -T prefix d 2>&1 | Out-String
if ($lkD -match "bind-key.*-T\s+prefix\s+d\s+detach-client") {
    Pass "d => detach-client: $($lkD.Trim())"
} else {
    Fail "d did not map to detach-client, got: $($lkD.Trim())"
}

Write-Host "  [Test 3e] list-keys -T prefix x returns kill-pane (offline)"
$lkX = & $PSMUX list-keys -T prefix x 2>&1 | Out-String
if ($lkX -match "bind-key.*-T\s+prefix\s+x\s+kill-pane") {
    Pass "x => kill-pane: $($lkX.Trim())"
} else {
    Fail "x did not map to kill-pane, got: $($lkX.Trim())"
}

Write-Host "  [Test 3f] list-keys -T prefix (no key) lists ALL prefix bindings"
$lkAll = & $PSMUX list-keys -T prefix 2>&1
$lkAllStr = ($lkAll | Out-String)
$bindCount = ($lkAll | Where-Object { $_ -match "^bind-key" }).Count
if ($bindCount -ge 20) {
    Pass "prefix table has $bindCount bindings (expected >= 20)"
} else {
    Fail "expected >= 20 prefix bindings, got $bindCount"
}

Write-Host "  [Test 3g] list-keys -T prefix n returns same result WITH a live session"
$lkSess = "d210_lk_live"
KillSession $lkSess
& $PSMUX new-session -d -s $lkSess 2>&1 | Out-Null
WaitAlive $lkSess | Out-Null
$lkNLive = & $PSMUX list-keys -T prefix n 2>&1 | Out-String
if ($lkNLive -match "bind-key.*-T\s+prefix\s+n\s+next-window") {
    Pass "n => next-window works WITH live session too: $($lkNLive.Trim())"
} else {
    Fail "n => next-window broke with live session: $($lkNLive.Trim())"
}
KillSession $lkSess

# ════════════════════════════════════════════════════════════════════════
# Summary
# ════════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host "════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host ("  Passed: {0,-3}  Failed: {1}" -f $script:pass, $script:fail) -ForegroundColor $(if ($script:fail -gt 0) { "Red" } else { "Green" })
Write-Host "════════════════════════════════════════════════════════" -ForegroundColor Cyan
exit $script:fail
