# test_e2e_latency.ps1 - End-to-end latency test for psmux
# Tests both WSL and pwsh, with tight sub-millisecond polling

param(
    [int]$CharCount = 60,
    [int]$InterKeyDelayMs = 80,
    [switch]$SkipWSL,
    [switch]$PwshOnly,
    [switch]$WSLOnly
)

$ErrorActionPreference = "Stop"
$psmuxExe = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $psmuxExe)) { $psmuxExe = "$PSScriptRoot\..\target\debug\psmux.exe" }

# Auto-detect WSL availability and skip WSL tests if not present
if (-not $SkipWSL -and -not $PwshOnly) {
    $wslExe = "$env:SystemRoot\System32\wsl.exe"
    if (-not (Test-Path $wslExe)) {
        Write-Host "[INFO] WSL not found — running pwsh-only tests" -ForegroundColor Yellow
        $PwshOnly = $true
    } else {
        $distroCheck = & $wslExe --list --quiet 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0 -or $distroCheck.Trim().Length -eq 0) {
            Write-Host "[INFO] No WSL distro installed — running pwsh-only tests" -ForegroundColor Yellow
            $PwshOnly = $true
        }
    }
}

function Get-LayoutHash {
    param([string]$json)
    $idx = $json.IndexOf('"layout":')
    if ($idx -lt 0) { return $json.GetHashCode() }
    $start = $json.IndexOf('{', $idx)
    if ($start -lt 0) { return $json.GetHashCode() }
    $depth = 0
    for ($p = $start; $p -lt $json.Length; $p++) {
        $c = $json[$p]
        if ($c -eq '{') { $depth++ }
        elseif ($c -eq '}') { $depth--; if ($depth -eq 0) { return $json.Substring($start, $p - $start + 1).GetHashCode() } }
    }
    return $json.GetHashCode()
}

function Run-LatencyTest {
    param(
        [string]$Label,
        [bool]$UseWSL,
        [int]$Chars,
        [int]$InterDelay
    )
    
    $sessionName = "lattest_$(Get-Random)"
    
    Write-Host ""
    Write-Host "=== $Label ===" -ForegroundColor Cyan
    Write-Host "  Chars: $Chars, Inter-key delay: ${InterDelay}ms"
    
    # Start server
    if ($UseWSL) {
        $proc = Start-Process -FilePath $psmuxExe -ArgumentList "new-session", "-d", "-s", $sessionName, "wsl" -PassThru -WindowStyle Hidden
    } else {
        $proc = Start-Process -FilePath $psmuxExe -ArgumentList "new-session", "-d", "-s", $sessionName -PassThru -WindowStyle Hidden
    }
    
    $homeDir = $env:USERPROFILE
    $pf = "$homeDir\.psmux\${sessionName}.port"
    $kf = "$homeDir\.psmux\${sessionName}.key"
    
    $t = 0
    while ((-not (Test-Path $pf)) -or (-not (Test-Path $kf))) {
        Start-Sleep -Milliseconds 200; $t += 0.2
        if ($t -ge 10) { Write-Host "ERROR: Server start timeout" -ForegroundColor Red; $proc.Kill(); return $null }
    }
    
    $port = [int](Get-Content $pf -Raw).Trim()
    $key = (Get-Content $kf -Raw).Trim()
    
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.NoDelay = $true
    $tcp.Connect("127.0.0.1", $port)
    $ns = $tcp.GetStream()
    $ns.ReadTimeout = 10000
    $wr = New-Object System.IO.StreamWriter($ns)
    $wr.AutoFlush = $false
    $rd = New-Object System.IO.StreamReader($ns)
    
    $wr.WriteLine("AUTH $key"); $wr.Flush()
    $auth = $rd.ReadLine()
    if ($auth -ne "OK") { Write-Host "Auth failed" -ForegroundColor Red; $tcp.Close(); $proc.Kill(); return $null }
    
    $wr.WriteLine("PERSISTENT"); $wr.Flush()
    Start-Sleep -Milliseconds 100
    
    # Set size
    $wr.WriteLine("client-size 120 30"); $wr.Flush()
    Start-Sleep -Milliseconds 500
    
    # Wait for shell
    for ($i = 0; $i -lt 50; $i++) {
        $wr.WriteLine("dump-state"); $wr.Flush()
        $r = $rd.ReadLine()
        if ($r -and $r -ne "NC" -and $r.Length -gt 100) { break }
        Start-Sleep -Milliseconds 200
    }
    Start-Sleep -Milliseconds 1000
    
    # Clear screen  
    foreach ($c in [char[]]"clear") {
        $wr.WriteLine("send-text ""$c"""); $wr.Flush()
        Start-Sleep -Milliseconds 30
    }
    $wr.WriteLine("send-key enter"); $wr.Flush()
    Start-Sleep -Milliseconds 500
    
    for ($i = 0; $i -lt 10; $i++) {
        $wr.WriteLine("dump-state"); $wr.Flush()
        $r = $rd.ReadLine()
        if ($r -eq "NC") { break }
        Start-Sleep -Milliseconds 50
    }
    
    # Get baseline
    $wr.WriteLine("dump-state"); $wr.Flush()
    $baseline = $rd.ReadLine()
    if ($baseline -eq "NC") { Start-Sleep -Milliseconds 100; $wr.WriteLine("dump-state"); $wr.Flush(); $baseline = $rd.ReadLine() }
    $prevHash = Get-LayoutHash $baseline
    
    Write-Host "  Ready. JSON: $($baseline.Length) bytes"
    
    # Type characters with tight polling (NO sleep between polls)
    $charStr = "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz"
    $latencies = [System.Collections.ArrayList]::new()
    $pollCountList = [System.Collections.ArrayList]::new()
    
    # Use high-resolution timer
    $freq = [System.Diagnostics.Stopwatch]::Frequency
    
    for ($i = 0; $i -lt $Chars; $i++) {
        $ch = $charStr[$i % $charStr.Length]
        
        $startTick = [System.Diagnostics.Stopwatch]::GetTimestamp()
        
        # Send char
        $wr.WriteLine("send-text ""$ch"""); $wr.Flush()
        
        # Immediately start polling dump-state (no sleep between polls)
        $polls = 0
        $found = $false
        $maxTicks = $freq / 2  # 500ms timeout
        
        while (([System.Diagnostics.Stopwatch]::GetTimestamp() - $startTick) -lt $maxTicks) {
            $wr.WriteLine("dump-state"); $wr.Flush()
            $resp = $rd.ReadLine()
            $polls++
            
            if ($resp -ne "NC") {
                $h = Get-LayoutHash $resp
                if ($h -ne $prevHash) {
                    $found = $true
                    $prevHash = $h
                    break
                }
            }
        }
        
        $endTick = [System.Diagnostics.Stopwatch]::GetTimestamp()
        $elapsedMs = [math]::Round(($endTick - $startTick) * 1000.0 / $freq, 1)
        
        [void]$latencies.Add($elapsedMs)
        [void]$pollCountList.Add($polls)
        
        if (-not $found) {
            Write-Host "  WARN: no echo for '$ch' (idx $i)" -ForegroundColor Red
        }
        
        # Progress
        if (($i + 1) % 10 -eq 0) {
            $s = [math]::Max(0, $i - 9)
            $slice = $latencies[$s..$i]
            $avg = ($slice | Measure-Object -Average).Average
            $max = ($slice | Measure-Object -Maximum).Maximum
            $pa = ($pollCountList[$s..$i] | Measure-Object -Average).Average
            Write-Host ("  [{0,3}-{1,3}] avg={2,6:F1}ms  max={3,6:F1}ms  polls={4,5:F1}" -f ($s+1), ($i+1), $avg, $max, $pa)
        }
        
        if ($InterDelay -gt 0 -and $i -lt ($Chars - 1)) {
            Start-Sleep -Milliseconds $InterDelay
        }
    }
    
    # Analysis
    $stats = $latencies | Measure-Object -Average -Minimum -Maximum
    $sorted = [double[]]($latencies | Sort-Object)
    $p50 = $sorted[[math]::Floor($sorted.Count * 0.5)]
    $p90 = $sorted[[math]::Floor($sorted.Count * 0.9)]
    $p99 = $sorted[[math]::Min($sorted.Count - 1, [math]::Floor($sorted.Count * 0.99))]
    
    $q1e = [math]::Floor($Chars/4) - 1
    $q4s = [math]::Floor($Chars*3/4)
    $q1a = ($latencies[0..$q1e] | Measure-Object -Average).Average
    $q4a = ($latencies[$q4s..($Chars-1)] | Measure-Object -Average).Average
    $deg = if ($q1a -gt 0) { (($q4a - $q1a) / $q1a) * 100 } else { 0 }
    
    Write-Host ""
    Write-Host ("  Avg={0:F1}ms  P50={1:F1}ms  P90={2:F1}ms  P99={3:F1}ms  Min={4:F1}ms  Max={5:F1}ms" -f `
        $stats.Average, $p50, $p90, $p99, $stats.Minimum, $stats.Maximum) -ForegroundColor White
    Write-Host ("  Q1={0:F1}ms  Q4={1:F1}ms  Degrade={2:+0.0;-0.0}%" -f $q1a, $q4a, $deg) -ForegroundColor White
    
    # Distribution
    $ranges = @(@{N="0-5ms";Lo=0;Hi=5}, @{N="5-10ms";Lo=5;Hi=10}, @{N="10-20ms";Lo=10;Hi=20}, @{N="20-40ms";Lo=20;Hi=40}, @{N="40-60ms";Lo=40;Hi=60}, @{N="60ms+";Lo=60;Hi=99999})
    foreach ($r in $ranges) {
        $cnt = @($latencies | Where-Object { $_ -ge $r.Lo -and $_ -lt $r.Hi }).Count
        if ($cnt -gt 0) {
            $pct = [math]::Round(($cnt / $Chars) * 100)
            Write-Host ("    {0,8}: {1,3} ({2,3}%)" -f $r.N, $cnt, $pct)
        }
    }
    
    Write-Host "  Raw: $($latencies -join ', ')" -ForegroundColor DarkGray
    
    # Cleanup
    try { $tcp.Close() } catch {}
    try { & $psmuxExe kill-server -t $sessionName 2>$null } catch {}
    Start-Sleep -Milliseconds 300
    if (-not $proc.HasExited) { try { $proc.Kill() } catch {} }
    Remove-Item $pf -ErrorAction SilentlyContinue
    Remove-Item $kf -ErrorAction SilentlyContinue
    
    return @{
        Label = $Label
        Avg = $stats.Average
        P50 = $p50
        P90 = $p90
        P99 = $p99
        Min = $stats.Minimum
        Max = $stats.Maximum
        Q1 = $q1a
        Q4 = $q4a
        Degradation = $deg
    }
}

# ── Run tests ──
$results = @()

if (-not $PwshOnly) {
    $r = Run-LatencyTest -Label "WSL (80ms between keys)" -UseWSL $true -Chars $CharCount -InterDelay $InterKeyDelayMs
    if ($r) { $results += $r }
    
    $r = Run-LatencyTest -Label "WSL (20ms burst typing)" -UseWSL $true -Chars $CharCount -InterDelay 20
    if ($r) { $results += $r }
}

if (-not $WSLOnly) {
    $r = Run-LatencyTest -Label "pwsh (80ms between keys)" -UseWSL $false -Chars $CharCount -InterDelay $InterKeyDelayMs
    if ($r) { $results += $r }
    
    $r = Run-LatencyTest -Label "pwsh (20ms burst typing)" -UseWSL $false -Chars $CharCount -InterDelay 20
    if ($r) { $results += $r }
}

# Summary
Write-Host ""
Write-Host "=== SUMMARY ===" -ForegroundColor Cyan
Write-Host ("{0,-30} {1,8} {2,8} {3,8} {4,8} {5,10}" -f "Test", "Avg", "P50", "P90", "Max", "Degrade") -ForegroundColor Yellow
foreach ($r in $results) {
    Write-Host ("{0,-30} {1,7:F1}ms {2,7:F1}ms {3,7:F1}ms {4,7:F1}ms {5,9:+0.0;-0.0}%" -f `
        $r.Label, $r.Avg, $r.P50, $r.P90, $r.Max, $r.Degradation)
}
Write-Host ""
