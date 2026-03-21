# test_wsl_latency.ps1 - Realistic WSL echo latency test
# Simulates the actual client poll+parse+render cycle timing
#
# Three test modes:
#   A) Slow typing (200ms gap) with full JSON parse - baseline
#   B) Rapid typing (50ms gap) with full JSON parse - stress
#   C) Burst typing (10ms gap) - pathological fast typing

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

$exe = ".\target\release\psmux.exe"
$home_ = $env:USERPROFILE
$session = "default"

Write-Host "=== psmux WSL Echo Latency Test (Realistic) ===" -ForegroundColor Cyan
Write-Host ""

# Step 1: Start fresh server
Write-Host "[1] Starting fresh psmux server..."
try { & $exe kill-server 2>$null } catch {}
Start-Sleep -Seconds 2
Remove-Item "$home_\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$home_\.psmux\*.key"  -Force -ErrorAction SilentlyContinue

$proc = Start-Process -FilePath $exe -ArgumentList "new-session","-d" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

$portFile = "$home_\.psmux\$session.port"
$keyFile  = "$home_\.psmux\$session.key"
if (-not (Test-Path $portFile)) { Write-Host "FAIL: no port file"; exit 1 }
$port = (Get-Content $portFile).Trim()
$key  = (Get-Content $keyFile).Trim()
Write-Host "  Server on port $port"

# TCP helpers
function New-PsmuxConnection {
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.NoDelay = $true
    $tcp.Connect("127.0.0.1", [int]$port)
    $stream = $tcp.GetStream()
    $stream.ReadTimeout = 10000
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.AutoFlush = $true
    $reader = New-Object System.IO.StreamReader($stream)
    $writer.WriteLine("AUTH $key")
    $authResp = $reader.ReadLine()
    if (-not $authResp.StartsWith("OK")) { throw "Auth failed" }
    $writer.WriteLine("PERSISTENT")
    return @{ tcp = $tcp; writer = $writer; reader = $reader }
}

function Send-Cmd($conn, $cmd) { $conn.writer.WriteLine($cmd) }
function Read-Response($conn) { return $conn.reader.ReadLine() }
function Send-TextCmd($conn, $t) { Send-Cmd $conn "send-text `"$t`"" }
function Send-KeyCmd($conn, $k) { Send-Cmd $conn "send-key $k" }

# This function simulates what the real client does:
#   send key -> poll for response -> parse JSON -> check if screen changed
# It measures key-to-echo time using the SAME polling pattern as the real client
function Measure-EchoLatency {
    param($conn, $ch, $pollInterval)

    # Simulate client: send key + dump-state together (like cmd_batch + dump-state)
    Send-TextCmd $conn "$ch"
    Send-Cmd $conn "dump-state"

    $sw = [System.Diagnostics.Stopwatch]::StartNew()

    $prevJson = ""
    $echoFound = $false
    $polls = 0
    $ncCount = 0
    $parseTimeTotal = 0

    while (-not $echoFound -and $sw.ElapsedMilliseconds -lt 2000) {
        $raw = Read-Response $conn
        $polls++

        if ($raw -eq "NC") {
            $ncCount++
        } else {
            # Simulate JSON parse cost (this is what serde_json does)
            $parseSw = [System.Diagnostics.Stopwatch]::StartNew()
            try {
                $obj = $raw | ConvertFrom-Json -ErrorAction Stop
                $parseSw.Stop()
                $parseTimeTotal += $parseSw.ElapsedMilliseconds
            } catch {
                $parseSw.Stop()
            }

            # Check if screen changed (like dump_buf != prev_dump_buf)
            if ($raw -ne $prevJson) {
                $echoFound = $true
                $prevJson = $raw
            }
        }

        if (-not $echoFound) {
            # Simulate client poll interval
            Start-Sleep -Milliseconds $pollInterval
            # Send another dump-state (like the client does every poll interval)
            Send-Cmd $conn "dump-state"
        }
    }

    $sw.Stop()
    return @{
        ms = $sw.ElapsedMilliseconds
        found = $echoFound
        polls = $polls
        ncs = $ncCount
        parseMs = $parseTimeTotal
    }
}

# Step 2: Launch WSL
Write-Host "[2] Launching WSL..."
$conn = New-PsmuxConnection
Send-TextCmd $conn "wsl"
Start-Sleep -Milliseconds 200
Send-KeyCmd $conn "Enter"

Write-Host "  Waiting for WSL prompt..."
$ready = $false
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    Send-Cmd $conn "dump-state"
    $raw = Read-Response $conn
    if ($raw -and $raw -ne "NC" -and ($raw -match '\$' -or $raw -match '#')) {
        $ready = $true; break
    }
}
if (-not $ready) { Write-Host "  WARNING: prompt not detected" -ForegroundColor Yellow }
else { Write-Host "  WSL ready." }
Start-Sleep -Milliseconds 500

# Get baseline state
Send-Cmd $conn "dump-state"
$baseline = Read-Response $conn
Start-Sleep -Milliseconds 200

# ============================================================
# TEST A: SLOW TYPING (200ms gaps, 10ms poll - matches client)
# ============================================================
Write-Host ""
Write-Host "--- TEST A: Slow typing (200ms gap, 10ms poll, 20 chars) ---" -ForegroundColor Yellow

Send-TextCmd $conn "echo "
Start-Sleep -Milliseconds 300
# Flush
Send-Cmd $conn "dump-state"
$null = Read-Response $conn
Start-Sleep -Milliseconds 100

$slowLatencies = @()
$testChars = "abcdefghijklmnopqrst".ToCharArray()

foreach ($ch in $testChars) {
    $result = Measure-EchoLatency $conn "$ch" 10
    $slowLatencies += $result.ms

    $color = if ($result.ms -lt 80) { "Green" } elseif ($result.ms -lt 150) { "Yellow" } else { "Red" }
    Write-Host ("    '{0}': {1,4}ms  polls:{2} nc:{3} parse:{4}ms" -f $ch, $result.ms, $result.polls, $result.ncs, $result.parseMs) -ForegroundColor $color

    Start-Sleep -Milliseconds 200
}

# Clear line
Send-KeyCmd $conn "C-c"
Start-Sleep -Milliseconds 300
Send-Cmd $conn "dump-state"
$null = Read-Response $conn
Start-Sleep -Milliseconds 200

# ============================================================
# TEST B: RAPID TYPING (50ms gaps, 10ms poll)
# ============================================================
Write-Host ""
Write-Host "--- TEST B: Rapid typing (50ms gap, 10ms poll, 40 chars) ---" -ForegroundColor Yellow

Send-TextCmd $conn "echo "
Start-Sleep -Milliseconds 300
Send-Cmd $conn "dump-state"
$null = Read-Response $conn
Start-Sleep -Milliseconds 100

$fastLatencies = @()
$fastChars = "thequickbrownfoxjumpsoverlazydog12345678".ToCharArray()

$totalSw = [System.Diagnostics.Stopwatch]::StartNew()
foreach ($ch in $fastChars) {
    $result = Measure-EchoLatency $conn "$ch" 10
    $fastLatencies += $result.ms

    $idx = $fastLatencies.Count
    $color = if ($result.ms -lt 80) { "Green" } elseif ($result.ms -lt 150) { "Yellow" } else { "Red" }
    Write-Host ("    [{0,2}] '{1}': {2,4}ms  polls:{3} nc:{4}" -f $idx, $ch, $result.ms, $result.polls, $result.ncs) -ForegroundColor $color

    # Wait remaining time to achieve 50ms gap
    $remaining = 50 - $result.ms
    if ($remaining -gt 0) { Start-Sleep -Milliseconds $remaining }
}
$totalSw.Stop()

Send-KeyCmd $conn "C-c"
Start-Sleep -Milliseconds 300

# ============================================================
# TEST C: BURST TYPING (no gap between chars, 5ms poll)
# ============================================================
Write-Host ""
Write-Host "--- TEST C: Burst typing (no gap, 5ms poll, 20 chars) ---" -ForegroundColor Yellow

Send-TextCmd $conn "echo "
Start-Sleep -Milliseconds 300
Send-Cmd $conn "dump-state"
$null = Read-Response $conn
Start-Sleep -Milliseconds 100

$burstLatencies = @()
$burstChars = "burstmodespeedtest20".ToCharArray()

foreach ($ch in $burstChars) {
    $result = Measure-EchoLatency $conn "$ch" 5
    $burstLatencies += $result.ms

    $idx = $burstLatencies.Count
    $color = if ($result.ms -lt 80) { "Green" } elseif ($result.ms -lt 150) { "Yellow" } else { "Red" }
    Write-Host ("    [{0,2}] '{1}': {2,4}ms  polls:{3} nc:{4}" -f $idx, $ch, $result.ms, $result.polls, $result.ncs) -ForegroundColor $color
    # No inter-character gap - immediate next character
}

Send-KeyCmd $conn "C-c"
Start-Sleep -Milliseconds 300

# ============================================================
# Results
# ============================================================
Write-Host ""
Write-Host "============ RESULTS ============" -ForegroundColor Cyan

function Show-Stats($label, $lats) {
    $avg = ($lats | Measure-Object -Average).Average
    $min_ = ($lats | Measure-Object -Minimum).Minimum
    $max_ = ($lats | Measure-Object -Maximum).Maximum
    $sorted = $lats | Sort-Object
    $cnt = $sorted.Count
    $p50 = $sorted[([Math]::Floor($cnt * 0.5))]
    $p90 = $sorted[([Math]::Floor($cnt * 0.9))]

    Write-Host "$label" -ForegroundColor Yellow
    Write-Host ("  Avg: {0:F1}ms | Min: {1}ms | Max: {2}ms | P50: {3}ms | P90: {4}ms" -f $avg, $min_, $max_, $p50, $p90)

    # Check degradation (first half vs second half)
    $half = [Math]::Floor($cnt / 2)
    $first = ($lats[0..($half-1)] | Measure-Object -Average).Average
    $second = ($lats[$half..($cnt-1)] | Measure-Object -Average).Average
    $drift = $second - $first
    if ([Math]::Abs($drift) -gt 20) {
        Write-Host ("  DRIFT: first-half={0:F1}ms second-half={1:F1}ms delta={2:F1}ms" -f $first, $second, $drift) -ForegroundColor Red
    } else {
        Write-Host ("  Stable: first-half={0:F1}ms second-half={1:F1}ms" -f $first, $second) -ForegroundColor Green
    }
}

Show-Stats "TEST A (slow, 200ms gap):" $slowLatencies
Show-Stats "TEST B (rapid, 50ms gap):" $fastLatencies
Show-Stats "TEST C (burst, no gap):" $burstLatencies

Write-Host ""
$overallAvg = (($slowLatencies + $fastLatencies + $burstLatencies) | Measure-Object -Average).Average
$over150 = (($slowLatencies + $fastLatencies + $burstLatencies) | Where-Object { $_ -gt 150 }).Count
$total = $slowLatencies.Count + $fastLatencies.Count + $burstLatencies.Count
Write-Host ("Overall avg: {0:F1}ms | Over 150ms: {1}/{2}" -f $overallAvg, $over150, $total)

if ($overallAvg -lt 60) {
    Write-Host "VERDICT: GOOD" -ForegroundColor Green
} elseif ($overallAvg -lt 100) {
    Write-Host "VERDICT: ACCEPTABLE" -ForegroundColor Yellow
} else {
    Write-Host "VERDICT: SLOW" -ForegroundColor Red
}

# Cleanup
Write-Host ""
Write-Host "[cleanup]..."
$conn.tcp.Close()
try { & $exe kill-server 2>$null } catch {}
Write-Host "Done."
