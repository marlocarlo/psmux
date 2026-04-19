# test_vt_paste_missing_close.ps1
# Exercises the VT parser timeout path: sends bracket paste open + content
# but NO close sequence, forcing the 2s timeout flush.
# Then verifies no tilde or junk leaks.

$ErrorActionPreference = "Continue"
$PSMUX = "tmux"

Write-Host "=== VT Parser Paste: Missing Close Sequence Test ===" -ForegroundColor Cyan

# Step 1: Clean up
ssh gj@localhost "$PSMUX kill-server" 2>$null
Start-Sleep -Seconds 2

# Step 2: Clear debug log
ssh gj@localhost "cmd /c echo. > C:\Users\gj\.psmux\ssh_input.log" 2>$null

# Step 3: Create session with verbose debug logging
ssh gj@localhost "set PSMUX_SSH_DEBUG=1&& tmux new-session -d -s missing_close"
Start-Sleep -Seconds 3
Write-Host "[OK] Session created" -ForegroundColor Green

# Step 4: Clear pane
ssh gj@localhost "$PSMUX send-keys -t missing_close 'clear' Enter"
Start-Sleep -Seconds 1

# Step 5: Attach via SSH, inject bracket paste WITHOUT close sequence
Write-Host "[STEP 5] Attaching and injecting paste WITHOUT close sequence..." -ForegroundColor Yellow

$proc = New-Object System.Diagnostics.Process
$proc.StartInfo.FileName = "ssh"
$proc.StartInfo.Arguments = "-tt gj@localhost $PSMUX attach -t missing_close"
$proc.StartInfo.UseShellExecute = $false
$proc.StartInfo.RedirectStandardInput = $true
$proc.StartInfo.RedirectStandardOutput = $true
$proc.StartInfo.RedirectStandardError = $true
$proc.StartInfo.CreateNoWindow = $true
$proc.Start() | Out-Null
Write-Host "  PID: $($proc.Id)"
Start-Sleep -Seconds 3

$writer = $proc.StandardInput
$ESC = [char]0x1b

# Send ONLY the open sequence + content, NO close sequence
# This simulates ConPTY stripping the entire close sequence
$openSeq = "${ESC}[200~"
$payload = "MISSING_CLOSE_TEST"
Write-Host "  Sending: [open]${payload} (NO close sequence)" -ForegroundColor Yellow
$writer.Write($openSeq)
$writer.Write($payload)
$writer.Flush()

# Wait for the 2 second paste timeout to fire
Write-Host "  Waiting 3 seconds for paste timeout..." -ForegroundColor Yellow
Start-Sleep -Seconds 3

# Now send a tilde (simulating ConPTY leaking only the ~ from close seq)
# In real usage this arrives within ms, we send it ~1s after timeout flush
Write-Host "  Sending lone '~' (residue from stripped close sequence)..." -ForegroundColor Yellow
$writer.Write("~")
$writer.Flush()
Start-Sleep -Seconds 1

# Send Enter to execute whatever is on the command line
$writer.Write("`r")
$writer.Flush()
Start-Sleep -Seconds 1

# Now type a marker to prove we can type normally after
$writer.Write("echo NORMAL_TYPING_WORKS")
$writer.Write("`r")
$writer.Flush()
Start-Sleep -Seconds 2

# Detach
$writer.Write([char]0x02)  # Ctrl-B
Start-Sleep -Milliseconds 300
$writer.Write("d")
$writer.Flush()
Start-Sleep -Seconds 2

try { $proc.Kill() } catch {}
Start-Sleep -Seconds 1

# Capture
$capture = ssh gj@localhost "$PSMUX capture-pane -t missing_close -p"
Write-Host "--- PANE CONTENT ---" -ForegroundColor Cyan
$capture | ForEach-Object { Write-Host "  $_" }
Write-Host "--- END ---" -ForegroundColor Cyan

# Debug log
$debugLog = ssh gj@localhost "type C:\Users\gj\.psmux\ssh_input.log"
Write-Host "--- DEBUG LOG (relevant lines) ---" -ForegroundColor DarkGray
$debugLog | Where-Object { $_ -match "paste|Paste|drain|Drain|flush|tilde|KEY|emit|u_char" } | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
Write-Host "--- END ---" -ForegroundColor DarkGray

# Analyze
Write-Host ""
Write-Host "=== ANALYSIS ===" -ForegroundColor Cyan
$captureStr = ($capture | Out-String)

# The paste content should have been flushed after timeout
if ($captureStr -match "MISSING_CLOSE_TEST") {
    Write-Host "[PASS] Paste text was flushed after timeout" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Paste text NOT visible (still stuck in Paste state?)" -ForegroundColor Red
}

# The tilde MUST NOT appear
if ($captureStr -match "MISSING_CLOSE_TEST~" -or $captureStr -match "~MISSING") {
    Write-Host "[FAIL] TILDE leaked as visible text (issue #197 BUG!)" -ForegroundColor Red
} elseif ($captureStr -match "(?<![~\w])~(?![~\w])") {
    Write-Host "[WARN] Stray tilde found somewhere in pane" -ForegroundColor Yellow
} else {
    Write-Host "[PASS] No trailing tilde" -ForegroundColor Green
}

# Normal typing should work after paste timeout
if ($captureStr -match "NORMAL_TYPING_WORKS") {
    Write-Host "[PASS] Normal typing works after paste timeout recovery" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Normal typing broken after paste timeout" -ForegroundColor Red
}

# Debug log should show flush_stale_paste
$debugStr = ($debugLog | Out-String)
if ($debugStr -match "flush_stale_paste") {
    Write-Host "[PASS] flush_stale_paste fired (timeout detected)" -ForegroundColor Green
} else {
    Write-Host "[FAIL] flush_stale_paste NOT fired" -ForegroundColor Red
}

if ($debugStr -match "PasteDrain") {
    Write-Host "[PASS] PasteDrain state entered" -ForegroundColor Green
}

# Cleanup
ssh gj@localhost "$PSMUX kill-server" 2>$null
Write-Host "=== DONE ===" -ForegroundColor Cyan
