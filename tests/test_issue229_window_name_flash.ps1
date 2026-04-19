# Issue #229: Window name briefly shows pwsh when starting a session with an initial command
# Tests that the window name does NOT flash to "pwsh" before settling on the
# actual command name when automatic-rename is enabled.
#
# The bug: when creating a session with a command like 'timeout /T 300 > NUL',
# the window name is initially set correctly (e.g. "timeout"), but the
# automatic-rename loop runs before the child command has spawned inside
# the shell wrapper (pwsh -Command ...), finds only pwsh in the process tree,
# and temporarily renames the window to "pwsh".

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    param([string[]]$Sessions)
    foreach ($s in $Sessions) {
        & $PSMUX kill-session -t $s 2>&1 | Out-Null
        Start-Sleep -Milliseconds 300
        Remove-Item "$psmuxDir\$s.*" -Force -EA SilentlyContinue
    }
}

function Wait-Session {
    param([string]$Name, [int]$TimeoutMs = 15000)
    $portFile = "$psmuxDir\$Name.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $portFile) {
            $port = (Get-Content $portFile -Raw).Trim()
            if ($port -match '^\d+$') {
                try {
                    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
                    $tcp.Close()
                    return $true
                } catch {}
            }
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Send-TcpCommand {
    param([string]$Session, [string]$Command)
    $port = (Get-Content "$psmuxDir\$Session.port" -Raw).Trim()
    $key = (Get-Content "$psmuxDir\$Session.key" -Raw).Trim()
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

$AllSessions = @(
    "issue229_timeout",
    "issue229_ping",
    "issue229_findstr",
    "issue229_named",
    "issue229_tcp",
    "issue229_tui"
)

# === SETUP: Kill any leftover test sessions ===
Cleanup -Sessions $AllSessions

Write-Host "`n=== Issue #229: Window Name Flash Bug Reproduction ===" -ForegroundColor Cyan
Write-Host "Bug: window name briefly shows 'pwsh' before settling on the command name`n"

# ============================================================================
# Part A: CLI Path Tests (main.rs dispatch)
# ============================================================================
Write-Host "--- Part A: CLI Path (direct command invocation) ---" -ForegroundColor Magenta

# === TEST 1: new-session with 'timeout' command ===
# This is the exact reproduction from the issue
Write-Host "`n[Test 1] new-session with 'timeout /T 300 > NUL' (exact issue repro)" -ForegroundColor Yellow
$SESSION = "issue229_timeout"
& $PSMUX new-session -d -s $SESSION "timeout /T 300 > NUL"
Start-Sleep -Seconds 3
if (-not (Wait-Session $SESSION)) {
    Write-Fail "Session creation failed for $SESSION"
} else {
    # Sample window name rapidly over 8 seconds to detect the flash
    $names = [System.Collections.ArrayList]::new()
    $startTime = [System.Diagnostics.Stopwatch]::StartNew()
    $samplingDurationMs = 8000
    while ($startTime.ElapsedMilliseconds -lt $samplingDurationMs) {
        $wname = (& $PSMUX display-message -t $SESSION -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname) {
            [void]$names.Add(@{
                TimeMs = [math]::Round($startTime.Elapsed.TotalMilliseconds)
                Name = $wname
            })
        }
        Start-Sleep -Milliseconds 200
    }

    Write-Host "    Sampled $($names.Count) window name readings over ${samplingDurationMs}ms:" -ForegroundColor Gray
    $uniqueNames = $names | ForEach-Object { $_.Name } | Sort-Object -Unique
    foreach ($n in $uniqueNames) {
        $count = ($names | Where-Object { $_.Name -eq $n }).Count
        $firstSeen = ($names | Where-Object { $_.Name -eq $n } | Select-Object -First 1).TimeMs
        $lastSeen = ($names | Where-Object { $_.Name -eq $n } | Select-Object -Last 1).TimeMs
        Write-Host "      '$n': seen $count times (first: ${firstSeen}ms, last: ${lastSeen}ms)" -ForegroundColor Gray
    }

    $flashedToPwsh = $names | Where-Object { $_.Name -eq "pwsh" -or $_.Name -eq "powershell" }
    if ($flashedToPwsh.Count -gt 0) {
        $flashMs = ($flashedToPwsh | Select-Object -First 1).TimeMs
        Write-Fail "BUG REPRODUCED: window name flashed to '$($flashedToPwsh[0].Name)' at ${flashMs}ms ($($flashedToPwsh.Count) samples)"
    } else {
        Write-Pass "Window name never showed 'pwsh' during $samplingDurationMs ms sampling"
    }

    # Verify it eventually settles on the expected name
    $finalName = ($names | Select-Object -Last 1).Name
    if ($finalName -match "timeout") {
        Write-Pass "Final window name is '$finalName' (contains 'timeout')"
    } else {
        Write-Fail "Final window name is '$finalName', expected something with 'timeout'"
    }
}

# === TEST 2: new-session with 'ping' command (another long-running command) ===
Write-Host "`n[Test 2] new-session with 'ping -n 300 127.0.0.1' (another long-running command)" -ForegroundColor Yellow
$SESSION = "issue229_ping"
& $PSMUX new-session -d -s $SESSION "ping -n 300 127.0.0.1"
Start-Sleep -Seconds 3
if (-not (Wait-Session $SESSION)) {
    Write-Fail "Session creation failed for $SESSION"
} else {
    $names = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 8000) {
        $wname = (& $PSMUX display-message -t $SESSION -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname) { [void]$names.Add(@{ TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds); Name = $wname }) }
        Start-Sleep -Milliseconds 200
    }

    $uniqueNames = $names | ForEach-Object { $_.Name } | Sort-Object -Unique
    Write-Host "    Window names observed: $($uniqueNames -join ', ')" -ForegroundColor Gray

    $flashedToPwsh = $names | Where-Object { $_.Name -eq "pwsh" -or $_.Name -eq "powershell" }
    if ($flashedToPwsh.Count -gt 0) {
        Write-Fail "BUG REPRODUCED: window name flashed to 'pwsh' ($($flashedToPwsh.Count) times)"
    } else {
        Write-Pass "Window name never showed 'pwsh' for ping session"
    }

    $finalName = ($names | Select-Object -Last 1).Name
    if ($finalName -match "ping|PING") {
        Write-Pass "Final window name is '$finalName' (contains 'ping')"
    } else {
        Write-Fail "Final window name is '$finalName', expected 'ping'"
    }
}

# === TEST 3: new-session with 'findstr' (fast-spawning child) ===
Write-Host "`n[Test 3] new-session with 'findstr /R . NUL' (fast-spawning child)" -ForegroundColor Yellow
$SESSION = "issue229_findstr"
& $PSMUX new-session -d -s $SESSION 'findstr /R "." NUL'
Start-Sleep -Seconds 3
if (-not (Wait-Session $SESSION)) {
    Write-Fail "Session creation failed for $SESSION"
} else {
    $names = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 6000) {
        $wname = (& $PSMUX display-message -t $SESSION -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname) { [void]$names.Add(@{ TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds); Name = $wname }) }
        Start-Sleep -Milliseconds 200
    }

    $uniqueNames = $names | ForEach-Object { $_.Name } | Sort-Object -Unique
    Write-Host "    Window names observed: $($uniqueNames -join ', ')" -ForegroundColor Gray

    $flashedToPwsh = $names | Where-Object { $_.Name -eq "pwsh" -or $_.Name -eq "powershell" }
    if ($flashedToPwsh.Count -gt 0) {
        Write-Fail "BUG REPRODUCED: window name flashed to 'pwsh' for findstr ($($flashedToPwsh.Count) times)"
    } else {
        Write-Pass "Window name never showed 'pwsh' for findstr session"
    }
}

# === TEST 4: new-session with -n flag (manual name should be immune) ===
Write-Host "`n[Test 4] new-session with -n MyWindow (manual name, should never change)" -ForegroundColor Yellow
$SESSION = "issue229_named"
& $PSMUX new-session -d -s $SESSION -n "MyWindow" "timeout /T 300 > NUL"
Start-Sleep -Seconds 3
if (-not (Wait-Session $SESSION)) {
    Write-Fail "Session creation failed for $SESSION"
} else {
    $names = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 6000) {
        $wname = (& $PSMUX display-message -t $SESSION -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname) { [void]$names.Add(@{ TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds); Name = $wname }) }
        Start-Sleep -Milliseconds 200
    }

    $uniqueNames = $names | ForEach-Object { $_.Name } | Sort-Object -Unique
    Write-Host "    Window names observed: $($uniqueNames -join ', ')" -ForegroundColor Gray

    $nonMyWindow = $names | Where-Object { $_.Name -ne "MyWindow" }
    if ($nonMyWindow.Count -gt 0) {
        Write-Fail "BUG: manual name overridden, saw: $($nonMyWindow[0].Name)"
    } else {
        Write-Pass "Manual name 'MyWindow' stayed stable across all $($names.Count) samples"
    }
}

# ============================================================================
# Part B: TCP Server Path Tests (server/connection.rs)
# ============================================================================
Write-Host "`n--- Part B: TCP Server Path (raw socket commands) ---" -ForegroundColor Magenta

Write-Host "`n[Test 5] TCP: create session with command, monitor name via dump-state" -ForegroundColor Yellow
$SESSION = "issue229_tcp"
# Use one of the existing sessions as a server entry point
$baseSess = "issue229_timeout"
if (Test-Path "$psmuxDir\$baseSess.port") {
    # Ask existing server to create a new session with a command
    $resp = Send-TcpCommand -Session $baseSess -Command "new-session -d -s $SESSION `"timeout /T 300`""
    Start-Sleep -Seconds 2

    if (Wait-Session $SESSION) {
        # Use persistent connection to rapidly poll dump-state for window name
        $conn = Connect-Persistent -Session $SESSION
        $nameHistory = [System.Collections.ArrayList]::new()
        $sw = [System.Diagnostics.Stopwatch]::StartNew()

        while ($sw.ElapsedMilliseconds -lt 8000) {
            $state = Get-Dump $conn
            if ($state) {
                try {
                    $json = $state | ConvertFrom-Json
                    $wn = $json.windows[0].name
                    if ($wn) {
                        [void]$nameHistory.Add(@{ TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds); Name = $wn })
                    }
                } catch {}
            }
            Start-Sleep -Milliseconds 300
        }
        $conn.tcp.Close()

        $uniqueNames = $nameHistory | ForEach-Object { $_.Name } | Sort-Object -Unique
        Write-Host "    TCP dump-state window names: $($uniqueNames -join ', ')" -ForegroundColor Gray

        $flashedToPwsh = $nameHistory | Where-Object { $_.Name -eq "pwsh" -or $_.Name -eq "powershell" }
        if ($flashedToPwsh.Count -gt 0) {
            Write-Fail "BUG REPRODUCED via TCP: window name flashed to '$($flashedToPwsh[0].Name)' ($($flashedToPwsh.Count) times)"
        } else {
            Write-Pass "TCP: Window name never showed 'pwsh'"
        }
    } else {
        Write-Fail "TCP: session $SESSION never became reachable"
    }
} else {
    Write-Fail "TCP: base session $baseSess not available for TCP test"
}

# ============================================================================
# Part C: Edge Cases
# ============================================================================
Write-Host "`n--- Part C: Edge Cases ---" -ForegroundColor Magenta

Write-Host "`n[Test 6] Immediate window name after creation (before auto-rename runs)" -ForegroundColor Yellow
# The initial name should be set by default_shell_name(), not wait for auto-rename
$SESSION_EDGE = "issue229_edge_$$"
& $PSMUX new-session -d -s $SESSION_EDGE "timeout /T 300 > NUL"
# Query IMMEDIATELY (as fast as possible after creation)
$immediateNames = @()
for ($i = 0; $i -lt 5; $i++) {
    Start-Sleep -Milliseconds 500
    $wname = (& $PSMUX display-message -t $SESSION_EDGE -p '#{window_name}' 2>&1 | Out-String).Trim()
    if ($wname) { $immediateNames += $wname }
}
if ($immediateNames.Count -gt 0) {
    $firstName = $immediateNames[0]
    Write-Host "    Earliest observed name: '$firstName'" -ForegroundColor Gray
    if ($firstName -eq "timeout") {
        Write-Pass "Initial name is 'timeout' (correct, set by default_shell_name)"
    } elseif ($firstName -eq "pwsh" -or $firstName -eq "powershell") {
        Write-Fail "BUG: initial name is '$firstName' (auto-rename already overwrote it)"
    } else {
        Write-Host "    Unexpected initial name: '$firstName'" -ForegroundColor DarkYellow
    }
}
& $PSMUX kill-session -t $SESSION_EDGE 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION_EDGE.*" -Force -EA SilentlyContinue

Write-Host "`n[Test 7] Window name with 'automatic-rename off' (should keep initial name)" -ForegroundColor Yellow
$SESSION_NOAUTO = "issue229_noauto_$$"
& $PSMUX new-session -d -s $SESSION_NOAUTO "timeout /T 300 > NUL"
Start-Sleep -Seconds 3
if (Wait-Session $SESSION_NOAUTO) {
    # Disable automatic-rename
    & $PSMUX set-option -t $SESSION_NOAUTO automatic-rename off 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    $names = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 5000) {
        $wname = (& $PSMUX display-message -t $SESSION_NOAUTO -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname) { [void]$names.Add(@{ TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds); Name = $wname }) }
        Start-Sleep -Milliseconds 300
    }

    $uniqueNames = $names | ForEach-Object { $_.Name } | Sort-Object -Unique
    Write-Host "    Names with auto-rename off: $($uniqueNames -join ', ')" -ForegroundColor Gray
    if ($uniqueNames.Count -eq 1) {
        Write-Pass "Name stable with automatic-rename off: '$($uniqueNames[0])'"
    } else {
        Write-Fail "Name changed even with automatic-rename off: $($uniqueNames -join ' -> ')"
    }
}
& $PSMUX kill-session -t $SESSION_NOAUTO 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION_NOAUTO.*" -Force -EA SilentlyContinue

# ============================================================================
# Part D: Timing Analysis (measure WHEN the flash occurs)
# ============================================================================
Write-Host "`n--- Part D: Timing Analysis ---" -ForegroundColor Magenta

Write-Host "`n[Test 8] High-frequency name sampling to pinpoint flash timing" -ForegroundColor Yellow
$SESSION_TIMING = "issue229_timing_$$"
# Kill any existing session cleanly
& $PSMUX kill-session -t $SESSION_TIMING 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Remove-Item "$psmuxDir\$SESSION_TIMING.*" -Force -EA SilentlyContinue

# Create with detached mode and time everything
$createSw = [System.Diagnostics.Stopwatch]::StartNew()
& $PSMUX new-session -d -s $SESSION_TIMING "timeout /T 300 > NUL"
$createMs = $createSw.ElapsedMilliseconds
Write-Host "    new-session returned in ${createMs}ms" -ForegroundColor Gray

# Wait for session then start high-frequency polling
if (Wait-Session $SESSION_TIMING) {
    $nameTimeline = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $prevName = ""

    while ($sw.ElapsedMilliseconds -lt 12000) {
        $wname = (& $PSMUX display-message -t $SESSION_TIMING -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname -and $wname -ne $prevName) {
            [void]$nameTimeline.Add(@{
                TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds)
                Name = $wname
            })
            $prevName = $wname
        }
        Start-Sleep -Milliseconds 100
    }

    Write-Host "    Name transition timeline:" -ForegroundColor Gray
    foreach ($entry in $nameTimeline) {
        $t = $entry.TimeMs
        $n = $entry.Name
        Write-Host "      ${t}ms: '$n'" -ForegroundColor $(if ($n -eq "pwsh" -or $n -eq "powershell") { "Red" } else { "Gray" })
    }

    $transitions = $nameTimeline.Count
    $flashEntries = $nameTimeline | Where-Object { $_.Name -eq "pwsh" -or $_.Name -eq "powershell" }
    if ($flashEntries.Count -gt 0) {
        $flashTime = $flashEntries[0].TimeMs
        Write-Fail "BUG TIMING: name flashed to 'pwsh' at ${flashTime}ms ($transitions total transitions)"
        # Find how long the flash lasted
        $nextAfterFlash = $nameTimeline | Where-Object { $_.TimeMs -gt $flashTime -and $_.Name -ne "pwsh" -and $_.Name -ne "powershell" } | Select-Object -First 1
        if ($nextAfterFlash) {
            $flashDuration = $nextAfterFlash.TimeMs - $flashTime
            Write-Host "    Flash duration: ~${flashDuration}ms before settling on '$($nextAfterFlash.Name)'" -ForegroundColor DarkYellow
        }
    } else {
        Write-Pass "No 'pwsh' flash detected in name timeline ($transitions transitions)"
    }
} else {
    Write-Fail "Timing session never became reachable"
}
& $PSMUX kill-session -t $SESSION_TIMING 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION_TIMING.*" -Force -EA SilentlyContinue

# ============================================================================
# Part E: Win32 TUI Visual Verification (MANDATORY)
# ============================================================================
Write-Host "`n" -NoNewline
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION"
Write-Host ("=" * 60)

Write-Host "`n[Test 9] TUI: Visible session with command, verify window name" -ForegroundColor Yellow
$SESSION_TUI = "issue229_tui"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION_TUI,"timeout /T 300 > NUL" -PassThru
Start-Sleep -Seconds 4

if (Wait-Session $SESSION_TUI) {
    # Sample name from the TUI session
    $tuiNames = [System.Collections.ArrayList]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 8000) {
        $wname = (& $PSMUX display-message -t $SESSION_TUI -p '#{window_name}' 2>&1 | Out-String).Trim()
        if ($wname) { [void]$tuiNames.Add(@{ TimeMs = [math]::Round($sw.Elapsed.TotalMilliseconds); Name = $wname }) }
        Start-Sleep -Milliseconds 200
    }

    $uniqueNames = $tuiNames | ForEach-Object { $_.Name } | Sort-Object -Unique
    Write-Host "    TUI window names observed: $($uniqueNames -join ', ')" -ForegroundColor Gray

    $flashedToPwsh = $tuiNames | Where-Object { $_.Name -eq "pwsh" -or $_.Name -eq "powershell" }
    if ($flashedToPwsh.Count -gt 0) {
        Write-Fail "BUG REPRODUCED in TUI: window name flashed to 'pwsh' ($($flashedToPwsh.Count) samples)"
    } else {
        Write-Pass "TUI: Window name never showed 'pwsh'"
    }

    # Verify session is functional (the TUI window is alive and responsive)
    $sessName = (& $PSMUX display-message -t $SESSION_TUI -p '#{session_name}' 2>&1 | Out-String).Trim()
    if ($sessName -eq $SESSION_TUI) {
        Write-Pass "TUI: Session '$SESSION_TUI' is responsive to CLI queries"
    } else {
        Write-Fail "TUI: Session not responsive, got '$sessName'"
    }

    # Verify automatic-rename is on (default)
    $autoRename = (& $PSMUX show-options -g -v automatic-rename -t $SESSION_TUI 2>&1 | Out-String).Trim()
    Write-Host "    automatic-rename value: '$autoRename'" -ForegroundColor Gray
    if ($autoRename -eq "on") {
        Write-Pass "TUI: automatic-rename is 'on' (default, confirming auto-rename is active)"
    }

    # Check autorename log for insights
    $autoRenameLog = "$psmuxDir\autorename.log"
    if (Test-Path $autoRenameLog) {
        $logContent = Get-Content $autoRenameLog -Tail 30 | Out-String
        $shellFallbacks = ($logContent | Select-String "fallback_name").Count
        Write-Host "    autorename.log last 30 lines contain $shellFallbacks 'fallback_name' entries" -ForegroundColor Gray
        if ($shellFallbacks -gt 0) {
            Write-Host "    (fallback_name means auto-rename fell back to the shell name because no child was found)" -ForegroundColor DarkYellow
        }
    }
} else {
    Write-Fail "TUI: Session $SESSION_TUI never became reachable"
}

# Cleanup TUI
& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}

# ============================================================================
# Final Cleanup
# ============================================================================
Write-Host "`n--- Cleanup ---" -ForegroundColor Magenta
Cleanup -Sessions $AllSessions

# ============================================================================
# Results Summary
# ============================================================================
Write-Host "`n=== Issue #229 Test Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })

if ($script:TestsFailed -gt 0) {
    Write-Host "`n  VERDICT: Bug #229 is CONFIRMED. Window name flashes to 'pwsh'" -ForegroundColor Red
    Write-Host "  before settling on the actual command name." -ForegroundColor Red
    Write-Host "  Root cause: automatic-rename loop runs before the child command" -ForegroundColor Red
    Write-Host "  has spawned inside the shell wrapper (pwsh -Command ...)." -ForegroundColor Red
} else {
    Write-Host "`n  VERDICT: Bug #229 could NOT be reproduced." -ForegroundColor Green
    Write-Host "  The window name appears stable without 'pwsh' flash." -ForegroundColor Green
}

exit $script:TestsFailed
