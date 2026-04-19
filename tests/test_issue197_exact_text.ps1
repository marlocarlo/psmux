<#
.SYNOPSIS
  Test issue #197 with the EXACT text reported by the user.
  Uses the same Process.Start pattern that works in test_vt_paste_missing_close.ps1
#>

$ErrorActionPreference = "Continue"
$PSMUX = "tmux"
$SshHost = "gj@localhost"

Write-Host "=== Issue #197 Exact Text Reproduction Test ===" -ForegroundColor Cyan

# Cleanup
ssh $SshHost "$PSMUX kill-server" 2>$null
Start-Sleep -Seconds 2

# Clear debug log
ssh $SshHost "cmd /c echo. > C:\Users\gj\.psmux\ssh_input.log" 2>$null

$ESC = [char]0x1b
$BPO = "${ESC}[200~"
$BPC = "${ESC}[201~"

# The EXACT text from the issue that caused the freeze
$issueTexts = @(
    'C:\Users\myusername\Documents\PowerShell\Microsoft.PowerShell_profile.ps1',
    'C:\Users\myusername\Documents\PowerShell\',
    '"C:\Users\myusername\Documents\PowerShell\Microsoft.PowerShell_profile.ps1"',
    'ddddddddddd',
    'C:\Users\myusername\unity_build.log'
)

$testNum = 0
$allPass = $true

foreach ($pasteText in $issueTexts) {
    $testNum++

    # Create a fresh session for each test
    ssh $SshHost "$PSMUX kill-server" 2>$null
    Start-Sleep -Seconds 1
    ssh $SshHost "$PSMUX new-session -d -s test197"
    Start-Sleep -Seconds 2

    # Clear the pane
    ssh $SshHost "$PSMUX send-keys -t test197 'clear' Enter"
    Start-Sleep -Seconds 1

    Write-Host "`n--- Test $testNum : Normal paste of '$pasteText' ---" -ForegroundColor Green
    Write-Host "  Length: $($pasteText.Length) chars" -ForegroundColor Yellow

    # Attach via SSH with redirected stdin
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo.FileName = "ssh"
    $proc.StartInfo.Arguments = "-tt $SshHost $PSMUX attach -t test197"
    $proc.StartInfo.UseShellExecute = $false
    $proc.StartInfo.RedirectStandardInput = $true
    $proc.StartInfo.RedirectStandardOutput = $true
    $proc.StartInfo.RedirectStandardError = $true
    $proc.StartInfo.CreateNoWindow = $true
    $proc.Start() | Out-Null
    Write-Host "  PID: $($proc.Id)"
    Start-Sleep -Seconds 3

    $writer = $proc.StandardInput

    # Time the paste
    $sw = [System.Diagnostics.Stopwatch]::StartNew()

    # Send complete bracket paste
    $writer.Write($BPO)
    $writer.Write($pasteText)
    $writer.Write($BPC)
    $writer.Flush()

    Start-Sleep -Milliseconds 500
    $sw.Stop()
    Write-Host "  Paste inject + 500ms settle: $($sw.ElapsedMilliseconds) ms" -ForegroundColor Yellow

    # Type a marker to prove terminal is NOT frozen
    $writer.Write("ALIVE_$testNum")
    $writer.Write("`r")
    $writer.Flush()
    Start-Sleep -Seconds 1

    # Detach
    $writer.Write([char]0x02)  # Ctrl-B
    Start-Sleep -Milliseconds 300
    $writer.Write("d")
    $writer.Flush()
    Start-Sleep -Seconds 2

    try { $proc.Kill() } catch {}
    Start-Sleep -Seconds 1

    # Capture pane content
    $capture = ssh $SshHost "$PSMUX capture-pane -t test197 -p"
    $captureStr = ($capture | Out-String)
    Write-Host "  --- PANE ---" -ForegroundColor Cyan
    $capture | ForEach-Object { Write-Host "    $_" }
    Write-Host "  --- END ---" -ForegroundColor Cyan

    # Check: paste text visible?
    $escaped = [regex]::Escape($pasteText)
    if ($captureStr -match $escaped) {
        Write-Host "  [PASS] Text appeared" -ForegroundColor Green
    } else {
        Write-Host "  [FAIL] Text missing!" -ForegroundColor Red
        $allPass = $false
    }

    # Check: no trailing tilde?
    if ($captureStr -match ($escaped + '~')) {
        Write-Host "  [FAIL] Trailing tilde!" -ForegroundColor Red
        $allPass = $false
    } else {
        Write-Host "  [PASS] No trailing tilde" -ForegroundColor Green
    }

    # Check: terminal not frozen?
    if ($captureStr -match "ALIVE_$testNum") {
        Write-Host "  [PASS] Terminal not frozen (typing works)" -ForegroundColor Green
    } else {
        Write-Host "  [FAIL] Terminal frozen (could not type after paste)" -ForegroundColor Red
        $allPass = $false
    }
}

# ── Test: Missing close sequence with exact issue text ──────────────
Write-Host "`n--- Test $($testNum+1): Missing close sequence with exact issue text ---" -ForegroundColor Green
$freezeText = 'C:\Users\myusername\Documents\PowerShell\Microsoft.PowerShell_profile.ps1'

ssh $SshHost "$PSMUX kill-server" 2>$null
Start-Sleep -Seconds 1
ssh $SshHost "$PSMUX new-session -d -s test197"
Start-Sleep -Seconds 2
ssh $SshHost "$PSMUX send-keys -t test197 'clear' Enter"
Start-Sleep -Seconds 1

$proc2 = New-Object System.Diagnostics.Process
$proc2.StartInfo.FileName = "ssh"
$proc2.StartInfo.Arguments = "-tt $SshHost $PSMUX attach -t test197"
$proc2.StartInfo.UseShellExecute = $false
$proc2.StartInfo.RedirectStandardInput = $true
$proc2.StartInfo.RedirectStandardOutput = $true
$proc2.StartInfo.RedirectStandardError = $true
$proc2.StartInfo.CreateNoWindow = $true
$proc2.Start() | Out-Null
Start-Sleep -Seconds 3

$writer2 = $proc2.StandardInput

# Send paste open + text, NO close (this is what caused the freeze)
$writer2.Write($BPO)
$writer2.Write($freezeText)
$writer2.Flush()

Write-Host "  Sent open + text (NO close). Waiting 3s for 2s timeout..." -ForegroundColor Yellow
Start-Sleep -Seconds 3

# Send tilde (ConPTY residue) after timeout
$writer2.Write("~")
$writer2.Flush()
Start-Sleep -Seconds 1

# Try typing normally
$writer2.Write("RECOVERED")
$writer2.Write("`r")
$writer2.Flush()
Start-Sleep -Seconds 1

# Detach
$writer2.Write([char]0x02)
Start-Sleep -Milliseconds 300
$writer2.Write("d")
$writer2.Flush()
Start-Sleep -Seconds 2
try { $proc2.Kill() } catch {}
Start-Sleep -Seconds 1

$capture2 = ssh $SshHost "$PSMUX capture-pane -t test197 -p"
$captureStr2 = ($capture2 | Out-String)
Write-Host "  --- PANE ---" -ForegroundColor Cyan
$capture2 | ForEach-Object { Write-Host "    $_" }
Write-Host "  --- END ---" -ForegroundColor Cyan

if ($captureStr2 -match [regex]::Escape($freezeText)) {
    Write-Host "  [PASS] Paste flushed after timeout" -ForegroundColor Green
} else {
    Write-Host "  [FAIL] Paste NOT flushed!" -ForegroundColor Red
    $allPass = $false
}

if ($captureStr2 -match ([regex]::Escape($freezeText) + '~')) {
    Write-Host "  [FAIL] Tilde leaked!" -ForegroundColor Red
    $allPass = $false
} else {
    Write-Host "  [PASS] No trailing tilde" -ForegroundColor Green
}

if ($captureStr2 -match "RECOVERED") {
    Write-Host "  [PASS] Terminal recovered (not frozen)" -ForegroundColor Green
} else {
    Write-Host "  [FAIL] Terminal frozen!" -ForegroundColor Red
    $allPass = $false
}

# Cleanup
ssh $SshHost "$PSMUX kill-server" 2>$null

Write-Host ""
if ($allPass) {
    Write-Host "=== ALL TESTS PASSED ===" -ForegroundColor Green
} else {
    Write-Host "=== SOME TESTS FAILED ===" -ForegroundColor Red
}
exit $(if ($allPass) { 0 } else { 1 })
