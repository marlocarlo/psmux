#!/usr/bin/env pwsh
# Test psmux latency: pwsh -> wsl nesting (user's exact scenario)
# In PERSISTENT mode: send-text/send-keys are fire-and-forget (no response).
# Only dump-state returns a response (the JSON or "NC").

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
$sessionName = "lattest5"
$dotPsmux = "$env:USERPROFILE\.psmux"

Write-Host "=== WSL-inside-pwsh Latency Test ==="

# 1. Kill existing & clean up
& $psmux kill-server 2>$null
Start-Sleep 2
Remove-Item "$dotPsmux\$sessionName.*" -Force -ea 0

# 2. Start detached session (launches pwsh)
Write-Host "Starting detached psmux session..."
& $psmux new-session -d -s $sessionName
if ($LASTEXITCODE -ne 0) { Write-Host "ERROR: new-session failed ($LASTEXITCODE)"; exit 1 }
Start-Sleep 3

# 3. Read port & key
$portFile = "$dotPsmux\$sessionName.port"
$keyFile  = "$dotPsmux\$sessionName.key"
if (-not (Test-Path $portFile)) { Write-Host "ERROR: no port file"; exit 1 }
if (-not (Test-Path $keyFile))  { Write-Host "ERROR: no key file"; exit 1 }
$port = (Get-Content $portFile).Trim()
$key  = (Get-Content $keyFile).Trim()
Write-Host "  Port=$port"

# 4. Connect TCP (no BOM, LF line endings)
$tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
$tcp.NoDelay = $true
$tcp.ReceiveTimeout = 10000
$stream = $tcp.GetStream()
$enc = [System.Text.UTF8Encoding]::new($false)
$reader = [System.IO.StreamReader]::new($stream, $enc, $false, 131072)
$writer = [System.IO.StreamWriter]::new($stream, $enc, 4096)
$writer.NewLine = "`n"
$writer.AutoFlush = $false

# 5. AUTH
$writer.WriteLine("AUTH $key")
$writer.Flush()
$authResp = $reader.ReadLine()
if ($authResp -ne "OK") { Write-Host "AUTH FAILED: $authResp"; $tcp.Close(); exit 1 }
Write-Host "  Auth: OK"

# 6. PERSISTENT (no response)
$writer.WriteLine("PERSISTENT")
$writer.Flush()
Write-Host "  Persistent mode"

# Fire-and-forget: send command with no response expected
function Send-Fire([string]$cmd) {
    $writer.WriteLine($cmd)
    $writer.Flush()
}

# Query dump-state and return the response line
function Get-Dump {
    $writer.WriteLine("dump-state")
    $writer.Flush()
    return $reader.ReadLine()
}

# 7. Initial dump to confirm connection works
Write-Host "Verifying connection..."
$state = Get-Dump
Write-Host "  Initial state: $($state.Length) bytes"

# 8. Type 'wsl' + Enter
Write-Host "Typing 'wsl' Enter..."
Send-Fire 'send-text "w"'
Start-Sleep -Milliseconds 150
Send-Fire 'send-text "s"'
Start-Sleep -Milliseconds 150
Send-Fire 'send-text "l"'
Start-Sleep -Milliseconds 150
Send-Fire 'send-keys Enter'

# Wait for WSL/bash to initialize
Write-Host "Waiting 6s for WSL bash..."
Start-Sleep 6

# Flush any queued dump responses by doing a fresh dump
$state = Get-Dump
Write-Host "  Post-WSL state: $($state.Length) bytes"
Start-Sleep 1

# 9. Latency test
Write-Host ""
Write-Host "=== Latency Measurements ==="
Write-Host "  Sending 20 chars, 250ms apart"
Write-Host "  For each: send-text then poll dump-state until echo arrives"
Write-Host ""

$chars = "abcdefghijklmnopqrst"
$results = @()

foreach ($ch in $chars.ToCharArray()) {
    # Send the character (fire-and-forget)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Send-Fire "send-text `"$ch`""
    
    # Poll dump-state until we get a real (non-NC) response
    $ncCount = 0
    $gotEcho = $false
    while ($sw.ElapsedMilliseconds -lt 200) {
        $resp = Get-Dump
        if ($null -eq $resp) { Write-Host "  CONNECTION LOST"; break }
        $trimmed = $resp.Trim()
        if ($trimmed -eq "NC") {
            $ncCount++
            # Tight poll - just 1ms sleep
            Start-Sleep -Milliseconds 1
        } else {
            $gotEcho = $true
            break
        }
    }
    $echoMs = $sw.ElapsedMilliseconds
    
    if ($gotEcho) {
        Write-Host ("  '{0}': {1,3}ms  (NCs={2})" -f $ch, $echoMs, $ncCount)
        $results += $echoMs
    } else {
        Write-Host ("  '{0}': TIMEOUT  (NCs={1})" -f $ch, $ncCount)
        $results += 200
    }
    
    Start-Sleep -Milliseconds 250
}

# 10. Summary
Write-Host ""
Write-Host "=== Summary ==="
if ($results.Count -gt 0) {
    $avg = ($results | Measure-Object -Average).Average
    $max = ($results | Measure-Object -Maximum).Maximum
    $min = ($results | Measure-Object -Minimum).Minimum
    $sorted = $results | Sort-Object
    $p90idx = [math]::Min([math]::Floor($results.Count * 0.9), $results.Count - 1)
    $p90 = $sorted[$p90idx]
    Write-Host ("  Chars: {0}  Min: {1}ms  Max: {2}ms  Avg: {3:F1}ms  P90: {4}ms" -f $results.Count, $min, $max, $avg, $p90)
    Write-Host "  All values: $($results -join ', ')"
}

# Cleanup
Write-Host "`nCleaning up..."
try { Send-Fire "kill-server" } catch {}
Start-Sleep 1
try { $tcp.Close() } catch {}
Write-Host "Done."
