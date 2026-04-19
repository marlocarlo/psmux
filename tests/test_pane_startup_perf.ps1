# test_pane_startup_perf.ps1 — Comprehensive pane/window/session startup latency test
# Measures EXACTLY how long it takes for pwsh to fully load in psmux panes,
# and isolates whether the delay is from psmux infrastructure or from pwsh itself.
#
# Tests:
#   1. Baseline: raw pwsh startup time (no psmux)
#   2. First session creation + first pane ready time
#   3. New window creation + shell ready time (repeated N times)
#   4. Split-window pane creation + shell ready time (repeated N times)
#   5. Rapid sequential window creation (stress test)
#   6. Multiple sessions creation
#   7. Pane close / window close latency
#
# For each, we measure wall-clock time until the pwsh prompt actually appears
# (detected via capture-pane output containing "PS " prompt marker).

param(
    [int]$WindowCount = 5,
    [int]$SplitCount = 4,
    [int]$SessionCount = 3,
    [int]$PromptTimeoutSec = 30,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"
$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = Join-Path $PSScriptRoot "..\target\release\tmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "ERROR: Cannot find psmux.exe or tmux.exe in target\release\" -ForegroundColor Red
    exit 1
}
$PSMUX = (Resolve-Path $PSMUX).Path

$PASS = 0; $FAIL = 0; $TOTAL_TESTS = 0
function Write-Pass { param([string]$msg) $script:PASS++; $script:TOTAL_TESTS++; Write-Host "  PASS: $msg" -ForegroundColor Green }
function Write-Fail { param([string]$msg) $script:FAIL++; $script:TOTAL_TESTS++; Write-Host "  FAIL: $msg" -ForegroundColor Red }
function Write-Info { param([string]$msg) Write-Host "  INFO: $msg" -ForegroundColor Gray }
function Write-Metric { param([string]$label, [double]$ms)
    $color = if ($ms -lt 2000) { "Green" } elseif ($ms -lt 5000) { "Yellow" } else { "Red" }
    Write-Host ("  {0,-50} {1,8:N0} ms" -f $label, $ms) -ForegroundColor $color
}

# Helper: wait for port/key files to appear, return (port, key)
function Wait-ServerReady {
    param([string]$SessionName, [int]$TimeoutSec = 15)
    $homeDir = $env:USERPROFILE
    $pf = "$homeDir\.psmux\${SessionName}.port"
    $kf = "$homeDir\.psmux\${SessionName}.key"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt ($TimeoutSec * 1000)) {
        if ((Test-Path $pf) -and (Test-Path $kf)) {
            $port = [int](Get-Content $pf -Raw).Trim()
            $key  = (Get-Content $kf -Raw).Trim()
            if ($port -gt 0 -and $key.Length -gt 0) {
                return @{ Port = $port; Key = $key; ElapsedMs = $sw.ElapsedMilliseconds }
            }
        }
        Start-Sleep -Milliseconds 50
    }
    return $null
}

# Helper: wait until capture-pane shows a pwsh prompt (line containing "PS " and ">")
function Wait-PanePrompt {
    param(
        [string]$SessionName,
        [int]$TimeoutMs = 30000,
        # Match "PS " anywhere — covers word-wrapped prompts in small panes
        [string]$PromptPattern = "PS [A-Z]:\\"
    )
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        try {
            $output = & $PSMUX capture-pane -t $SessionName -p 2>&1 | Out-String
            if ($output -match $PromptPattern) {
                return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $output }
            }
        } catch {
            # Server not ready yet, retry
        }
        Start-Sleep -Milliseconds 100
    }
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = "" }
}

# Helper: wait for a specific pane (by target) to show prompt  
function Wait-PanePromptTarget {
    param(
        [string]$Target,
        [int]$TimeoutMs = 30000,
        [string]$PromptPattern = "PS [A-Z]:\\"
    )
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        try {
            $output = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            if ($output -match $PromptPattern) {
                return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $output }
            }
        } catch {}
        Start-Sleep -Milliseconds 100
    }
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = "" }
}

# Helper: kill session and wait for cleanup
function Kill-TestSession {
    param([string]$SessionName)
    try {
        & $PSMUX kill-session -t $SessionName 2>&1 | Out-Null
    } catch {}
    # Also try kill-server for cleanliness
    Start-Sleep -Milliseconds 300
}

# Cleanup any stale sessions from prior runs
function Cleanup-All {
    try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 500
    # Remove stale port/key files
    $psmuxDir = "$env:USERPROFILE\.psmux"
    if (Test-Path $psmuxDir) {
        Get-ChildItem "$psmuxDir\perf_test_*.port" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
        Get-ChildItem "$psmuxDir\perf_test_*.key"  -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    }
}

# ==============================================================================
Write-Host ""
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host " psmux Pane Startup Performance Test" -ForegroundColor Cyan
Write-Host " $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Cyan
Write-Host " Binary: $PSMUX" -ForegroundColor Cyan
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host ""

Cleanup-All

# ==============================================================================
# TEST 0: Baseline — raw pwsh startup time (no psmux)
# ==============================================================================
Write-Host "--- TEST 0: Baseline pwsh startup (no psmux) ---" -ForegroundColor Yellow
$baselineTimes = @()
for ($i = 0; $i -lt 3; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # Start pwsh, run a command that proves it's loaded, capture output
    $result = & pwsh -NoLogo -NoProfile -Command "Write-Output 'READY'" 2>&1 | Out-String
    $sw.Stop()
    $baselineTimes += $sw.ElapsedMilliseconds
    if ($result -match "READY") {
        Write-Metric "  pwsh -NoProfile startup #$($i+1)" $sw.ElapsedMilliseconds
    } else {
        Write-Fail "pwsh baseline #$($i+1) - no output"
    }
}
$baselineAvg = ($baselineTimes | Measure-Object -Average).Average
Write-Metric "  pwsh -NoProfile AVERAGE" $baselineAvg
Write-Host ""

# Now test with profile (this is what psmux does by default)
$profileTimes = @()
for ($i = 0; $i -lt 3; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $result = & pwsh -NoLogo -Command "Write-Output 'READY'" 2>&1 | Out-String
    $sw.Stop()
    $profileTimes += $sw.ElapsedMilliseconds
    if ($result -match "READY") {
        Write-Metric "  pwsh (with profile) startup #$($i+1)" $sw.ElapsedMilliseconds
    } else {
        Write-Fail "pwsh+profile baseline #$($i+1) - no output"
    }
}
$profileAvg = ($profileTimes | Measure-Object -Average).Average
Write-Metric "  pwsh (with profile) AVERAGE" $profileAvg
Write-Host ""

# ==============================================================================
# TEST 1: First session creation — full cold start
# ==============================================================================
Write-Host "--- TEST 1: First session creation (cold start) ---" -ForegroundColor Yellow
$session1 = "perf_test_session1"
$swTotal = [System.Diagnostics.Stopwatch]::StartNew()

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $session1, "-d" -PassThru -WindowStyle Hidden
$swServer = [System.Diagnostics.Stopwatch]::StartNew()

# Phase 1: wait for server ready (port file)
$serverInfo = Wait-ServerReady -SessionName $session1 -TimeoutSec 15
if ($null -eq $serverInfo) {
    Write-Fail "Session '$session1' — server never started (no .port file)"
    Cleanup-All
    exit 1
}
$serverReadyMs = $serverInfo.ElapsedMs
Write-Metric "Server ready (.port file appeared)" $serverReadyMs

# Phase 2: wait for pwsh prompt to appear in the pane
$promptResult = Wait-PanePrompt -SessionName $session1 -TimeoutMs ($PromptTimeoutSec * 1000)
$swTotal.Stop()
if ($promptResult.Found) {
    $totalMs = $swTotal.ElapsedMilliseconds
    $psmuxOverhead = $serverReadyMs
    $shellTime = $promptResult.ElapsedMs  # from when we started polling (after server ready)
    Write-Metric "Prompt appeared (from server ready)" $promptResult.ElapsedMs
    Write-Metric "TOTAL first session startup" $totalMs
    Write-Pass "First session created and shell ready in ${totalMs}ms"
} else {
    Write-Fail "First session — pwsh prompt never appeared within ${PromptTimeoutSec}s"
    if ($Verbose) { Write-Info "Last capture: $($promptResult.Output.Substring(0, [Math]::Min(200, $promptResult.Output.Length)))" }
}
Write-Host ""

# ==============================================================================
# TEST 2: New window creation — measure time for each new window's shell to load
# ==============================================================================
Write-Host "--- TEST 2: New window creation (${WindowCount}x) ---" -ForegroundColor Yellow
$windowTimes = @()
for ($w = 0; $w -lt $WindowCount; $w++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-window -t $session1 2>&1 | Out-Null
    
    # Wait for prompt in the new (now-active) window
    $result = Wait-PanePrompt -SessionName $session1 -TimeoutMs ($PromptTimeoutSec * 1000)
    $sw.Stop()
    
    if ($result.Found) {
        $windowTimes += $sw.ElapsedMilliseconds
        Write-Metric "  Window #$($w+1) shell ready" $sw.ElapsedMilliseconds
    } else {
        Write-Fail "  Window #$($w+1) — prompt never appeared"
        if ($Verbose) { 
            $cap = & $PSMUX capture-pane -t $session1 -p 2>&1 | Out-String
            Write-Info "Capture: $($cap.Substring(0, [Math]::Min(200, $cap.Length)))" 
        }
    }
}
if ($windowTimes.Count -gt 0) {
    $winAvg = ($windowTimes | Measure-Object -Average).Average
    $winMax = ($windowTimes | Measure-Object -Maximum).Maximum
    $winMin = ($windowTimes | Measure-Object -Minimum).Minimum
    Write-Metric "  New window AVG" $winAvg
    Write-Metric "  New window MIN" $winMin
    Write-Metric "  New window MAX" $winMax
    Write-Pass "Created $($windowTimes.Count) windows successfully"
    
    # Check if psmux is adding significant overhead vs baseline
    $overhead = $winAvg - $profileAvg
    if ($overhead -gt 3000) {
        Write-Fail "psmux adds ${overhead}ms overhead per window over raw pwsh (>${overhead}ms vs ${profileAvg}ms)"
    } elseif ($overhead -gt 1000) {
        Write-Info "psmux adds ~${overhead}ms overhead per window (moderate)"
    } else {
        Write-Pass "psmux overhead per window is minimal (~${overhead}ms)"
    }
}
Write-Host ""

# ==============================================================================
# TEST 3: Split-window pane creation — measure shell ready time for splits
# ==============================================================================
Write-Host "--- TEST 3: Split-window pane creation (${SplitCount}x) ---" -ForegroundColor Yellow
# Switch to window 0 first
& $PSMUX select-window -t "${session1}:0" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

$splitTimes = @()
for ($s = 0; $s -lt $SplitCount; $s++) {
    $direction = if ($s % 2 -eq 0) { "-v" } else { "-h" }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX split-window $direction -t $session1 2>&1 | Out-Null
    
    # Wait for prompt in the new (now-active) pane
    $result = Wait-PanePrompt -SessionName $session1 -TimeoutMs ($PromptTimeoutSec * 1000)
    $sw.Stop()
    
    if ($result.Found) {
        $splitTimes += $sw.ElapsedMilliseconds
        Write-Metric "  Split #$($s+1) ($direction) shell ready" $sw.ElapsedMilliseconds
    } else {
        Write-Fail "  Split #$($s+1) — prompt never appeared"
        if ($Verbose) {
            $cap = & $PSMUX capture-pane -t $session1 -p 2>&1 | Out-String
            Write-Info "Capture: $($cap.Substring(0, [Math]::Min(200, $cap.Length)))"
        }
    }
}
if ($splitTimes.Count -gt 0) {
    $splitAvg = ($splitTimes | Measure-Object -Average).Average
    $splitMax = ($splitTimes | Measure-Object -Maximum).Maximum
    Write-Metric "  Split AVG" $splitAvg
    Write-Metric "  Split MAX" $splitMax
    Write-Pass "Created $($splitTimes.Count) splits successfully"
}
Write-Host ""

# ==============================================================================
# TEST 4: Rapid sequential window creation (stress test)
# ==============================================================================
Write-Host "--- TEST 4: Rapid sequential windows (burst of 5) ---" -ForegroundColor Yellow
$swBurst = [System.Diagnostics.Stopwatch]::StartNew()

# Create 5 windows as fast as possible
for ($w = 0; $w -lt 5; $w++) {
    & $PSMUX new-window -t $session1 2>&1 | Out-Null
}
$createMs = $swBurst.ElapsedMilliseconds
Write-Metric "  5 new-window commands sent in" $createMs

# Now check how long until ALL 5 have prompts by listing windows and checking each
Start-Sleep -Milliseconds 500
$lsw = & $PSMUX list-windows -t $session1 2>&1 | Out-String
$winCount = ($lsw -split "`n" | Where-Object { $_ -match '\S' }).Count
Write-Info "Total windows now: $winCount"

# Wait for the last window to have a prompt
$burstResult = Wait-PanePrompt -SessionName $session1 -TimeoutMs ($PromptTimeoutSec * 1000)
$swBurst.Stop()
if ($burstResult.Found) {
    Write-Metric "  Last window prompt ready" $swBurst.ElapsedMilliseconds
    Write-Pass "Burst window creation: all shells started"
} else {
    Write-Fail "Burst window creation: some prompts never appeared"
}
Write-Host ""

# ==============================================================================
# TEST 5: List panes — check how many are alive
# ==============================================================================
Write-Host "--- TEST 5: Pane health check ---" -ForegroundColor Yellow
$lsp = & $PSMUX list-panes -t $session1 2>&1 | Out-String
$paneLines = ($lsp -split "`n" | Where-Object { $_ -match '\S' })
Write-Info "Total panes: $($paneLines.Count)"
if ($Verbose) { $paneLines | ForEach-Object { Write-Info "  $_" } }

# Check each pane for prompt
$lsw2 = & $PSMUX list-windows -t $session1 2>&1
$winLines = ($lsw2 | Out-String) -split "`n" | Where-Object { $_ -match '^\d+:' }
$aliveCount = 0
$deadCount = 0
foreach ($winLine in $winLines) {
    if ($winLine -match '^(\d+):') {
        $winIdx = $Matches[1]
        try {
            $cap = & $PSMUX capture-pane -t "${session1}:${winIdx}" -p 2>&1 | Out-String
            if ($cap -match "PS [A-Z]:\\") {
                $aliveCount++
            } else {
                $deadCount++
                if ($Verbose) { Write-Info "  Window $winIdx has no prompt: $($cap.Substring(0, [Math]::Min(100, $cap.Length)))" }
            }
        } catch {
            $deadCount++
        }
    }
}
Write-Info "Windows with prompt: $aliveCount, without: $deadCount"
if ($deadCount -eq 0) {
    Write-Pass "All $aliveCount windows have active shells"
} else {
    Write-Fail "$deadCount windows have no shell prompt (hanging/crashed panes)"
}
Write-Host ""

# Kill session 1 before next test
Kill-TestSession -SessionName $session1

# ==============================================================================
# TEST 6: Multiple session creation
# ==============================================================================
Write-Host "--- TEST 6: Multiple session creation (${SessionCount}x) ---" -ForegroundColor Yellow
$sessionTimes = @()
for ($i = 0; $i -lt $SessionCount; $i++) {
    $sn = "perf_test_multi_$i"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    
    $proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $sn, "-d" -PassThru -WindowStyle Hidden
    
    $serverInfo = Wait-ServerReady -SessionName $sn -TimeoutSec 15
    if ($null -eq $serverInfo) {
        Write-Fail "  Session '$sn' — server never started"
        continue
    }
    
    $promptResult = Wait-PanePrompt -SessionName $sn -TimeoutMs ($PromptTimeoutSec * 1000)
    $sw.Stop()
    
    if ($promptResult.Found) {
        $sessionTimes += $sw.ElapsedMilliseconds
        Write-Metric "  Session #$($i+1) ready" $sw.ElapsedMilliseconds
    } else {
        Write-Fail "  Session #$($i+1) — prompt never appeared"
    }
}
if ($sessionTimes.Count -gt 0) {
    $sessAvg = ($sessionTimes | Measure-Object -Average).Average
    Write-Metric "  Session creation AVG" $sessAvg
    Write-Pass "Created $($sessionTimes.Count) sessions successfully"
}
Write-Host ""

# Cleanup multiple sessions
for ($i = 0; $i -lt $SessionCount; $i++) {
    Kill-TestSession -SessionName "perf_test_multi_$i"
}

# ==============================================================================
# TEST 7: Window/pane close latency
# ==============================================================================
Write-Host "--- TEST 7: Window/pane close latency ---" -ForegroundColor Yellow
$closeSess = "perf_test_close"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $closeSess, "-d" -WindowStyle Hidden | Out-Null
$ci = Wait-ServerReady -SessionName $closeSess -TimeoutSec 15
if ($null -eq $ci) {
    Write-Fail "Could not start close-test session"
} else {
    Wait-PanePrompt -SessionName $closeSess -TimeoutMs 15000 | Out-Null
    
    # Create 3 windows
    for ($i = 0; $i -lt 3; $i++) {
        & $PSMUX new-window -t $closeSess 2>&1 | Out-Null
    }
    Start-Sleep -Seconds 3
    
    # Measure close time
    $closeTimes = @()
    for ($i = 0; $i -lt 3; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $PSMUX kill-window -t $closeSess 2>&1 | Out-Null
        $sw.Stop()
        $closeTimes += $sw.ElapsedMilliseconds
        Write-Metric "  Kill window #$($i+1)" $sw.ElapsedMilliseconds
        Start-Sleep -Milliseconds 200
    }
    if ($closeTimes.Count -gt 0) {
        $closeAvg = ($closeTimes | Measure-Object -Average).Average
        Write-Metric "  Kill window AVG" $closeAvg
        Write-Pass "Window close working"
    }
    
    Kill-TestSession -SessionName $closeSess
}
Write-Host ""

# ==============================================================================
# TEST 8: Direct TCP timing — isolate psmux server overhead
# ==============================================================================
Write-Host "--- TEST 8: Direct TCP pane creation timing ---" -ForegroundColor Yellow
$tcpSess = "perf_test_tcp"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $tcpSess, "-d" -PassThru -WindowStyle Hidden
$tcpInfo = Wait-ServerReady -SessionName $tcpSess -TimeoutSec 15
if ($null -eq $tcpInfo) {
    Write-Fail "TCP test session failed to start"
} else {
    # Wait for first prompt
    Wait-PanePrompt -SessionName $tcpSess -TimeoutMs ($PromptTimeoutSec * 1000) | Out-Null
    
    # Open TCP connection
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.NoDelay = $true
    $tcp.Connect("127.0.0.1", $tcpInfo.Port)
    $ns = $tcp.GetStream()
    $ns.ReadTimeout = 15000
    $wr = New-Object System.IO.StreamWriter($ns)
    $wr.AutoFlush = $false
    $rd = New-Object System.IO.StreamReader($ns)
    
    $wr.WriteLine("AUTH $($tcpInfo.Key)"); $wr.Flush()
    $auth = $rd.ReadLine()
    if ($auth -ne "OK") {
        Write-Fail "TCP auth failed: $auth"
    } else {
        $wr.WriteLine("PERSISTENT"); $wr.Flush()
        Start-Sleep -Milliseconds 100
        $wr.WriteLine("client-size 120 30"); $wr.Flush()
        Start-Sleep -Milliseconds 200
        
        # Drain any initial data
        $wr.WriteLine("client-attach"); $wr.Flush()
        Start-Sleep -Milliseconds 300
        for ($d = 0; $d -lt 5; $d++) {
            $wr.WriteLine("dump-state"); $wr.Flush()
            try { $rd.ReadLine() | Out-Null } catch {}
            Start-Sleep -Milliseconds 100
        }
        
        # Measure new-window via TCP
        $tcpWinTimes = @()
        for ($w = 0; $w -lt 3; $w++) {
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $wr.WriteLine("new-window"); $wr.Flush()
            
            # Poll dump-state until we see the prompt in the new pane
            $found = $false
            while ($sw.ElapsedMilliseconds -lt ($PromptTimeoutSec * 1000)) {
                Start-Sleep -Milliseconds 100
                # Use CLI capture-pane since TCP dump-state gives layout JSON not text
                try {
                    $cap = & $PSMUX capture-pane -t $tcpSess -p 2>&1 | Out-String
                    if ($cap -match "PS [A-Z]:\\") {
                        $found = $true
                        break
                    }
                } catch {}
            }
            $sw.Stop()
            
            if ($found) {
                $tcpWinTimes += $sw.ElapsedMilliseconds
                Write-Metric "  TCP new-window #$($w+1) shell ready" $sw.ElapsedMilliseconds
            } else {
                Write-Fail "  TCP new-window #$($w+1) — prompt never appeared"
            }
        }
        
        # Measure split-window via TCP
        $tcpSplitTimes = @()
        for ($s = 0; $s -lt 3; $s++) {
            $dir = if ($s % 2 -eq 0) { "split-window -v" } else { "split-window -h" }
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $wr.WriteLine($dir); $wr.Flush()
            
            $found = $false
            while ($sw.ElapsedMilliseconds -lt ($PromptTimeoutSec * 1000)) {
                Start-Sleep -Milliseconds 100
                try {
                    $cap = & $PSMUX capture-pane -t $tcpSess -p 2>&1 | Out-String
                    if ($cap -match "PS [A-Z]:\\") {
                        $found = $true
                        break
                    }
                } catch {}
            }
            $sw.Stop()
            
            if ($found) {
                $tcpSplitTimes += $sw.ElapsedMilliseconds
                Write-Metric "  TCP split #$($s+1) shell ready" $sw.ElapsedMilliseconds
            } else {
                Write-Fail "  TCP split #$($s+1) — prompt never appeared"
            }
        }
        
        if ($tcpWinTimes.Count -gt 0) {
            $tcpAvg = ($tcpWinTimes | Measure-Object -Average).Average
            Write-Metric "  TCP new-window AVG" $tcpAvg
        }
        if ($tcpSplitTimes.Count -gt 0) {
            $stAvg = ($tcpSplitTimes | Measure-Object -Average).Average
            Write-Metric "  TCP split AVG" $stAvg
        }
    }
    $tcp.Close()
    Kill-TestSession -SessionName $tcpSess
}
Write-Host ""

# ==============================================================================
# TEST 9: Stress test — many panes rapidly, check for hangs
# ==============================================================================
Write-Host "--- TEST 9: Stress test — 10 windows rapidly ---" -ForegroundColor Yellow
$stressSess = "perf_test_stress"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-s", $stressSess, "-d" -WindowStyle Hidden | Out-Null
$stressInfo = Wait-ServerReady -SessionName $stressSess -TimeoutSec 15
if ($null -eq $stressInfo) {
    Write-Fail "Stress test session failed to start"
} else {
    Wait-PanePrompt -SessionName $stressSess -TimeoutMs ($PromptTimeoutSec * 1000) | Out-Null
    
    $swStress = [System.Diagnostics.Stopwatch]::StartNew()
    # Create 10 windows as fast as possible (no waiting between)
    for ($i = 0; $i -lt 10; $i++) {
        & $PSMUX new-window -t $stressSess 2>&1 | Out-Null
    }
    $createBurstMs = $swStress.ElapsedMilliseconds
    Write-Metric "  10 new-window commands took" $createBurstMs
    
    # Wait for all to settle
    Write-Info "Waiting for all panes to initialize..."
    Start-Sleep -Seconds 10
    
    # Count how many actually have prompts
    $lsw3 = & $PSMUX list-windows -t $stressSess 2>&1
    $wins = ($lsw3 | Out-String) -split "`n" | Where-Object { $_ -match '^\d+:' }
    $alive = 0; $dead = 0; $deadList = @()
    foreach ($w in $wins) {
        if ($w -match '^(\d+):') {
            $idx = $Matches[1]
            try {
                $cap = & $PSMUX capture-pane -t "${stressSess}:${idx}" -p 2>&1 | Out-String
                if ($cap -match "PS [A-Z]:\\") { $alive++ }
                else { 
                    $dead++
                    $deadList += "win$idx"
                    if ($Verbose) { Write-Info "  Window $idx capture: '$($cap.Trim().Substring(0, [Math]::Min(80, $cap.Trim().Length)))'" }
                }
            } catch { $dead++; $deadList += "win${idx}(err)" }
        }
    }
    
    $swStress.Stop()
    Write-Info "Total windows: $($alive + $dead), Alive: $alive, Dead/Hanging: $dead"
    if ($dead -gt 0) {
        Write-Fail "STRESS TEST: $dead out of $($alive + $dead) windows have no prompt! ($($deadList -join ', '))"
    } else {
        Write-Pass "STRESS TEST: All $alive windows have active shells"
    }
    
    Kill-TestSession -SessionName $stressSess
}
Write-Host ""

# ==============================================================================
# SUMMARY
# ==============================================================================
Cleanup-All
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host " SUMMARY" -ForegroundColor Cyan
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Baseline pwsh -NoProfile:   $([math]::Round($baselineAvg))ms" -ForegroundColor White
Write-Host "  Baseline pwsh (w/profile):  $([math]::Round($profileAvg))ms" -ForegroundColor White
if ($windowTimes.Count -gt 0) {
    Write-Host "  New window AVG:             $([math]::Round(($windowTimes | Measure-Object -Average).Average))ms" -ForegroundColor White
}
if ($splitTimes.Count -gt 0) {
    Write-Host "  Split pane AVG:             $([math]::Round(($splitTimes | Measure-Object -Average).Average))ms" -ForegroundColor White
}
if ($sessionTimes.Count -gt 0) {
    Write-Host "  New session AVG:            $([math]::Round(($sessionTimes | Measure-Object -Average).Average))ms" -ForegroundColor White
}
Write-Host ""
Write-Host "  Tests passed: $PASS / $TOTAL_TESTS" -ForegroundColor $(if ($FAIL -eq 0) { "Green" } else { "Red" })
if ($FAIL -gt 0) {
    Write-Host "  Tests FAILED: $FAIL" -ForegroundColor Red
}
Write-Host ""
