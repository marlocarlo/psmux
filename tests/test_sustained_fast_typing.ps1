# Sustained Fast Typing Test
# Proves whether psmux drops or delays chars during continuous fast typing
# at various speeds, simulating a real human typing 2-3 lines nonstop.
#
# Tests at multiple typing speeds:
#   - 100ms interval (~10 chars/sec, ~60 WPM) - normal fast typing
#   -  50ms interval (~20 chars/sec, ~120 WPM) - very fast typing
#   -  15ms interval (~66 chars/sec) - extreme / keyboard repeat rate
#   -   5ms interval (~200 chars/sec) - near-batch speed (triggers stage2)
#   -   0ms interval (batch) - all at once (definitely triggers stage2)
#
# For each speed: inject a known string, wait, check capture-pane for
# dropped/missing/reordered chars, and measure delivery latency.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "fasttype_test"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:Results = @()

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

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
}

# Compile timed injector
$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$timedExe = "$env:TEMP\psmux_timed_injector.exe"
$injectorExe = "$env:TEMP\psmux_injector.exe"
& $csc /nologo /optimize /out:$timedExe "$PSScriptRoot\timed_injector.cs" 2>&1 | Out-Null
& $csc /nologo /optimize /out:$injectorExe "$PSScriptRoot\injector.cs" 2>&1 | Out-Null

Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "SUSTAINED FAST TYPING TEST" -ForegroundColor Cyan
Write-Host "Does psmux drop or freeze chars during continuous fast typing?" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan

# Launch session with input debug
Cleanup
Remove-Item "$psmuxDir\input_debug.log" -Force -EA SilentlyContinue

$env:PSMUX_INPUT_DEBUG = "1"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
$env:PSMUX_INPUT_DEBUG = $null
$PID_TUI = $proc.Id
Write-Host "`nLaunched TUI PID: $PID_TUI" -ForegroundColor Cyan
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
# Define test scenarios
# =========================================================================

# A realistic 2-line sentence (no special chars, no spaces to avoid shell issues)
# We use echo "..." so we can check the output
$testSentences = @{
    "short"  = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"                                    # 26 chars
    "medium" = "TheQuickBrownFoxJumpsOverTheLazyDogAndThenRunsBackAgain"        # 54 chars
    "long"   = "TheQuickBrownFoxJumpsOverTheLazyDogThenRunsBackAgainAndAgainAndAgainUntilItIsTooTiredToMoveAnymore"  # 97 chars
}

$intervals = @(
    @{ ms = 100; label = "100ms (normal fast, ~60 WPM)" },
    @{ ms = 50;  label = "50ms (very fast, ~120 WPM)" },
    @{ ms = 15;  label = "15ms (extreme, keyboard repeat)" },
    @{ ms = 5;   label = "5ms (near batch, may trigger stage2)" },
    @{ ms = 0;   label = "0ms (batch, definitely triggers stage2)" }
)

$testNum = 0
foreach ($iv in $intervals) {
    $testNum++
    $ms = $iv.ms
    $label = $iv.label

    Write-Host ("=" * 60) -ForegroundColor Cyan
    Write-Host "TEST $testNum : Interval $label" -ForegroundColor Cyan
    Write-Host ("=" * 60) -ForegroundColor Cyan

    # Use the "long" sentence for all tests
    $text = $testSentences["long"]
    $marker = "M${testNum}_" + (Get-Random -Maximum 99999)

    # Clear pane
    & $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # Type: echo "<marker><text>"
    & $PSMUX send-keys -t $SESSION -l "echo " 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
    & $injectorExe $PID_TUI "$marker"
    Start-Sleep -Milliseconds 500

    # Now inject the long text at the specified interval
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $timedExe $PID_TUI $text $ms
    $injectTime = $sw.ElapsedMilliseconds
    Write-Host "  Injected $($text.Length) chars in ${injectTime}ms (interval=${ms}ms)"

    # Wait for delivery (longer for slower intervals)
    $waitMs = [Math]::Max(3000, $text.Length * $ms + 2000)
    Write-Host "  Waiting ${waitMs}ms for delivery..."
    Start-Sleep -Milliseconds $waitMs

    # Press Enter to execute
    & $injectorExe $PID_TUI "{ENTER}"
    Start-Sleep -Seconds 2

    $paneOut = Show-Pane "After typing at ${ms}ms interval"

    # Extract the echo output line (the line AFTER the echo command)
    # Look for the marker in the output
    $delivered = ""
    $paneLines = $paneOut -split "`n"
    foreach ($line in $paneLines) {
        $trimmed = $line.Trim()
        # Look for the line that starts with our marker but is NOT the echo command
        if ($trimmed -match "^${marker}" -and $trimmed -notmatch "^echo ") {
            $delivered = $trimmed
            break
        }
    }

    # Also check the echo command line itself for what was typed
    $cmdLine = ""
    foreach ($line in $paneLines) {
        if ($line -match "echo.*${marker}") {
            # Extract everything after "echo "
            if ($line -match "echo\s+(.+)$") {
                $cmdLine = $Matches[1].Trim()
            }
            break
        }
    }

    $expected = "${marker}${text}"
    Write-Host "`n  Expected ($($expected.Length) chars): $expected"
    Write-Host "  CmdLine  ($($cmdLine.Length) chars): $cmdLine"

    if ($cmdLine -eq $expected) {
        Write-Pass "TEST $testNum ($label): ALL $($expected.Length) chars delivered correctly"
        $dropCount = 0
    } else {
        # Find which chars were dropped
        $dropCount = 0
        $extraCount = 0
        $expectedChars = $expected.ToCharArray()
        $gotChars = $cmdLine.ToCharArray()

        # Simple diff: walk through expected and mark missing
        $gi = 0
        $missing = @()
        foreach ($ec in $expectedChars) {
            if ($gi -lt $gotChars.Length -and $gotChars[$gi] -eq $ec) {
                $gi++
            } else {
                $missing += $ec
                $dropCount++
            }
        }

        if ($dropCount -gt 0) {
            Write-Fail "TEST $testNum ($label): $dropCount of $($expected.Length) chars DROPPED!"
            Write-Host "  Missing chars: $($missing -join '')" -ForegroundColor Red
        } elseif ($cmdLine.Length -ne $expected.Length) {
            Write-Fail "TEST $testNum ($label): Length mismatch (expected $($expected.Length), got $($cmdLine.Length))"
        } else {
            Write-Fail "TEST $testNum ($label): Content mismatch"
            # Show first difference
            for ($i = 0; $i -lt [Math]::Min($expected.Length, $cmdLine.Length); $i++) {
                if ($expected[$i] -ne $cmdLine[$i]) {
                    Write-Host "  First diff at position $i : expected '$($expected[$i])' got '$($cmdLine[$i])'" -ForegroundColor Red
                    break
                }
            }
        }
    }

    $script:Results += [PSCustomObject]@{
        Test       = "TEST $testNum"
        Interval   = "${ms}ms"
        Label      = $label
        Expected   = $expected.Length
        Got        = $cmdLine.Length
        Dropped    = $dropCount
        InjectMs   = $injectTime
    }
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

    $stage2 = @($logLines | Where-Object { $_ -match "stage2:" -and $_ -match "chars in 20ms" })
    $stage2Timeout = @($logLines | Where-Object { $_ -match "stage2 timeout" })
    $suppressed = @($logLines | Where-Object { $_ -match "suppressed char" })
    $flushNormal = @($logLines | Where-Object { $_ -match "flush.*chars as normal" })
    $sendPaste = @($logLines | Where-Object { $_ -match "send-paste" -and $_ -match "stage2|CONFIRMED" })

    Write-Host "  Stage2 entries (paste heuristic triggered): $($stage2.Count)" -ForegroundColor $(if ($stage2.Count -gt 0) {"Yellow"} else {"Green"})
    Write-Host "  Stage2 timeouts (buffer flushed as paste):  $($stage2Timeout.Count)" -ForegroundColor $(if ($stage2Timeout.Count -gt 0) {"Yellow"} else {"Green"})
    Write-Host "  Chars suppressed (DROPPED by suppress):     $($suppressed.Count)" -ForegroundColor $(if ($suppressed.Count -gt 0) {"Red"} else {"Green"})
    Write-Host "  Normal flushes (< 3 chars in 20ms):         $($flushNormal.Count)" -ForegroundColor DarkGray

    if ($stage2.Count -gt 0) {
        Write-Host "`n  Stage2 triggers (fast typing tripped paste heuristic):" -ForegroundColor Yellow
        $stage2 | Select-Object -First 10 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkYellow }
        if ($stage2.Count -gt 10) { Write-Host "    ... ($($stage2.Count - 10) more)" }
    }

    if ($suppressed.Count -gt 0) {
        Write-Host "`n  SUPPRESSED chars (paste_suppress_until dropped these):" -ForegroundColor Red
        $suppressed | Select-Object -First 20 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkRed }
        if ($suppressed.Count -gt 20) { Write-Host "    ... ($($suppressed.Count - 20) more)" }
    }

    if ($stage2Timeout.Count -gt 0) {
        Write-Host "`n  Stage2 timeouts:" -ForegroundColor Yellow
        $stage2Timeout | Select-Object -First 10 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkYellow }
    }
} else {
    Write-Host "  Input debug log not found" -ForegroundColor Red
}

# =========================================================================
# CLEANUP
# =========================================================================
Cleanup
try { if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } } catch {}

# =========================================================================
# SUMMARY TABLE
# =========================================================================
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "RESULTS SUMMARY" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan

$script:Results | Format-Table -AutoSize

Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) {"Red"} else {"Green"})
Write-Host ""
Write-Host "INTERPRETATION:" -ForegroundColor Cyan
Write-Host "  100ms/50ms should NEVER trigger stage2 (1-2 chars per 20ms window)" -ForegroundColor White
Write-Host "  15ms CAN trigger stage2 (1-2 chars per 20ms, borderline)" -ForegroundColor White
Write-Host "  5ms/0ms WILL trigger stage2 (3+ chars in 20ms)" -ForegroundColor White
Write-Host "  Any suppressed chars = PROVEN BUG (typing dropped during paste suppress)" -ForegroundColor White
Write-Host "  Stage2 triggers during typing = paste heuristic false positive" -ForegroundColor White
Write-Host ""

if ($script:TestsFailed -eq 0) {
    Write-Host "[PASS] Sustained fast typing: All tests passed" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Sustained fast typing: $($script:TestsFailed) test(s) failed" -ForegroundColor Red
}
exit $script:TestsFailed
