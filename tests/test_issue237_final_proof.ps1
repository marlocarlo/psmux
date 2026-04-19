# Issue #237 FINAL DEFINITIVE PROOF
# We now KNOW:
# 1. Batch injector works on the launched PID
# 2. Stage2 fires on batch injection (proven by input_debug.log)
# 3. Stage2 timeout fires after 300ms, sends chars as send-paste
# 4. paste_suppress_until = now + 2s is set at that moment
#
# THIS TEST PROVES: characters injected DURING the 2s window are DROPPED.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$batchExe = "$env:TEMP\psmux_batch_injector.exe"
$injectorExe = "$env:TEMP\psmux_injector.exe"
$SESSION = "proof237_final"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Show-Pane {
    param([string]$Label)
    Write-Host "`n  --- $Label ---" -ForegroundColor DarkYellow
    $lines = & $PSMUX capture-pane -t $SESSION -p 2>&1
    $result = ""
    foreach ($line in $lines) {
        $s = $line.ToString()
        if ($s.Trim()) {
            Write-Host "  | $s"
            $result += "$s`n"
        }
    }
    if (-not $result) { Write-Host "  | (empty)" -ForegroundColor DarkGray }
    return $result
}

# ===== COMPILE =====
$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
& $csc /nologo /optimize /out:$batchExe "$PSScriptRoot\injector_batch.cs" 2>&1 | Out-Null
& $csc /nologo /optimize /out:$injectorExe "$PSScriptRoot\injector.cs" 2>&1 | Out-Null

# ===== CLEAN & LAUNCH =====
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "ISSUE #237 FINAL PROOF: 2s paste_suppress_until DROPS keystrokes" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\input_debug.log" -Force -EA SilentlyContinue

$env:PSMUX_INPUT_DEBUG = "1"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
$env:PSMUX_INPUT_DEBUG = $null
$PID_TUI = $proc.Id
Write-Host "Launched TUI PID: $PID_TUI" -ForegroundColor Cyan
Start-Sleep -Seconds 5

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "Session creation FAILED" -ForegroundColor Red; exit 1 }

# Wait for prompt
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($cap -match "PS [A-Z]:\\") { break }
}
Write-Host "Session ready.`n" -ForegroundColor Green

# =========================================================================
# TEST 1: PROVE stage2 fires on batch injection (baseline)
# =========================================================================
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "TEST 1: Confirm batch injection triggers stage2" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

& $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1

# Type 'echo ' via CLI, then batch inject the marker
& $PSMUX send-keys -t $SESSION -l "echo " 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

Write-Host "  Injecting 'MARKER' (6 chars) via batch injector..."
& $batchExe $PID_TUI "MARKER"
Start-Sleep -Seconds 1  # Wait for stage2 to process

Show-Pane "After 'MARKER' batch + 1s wait"

# Press Enter
& $injectorExe $PID_TUI "{ENTER}"
Start-Sleep -Seconds 2
$t1Out = Show-Pane "After Enter"

if ($t1Out -match "MARKER") {
    Write-Pass "TEST 1: Batch injection delivered 'MARKER' via stage2 send-paste"
} else {
    Write-Fail "TEST 1: 'MARKER' not found. Batch injection failed."
}

# =========================================================================
# TEST 2: THE BUG PROOF
# Step 1: Batch inject to trigger stage2 -> paste_suppress_until = now+2s
# Step 2: Wait 500ms (stage2 fires at 300ms + margin)
# Step 3: Inject single chars via regular injector (30ms delay each)
#         These arrive DURING the 2s suppression window
# Step 4: Show pane to see if those chars were DROPPED
# =========================================================================
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "TEST 2: THE BUG PROOF (paste_suppress_until drops chars)" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

& $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1
& $PSMUX send-keys -t $SESSION -l "echo " 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

Write-Host "`n  STEP 1: Batch inject 'TRIGGER' (7 chars, triggers stage2)"
& $batchExe $PID_TUI "TRIGGER"

Write-Host "  STEP 2: Wait 500ms for stage2 timeout..."
Start-Sleep -Milliseconds 500
# At this point, stage2 has fired and paste_suppress_until = now + 2s

Write-Host "  STEP 3: Inject 'XYZ' via regular injector (DURING suppression window)"
Write-Host "          These chars should be DROPPED if 2s bug exists"
& $injectorExe $PID_TUI "XYZ"
Start-Sleep -Milliseconds 500

Show-Pane "500ms after 'XYZ' injection (still in 2s window)"

Write-Host "  STEP 4: Wait for 2s window to expire, inject 'OK'..."
Start-Sleep -Milliseconds 2000
# paste_suppress_until should have expired now (2s from step 2)
& $injectorExe $PID_TUI "OK"
Start-Sleep -Milliseconds 500
Show-Pane "After 'OK' injection (suppression should have expired)"

& $injectorExe $PID_TUI "{ENTER}"
Start-Sleep -Seconds 2
$t2Out = Show-Pane "FINAL OUTPUT"

$hasTrigger = $t2Out -match "TRIGGER"
$hasXYZ = $t2Out -match "XYZ"
$hasOK = $t2Out -match "OK"

Write-Host "`n  RESULTS:" -ForegroundColor Cyan
Write-Host "    'TRIGGER' present: $hasTrigger" -ForegroundColor $(if ($hasTrigger) {"Green"} else {"Red"})
Write-Host "    'XYZ' present: $hasXYZ" -ForegroundColor $(if ($hasXYZ) {"Green"} else {"Red"})
Write-Host "    'OK' present: $hasOK" -ForegroundColor $(if ($hasOK) {"Green"} else {"Red"})

if ($hasTrigger -and -not $hasXYZ -and $hasOK) {
    Write-Fail "TEST 2: 'XYZ' DROPPED, 'OK' arrived after 2s. BUG #237 CONFIRMED!"
    Write-Host ""
    Write-Host "  ================================================================" -ForegroundColor Red
    Write-Host "  DEFINITIVE PROOF: paste_suppress_until = now + 2s" -ForegroundColor Red
    Write-Host "  silently drops ALL typed characters for 2 seconds." -ForegroundColor Red
    Write-Host "  'TRIGGER' triggered stage2 -> suppression set" -ForegroundColor Red
    Write-Host "  'XYZ' typed during window -> DROPPED" -ForegroundColor Red
    Write-Host "  'OK' typed after window expired -> ARRIVED" -ForegroundColor Red
    Write-Host "  ================================================================" -ForegroundColor Red
} elseif ($hasTrigger -and -not $hasXYZ -and -not $hasOK) {
    Write-Fail "TEST 2: Both 'XYZ' and 'OK' dropped. Severe suppression."
} elseif ($hasTrigger -and $hasXYZ) {
    Write-Pass "TEST 2: 'XYZ' arrived (suppression did not affect these chars)"
    Write-Host "  This means either fix is applied or 200ms window expired before XYZ" -ForegroundColor Green
} else {
    Write-Host "  Unexpected result. Check pane output above." -ForegroundColor Yellow
}

# =========================================================================
# TEST 3: Repeated fast bursts (sustained typing)
# Each burst triggers stage2, causing repeated 2s freeze windows
# =========================================================================
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "TEST 3: Repeated fast bursts (sustained typing scenario)" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

& $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1
& $PSMUX send-keys -t $SESSION -l "echo " 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

Write-Host "  Burst 1: 'AAA' (batch, triggers stage2)"
& $batchExe $PID_TUI "AAA"
Start-Sleep -Milliseconds 500  # Stage2 fires

Write-Host "  Burst 2: 'BBB' (during 2s window from burst 1)"
& $batchExe $PID_TUI "BBB"
Start-Sleep -Milliseconds 500

Write-Host "  Burst 3: 'CCC' (still in 2s window)"
& $batchExe $PID_TUI "CCC"
Start-Sleep -Milliseconds 500

Write-Host "  Wait 2s for window to expire..."
Start-Sleep -Seconds 2

Write-Host "  Burst 4: 'DDD' (after window expired)"
& $batchExe $PID_TUI "DDD"
Start-Sleep -Milliseconds 500

& $injectorExe $PID_TUI "{ENTER}"
Start-Sleep -Seconds 2
$t3Out = Show-Pane "FINAL"

$a = $t3Out -match "AAA"
$b = $t3Out -match "BBB"
$c = $t3Out -match "CCC"
$d = $t3Out -match "DDD"

Write-Host "`n  Burst results:" -ForegroundColor Cyan
Write-Host "    AAA (first burst): $a" -ForegroundColor $(if ($a) {"Green"} else {"Red"})
Write-Host "    BBB (in 2s window): $b" -ForegroundColor $(if ($b) {"Green"} else {"Red"})
Write-Host "    CCC (in 2s window): $c" -ForegroundColor $(if ($c) {"Green"} else {"Red"})
Write-Host "    DDD (after window): $d" -ForegroundColor $(if ($d) {"Green"} else {"Red"})

$dropped = @()
if (-not $b) { $dropped += "BBB" }
if (-not $c) { $dropped += "CCC" }
if ($dropped.Count -gt 0 -and $a) {
    Write-Fail "TEST 3: Dropped bursts during window: $($dropped -join ', ')"
} elseif ($a -and $b -and $c -and $d) {
    Write-Pass "TEST 3: All bursts arrived"
} else {
    Write-Host "  Mixed results. Check output above." -ForegroundColor Yellow
}

# =========================================================================
# TEST 4: Control (slow typing should never freeze)
# =========================================================================
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "TEST 4: Control: slow single chars (should never freeze)" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

& $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1
& $PSMUX send-keys -t $SESSION -l "echo " 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

# One char at a time with 100ms gap (well above 20ms threshold)
foreach ($ch in "SLOW".ToCharArray()) {
    & $batchExe $PID_TUI "$ch"
    Start-Sleep -Milliseconds 100
}
& $injectorExe $PID_TUI "{ENTER}"
Start-Sleep -Seconds 2
$t4Out = Show-Pane "FINAL"

if ($t4Out -match "SLOW") {
    Write-Pass "TEST 4: Slow typing delivered all chars"
} else {
    Write-Fail "TEST 4: Even slow typing lost chars"
}

# =========================================================================
# INPUT DEBUG LOG ANALYSIS
# =========================================================================
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "INPUT DEBUG LOG ANALYSIS" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

$inputLog = "$psmuxDir\input_debug.log"
if (Test-Path $inputLog) {
    $logLines = Get-Content $inputLog -EA SilentlyContinue

    $stage2Enter = @($logLines | Where-Object { $_ -match "stage2:" -and $_ -match "chars in 20ms" })
    $stage2Timeout = @($logLines | Where-Object { $_ -match "stage2 timeout" })
    $suppressed = @($logLines | Where-Object { $_ -match "suppressed char" })
    $sendPaste = @($logLines | Where-Object { $_ -match "send-paste|send.paste" })
    $flushNormal = @($logLines | Where-Object { $_ -match "flush.*as normal" })

    Write-Host "  Stage2 ENTERED (chars in 20ms): $($stage2Enter.Count) times" -ForegroundColor Yellow
    Write-Host "  Stage2 TIMEOUT (300ms, sets suppression): $($stage2Timeout.Count) times" -ForegroundColor Yellow
    Write-Host "  Characters SUPPRESSED: $($suppressed.Count)" -ForegroundColor $(if ($suppressed.Count -gt 0) {"Red"} else {"Green"})
    Write-Host "  Sent as send-paste: $($sendPaste.Count) times" -ForegroundColor DarkGray
    Write-Host "  Flushed as normal: $($flushNormal.Count) times" -ForegroundColor DarkGray

    if ($stage2Enter.Count -gt 0) {
        Write-Host "`n  Stage2 trigger entries:" -ForegroundColor Yellow
        $stage2Enter | Select-Object -First 5 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
    }

    if ($stage2Timeout.Count -gt 0) {
        Write-Host "`n  Stage2 timeout entries (these SET paste_suppress_until):" -ForegroundColor Yellow
        $stage2Timeout | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
    }

    if ($suppressed.Count -gt 0) {
        Write-Host "`n  SUPPRESSED CHARACTERS (proof of keystroke dropping):" -ForegroundColor Red
        $suppressed | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkRed }
        Write-Host ""
        Write-Host "  ================================================================" -ForegroundColor Red
        Write-Host "  IRREFUTABLE LOG EVIDENCE: $($suppressed.Count) chars dropped" -ForegroundColor Red
        Write-Host "  by paste_suppress_until in the input event loop." -ForegroundColor Red
        Write-Host "  ================================================================" -ForegroundColor Red
    }

    if ($sendPaste.Count -gt 0) {
        Write-Host "`n  send-paste entries (normal typing sent as paste by stage2):" -ForegroundColor Yellow
        $sendPaste | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
    }

    # Show full log for completeness
    Write-Host "`n  Full debug log ($($logLines.Count) lines):" -ForegroundColor DarkGray
    $logLines | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
} else {
    Write-Host "  Log not found at $inputLog" -ForegroundColor Red
}

# =========================================================================
# CLEANUP & VERDICT
# =========================================================================
& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
try { if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } } catch {}

Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "VERDICT" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) {"Red"} else {"Green"})

if ($script:TestsFailed -gt 0) {
    Write-Host ""
    Write-Host "  BUG #237 IS PROVEN. PR #238 is justified." -ForegroundColor Red
    Write-Host "  Reducing paste_suppress_until from 2s to 200ms prevents" -ForegroundColor Yellow
    Write-Host "  the visible typing freeze while still guarding against" -ForegroundColor Yellow
    Write-Host "  ConPTY paste duplication (<50ms race)." -ForegroundColor Yellow
} else {
    Write-Host ""
    Write-Host "  All tests passed. Check the debug log above for evidence" -ForegroundColor Green
    Write-Host "  that stage2 IS triggering on fast typing." -ForegroundColor Green
}
Write-Host ""
exit $script:TestsFailed
