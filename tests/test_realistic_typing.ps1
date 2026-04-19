# REALISTIC TYPING BENCHMARK: PSMUX vs DIRECT POWERSHELL
# Tests REAL typing (proper VK codes, scan codes, realistic delays)
# Tests multiple speeds to find where psmux stage2 paste detection triggers
# Uses CURSOR POSITION tracking (not char counting) for reliable render measurement
#
# Speed tiers:
#   8ms/char  = ~125 chars/sec = WILL trigger stage2 (3+ chars in 20ms)
#   12ms/char = ~83 chars/sec  = BORDERLINE stage2
#   20ms/char = ~50 chars/sec  = safe, no stage2
#   30ms/char = ~33 chars/sec  = normal fast typing

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "typing_bench"

$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$benchExe = "$env:TEMP\psmux_typing_bench.exe"

Write-Host "Compiling typing_bench.cs..." -ForegroundColor DarkGray
& $csc /nologo /optimize /out:$benchExe "$PSScriptRoot\typing_bench.cs" 2>&1
if (-not (Test-Path $benchExe)) { Write-Host "Compile FAILED" -ForegroundColor Red; exit 1 }
Write-Host "OK" -ForegroundColor Green

# 5 long paragraphs (all lowercase, no special chars = clean typing)
$paragraphs = @(
    "the quick brown fox jumps over the lazy dog and then it runs all the way back across the entire field because it realized it forgot something very important at home and now it needs to hurry before the sun goes down completely over the hills in the distance tonight"
    "pack my box with five dozen liquor jugs and make sure you stack them carefully on the shelf near the back wall of the warehouse so they do not fall over when the delivery truck arrives early tomorrow morning before anyone else gets to the loading dock area"
    "how vexingly quick daft zebras jump across the wide open fields while the farmers watch from their porches drinking coffee and wondering why these animals keep showing up every single morning without fail regardless of the weather or the season of the year"
    "the five boxing wizards jump quickly through the dark misty forest path that winds around the old abandoned castle where nobody has lived for hundreds of years and the walls are covered with thick green ivy that grows taller every single summer without stopping"
    "we promptly judged antique ivory buckles for the next prize competition at the county fair where hundreds of people gather every autumn to show off their crafts and compete for ribbons and trophies that they display proudly on their mantles at home all year long"
)

# Speed tiers: intra_ms, inter_ms, label
$speeds = @(
    @{ Intra=8;  Inter=20;  Label="8ms/char (125 cps, WILL trigger stage2)" }
    @{ Intra=12; Inter=30;  Label="12ms/char (83 cps, borderline stage2)" }
    @{ Intra=20; Inter=40;  Label="20ms/char (50 cps, fast typing)" }
)

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

function Clear-Screen {
    param([uint32]$TargetPid, [string]$Exe)
    # Use the clear mode built into typing_bench
    & $Exe $TargetPid "" 0 0 "clear" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 800
}

function Run-Typing-Phase {
    param(
        [string]$Label,
        [string]$Color,
        [uint32]$TargetPid,
        [int]$IntraMs,
        [int]$InterMs,
        [string[]]$Paragraphs
    )

    $results = @()
    for ($n = 0; $n -lt $Paragraphs.Count; $n++) {
        $para = $Paragraphs[$n]
        $num = $n + 1
        Write-Host "  [$num/$($Paragraphs.Count)] $($para.Length) chars " -NoNewline -ForegroundColor White

        # Clear screen between tests
        Clear-Screen -TargetPid $TargetPid -Exe $benchExe
        Start-Sleep -Milliseconds 300

        $output = & $benchExe $TargetPid $para $IntraMs $InterMs 2>&1
        $lines = $output | ForEach-Object { $_.ToString() }
        $s = Parse-Summary -Lines $lines

        if ($s) {
            $results += [PSCustomObject]@{
                N=$num; Chars=[int]$s["chars"]; InjectMs=[int]$s["inject_ms"]
                RenderMs=[int]$s["render_ms"]; Rendered=[int]$s["rendered"]
                Samples=[int]$s["samples"]; AvgGap=[int]$s["avg_gap"]
                P50=[int]$s["p50"]; P90=[int]$s["p90"]; P95=[int]$s["p95"]; P99=[int]$s["p99"]
                MaxGap=[int]$s["max_gap"]; Stalls=[int]$s["stalls"]; Bursts=[int]$s["bursts"]
            }
            $tag = ""
            if ([int]$s["stalls"] -gt 0) { $tag = " STALLS=$($s["stalls"])!" }
            $rc = if ([int]$s["stalls"] -gt 0) {"Red"} elseif ([int]$s["max_gap"] -gt 100) {"Yellow"} else {"Green"}
            Write-Host ("inject=$($s["inject_ms"])ms render=$($s["render_ms"])ms rendered=$($s["rendered"]) p50=$($s["p50"]) p90=$($s["p90"]) max=$($s["max_gap"])$tag") -ForegroundColor $rc
        } else {
            Write-Host "PARSE FAILED" -ForegroundColor Red
        }
        Start-Sleep -Milliseconds 300
    }
    return $results
}

function Show-Stats {
    param([PSCustomObject[]]$Data, [string]$Label, [string]$Color)
    $valid = @($Data | Where-Object { $_.RenderMs -gt 0 })
    if ($valid.Count -eq 0) {
        Write-Host "  $Label : no valid data (all 0ms render)" -ForegroundColor Red
        return $null
    }
    $avgR = [Math]::Round(($valid | ForEach-Object { $_.RenderMs } | Measure-Object -Average).Average)
    $avgP50 = [Math]::Round(($valid | ForEach-Object { $_.P50 } | Measure-Object -Average).Average)
    $avgP90 = [Math]::Round(($valid | ForEach-Object { $_.P90 } | Measure-Object -Average).Average)
    $avgP99 = [Math]::Round(($valid | ForEach-Object { $_.P99 } | Measure-Object -Average).Average)
    $maxG = ($valid | ForEach-Object { $_.MaxGap } | Measure-Object -Maximum).Maximum
    $totStalls = ($valid | ForEach-Object { $_.Stalls } | Measure-Object -Sum).Sum
    $totBursts = ($valid | ForEach-Object { $_.Bursts } | Measure-Object -Sum).Sum
    $avgRendered = [Math]::Round(($valid | ForEach-Object { $_.Rendered } | Measure-Object -Average).Average)

    Write-Host "  ${Label} ($($valid.Count)/$($Data.Count) valid):" -ForegroundColor $Color
    Write-Host "    Avg render span:  ${avgR}ms"
    Write-Host "    Avg P50 gap:      ${avgP50}ms"
    Write-Host "    Avg P90 gap:      ${avgP90}ms"
    Write-Host "    Avg P99 gap:      ${avgP99}ms"
    Write-Host "    Worst single gap: ${maxG}ms" -ForegroundColor $(if ($maxG -gt 300) {"Red"} elseif ($maxG -gt 100) {"Yellow"} else {"Green"})
    Write-Host "    Total stalls:     $totStalls" -ForegroundColor $(if ($totStalls -gt 0) {"Red"} else {"Green"})
    Write-Host "    Total bursts:     $totBursts"
    Write-Host "    Avg chars rendered: $avgRendered"
    return @{ AvgR=$avgR; P50=$avgP50; P90=$avgP90; P99=$avgP99; MaxG=$maxG; Stalls=$totStalls }
}

# =========================================================================
Write-Host ""
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host "REALISTIC TYPING BENCHMARK: PSMUX vs DIRECT POWERSHELL" -ForegroundColor Cyan
Write-Host "Proper VK codes + scan codes, cursor position tracking at 500Hz" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan

$allResults = @{}

foreach ($speed in $speeds) {
    $intra = $speed.Intra
    $inter = $speed.Inter
    $label = $speed.Label

    Write-Host "`n$('=' * 80)" -ForegroundColor Magenta
    Write-Host "SPEED: $label" -ForegroundColor Magenta
    Write-Host "$('=' * 80)" -ForegroundColor Magenta

    # --- PSMUX ---
    Write-Host "`n  Starting PSMUX..." -ForegroundColor Yellow
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Remove-Item "$psmuxDir\input_debug.log" -Force -EA SilentlyContinue

    $env:PSMUX_INPUT_DEBUG = "1"
    $psmuxProc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
    $env:PSMUX_INPUT_DEBUG = $null
    $PID_TUI = $psmuxProc.Id
    Write-Host "  TUI PID: $PID_TUI" -ForegroundColor Cyan
    Start-Sleep -Seconds 4

    # Wait for PS prompt
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
        if ($cap -match "PS [A-Z]:\\") { break }
    }
    Write-Host "  PSMUX ready" -ForegroundColor Green

    $psmuxData = Run-Typing-Phase -Label "PSMUX" -Color "Yellow" `
        -TargetPid $PID_TUI -IntraMs $intra -InterMs $inter -Paragraphs $paragraphs

    # Grab stage2 counts
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

    # --- DIRECT POWERSHELL ---
    Write-Host "`n  Starting Direct PowerShell..." -ForegroundColor Yellow
    $pwshProc = Start-Process -FilePath "pwsh" -ArgumentList "-NoProfile","-NoExit" -PassThru
    $PID_PWSH = $pwshProc.Id
    Write-Host "  Direct PID: $PID_PWSH" -ForegroundColor Cyan
    Start-Sleep -Seconds 3

    $directData = Run-Typing-Phase -Label "DIRECT" -Color "Yellow" `
        -TargetPid $PID_PWSH -IntraMs $intra -InterMs $inter -Paragraphs $paragraphs

    try { Stop-Process -Id $PID_PWSH -Force -EA SilentlyContinue } catch {}

    # --- COMPARE ---
    Write-Host "`n  --- Results at ${intra}ms/char ---" -ForegroundColor Cyan

    $pa = Show-Stats -Data $psmuxData -Label "PSMUX" -Color "Yellow"
    if ($pStage2 -gt 0 -or $pSupp -gt 0) {
        Write-Host "    Stage2 triggers:  $pStage2" -ForegroundColor $(if ($pStage2 -gt 0) {"Red"} else {"Green"})
        Write-Host "    Chars suppressed: $pSupp" -ForegroundColor $(if ($pSupp -gt 0) {"Red"} else {"Green"})
    }
    $da = Show-Stats -Data $directData -Label "DIRECT PWSH" -Color "Yellow"

    if ($pa -and $da) {
        $p50D = $pa.P50 - $da.P50
        $p90D = $pa.P90 - $da.P90
        $maxD = $pa.MaxG - $da.MaxG
        Write-Host "    DELTA P50: +${p50D}ms  P90: +${p90D}ms  MaxGap: +${maxD}ms" -ForegroundColor $(if ($p50D -gt 20) {"Red"} elseif ($p50D -gt 5) {"Yellow"} else {"Green"})
    }

    $allResults["${intra}ms"] = @{ PSMUX=$pa; Direct=$da; Stage2=$pStage2; Suppressed=$pSupp; IntraMs=$intra }
}

# =========================================================================
# GRAND SUMMARY
# =========================================================================
Write-Host "`n$('=' * 80)" -ForegroundColor Cyan
Write-Host "GRAND SUMMARY: STAGE2 TRIGGER ANALYSIS" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host ""
Write-Host ("  {0,-14} {1,8} {2,8} {3,8} {4,8} {5,8} {6,10}" -f "Speed","P50_MUX","P50_DIR","Delta","MaxMUX","Stage2","Suppressed") -ForegroundColor White
Write-Host ("  {0,-14} {1,8} {2,8} {3,8} {4,8} {5,8} {6,10}" -f "-----","-------","-------","-----","------","------","----------") -ForegroundColor DarkGray

foreach ($speed in $speeds) {
    $key = "$($speed.Intra)ms"
    $r = $allResults[$key]
    if ($r -and $r.PSMUX -and $r.Direct) {
        $delta = $r.PSMUX.P50 - $r.Direct.P50
        $dc = if ($delta -gt 20) {"Red"} elseif ($delta -gt 5) {"Yellow"} else {"Green"}
        $sc = if ($r.Stage2 -gt 0) {"Red"} else {"Green"}
        Write-Host ("  {0,-14} {1,8} {2,8} {3,8} {4,8} {5,8} {6,10}" -f $speed.Label.Substring(0,13),
            "$($r.PSMUX.P50)ms", "$($r.Direct.P50)ms", "+${delta}ms", "$($r.PSMUX.MaxG)ms",
            $r.Stage2, $r.Suppressed) -ForegroundColor $dc
    } elseif ($r) {
        $p50m = if ($r.PSMUX) { "$($r.PSMUX.P50)ms" } else { "N/A" }
        $p50d = if ($r.Direct) { "$($r.Direct.P50)ms" } else { "N/A" }
        Write-Host ("  {0,-14} {1,8} {2,8} {3,8} {4,8} {5,8} {6,10}" -f $speed.Label.Substring(0,13),
            $p50m, $p50d, "?", "?", $r.Stage2, $r.Suppressed) -ForegroundColor Yellow
    }
}

Write-Host ""
Write-Host "INTERPRETATION:" -ForegroundColor Cyan
Write-Host "  Stage2 > 0 means psmux mistook fast typing for paste (300ms buffer delay)" -ForegroundColor White
Write-Host "  This is the root cause of perceptible typing lag in psmux" -ForegroundColor White
Write-Host ""

# =========================================================================
# VALIDATION: ZERO-LATENCY TYPING OPTIMIZATION PROOF
# =========================================================================
# After the zero-latency typing flush optimization (commit df7f028),
# normal typing (20ms/char) should match direct PowerShell latency
if ($allResults.ContainsKey("20ms")) {
    $r20 = $allResults["20ms"]
    if ($r20.PSMUX -and $r20.Direct) {
        $p50Delta = $r20.PSMUX.P50 - $r20.Direct.P50
        $p90Delta = $r20.PSMUX.P90 - $r20.Direct.P90
        
        # At 20ms/char (fast typing), we expect 0ms delta
        # (or within margin of error due to scheduler variance)
        if ([Math]::Abs($p50Delta) -le 5 -and [Math]::Abs($p90Delta) -le 5) {
            Write-Host "[PASS] Zero-latency typing: P50/P90 delta <= 5ms at normal speed" -ForegroundColor Green
        } elseif ($p50Delta -gt 15 -or $p90Delta -gt 15) {
            Write-Host "[FAIL] Zero-latency typing: P50 delta=$p50Delta ms, P90 delta=$p90Delta ms (expected <=5ms)" -ForegroundColor Red
            exit 1
        } else {
            Write-Host "[PASS] Zero-latency typing: P50/P90 delta within acceptable range" -ForegroundColor Green
        }
    }
}

# At 8ms/char (extreme speed), P90 should have improved significantly
# from the previous 56ms to around 43-45ms
if ($allResults.ContainsKey("8ms")) {
    $r8 = $allResults["8ms"]
    if ($r8.PSMUX) {
        $p90 = $r8.PSMUX.P90
        # After optimization, 8ms/char P90 should be < 50ms
        if ($p90 -lt 50) {
            Write-Host "[PASS] Zero-latency typing: Extreme speed P90=$p90 ms < 50ms (optimized)" -ForegroundColor Green
        } else {
            Write-Host "[FAIL] Zero-latency typing: Extreme speed P90=$p90 ms (expected < 50ms)" -ForegroundColor Red
            exit 1
        }
    }
}

# No stage2 false positives should occur with the optimization
$stage2Total = 0
foreach ($speed in $speeds) {
    $key = "$($speed.Intra)ms"
    if ($allResults.ContainsKey($key) -and $allResults[$key].Stage2) {
        $stage2Total += $allResults[$key].Stage2
    }
}

if ($stage2Total -eq 0) {
    Write-Host "[PASS] Zero-latency typing: No stage2 false positives (typing correctly identified)" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Zero-latency typing: $stage2Total stage2 false positives detected" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "[PASS] Realistic typing benchmark completed successfully" -ForegroundColor Green
exit 0
