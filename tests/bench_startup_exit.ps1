# bench_startup_exit.ps1 — Comprehensive startup and exit timing benchmark
#
# Measures with high precision:
#   1. Cold first session startup (no warm server)
#   2. Warm session startup (warm server claim)
#   3. Per-pane exit time (kill-pane)
#   4. Per-window exit time (kill-window)
#   5. Per-session exit time (kill-session)
#   6. Full kill-server time
#   7. exit-empty detection latency
#   8. destroy-unattached exit time
#
# Outputs a summary table with avg/min/max/p95 for each metric.

param(
    [int]$Iterations = 5,
    [int]$TimeoutSec = 20,
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"
$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = Join-Path $PSScriptRoot "..\target\release\tmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "ERROR: Cannot find psmux.exe in target\release\" -ForegroundColor Red
    Write-Host "Run: cargo build --release" -ForegroundColor Yellow
    exit 1
}
$PSMUX = (Resolve-Path $PSMUX).Path

$HOME_DIR = $env:USERPROFILE
$PSMUX_DIR = "$HOME_DIR\.psmux"

# ── Utility functions ──

function Write-Header { param([string]$text)
    Write-Host ""
    Write-Host ("=" * 76) -ForegroundColor Cyan
    Write-Host "  $text" -ForegroundColor Cyan
    Write-Host ("=" * 76) -ForegroundColor Cyan
}

function Write-Metric { param([string]$label, [double]$ms)
    $color = if ($ms -lt 500) { "Green" } elseif ($ms -lt 2000) { "Yellow" } else { "Red" }
    Write-Host ("    {0,-48} {1,8:N1} ms" -f $label, $ms) -ForegroundColor $color
}

function Write-Summary { param([string]$label, [double[]]$values)
    if ($values.Count -eq 0) { Write-Host "    $label  NO DATA" -ForegroundColor Red; return }
    $sorted = $values | Sort-Object
    $avg = [math]::Round(($sorted | Measure-Object -Average).Average, 1)
    $min = $sorted[0]
    $max = $sorted[-1]
    $p95idx = [math]::Min([math]::Floor($sorted.Count * 0.95), $sorted.Count - 1)
    $p95 = $sorted[$p95idx]
    $med = $sorted[[math]::Floor($sorted.Count / 2)]
    Write-Host ("    {0,-32} avg={1,7:N1}  min={2,7:N1}  med={3,7:N1}  p95={4,7:N1}  max={5,7:N1}  (n={6})" `
        -f $label, $avg, $min, $med, $p95, $max, $sorted.Count) -ForegroundColor White
}

function Kill-All-Psmux {
    # Kill all psmux processes and clean stale files
    Get-Process psmux, pmux, tmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    if (Test-Path $PSMUX_DIR) {
        Remove-Item "$PSMUX_DIR\bench_*.port" -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\bench_*.key"  -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\__warm__.port" -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\__warm__.key"  -Force -ErrorAction SilentlyContinue
    }
}

function Wait-PortFile {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = [int](Get-Content $pf -Raw).Trim()
            if ($port -gt 0) { return @{ Port = $port; Ms = $sw.ElapsedMilliseconds } }
        }
        Start-Sleep -Milliseconds 2
    }
    return $null
}

function Wait-SessionAlive {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw).Trim()
            if ($port -match '^\d+$') {
                try {
                    $tcp = New-Object System.Net.Sockets.TcpClient
                    $tcp.Connect("127.0.0.1", [int]$port)
                    $tcp.Close()
                    return @{ Port = [int]$port; Ms = $sw.ElapsedMilliseconds }
                } catch {}
            }
        }
        Start-Sleep -Milliseconds 5
    }
    return $null
}

function Wait-SessionDead {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $kf = "$PSMUX_DIR\${SessionName}.key"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (-not (Test-Path $pf)) { return $sw.ElapsedMilliseconds }
        # Port file still exists — check if server is actually dead
        $port = (Get-Content $pf -Raw -ErrorAction SilentlyContinue)
        if ($null -eq $port) { return $sw.ElapsedMilliseconds }
        $port = $port.Trim()
        if ($port -match '^\d+$') {
            try {
                $tcp = New-Object System.Net.Sockets.TcpClient
                $tcp.Connect("127.0.0.1", [int]$port)
                $tcp.Close()
                # Still alive, wait
            } catch {
                # Connection refused = dead
                return $sw.ElapsedMilliseconds
            }
        } else {
            return $sw.ElapsedMilliseconds
        }
        Start-Sleep -Milliseconds 5
    }
    return $TimeoutMs  # Timed out
}

function Wait-PanePrompt {
    param([string]$Target, [int]$TimeoutMs = 20000, [string]$Pattern = "PS [A-Z]:\\")
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            if ($cap -match $Pattern) { return @{ Found = $true; Ms = $sw.ElapsedMilliseconds } }
        } catch {}
        Start-Sleep -Milliseconds 50
    }
    return @{ Found = $false; Ms = $sw.ElapsedMilliseconds }
}

function Create-Session-Detached {
    param([string]$Name, [switch]$NoConfig)
    if ($NoConfig) {
        $origConf = $env:PSMUX_CONFIG_FILE
        $env:PSMUX_CONFIG_FILE = "NUL"
    }
    & $PSMUX new-session -s $Name -d 2>&1 | Out-Null
    if ($NoConfig) {
        $env:PSMUX_CONFIG_FILE = $origConf
    }
}

# ── Banner ──

Write-Host ""
Write-Host ("*" * 76) -ForegroundColor Magenta
Write-Host "    PSMUX STARTUP & EXIT BENCHMARK SUITE" -ForegroundColor Magenta
Write-Host "    $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')  |  Iterations: $Iterations" -ForegroundColor Magenta
Write-Host "    Binary: $PSMUX" -ForegroundColor Magenta
Write-Host ("*" * 76) -ForegroundColor Magenta

$allResults = @{}

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 1: COLD SESSION STARTUP (no warm server, no config)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "1. COLD SESSION STARTUP (no warm server, empty config)"

$coldStartTimes = @()
$coldPromptTimes = @()
for ($i = 0; $i -lt $Iterations; $i++) {
    Kill-All-Psmux
    $sess = "bench_cold_$i"

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null

    # Measure time to server ready (port file + TCP reachable)
    $info = Wait-SessionAlive -SessionName $sess -TimeoutMs ($TimeoutSec * 1000)
    if ($null -ne $info) {
        $coldStartTimes += $info.Ms
        Write-Metric "  Cold start #$($i+1) (server ready)" $info.Ms

        # Measure time to prompt
        $prompt = Wait-PanePrompt -Target $sess -TimeoutMs ($TimeoutSec * 1000)
        $sw.Stop()
        if ($prompt.Found) {
            $totalMs = $info.Ms + $prompt.Ms
            $coldPromptTimes += $totalMs
            Write-Metric "  Cold start #$($i+1) (prompt ready)" $totalMs
        }
    } else {
        Write-Host "    [TIMEOUT] Cold start #$($i+1)" -ForegroundColor Red
    }
}
Write-Summary "Cold startup (server ready)" $coldStartTimes
Write-Summary "Cold startup (prompt ready)" $coldPromptTimes
$allResults["cold_start_server"] = $coldStartTimes
$allResults["cold_start_prompt"] = $coldPromptTimes

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 2: WARM SESSION STARTUP (warm server claim)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "2. WARM SESSION STARTUP (claim from pre-spawned warm server)"

Kill-All-Psmux
# First, create a session to trigger warm server spawn
$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s bench_warmbase -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null
$wbInfo = Wait-SessionAlive -SessionName "bench_warmbase" -TimeoutMs 15000
if ($null -eq $wbInfo) {
    Write-Host "    [SKIP] Could not start base session for warm server test" -ForegroundColor Yellow
} else {
    # Wait for warm server to be spawned
    Start-Sleep -Seconds 5

    $warmStartTimes = @()
    $warmPromptTimes = @()
    for ($i = 0; $i -lt $Iterations; $i++) {
        # Wait for __warm__ to exist
        $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
        if ($null -eq $warmReady) {
            Write-Host "    [SKIP] Warm server not available for run #$($i+1)" -ForegroundColor Yellow
            continue
        }

        $sess = "bench_warm_$i"
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        $sw.Stop()
        $warmStartTimes += $sw.ElapsedMilliseconds
        Write-Metric "  Warm start #$($i+1) (claim)" $sw.ElapsedMilliseconds

        # Measure time to prompt
        $prompt = Wait-PanePrompt -Target $sess -TimeoutMs ($TimeoutSec * 1000)
        if ($prompt.Found) {
            $totalMs = $sw.ElapsedMilliseconds + $prompt.Ms
            $warmPromptTimes += $totalMs
            Write-Metric "  Warm start #$($i+1) (prompt ready)" $totalMs
        }

        # Wait for next warm server to spawn
        Start-Sleep -Seconds 4
    }
    Write-Summary "Warm startup (claim)" $warmStartTimes
    Write-Summary "Warm startup (prompt)" $warmPromptTimes
    $allResults["warm_start_claim"] = $warmStartTimes
    $allResults["warm_start_prompt"] = $warmPromptTimes
}

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 3: PER-PANE EXIT TIME (kill-pane)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "3. PER-PANE EXIT TIME (kill-pane)"

Kill-All-Psmux
$paneExitSess = "bench_pane_exit"
$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s $paneExitSess -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null
$peInfo = Wait-SessionAlive -SessionName $paneExitSess -TimeoutMs 15000
if ($null -eq $peInfo) {
    Write-Host "    [SKIP] Could not start session for pane exit test" -ForegroundColor Yellow
} else {
    Wait-PanePrompt -Target $paneExitSess -TimeoutMs 15000 | Out-Null

    # Create split panes
    $paneCount = [math]::Max($Iterations, 5)
    for ($i = 0; $i -lt $paneCount; $i++) {
        $dir = if ($i % 2 -eq 0) { "-v" } else { "-h" }
        & $PSMUX split-window $dir -t $paneExitSess 2>&1 | Out-Null
    }
    Start-Sleep -Seconds 3  # Let shells load

    $paneExitTimes = @()
    for ($i = 0; $i -lt $paneCount; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $PSMUX kill-pane -t $paneExitSess 2>&1 | Out-Null
        $sw.Stop()
        $paneExitTimes += $sw.ElapsedMilliseconds
        Write-Metric "  kill-pane #$($i+1)" $sw.ElapsedMilliseconds
        Start-Sleep -Milliseconds 100
    }
    Write-Summary "kill-pane latency" $paneExitTimes
    $allResults["kill_pane"] = $paneExitTimes
}

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 4: PER-WINDOW EXIT TIME (kill-window)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "4. PER-WINDOW EXIT TIME (kill-window)"

Kill-All-Psmux
$winExitSess = "bench_win_exit"
$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s $winExitSess -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null
$weInfo = Wait-SessionAlive -SessionName $winExitSess -TimeoutMs 15000
if ($null -eq $weInfo) {
    Write-Host "    [SKIP] Could not start session for window exit test" -ForegroundColor Yellow
} else {
    Wait-PanePrompt -Target $winExitSess -TimeoutMs 15000 | Out-Null

    # Create extra windows
    $winCount = [math]::Max($Iterations, 5)
    for ($i = 0; $i -lt $winCount; $i++) {
        & $PSMUX new-window -t $winExitSess 2>&1 | Out-Null
    }
    Start-Sleep -Seconds 5  # Let shells load in all windows

    $winExitTimes = @()
    for ($i = 0; $i -lt $winCount; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $PSMUX kill-window -t $winExitSess 2>&1 | Out-Null
        $sw.Stop()
        $winExitTimes += $sw.ElapsedMilliseconds
        Write-Metric "  kill-window #$($i+1)" $sw.ElapsedMilliseconds
        Start-Sleep -Milliseconds 100
    }
    Write-Summary "kill-window latency" $winExitTimes
    $allResults["kill_window"] = $winExitTimes
}

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 5: SESSION EXIT TIME (kill-session)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "5. SESSION EXIT TIME (kill-session)"

$sessExitTimes = @()
for ($i = 0; $i -lt $Iterations; $i++) {
    Kill-All-Psmux
    $sess = "bench_sess_exit_$i"
    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null

    $si = Wait-SessionAlive -SessionName $sess -TimeoutMs 15000
    if ($null -eq $si) {
        Write-Host "    [SKIP] Session $sess did not start" -ForegroundColor Yellow
        continue
    }
    Wait-PanePrompt -Target $sess -TimeoutMs 15000 | Out-Null

    # Also create 2 extra windows to make it realistic
    & $PSMUX new-window -t $sess 2>&1 | Out-Null
    & $PSMUX split-window -v -t $sess 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-session -t $sess 2>&1 | Out-Null
    # Measure until server is actually dead
    $deadMs = Wait-SessionDead -SessionName $sess -TimeoutMs ($TimeoutSec * 1000)
    $sw.Stop()
    $sessExitTimes += $sw.ElapsedMilliseconds
    Write-Metric "  kill-session #$($i+1) (CLI returned)" $sw.ElapsedMilliseconds
    Write-Metric "    -> server dead after" $deadMs
}
Write-Summary "kill-session (CLI return)" $sessExitTimes
$allResults["kill_session"] = $sessExitTimes

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 6: KILL-SERVER TIME (multiple sessions)
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "6. KILL-SERVER TIME (3 sessions)"

$killServerTimes = @()
for ($i = 0; $i -lt [math]::Min($Iterations, 3); $i++) {
    Kill-All-Psmux

    # Create 3 sessions
    for ($s = 0; $s -lt 3; $s++) {
        $sn = "bench_ks_${i}_${s}"
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s $sn -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
    }
    # Wait for all 3 to be alive
    $allAlive = $true
    for ($s = 0; $s -lt 3; $s++) {
        $sn = "bench_ks_${i}_${s}"
        $info = Wait-SessionAlive -SessionName $sn -TimeoutMs 15000
        if ($null -eq $info) { $allAlive = $false; break }
    }
    if (-not $allAlive) {
        Write-Host "    [SKIP] Not all sessions started for kill-server #$($i+1)" -ForegroundColor Yellow
        continue
    }
    Start-Sleep -Seconds 2

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX kill-server 2>&1 | Out-Null
    $sw.Stop()
    $killServerTimes += $sw.ElapsedMilliseconds
    Write-Metric "  kill-server #$($i+1) (3 sessions)" $sw.ElapsedMilliseconds
}
Write-Summary "kill-server (3 sessions)" $killServerTimes
$allResults["kill_server"] = $killServerTimes

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 7: EXIT-EMPTY DETECTION LATENCY
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "7. EXIT-EMPTY DETECTION (shell exit -> server exits)"

$exitEmptyTimes = @()
for ($i = 0; $i -lt $Iterations; $i++) {
    Kill-All-Psmux
    $sess = "bench_ee_$i"
    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null

    $si = Wait-SessionAlive -SessionName $sess -TimeoutMs 15000
    if ($null -eq $si) {
        Write-Host "    [SKIP] Session $sess did not start" -ForegroundColor Yellow
        continue
    }
    Wait-PanePrompt -Target $sess -TimeoutMs 15000 | Out-Null

    # Enable exit-empty (default on, but be explicit)
    & $PSMUX set-option -g exit-empty on -t $sess 2>&1 | Out-Null

    # Send "exit" to the shell, then measure how long until server dies
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX send-keys -t $sess "exit" Enter 2>&1 | Out-Null
    $deadMs = Wait-SessionDead -SessionName $sess -TimeoutMs ($TimeoutSec * 1000)
    $sw.Stop()
    $exitEmptyTimes += $sw.ElapsedMilliseconds
    Write-Metric "  exit-empty #$($i+1)" $sw.ElapsedMilliseconds
}
Write-Summary "exit-empty latency" $exitEmptyTimes
$allResults["exit_empty"] = $exitEmptyTimes

# ══════════════════════════════════════════════════════════════════════════════
# BENCHMARK 8: DESTROY-UNATTACHED EXIT TIME
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "8. DESTROY-UNATTACHED EXIT TIME"

# This is harder to measure from CLI since we can't truly "attach".
# We can set destroy-unattached on, attach via TCP persistent, then detach.
$duTimes = @()
for ($i = 0; $i -lt $Iterations; $i++) {
    Kill-All-Psmux
    $sess = "bench_du_$i"
    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s $sess -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null

    $si = Wait-SessionAlive -SessionName $sess -TimeoutMs 15000
    if ($null -eq $si) {
        Write-Host "    [SKIP] Session $sess did not start" -ForegroundColor Yellow
        continue
    }
    Wait-PanePrompt -Target $sess -TimeoutMs 15000 | Out-Null

    # Enable destroy-unattached
    & $PSMUX set-option -g destroy-unattached on -t $sess 2>&1 | Out-Null

    # Simulate attach via TCP
    $key = (Get-Content "$PSMUX_DIR\${sess}.key" -Raw).Trim()
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.NoDelay = $true
    try {
        $tcp.Connect("127.0.0.1", $si.Port)
        $ns = $tcp.GetStream()
        $wr = New-Object System.IO.StreamWriter($ns)
        $wr.AutoFlush = $true
        $rd = New-Object System.IO.StreamReader($ns)
        $wr.WriteLine("AUTH $key")
        $auth = $rd.ReadLine()
        if ($auth -eq "OK") {
            $wr.WriteLine("PERSISTENT")
            $wr.WriteLine("client-attach")
            $wr.WriteLine("client-size 120 30")
            Start-Sleep -Milliseconds 500

            # Now detach (close connection) and measure exit time
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $tcp.Close()
            $deadMs = Wait-SessionDead -SessionName $sess -TimeoutMs ($TimeoutSec * 1000)
            $sw.Stop()
            $duTimes += $sw.ElapsedMilliseconds
            Write-Metric "  destroy-unattached #$($i+1)" $sw.ElapsedMilliseconds
        } else {
            Write-Host "    [SKIP] Auth failed for #$($i+1)" -ForegroundColor Yellow
        }
    } catch {
        Write-Host "    [SKIP] TCP error for #$($i+1): $_" -ForegroundColor Yellow
    }
}
Write-Summary "destroy-unattached exit" $duTimes
$allResults["destroy_unattached"] = $duTimes

# ══════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ══════════════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("*" * 76) -ForegroundColor Magenta
Write-Host "    FINAL BENCHMARK RESULTS" -ForegroundColor Magenta
Write-Host ("*" * 76) -ForegroundColor Magenta
Write-Host ""

$table = @(
    @{ Name = "Cold start (server ready)";    Key = "cold_start_server" }
    @{ Name = "Cold start (prompt ready)";    Key = "cold_start_prompt" }
    @{ Name = "Warm start (claim)";           Key = "warm_start_claim" }
    @{ Name = "Warm start (prompt)";          Key = "warm_start_prompt" }
    @{ Name = "kill-pane";                    Key = "kill_pane" }
    @{ Name = "kill-window";                  Key = "kill_window" }
    @{ Name = "kill-session";                 Key = "kill_session" }
    @{ Name = "kill-server (3 sess)";         Key = "kill_server" }
    @{ Name = "exit-empty";                   Key = "exit_empty" }
    @{ Name = "destroy-unattached";           Key = "destroy_unattached" }
)

Write-Host ("{0,-32} {1,8} {2,8} {3,8} {4,4}" -f "METRIC", "AVG(ms)", "MIN(ms)", "MAX(ms)", "N") -ForegroundColor White
Write-Host ("{0,-32} {1,8} {2,8} {3,8} {4,4}" -f ("─" * 32), ("─" * 8), ("─" * 8), ("─" * 8), ("─" * 4)) -ForegroundColor DarkGray

foreach ($row in $table) {
    $vals = $allResults[$row.Key]
    if ($null -eq $vals -or $vals.Count -eq 0) {
        Write-Host ("{0,-32} {1,8} {2,8} {3,8} {4,4}" -f $row.Name, "N/A", "N/A", "N/A", "0") -ForegroundColor DarkGray
    } else {
        $avg = [math]::Round(($vals | Measure-Object -Average).Average, 1)
        $min = [math]::Round(($vals | Measure-Object -Minimum).Minimum, 1)
        $max = [math]::Round(($vals | Measure-Object -Maximum).Maximum, 1)
        $n = $vals.Count
        $color = if ($avg -lt 500) { "Green" } elseif ($avg -lt 2000) { "Yellow" } else { "Red" }
        Write-Host ("{0,-32} {1,8} {2,8} {3,8} {4,4}" -f $row.Name, $avg, $min, $max, $n) -ForegroundColor $color
    }
}

Write-Host ""

# Cleanup
Kill-All-Psmux
Write-Host "Benchmark complete." -ForegroundColor Cyan
