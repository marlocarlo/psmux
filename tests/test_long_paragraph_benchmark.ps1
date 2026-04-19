# PSMUX vs Direct PowerShell: Long Paragraph Fast Typing Benchmark
# 10 long paragraphs (200-300+ chars each), 15ms per char (~66 chars/sec)
# Measures render latency with capture-pane (psmux) and screen buffer (direct)

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "longbench"

$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$benchExe = "$env:TEMP\psmux_typing_benchmark.exe"
$injectorExe = "$env:TEMP\psmux_injector.exe"
$timedExe = "$env:TEMP\psmux_timed_injector.exe"

& $csc /nologo /optimize /out:$benchExe "$PSScriptRoot\typing_benchmark.cs" 2>&1 | Out-Null
& $csc /nologo /optimize /out:$timedExe "$PSScriptRoot\timed_injector.cs" 2>&1 | Out-Null
& $csc /nologo /optimize /out:$injectorExe "$PSScriptRoot\injector.cs" 2>&1 | Out-Null

Write-Host ""
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host "LONG PARAGRAPH FAST TYPING BENCHMARK: PSMUX vs DIRECT POWERSHELL" -ForegroundColor Cyan
Write-Host "10 paragraphs, 200-300 chars each, 15ms per char (~66 chars/sec)" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan

# 10 long continuous paragraphs with spaces, realistic text, 200-300+ chars each
$paragraphs = @(
    "the quick brown fox jumps over the lazy dog and then it runs all the way back across the entire field because it realized it forgot something very important at home and now it needs to hurry before the sun goes down completely over the hills in the distance tonight"
    "pack my box with five dozen liquor jugs and make sure you stack them carefully on the shelf near the back wall of the warehouse so they do not fall over when the delivery truck arrives early tomorrow morning before anyone else gets to the loading dock area"
    "how vexingly quick daft zebras jump across the wide open fields while the farmers watch from their porches drinking coffee and wondering why these animals keep showing up every single morning without fail regardless of the weather or the season of the year"
    "the five boxing wizards jump quickly through the dark misty forest path that winds around the old abandoned castle where nobody has lived for hundreds of years and the walls are covered with thick green ivy that grows taller every single summer without stopping"
    "a large fawn jumped quickly over white zinc boxes left near the highway rest stop where truckers often park their vehicles overnight to get some sleep before continuing on their long journey across the country to deliver goods to stores and warehouses everywhere"
    "crazy frederick bought many very exquisite opal jewels from the old antique shop downtown near the river and he paid with cash because he did not trust the card reader that looked like it had been sitting there since the early nineteen eighties without being updated"
    "we promptly judged antique ivory buckles for the next prize competition at the county fair where hundreds of people gather every autumn to show off their crafts and compete for ribbons and trophies that they display proudly on their mantles at home all year long"
    "sixty zippers were quickly picked from the woven jute bag on the warehouse floor by the new employee who was trying very hard to impress the supervisor on her first day at work because she really needed this job to pay for her college tuition and rent this month"
    "back in june we delivered oxygen equipment of the same size and weight to the city hospital emergency room on the third floor and the nurses were so grateful because they had been waiting for weeks and the patients really needed those supplies right away urgently"
    "playing a quiet game of chess with the king requires very careful strategic moves and a deep understanding of all the possible outcomes that could arise from each decision you make on the board because one wrong move and the entire game could be lost in seconds flat"
)

$INTERVAL_MS = 15  # 15ms per char = ~66 chars/sec, very fast typing / keyboard repeat

function Cleanup-Psmux {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
}

function Parse-BenchmarkOutput {
    param([string[]]$Lines)
    $summary = $null
    foreach ($line in $Lines) {
        if ($line -match "^SUMMARY ") {
            $summary = @{}
            $parts = $line -replace "^SUMMARY ", "" -split " "
            foreach ($p in $parts) {
                $kv = $p -split "="
                if ($kv.Length -eq 2) { $summary[$kv[0]] = $kv[1] }
            }
        }
    }
    return $summary
}

# =========================================================================
# PHASE 1: PSMUX
# =========================================================================
Write-Host "`n"
Write-Host ("=" * 80) -ForegroundColor Yellow
Write-Host "PHASE 1: PSMUX (through multiplexer)" -ForegroundColor Yellow
Write-Host ("=" * 80) -ForegroundColor Yellow

Cleanup-Psmux
Remove-Item "$psmuxDir\input_debug.log" -Force -EA SilentlyContinue

$env:PSMUX_INPUT_DEBUG = "1"
$psmuxProc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
$env:PSMUX_INPUT_DEBUG = $null
$PID_TUI = $psmuxProc.Id
Write-Host "TUI PID: $PID_TUI" -ForegroundColor Cyan
Start-Sleep -Seconds 5

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "Session FAILED" -ForegroundColor Red; exit 1 }
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($cap -match "PS [A-Z]:\\") { break }
}
Write-Host "Ready.`n" -ForegroundColor Green

$psmuxResults = @()
$num = 0

foreach ($para in $paragraphs) {
    $num++
    $charCount = $para.Length
    $expectedSec = [Math]::Round($charCount * $INTERVAL_MS / 1000, 1)
    Write-Host "  [$num/10] ${charCount} chars (~${expectedSec}s) " -NoNewline -ForegroundColor White

    & $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
    & $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # Get baseline prompt content length
    $baseCap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    $baseLen = 0
    foreach ($l in ($baseCap -split "`n")) { if ($l.Trim()) { $baseLen = $l.TrimEnd().Length } }

    $timeoutJob = ($charCount * $INTERVAL_MS) + 10000

    # Monitor job: polls capture-pane every 15ms
    $monJob = Start-Job -ScriptBlock {
        param($PSMUX, $SESSION, $baseLen, $totalChars, $timeoutMs)
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $prevLen = $baseLen
        $firstMs = 0; $lastMs = 0; $lastChangeMs = 0; $maxGap = 0
        $stallCount = 0; $burstCount = 0
        $gaps = [System.Collections.ArrayList]::new()
        $allSeen = $false
        $stallDetails = [System.Collections.ArrayList]::new()

        while ($sw.ElapsedMilliseconds -lt $timeoutMs -and -not $allSeen) {
            $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
            $ts = $sw.ElapsedMilliseconds

            # Concatenate ALL content from pane (handles line wrapping)
            $allText = ""
            $lines = $cap -split "`n"
            foreach ($l in $lines) {
                $trimmed = $l.TrimEnd()
                if ($trimmed) { $allText += $trimmed }
            }
            $curLen = $allText.Length

            if ($curLen -gt $prevLen) {
                $delta = $curLen - $prevLen
                if ($firstMs -eq 0) { $firstMs = $ts }
                $lastMs = $ts
                if ($lastChangeMs -gt 0) {
                    $gap = [int]($ts - $lastChangeMs)
                    $null = $gaps.Add($gap)
                    if ($gap -gt $maxGap) { $maxGap = $gap }
                    if ($gap -gt 200) {
                        $stallCount++
                        $null = $stallDetails.Add("${gap}ms gap at ${ts}ms (+${delta} chars)")
                    }
                    if ($delta -gt 10) { $burstCount++ }
                }
                $lastChangeMs = $ts
                $prevLen = $curLen
            }
            if (($curLen - $baseLen) -ge $totalChars) { $allSeen = $true }
            Start-Sleep -Milliseconds 15
        }

        $sortedGaps = @($gaps | Sort-Object)
        $cnt = $sortedGaps.Count
        $p50 = if ($cnt -gt 0) { $sortedGaps[[Math]::Floor($cnt * 0.5)] } else { 0 }
        $p90 = if ($cnt -gt 0) { $sortedGaps[[Math]::Floor($cnt * 0.9)] } else { 0 }
        $p95 = if ($cnt -gt 0) { $sortedGaps[[Math]::Floor($cnt * 0.95)] } else { 0 }
        $p99 = if ($cnt -gt 0) { $sortedGaps[[Math]::Floor($cnt * 0.99)] } else { 0 }
        $avg = if ($cnt -gt 0) { [Math]::Round(($gaps | Measure-Object -Average).Average, 1) } else { 0 }

        return @{
            RenderMs = $lastMs - $firstMs
            FirstMs = $firstMs; LastMs = $lastMs
            Samples = $cnt; MaxGap = $maxGap
            Stalls = $stallCount; Bursts = $burstCount
            Avg = $avg; P50 = $p50; P90 = $p90; P95 = $p95; P99 = $p99
            AllSeen = $allSeen
            StallDetails = $stallDetails
        }
    } -ArgumentList $PSMUX, $SESSION, $baseLen, $charCount, $timeoutJob

    Start-Sleep -Milliseconds 100
    & $timedExe $PID_TUI $para $INTERVAL_MS 2>&1 | Out-Null

    $result = $monJob | Wait-Job -Timeout 60 | Receive-Job
    Remove-Job $monJob -Force -EA SilentlyContinue

    if ($result -and $result.RenderMs -gt 0) {
        $psmuxResults += [PSCustomObject]@{
            N = $num; Chars = $charCount
            RenderMs = $result.RenderMs; Avg = $result.Avg
            P50 = $result.P50; P90 = $result.P90; P95 = $result.P95; P99 = $result.P99
            Max = $result.MaxGap; Stalls = $result.Stalls; Bursts = $result.Bursts
        }
        $stallStr = if ($result.Stalls -gt 0) { " STALLS=$($result.Stalls)!" } else { "" }
        $burstStr = if ($result.Bursts -gt 0) { " bursts=$($result.Bursts)" } else { "" }
        Write-Host "render=$($result.RenderMs)ms p50=$($result.P50) p90=$($result.P90) p99=$($result.P99) max=$($result.MaxGap)${stallStr}${burstStr}" -ForegroundColor $(if ($result.Stalls -gt 0) {"Red"} elseif ($result.MaxGap -gt 150) {"Yellow"} else {"Green"})
        if ($result.StallDetails -and $result.StallDetails.Count -gt 0) {
            foreach ($d in $result.StallDetails) { Write-Host "         STALL: $d" -ForegroundColor Red }
        }
    } else {
        Write-Host "FAILED" -ForegroundColor Red
    }
}

# Debug log analysis
$inputLog = "$psmuxDir\input_debug.log"
$pStage2 = 0; $pSupp = 0; $pFlush = 0
if (Test-Path $inputLog) {
    $logLines = Get-Content $inputLog -EA SilentlyContinue
    $pStage2 = @($logLines | Where-Object { $_ -match "stage2:" -and $_ -match "chars in 20ms" }).Count
    $pSupp = @($logLines | Where-Object { $_ -match "suppressed char" }).Count
    $pFlush = @($logLines | Where-Object { $_ -match "flush.*chars as normal" }).Count
}

Cleanup-Psmux
try { if (-not $psmuxProc.HasExited) { Stop-Process -Id $psmuxProc.Id -Force -EA SilentlyContinue } } catch {}

# =========================================================================
# PHASE 2: Direct PowerShell (screen buffer monitoring via C# tool)
# =========================================================================
Write-Host "`n"
Write-Host ("=" * 80) -ForegroundColor Yellow
Write-Host "PHASE 2: DIRECT POWERSHELL (no multiplexer)" -ForegroundColor Yellow
Write-Host ("=" * 80) -ForegroundColor Yellow

$pwshProc = Start-Process -FilePath "pwsh" -ArgumentList "-NoProfile","-NoExit","-Command","cls; function prompt { 'B> ' }" -PassThru
$PID_PWSH = $pwshProc.Id
Write-Host "Direct pwsh PID: $PID_PWSH" -ForegroundColor Cyan
Start-Sleep -Seconds 4

$directResults = @()
$num = 0

foreach ($para in $paragraphs) {
    $num++
    $charCount = $para.Length
    $expectedSec = [Math]::Round($charCount * $INTERVAL_MS / 1000, 1)
    Write-Host "  [$num/10] ${charCount} chars (~${expectedSec}s) " -NoNewline -ForegroundColor White

    # Clear screen
    & $injectorExe $PID_PWSH "cls{ENTER}"
    Start-Sleep -Seconds 1

    $benchOut = & $benchExe $PID_PWSH $para $INTERVAL_MS 2>&1
    $s = Parse-BenchmarkOutput -Lines ($benchOut | ForEach-Object { $_.ToString() })

    if ($s -and $s["render_ms"] -and [int]$s["render_ms"] -gt 0) {
        $directResults += [PSCustomObject]@{
            N = $num; Chars = $charCount
            RenderMs = [int]$s["render_ms"]; Avg = [int]$s["avg_gap_ms"]
            P50 = [int]$s["p50_ms"]; P90 = [int]$s["p90_ms"]; P95 = 0; P99 = [int]$s["p99_ms"]
            Max = [int]$s["max_gap_ms"]; Stalls = [int]$s["stalls"]; Bursts = [int]$s["bursts"]
        }
        $stallStr = if ([int]$s["stalls"] -gt 0) { " STALLS=$($s["stalls"])!" } else { "" }
        Write-Host "render=$($s["render_ms"])ms p50=$($s["p50_ms"]) p90=$($s["p90_ms"]) p99=$($s["p99_ms"]) max=$($s["max_gap_ms"])${stallStr}" -ForegroundColor $(if ([int]$s["stalls"] -gt 0) {"Red"} else {"Green"})
    } else {
        Write-Host "FAILED (no data)" -ForegroundColor Red
    }
    Start-Sleep -Milliseconds 300
}

try { Stop-Process -Id $PID_PWSH -Force -EA SilentlyContinue } catch {}

# =========================================================================
# COMPARISON
# =========================================================================
Write-Host "`n"
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host "HEAD TO HEAD: 10 LONG PARAGRAPHS at 15ms/char (~66 chars/sec)" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan

Write-Host "`nPSMUX:" -ForegroundColor Yellow
$psmuxResults | Format-Table N, Chars, RenderMs, Avg, P50, P90, P95, P99, Max, Stalls, Bursts -AutoSize

Write-Host "DIRECT POWERSHELL:" -ForegroundColor Yellow
$directResults | Format-Table N, Chars, RenderMs, Avg, P50, P90, P95, P99, Max, Stalls, Bursts -AutoSize

$vp = @($psmuxResults | Where-Object { $_.RenderMs -gt 0 })
$vd = @($directResults | Where-Object { $_.RenderMs -gt 0 })

Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host "AGGREGATE (valid runs only: psmux=$($vp.Count), direct=$($vd.Count))" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan

if ($vp.Count -gt 0) {
    Write-Host "`n  PSMUX:" -ForegroundColor Yellow
    Write-Host "    Avg render:  $([Math]::Round(($vp | Measure-Object RenderMs -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Avg P50:     $([Math]::Round(($vp | Measure-Object P50 -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Avg P90:     $([Math]::Round(($vp | Measure-Object P90 -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Avg P99:     $([Math]::Round(($vp | Measure-Object P99 -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Worst gap:   $(($vp | Measure-Object Max -Maximum).Maximum)ms" -ForegroundColor $(if (($vp | Measure-Object Max -Max).Maximum -gt 200) {"Red"} else {"Green"})
    Write-Host "    Total stalls (>200ms): $(($vp | Measure-Object Stalls -Sum).Sum)" -ForegroundColor $(if (($vp | Measure-Object Stalls -Sum).Sum -gt 0) {"Red"} else {"Green"})
    Write-Host "    Total bursts (>10ch):  $(($vp | Measure-Object Bursts -Sum).Sum)" -ForegroundColor $(if (($vp | Measure-Object Bursts -Sum).Sum -gt 0) {"Yellow"} else {"Green"})
    Write-Host "    Stage2 false pos:      $pStage2" -ForegroundColor $(if ($pStage2 -gt 0) {"Red"} else {"Green"})
    Write-Host "    Chars suppressed:      $pSupp" -ForegroundColor $(if ($pSupp -gt 0) {"Red"} else {"Green"})
}
if ($vd.Count -gt 0) {
    Write-Host "`n  DIRECT POWERSHELL:" -ForegroundColor Yellow
    Write-Host "    Avg render:  $([Math]::Round(($vd | Measure-Object RenderMs -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Avg P50:     $([Math]::Round(($vd | Measure-Object P50 -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Avg P90:     $([Math]::Round(($vd | Measure-Object P90 -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Avg P99:     $([Math]::Round(($vd | Measure-Object P99 -Avg).Average))ms" -ForegroundColor White
    Write-Host "    Worst gap:   $(($vd | Measure-Object Max -Maximum).Maximum)ms" -ForegroundColor $(if (($vd | Measure-Object Max -Max).Maximum -gt 200) {"Red"} else {"Green"})
    Write-Host "    Total stalls: $(($vd | Measure-Object Stalls -Sum).Sum)" -ForegroundColor $(if (($vd | Measure-Object Stalls -Sum).Sum -gt 0) {"Red"} else {"Green"})
}

if ($vp.Count -gt 0 -and $vd.Count -gt 0) {
    $pR = [Math]::Round(($vp | Measure-Object RenderMs -Avg).Average)
    $dR = [Math]::Round(($vd | Measure-Object RenderMs -Avg).Average)
    $pP90 = [Math]::Round(($vp | Measure-Object P90 -Avg).Average)
    $dP90 = [Math]::Round(($vd | Measure-Object P90 -Avg).Average)
    $pP99 = [Math]::Round(($vp | Measure-Object P99 -Avg).Average)
    $dP99 = [Math]::Round(($vd | Measure-Object P99 -Avg).Average)
    $pMax = ($vp | Measure-Object Max -Max).Maximum
    $dMax = ($vd | Measure-Object Max -Max).Maximum
    $pStalls = ($vp | Measure-Object Stalls -Sum).Sum
    $dStalls = ($vd | Measure-Object Stalls -Sum).Sum

    Write-Host "`n$('=' * 80)" -ForegroundColor Cyan
    Write-Host "DELTA (PSMUX overhead vs Direct PowerShell)" -ForegroundColor Cyan
    Write-Host "$('=' * 80)" -ForegroundColor Cyan
    Write-Host "    Render:  psmux ${pR}ms vs direct ${dR}ms  (+$($pR - $dR)ms)" -ForegroundColor $(if (($pR - $dR) -gt 500) {"Red"} elseif (($pR - $dR) -gt 200) {"Yellow"} else {"White"})
    Write-Host "    P90:     psmux ${pP90}ms vs direct ${dP90}ms  (+$($pP90 - $dP90)ms)" -ForegroundColor $(if (($pP90 - $dP90) -gt 30) {"Red"} elseif (($pP90 - $dP90) -gt 10) {"Yellow"} else {"White"})
    Write-Host "    P99:     psmux ${pP99}ms vs direct ${dP99}ms  (+$($pP99 - $dP99)ms)" -ForegroundColor $(if (($pP99 - $dP99) -gt 50) {"Red"} elseif (($pP99 - $dP99) -gt 20) {"Yellow"} else {"White"})
    Write-Host "    Max gap: psmux ${pMax}ms vs direct ${dMax}ms  (+$($pMax - $dMax)ms)" -ForegroundColor $(if (($pMax - $dMax) -gt 100) {"Red"} elseif (($pMax - $dMax) -gt 50) {"Yellow"} else {"White"})
    Write-Host "    Stalls:  psmux $pStalls vs direct $dStalls" -ForegroundColor $(if ($pStalls -gt $dStalls) {"Red"} else {"Green"})

    Write-Host "`nVERDICT:" -ForegroundColor Cyan
    if ($pStalls -gt 0 -and $dStalls -eq 0) {
        Write-Host "  FREEZE CONFIRMED: psmux has $pStalls stall(s) that direct PowerShell does NOT." -ForegroundColor Red
    Write-Host "[FAIL] Long paragraph benchmark: Freezes detected" -ForegroundColor Red
    exit 1
} elseif ($pMax - $dMax -gt 100) {
    Write-Host "  NOTICEABLE LAG: psmux worst gap is ${pMax}ms vs ${dMax}ms (+$($pMax - $dMax)ms)." -ForegroundColor Red
    Write-Host "[FAIL] Long paragraph benchmark: Excessive latency gap" -ForegroundColor Red
    exit 1
} elseif ($pP90 - $dP90 -gt 20) {
    Write-Host "  PERCEPTIBLE OVERHEAD: psmux P90 is ${pP90}ms vs ${dP90}ms (+$($pP90 - $dP90)ms)." -ForegroundColor Yellow
    Write-Host "[PASS] Long paragraph benchmark: Acceptable overhead" -ForegroundColor Green
} else {
    Write-Host "  SMOOTH: psmux overhead is within acceptable range." -ForegroundColor Green
    Write-Host "[PASS] Long paragraph benchmark: Minimal overhead" -ForegroundColor Green
}
}
Write-Host ""
Write-Host "[PASS] Long paragraph benchmark completed" -ForegroundColor Green
exit 0
