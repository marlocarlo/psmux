#!/usr/bin/env pwsh
# Test latency for the EXACT user scenario:
#   psmux -> ConPTY(pwsh) -> user types "wsl" -> bash
# This matches: "nesting wsl inside pwsh inside psmux"

param(
    [int]$Chars = 60,
    [int]$DelayMs = 100  # inter-key delay
)

$ErrorActionPreference = "Stop"

# WSL availability check
$wslExe = "$env:SystemRoot\System32\wsl.exe"
if (-not (Test-Path $wslExe)) {
    Write-Host "[SKIP] WSL not available (wsl.exe not found)" -ForegroundColor Yellow
    exit 0
}
$distroCheck = & $wslExe --list --quiet 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $distroCheck.Trim().Length -eq 0) {
    Write-Host "[SKIP] WSL not available (no distro installed)" -ForegroundColor Yellow
    exit 0
}

$psmux = "$PSScriptRoot\..\target\release\psmux.exe"
$session = "wslpwsh_$(Get-Random -Max 9999)"
$home_ = $env:USERPROFILE

Write-Host "=== WSL-inside-pwsh-inside-psmux Latency Test ==="
Write-Host "  Session: $session, Chars: $Chars, Delay: ${DelayMs}ms"

# 1. Kill old server, start fresh
& $psmux kill-server 2>$null
Start-Sleep 2
Remove-Item "$home_\.psmux\*.port" -Force -ea 0
Remove-Item "$home_\.psmux\*.key" -Force -ea 0

# 2. Create session with DEFAULT shell (pwsh) — NOT wsl directly
Write-Host "`n[1] Starting psmux session (default shell = pwsh)..."
& $psmux new-session -d -s $session
Start-Sleep 3

$portFile = "$home_\.psmux\$session.port"
$keyFile = "$home_\.psmux\$session.key"
if (!(Test-Path $portFile)) { Write-Host "ERROR: No port file"; exit 1 }
$port = [int](Get-Content $portFile).Trim()
$key = (Get-Content $keyFile).Trim()
Write-Host "  Server on port $port"

# 3. Connect TCP
$tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", $port)
$tcp.ReceiveTimeout = 10000
$tcp.SendTimeout = 5000
$stream = $tcp.GetStream()
$writer = [System.IO.StreamWriter]::new($stream)
$reader = [System.IO.StreamReader]::new($stream)
$writer.AutoFlush = $true

$writer.WriteLine("AUTH $key")
$writer.Flush()
$auth = $reader.ReadLine()
Write-Host "  Auth: $auth"
if ($auth -ne "OK") { Write-Host "Auth failed!"; exit 1 }
$writer.WriteLine("PERSISTENT")
$writer.Flush()
Start-Sleep -Milliseconds 200

# Set terminal size
$writer.WriteLine("client-size 120 30")
$writer.Flush()

# 4. Type "wsl" + Enter inside the pwsh pane
Write-Host "`n[2] Typing 'wsl' + Enter inside pwsh pane..."
$writer.WriteLine("send-keys w s l Enter")
Write-Host "  Waiting 5s for WSL to start..."
Start-Sleep 5

# Verify WSL started by checking dump-state
$writer.WriteLine("dump-state")
$ds = $reader.ReadLine()
if ($ds -eq "NC") {
    $writer.WriteLine("dump-state")
    $ds = $reader.ReadLine()
}
Write-Host "  dump-state len=$($ds.Length)"

# 5. Disable status bar clock
Write-Host "`n[3] Disabling status bar clock..."
$writer.WriteLine("set status-right `"`"")
$writer.WriteLine("set status-left `"test`"")
Start-Sleep 1

# 6. Get a fresh baseline
$writer.WriteLine("dump-state")
$baseline = $reader.ReadLine()
if ($baseline -eq "NC") { $writer.WriteLine("dump-state"); $baseline = $reader.ReadLine() }

# Helper: wait for dump-state to change (with timeout)
function WaitForChange($writer, $reader, $before, $timeoutMs = 2000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        Start-Sleep -Milliseconds 2  # Don't hammer the server
        $writer.WriteLine("dump-state")
        $resp = $reader.ReadLine()
        if ($resp -ne "NC" -and $resp -ne $before) {
            return @{ Ms = $sw.Elapsed.TotalMilliseconds; TimedOut = $false; Response = $resp }
        }
    }
    return @{ Ms = $timeoutMs; TimedOut = $true; Response = $before }
}

# 7. Type chars and measure
Write-Host "`n[4] Typing $Chars chars (${DelayMs}ms gap). Measuring echo latency..."
Write-Host "    Pipeline: keystroke -> TCP -> server -> ConPTY(pwsh/wsl) -> echo -> JSON -> TCP"

$alphabet = "abcdefghijklmnopqrstuvwxyz0123456789"
$results = @()
$timeouts = 0

# Get fresh state before each char
$writer.WriteLine("dump-state")
$prev = $reader.ReadLine()
if ($prev -eq "NC") { $writer.WriteLine("dump-state"); $prev = $reader.ReadLine() }

for ($i = 0; $i -lt $Chars; $i++) {
    $c = $alphabet[$i % $alphabet.Length]
    
    # Send the character
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $writer.WriteLine("send-text `"$c`"")
    
    # Wait for screen to change (poll every 2ms, timeout 2s)
    $result = WaitForChange $writer $reader $prev 2000
    $sw.Stop()
    $ms = [math]::Round($result.Ms, 1)
    
    if ($result.TimedOut) {
        $timeouts++
        if ($timeouts -le 3) { Write-Host "  TIMEOUT at char $i ('$c')" }
    }
    
    $results += $ms
    $prev = $result.Response
    
    # Print batch stats
    if (($i + 1) % 10 -eq 0) {
        $batch = $results[($i-9)..$i]
        $avg = [math]::Round(($batch | Measure-Object -Average).Average, 1)
        $max = [math]::Round(($batch | Measure-Object -Maximum).Maximum, 1)
        $min = [math]::Round(($batch | Measure-Object -Minimum).Minimum, 1)
        Write-Host ("  [{0,3}-{1,3}] avg={2,7:F1}ms  min={3,7:F1}ms  max={4,7:F1}ms" -f ($i-8), ($i+1), $avg, $min, $max)
    }
    
    # Inter-key delay (like real typing)
    if ($DelayMs -gt 0) { Start-Sleep -Milliseconds $DelayMs }
}

try { $tcp.Close() } catch {}

# 8. Results
Write-Host "`n=== RESULTS: WSL inside pwsh inside psmux ==="
$sorted = $results | Sort-Object
$avg = [math]::Round(($results | Measure-Object -Average).Average, 1)
$p50idx = [math]::Floor($sorted.Count * 0.5)
$p90idx = [math]::Floor($sorted.Count * 0.9)
$p99idx = [math]::Floor($sorted.Count * 0.99)
$p50 = $sorted[$p50idx]
$p90 = $sorted[$p90idx]
$max = $sorted[-1]
$min = $sorted[0]

Write-Host "  Avg=${avg}ms  P50=${p50}ms  P90=${p90}ms  Min=${min}ms  Max=${max}ms"
Write-Host "  Timeouts: $timeouts / $Chars"

# Quartile analysis (degradation?)
if ($Chars -ge 20) {
    $qsize = [math]::Floor($Chars / 4)
    $q1 = $results[0..($qsize-1)]
    $q4 = $results[($Chars-$qsize)..($Chars-1)]
    $q1a = [math]::Round(($q1 | Measure-Object -Average).Average, 1)
    $q4a = [math]::Round(($q4 | Measure-Object -Average).Average, 1)
    $deg = if ($q1a -gt 0) { [math]::Round(($q4a - $q1a) / $q1a * 100, 1) } else { 0 }
    Write-Host "  Q1(first $qsize)=${q1a}ms  Q4(last $qsize)=${q4a}ms  Degradation=${deg}%"
}

# Histogram
$buckets = @{ "0-5ms" = 0; "5-10ms" = 0; "10-20ms" = 0; "20-50ms" = 0; "50-100ms" = 0; "100ms+" = 0 }
foreach ($r in $results) {
    if ($r -lt 5) { $buckets["0-5ms"]++ }
    elseif ($r -lt 10) { $buckets["5-10ms"]++ }
    elseif ($r -lt 20) { $buckets["10-20ms"]++ }
    elseif ($r -lt 50) { $buckets["20-50ms"]++ }
    elseif ($r -lt 100) { $buckets["50-100ms"]++ }
    else { $buckets["100ms+"]++ }
}
Write-Host "`n  Histogram:"
foreach ($k in @("0-5ms", "5-10ms", "10-20ms", "20-50ms", "50-100ms", "100ms+")) {
    $n = $buckets[$k]
    $pct = [math]::Round($n / $Chars * 100, 0)
    $bar = "#" * [math]::Min($pct, 50)
    Write-Host ("    {0,8}: {1,3} ({2,3}%) {3}" -f $k, $n, $pct, $bar)
}

Write-Host "`n  Raw: $($results -join ', ')"

# Cleanup
& $psmux kill-server 2>$null
Write-Host "`nDone."
