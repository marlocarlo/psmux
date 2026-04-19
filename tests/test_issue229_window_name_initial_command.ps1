# Issue #229: Window name briefly shows pwsh when starting a session with an initial command
# Tests whether the window name flickers to 'pwsh' before settling on the actual command name
# when creating a session with an initial shell command.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:AllNames = @{}

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    param([string]$Name)
    & $PSMUX kill-session -t $Name 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Remove-Item "$psmuxDir\$Name.*" -Force -EA SilentlyContinue
}

function Wait-Session {
    param([string]$Name, [int]$TimeoutMs = 15000)
    $pf = "$psmuxDir\$Name.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw).Trim()
            if ($port -match '^\d+$') {
                try {
                    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
                    $tcp.Close()
                    return $true
                } catch {}
            }
        }
        Start-Sleep -Milliseconds 50
    }
    return $false
}

function Send-TcpCommand {
    param([string]$Session, [string]$Command)
    $portFile = "$psmuxDir\$Session.port"
    $keyFile = "$psmuxDir\$Session.key"
    if (-not (Test-Path $portFile) -or -not (Test-Path $keyFile)) { return "NO_FILES" }
    $port = (Get-Content $portFile -Raw).Trim()
    $key = (Get-Content $keyFile -Raw).Trim()
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
        $tcp.NoDelay = $true
        $stream = $tcp.GetStream()
        $writer = [System.IO.StreamWriter]::new($stream)
        $reader = [System.IO.StreamReader]::new($stream)
        $writer.Write("AUTH $key`n"); $writer.Flush()
        $authResp = $reader.ReadLine()
        if ($authResp -ne "OK") { $tcp.Close(); return "AUTH_FAILED" }
        $writer.Write("$Command`n"); $writer.Flush()
        $stream.ReadTimeout = 10000
        try { $resp = $reader.ReadLine() } catch { $resp = "TIMEOUT" }
        $tcp.Close()
        return $resp
    } catch {
        return "CONNECTION_FAILED: $_"
    }
}

function Connect-Persistent {
    param([string]$Session)
    $port = (Get-Content "$psmuxDir\$Session.port" -Raw).Trim()
    $key = (Get-Content "$psmuxDir\$Session.key" -Raw).Trim()
    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
    $tcp.NoDelay = $true; $tcp.ReceiveTimeout = 10000
    $stream = $tcp.GetStream()
    $writer = [System.IO.StreamWriter]::new($stream)
    $reader = [System.IO.StreamReader]::new($stream)
    $writer.Write("AUTH $key`n"); $writer.Flush()
    $null = $reader.ReadLine()
    $writer.Write("PERSISTENT`n"); $writer.Flush()
    return @{ tcp=$tcp; writer=$writer; reader=$reader }
}

function Get-Dump {
    param($conn)
    $conn.writer.Write("dump-state`n"); $conn.writer.Flush()
    $best = $null
    $conn.tcp.ReceiveTimeout = 3000
    for ($j = 0; $j -lt 100; $j++) {
        try { $line = $conn.reader.ReadLine() } catch { break }
        if ($null -eq $line) { break }
        if ($line -ne "NC" -and $line.Length -gt 100) { $best = $line }
        if ($best) { $conn.tcp.ReceiveTimeout = 50 }
    }
    $conn.tcp.ReceiveTimeout = 10000
    return $best
}

Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Issue #229: Window Name with Initial Command" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

# ============================================================================
# PART A: CLI Path Tests
# ============================================================================
Write-Host "`n--- Part A: CLI Path Tests ---" -ForegroundColor Cyan

# === Test 1: Session with 'timeout' command, rapid poll for name flicker ===
Write-Host "`n[Test 1] Rapid poll window name during session creation with initial command" -ForegroundColor Yellow
$SESSION1 = "issue229_test1"
Cleanup -Name $SESSION1

# Create session with an explicit command
& $PSMUX new-session -d -s $SESSION1 "timeout /T 120 > NUL"
Start-Sleep -Milliseconds 500

# Wait for session to be alive
$alive = Wait-Session -Name $SESSION1 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 1: Session $SESSION1 never became alive"
} else {
    # Rapid poll window name every 200ms for 10 seconds
    $observedNames = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 10000) {
        $name = (& $PSMUX display-message -t $SESSION1 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "") {
            [void]$observedNames.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 200
    }
    $sw.Stop()

    # Analyze: collect unique names observed
    $uniqueNames = $observedNames | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test1_timeout"] = $uniqueNames

    Write-Host "    Observed unique window names over 10s:" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames) {
        $count = ($observedNames | Where-Object { $_.Name -eq $un }).Count
        $firstSeen = ($observedNames | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
        $lastSeen = ($observedNames | Where-Object { $_.Name -eq $un } | Select-Object -Last 1).Time
        Write-Host "      '$un' seen $count times (first: ${firstSeen}ms, last: ${lastSeen}ms)" -ForegroundColor DarkGray
    }

    # Check if pwsh/powershell appeared at any point
    $shellNames = $uniqueNames | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
    if ($shellNames) {
        Write-Fail "Test 1: Window name flickered to shell name(s): $($shellNames -join ', ')"
        Write-Host "    Timeline:" -ForegroundColor DarkGray
        foreach ($obs in $observedNames) {
            if ($obs.Name -match '(?i)^(pwsh|powershell|cmd)$') {
                Write-Host "      $($obs.Time)ms: '$($obs.Name)' <<< SHELL NAME" -ForegroundColor Red
            } else {
                Write-Host "      $($obs.Time)ms: '$($obs.Name)'" -ForegroundColor DarkGray
            }
        }
    } else {
        Write-Pass "Test 1: Window name never showed shell name. Names: $($uniqueNames -join ', ')"
    }

    # Also check initial name was sensible (should be 'timeout' or similar)
    $firstName = $observedNames[0].Name
    if ($firstName -match '(?i)timeout') {
        Write-Pass "Test 1: Initial window name was '$firstName' (matches command)"
    } else {
        Write-Fail "Test 1: Initial window name was '$firstName', expected something related to 'timeout'"
    }
}
Cleanup -Name $SESSION1

# === Test 2: Session with 'ping' command ===
Write-Host "`n[Test 2] Window name with 'ping' command" -ForegroundColor Yellow
$SESSION2 = "issue229_test2"
Cleanup -Name $SESSION2

& $PSMUX new-session -d -s $SESSION2 "ping -n 120 127.0.0.1"
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION2 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 2: Session $SESSION2 never became alive"
} else {
    $observedNames2 = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 10000) {
        $name = (& $PSMUX display-message -t $SESSION2 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "") {
            [void]$observedNames2.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 200
    }
    $sw.Stop()

    $uniqueNames2 = $observedNames2 | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test2_ping"] = $uniqueNames2

    Write-Host "    Observed unique window names over 10s:" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames2) {
        $count = ($observedNames2 | Where-Object { $_.Name -eq $un }).Count
        $firstSeen = ($observedNames2 | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
        Write-Host "      '$un' seen $count times (first: ${firstSeen}ms)" -ForegroundColor DarkGray
    }

    $shellNames2 = $uniqueNames2 | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
    if ($shellNames2) {
        Write-Fail "Test 2: Window name flickered to shell name(s): $($shellNames2 -join ', ')"
    } else {
        Write-Pass "Test 2: Window name never showed shell name. Names: $($uniqueNames2 -join ', ')"
    }
}
Cleanup -Name $SESSION2

# === Test 3: Session with explicit -n name (should NOT auto-rename at all) ===
Write-Host "`n[Test 3] Session with -n name flag (manual rename lock)" -ForegroundColor Yellow
$SESSION3 = "issue229_test3"
Cleanup -Name $SESSION3

& $PSMUX new-session -d -s $SESSION3 -n "MyCustomName" "timeout /T 120 > NUL"
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION3 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 3: Session $SESSION3 never became alive"
} else {
    $observedNames3 = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 8000) {
        $name = (& $PSMUX display-message -t $SESSION3 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "") {
            [void]$observedNames3.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 300
    }
    $sw.Stop()

    $uniqueNames3 = $observedNames3 | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test3_manual_name"] = $uniqueNames3

    Write-Host "    Observed unique window names:" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames3) {
        $count = ($observedNames3 | Where-Object { $_.Name -eq $un }).Count
        Write-Host "      '$un' seen $count times" -ForegroundColor DarkGray
    }

    $allCustom = @($uniqueNames3 | Where-Object { $_ -eq "MyCustomName" })
    $notCustom = @($uniqueNames3 | Where-Object { $_ -ne "MyCustomName" })
    if ($notCustom.Count -eq 0 -and $allCustom.Count -gt 0) {
        Write-Pass "Test 3: Window name stayed at 'MyCustomName' the entire time (manual_rename lock)"
    } else {
        Write-Fail "Test 3: Expected only 'MyCustomName', got: $($uniqueNames3 -join ', ')"
    }
}
Cleanup -Name $SESSION3

# ============================================================================
# PART B: TCP Server Path Tests (dump-state JSON)
# ============================================================================
Write-Host "`n--- Part B: TCP Server Path Tests (dump-state) ---" -ForegroundColor Cyan

# === Test 4: Persistent TCP connection, rapid dump-state polling for name ===
Write-Host "`n[Test 4] TCP dump-state rapid poll for window name flicker" -ForegroundColor Yellow
$SESSION4 = "issue229_test4"
Cleanup -Name $SESSION4

& $PSMUX new-session -d -s $SESSION4 "timeout /T 120 > NUL"
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION4 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 4: Session $SESSION4 never became alive"
} else {
    try {
        $conn = Connect-Persistent -Session $SESSION4
        $dumpNames = [System.Collections.ArrayList]::new()
        $sw = [System.Diagnostics.Stopwatch]::StartNew()

        # Poll dump-state as fast as possible for 10 seconds
        while ($sw.ElapsedMilliseconds -lt 10000) {
            $state = Get-Dump $conn
            if ($state) {
                try {
                    $json = $state | ConvertFrom-Json
                    if ($json.windows -and $json.windows.Count -gt 0) {
                        $wname = $json.windows[0].name
                        [void]$dumpNames.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $wname })
                    }
                } catch {}
            }
            Start-Sleep -Milliseconds 100
        }
        $sw.Stop()
        $conn.tcp.Close()

        $uniqueDump = $dumpNames | ForEach-Object { $_.Name } | Select-Object -Unique
        $script:AllNames["Test4_dump_state"] = $uniqueDump

        Write-Host "    Observed unique names via dump-state:" -ForegroundColor DarkGray
        foreach ($un in $uniqueDump) {
            $count = ($dumpNames | Where-Object { $_.Name -eq $un }).Count
            $firstSeen = ($dumpNames | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
            $lastSeen = ($dumpNames | Where-Object { $_.Name -eq $un } | Select-Object -Last 1).Time
            Write-Host "      '$un' seen $count times (first: ${firstSeen}ms, last: ${lastSeen}ms)" -ForegroundColor DarkGray
        }

        $shellDump = $uniqueDump | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
        if ($shellDump) {
            Write-Fail "Test 4: dump-state showed shell name(s): $($shellDump -join ', ')"
            Write-Host "    Full timeline:" -ForegroundColor DarkGray
            foreach ($obs in $dumpNames) {
                $color = if ($obs.Name -match '(?i)^(pwsh|powershell|cmd)$') { "Red" } else { "DarkGray" }
                Write-Host "      $($obs.Time)ms: '$($obs.Name)'" -ForegroundColor $color
            }
        } else {
            Write-Pass "Test 4: dump-state never showed shell name. Names: $($uniqueDump -join ', ')"
        }
    } catch {
        Write-Fail "Test 4: TCP connection error: $_"
    }
}
Cleanup -Name $SESSION4

# ============================================================================
# PART C: Edge Cases
# ============================================================================
Write-Host "`n--- Part C: Edge Cases ---" -ForegroundColor Cyan

# === Test 5: Session with no initial command (should show shell name, that's expected) ===
Write-Host "`n[Test 5] Session with no initial command (baseline: shell name expected)" -ForegroundColor Yellow
$SESSION5 = "issue229_test5"
Cleanup -Name $SESSION5

& $PSMUX new-session -d -s $SESSION5
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION5 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 5: Session never became alive"
} else {
    Start-Sleep -Seconds 3
    $name5 = (& $PSMUX display-message -t $SESSION5 -p '#{window_name}' 2>&1 | Out-String).Trim()
    Write-Host "    Window name (no command): '$name5'" -ForegroundColor DarkGray
    if ($name5 -match '(?i)^(pwsh|powershell|cmd|shell|bash|zsh|fish)$') {
        Write-Pass "Test 5: No-command session correctly shows shell name: '$name5'"
    } else {
        Write-Pass "Test 5: No-command session shows: '$name5' (acceptable)"
    }
}
Cleanup -Name $SESSION5

# === Test 6: Session with full path command ===
Write-Host "`n[Test 6] Session with full path command (C:\Windows\System32\timeout.exe)" -ForegroundColor Yellow
$SESSION6 = "issue229_test6"
Cleanup -Name $SESSION6

$timeoutExe = "C:\Windows\System32\timeout.exe"
if (Test-Path $timeoutExe) {
    & $PSMUX new-session -d -s $SESSION6 "$timeoutExe /T 120"
    Start-Sleep -Milliseconds 500

    $alive = Wait-Session -Name $SESSION6 -TimeoutMs 15000
    if (-not $alive) {
        Write-Fail "Test 6: Session never became alive"
    } else {
        $observedNames6 = [System.Collections.ArrayList]::new()
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        while ($sw.ElapsedMilliseconds -lt 8000) {
            $name = (& $PSMUX display-message -t $SESSION6 -p '#{window_name}' 2>&1 | Out-String).Trim()
            if ($name -and $name -ne "") {
                [void]$observedNames6.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
            }
            Start-Sleep -Milliseconds 200
        }
        $sw.Stop()

        $uniqueNames6 = $observedNames6 | ForEach-Object { $_.Name } | Select-Object -Unique
        $script:AllNames["Test6_fullpath"] = $uniqueNames6

        Write-Host "    Observed unique window names:" -ForegroundColor DarkGray
        foreach ($un in $uniqueNames6) {
            $count = ($observedNames6 | Where-Object { $_.Name -eq $un }).Count
            Write-Host "      '$un' seen $count times" -ForegroundColor DarkGray
        }

        $shellNames6 = $uniqueNames6 | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
        if ($shellNames6) {
            Write-Fail "Test 6: Window name flickered to shell name(s): $($shellNames6 -join ', ')"
        } else {
            Write-Pass "Test 6: Window name never showed shell name. Names: $($uniqueNames6 -join ', ')"
        }
    }
    Cleanup -Name $SESSION6
} else {
    Write-Host "    Skipped: $timeoutExe not found" -ForegroundColor DarkGray
}

# === Test 7: Session with PowerShell cmdlet (Get-Process) ===
Write-Host "`n[Test 7] Session with PowerShell cmdlet 'Start-Sleep -Seconds 120'" -ForegroundColor Yellow
$SESSION7 = "issue229_test7"
Cleanup -Name $SESSION7

& $PSMUX new-session -d -s $SESSION7 "Start-Sleep -Seconds 120"
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION7 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 7: Session never became alive"
} else {
    $observedNames7 = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 10000) {
        $name = (& $PSMUX display-message -t $SESSION7 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "") {
            [void]$observedNames7.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 200
    }
    $sw.Stop()

    $uniqueNames7 = $observedNames7 | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test7_startsleep"] = $uniqueNames7

    Write-Host "    Observed unique window names:" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames7) {
        $count = ($observedNames7 | Where-Object { $_.Name -eq $un }).Count
        $firstSeen = ($observedNames7 | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
        Write-Host "      '$un' seen $count times (first: ${firstSeen}ms)" -ForegroundColor DarkGray
    }

    # For Start-Sleep, the process tree is: pwsh -> (Start-Sleep is internal, no child exe)
    # So auto-rename will likely show 'pwsh' here, which is actually expected behavior
    # since Start-Sleep doesn't spawn a child process
    $shellNames7 = $uniqueNames7 | Where-Object { $_ -match '(?i)^(pwsh|powershell)$' }
    if ($shellNames7) {
        Write-Host "    Note: 'pwsh' is expected for Start-Sleep (it is a PowerShell cmdlet, no child process)" -ForegroundColor DarkGray
        Write-Pass "Test 7: Start-Sleep correctly shows pwsh (cmdlet has no child process)"
    } else {
        Write-Pass "Test 7: Window name was: $($uniqueNames7 -join ', ')"
    }
}
Cleanup -Name $SESSION7

# === Test 8: Very rapid polling to catch brief flicker ===
Write-Host "`n[Test 8] Ultra-rapid polling (every 50ms) for first 5 seconds" -ForegroundColor Yellow
$SESSION8 = "issue229_test8"
Cleanup -Name $SESSION8

& $PSMUX new-session -d -s $SESSION8 "timeout /T 120 > NUL"

# Start polling IMMEDIATELY, even before session is fully up
$observedNames8 = [System.Collections.ArrayList]::new()
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 12000) {
    $name = (& $PSMUX display-message -t $SESSION8 -p '#{window_name}' 2>&1 | Out-String).Trim()
    if ($name -and $name -ne "" -and $name -notmatch "error|failed|no server|can't") {
        [void]$observedNames8.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
    }
    Start-Sleep -Milliseconds 50
}
$sw.Stop()

if ($observedNames8.Count -eq 0) {
    Write-Fail "Test 8: Never got a window name"
} else {
    $uniqueNames8 = $observedNames8 | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test8_rapid"] = $uniqueNames8

    Write-Host "    Total samples: $($observedNames8.Count)" -ForegroundColor DarkGray
    Write-Host "    Observed unique window names:" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames8) {
        $count = ($observedNames8 | Where-Object { $_.Name -eq $un }).Count
        $firstSeen = ($observedNames8 | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
        $lastSeen = ($observedNames8 | Where-Object { $_.Name -eq $un } | Select-Object -Last 1).Time
        Write-Host "      '$un' seen $count times (first: ${firstSeen}ms, last: ${lastSeen}ms)" -ForegroundColor DarkGray
    }

    $shellNames8 = $uniqueNames8 | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
    if ($shellNames8) {
        # Find the duration of the flicker
        $firstShell = ($observedNames8 | Where-Object { $_.Name -match '(?i)^(pwsh|powershell|cmd)$' } | Select-Object -First 1).Time
        $lastShell = ($observedNames8 | Where-Object { $_.Name -match '(?i)^(pwsh|powershell|cmd)$' } | Select-Object -Last 1).Time
        $flickerDuration = $lastShell - $firstShell
        Write-Fail "Test 8: FLICKER DETECTED! Shell name visible for ~${flickerDuration}ms (${firstShell}ms to ${lastShell}ms)"
        Write-Host "    Detailed timeline (first 30 samples):" -ForegroundColor DarkGray
        $observedNames8 | Select-Object -First 30 | ForEach-Object {
            $color = if ($_.Name -match '(?i)^(pwsh|powershell|cmd)$') { "Red" } else { "DarkGray" }
            Write-Host "      $($_.Time)ms: '$($_.Name)'" -ForegroundColor $color
        }
    } else {
        Write-Pass "Test 8: Ultra-rapid poll ($($observedNames8.Count) samples) never caught shell name"
    }
}
Cleanup -Name $SESSION8

# ============================================================================
# PART D: Interaction Tests
# ============================================================================
Write-Host "`n--- Part D: Interaction with automatic-rename ---" -ForegroundColor Cyan

# === Test 9: automatic-rename off should not flicker ===
Write-Host "`n[Test 9] automatic-rename off: name should stay fixed" -ForegroundColor Yellow
$SESSION9 = "issue229_test9"
Cleanup -Name $SESSION9

& $PSMUX new-session -d -s $SESSION9 "timeout /T 120 > NUL"
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION9 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 9: Session never became alive"
} else {
    # Turn off automatic-rename
    & $PSMUX set-option -g -t $SESSION9 automatic-rename off 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Get current name
    $nameBeforeOff = (& $PSMUX display-message -t $SESSION9 -p '#{window_name}' 2>&1 | Out-String).Trim()
    Write-Host "    Name after disabling auto-rename: '$nameBeforeOff'" -ForegroundColor DarkGray

    # Poll for a few seconds
    $observedNames9 = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 5000) {
        $name = (& $PSMUX display-message -t $SESSION9 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "") {
            [void]$observedNames9.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 200
    }

    $uniqueNames9 = $observedNames9 | ForEach-Object { $_.Name } | Select-Object -Unique
    if ($uniqueNames9.Count -eq 1) {
        Write-Pass "Test 9: Name stayed at '$($uniqueNames9[0])' with automatic-rename off"
    } else {
        Write-Fail "Test 9: Name changed even with automatic-rename off: $($uniqueNames9 -join ', ')"
    }
}
Cleanup -Name $SESSION9

# ============================================================================
# PART E: Exact reproduction from issue reporter
# ============================================================================
Write-Host "`n--- Part E: Exact Issue Reporter Reproduction ---" -ForegroundColor Cyan

# === Test 10: Exact command from issue report ===
Write-Host "`n[Test 10] Exact repro: psmux new -s repro-timeout 'timeout /T 300 > NUL'" -ForegroundColor Yellow
$SESSION10 = "repro-timeout"
Cleanup -Name $SESSION10

& $PSMUX new -s $SESSION10 -d 'timeout /T 300 > NUL'
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION10 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 10: Session never became alive"
} else {
    $observedNames10 = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 15000) {
        $name = (& $PSMUX display-message -t $SESSION10 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "" -and $name -notmatch "error|failed|no server|can't") {
            [void]$observedNames10.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 100
    }
    $sw.Stop()

    $uniqueNames10 = $observedNames10 | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test10_exact_repro"] = $uniqueNames10

    Write-Host "    Observed unique window names (15s, 100ms intervals):" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames10) {
        $count = ($observedNames10 | Where-Object { $_.Name -eq $un }).Count
        $firstSeen = ($observedNames10 | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
        $lastSeen = ($observedNames10 | Where-Object { $_.Name -eq $un } | Select-Object -Last 1).Time
        Write-Host "      '$un' seen $count times (first: ${firstSeen}ms, last: ${lastSeen}ms)" -ForegroundColor DarkGray
    }

    $shellNames10 = $uniqueNames10 | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
    if ($shellNames10) {
        $firstShell = ($observedNames10 | Where-Object { $_.Name -match '(?i)^(pwsh|powershell|cmd)$' } | Select-Object -First 1).Time
        $lastShell = ($observedNames10 | Where-Object { $_.Name -match '(?i)^(pwsh|powershell|cmd)$' } | Select-Object -Last 1).Time
        Write-Fail "Test 10: REPRO CONFIRMED! Shell name visible from ${firstShell}ms to ${lastShell}ms"
    } else {
        Write-Pass "Test 10: Exact repro did NOT show shell name flicker. Names: $($uniqueNames10 -join ', ')"
    }
}
Cleanup -Name $SESSION10

# === Test 11: Second exact command from issue ===
Write-Host "`n[Test 11] Exact repro: psmux new -s start-session-with-command 'timeout /T 300 > NUL'" -ForegroundColor Yellow
$SESSION11 = "start-session-with-command"
Cleanup -Name $SESSION11

& $PSMUX new -s $SESSION11 -d 'timeout /T 300 > NUL'
Start-Sleep -Milliseconds 500

$alive = Wait-Session -Name $SESSION11 -TimeoutMs 15000
if (-not $alive) {
    Write-Fail "Test 11: Session never became alive"
} else {
    $observedNames11 = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 15000) {
        $name = (& $PSMUX display-message -t $SESSION11 -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($name -and $name -ne "" -and $name -notmatch "error|failed|no server|can't") {
            [void]$observedNames11.Add(@{ Time = $sw.ElapsedMilliseconds; Name = $name })
        }
        Start-Sleep -Milliseconds 100
    }
    $sw.Stop()

    $uniqueNames11 = $observedNames11 | ForEach-Object { $_.Name } | Select-Object -Unique
    $script:AllNames["Test11_named_session"] = $uniqueNames11

    Write-Host "    Observed unique window names (15s, 100ms intervals):" -ForegroundColor DarkGray
    foreach ($un in $uniqueNames11) {
        $count = ($observedNames11 | Where-Object { $_.Name -eq $un }).Count
        $firstSeen = ($observedNames11 | Where-Object { $_.Name -eq $un } | Select-Object -First 1).Time
        Write-Host "      '$un' seen $count times (first: ${firstSeen}ms)" -ForegroundColor DarkGray
    }

    $shellNames11 = $uniqueNames11 | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
    if ($shellNames11) {
        Write-Fail "Test 11: Shell name appeared in names: $($uniqueNames11 -join ', ')"
    } else {
        Write-Pass "Test 11: No shell name flicker. Names: $($uniqueNames11 -join ', ')"
    }
}
Cleanup -Name $SESSION11

# ============================================================================
# Win32 TUI VISUAL VERIFICATION
# ============================================================================
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Win32 TUI Visual Verification" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

$SESSION_TUI = "issue229_tui"
Cleanup -Name $SESSION_TUI

# Launch a REAL visible psmux window with an initial command
# Start-Process with -PassThru creates a new console window (separate process)
# Use -d (detached) for server launch, then attach in a separate visible window
# Note: using array form for ArgumentList; Start-Process joins with spaces
& $PSMUX new-session -d -s $SESSION_TUI "timeout /T 120"
Start-Sleep -Seconds 3
# Now launch a visible TUI that attaches to this session
$proc = Start-Process -FilePath $PSMUX -ArgumentList 'attach-session','-t',$SESSION_TUI -PassThru
Start-Sleep -Seconds 3

# Verify session exists
& $PSMUX has-session -t $SESSION_TUI 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "TUI: Session creation failed"
} else {
    # TUI Check 1: Window name should reflect the command
    $tuiName = (& $PSMUX display-message -t $SESSION_TUI -p '#{window_name}' 2>&1 | Out-String).Trim()
    Write-Host "    TUI window name after 6s: '$tuiName'" -ForegroundColor DarkGray
    if ($tuiName -match '(?i)^(pwsh|powershell|cmd)$') {
        Write-Fail "TUI Check 1: Window name is '$tuiName' (still showing shell name after 6s)"
    } else {
        Write-Pass "TUI Check 1: Window name is '$tuiName' (not shell name)"
    }

    # TUI Check 2: Window name stability over next 5 seconds
    $tuiObserved = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 5000) {
        $n = (& $PSMUX display-message -t $SESSION_TUI -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($n) { [void]$tuiObserved.Add($n) }
        Start-Sleep -Milliseconds 300
    }
    $tuiUnique = $tuiObserved | Select-Object -Unique
    if ($tuiUnique.Count -le 1) {
        Write-Pass "TUI Check 2: Window name stable: '$($tuiUnique -join ', ')'"
    } else {
        Write-Fail "TUI Check 2: Window name changed during observation: $($tuiUnique -join ', ')"
    }

    # TUI Check 3: Verify session count
    $sessCount = (& $PSMUX display-message -t $SESSION_TUI -p '#{session_windows}' 2>&1 | Out-String).Trim()
    if ($sessCount -ge 1) {
        Write-Pass "TUI Check 3: Session has $sessCount window(s)"
    } else {
        Write-Fail "TUI Check 3: Expected at least 1 window, got $sessCount"
    }
}

# Cleanup TUI
& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}

# ============================================================================
# SUMMARY
# ============================================================================
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Summary of Observed Names" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

$bugFound = $false
foreach ($testKey in ($script:AllNames.Keys | Sort-Object)) {
    $names = $script:AllNames[$testKey]
    $hasShell = $names | Where-Object { $_ -match '(?i)^(pwsh|powershell|cmd)$' }
    $marker = if ($hasShell -and $testKey -notmatch 'startsleep|baseline') { " <<< BUG" } else { "" }
    if ($hasShell -and $testKey -notmatch 'startsleep|baseline') { $bugFound = $true }
    Write-Host "  $testKey : $($names -join ' -> ')$marker" -ForegroundColor $(if ($marker) { "Red" } else { "Green" })
}

Write-Host ""
if ($bugFound) {
    Write-Host "  VERDICT: Issue #229 is CONFIRMED. Window name briefly shows shell name." -ForegroundColor Red
} else {
    Write-Host "  VERDICT: Issue #229 could NOT be reproduced. Window name never flickered to shell name." -ForegroundColor Green
}

Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
