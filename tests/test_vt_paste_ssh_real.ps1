# test_vt_paste_ssh.ps1
# Exercises the REAL VT parser paste path in ssh_input.rs
# by injecting raw bracket paste bytes through an SSH stdin pipe.

$ErrorActionPreference = "Continue"
$PSMUX = "tmux"

Write-Host "=== VT Parser Paste Path Test (SSH) ===" -ForegroundColor Cyan

# Step 1: Clean up
Write-Host "[STEP 1] Cleaning up old sessions..." -ForegroundColor Yellow
ssh gj@localhost "$PSMUX kill-server" 2>$null
Start-Sleep -Seconds 2

# Step 2: Clear debug log
Write-Host "[STEP 2] Clearing debug log..." -ForegroundColor Yellow
ssh gj@localhost "cmd /c echo. > C:\Users\gj\.psmux\ssh_input.log" 2>$null
Start-Sleep -Milliseconds 500

# Step 3: Start detached session
Write-Host "[STEP 3] Creating detached session..." -ForegroundColor Yellow
ssh gj@localhost "$PSMUX new-session -d -s vt_paste"
Start-Sleep -Seconds 3

# Verify
$sessions = ssh gj@localhost "$PSMUX list-sessions"
Write-Host "  Sessions: $sessions"
if ($sessions -notmatch "vt_paste") {
    Write-Host "[FATAL] Session not created" -ForegroundColor Red
    exit 1
}

# Step 4: Clear the pane
Write-Host "[STEP 4] Clearing pane..." -ForegroundColor Yellow
ssh gj@localhost "$PSMUX send-keys -t vt_paste 'clear' Enter"
Start-Sleep -Seconds 1

# Step 5: Attach via SSH process with redirected stdin, inject bracket paste bytes
Write-Host "[STEP 5] Attaching via SSH and injecting bracket paste bytes..." -ForegroundColor Yellow

$proc = New-Object System.Diagnostics.Process
$proc.StartInfo.FileName = "ssh"
$proc.StartInfo.Arguments = "-tt gj@localhost $PSMUX attach -t vt_paste"
$proc.StartInfo.UseShellExecute = $false
$proc.StartInfo.RedirectStandardInput = $true
$proc.StartInfo.RedirectStandardOutput = $true
$proc.StartInfo.RedirectStandardError = $true
$proc.StartInfo.CreateNoWindow = $true
$proc.Start() | Out-Null

Write-Host "  SSH attach PID: $($proc.Id)"
Start-Sleep -Seconds 3

# Step 6: Write bracket paste sequence to stdin
# This is the EXACT path that triggers the bug:
# SSH stdin -> sshd -> ConPTY -> ReadConsoleInputW -> KEY_EVENT u_char -> VtParser
$writer = $proc.StandardInput

# Build the bracket paste sequence
$ESC = [char]0x1b
$openSeq = "${ESC}[200~"
$payload = "VT_PASTE_PROOF_12345"
$closeSeq = "${ESC}[201~"

Write-Host "  Sending: [open]${payload}[close]" -ForegroundColor Yellow
$writer.Write($openSeq)
$writer.Write($payload)
$writer.Write($closeSeq)
$writer.Flush()
Start-Sleep -Seconds 2

# Step 7: Send Enter to execute
Write-Host "[STEP 7] Sending Enter..." -ForegroundColor Yellow
$writer.Write("`r")
$writer.Flush()
Start-Sleep -Seconds 1

# Step 8: Detach (Ctrl-B then d)
Write-Host "[STEP 8] Detaching..." -ForegroundColor Yellow
$writer.Write([char]0x02)  # Ctrl-B (default prefix)
Start-Sleep -Milliseconds 300
$writer.Write("d")
$writer.Flush()
Start-Sleep -Seconds 2

# Kill the SSH process
try { $proc.Kill() } catch {}
Start-Sleep -Seconds 1

# Step 9: Capture pane content
Write-Host "[STEP 9] Capturing pane content..." -ForegroundColor Yellow
$capture = ssh gj@localhost "$PSMUX capture-pane -t vt_paste -p"
Write-Host "--- PANE CONTENT ---" -ForegroundColor Cyan
$capture | ForEach-Object { Write-Host "  $_" }
Write-Host "--- END ---" -ForegroundColor Cyan

# Step 10: Read debug log
Write-Host "[STEP 10] Reading SSH debug log..." -ForegroundColor Yellow
$debugLog = ssh gj@localhost "type C:\Users\gj\.psmux\ssh_input.log"
Write-Host "--- DEBUG LOG (last 30 lines) ---" -ForegroundColor DarkGray
$debugLog | Select-Object -Last 30 | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
Write-Host "--- END ---" -ForegroundColor DarkGray

# Step 11: Analyze results
Write-Host ""
Write-Host "=== ANALYSIS ===" -ForegroundColor Cyan

$captureStr = ($capture | Out-String)

# Check 1: Was paste text delivered?
if ($captureStr -match "VT_PASTE_PROOF_12345") {
    Write-Host "[PASS] Paste text visible in pane" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Paste text NOT visible in pane" -ForegroundColor Red
}

# Check 2: Is there a trailing tilde?
if ($captureStr -match "VT_PASTE_PROOF_12345~") {
    Write-Host "[FAIL] TRAILING TILDE found after paste text (issue #197 BUG)" -ForegroundColor Red
} else {
    Write-Host "[PASS] No trailing tilde" -ForegroundColor Green
}

# Check 3: Any stray bracket sequence chars?
if ($captureStr -match "200~|201~|\[200|\[201") {
    Write-Host "[FAIL] Bracket sequence markers leaked into pane" -ForegroundColor Red
} else {
    Write-Host "[PASS] No bracket sequence markers in pane" -ForegroundColor Green
}

# Check 4: Debug log shows paste processing
$debugStr = ($debugLog | Out-String)
if ($debugStr -match "flush_stale_paste") {
    Write-Host "[INFO] flush_stale_paste was triggered (close sequence was lost)" -ForegroundColor Yellow
    if ($debugStr -match "PasteDrain") {
        Write-Host "[PASS] PasteDrain state was entered (residue absorption active)" -ForegroundColor Green
    } else {
        Write-Host "[INFO] PasteDrain not logged (close sequence may have arrived normally)" -ForegroundColor Yellow  
    }
} else {
    Write-Host "[INFO] No paste timeout (close sequence arrived normally)" -ForegroundColor Green
}

# Cleanup
Write-Host ""
Write-Host "Cleaning up..." -ForegroundColor Yellow
ssh gj@localhost "$PSMUX kill-server" 2>$null

Write-Host "=== DONE ===" -ForegroundColor Cyan
