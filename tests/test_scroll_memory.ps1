# test_scroll_memory.ps1 — Memory leak regression test for copy-mode scrolling
#
# Verifies that rapid scroll events in copy mode do not cause unbounded
# memory growth in the psmux server process.
#
# Background: push_frame() previously used unbounded mpsc::channel per client.
# Each scroll event triggered a ~500KB frame rebuild, and frames accumulated
# faster than the writer thread could flush.  Measured: 8 MB → 1 GB in <2000
# scroll events.  Fix: single-slot frame push that overwrites unconsumed frames.
#
# Usage:  pwsh tests/test_scroll_memory.ps1 [-ScrollCount 2000] [-MemoryLimitMB 500]

param(
    [int]$ScrollCount   = 2000,    # total scroll events to inject
    [int]$MemoryLimitMB = 500,     # fail if server exceeds this
    [int]$BurstSize     = 50,      # events per burst
    [int]$BurstDelayMs  = 10,      # ms between events within a burst
    [int]$PauseMs       = 200      # ms pause between bursts (lets server process)
)

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

# ── Resolve binary ──────────────────────────────────────────────────────────

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) {
    $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path
}
if (-not $PSMUX) {
    Write-Error "psmux binary not found — run 'cargo build --release' first"
    exit 1
}

Write-Info "Binary: $PSMUX"

$SESSION = "mem-leak-test"
$PSMUX_DIR = "$env:USERPROFILE\.psmux"

# ── Helpers ─────────────────────────────────────────────────────────────────

function Get-ServerPid {
    $portFile = "$PSMUX_DIR\$SESSION.port"
    if (!(Test-Path $portFile)) { return $null }
    $sessionPort = [int](Get-Content $portFile)
    $listener = netstat -ano 2>$null | Select-String "127\.0\.0\.1:$sessionPort\s" |
        Select-String "LISTENING" | Select-Object -First 1
    if ($listener) {
        $parts = ($listener.ToString().Trim()) -split '\s+'
        $foundPid = [int]$parts[-1]
        return Get-Process -Id $foundPid -ErrorAction SilentlyContinue
    }
    return Get-Process psmux -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -ne $PID } |
        Sort-Object StartTime -Descending |
        Select-Object -First 1
}

function Get-MemoryMB {
    param([int]$ProcessId)
    if ($ProcessId -eq 0) { return 0 }
    $p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $p) { return 0 }
    return [math]::Round($p.WorkingSet64 / 1MB, 1)
}

# ── Cleanup ─────────────────────────────────────────────────────────────────

Write-Info "Cleaning up prior test sessions..."
& $PSMUX kill-session -t $SESSION 2>$null | Out-Null
Start-Sleep -Seconds 1

# ── Start session ───────────────────────────────────────────────────────────

Write-Test "Starting detached session '$SESSION'"
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 4

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Session '$SESSION' failed to start"
    exit 1
}
Write-Pass "Session started"

# ── Get server process ──────────────────────────────────────────────────────

$serverProc = Get-ServerPid
if ($null -eq $serverProc) {
    Write-Fail "Could not find server process"
    & $PSMUX kill-session -t $SESSION 2>$null
    exit 1
}
$serverPid = $serverProc.Id
$baselineMB = Get-MemoryMB $serverPid
Write-Info "Server PID: $serverPid, baseline memory: ${baselineMB} MB"

# ── Fill scrollback ────────────────────────────────────────────────────────

Write-Test "Filling scrollback buffer with content..."
for ($i = 0; $i -lt 10; $i++) {
    & $PSMUX send-keys -t $SESSION "seq 1 100" Enter 2>$null | Out-Null
    Start-Sleep -Milliseconds 300
}
Start-Sleep -Seconds 2
Write-Pass "Scrollback populated"

# ── Connect TCP for scroll injection ────────────────────────────────────────

$portFile = "$PSMUX_DIR\$SESSION.port"
$keyFile  = "$PSMUX_DIR\$SESSION.key"

if (!(Test-Path $portFile) -or !(Test-Path $keyFile)) {
    Write-Fail "Port/key files not found for session '$SESSION'"
    & $PSMUX kill-session -t $SESSION 2>$null
    exit 1
}

$port = [int](Get-Content $portFile)
$key  = (Get-Content $keyFile).Trim()

Write-Info "Connecting to 127.0.0.1:$port for scroll injection..."
$tcp = [System.Net.Sockets.TcpClient]::new()
$tcp.NoDelay = $true
try {
    $tcp.Connect("127.0.0.1", $port)
} catch {
    Write-Fail "TCP connection failed: $_"
    & $PSMUX kill-session -t $SESSION 2>$null
    exit 1
}

$stream = $tcp.GetStream()
$writer = [System.IO.StreamWriter]::new($stream)
$writer.AutoFlush = $true

$writer.WriteLine("AUTH $key")
$writer.WriteLine("PERSISTENT")
Start-Sleep -Milliseconds 200

Write-Pass "TCP connected and authenticated"

# ── Inject scroll events in bursts ──────────────────────────────────────────

Write-Test "Injecting $ScrollCount scroll-up events (burst=$BurstSize, delay=${BurstDelayMs}ms)..."

$memorySamples = @()
$sent = 0
$burstNum = 0

$memorySamples += [PSCustomObject]@{
    Events = 0; MemoryMB = $baselineMB; Timestamp = (Get-Date)
}

while ($sent -lt $ScrollCount) {
    $burstNum++
    $thisBurst = [math]::Min($BurstSize, $ScrollCount - $sent)

    for ($i = 0; $i -lt $thisBurst; $i++) {
        try { $writer.WriteLine("scroll-up 40 20") } catch {
            Write-Fail "TCP write failed at event $sent : $_"; break
        }
        $sent++
        if ($BurstDelayMs -gt 0) { Start-Sleep -Milliseconds $BurstDelayMs }
    }

    $currentMB = Get-MemoryMB $serverPid
    $memorySamples += [PSCustomObject]@{
        Events = $sent; MemoryMB = $currentMB; Timestamp = (Get-Date)
    }

    if ($currentMB -gt ($MemoryLimitMB * 2)) {
        Write-Fail "EARLY ABORT: memory at ${currentMB} MB after $sent events (limit: $MemoryLimitMB MB)"
        break
    }

    if ($burstNum % 5 -eq 0) {
        Write-Info "  $sent/$ScrollCount events sent — server at ${currentMB} MB"
    }

    if ($PauseMs -gt 0) { Start-Sleep -Milliseconds $PauseMs }
}

Start-Sleep -Seconds 2
$finalMB = Get-MemoryMB $serverPid
$memorySamples += [PSCustomObject]@{
    Events = $sent; MemoryMB = $finalMB; Timestamp = (Get-Date)
}

Write-Info "Injection complete: $sent events sent"

try { $tcp.Close() } catch {}

# ── Verify copy mode was entered ────────────────────────────────────────────

Write-Test "Verifying copy mode was triggered..."
$inMode = & $PSMUX display-message -t $SESSION -p '#{pane_in_mode}' 2>$null
if ($inMode -match "1") {
    Write-Pass "Pane entered copy mode (as expected from scroll injection)"
} else {
    Write-Info "Pane not in copy mode (may have auto-exited) — mode=$inMode"
}

# ── Memory analysis ─────────────────────────────────────────────────────────

Write-Test "Analyzing memory growth..."

$peakMB = ($memorySamples | Measure-Object -Property MemoryMB -Maximum).Maximum
$growthMB = [math]::Round($finalMB - $baselineMB, 1)
$duration = ($memorySamples[-1].Timestamp - $memorySamples[0].Timestamp).TotalSeconds
$growthRate = if ($duration -gt 0) { [math]::Round($growthMB / $duration, 1) } else { 0 }

Write-Info "  Baseline:    ${baselineMB} MB"
Write-Info "  Peak:        ${peakMB} MB"
Write-Info "  Final:       ${finalMB} MB"
Write-Info "  Growth:      ${growthMB} MB over $([math]::Round($duration, 1))s"
Write-Info "  Growth rate: ${growthRate} MB/s"
Write-Info "  Samples:     $($memorySamples.Count)"

Write-Host ""
Write-Host "  Events  | Memory (MB)" -ForegroundColor DarkGray
Write-Host "  --------|------------" -ForegroundColor DarkGray
foreach ($s in $memorySamples) {
    $bar = "#" * [math]::Min([math]::Max([int]($s.MemoryMB / 10), 1), 50)
    Write-Host ("  {0,6}  | {1,8:N1}  {2}" -f $s.Events, $s.MemoryMB, $bar) -ForegroundColor DarkGray
}
Write-Host ""

# ── Assertions ──────────────────────────────────────────────────────────────

if ($peakMB -le $MemoryLimitMB) {
    Write-Pass "Peak memory ${peakMB} MB within limit (${MemoryLimitMB} MB)"
} else {
    Write-Fail "Peak memory ${peakMB} MB EXCEEDS limit (${MemoryLimitMB} MB)"
}

# The original leak was 22+ MB/s; a healthy server should be < 5 MB/s
if ($growthRate -lt 10) {
    Write-Pass "Growth rate ${growthRate} MB/s is acceptable"
} else {
    Write-Fail "Growth rate ${growthRate} MB/s suggests unbounded allocation"
}

# ── Cleanup ─────────────────────────────────────────────────────────────────

Write-Info "Cleaning up..."
& $PSMUX kill-session -t $SESSION 2>$null | Out-Null
Start-Sleep -Seconds 1

# ── Summary ─────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "======================================================" -ForegroundColor White
Write-Host "  Scroll Memory Test: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "  Peak: ${peakMB} MB | Growth: ${growthMB} MB | Rate: ${growthRate} MB/s" -ForegroundColor White
Write-Host "======================================================" -ForegroundColor White

exit $script:TestsFailed
