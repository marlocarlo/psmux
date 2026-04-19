# bench_squelch_cwd.ps1 - Benchmark warm session claim with CWD change (squelch)
#
# Measures the time from launching psmux (from a different directory than the
# warm server) until a clean prompt appears.  This specifically exercises the
# squelch path: cd + cls injection, blank frame suppression, and event-driven
# unsquelch via CSI 2J/3J detection.
#
# What it measures:
#   1. Warm claim wall time (new-session -d returns)
#   2. Time to clean prompt after CWD change (squelch lift)
#   3. Whether cd command text leaks into captured pane output
#   4. Correctness: pane CWD matches requested directory
#
# Usage:
#   .\tests\bench_squelch_cwd.ps1 [-Iterations 10] [-Verbose]

param(
    [int]$Iterations = 5,
    [int]$TimeoutSec = 15,
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"

$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = Join-Path $PSScriptRoot "..\target\release\tmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source
}
if (-not $PSMUX -or -not (Test-Path $PSMUX)) {
    Write-Host "ERROR: Cannot find psmux.exe" -ForegroundColor Red
    exit 1
}
$PSMUX = (Resolve-Path $PSMUX).Path

$HOME_DIR = $env:USERPROFILE
$PSMUX_DIR = "$HOME_DIR\.psmux"

# Pick a target directory that differs from the workspace
$TEST_CWD = $env:TEMP
if (-not (Test-Path $TEST_CWD)) { $TEST_CWD = "C:\" }
$ORIGINAL_CWD = (Get-Location).Path

# ── Helpers ──

function Write-Header { param([string]$text)
    Write-Host ""
    Write-Host ("=" * 76) -ForegroundColor Cyan
    Write-Host "  $text" -ForegroundColor Cyan
    Write-Host ("=" * 76) -ForegroundColor Cyan
}

function Write-Metric { param([string]$label, [double]$ms)
    $color = if ($ms -lt 200) { "Green" } elseif ($ms -lt 500) { "Yellow" } else { "Red" }
    Write-Host ("    {0,-52} {1,8:N1} ms" -f $label, $ms) -ForegroundColor $color
}

function Write-Summary { param([string]$label, [double[]]$values)
    if ($values.Count -eq 0) { Write-Host "    $label  NO DATA" -ForegroundColor Red; return }
    $sorted = $values | Sort-Object
    $avg = [math]::Round(($sorted | Measure-Object -Average).Average, 1)
    $min = $sorted[0]
    $max = $sorted[-1]
    $p95idx = [math]::Min([math]::Floor($sorted.Count * 0.95), $sorted.Count - 1)
    $p95 = $sorted[$p95idx]
    $med = $sorted[[math]::Floor($sorted.Count / 2)]
    Write-Host ("    {0,-32} avg={1,7:N1}  min={2,7:N1}  med={3,7:N1}  p95={4,7:N1}  max={5,7:N1}  (n={6})" `
        -f $label, $avg, $min, $med, $p95, $max, $sorted.Count) -ForegroundColor White
}

function Kill-All-Psmux {
    Get-Process psmux, pmux, tmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    if (Test-Path $PSMUX_DIR) {
        Remove-Item "$PSMUX_DIR\bench_sq_*.port" -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\bench_sq_*.key"  -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\__warm__.port" -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\__warm__.key"  -Force -ErrorAction SilentlyContinue
    }
}

function Wait-PortFile {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw -ErrorAction SilentlyContinue)
            if ($port -and $port.Trim() -match '^\d+$') { return @{ Port = [int]$port.Trim(); Ms = $sw.ElapsedMilliseconds } }
        }
        Start-Sleep -Milliseconds 2
    }
    return $null
}

function Wait-SessionAlive {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw -ErrorAction SilentlyContinue)
            if ($port -and $port.Trim() -match '^\d+$') {
                try {
                    $tcp = New-Object System.Net.Sockets.TcpClient
                    $tcp.Connect("127.0.0.1", [int]$port.Trim())
                    $tcp.Close()
                    return @{ Port = [int]$port.Trim(); Ms = $sw.ElapsedMilliseconds }
                } catch {}
            }
        }
        Start-Sleep -Milliseconds 5
    }
    return $null
}

function Wait-PanePrompt {
    param([string]$Target, [int]$TimeoutMs = 20000, [string]$Pattern = "PS [A-Z]:\\")
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            if ($cap -match $Pattern) { return @{ Found = $true; Ms = $sw.ElapsedMilliseconds; Content = $cap } }
        } catch {}
        Start-Sleep -Milliseconds 25
    }
    return @{ Found = $false; Ms = $sw.ElapsedMilliseconds; Content = "" }
}

# ── Banner ──

Write-Host ""
Write-Host ("*" * 76) -ForegroundColor Magenta
Write-Host "    PSMUX SQUELCH + CWD CHANGE BENCHMARK" -ForegroundColor Magenta
Write-Host "    $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')  |  Iterations: $Iterations" -ForegroundColor Magenta
Write-Host "    Binary: $PSMUX" -ForegroundColor Magenta
Write-Host "    Test CWD: $TEST_CWD" -ForegroundColor Magenta
Write-Host ("*" * 76) -ForegroundColor Magenta

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 1: WARM CLAIM WITH CWD CHANGE (squelch active)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "1. WARM CLAIM WITH CWD CHANGE (squelch path)"

Kill-All-Psmux

# Create base session (spawns warm server from ORIGINAL_CWD)
$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s bench_sq_base -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$baseInfo = Wait-SessionAlive -SessionName "bench_sq_base" -TimeoutMs 15000
if ($null -eq $baseInfo) {
    Write-Host "    [FAIL] Could not start base session" -ForegroundColor Red
    exit 1
}

# Wait for warm server to spawn
Start-Sleep -Seconds 4

$claimTimes    = @()
$promptTimes   = @()
$leakCount     = 0
$cwdMatchCount = 0
$totalRuns     = 0

for ($i = 0; $i -lt $Iterations; $i++) {
    # Ensure warm server is ready
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -eq $warmReady) {
        Write-Host "    [SKIP] Warm server not available for run #$($i+1)" -ForegroundColor Yellow
        continue
    }

    $sess = "bench_sq_cwd_$i"
    $totalRuns++

    # Change to TEST_CWD before launching (triggers squelch path)
    Push-Location $TEST_CWD

    $swTotal = [System.Diagnostics.Stopwatch]::StartNew()

    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null

    $swTotal.Stop()
    $claimMs = $swTotal.ElapsedMilliseconds
    $claimTimes += $claimMs

    Pop-Location

    # Measure time to clean prompt (this is the squelch lift latency)
    $prompt = Wait-PanePrompt -Target $sess -TimeoutMs ($TimeoutSec * 1000)
    if ($prompt.Found) {
        $promptMs = $claimMs + $prompt.Ms
        $promptTimes += $promptMs
        Write-Metric "  Run #$($i+1): claim=$($claimMs)ms, prompt" $promptMs

        # Check for command leaks in pane content
        $cap = & $PSMUX capture-pane -t $sess -p 2>&1 | Out-String
        if ($cap -match " cd '") {
            $leakCount++
            Write-Host "    [LEAK] Run #$($i+1): cd command visible in pane output!" -ForegroundColor Red
            if ($Verbose) { Write-Host $cap -ForegroundColor DarkGray }
        }

        # Check CWD correctness
        $expectedPath = (Resolve-Path $TEST_CWD).Path.TrimEnd('\')
        if ($cap -match [regex]::Escape($expectedPath)) {
            $cwdMatchCount++
        } elseif ($cap -match [regex]::Escape($TEST_CWD.TrimEnd('\'))) {
            $cwdMatchCount++
        } else {
            Write-Host "    [WARN] Run #$($i+1): Prompt CWD may not match $TEST_CWD" -ForegroundColor Yellow
            if ($Verbose) { Write-Host $cap -ForegroundColor DarkGray }
        }
    } else {
        Write-Host "    [TIMEOUT] Run #$($i+1): No prompt within ${TimeoutSec}s" -ForegroundColor Red
    }

    # Wait for next warm server
    Start-Sleep -Seconds 4
}

# ── Results ──

Write-Host ""
Write-Header "RESULTS"
Write-Summary "Claim time" $claimTimes
Write-Summary "Time to prompt (claim+squelch)" $promptTimes
Write-Host ""

$passColor = if ($leakCount -eq 0) { "Green" } else { "Red" }
Write-Host ("    Command leak check:   {0}/{1} runs clean" -f ($totalRuns - $leakCount), $totalRuns) -ForegroundColor $passColor
$cwdColor = if ($cwdMatchCount -eq $totalRuns) { "Green" } else { "Yellow" }
Write-Host ("    CWD correctness:      {0}/{1} runs correct" -f $cwdMatchCount, $totalRuns) -ForegroundColor $cwdColor

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 2: WARM CLAIM SAME CWD (no squelch, baseline comparison)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "2. WARM CLAIM SAME CWD (no squelch, baseline)"

# Kill everything and restart from ORIGINAL_CWD
Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s bench_sq_base2 -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$baseInfo2 = Wait-SessionAlive -SessionName "bench_sq_base2" -TimeoutMs 15000
if ($null -eq $baseInfo2) {
    Write-Host "    [FAIL] Could not start base session" -ForegroundColor Red
} else {
    Start-Sleep -Seconds 4

    $baseClaimTimes  = @()
    $basePromptTimes = @()

    for ($i = 0; $i -lt $Iterations; $i++) {
        $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
        if ($null -eq $warmReady) {
            Write-Host "    [SKIP] Warm server not available for run #$($i+1)" -ForegroundColor Yellow
            continue
        }

        $sess = "bench_sq_same_$i"
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        $sw.Stop()
        $baseClaimTimes += $sw.ElapsedMilliseconds

        $prompt = Wait-PanePrompt -Target $sess -TimeoutMs ($TimeoutSec * 1000)
        if ($prompt.Found) {
            $totalMs = $sw.ElapsedMilliseconds + $prompt.Ms
            $basePromptTimes += $totalMs
            Write-Metric "  Run #$($i+1): claim=$($sw.ElapsedMilliseconds)ms, prompt" $totalMs
        } else {
            Write-Host "    [TIMEOUT] Run #$($i+1)" -ForegroundColor Red
        }

        Start-Sleep -Seconds 4
    }

    Write-Summary "Claim time (same CWD)" $baseClaimTimes
    Write-Summary "Time to prompt (same CWD)" $basePromptTimes
}

Pop-Location

# ── Overhead comparison ──
if ($promptTimes.Count -gt 0 -and $basePromptTimes.Count -gt 0) {
    $squelchAvg = [math]::Round(($promptTimes | Measure-Object -Average).Average, 1)
    $baseAvg    = [math]::Round(($basePromptTimes | Measure-Object -Average).Average, 1)
    $overhead   = [math]::Round($squelchAvg - $baseAvg, 1)
    Write-Host ""
    Write-Header "OVERHEAD ANALYSIS"
    Write-Host ("    Squelch path avg:     {0,7:N1} ms" -f $squelchAvg) -ForegroundColor White
    Write-Host ("    No squelch avg:       {0,7:N1} ms" -f $baseAvg) -ForegroundColor White
    $ohColor = if ($overhead -lt 100) { "Green" } elseif ($overhead -lt 300) { "Yellow" } else { "Red" }
    Write-Host ("    CWD change overhead:  {0,7:N1} ms" -f $overhead) -ForegroundColor $ohColor
}

# ── Cleanup ──
Write-Host ""
Kill-All-Psmux
Write-Host "    Cleanup complete." -ForegroundColor DarkGray
Write-Host ""
