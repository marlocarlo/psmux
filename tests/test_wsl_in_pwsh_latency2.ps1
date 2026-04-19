#!/usr/bin/env pwsh
# Test latency for EXACT user scenario:
#   psmux -> pwsh -> user types "wsl" -> bash running inside pwsh inside psmux
#
# Usage: pwsh -File tests\test_wsl_in_pwsh_latency2.ps1

param(
    [int]$Chars = 40,
    [int]$DelayMs = 150
)

$ErrorActionPreference = "Continue"

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

$psmux = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
$session = "wsltest"

Write-Host "=== WSL-inside-pwsh-inside-psmux Latency Test ==="
Write-Host "  Chars: $Chars, Delay: ${DelayMs}ms"
Write-Host ""

# 1. Kill old, start fresh
& $psmux kill-server 2>$null
Start-Sleep 3

# 2. Create session (default shell = pwsh)
Write-Host "[1] Starting psmux session (default shell = pwsh)..."
& $psmux new-session -d -s $session 2>$null
Start-Sleep 3

$portFile = Join-Path $env:USERPROFILE ".psmux\$session.port"
$keyFile  = Join-Path $env:USERPROFILE ".psmux\$session.key"
for ($w = 0; $w -lt 30; $w++) {
    if ((Test-Path $portFile) -and (Test-Path $keyFile)) { break }
    Start-Sleep -Milliseconds 200
}
if (!(Test-Path $portFile)) { Write-Host "ERROR: No port file after 6s"; exit 1 }

$port = [int](Get-Content $portFile).Trim()
$key  = (Get-Content $keyFile).Trim()
Write-Host "  Port: $port"

# 3. Connect
Write-Host "[2] Connecting..."
$tcp = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $port)
$tcp.ReceiveTimeout = 15000
$ns = $tcp.GetStream()
$wr = New-Object System.IO.StreamWriter($ns)
$wr.AutoFlush = $true
$rd = New-Object System.IO.StreamReader($ns)

$wr.WriteLine("AUTH $key")
$authResp = $rd.ReadLine()
if ($authResp -ne "OK") { Write-Host "Auth failed: $authResp"; $tcp.Close(); exit 1 }
Write-Host "  Auth OK"

$wr.WriteLine("PERSISTENT")
Start-Sleep -Milliseconds 300

# Set size
$wr.WriteLine("client-size 120 30")
Start-Sleep -Milliseconds 200

# 4. Type "wsl" + Enter
Write-Host "[3] Typing 'wsl' inside pwsh pane..."
$wr.WriteLine("send-keys w s l Enter")
Write-Host "  Waiting 6s for WSL to start..."
Start-Sleep 6

# 5. Verify we have a dump-state
Write-Host "[4] Getting baseline dump-state..."
$wr.WriteLine("dump-state")
$base = $rd.ReadLine()
if ($base -eq "NC") { $wr.WriteLine("dump-state"); $base = $rd.ReadLine() }
Write-Host "  dump-state length: $($base.Length)"
if ($base.Length -lt 100) { Write-Host "WARNING: dump-state seems too short"; }

# 6. Disable status bar clock
$wr.WriteLine("set status-right `"`"")
$wr.WriteLine("set status-left `"test`"")
Start-Sleep -Milliseconds 500

# Get fresh baseline after clock disabled
$wr.WriteLine("dump-state")
$prev = $rd.ReadLine()
if ($prev -eq "NC") { $wr.WriteLine("dump-state"); $prev = $rd.ReadLine() }

# 7. Type chars, measure echo latency
Write-Host "`n[5] Typing $Chars chars (${DelayMs}ms gap)..."
Write-Host "    Pipeline: key -> TCP -> server -> ConPTY(pwsh+wsl) -> echo -> vt100 -> JSON -> TCP"

$alphabet = "abcdefghijklmnopqrstuvwxyz0123456789"
$results = [System.Collections.ArrayList]::new()
$timeouts = 0

for ($i = 0; $i -lt $Chars; $i++) {
    $c = $alphabet[$i % $alphabet.Length]
    
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $wr.WriteLine("send-text `"$c`"")
    
    # Poll for changed dump-state (2ms sleep between polls, 3s timeout)
    $found = $false
    while ($sw.ElapsedMilliseconds -lt 3000) {
        Start-Sleep -Milliseconds 3
        $wr.WriteLine("dump-state")
        try {
            $resp = $rd.ReadLine()
        } catch {
            Write-Host "  ERROR: ReadLine failed at char $i"
            break
        }
        if ($null -eq $resp) { Write-Host "  ERROR: null response at char $i"; break }
        if ($resp -ne "NC" -and $resp -ne $prev) {
            $prev = $resp
            $found = $true
            break
        }
    }
    $sw.Stop()
    $ms = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
    
    if (!$found) {
        $timeouts++
        $ms = 3000.0
        if ($timeouts -le 5) { Write-Host "  TIMEOUT char $i ('$c')" }
    }
    
    [void]$results.Add($ms)
    
    if (($i + 1) % 10 -eq 0) {
        $batch = $results[($i-9)..$i]
        $avg = [math]::Round(($batch | Measure-Object -Average).Average, 1)
        $max = [math]::Round(($batch | Measure-Object -Maximum).Maximum, 1)
        $min = [math]::Round(($batch | Measure-Object -Minimum).Minimum, 1)
        Write-Host ("  [{0,3}-{1,3}] avg={2,7:F1}ms  min={3,7:F1}ms  max={4,7:F1}ms" -f ($i-8), ($i+1), $avg, $min, $max)
    }
    
    Start-Sleep -Milliseconds $DelayMs
}

try { $tcp.Close() } catch {}

# 8. Results
Write-Host "`n=== RESULTS: WSL inside pwsh inside psmux ==="
$arr = $results.ToArray()
$sorted = $arr | Sort-Object
$avg = [math]::Round(($arr | Measure-Object -Average).Average, 1)
$p50 = $sorted[[math]::Floor($sorted.Count * 0.5)]
$p90 = $sorted[[math]::Floor($sorted.Count * 0.9)]
$min = $sorted[0]
$max = $sorted[-1]
Write-Host "  Avg=${avg}ms  P50=${p50}ms  P90=${p90}ms  Min=${min}ms  Max=${max}ms"
Write-Host "  Timeouts: $timeouts / $Chars"

if ($Chars -ge 20) {
    $qsize = [math]::Floor($Chars / 4)
    $q1 = $arr[0..($qsize-1)]
    $q4 = $arr[($Chars-$qsize)..($Chars-1)]
    $q1a = [math]::Round(($q1 | Measure-Object -Average).Average, 1)
    $q4a = [math]::Round(($q4 | Measure-Object -Average).Average, 1)
    $deg = if ($q1a -gt 0) { [math]::Round(($q4a - $q1a) / $q1a * 100, 1) } else { 0 }
    Write-Host "  Q1(first $qsize)=${q1a}ms  Q4(last $qsize)=${q4a}ms  Degradation=${deg}%"
}

Write-Host "`n  Raw: $($arr -join ', ')"

# Cleanup
& $psmux kill-server 2>$null
Write-Host "`nDone."
