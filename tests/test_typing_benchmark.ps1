# PSMUX vs Direct PowerShell Typing Benchmark
# Types 10 long sentences with spaces at realistic speed
# Measures per-character render latency for both environments
# Uses screen buffer polling for fair comparison

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "bench_typing"

# Compile tools
$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$benchExe = "$env:TEMP\psmux_typing_benchmark.exe"
$injectorExe = "$env:TEMP\psmux_injector.exe"
$timedExe = "$env:TEMP\psmux_timed_injector.exe"

Write-Host "Compiling benchmark tools..." -ForegroundColor DarkGray
& $csc /nologo /optimize /out:$benchExe "$PSScriptRoot\typing_benchmark.cs" 2>&1 | Out-Null
if (-not (Test-Path $benchExe)) {
    Write-Host "FAILED to compile typing_benchmark.cs" -ForegroundColor Red
    exit 1
}
& $csc /nologo /optimize /out:$injectorExe "$PSScriptRoot\injector.cs" 2>&1 | Out-Null
& $csc /nologo /optimize /out:$timedExe "$PSScriptRoot\timed_injector.cs" 2>&1 | Out-Null

Write-Host ""
Write-Host ("=" * 75) -ForegroundColor Cyan
Write-Host "PSMUX vs DIRECT POWERSHELL TYPING BENCHMARK" -ForegroundColor Cyan
Write-Host "10 long sentences, realistic typing speed, render latency comparison" -ForegroundColor Cyan
Write-Host ("=" * 75) -ForegroundColor Cyan

# 10 realistic long sentences with spaces (40-80 chars each)
$sentences = @(
    "the quick brown fox jumps over the lazy dog and runs back home again"
    "pack my box with five dozen liquor jugs before the party starts tonight"
    "how vexingly quick daft zebras jump across the wide open fields today"
    "the five boxing wizards jump quickly through the dark misty forest path"
    "a large fawn jumped quickly over white zinc boxes left near the highway"
    "crazy frederick bought many very exquisite opal jewels from the old shop"
    "we promptly judged antique ivory buckles for the next prize competition"
    "sixty zippers were quickly picked from the woven jute bag on the floor"
    "back in june we delivered oxygen equipment of the same size to the city"
    "playing a quiet game of chess with the king requires very careful moves"
)

$INTERVAL_MS = 40  # ~150 WPM, fast realistic typing

function Cleanup-Psmux {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
}

function Parse-BenchmarkOutput {
    param([string[]]$Lines)
    $summary = $null
    $csvData = @()
    foreach ($line in $Lines) {
        if ($line -match "^SUMMARY ") {
            $summary = @{}
            $parts = $line -replace "^SUMMARY ", "" -split " "
            foreach ($p in $parts) {
                $kv = $p -split "="
                if ($kv.Length -eq 2) {
                    $summary[$kv[0]] = $kv[1]
                }
            }
        } elseif ($line -match "^\d+,\d+,") {
            $csvData += $line
        }
    }
    return @{ Summary = $summary; CSV = $csvData }
}

# =========================================================================
# PHASE 1: PSMUX Benchmark (capture-pane based monitoring)
# =========================================================================
Write-Host "`n"
Write-Host ("=" * 75) -ForegroundColor Yellow
Write-Host "PHASE 1: PSMUX (through multiplexer)" -ForegroundColor Yellow
Write-Host ("=" * 75) -ForegroundColor Yellow

Cleanup-Psmux
Remove-Item "$psmuxDir\input_debug.log" -Force -EA SilentlyContinue

$env:PSMUX_INPUT_DEBUG = "1"
$psmuxProc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
$env:PSMUX_INPUT_DEBUG = $null
$PID_TUI = $psmuxProc.Id
Write-Host "Launched psmux TUI PID: $PID_TUI" -ForegroundColor Cyan
Start-Sleep -Seconds 5

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "Session FAILED" -ForegroundColor Red; exit 1 }
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($cap -match "PS [A-Z]:\\") { break }
}
Write-Host "Session ready.`n" -ForegroundColor Green

$psmuxResults = @()
$sentenceNum = 0

foreach ($sentence in $sentences) {
    $sentenceNum++
    Write-Host "  Sentence $sentenceNum/$($sentences.Count): '$($sentence.Substring(0, [Math]::Min(50, $sentence.Length)))...'" -ForegroundColor White -NoNewline

    # Clear and prepare
    & $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
    & $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # Get baseline
    $baseCap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    $baseLen = 0
    foreach ($l in ($baseCap -split "`n")) {
        if ($l.Trim()) { $baseLen = $l.TrimEnd().Length }
    }

    # Start monitor job (polls capture-pane every 20ms)
    $monJob = Start-Job -ScriptBlock {
        param($PSMUX, $SESSION, $baseLen, $totalChars, $timeoutMs)
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $prevLen = $baseLen
        $firstMs = 0
        $lastMs = 0
        $lastChangeMs = 0
        $maxGap = 0
        $stallCount = 0
        $burstCount = 0
        $gaps = [System.Collections.ArrayList]::new()
        $allSeen = $false

        while ($sw.ElapsedMilliseconds -lt $timeoutMs -and -not $allSeen) {
            $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
            $ts = $sw.ElapsedMilliseconds

            # Concatenate all non-empty lines to handle wrapping
            $allText = ""
            $lines = $cap -split "`n"
            $foundPrompt = $false
            foreach ($l in $lines) {
                $trimmed = $l.TrimEnd()
                if ($trimmed -match "^.*PS [A-Z]:\\" -and -not $foundPrompt) {
                    $foundPrompt = $true
                    $allText = $trimmed
                } elseif ($foundPrompt -and $trimmed -and $trimmed -notmatch "^.*PS [A-Z]:\\") {
                    $allText += $trimmed
                } elseif ($foundPrompt -and $trimmed -match "^.*PS [A-Z]:\\") {
                    break
                }
            }

            $curLen = $allText.Length
            if ($curLen -ne $prevLen -and $curLen -gt $prevLen) {
                $delta = $curLen - $prevLen
                if ($firstMs -eq 0) { $firstMs = $ts }
                $lastMs = $ts

                if ($lastChangeMs -gt 0) {
                    $gap = $ts - $lastChangeMs
                    $null = $gaps.Add($gap)
                    if ($gap -gt $maxGap) { $maxGap = $gap }
                    if ($gap -gt 200) { $stallCount++ }
                    if ($delta -gt 5) { $burstCount++ }
                }
                $lastChangeMs = $ts
                $prevLen = $curLen
            }

            if (($curLen - $baseLen) -ge $totalChars) { $allSeen = $true }
            Start-Sleep -Milliseconds 20
        }

        $sortedGaps = @($gaps | Sort-Object)
        $p50 = if ($sortedGaps.Count -gt 0) { $sortedGaps[[Math]::Floor($sortedGaps.Count * 0.5)] } else { 0 }
        $p90 = if ($sortedGaps.Count -gt 0) { $sortedGaps[[Math]::Floor($sortedGaps.Count * 0.9)] } else { 0 }
        $p99 = if ($sortedGaps.Count -gt 0) { $sortedGaps[[Math]::Floor($sortedGaps.Count * 0.99)] } else { 0 }
        $avg = if ($sortedGaps.Count -gt 0) { [Math]::Round(($gaps | Measure-Object -Average).Average, 1) } else { 0 }

        return @{
            FirstMs   = $firstMs
            LastMs    = $lastMs
            RenderMs  = $lastMs - $firstMs
            Samples   = $gaps.Count
            Stalls    = $stallCount
            Bursts    = $burstCount
            MaxGapMs  = $maxGap
            AvgGapMs  = $avg
            P50Ms     = $p50
            P90Ms     = $p90
            P99Ms     = $p99
            AllSeen   = $allSeen
        }
    } -ArgumentList $PSMUX, $SESSION, $baseLen, $sentence.Length, (($sentence.Length * $INTERVAL_MS) + 8000)

    Start-Sleep -Milliseconds 100

    # Inject the sentence
    & $timedExe $PID_TUI $sentence $INTERVAL_MS 2>&1 | Out-Null

    # Wait for monitor to finish
    $result = $monJob | Wait-Job -Timeout 30 | Receive-Job
    Remove-Job $monJob -Force -EA SilentlyContinue

    if ($result) {
        $psmuxResults += [PSCustomObject]@{
            Num      = $sentenceNum
            Chars    = $sentence.Length
            RenderMs = $result.RenderMs
            AvgGap   = $result.AvgGapMs
            P50      = $result.P50Ms
            P90      = $result.P90Ms
            P99      = $result.P99Ms
            MaxGap   = $result.MaxGapMs
            Stalls   = $result.Stalls
            Bursts   = $result.Bursts
            AllSeen  = $result.AllSeen
        }
        $stallStr = if ($result.Stalls -gt 0) { " STALLS=$($result.Stalls)" } else { "" }
        Write-Host " | render=$($result.RenderMs)ms max_gap=$($result.MaxGapMs)ms p90=$($result.P90Ms)ms$stallStr" -ForegroundColor $(if ($result.Stalls -gt 0) {"Red"} else {"Green"})
    } else {
        Write-Host " | FAILED (no data)" -ForegroundColor Red
        $psmuxResults += [PSCustomObject]@{
            Num = $sentenceNum; Chars = $sentence.Length; RenderMs = -1
            AvgGap = -1; P50 = -1; P90 = -1; P99 = -1; MaxGap = -1
            Stalls = -1; Bursts = -1; AllSeen = $false
        }
    }
}

# Collect psmux debug log stats
$inputLog = "$psmuxDir\input_debug.log"
$psmuxStage2 = 0
$psmuxSuppressed = 0
if (Test-Path $inputLog) {
    $logLines = Get-Content $inputLog -EA SilentlyContinue
    $psmuxStage2 = @($logLines | Where-Object { $_ -match "stage2:" -and $_ -match "chars in 20ms" }).Count
    $psmuxSuppressed = @($logLines | Where-Object { $_ -match "suppressed char" }).Count
}

Cleanup-Psmux
try { if (-not $psmuxProc.HasExited) { Stop-Process -Id $psmuxProc.Id -Force -EA SilentlyContinue } } catch {}

# =========================================================================
# PHASE 2: Direct PowerShell (screen buffer monitoring)
# =========================================================================
Write-Host "`n"
Write-Host ("=" * 75) -ForegroundColor Yellow
Write-Host "PHASE 2: DIRECT POWERSHELL (no multiplexer)" -ForegroundColor Yellow
Write-Host ("=" * 75) -ForegroundColor Yellow

# Launch a plain pwsh in a new console window
$pwshProc = Start-Process -FilePath "pwsh" -ArgumentList "-NoProfile","-NoExit","-Command","function prompt { 'BENCH> ' }" -PassThru
$PID_PWSH = $pwshProc.Id
Write-Host "Launched direct pwsh PID: $PID_PWSH" -ForegroundColor Cyan
Start-Sleep -Seconds 4

$directResults = @()
$sentenceNum = 0

foreach ($sentence in $sentences) {
    $sentenceNum++
    Write-Host "  Sentence $sentenceNum/$($sentences.Count): '$($sentence.Substring(0, [Math]::Min(50, $sentence.Length)))...'" -ForegroundColor White -NoNewline

    # Clear screen first via injector
    & $injectorExe $PID_PWSH "clear{ENTER}"
    Start-Sleep -Seconds 1

    # Run the benchmark tool (injects + monitors screen buffer)
    $benchOutput = & $benchExe $PID_PWSH $sentence $INTERVAL_MS 2>&1
    $parsed = Parse-BenchmarkOutput -Lines ($benchOutput | ForEach-Object { $_.ToString() })

    if ($parsed.Summary) {
        $s = $parsed.Summary
        $directResults += [PSCustomObject]@{
            Num      = $sentenceNum
            Chars    = $sentence.Length
            RenderMs = [int]$s["render_ms"]
            AvgGap   = [int]$s["avg_gap_ms"]
            P50      = [int]$s["p50_ms"]
            P90      = [int]$s["p90_ms"]
            P99      = [int]$s["p99_ms"]
            MaxGap   = [int]$s["max_gap_ms"]
            Stalls   = [int]$s["stalls"]
            Bursts   = [int]$s["bursts"]
            AllSeen  = $s["all_seen"] -eq "True"
        }
        $stallStr = if ([int]$s["stalls"] -gt 0) { " STALLS=$($s["stalls"])" } else { "" }
        Write-Host " | render=$($s["render_ms"])ms max_gap=$($s["max_gap_ms"])ms p90=$($s["p90_ms"])ms$stallStr" -ForegroundColor $(if ([int]$s["stalls"] -gt 0) {"Red"} else {"Green"})
    } else {
        Write-Host " | FAILED (no summary)" -ForegroundColor Red
        $directResults += [PSCustomObject]@{
            Num = $sentenceNum; Chars = $sentence.Length; RenderMs = -1
            AvgGap = -1; P50 = -1; P90 = -1; P99 = -1; MaxGap = -1
            Stalls = -1; Bursts = -1; AllSeen = $false
        }
    }

    # Small gap between sentences
    Start-Sleep -Milliseconds 500
}

# Kill direct pwsh
try { Stop-Process -Id $PID_PWSH -Force -EA SilentlyContinue } catch {}

# =========================================================================
# COMPARISON TABLE
# =========================================================================
Write-Host "`n"
Write-Host ("=" * 75) -ForegroundColor Cyan
Write-Host "HEAD TO HEAD COMPARISON" -ForegroundColor Cyan
Write-Host ("=" * 75) -ForegroundColor Cyan
Write-Host "Typing speed: ${INTERVAL_MS}ms between chars (~$([Math]::Round(1000/$INTERVAL_MS * 60 / 5)) WPM)" -ForegroundColor White
Write-Host ""

Write-Host "PSMUX RESULTS (through multiplexer):" -ForegroundColor Yellow
$psmuxResults | Format-Table Num, Chars, RenderMs, AvgGap, P50, P90, P99, MaxGap, Stalls, Bursts -AutoSize
Write-Host ""
Write-Host "DIRECT POWERSHELL RESULTS (no multiplexer):" -ForegroundColor Yellow
$directResults | Format-Table Num, Chars, RenderMs, AvgGap, P50, P90, P99, MaxGap, Stalls, Bursts -AutoSize

# Aggregate stats
$validPsmux = @($psmuxResults | Where-Object { $_.RenderMs -ge 0 })
$validDirect = @($directResults | Where-Object { $_.RenderMs -ge 0 })

Write-Host ""
Write-Host ("=" * 75) -ForegroundColor Cyan
Write-Host "AGGREGATE STATISTICS" -ForegroundColor Cyan
Write-Host ("=" * 75) -ForegroundColor Cyan

if ($validPsmux.Count -gt 0) {
    $pAvgRender = [Math]::Round(($validPsmux | Measure-Object -Property RenderMs -Average).Average)
    $pAvgP90    = [Math]::Round(($validPsmux | Measure-Object -Property P90 -Average).Average)
    $pAvgP99    = [Math]::Round(($validPsmux | Measure-Object -Property P99 -Average).Average)
    $pMaxGap    = ($validPsmux | Measure-Object -Property MaxGap -Maximum).Maximum
    $pTotalStalls = ($validPsmux | Measure-Object -Property Stalls -Sum).Sum
    $pTotalBursts = ($validPsmux | Measure-Object -Property Bursts -Sum).Sum

    Write-Host ""
    Write-Host "  PSMUX:" -ForegroundColor Yellow
    Write-Host "    Avg render span:   ${pAvgRender}ms" -ForegroundColor White
    Write-Host "    Avg P90 gap:       ${pAvgP90}ms" -ForegroundColor White
    Write-Host "    Avg P99 gap:       ${pAvgP99}ms" -ForegroundColor White
    Write-Host "    Worst single gap:  ${pMaxGap}ms" -ForegroundColor $(if ($pMaxGap -gt 200) {"Red"} else {"Green"})
    Write-Host "    Total stalls:      $pTotalStalls" -ForegroundColor $(if ($pTotalStalls -gt 0) {"Red"} else {"Green"})
    Write-Host "    Total bursts:      $pTotalBursts" -ForegroundColor $(if ($pTotalBursts -gt 0) {"Yellow"} else {"Green"})
    Write-Host "    Stage2 triggers:   $psmuxStage2" -ForegroundColor $(if ($psmuxStage2 -gt 0) {"Yellow"} else {"Green"})
    Write-Host "    Chars suppressed:  $psmuxSuppressed" -ForegroundColor $(if ($psmuxSuppressed -gt 0) {"Red"} else {"Green"})
}

if ($validDirect.Count -gt 0) {
    $dAvgRender = [Math]::Round(($validDirect | Measure-Object -Property RenderMs -Average).Average)
    $dAvgP90    = [Math]::Round(($validDirect | Measure-Object -Property P90 -Average).Average)
    $dAvgP99    = [Math]::Round(($validDirect | Measure-Object -Property P99 -Average).Average)
    $dMaxGap    = ($validDirect | Measure-Object -Property MaxGap -Maximum).Maximum
    $dTotalStalls = ($validDirect | Measure-Object -Property Stalls -Sum).Sum
    $dTotalBursts = ($validDirect | Measure-Object -Property Bursts -Sum).Sum

    Write-Host ""
    Write-Host "  DIRECT POWERSHELL:" -ForegroundColor Yellow
    Write-Host "    Avg render span:   ${dAvgRender}ms" -ForegroundColor White
    Write-Host "    Avg P90 gap:       ${dAvgP90}ms" -ForegroundColor White
    Write-Host "    Avg P99 gap:       ${dAvgP99}ms" -ForegroundColor White
    Write-Host "    Worst single gap:  ${dMaxGap}ms" -ForegroundColor $(if ($dMaxGap -gt 200) {"Red"} else {"Green"})
    Write-Host "    Total stalls:      $dTotalStalls" -ForegroundColor $(if ($dTotalStalls -gt 0) {"Red"} else {"Green"})
    Write-Host "    Total bursts:      $dTotalBursts" -ForegroundColor $(if ($dTotalBursts -gt 0) {"Yellow"} else {"Green"})
}

if ($validPsmux.Count -gt 0 -and $validDirect.Count -gt 0) {
    Write-Host ""
    Write-Host ("=" * 75) -ForegroundColor Cyan
    Write-Host "DELTA (PSMUX OVERHEAD)" -ForegroundColor Cyan
    Write-Host ("=" * 75) -ForegroundColor Cyan
    $renderOverhead = $pAvgRender - $dAvgRender
    $p90Overhead = $pAvgP90 - $dAvgP90
    Write-Host "    Render span overhead: +${renderOverhead}ms per sentence" -ForegroundColor $(if ($renderOverhead -gt 500) {"Red"} elseif ($renderOverhead -gt 100) {"Yellow"} else {"Green"})
    Write-Host "    P90 gap overhead:     +${p90Overhead}ms" -ForegroundColor $(if ($p90Overhead -gt 50) {"Red"} elseif ($p90Overhead -gt 20) {"Yellow"} else {"Green"})
    $maxGapDelta = $pMaxGap - $dMaxGap
    Write-Host "    Max gap overhead:     +${maxGapDelta}ms" -ForegroundColor $(if ($maxGapDelta -gt 100) {"Red"} elseif ($maxGapDelta -gt 50) {"Yellow"} else {"Green"})

    if ($pTotalStalls -gt 0 -and $dTotalStalls -eq 0) {
        Write-Host "`n    VERDICT: PSMUX has $pTotalStalls stall(s) that direct PowerShell does NOT have." -ForegroundColor Red
        Write-Host "    The user's reported 'freeze feeling' is measurably real." -ForegroundColor Red
        Write-Host "[FAIL] Typing benchmark: Stalls detected" -ForegroundColor Red
        exit 1
    } elseif ($pTotalStalls -eq 0 -and $dTotalStalls -eq 0) {
        Write-Host "`n    VERDICT: No stalls in either environment." -ForegroundColor Green
        if ($p90Overhead -gt 30) {
            Write-Host "    However, psmux P90 is ${p90Overhead}ms higher, which may feel sluggish." -ForegroundColor Yellow
            Write-Host "[PASS] Typing benchmark: No stalls (acceptable overhead)" -ForegroundColor Green
        } else {
            Write-Host "    Psmux overhead is minimal and should not be perceptible." -ForegroundColor Green
            Write-Host "[PASS] Typing benchmark: Minimal overhead" -ForegroundColor Green
        }
    } else {
        Write-Host "`n    VERDICT: Both environments show stalls (possible system load)." -ForegroundColor Yellow
        Write-Host "[PASS] Typing benchmark: Equal stall profiles" -ForegroundColor Green
    }
}

Write-Host ""
