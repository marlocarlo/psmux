#!/usr/bin/env pwsh
# Test psmux latency: pwsh -> wsl nesting (user's exact scenario)
# Usage: pwsh -NoProfile -File tests\test_wsl_pwsh_latency3.ps1

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
$sessionName = "lattest3"
$dotPsmux = "$env:USERPROFILE\.psmux"

Write-Host "=== WSL-inside-pwsh-inside-psmux Latency Test ==="
Write-Host "  psmux: $psmux"

# 1. Kill existing & clean up
& $psmux kill-server 2>$null
Start-Sleep 2
Remove-Item "$dotPsmux\$sessionName.*" -Force -ea 0

# 2. Start detached session (launches pwsh by default)
Write-Host "Starting detached session '$sessionName'..."
& $psmux new-session -d -s $sessionName
Start-Sleep 3

# 3. Read port & key
$portFile = "$dotPsmux\$sessionName.port"
$keyFile  = "$dotPsmux\$sessionName.key"
if (-not (Test-Path $portFile)) { Write-Host "ERROR: port file not found: $portFile"; exit 1 }
if (-not (Test-Path $keyFile))  { Write-Host "ERROR: key file not found: $keyFile"; exit 1 }
$port = (Get-Content $portFile).Trim()
$key  = (Get-Content $keyFile).Trim()
Write-Host "  Port: $port  Key length: $($key.Length)"

# 4. Connect TCP
Write-Host "Connecting to localhost:$port..."
$tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
$tcp.NoDelay = $true
$stream = $tcp.GetStream()
$reader = [System.IO.StreamReader]::new($stream)
$writer = [System.IO.StreamWriter]::new($stream)
$writer.AutoFlush = $true

# 5. AUTH + PERSISTENT
$writer.WriteLine("AUTH $key")
$authResp = $reader.ReadLine()
Write-Host "  Auth: $authResp"
if ($authResp -ne "OK") { Write-Host "AUTH FAILED"; $tcp.Close(); exit 1 }

$writer.WriteLine("PERSISTENT")
$persResp = $reader.ReadLine()
Write-Host "  Persistent: $persResp"

# Helper: send command, read response
function Send-Cmd($cmd) {
    $writer.WriteLine($cmd)
    $resp = $reader.ReadLine()
    return $resp
}

# 6. Get initial dump to confirm pwsh is running
Write-Host "Getting initial state..."
$state = Send-Cmd "dump-state"
Write-Host "  Initial state length: $($state.Length)"

# 7. Type 'wsl' + Enter to launch WSL inside pwsh
Write-Host "Typing 'wsl' + Enter..."
Send-Cmd 'send-text "w"' | Out-Null
Start-Sleep -Milliseconds 100
Send-Cmd 'send-text "s"' | Out-Null
Start-Sleep -Milliseconds 100
Send-Cmd 'send-text "l"' | Out-Null
Start-Sleep -Milliseconds 100
Send-Cmd 'send-keys Enter' | Out-Null

# Wait for WSL/bash to initialize (it takes a few seconds)
Write-Host "Waiting 5s for WSL bash to start..."
Start-Sleep 5

# Get state to confirm WSL is running
$state = Send-Cmd "dump-state"
Write-Host "  State after WSL launch (len=$($state.Length))"

# 8. Latency test: send chars and measure round-trip
Write-Host ""
Write-Host "=== Latency Measurements (20 chars, 200ms apart) ==="
$chars = "abcdefghijklmnopqrst"
$results = @()

foreach ($ch in $chars.ToCharArray()) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    
    # Send the character
    $writer.WriteLine("send-text `"$ch`"")
    $sendResp = $reader.ReadLine()  # "OK"
    $sendMs = $sw.ElapsedMilliseconds
    
    # Immediately request dump-state and measure total time
    $writer.WriteLine("dump-state")
    $dumpResp = $reader.ReadLine()
    $totalMs = $sw.ElapsedMilliseconds
    
    # Check if we got NC or real data
    $isNC = ($dumpResp.Trim() -eq "NC")
    
    if ($isNC) {
        # Got NC - echo hasn't arrived yet. Poll again.
        $ncCount = 1
        while ($isNC -and $sw.ElapsedMilliseconds -lt 100) {
            Start-Sleep -Milliseconds 1
            $writer.WriteLine("dump-state")
            $dumpResp = $reader.ReadLine()
            $isNC = ($dumpResp.Trim() -eq "NC")
            if ($isNC) { $ncCount++ }
        }
        $echoMs = $sw.ElapsedMilliseconds
        Write-Host ("  '{0}': send={1}ms  echo={2}ms  NCs={3}  final={4}" -f $ch, $sendMs, $echoMs, $ncCount, $(if($isNC){"NC"}else{"DATA($($dumpResp.Length))"}))
        $results += $echoMs
    } else {
        Write-Host ("  '{0}': send={1}ms  echo={2}ms  (immediate DATA, len={3})" -f $ch, $sendMs, $totalMs, $dumpResp.Length)
        $results += $totalMs
    }
    
    Start-Sleep -Milliseconds 200
}

# 9. Summary
Write-Host ""
Write-Host "=== Summary ==="
$avg = ($results | Measure-Object -Average).Average
$max = ($results | Measure-Object -Maximum).Maximum
$min = ($results | Measure-Object -Minimum).Minimum
$p90 = ($results | Sort-Object)[[math]::Floor($results.Count * 0.9)]
Write-Host ("  Min: {0}ms  Max: {1}ms  Avg: {2:F1}ms  P90: {3}ms" -f $min, $max, $avg, $p90)
Write-Host "  All: $($results -join ', ')"

# Cleanup
Write-Host ""
Write-Host "Cleaning up..."
$writer.WriteLine("kill-server")
$tcp.Close()
Write-Host "Done."
