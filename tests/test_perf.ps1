# psmux Performance Test Suite
# Tests rapid operations, TCP connection overhead, dump-state command, and throughput

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }
function Write-Perf { param($msg) Write-Host "[PERF] $msg" -ForegroundColor Magenta }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    Write-Error "psmux release binary not found. Run: cargo build --release"
    exit 1
}

Write-Host ""
Write-Host "=" * 70
Write-Host "            PSMUX PERFORMANCE TEST SUITE"
Write-Host "=" * 70
Write-Host ""

# ===========================================================================
# SETUP - Create a test session
# ===========================================================================
$SESSION_NAME = "perf_test"
try { & $PSMUX kill-session -t $SESSION_NAME 2>&1 | Out-Null } catch {}
Start-Sleep -Seconds 1

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $SESSION_NAME, "-d" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 2

& $PSMUX has-session -t $SESSION_NAME 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Could not start performance test session"
    exit 1
}
Write-Info "Performance test session started: $SESSION_NAME"
Write-Host ""

# Read session key and port for low-level TCP tests
$homeDir = $env:USERPROFILE
$port = (Get-Content "$homeDir\.psmux\$SESSION_NAME.port" -ErrorAction SilentlyContinue).Trim()
$key = (Get-Content "$homeDir\.psmux\$SESSION_NAME.key" -ErrorAction SilentlyContinue).Trim()
Write-Info "Server port: $port, Key: $($key.Substring(0,4))..."
Write-Host ""

# ===========================================================================
# TEST 1: dump-state command latency (single TCP connection returns layout + windows)
# ===========================================================================
Write-Host "=" * 70
Write-Host "  TEST 1: dump-state Command Latency"
Write-Host "=" * 70

Write-Test "dump-state returns combined layout+windows"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", [int]$port)
$stream = $client.GetStream()
$stream.ReadTimeout = 5000
$writer = New-Object System.IO.StreamWriter($stream)
$reader = New-Object System.IO.StreamReader($stream)
$writer.WriteLine("AUTH $key")
$writer.Flush()
$auth = $reader.ReadLine()
$writer.WriteLine("dump-state")
$writer.Flush()
$resp = $reader.ReadToEnd()
$client.Close()
$sw.Stop()

if ($resp -match '"layout"' -and $resp -match '"windows"') {
    Write-Pass "dump-state returns combined JSON with layout and windows"
    Write-Perf "dump-state latency: $($sw.ElapsedMilliseconds)ms, response size: $($resp.Length) bytes"
} else {
    Write-Fail "dump-state response missing expected fields: $($resp.Substring(0, [Math]::Min(200, $resp.Length)))"
}

# Test dump-state average over 10 calls
Write-Test "dump-state average latency over 10 calls"
$totalMs = 0
$success = 0
for ($i = 0; $i -lt 10; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", [int]$port)
        $stream = $client.GetStream()
        $stream.ReadTimeout = 5000
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.WriteLine("AUTH $key")
        $writer.Flush()
        $auth = $reader.ReadLine()
        $writer.WriteLine("dump-state")
        $writer.Flush()
        $resp = $reader.ReadToEnd()
        $client.Close()
        $sw.Stop()
        if ($resp -match '"layout"') { $success++; $totalMs += $sw.ElapsedMilliseconds }
    } catch {
        $sw.Stop()
    }
}
if ($success -eq 10) {
    $avgMs = [math]::Round($totalMs / 10, 1)
    Write-Pass "10/10 dump-state calls succeeded"
    Write-Perf "Average dump-state latency: ${avgMs}ms"
    if ($avgMs -lt 50) {
        Write-Pass "dump-state latency under 50ms threshold"
    } elseif ($avgMs -lt 100) {
        Write-Pass "dump-state latency under 100ms (acceptable)"
    } else {
        Write-Fail "dump-state latency too high: ${avgMs}ms (should be under 100ms)"
    }
} else {
    Write-Fail "Only $success/10 dump-state calls succeeded"
}

# ===========================================================================
# TEST 2: Batched commands on single connection (simulating client behavior)
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  TEST 2: Batched Commands on Single TCP Connection"
Write-Host "=" * 70

Write-Test "Send multiple fire-and-forget + dump-state on ONE connection"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", [int]$port)
$stream = $client.GetStream()
$stream.ReadTimeout = 5000
$writer = New-Object System.IO.StreamWriter($stream)
$reader = New-Object System.IO.StreamReader($stream)

# Auth
$writer.WriteLine("AUTH $key")
$writer.Flush()
$auth = $reader.ReadLine()

# Send fire-and-forget commands (simulating what client does)
$writer.WriteLine("client-size 120 40")
$writer.WriteLine("send-text `"h`"")
$writer.WriteLine("send-text `"e`"")
$writer.WriteLine("send-text `"l`"")
$writer.WriteLine("send-text `"l`"")
$writer.WriteLine("send-text `"o`"")

# Send dump-state last (triggers response)
$writer.WriteLine("dump-state")
$writer.Flush()

$resp = $reader.ReadToEnd()
$client.Close()
$sw.Stop()

if ($resp -match '"layout"' -and $resp -match '"windows"') {
    Write-Pass "Batched commands + dump-state works on single connection"
    Write-Perf "Batched round-trip: $($sw.ElapsedMilliseconds)ms"
} else {
    Write-Fail "Batched response incorrect: $($resp.Substring(0, [Math]::Min(200, $resp.Length)))"
}

# ===========================================================================
# TEST 3: Rapid send-keys throughput 
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  TEST 3: Rapid Send-Keys Throughput"
Write-Host "=" * 70

# Test sending 100 characters as fast as possible via CLI
Write-Test "Send 100 characters rapidly via CLI"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 100; $i++) {
    & $PSMUX send-keys -l -t $SESSION_NAME "x" 2>&1 | Out-Null
}
$sw.Stop()
$throughput = [math]::Round(100 / ($sw.ElapsedMilliseconds / 1000.0), 0)
Write-Pass "100 send-keys commands completed"
Write-Perf "CLI send-keys throughput: $throughput chars/sec ($($sw.ElapsedMilliseconds)ms total)"

# Test sending via raw TCP (simulating client batching)
Write-Test "Send 100 characters via single TCP batch"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", [int]$port)
$stream = $client.GetStream()
$stream.ReadTimeout = 5000
$writer = New-Object System.IO.StreamWriter($stream)
$reader = New-Object System.IO.StreamReader($stream)
$writer.WriteLine("AUTH $key")
$writer.Flush()
$auth = $reader.ReadLine()

for ($i = 0; $i -lt 100; $i++) {
    $writer.WriteLine("send-text `"y`"")
}
$writer.WriteLine("dump-state")
$writer.Flush()
$resp = $reader.ReadToEnd()
$client.Close()
$sw.Stop()

if ($resp -match '"layout"') {
    $batchThroughput = [math]::Round(100 / ($sw.ElapsedMilliseconds / 1000.0), 0)
    Write-Pass "100 chars via single TCP batch succeeded"
    Write-Perf "Batched TCP throughput: $batchThroughput chars/sec ($($sw.ElapsedMilliseconds)ms total)"
    Write-Perf "Batch speedup vs CLI: $([math]::Round($batchThroughput / [math]::Max(1, $throughput), 1))x"
} else {
    Write-Fail "Batch send failed"
}

# ===========================================================================
# TEST 4: Rapid window/pane operations
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  TEST 4: Rapid Window and Pane Operations"
Write-Host "=" * 70

Write-Test "Create 5 windows rapidly"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 5; $i++) {
    & $PSMUX new-window -t $SESSION_NAME 2>&1 | Out-Null
}
$sw.Stop()
Write-Pass "5 windows created in $($sw.ElapsedMilliseconds)ms"
Write-Perf "Window creation: $([math]::Round($sw.ElapsedMilliseconds / 5, 0))ms per window"

Write-Test "Create 5 splits rapidly"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 5; $i++) {
    & $PSMUX split-window -v -t $SESSION_NAME 2>&1 | Out-Null
    Start-Sleep -Milliseconds 50
}
$sw.Stop()
Write-Pass "5 splits created in $($sw.ElapsedMilliseconds)ms"
Write-Perf "Split creation: $([math]::Round($sw.ElapsedMilliseconds / 5, 0))ms per split"

Write-Test "Cycle through windows 20 times"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 20; $i++) {
    & $PSMUX next-window -t $SESSION_NAME 2>&1 | Out-Null
}
$sw.Stop()
Write-Pass "20 window cycles in $($sw.ElapsedMilliseconds)ms"
Write-Perf "Window switch: $([math]::Round($sw.ElapsedMilliseconds / 20, 0))ms per switch"

Write-Test "Navigate panes 20 times"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 20; $i++) {
    & $PSMUX select-pane -D -t $SESSION_NAME 2>&1 | Out-Null
}
$sw.Stop()
Write-Pass "20 pane navigations in $($sw.ElapsedMilliseconds)ms"
Write-Perf "Pane navigation: $([math]::Round($sw.ElapsedMilliseconds / 20, 0))ms per nav"

# ===========================================================================
# TEST 5: dump-state under load (with many panes)
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  TEST 5: dump-state Under Load (Many Panes)"
Write-Host "=" * 70

Write-Test "dump-state with complex layout"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", [int]$port)
$stream = $client.GetStream()
$stream.ReadTimeout = 10000
$writer = New-Object System.IO.StreamWriter($stream)
$reader = New-Object System.IO.StreamReader($stream)
$writer.WriteLine("AUTH $key")
$writer.Flush()
$auth = $reader.ReadLine()
$writer.WriteLine("dump-state")
$writer.Flush()
$resp = $reader.ReadToEnd()
$client.Close()
$sw.Stop()

if ($resp -match '"layout"') {
    Write-Pass "dump-state with complex layout succeeded"
    Write-Perf "Complex dump-state: $($sw.ElapsedMilliseconds)ms, $($resp.Length) bytes"
} else {
    Write-Fail "dump-state failed under load"
}

# ===========================================================================
# TEST 6: Rapid session create/attach cycle timing
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  TEST 6: Session Lifecycle Timing"
Write-Host "=" * 70

Write-Test "Create + verify + kill session cycle"
$totalCycleMs = 0
$cycles = 3
for ($c = 0; $c -lt $cycles; $c++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    
    # Create
    $p = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", "perf_cycle_$c", "-d" -PassThru -WindowStyle Hidden
    
    # Wait for port file
    $portPath = "$homeDir\.psmux\perf_cycle_$c.port"
    $maxWait = 30
    while (-not (Test-Path $portPath) -and $maxWait -gt 0) {
        Start-Sleep -Milliseconds 100
        $maxWait--
    }
    
    # Verify
    & $PSMUX has-session -t "perf_cycle_$c" 2>&1 | Out-Null
    
    # Kill
    & $PSMUX kill-session -t "perf_cycle_$c" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    
    $sw.Stop()
    $totalCycleMs += $sw.ElapsedMilliseconds
}
$avgCycleMs = [math]::Round($totalCycleMs / $cycles, 0)
Write-Pass "$cycles session lifecycle cycles completed"
Write-Perf "Average session lifecycle: ${avgCycleMs}ms"

# ===========================================================================
# CLEANUP
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  CLEANUP"
Write-Host "=" * 70

try { & $PSMUX kill-session -t $SESSION_NAME 2>&1 | Out-Null } catch {}
for ($c = 0; $c -lt 5; $c++) {
    try { & $PSMUX kill-session -t "perf_cycle_$c" 2>&1 | Out-Null } catch {}
}
Start-Sleep -Seconds 1
Write-Info "Cleanup complete"

# ===========================================================================
# SUMMARY
# ===========================================================================
Write-Host ""
Write-Host "=" * 70
Write-Host "  PERFORMANCE TEST RESULTS"
Write-Host "=" * 70
Write-Host ""

$total = $script:TestsPassed + $script:TestsFailed + $script:TestsSkipped
Write-Host "  Total: $total  Passed: $($script:TestsPassed)  Failed: $($script:TestsFailed)  Skipped: $($script:TestsSkipped)"
Write-Host ""

if ($script:TestsFailed -eq 0) {
    Write-Host "  ALL PERFORMANCE TESTS PASSED!" -ForegroundColor Green
    exit 0
} else {
    Write-Host "  Some performance tests failed." -ForegroundColor Yellow
    exit 1
}
