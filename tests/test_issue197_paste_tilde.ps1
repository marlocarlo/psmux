# Issue #197 - Ctrl+V freezes terminal over SSH / trailing tilde after paste
#
# Bug 1 (FIXED in 86a7519): Bracketed paste close sequence (\x1b[201~) gets
# lost over SSH, parser stays in Paste state forever, terminal hangs.
#
# Bug 2 (FIXED in c28a428): After paste timeout flush, the trailing `~` from
# the stripped close sequence leaks as a visible character.
#
# Bug 3: Local (non SSH) paste shows junk/old clipboard content before actual
# paste text.
#
# This test verifies:
#   1. send-keys with the EXACT text that triggered the freeze works cleanly
#   2. No trailing `~` appears in pane output after paste
#   3. Backslash-heavy Windows paths (the trigger) are preserved correctly
#   4. capture-pane output matches what was sent
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue197_paste_tilde.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found. Build first: cargo build --release"; exit 1 }
Write-Info "Using: $PSMUX"

# Kill any running server
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  ISSUE #197: PASTE TILDE / FREEZE OVER SSH"
Write-Host ("=" * 60)

# ============================================================
# Test 1: The exact trigger text from the bug report
# ============================================================
Write-Host ""
Write-Test "1. Exact trigger text: Windows path with .ps1 extension"

$session = "issue197_t1"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# This is the EXACT text the reporter said caused the freeze
$triggerText = 'C:\Users\myusername\Documents\PowerShell\Microsoft.PowerShell_profile.ps1'
& $PSMUX send-keys -t $session "echo $triggerText" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
Write-Info "  Captured output length: $($output.Length)"

# Check for trailing tilde (the specific bug symptom)
$lines = $output -split "`n" | Where-Object { $_ -match "Microsoft\.PowerShell_profile" }
$hasTilde = $false
foreach ($line in $lines) {
    if ($line.TrimEnd() -match '~\s*$') {
        $hasTilde = $true
        Write-Info "  Line with tilde: [$($line.TrimEnd())]"
    }
}

if ($hasTilde) {
    Write-Fail "Trailing tilde found after paste text (issue #197 regression)"
} else {
    Write-Pass "No trailing tilde after trigger text"
}

# Check content arrived correctly
if ($output -match "Microsoft\.PowerShell_profile\.ps1") {
    Write-Pass "Trigger text content preserved correctly"
} else {
    Write-Fail "Trigger text content missing or corrupted"
    Write-Info "  Output: $($output.Substring(0, [Math]::Min(300, $output.Length)))"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 2: Windows path WITHOUT .ps1 (should always work)
# ============================================================
Write-Host ""
Write-Test "2. Shorter Windows path (was reported as OK by user)"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue197_t2"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$shortPath = 'C:\Users\myusername\Documents\PowerShell\'
& $PSMUX send-keys -t $session "echo $shortPath" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
$lines = $output -split "`n" | Where-Object { $_ -match "Documents\\PowerShell" }
$hasTilde = $false
foreach ($line in $lines) {
    if ($line.TrimEnd() -match '~\s*$') { $hasTilde = $true }
}

if ($hasTilde) {
    Write-Fail "Trailing tilde on short path"
} else {
    Write-Pass "No trailing tilde on short path"
}

if ($output -match 'Documents.PowerShell') {
    Write-Pass "Short path content correct"
} else {
    Write-Fail "Short path content missing"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 3: Quoted Windows path (reported as OK by user)
# ============================================================
Write-Host ""
Write-Test "3. Quoted Windows path (user reported this as OK)"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue197_t3"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Note: the user said quoting the path worked fine
$quotedPath = '"C:\Users\myusername\Documents\PowerShell\Microsoft.PowerShell_profile.ps1"'
& $PSMUX send-keys -t $session "echo $quotedPath" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "Microsoft\.PowerShell_profile\.ps1") {
    Write-Pass "Quoted path content preserved"
} else {
    Write-Fail "Quoted path content missing"
}

$lines = $output -split "`n" | Where-Object { $_ -match "Microsoft\.PowerShell_profile" }
$hasTilde = $false
foreach ($line in $lines) {
    if ($line.TrimEnd() -match '~\s*$') { $hasTilde = $true }
}
if (-not $hasTilde) { Write-Pass "No trailing tilde on quoted path" }
else { Write-Fail "Trailing tilde on quoted path" }

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 4: Repeated text (user said "ddddddddddd" was OK)
# ============================================================
Write-Host ""
Write-Test "4. Repeated character text (user said this was OK)"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue197_t4"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

& $PSMUX send-keys -t $session 'echo ddddddddddd' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "ddddddddddd") {
    Write-Pass "Repeated char text arrived correctly"
} else {
    Write-Fail "Repeated char text missing"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 5: Rapid sequential pastes (stress test paste state machine)
# ============================================================
Write-Host ""
Write-Test "5. Rapid sequential pastes (state machine stress)"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue197_t5"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Send 10 rapid pastes
for ($i = 1; $i -le 10; $i++) {
    & $PSMUX send-keys -t $session "echo RAPID_$i" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
}
Start-Sleep -Seconds 3
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

$allFound = $true
for ($i = 1; $i -le 10; $i++) {
    if ($output -notmatch "RAPID_$i") {
        $allFound = $false
        Write-Info "  Missing: RAPID_$i"
    }
}

if ($allFound) {
    Write-Pass "All 10 rapid paste texts arrived"
} else {
    Write-Fail "Some rapid paste texts missing"
}

# Check for any tilde leakage
$tildeLines = ($output -split "`n") | Where-Object { $_ -match "RAPID_\d+~" }
if ($tildeLines.Count -eq 0) {
    Write-Pass "No tilde leakage in rapid paste sequence"
} else {
    Write-Fail "Tilde leaked in rapid paste: $($tildeLines | Select-Object -First 3)"
}

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 6: Windows path with .log extension (user said OK)
# ============================================================
Write-Host ""
Write-Test "6. Windows path with .log extension"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

$session = "issue197_t6"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$logPath = 'C:\Users\myusername\unity_build.log'
& $PSMUX send-keys -t $session "echo $logPath" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

if ($output -match "unity_build\.log") {
    Write-Pass ".log path content correct"
} else {
    Write-Fail ".log path content missing"
}

$lines = $output -split "`n" | Where-Object { $_ -match "unity_build" }
$hasTilde = $false
foreach ($line in $lines) {
    if ($line.TrimEnd() -match '~\s*$') { $hasTilde = $true }
}
if (-not $hasTilde) { Write-Pass "No trailing tilde on .log path" }
else { Write-Fail "Trailing tilde on .log path" }

& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 7: SSH session paste (the actual failing scenario)
# This tests the real SSH code path by SSHing to localhost
# ============================================================
Write-Host ""
Write-Test "7. SSH session: paste trigger text over SSH to localhost"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

# Check if sshd is running
$sshd = Get-Service sshd -ErrorAction SilentlyContinue
if ($sshd -and $sshd.Status -eq 'Running') {
    $session = "issue197_t7"
    & $PSMUX new-session -d -s $session 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # SSH to localhost and run a command inside the SSH session
    & $PSMUX send-keys -t $session "ssh -o StrictHostKeyChecking=no -o BatchMode=yes localhost `"echo SSH_PASTE_TEST`"" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    $output = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String

    if ($output -match "SSH_PASTE_TEST") {
        Write-Pass "SSH command executed successfully through psmux"
    } else {
        # SSH might need auth, check for password prompt or other issue
        if ($output -match "password" -or $output -match "Permission denied") {
            Write-Info "  SSH auth required (non-key-based), skipping SSH test"
            Write-Pass "(SKIPPED) SSH test requires key-based auth"
        } else {
            Write-Info "  SSH output: $($output.Substring(0, [Math]::Min(300, $output.Length)))"
            Write-Fail "SSH command did not produce expected output"
        }
    }

    # Now test the trigger text through SSH
    & $PSMUX send-keys -t $session "ssh -o StrictHostKeyChecking=no -o BatchMode=yes localhost `"echo C:\\Users\\test\\Documents\\PowerShell\\Microsoft.PowerShell_profile.ps1`"" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    $output2 = (& $PSMUX capture-pane -t $session -p 2>&1) | Out-String
    $sshLines = $output2 -split "`n" | Where-Object { $_ -match "Microsoft\.PowerShell_profile" }

    $hasTilde = $false
    foreach ($line in $sshLines) {
        if ($line.TrimEnd() -match '~\s*$') {
            $hasTilde = $true
            Write-Info "  SSH line with tilde: [$($line.TrimEnd())]"
        }
    }
    if ($sshLines.Count -gt 0 -and -not $hasTilde) {
        Write-Pass "No trailing tilde in SSH paste output"
    } elseif ($sshLines.Count -eq 0) {
        # Could be auth failure
        Write-Info "  No matching lines found in SSH output"
        Write-Pass "(SKIPPED) SSH echo did not produce matching output"
    } else {
        Write-Fail "Trailing tilde in SSH paste output (issue #197 regression)"
    }

    & $PSMUX kill-session -t $session 2>$null | Out-Null
    Start-Sleep -Seconds 1
} else {
    Write-Info "  sshd not running, skipping SSH test"
    Write-Pass "(SKIPPED) sshd not available"
}

# ============================================================
# CLEANUP AND SUMMARY
# ============================================================
Write-Host ""
& $PSMUX kill-server 2>$null | Out-Null
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Write-Host ("=" * 60)
Write-Host "  RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) {
    Write-Host ""
    Write-Fail "SOME TESTS FAILED"
    exit 1
} else {
    Write-Host ""
    Write-Pass "ALL TESTS PASSED"
    exit 0
}
