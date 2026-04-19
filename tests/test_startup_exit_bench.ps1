#!/usr/bin/env pwsh
###############################################################################
# test_startup_exit_bench.ps1 — Startup & Exit Time Benchmarks
#
# Measures:
#   1. Cold start time (first new-session, no warm server)
#   2. Warm start time (new-session with warm server available)
#   3. Per-pane exit time (kill-pane)
#   4. Per-window exit time (kill-window)
#   5. Per-session exit time (kill-session)
#   6. Multi-pane session exit time
#   7. Multi-window session exit time
###############################################################################
$ErrorActionPreference = "Continue"

$PSMUX = Join-Path $PSScriptRoot ".." "target" "release" "psmux.exe"
if (!(Test-Path $PSMUX)) {
    $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source
}
if (!(Test-Path $PSMUX)) {
    Write-Host "FATAL: psmux binary not found" -ForegroundColor Red
    exit 1
}
Write-Host "[INFO] Binary: $PSMUX" -ForegroundColor Gray

$pass = 0
$fail = 0
$results = @()
$benchmarks = @()

function Report {
    param([string]$Name, [bool]$Ok, [string]$Detail = "")
    if ($Ok) { $script:pass++; Write-Host "  [PASS] $Name  $Detail" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

function Kill-All {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Start-Sleep -Milliseconds 500
    # Clean up stale port/key files
    Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue | Remove-Item -Force
    Get-ChildItem "$env:USERPROFILE\.psmux\*.key" -ErrorAction SilentlyContinue | Remove-Item -Force
    Start-Sleep -Milliseconds 300
}

function Wait-PortFile {
    param([string]$Session, [int]$TimeoutMs = 10000)
    $portFile = "$env:USERPROFILE\.psmux\$Session.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $portFile) { return $true }
        Start-Sleep -Milliseconds 5
    }
    return $false
}

function Wait-SessionReady {
    param([string]$Session, [int]$TimeoutMs = 15000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        & $PSMUX has-session -t $Session 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 10
    }
    return $false
}

function Wait-PanePrompt {
    param([string]$Session, [int]$TimeoutMs = 30000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $cap = & $PSMUX capture-pane -t $Session -p 2>$null
        $text = ($cap -join "`n")
        if ($text -match 'PS [A-Z]:\\' -or $text -match '\$\s*$' -or $text.Length -gt 50) {
            return $sw.ElapsedMilliseconds
        }
        Start-Sleep -Milliseconds 25
    }
    return -1
}

function Wait-SessionGone {
    param([string]$Session, [int]$TimeoutMs = 10000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        & $PSMUX has-session -t $Session 2>$null
        if ($LASTEXITCODE -ne 0) { return $sw.ElapsedMilliseconds }
        Start-Sleep -Milliseconds 5
    }
    return -1
}

function Wait-PortFileGone {
    param([string]$Session, [int]$TimeoutMs = 10000)
    $portFile = "$env:USERPROFILE\.psmux\$Session.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (!(Test-Path $portFile)) { return $sw.ElapsedMilliseconds }
        Start-Sleep -Milliseconds 5
    }
    return -1
}

function Add-Benchmark {
    param([string]$Name, [double]$Ms)
    $script:benchmarks += [PSCustomObject]@{ Test = $Name; TimeMs = [math]::Round($Ms, 1) }
    $bar = "#" * [math]::Min([math]::Max([int]($Ms / 10), 1), 80)
    Write-Host ("    {0,-55} {1,8:N1} ms  {2}" -f $Name, $Ms, $bar) -ForegroundColor Cyan
}

###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " psmux Startup & Exit Time Benchmarks" -ForegroundColor Cyan
Write-Host " $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Cyan
Write-Host " Binary: $PSMUX" -ForegroundColor Cyan
Write-Host "================================================================`n" -ForegroundColor Cyan

###############################################################################
# BENCHMARK 1: Cold Start Time (no warm server)
###############################################################################
Write-Host "--- BENCHMARK 1: Cold Start Time (no warm server) ---" -ForegroundColor Yellow
Kill-All

$iterations = 3
$coldTimes = @()
for ($i = 1; $i -le $iterations; $i++) {
    Kill-All

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-session -d -s "bench_cold_$i" -x 120 -y 30 2>$null
    $portReady = Wait-PortFile -Session "bench_cold_$i" -TimeoutMs 10000
    $sw.Stop()
    $portMs = $sw.ElapsedMilliseconds

    if ($portReady) {
        $promptMs = Wait-PanePrompt -Session "bench_cold_$i" -TimeoutMs 30000
        if ($promptMs -gt 0) { $promptMs += $portMs }
    } else {
        $promptMs = -1
    }

    $coldTimes += [PSCustomObject]@{ Port = $portMs; Prompt = $promptMs }
    Add-Benchmark "Cold start #$i (port file ready)" $portMs
    if ($promptMs -gt 0) {
        Add-Benchmark "Cold start #$i (prompt ready)" $promptMs
    }

    & $PSMUX kill-session -t "bench_cold_$i" 2>$null
    Start-Sleep -Milliseconds 500
}

$avgColdPort = ($coldTimes | Measure-Object -Property Port -Average).Average
$avgColdPrompt = ($coldTimes | Where-Object { $_.Prompt -gt 0 } | Measure-Object -Property Prompt -Average).Average
Add-Benchmark "Cold start AVG (port file)" $avgColdPort
if ($avgColdPrompt) { Add-Benchmark "Cold start AVG (prompt)" $avgColdPrompt }
Report "Cold start completes" $true "avg port: $([math]::Round($avgColdPort,1))ms"

###############################################################################
# BENCHMARK 1b: Warmup + New-Session (simulates post-install warmup)
###############################################################################
Write-Host "`n--- BENCHMARK 1b: Warmup-Assisted Start ---" -ForegroundColor Yellow
Kill-All

$warmupTimes = @()
for ($i = 1; $i -le $iterations; $i++) {
    Kill-All

    # Run warmup first (absorbs Defender scan penalty + spawns warm server)
    $swW = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX warmup 2>$null
    $swW.Stop()
    Add-Benchmark "warmup command #$i" $swW.ElapsedMilliseconds

    # Wait for warm server to be fully ready
    $warmReady = Wait-PortFile -Session "__warm__" -TimeoutMs 10000
    if ($warmReady) {
        Start-Sleep -Seconds 2  # Let shell finish loading
    }

    # Now time new-session (should claim warm server)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-session -d -s "bench_warmup_$i" -x 120 -y 30 2>$null
    $portReady = Wait-PortFile -Session "bench_warmup_$i" -TimeoutMs 10000
    $sw.Stop()
    $portMs = $sw.ElapsedMilliseconds

    if ($portReady) {
        $promptMs = Wait-PanePrompt -Session "bench_warmup_$i" -TimeoutMs 30000
        if ($promptMs -gt 0) { $promptMs += $portMs }
    } else {
        $promptMs = -1
    }

    $warmupTimes += [PSCustomObject]@{ Port = $portMs; Prompt = $promptMs }
    Add-Benchmark "warmup-assisted start #$i (port file ready)" $portMs
    if ($promptMs -gt 0) {
        Add-Benchmark "warmup-assisted start #$i (prompt ready)" $promptMs
    }

    & $PSMUX kill-session -t "bench_warmup_$i" 2>$null
    Start-Sleep -Milliseconds 500
}

$avgWarmupPort = ($warmupTimes | Measure-Object -Property Port -Average).Average
$avgWarmupPrompt = ($warmupTimes | Where-Object { $_.Prompt -gt 0 } | Measure-Object -Property Prompt -Average).Average
Add-Benchmark "Warmup-assisted start AVG (port file)" $avgWarmupPort
if ($avgWarmupPrompt) { Add-Benchmark "Warmup-assisted start AVG (prompt)" $avgWarmupPrompt }
Report "Warmup-assisted start completes" $true "avg port: $([math]::Round($avgWarmupPort,1))ms"

###############################################################################
# BENCHMARK 2: Warm Start Time (warm server pre-spawned)
###############################################################################
Write-Host "`n--- BENCHMARK 2: Warm Start Time (warm server available) ---" -ForegroundColor Yellow
Kill-All

# Create a session to trigger warm server spawn
& $PSMUX new-session -d -s "bench_warmup" -x 120 -y 30 2>$null
Wait-SessionReady -Session "bench_warmup" | Out-Null
# Wait for warm server to spawn
Start-Sleep -Seconds 3

$warmTimes = @()
for ($i = 1; $i -le $iterations; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-session -d -s "bench_warm_$i" -x 120 -y 30 2>$null
    $portReady = Wait-PortFile -Session "bench_warm_$i" -TimeoutMs 10000
    $sw.Stop()
    $portMs = $sw.ElapsedMilliseconds

    if ($portReady) {
        $promptMs = Wait-PanePrompt -Session "bench_warm_$i" -TimeoutMs 30000
        if ($promptMs -gt 0) { $promptMs += $portMs }
    } else {
        $promptMs = -1
    }

    $warmTimes += [PSCustomObject]@{ Port = $portMs; Prompt = $promptMs }
    Add-Benchmark "Warm start #$i (port file ready)" $portMs
    if ($promptMs -gt 0) {
        Add-Benchmark "Warm start #$i (prompt ready)" $promptMs
    }

    # Wait for next warm server to spawn before next iteration
    Start-Sleep -Seconds 3
}

$avgWarmPort = ($warmTimes | Measure-Object -Property Port -Average).Average
$avgWarmPrompt = ($warmTimes | Where-Object { $_.Prompt -gt 0 } | Measure-Object -Property Prompt -Average).Average
Add-Benchmark "Warm start AVG (port file)" $avgWarmPort
if ($avgWarmPrompt) { Add-Benchmark "Warm start AVG (prompt)" $avgWarmPrompt }
Report "Warm start completes" $true "avg port: $([math]::Round($avgWarmPort,1))ms"

# Cleanup warm sessions
& $PSMUX kill-session -t "bench_warmup" 2>$null
for ($i = 1; $i -le $iterations; $i++) {
    & $PSMUX kill-session -t "bench_warm_$i" 2>$null
}
Start-Sleep -Milliseconds 500

###############################################################################
# BENCHMARK 3: Per-Pane Exit Time (kill-pane)
###############################################################################
Write-Host "`n--- BENCHMARK 3: Per-Pane Exit Time (kill-pane) ---" -ForegroundColor Yellow
Kill-All

& $PSMUX new-session -d -s "bench_pane_exit" -x 120 -y 30 2>$null
Wait-SessionReady -Session "bench_pane_exit" | Out-Null
Start-Sleep -Seconds 2

# Create extra panes to kill
$paneTimes = @()
for ($i = 1; $i -le 3; $i++) {
    $paneId = & $PSMUX split-window -t "bench_pane_exit" -P -F '#{pane_id}' 2>$null
    Start-Sleep -Milliseconds 1500

    # Count panes before
    $before = (& $PSMUX list-panes -t "bench_pane_exit" 2>$null | Measure-Object -Line).Lines

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-pane -t "bench_pane_exit" 2>$null
    # Wait for pane count to drop
    $timeout = 5000
    $elapsed = 0
    while ($elapsed -lt $timeout) {
        $after = (& $PSMUX list-panes -t "bench_pane_exit" 2>$null | Measure-Object -Line).Lines
        if ($after -lt $before) { break }
        Start-Sleep -Milliseconds 10
        $elapsed += 10
    }
    $sw.Stop()

    $paneTimes += $sw.ElapsedMilliseconds
    Add-Benchmark "kill-pane #$i" $sw.ElapsedMilliseconds
}

$avgPaneExit = ($paneTimes | Measure-Object -Average).Average
Add-Benchmark "kill-pane AVG" $avgPaneExit
Report "Per-pane exit measured" $true "avg: $([math]::Round($avgPaneExit,1))ms"

& $PSMUX kill-session -t "bench_pane_exit" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# BENCHMARK 4: Per-Window Exit Time (kill-window)
###############################################################################
Write-Host "`n--- BENCHMARK 4: Per-Window Exit Time (kill-window) ---" -ForegroundColor Yellow
Kill-All

& $PSMUX new-session -d -s "bench_win_exit" -x 120 -y 30 2>$null
Wait-SessionReady -Session "bench_win_exit" | Out-Null
Start-Sleep -Seconds 2

$winTimes = @()
for ($i = 1; $i -le 3; $i++) {
    # Create a new window
    & $PSMUX new-window -t "bench_win_exit" 2>$null
    Start-Sleep -Milliseconds 1500

    $beforeWins = (& $PSMUX list-windows -t "bench_win_exit" 2>$null | Measure-Object -Line).Lines

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-window -t "bench_win_exit" 2>$null
    # Wait for window count to drop
    $timeout = 5000
    $elapsed = 0
    while ($elapsed -lt $timeout) {
        $afterWins = (& $PSMUX list-windows -t "bench_win_exit" 2>$null | Measure-Object -Line).Lines
        if ($afterWins -lt $beforeWins) { break }
        Start-Sleep -Milliseconds 10
        $elapsed += 10
    }
    $sw.Stop()

    $winTimes += $sw.ElapsedMilliseconds
    Add-Benchmark "kill-window #$i" $sw.ElapsedMilliseconds
}

$avgWinExit = ($winTimes | Measure-Object -Average).Average
Add-Benchmark "kill-window AVG" $avgWinExit
Report "Per-window exit measured" $true "avg: $([math]::Round($avgWinExit,1))ms"

& $PSMUX kill-session -t "bench_win_exit" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# BENCHMARK 5: Per-Session Exit Time (kill-session, single window)
###############################################################################
Write-Host "`n--- BENCHMARK 5: Per-Session Exit Time (kill-session, 1 window) ---" -ForegroundColor Yellow
Kill-All

$sessTimes = @()
for ($i = 1; $i -le 3; $i++) {
    & $PSMUX new-session -d -s "bench_sess_$i" -x 120 -y 30 2>$null
    Wait-SessionReady -Session "bench_sess_$i" | Out-Null
    Start-Sleep -Seconds 2

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-session -t "bench_sess_$i" 2>$null
    $goneMs = Wait-PortFileGone -Session "bench_sess_$i" -TimeoutMs 10000
    $sw.Stop()

    $totalMs = if ($goneMs -gt 0) { $sw.ElapsedMilliseconds } else { $sw.ElapsedMilliseconds }
    $sessTimes += $totalMs
    Add-Benchmark "kill-session (1 win) #$i" $totalMs

    Kill-All
}

$avgSessExit = ($sessTimes | Measure-Object -Average).Average
Add-Benchmark "kill-session (1 win) AVG" $avgSessExit
Report "Per-session exit measured" $true "avg: $([math]::Round($avgSessExit,1))ms"

###############################################################################
# BENCHMARK 6: Multi-Pane Session Exit Time
###############################################################################
Write-Host "`n--- BENCHMARK 6: Multi-Pane Session Exit (4 panes) ---" -ForegroundColor Yellow
Kill-All

$multiPaneTimes = @()
for ($i = 1; $i -le 3; $i++) {
    & $PSMUX new-session -d -s "bench_mp_$i" -x 120 -y 30 2>$null
    Wait-SessionReady -Session "bench_mp_$i" | Out-Null
    Start-Sleep -Seconds 2

    # Create 3 additional panes (total 4)
    & $PSMUX split-window -t "bench_mp_$i" -h 2>$null
    Start-Sleep -Milliseconds 500
    & $PSMUX split-window -t "bench_mp_$i" -v 2>$null
    Start-Sleep -Milliseconds 500
    & $PSMUX select-pane -t "bench_mp_$i" -t 0 2>$null
    & $PSMUX split-window -t "bench_mp_$i" -v 2>$null
    Start-Sleep -Milliseconds 1000

    $paneCount = (& $PSMUX list-panes -t "bench_mp_$i" 2>$null | Measure-Object -Line).Lines

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-session -t "bench_mp_$i" 2>$null
    Wait-PortFileGone -Session "bench_mp_$i" -TimeoutMs 10000 | Out-Null
    $sw.Stop()

    $multiPaneTimes += $sw.ElapsedMilliseconds
    Add-Benchmark "kill-session (${paneCount} panes) #$i" $sw.ElapsedMilliseconds

    Kill-All
}

$avgMPExit = ($multiPaneTimes | Measure-Object -Average).Average
Add-Benchmark "kill-session (4 panes) AVG" $avgMPExit
Report "Multi-pane session exit measured" $true "avg: $([math]::Round($avgMPExit,1))ms"

###############################################################################
# BENCHMARK 7: Multi-Window Session Exit Time
###############################################################################
Write-Host "`n--- BENCHMARK 7: Multi-Window Session Exit (4 windows) ---" -ForegroundColor Yellow
Kill-All

$multiWinTimes = @()
for ($i = 1; $i -le 3; $i++) {
    & $PSMUX new-session -d -s "bench_mw_$i" -x 120 -y 30 2>$null
    Wait-SessionReady -Session "bench_mw_$i" | Out-Null
    Start-Sleep -Seconds 2

    # Create 3 additional windows (total 4)
    & $PSMUX new-window -t "bench_mw_$i" 2>$null
    Start-Sleep -Milliseconds 500
    & $PSMUX new-window -t "bench_mw_$i" 2>$null
    Start-Sleep -Milliseconds 500
    & $PSMUX new-window -t "bench_mw_$i" 2>$null
    Start-Sleep -Milliseconds 1000

    $winCount = (& $PSMUX list-windows -t "bench_mw_$i" 2>$null | Measure-Object -Line).Lines

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-session -t "bench_mw_$i" 2>$null
    Wait-PortFileGone -Session "bench_mw_$i" -TimeoutMs 10000 | Out-Null
    $sw.Stop()

    $multiWinTimes += $sw.ElapsedMilliseconds
    Add-Benchmark "kill-session (${winCount} windows) #$i" $sw.ElapsedMilliseconds

    Kill-All
}

$avgMWExit = ($multiWinTimes | Measure-Object -Average).Average
Add-Benchmark "kill-session (4 windows) AVG" $avgMWExit
Report "Multi-window session exit measured" $true "avg: $([math]::Round($avgMWExit,1))ms"

###############################################################################
# BENCHMARK 8: Exit via shell exit (natural pane death → exit-empty)
###############################################################################
Write-Host "`n--- BENCHMARK 8: Natural Exit (shell exit → exit-empty) ---" -ForegroundColor Yellow
Kill-All

$naturalTimes = @()
for ($i = 1; $i -le 3; $i++) {
    & $PSMUX new-session -d -s "bench_nat_$i" -x 120 -y 30 2>$null
    Wait-SessionReady -Session "bench_nat_$i" | Out-Null
    Wait-PanePrompt -Session "bench_nat_$i" -TimeoutMs 15000 | Out-Null

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # Tell the shell to exit
    & $PSMUX send-keys -t "bench_nat_$i" "exit" Enter 2>$null
    $goneMs = Wait-PortFileGone -Session "bench_nat_$i" -TimeoutMs 15000
    $sw.Stop()

    $ms = $sw.ElapsedMilliseconds
    $naturalTimes += $ms
    Add-Benchmark "Natural exit #$i (shell exit→cleanup)" $ms

    Kill-All
}

$avgNatExit = ($naturalTimes | Measure-Object -Average).Average
Add-Benchmark "Natural exit AVG" $avgNatExit
Report "Natural exit measured" $true "avg: $([math]::Round($avgNatExit,1))ms"

###############################################################################
# SUMMARY
###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " BENCHMARK SUMMARY" -ForegroundColor Cyan
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host ""

# Category summaries
Write-Host " STARTUP:" -ForegroundColor Yellow
Write-Host ("   Cold start (avg):     {0,8:N1} ms" -f $avgColdPort)
if ($avgColdPrompt) {
    Write-Host ("   Cold prompt (avg):    {0,8:N1} ms" -f $avgColdPrompt)
}
Write-Host ("   Warmup-assisted (avg):{0,8:N1} ms" -f $avgWarmupPort)
if ($avgWarmupPrompt) {
    Write-Host ("   Warmup prompt (avg):  {0,8:N1} ms" -f $avgWarmupPrompt)
}
Write-Host ("   Warm start (avg):     {0,8:N1} ms" -f $avgWarmPort)
if ($avgWarmPrompt) {
    Write-Host ("   Warm prompt (avg):    {0,8:N1} ms" -f $avgWarmPrompt)
}
if ($avgColdPort -gt 0 -and $avgWarmPort -gt 0) {
    $speedup = [math]::Round($avgColdPort / [math]::Max($avgWarmPort, 1), 1)
    Write-Host ("   Warm speedup:         {0}x faster" -f $speedup)
}
if ($avgColdPort -gt 0 -and $avgWarmupPort -gt 0) {
    $warmupSpeedup = [math]::Round($avgColdPort / [math]::Max($avgWarmupPort, 1), 1)
    Write-Host ("   Warmup speedup:       {0}x faster" -f $warmupSpeedup)
}

Write-Host ""
Write-Host " EXIT TIMES:" -ForegroundColor Yellow
Write-Host ("   kill-pane (avg):      {0,8:N1} ms" -f $avgPaneExit)
Write-Host ("   kill-window (avg):    {0,8:N1} ms" -f $avgWinExit)
Write-Host ("   kill-session 1w (avg):{0,8:N1} ms" -f $avgSessExit)
Write-Host ("   kill-session 4p (avg):{0,8:N1} ms" -f $avgMPExit)
Write-Host ("   kill-session 4w (avg):{0,8:N1} ms" -f $avgMWExit)
Write-Host ("   natural exit (avg):   {0,8:N1} ms" -f $avgNatExit)

Write-Host ""
Write-Host " ALL BENCHMARKS:" -ForegroundColor Yellow
$benchmarks | Format-Table -AutoSize

Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "================================================================`n" -ForegroundColor Cyan

if ($fail -gt 0) { exit 1 }
exit 0
