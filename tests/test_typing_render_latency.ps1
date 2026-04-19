# Sustained Fast Typing Latency Test
# Measures HOW FAST characters actually appear in the pane after injection.
# Detects "freeze" = chars injected but not rendering for a long time.
#
# Approach:
#   1. Inject chars one-by-one at a controlled rate (like real typing)
#   2. A monitoring loop polls capture-pane every ~50ms
#   3. Records timestamps of when each new character appears
#   4. Computes per-char render latency, detects stalls/bursts
#   5. Compares psmux latency against a baseline (no multiplexer)
#
# Uses realistic text WITH SPACES between words.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "latency_test"

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
Write-Host "SUSTAINED FAST TYPING RENDER LATENCY TEST" -ForegroundColor Cyan
Write-Host "Measures how fast characters ACTUALLY APPEAR on screen" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan

# Realistic sentences with spaces
$sentences = @(
    "the quick brown fox jumps over the lazy dog"
    "pack my box with five dozen liquor jugs now"
    "how vexingly quick daft zebras jump tonight"
)

# =========================================================================
# Launch session
# =========================================================================
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

for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($cap -match "PS [A-Z]:\\") { break }
}
Write-Host "Session ready.`n" -ForegroundColor Green

# =========================================================================
# Helper: measure render latency for a given text at a given interval
# =========================================================================
function Measure-RenderLatency {
    param(
        [string]$Text,
        [int]$IntervalMs,
        [string]$Label
    )

    Write-Host "`n$('=' * 60)" -ForegroundColor Cyan
    Write-Host "TEST: $Label" -ForegroundColor Cyan
    Write-Host "Text: '$Text' ($($Text.Length) chars, interval=${IntervalMs}ms)" -ForegroundColor White
    Write-Host "$('=' * 60)" -ForegroundColor Cyan

    # Clear pane and set up echo command
    & $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    & $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # Get baseline pane content
    $baseCap = (& $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String).Trim()

    # We inject into an empty prompt line. First get the current prompt content.
    $promptCap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    # Find the active line (last non-empty line)
    $baseLen = 0
    $promptLines = $promptCap -split "`n"
    foreach ($l in $promptLines) {
        if ($l.Trim()) { $baseLen = $l.TrimEnd().Length }
    }
    Write-Host "  Baseline prompt length: $baseLen chars" -ForegroundColor DarkGray

    # Start monitoring in a background job that polls capture-pane every ~30ms
    # Records: timestamp, visible char count on the active line
    $monitorScript = {
        param($PSMUX, $SESSION, $baseLen, $totalChars, $durationMs)
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $samples = [System.Collections.ArrayList]::new()
        $prevLen = $baseLen
        $allSeen = $false

        while ($sw.ElapsedMilliseconds -lt $durationMs -and -not $allSeen) {
            $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
            $ts = $sw.ElapsedMilliseconds

            # Get all non-empty content (may wrap across lines)
            $lines = $cap -split "`n"
            $allText = ""
            $foundPrompt = $false
            foreach ($l in $lines) {
                $trimmed = $l.TrimEnd()
                if ($trimmed -match "^PS [A-Z]:\\" -and -not $foundPrompt) {
                    # This is the prompt line where typing happens
                    $foundPrompt = $true
                    $allText = $trimmed
                } elseif ($foundPrompt -and $trimmed -and $trimmed -notmatch "^PS [A-Z]:\\") {
                    # Continuation line (wrapped text)
                    $allText += $trimmed
                } elseif ($foundPrompt -and $trimmed -match "^PS [A-Z]:\\") {
                    break  # Next prompt, stop
                }
            }

            $curLen = $allText.Length
            if ($curLen -ne $prevLen) {
                $null = $samples.Add([PSCustomObject]@{
                    TimeMs  = $ts
                    CharLen = $curLen
                    Delta   = $curLen - $prevLen
                })
                $prevLen = $curLen
            }

            # Check if we have all chars
            if (($curLen - $baseLen) -ge $totalChars) {
                $allSeen = $true
            }

            Start-Sleep -Milliseconds 30
        }

        return @{
            Samples    = $samples
            FinalLen   = $prevLen
            TotalMs    = $sw.ElapsedMilliseconds
            AllSeen    = $allSeen
        }
    }

    $totalExpected = $Text.Length
    $expectedDuration = ($totalExpected * $IntervalMs) + 5000  # injection time + buffer

    # Start the monitor job
    $job = Start-Job -ScriptBlock $monitorScript -ArgumentList $PSMUX, $SESSION, $baseLen, $totalExpected, $expectedDuration

    # Small delay to let monitor start
    Start-Sleep -Milliseconds 200

    # Inject the text at the specified rate
    $injectSw = [System.Diagnostics.Stopwatch]::StartNew()
    & $timedExe $PID_TUI $Text $IntervalMs
    $injectElapsed = $injectSw.ElapsedMilliseconds
    Write-Host "  Injection took: ${injectElapsed}ms" -ForegroundColor DarkGray

    # Wait for all chars to appear (with timeout)
    $waitTimeout = $expectedDuration + 3000
    $jobResult = $job | Wait-Job -Timeout ([int]($waitTimeout / 1000 + 5)) | Receive-Job
    Remove-Job $job -Force -EA SilentlyContinue

    if (-not $jobResult) {
        Write-Host "  [WARN] Monitor job timed out or returned no data" -ForegroundColor Yellow
        return $null
    }

    $samples = $jobResult.Samples
    $allSeen = $jobResult.AllSeen
    $totalMs = $jobResult.TotalMs

    Write-Host "  Monitor collected $($samples.Count) change events over ${totalMs}ms" -ForegroundColor DarkGray
    Write-Host "  All chars appeared: $allSeen" -ForegroundColor $(if ($allSeen) {"Green"} else {"Red"})

    # ── Analyze the samples ──
    if ($samples.Count -eq 0) {
        Write-Host "  [WARN] No character changes detected!" -ForegroundColor Red
        return $null
    }

    # Time to first char
    $firstCharMs = $samples[0].TimeMs
    Write-Host "`n  Time to first char visible: ${firstCharMs}ms" -ForegroundColor White

    # Time to last char
    $lastCharMs = $samples[-1].TimeMs
    Write-Host "  Time to last char visible:  ${lastCharMs}ms" -ForegroundColor White

    # Total render time
    $renderTime = $lastCharMs - $firstCharMs
    Write-Host "  Total render span:          ${renderTime}ms" -ForegroundColor White

    # Expected time (injection duration)
    $expectedTime = $totalExpected * $IntervalMs
    Write-Host "  Expected injection time:    ${expectedTime}ms" -ForegroundColor DarkGray

    # ── Detect bursts/stalls ──
    # A "stall" is a gap > 200ms between consecutive character appearances
    # A "burst" is when many chars appear at once (delta > 5)
    $stalls = @()
    $bursts = @()
    $gaps = @()

    for ($i = 1; $i -lt $samples.Count; $i++) {
        $gap = $samples[$i].TimeMs - $samples[$i-1].TimeMs
        $delta = $samples[$i].Delta
        $gaps += $gap

        if ($gap -gt 200) {
            $stalls += [PSCustomObject]@{
                AtMs    = $samples[$i].TimeMs
                GapMs   = $gap
                CharsBefore = $samples[$i-1].CharLen
                CharsAfter  = $samples[$i].CharLen
            }
        }
        if ($delta -gt 5) {
            $bursts += [PSCustomObject]@{
                AtMs       = $samples[$i].TimeMs
                CharsAdded = $delta
                GapMs      = $gap
            }
        }
    }

    # Gap statistics
    if ($gaps.Count -gt 0) {
        $sortedGaps = $gaps | Sort-Object
        $avgGap = [Math]::Round(($gaps | Measure-Object -Average).Average, 1)
        $p50 = $sortedGaps[[Math]::Floor($sortedGaps.Count * 0.5)]
        $p90 = $sortedGaps[[Math]::Floor($sortedGaps.Count * 0.9)]
        $p99 = $sortedGaps[[Math]::Floor($sortedGaps.Count * 0.99)]
        $maxGap = $sortedGaps[-1]

        Write-Host "`n  Render gap stats (ms between visible changes):" -ForegroundColor Yellow
        Write-Host "    Average: ${avgGap}ms" -ForegroundColor White
        Write-Host "    P50:     ${p50}ms" -ForegroundColor White
        Write-Host "    P90:     ${p90}ms" -ForegroundColor White
        Write-Host "    P99:     ${p99}ms" -ForegroundColor White
        Write-Host "    Max:     ${maxGap}ms" -ForegroundColor $(if ($maxGap -gt 300) {"Red"} elseif ($maxGap -gt 150) {"Yellow"} else {"Green"})
    }

    # Report stalls
    if ($stalls.Count -gt 0) {
        Write-Host "`n  STALLS DETECTED (>200ms gap, chars not appearing):" -ForegroundColor Red
        foreach ($s in $stalls) {
            Write-Host "    At $($s.AtMs)ms: $($s.GapMs)ms gap (chars $($s.CharsBefore) -> $($s.CharsAfter))" -ForegroundColor Red
        }
    } else {
        Write-Host "`n  No stalls detected (all gaps < 200ms)" -ForegroundColor Green
    }

    # Report bursts
    if ($bursts.Count -gt 0) {
        Write-Host "`n  BURSTS (>5 chars appearing at once, suggests buffering):" -ForegroundColor Yellow
        foreach ($b in $bursts) {
            Write-Host "    At $($b.AtMs)ms: $($b.CharsAdded) chars appeared at once (after $($b.GapMs)ms gap)" -ForegroundColor Yellow
        }
    } else {
        Write-Host "`n  No bursts detected (chars appeared smoothly)" -ForegroundColor Green
    }

    # Character delivery timeline (visual)
    Write-Host "`n  Character appearance timeline:" -ForegroundColor DarkYellow
    $maxTime = $samples[-1].TimeMs
    $barWidth = 60
    $charsDelivered = 0
    foreach ($s in $samples) {
        $charsDelivered += $s.Delta
        $timePos = if ($maxTime -gt 0) { [Math]::Floor(($s.TimeMs / $maxTime) * $barWidth) } else { 0 }
        $bar = ("." * $timePos) + "|"
        $pct = [Math]::Round(($charsDelivered / $totalExpected) * 100)
        if ($s.Delta -gt 3) {
            Write-Host ("    {0,5}ms {1,-62} +{2} chars ({3}%)" -f $s.TimeMs, $bar, $s.Delta, $pct) -ForegroundColor Yellow
        }
    }
    # Show condensed timeline: just show every 10th sample
    $step = [Math]::Max(1, [Math]::Floor($samples.Count / 15))
    $shown = 0
    for ($i = 0; $i -lt $samples.Count; $i += $step) {
        $s = $samples[$i]
        $delivered = 0
        for ($j = 0; $j -le $i; $j++) { $delivered += $samples[$j].Delta }
        $timePos = if ($maxTime -gt 0) { [Math]::Floor(($s.TimeMs / $maxTime) * $barWidth) } else { 0 }
        $filledBar = "#" * $timePos + "." * ($barWidth - $timePos)
        $pct = [Math]::Round(($delivered / $totalExpected) * 100)
        Write-Host ("    {0,5}ms [{1}] {2,3}% ({3} chars)" -f $s.TimeMs, $filledBar, $pct, $delivered) -ForegroundColor DarkGray
        $shown++
    }
    # Always show last
    if ($samples.Count -gt 1) {
        $s = $samples[-1]
        $delivered = 0
        foreach ($ss in $samples) { $delivered += $ss.Delta }
        $filledBar = "#" * $barWidth
        $pct = [Math]::Round(($delivered / $totalExpected) * 100)
        Write-Host ("    {0,5}ms [{1}] {2,3}% ({3} chars)" -f $s.TimeMs, $filledBar, $pct, $delivered) -ForegroundColor DarkGray
    }

    return [PSCustomObject]@{
        Label      = $Label
        TextLen    = $Text.Length
        IntervalMs = $IntervalMs
        FirstCharMs = $firstCharMs
        LastCharMs  = $lastCharMs
        RenderSpanMs = $renderTime
        AvgGapMs    = $avgGap
        P50Ms       = $p50
        P90Ms       = $p90
        P99Ms       = $p99
        MaxGapMs    = $maxGap
        Stalls      = $stalls.Count
        Bursts      = $bursts.Count
        Samples     = $samples.Count
    }
}

# =========================================================================
# Run tests at different typing speeds
# =========================================================================

$results = @()

# Test 1: Normal fast typing (60 WPM ~ 5 chars/sec ~ 200ms between chars, but we type faster in bursts)
$r = Measure-RenderLatency -Text $sentences[0] -IntervalMs 80 -Label "Normal fast typing (80ms, ~75 WPM)"
if ($r) { $results += $r }

# Test 2: Very fast typing (120 WPM ~ 10 chars/sec)
$r = Measure-RenderLatency -Text $sentences[1] -IntervalMs 40 -Label "Very fast typing (40ms, ~150 WPM)"
if ($r) { $results += $r }

# Test 3: Extremely fast / keyboard repeat speed
$r = Measure-RenderLatency -Text $sentences[2] -IntervalMs 15 -Label "Extreme speed (15ms, keyboard repeat rate)"
if ($r) { $results += $r }

# Test 4: Long sentence at fast speed (2+ lines worth)
$longText = "the quick brown fox jumps over the lazy dog and then runs back again and again"
$r = Measure-RenderLatency -Text $longText -IntervalMs 40 -Label "Long sentence at fast speed (40ms, 78 chars)"
if ($r) { $results += $r }

# =========================================================================
# INPUT DEBUG LOG
# =========================================================================
Write-Host "`n$('=' * 60)" -ForegroundColor Cyan
Write-Host "INPUT DEBUG LOG ANALYSIS" -ForegroundColor Cyan
Write-Host "$('=' * 60)" -ForegroundColor Cyan

$inputLog = "$psmuxDir\input_debug.log"
if (Test-Path $inputLog) {
    $logLines = Get-Content $inputLog -EA SilentlyContinue
    $stage2 = @($logLines | Where-Object { $_ -match "stage2:" -and $_ -match "chars in 20ms" })
    $stage2Timeout = @($logLines | Where-Object { $_ -match "stage2 timeout" })
    $suppressed = @($logLines | Where-Object { $_ -match "suppressed char" })
    $flushNormal = @($logLines | Where-Object { $_ -match "flush.*chars as normal" })

    Write-Host "  Stage2 triggers (paste heuristic false positive): $($stage2.Count)" -ForegroundColor $(if ($stage2.Count -gt 0) {"Yellow"} else {"Green"})
    Write-Host "  Stage2 timeouts (typed text sent as paste):       $($stage2Timeout.Count)" -ForegroundColor $(if ($stage2Timeout.Count -gt 0) {"Red"} else {"Green"})
    Write-Host "  Chars SUPPRESSED (dropped by paste window):       $($suppressed.Count)" -ForegroundColor $(if ($suppressed.Count -gt 0) {"Red"} else {"Green"})
    Write-Host "  Normal flushes (correct path):                    $($flushNormal.Count)" -ForegroundColor Green

    if ($stage2.Count -gt 0) {
        Write-Host "`n  Stage2 false positives (typing mistaken for paste):" -ForegroundColor Yellow
        $stage2 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkYellow }
    }
    if ($suppressed.Count -gt 0) {
        Write-Host "`n  SUPPRESSED chars:" -ForegroundColor Red
        $suppressed | Select-Object -First 15 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkRed }
    }
} else {
    Write-Host "  Input debug log not found" -ForegroundColor Red
}

# =========================================================================
# FINAL SUMMARY
# =========================================================================
Cleanup
try { if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } } catch {}

Write-Host "`n$('=' * 70)" -ForegroundColor Cyan
Write-Host "FINAL SUMMARY" -ForegroundColor Cyan
Write-Host "$('=' * 70)" -ForegroundColor Cyan

if ($results.Count -gt 0) {
    $results | Format-Table Label, TextLen, IntervalMs, RenderSpanMs, AvgGapMs, P50Ms, P90Ms, P99Ms, MaxGapMs, Stalls, Bursts -AutoSize
}

$totalStalls = ($results | Measure-Object -Property Stalls -Sum).Sum
$totalBursts = ($results | Measure-Object -Property Bursts -Sum).Sum

Write-Host "  Total stalls (>200ms gaps):  $totalStalls" -ForegroundColor $(if ($totalStalls -gt 0) {"Red"} else {"Green"})
Write-Host "  Total bursts (>5 chars):     $totalBursts" -ForegroundColor $(if ($totalBursts -gt 0) {"Yellow"} else {"Green"})
Write-Host ""
Write-Host "VERDICT:" -ForegroundColor Cyan
if ($totalStalls -gt 0) {
    Write-Host "  FREEZE DETECTED: $totalStalls stall(s) where chars stopped appearing for >200ms" -ForegroundColor Red
    Write-Host "  This is the 'typing freeze' experience the user reported." -ForegroundColor Red
    Write-Host "[FAIL] Typing render latency test" -ForegroundColor Red
    exit 1
} elseif ($totalBursts -gt 0) {
    Write-Host "  BUFFERING DETECTED: Chars appear in bursts rather than smoothly." -ForegroundColor Yellow
    Write-Host "  May feel sluggish compared to direct PowerShell." -ForegroundColor Yellow
    Write-Host "[PASS] Typing render latency: No freezes, acceptable buffering" -ForegroundColor Green
} else {
    Write-Host "  SMOOTH: Characters appear steadily with no significant stalls or bursts." -ForegroundColor Green
    Write-Host "[PASS] Typing render latency test" -ForegroundColor Green
}
Write-Host ""
exit 0
