#!/usr/bin/env pwsh
# Test psmux latency: pwsh -> wsl nesting (user's exact scenario)
# Usage: pwsh -NoProfile -File tests\test_wsl_pwsh_latency4.ps1

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

$psmux = "$PSScriptRoot\..\target\release\psmux.exe"
$sessionName = "lattest4"
$dotPsmux = "$env:USERPROFILE\.psmux"

Write-Host "=== WSL-inside-pwsh-inside-psmux Latency Test ==="

# 1. Kill existing & clean up
& $psmux kill-server 2>$null
Start-Sleep 2
Remove-Item "$dotPsmux\$sessionName.*" -Force -ea 0

# 2. Start detached session (launches pwsh by default)
Write-Host "Starting detached session..."
& $psmux new-session -d -s $sessionName
if ($LASTEXITCODE -ne 0) { Write-Host "ERROR: new-session failed"; exit 1 }
Start-Sleep 3

# 3. Read port & key
$portFile = "$dotPsmux\$sessionName.port"
$keyFile  = "$dotPsmux\$sessionName.key"
if (-not (Test-Path $portFile)) { Write-Host "ERROR: no port file"; exit 1 }
if (-not (Test-Path $keyFile))  { Write-Host "ERROR: no key file"; exit 1 }
$port = (Get-Content $portFile).Trim()
$key  = (Get-Content $keyFile).Trim()
Write-Host "  Port=$port  KeyLen=$($key.Length)"

# 4. Connect TCP
$tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
$tcp.NoDelay = $true
$tcp.ReceiveTimeout = 10000
$stream = $tcp.GetStream()
$encoding = [System.Text.UTF8Encoding]::new($false)  # No BOM
$reader = [System.IO.StreamReader]::new($stream, $encoding, $false, 65536)
$writer = [System.IO.StreamWriter]::new($stream, $encoding, 65536)
$writer.NewLine = "`n"  # Unix line endings (LF only)
$writer.AutoFlush = $false

# 5. AUTH (server sends "OK\n")
$authCmd = "AUTH $key"
Write-Host "  Sending auth: '$authCmd'"
$writer.WriteLine($authCmd)
$writer.Flush()
$authResp = $reader.ReadLine()
Write-Host "  Auth: $authResp"
if ($authResp -ne "OK") { Write-Host "AUTH FAILED"; $tcp.Close(); exit 1 }

# 6. PERSISTENT (server does NOT send a response - it reads next line immediately)
$writer.WriteLine("PERSISTENT")
$writer.Flush()
# DO NOT read a response here!
Write-Host "  Persistent mode enabled (no response expected)"

# Helper: send a command and read the one-line response
function Send-Cmd([string]$cmd) {
    $writer.WriteLine($cmd)
    $writer.Flush()
    return $reader.ReadLine()
}

# 7. Type 'wsl' + Enter to launch WSL inside pwsh
Write-Host "Sending 'wsl' + Enter..."
Send-Cmd 'send-text "w"' | Out-Null
Start-Sleep -Milliseconds 150
Send-Cmd 'send-text "s"' | Out-Null
Start-Sleep -Milliseconds 150
Send-Cmd 'send-text "l"' | Out-Null
Start-Sleep -Milliseconds 150
Send-Cmd 'send-keys Enter' | Out-Null

# Wait for WSL/bash to initialize
Write-Host "Waiting 6s for WSL bash to start..."
Start-Sleep 6

# Do a dump-state to clear any stale state
$state = Send-Cmd "dump-state"
Write-Host "  Post-WSL state length: $($state.Length)"
Start-Sleep 1

# 8. Latency test: send each char, then poll dump-state until echo arrives
Write-Host ""
Write-Host "=== Latency Measurements ==="
Write-Host "  (sending 20 chars, 250ms apart, polling for echo)"
$chars = "abcdefghijklmnopqrst"
$results = @()

foreach ($ch in $chars.ToCharArray()) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    
    # Send the character
    $sendResp = Send-Cmd "send-text `"$ch`""
    $sendMs = $sw.ElapsedMilliseconds
    
    # Poll dump-state until we get actual data (not NC)
    $ncCount = 0
    $gotEcho = $false
    while ($sw.ElapsedMilliseconds -lt 200) {
        $resp = Send-Cmd "dump-state"
        if ($null -eq $resp) { Write-Host "  CONNECTION LOST"; break }
        if ($resp.Trim() -eq "NC") {
            $ncCount++
            # Brief sleep to avoid hammering
            Start-Sleep -Milliseconds 1
        } else {
            $gotEcho = $true
            break
        }
    }
    $echoMs = $sw.ElapsedMilliseconds
    
    if ($gotEcho) {
        Write-Host ("  '{0}': echo={1}ms  send={2}ms  NCs={3}" -f $ch, $echoMs, $sendMs, $ncCount)
        $results += $echoMs
    } else {
        Write-Host ("  '{0}': TIMEOUT 200ms  NCs={1}" -f $ch, $ncCount)
        $results += 200
    }
    
    Start-Sleep -Milliseconds 250
}

# 9. Summary
Write-Host ""
Write-Host "=== Summary ==="
if ($results.Count -gt 0) {
    $avg = ($results | Measure-Object -Average).Average
    $max = ($results | Measure-Object -Maximum).Maximum
    $min = ($results | Measure-Object -Minimum).Minimum
    $sorted = $results | Sort-Object
    $p90 = $sorted[[math]::Floor($results.Count * 0.9)]
    Write-Host ("  Min: {0}ms  Max: {1}ms  Avg: {2:F1}ms  P90: {3}ms" -f $min, $max, $avg, $p90)
    Write-Host "  All: $($results -join ', ')"
}

# Cleanup
Write-Host "`nStopping server..."
try { Send-Cmd "kill-server" | Out-Null } catch {}
try { $tcp.Close() } catch {}
Write-Host "Done."
