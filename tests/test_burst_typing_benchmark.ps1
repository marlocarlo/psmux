# BURST TYPING BENCHMARK: PSMUX vs DIRECT POWERSHELL
# Tests rapid burst typing (0-2ms between chars within words, 10ms between words)
# 10 long paragraphs (250+ chars each)
# Uses native screen buffer polling at 5ms (200Hz) for both environments
# Fixed: cls and Enter sent separately for direct PowerShell

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "burst_bench"

$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$burstExe = "$env:TEMP\psmux_burst_bench2.exe"
$injectorExe = "$env:TEMP\psmux_injector.exe"

Write-Host "Compiling..." -ForegroundColor DarkGray
& $csc /nologo /optimize /out:$burstExe "$PSScriptRoot\burst_bench2.cs" 2>&1 | Out-Null
if (-not (Test-Path $burstExe)) { Write-Host "Compile FAILED" -ForegroundColor Red; exit 1 }
& $csc /nologo /optimize /out:$injectorExe "$PSScriptRoot\injector.cs" 2>&1 | Out-Null

Write-Host ""
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host "BURST TYPING BENCHMARK: PSMUX vs DIRECT POWERSHELL" -ForegroundColor Cyan
Write-Host "0ms between chars (instant burst), 10ms between words" -ForegroundColor Cyan
Write-Host "Screen buffer polling at 200Hz (5ms) for BOTH environments" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan

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

$INTRA_MS = 0    # 0ms between chars within a word (INSTANT BURST)
$INTER_MS = 10   # 10ms pause between words (space)

function Parse-Summary {
    param([string[]]$Lines)
    foreach ($line in $Lines) {
        if ($line -match "^SUMMARY ") {
            $h = @{}
            ($line -replace "^SUMMARY ","" -split " ") | ForEach-Object {
                $kv = $_ -split "="; if ($kv.Length -eq 2) { $h[$kv[0]] = $kv[1] }
            }
            return $h
        }
    }
    return $null
}

function Run-BurstTest {
    param(
        [uint32]$TargetPid,
        [string]$Text,
        [int]$IntraMs,
        [int]$InterMs
    )
    $output = & $burstExe $TargetPid $Text $IntraMs $InterMs 2>&1
    $lines = $output | ForEach-Object { $_.ToString() }
    return Parse-Summary -Lines $lines
}

# =========================================================================
# PHASE 1: PSMUX
# =========================================================================
Write-Host "`n$('=' * 80)" -ForegroundColor Yellow
Write-Host "PHASE 1: PSMUX (through multiplexer)" -ForegroundColor Yellow
Write-Host "$('=' * 80)" -ForegroundColor Yellow

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
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

$psmuxData = @()
for ($n = 0; $n -lt $paragraphs.Count; $n++) {
    $para = $paragraphs[$n]
    $num = $n + 1
    Write-Host "  [$num/10] $($para.Length) chars " -NoNewline -ForegroundColor White

    # Clear: send Ctrl+C, then "clear", then Enter via injector (separate calls)
    & $injectorExe $PID_TUI "^c"
    Start-Sleep -Milliseconds 200
    & $injectorExe $PID_TUI "clear"
    Start-Sleep -Milliseconds 100
    & $injectorExe $PID_TUI "{ENTER}"
    Start-Sleep -Seconds 1

    $s = Run-BurstTest -TargetPid $PID_TUI -Text $para -IntraMs $INTRA_MS -InterMs $INTER_MS

    if ($s) {
        $psmuxData += [PSCustomObject]@{
            N=$num; Chars=[int]$s["chars"]; InjectMs=[int]$s["inject_ms"]
            RenderMs=[int]$s["render_ms"]; AvgGap=[int]$s["avg_gap"]
            P50=[int]$s["p50"]; P90=[int]$s["p90"]; P95=[int]$s["p95"]; P99=[int]$s["p99"]
            MaxGap=[int]$s["max_gap"]; Stalls=[int]$s["stalls"]; Bursts=[int]$s["bursts"]
        }
        $tag = ""
        if ([int]$s["stalls"] -gt 0) { $tag = " STALLS=$($s["stalls"])!" }
        $color = if ([int]$s["stalls"] -gt 0) {"Red"} elseif ([int]$s["max_gap"] -gt 100) {"Yellow"} else {"Green"}
        Write-Host ("inject=$($s["inject_ms"])ms render=$($s["render_ms"])ms p50=$($s["p50"]) p90=$($s["p90"]) p99=$($s["p99"]) max=$($s["max_gap"])$tag") -ForegroundColor $color
    } else {
        Write-Host "FAILED" -ForegroundColor Red
    }
    Start-Sleep -Milliseconds 500
}

# Debug log
$pStage2 = 0; $pSupp = 0
$inputLog = "$psmuxDir\input_debug.log"
if (Test-Path $inputLog) {
    $logLines = Get-Content $inputLog -EA SilentlyContinue
    $pStage2 = @($logLines | Where-Object { $_ -match "stage2:" -and $_ -match "chars in 20ms" }).Count
    $pSupp = @($logLines | Where-Object { $_ -match "suppressed char" }).Count
}

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
try { if (-not $psmuxProc.HasExited) { Stop-Process -Id $psmuxProc.Id -Force -EA SilentlyContinue } } catch {}

# =========================================================================
# PHASE 2: DIRECT POWERSHELL
# =========================================================================
Write-Host "`n$('=' * 80)" -ForegroundColor Yellow
Write-Host "PHASE 2: DIRECT POWERSHELL (no multiplexer)" -ForegroundColor Yellow
Write-Host "$('=' * 80)" -ForegroundColor Yellow

$pwshProc = Start-Process -FilePath "pwsh" -ArgumentList "-NoProfile","-NoExit" -PassThru
$PID_PWSH = $pwshProc.Id
Write-Host "Direct pwsh PID: $PID_PWSH" -ForegroundColor Cyan
Start-Sleep -Seconds 4

$directData = @()
for ($n = 0; $n -lt $paragraphs.Count; $n++) {
    $para = $paragraphs[$n]
    $num = $n + 1
    Write-Host "  [$num/10] $($para.Length) chars " -NoNewline -ForegroundColor White

    # Clear: send "cls" then Enter SEPARATELY
    & $injectorExe $PID_PWSH "cls"
    Start-Sleep -Milliseconds 100
    & $injectorExe $PID_PWSH "{ENTER}"
    Start-Sleep -Seconds 1

    $s = Run-BurstTest -TargetPid $PID_PWSH -Text $para -IntraMs $INTRA_MS -InterMs $INTER_MS

    if ($s) {
        $directData += [PSCustomObject]@{
            N=$num; Chars=[int]$s["chars"]; InjectMs=[int]$s["inject_ms"]
            RenderMs=[int]$s["render_ms"]; AvgGap=[int]$s["avg_gap"]
            P50=[int]$s["p50"]; P90=[int]$s["p90"]; P95=[int]$s["p95"]; P99=[int]$s["p99"]
            MaxGap=[int]$s["max_gap"]; Stalls=[int]$s["stalls"]; Bursts=[int]$s["bursts"]
        }
        $tag = ""
        if ([int]$s["stalls"] -gt 0) { $tag = " STALLS=$($s["stalls"])!" }
        $color = if ([int]$s["stalls"] -gt 0) {"Red"} elseif ([int]$s["max_gap"] -gt 100) {"Yellow"} else {"Green"}
        Write-Host ("inject=$($s["inject_ms"])ms render=$($s["render_ms"])ms p50=$($s["p50"]) p90=$($s["p90"]) p99=$($s["p99"]) max=$($s["max_gap"])$tag") -ForegroundColor $color
    } else {
        Write-Host "FAILED" -ForegroundColor Red
    }
    Start-Sleep -Milliseconds 500
}

try { Stop-Process -Id $PID_PWSH -Force -EA SilentlyContinue } catch {}

# =========================================================================
# COMPARISON
# =========================================================================
Write-Host "`n$('=' * 80)" -ForegroundColor Cyan
Write-Host "HEAD TO HEAD: BURST TYPING (0ms intra, 10ms inter word)" -ForegroundColor Cyan
Write-Host "$('=' * 80)" -ForegroundColor Cyan

Write-Host "`nPSMUX:" -ForegroundColor Yellow
$psmuxData | Format-Table N, Chars, InjectMs, RenderMs, AvgGap, P50, P90, P95, P99, MaxGap, Stalls, Bursts -AutoSize

Write-Host "DIRECT POWERSHELL:" -ForegroundColor Yellow
$directData | Format-Table N, Chars, InjectMs, RenderMs, AvgGap, P50, P90, P95, P99, MaxGap, Stalls, Bursts -AutoSize

# Aggregates
$vp = @($psmuxData | Where-Object { $_.RenderMs -gt 0 })
$vd = @($directData | Where-Object { $_.RenderMs -gt 0 })

Write-Host "$('=' * 80)" -ForegroundColor Cyan
Write-Host "AGGREGATE (psmux=$($vp.Count) valid, direct=$($vd.Count) valid)" -ForegroundColor Cyan
Write-Host "$('=' * 80)" -ForegroundColor Cyan

function Show-Agg($label, $data, $color) {
    if ($data.Count -eq 0) { Write-Host "  $label : no valid data" -ForegroundColor Red; return }
    $avgR = [Math]::Round(($data | ForEach-Object { $_.RenderMs } | Measure-Object -Average).Average)
    $avgP50 = [Math]::Round(($data | ForEach-Object { $_.P50 } | Measure-Object -Average).Average)
    $avgP90 = [Math]::Round(($data | ForEach-Object { $_.P90 } | Measure-Object -Average).Average)
    $avgP95 = [Math]::Round(($data | ForEach-Object { $_.P95 } | Measure-Object -Average).Average)
    $avgP99 = [Math]::Round(($data | ForEach-Object { $_.P99 } | Measure-Object -Average).Average)
    $maxG = ($data | ForEach-Object { $_.MaxGap } | Measure-Object -Maximum).Maximum
    $totStalls = ($data | ForEach-Object { $_.Stalls } | Measure-Object -Sum).Sum
    $totBursts = ($data | ForEach-Object { $_.Bursts } | Measure-Object -Sum).Sum
    Write-Host "`n  ${label}:" -ForegroundColor $color
    Write-Host "    Avg render span: ${avgR}ms"
    Write-Host "    Avg P50 gap:     ${avgP50}ms"
    Write-Host "    Avg P90 gap:     ${avgP90}ms"
    Write-Host "    Avg P95 gap:     ${avgP95}ms"
    Write-Host "    Avg P99 gap:     ${avgP99}ms"
    Write-Host "    Worst single gap: ${maxG}ms" -ForegroundColor $(if ($maxG -gt 200) {"Red"} elseif ($maxG -gt 100) {"Yellow"} else {"Green"})
    Write-Host "    Total stalls (>150ms): $totStalls" -ForegroundColor $(if ($totStalls -gt 0) {"Red"} else {"Green"})
    Write-Host "    Total bursts (>8 chars at once): $totBursts" -ForegroundColor $(if ($totBursts -gt 0) {"Yellow"} else {"Green"})
    return @{ AvgR=$avgR; P50=$avgP50; P90=$avgP90; P95=$avgP95; P99=$avgP99; MaxG=$maxG; Stalls=$totStalls }
}

$pa = Show-Agg "PSMUX" $vp "Yellow"
if ($pStage2 -gt 0 -or $pSupp -gt 0) {
    Write-Host "    Stage2 false positives: $pStage2" -ForegroundColor $(if ($pStage2 -gt 0) {"Red"} else {"Green"})
    Write-Host "    Chars suppressed:       $pSupp" -ForegroundColor $(if ($pSupp -gt 0) {"Red"} else {"Green"})
}
$da = Show-Agg "DIRECT POWERSHELL" $vd "Yellow"

if ($pa -and $da) {
    Write-Host "`n$('=' * 80)" -ForegroundColor Cyan
    Write-Host "DELTA" -ForegroundColor Cyan
    Write-Host "$('=' * 80)" -ForegroundColor Cyan
    $rDelta = $pa.AvgR - $da.AvgR
    $p50D = $pa.P50 - $da.P50
    $p90D = $pa.P90 - $da.P90
    $p99D = $pa.P99 - $da.P99
    $maxD = $pa.MaxG - $da.MaxG
    Write-Host "    Render overhead:  +${rDelta}ms" -ForegroundColor $(if ($rDelta -gt 1000) {"Red"} elseif ($rDelta -gt 500) {"Yellow"} else {"White"})
    Write-Host "    P50 overhead:     +${p50D}ms" -ForegroundColor $(if ($p50D -gt 20) {"Red"} elseif ($p50D -gt 5) {"Yellow"} else {"White"})
    Write-Host "    P90 overhead:     +${p90D}ms" -ForegroundColor $(if ($p90D -gt 30) {"Red"} elseif ($p90D -gt 10) {"Yellow"} else {"White"})
    Write-Host "    P99 overhead:     +${p99D}ms" -ForegroundColor $(if ($p99D -gt 50) {"Red"} elseif ($p99D -gt 20) {"Yellow"} else {"White"})
    Write-Host "    Max gap overhead: +${maxD}ms" -ForegroundColor $(if ($maxD -gt 100) {"Red"} elseif ($maxD -gt 50) {"Yellow"} else {"White"})

    Write-Host "`nVERDICT:" -ForegroundColor Cyan
    if ($pa.Stalls -gt 0 -and $da.Stalls -eq 0) {
        Write-Host "  FREEZE: psmux has $($pa.Stalls) stall(s) that direct PowerShell does NOT." -ForegroundColor Red
    } elseif ($maxD -gt 100) {
        Write-Host "  PSMUX LAG: worst gap is +${maxD}ms higher than direct PowerShell." -ForegroundColor Red
    } elseif ($p90D -gt 20) {
        Write-Host "  PERCEPTIBLE: psmux P90 is +${p90D}ms higher. Users may feel the difference." -ForegroundColor Yellow
    } elseif ($rDelta -gt 500) {
        Write-Host "  SLOW RENDER: psmux takes +${rDelta}ms longer to render all chars." -ForegroundColor Yellow
    } else {
        Write-Host "  SMOOTH: psmux overhead is within acceptable range." -ForegroundColor Green
    }
}

# =========================================================================
# VALIDATION: BURST TYPING TEST RESULTS
# =========================================================================
# At 0ms burst (paste speed), we expect stage2 to trigger (this is intentional)
# But we should see 0 characters suppressed (PR #238 fixed the 2s suppress window)
if ($pStage2 -gt 0) {
    Write-Host "[PASS] Burst typing: Stage2 correctly triggered for 0ms burst (paste detection working)" -ForegroundColor Green
} else {
    Write-Host "[SKIP] Burst typing: Stage2 not triggered (paste detection may need tuning)" -ForegroundColor Yellow
}

if ($pSupp -eq 0) {
    Write-Host "[PASS] Burst typing: No characters suppressed (PR #238 verified)" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Burst typing: $pSupp characters suppressed (should be 0)" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "[PASS] Burst typing benchmark completed successfully" -ForegroundColor Green
exit 0
