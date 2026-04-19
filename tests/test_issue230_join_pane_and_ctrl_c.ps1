# Issue #230: join-pane silent no-op; send-keys C-c not propagating SIGINT
# Tests that:
#   1. join-pane actually moves a pane from one window/session to another
#   2. send-keys C-c propagates SIGINT to child processes
# GOAL: VERIFY if these bugs are real with tangible proof

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    @("test230_donor", "test230_target", "test230_sig", "test230_jp", "test230_jp2",
      "test230_same", "test230_cross_src", "test230_cross_tgt", "test230_movep") | ForEach-Object {
        & $PSMUX kill-session -t $_ 2>&1 | Out-Null
    }
    Start-Sleep -Milliseconds 1000
    @("test230_donor", "test230_target", "test230_sig", "test230_jp", "test230_jp2",
      "test230_same", "test230_cross_src", "test230_cross_tgt", "test230_movep") | ForEach-Object {
        Remove-Item "$psmuxDir\$_.*" -Force -EA SilentlyContinue
    }
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
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Send-TcpCommand {
    param([string]$Session, [string]$Command)
    $portFile = "$psmuxDir\$Session.port"
    $keyFile = "$psmuxDir\$Session.key"
    if (-not (Test-Path $portFile) -or -not (Test-Path $keyFile)) { return "NO_SESSION_FILES" }
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
        if ($authResp -ne "OK") { $tcp.Close(); return "AUTH_FAILED: $authResp" }
        $writer.Write("$Command`n"); $writer.Flush()
        $stream.ReadTimeout = 10000
        try { $resp = $reader.ReadLine() } catch { $resp = "TIMEOUT" }
        $tcp.Close()
        return $resp
    } catch {
        return "TCP_ERROR: $_"
    }
}

# ============================================================
Cleanup
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Issue #230 Validation Tests" -ForegroundColor Cyan
Write-Host "  psmux version: $(& $PSMUX -V 2>&1)" -ForegroundColor Cyan
Write-Host "========================================`n" -ForegroundColor Cyan

# ============================================================
# PART A: join-pane TESTS (CLI path)
# ============================================================
Write-Host "=== PART A: join-pane via CLI ===" -ForegroundColor Cyan

# --- Test A1: join-pane within same session (2 windows, move pane from win1 to win0) ---
Write-Host "`n[Test A1] join-pane within same session (window to window)" -ForegroundColor Yellow

$S = "test230_jp"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

# Create a second window and split it so it has 2 panes
& $PSMUX new-window -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2
& $PSMUX split-window -v -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

# Check window count and pane counts before
$winCountBefore = (& $PSMUX display-message -t $S -p '#{session_windows}' 2>&1).Trim()
Write-Host "    Windows before: $winCountBefore"

# Get pane count in window 0
& $PSMUX select-window -t "${S}:0" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$panesW0Before = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 0 panes before: $panesW0Before"

# Get pane count in window 1
& $PSMUX select-window -t "${S}:1" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$panesW1Before = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 1 panes before: $panesW1Before"

# Now try join-pane: move pane from window 1 to window 0
$joinResult = & $PSMUX join-pane -h -s "${S}:1.0" -t "${S}:0.0" 2>&1 | Out-String
$joinExit = $LASTEXITCODE
Write-Host "    join-pane output: '$($joinResult.Trim())' (exit: $joinExit)"
Start-Sleep -Seconds 2

# Check pane counts after
& $PSMUX select-window -t "${S}:0" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$panesW0After = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 0 panes after: $panesW0After"

$winCountAfter = (& $PSMUX display-message -t $S -p '#{session_windows}' 2>&1).Trim()
Write-Host "    Windows after: $winCountAfter"

# If join-pane worked: window 0 should have gained a pane (1 -> 2)
if ([int]$panesW0After -gt [int]$panesW0Before) {
    Write-Pass "join-pane moved pane to window 0 (was $panesW0Before panes, now $panesW0After)"
} else {
    Write-Fail "join-pane DID NOT move pane. Window 0 still has $panesW0After panes (was $panesW0Before). BUG CONFIRMED: silent no-op"
}

# If donor window had only 1 pane left after split-pane's pane was moved, it might have been closed
# If it had 2 panes (from split), moving 1 should leave 1
if ($winCountBefore -eq "2" -and $winCountAfter -eq "2") {
    # Window 1 should now have 1 fewer pane
    & $PSMUX select-window -t "${S}:1" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $panesW1After = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
    if ([int]$panesW1After -lt [int]$panesW1Before) {
        Write-Pass "Donor window lost a pane (was $panesW1Before, now $panesW1After)"
    } else {
        Write-Fail "Donor window pane count unchanged ($panesW1After). Source pane was not removed."
    }
} elseif ([int]$winCountAfter -lt [int]$winCountBefore) {
    Write-Pass "Donor window was closed after last pane moved (windows: $winCountBefore -> $winCountAfter)"
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test A2: join-pane with bare window index (simplified args) ---
Write-Host "`n[Test A2] join-pane with simplified bare index arg" -ForegroundColor Yellow

$S = "test230_jp2"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

# Create 2 windows
& $PSMUX new-window -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

# Split window 1
& $PSMUX split-window -v -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

$panesW0Before2 = (& $PSMUX display-message -t "${S}:0" -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 0 panes before: $panesW0Before2"

# Try join-pane with just a target window (no -s/-t, use current pane as source)
& $PSMUX select-window -t "${S}:1" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$joinResult2 = & $PSMUX join-pane -t "${S}:0" 2>&1 | Out-String
$joinExit2 = $LASTEXITCODE
Write-Host "    join-pane -t ${S}:0 output: '$($joinResult2.Trim())' (exit: $joinExit2)"
Start-Sleep -Seconds 2

$panesW0After2 = (& $PSMUX display-message -t "${S}:0" -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 0 panes after: $panesW0After2"

if ([int]$panesW0After2 -gt [int]$panesW0Before2) {
    Write-Pass "join-pane with -t only moved pane (was $panesW0Before2, now $panesW0After2)"
} else {
    Write-Fail "join-pane with -t only DID NOT move pane. Still $panesW0After2 panes. BUG CONFIRMED."
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test A3: join-pane same window (move pane within same window should reflow) ---
Write-Host "`n[Test A3] join-pane within same window" -ForegroundColor Yellow

$S = "test230_same"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

# Create 3 panes in window 0
& $PSMUX split-window -v -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2
& $PSMUX split-window -h -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

$panesBefore3 = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Panes before: $panesBefore3"

# join-pane within same window should either error or reorganize layout
$joinResult3 = & $PSMUX join-pane -s "${S}:0.2" -t "${S}:0.0" 2>&1 | Out-String
$joinExit3 = $LASTEXITCODE
Write-Host "    join-pane same window output: '$($joinResult3.Trim())' (exit: $joinExit3)"

$panesAfter3 = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Panes after: $panesAfter3"

# tmux would error "src and dst panes must be in different windows" for same-window join
if ($joinResult3 -match "same|different|error|cannot" -or $joinExit3 -ne 0) {
    Write-Pass "join-pane same window rejected or errored (correct tmux behavior)"
} elseif ($panesBefore3 -eq $panesAfter3) {
    Write-Fail "join-pane same window: no change, no error. Silent no-op."
} else {
    Write-Pass "join-pane same window: layout changed ($panesBefore3 -> $panesAfter3)"
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test A4: join-pane cross-session ---
Write-Host "`n[Test A4] join-pane cross-session (donor to target)" -ForegroundColor Yellow

$SRC = "test230_cross_src"
$TGT = "test230_cross_tgt"
& $PSMUX new-session -d -s $SRC
Start-Sleep -Seconds 3
if (-not (Wait-Session $SRC)) { Write-Fail "Session $SRC did not start"; exit 1 }

& $PSMUX new-session -d -s $TGT
Start-Sleep -Seconds 3
if (-not (Wait-Session $TGT)) { Write-Fail "Session $TGT did not start"; exit 1 }

# Split source so it has 2 panes
& $PSMUX split-window -h -t $SRC 2>&1 | Out-Null
Start-Sleep -Seconds 2

$srcPanesBefore = (& $PSMUX display-message -t $SRC -p '#{window_panes}' 2>&1).Trim()
$tgtPanesBefore = (& $PSMUX display-message -t $TGT -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Source panes before: $srcPanesBefore"
Write-Host "    Target panes before: $tgtPanesBefore"

# Cross-session join-pane
$joinResult4 = & $PSMUX join-pane -h -s "${SRC}:0.1" -t "${TGT}:0.0" 2>&1 | Out-String
$joinExit4 = $LASTEXITCODE
Write-Host "    join-pane cross-session output: '$($joinResult4.Trim())' (exit: $joinExit4)"
Start-Sleep -Seconds 2

$srcPanesAfter = (& $PSMUX display-message -t $SRC -p '#{window_panes}' 2>&1).Trim()
$tgtPanesAfter = (& $PSMUX display-message -t $TGT -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Source panes after: $srcPanesAfter"
Write-Host "    Target panes after: $tgtPanesAfter"

if ([int]$tgtPanesAfter -gt [int]$tgtPanesBefore) {
    Write-Pass "Cross-session join-pane moved pane to target (was $tgtPanesBefore, now $tgtPanesAfter)"
} else {
    Write-Fail "Cross-session join-pane DID NOT move pane. Target still has $tgtPanesAfter panes. BUG CONFIRMED."
}

if ([int]$srcPanesAfter -lt [int]$srcPanesBefore) {
    Write-Pass "Source lost pane after cross-session join (was $srcPanesBefore, now $srcPanesAfter)"
} else {
    Write-Fail "Source pane count unchanged ($srcPanesAfter). Source pane was not removed."
}

& $PSMUX kill-session -t $SRC 2>&1 | Out-Null
& $PSMUX kill-session -t $TGT 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test A5: move-pane alias (should work same as join-pane) ---
Write-Host "`n[Test A5] move-pane alias test" -ForegroundColor Yellow

$S = "test230_movep"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

& $PSMUX new-window -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2
& $PSMUX split-window -v -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

$panesW0BeforeM = (& $PSMUX display-message -t "${S}:0" -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 0 panes before: $panesW0BeforeM"

$moveResult = & $PSMUX move-pane -h -s "${S}:1.0" -t "${S}:0.0" 2>&1 | Out-String
$moveExit = $LASTEXITCODE
Write-Host "    move-pane output: '$($moveResult.Trim())' (exit: $moveExit)"
Start-Sleep -Seconds 2

$panesW0AfterM = (& $PSMUX display-message -t "${S}:0" -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 0 panes after: $panesW0AfterM"

if ([int]$panesW0AfterM -gt [int]$panesW0BeforeM) {
    Write-Pass "move-pane alias worked (was $panesW0BeforeM, now $panesW0AfterM)"
} else {
    Write-Fail "move-pane alias DID NOT move pane. Still $panesW0AfterM panes. BUG CONFIRMED."
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# ============================================================
# PART B: join-pane via TCP (raw socket)
# ============================================================
Write-Host "`n=== PART B: join-pane via TCP ===" -ForegroundColor Cyan

$S = "test230_donor"
$T = "test230_target"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

# Split so donor has 2 panes
& $PSMUX split-window -h -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

$panesBefore_tcp = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()

# --- Test B1: join-pane via raw TCP with -s -t flags ---
Write-Host "`n[Test B1] join-pane via TCP with -s -t flags" -ForegroundColor Yellow

& $PSMUX new-window -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

$resp = Send-TcpCommand -Session $S -Command "join-pane -h -s ${S}:0.1 -t ${S}:1.0"
Write-Host "    TCP response: '$resp'"

Start-Sleep -Seconds 2
& $PSMUX select-window -t "${S}:1" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$panesW1_tcp = (& $PSMUX display-message -t $S -p '#{window_panes}' 2>&1).Trim()
Write-Host "    Window 1 panes after TCP join-pane: $panesW1_tcp"

if ([int]$panesW1_tcp -gt 1) {
    Write-Pass "TCP join-pane moved pane (window 1 now has $panesW1_tcp panes)"
} else {
    Write-Fail "TCP join-pane DID NOT move pane. Window 1 still has $panesW1_tcp pane(s). BUG CONFIRMED."
}

# --- Test B2: join-pane via TCP with bare integer arg ---
Write-Host "`n[Test B2] join-pane via TCP with bare integer" -ForegroundColor Yellow

$resp2 = Send-TcpCommand -Session $S -Command "join-pane 0"
Write-Host "    TCP response (bare int): '$resp2'"

# This tests the path the issue identified: bare usize parsing
if ($resp2 -eq "OK" -or $resp2 -match "success") {
    Write-Pass "TCP join-pane with bare int accepted (response: $resp2)"
} elseif ($resp2 -eq "TIMEOUT" -or $resp2 -eq "NC") {
    Write-Fail "TCP join-pane with bare int: no response (TIMEOUT/NC)"
} else {
    Write-Host "    Note: got response '$resp2'" -ForegroundColor DarkGray
    Write-Pass "TCP join-pane with bare int returned a response (may be error)"
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# ============================================================
# PART C: send-keys C-c TESTS
# ============================================================
Write-Host "`n=== PART C: send-keys C-c (SIGINT propagation) ===" -ForegroundColor Cyan

# --- Test C1: send-keys C-c to stop ping (cmd.exe child) ---
Write-Host "`n[Test C1] send-keys C-c to stop ping process" -ForegroundColor Yellow

$S = "test230_sig"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

# Wait for shell prompt
Start-Sleep -Seconds 3

# Start a ping that runs continuously
& $PSMUX send-keys -t $S "ping -t 127.0.0.1" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 4

# Verify ping is running (capture pane should show ping output)
$capBefore = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
$pingRunning = $capBefore -match "Reply from|Pinging|bytes="
Write-Host "    Ping running: $pingRunning"
if (-not $pingRunning) {
    Write-Host "    Capture before C-c:`n$capBefore" -ForegroundColor DarkGray
}

# Send C-c
Write-Host "    Sending C-c..."
& $PSMUX send-keys -t $S C-c 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Check if ping stopped (capture-pane should show new prompt or "Ping statistics")
$capAfter = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String

# The ping should have stopped. Look for either:
# 1. "Ping statistics" / "Approximate round trip" (ping printed summary and exited)
# 2. A new command prompt line (ping exited, shell is back)
# 3. "Control-C" or "^C" visible
$pingStats = $capAfter -match "Ping statistics|Approximate round trip|Control-C|\^C|Packets:|Average"
$promptReturned = $capAfter -match "PS [A-Z]:\\|>\s*$|C:\\.*>"

Write-Host "    Ping stats visible: $pingStats"
Write-Host "    Prompt returned: $promptReturned"

if ($pingStats -or $promptReturned) {
    Write-Pass "send-keys C-c stopped ping (stats/prompt visible)"
} else {
    # Double-check: is ping still running?
    & $PSMUX send-keys -t $S "" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $capFinal = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
    $stillPinging = $capFinal -match "Reply from 127\.0\.0\.1"
    
    if ($stillPinging) {
        Write-Fail "send-keys C-c DID NOT stop ping. Still receiving replies. BUG CONFIRMED."
        Write-Host "    === Capture after C-c ===" -ForegroundColor DarkGray
        Write-Host $capAfter -ForegroundColor DarkGray
    } else {
        Write-Pass "send-keys C-c appears to have stopped ping (no more replies)"
    }
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test C2: send-keys C-c via raw TCP ---
Write-Host "`n[Test C2] send-keys C-c via TCP to stop long process" -ForegroundColor Yellow

$S = "test230_sig"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

Start-Sleep -Seconds 3

# Start a long-running process
& $PSMUX send-keys -t $S "ping -t 127.0.0.1" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 4

# Send C-c via TCP
$resp = Send-TcpCommand -Session $S -Command "send-keys C-c"
Write-Host "    TCP send-keys C-c response: '$resp'"
Start-Sleep -Seconds 3

$capAfterTcp = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
$pingStoppedTcp = ($capAfterTcp -match "Ping statistics|Control-C|\^C|PS [A-Z]:\\|C:\\.*>")

if ($pingStoppedTcp) {
    Write-Pass "TCP send-keys C-c stopped ping"
} else {
    Write-Fail "TCP send-keys C-c DID NOT stop ping. BUG CONFIRMED (TCP path)."
    Write-Host "    Capture:`n$capAfterTcp" -ForegroundColor DarkGray
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test C3: send-keys C-c to PowerShell child ---
Write-Host "`n[Test C3] send-keys C-c to PowerShell child process" -ForegroundColor Yellow

$S = "test230_sig"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

Start-Sleep -Seconds 3

# Start a long-running PowerShell command
& $PSMUX send-keys -t $S 'while ($true) { Write-Host "RUNNING $(Get-Date -Format HH:mm:ss)"; Start-Sleep -Seconds 1 }' Enter 2>&1 | Out-Null
Start-Sleep -Seconds 5

# Verify it is running
$capPS = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
$psRunning = $capPS -match "RUNNING"
Write-Host "    PS loop running: $psRunning"

# Send C-c
& $PSMUX send-keys -t $S C-c 2>&1 | Out-Null
Start-Sleep -Seconds 3

$capPSAfter = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
# Check if the loop stopped
& $PSMUX send-keys -t $S "echo AFTER_CTRL_C" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2
$capPSFinal = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
$psPromptBack = $capPSFinal -match "AFTER_CTRL_C"

if ($psPromptBack) {
    Write-Pass "send-keys C-c stopped PowerShell loop (prompt returned, echo worked)"
} else {
    Write-Fail "send-keys C-c DID NOT stop PowerShell loop. BUG CONFIRMED (PowerShell child)."
    Write-Host "    Capture after C-c:`n$capPSFinal" -ForegroundColor DarkGray
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# --- Test C4: send-keys C-c with explicit 0x03 byte comparison ---
Write-Host "`n[Test C4] send-keys with literal ^C byte (0x03)" -ForegroundColor Yellow

$S = "test230_sig"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

Start-Sleep -Seconds 3

& $PSMUX send-keys -t $S "ping -t 127.0.0.1" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 4

# Try sending literal hex 0x03 (ETX / Ctrl-C) as an alternative
& $PSMUX send-keys -t $S -l ([char]0x03) 2>&1 | Out-Null
Start-Sleep -Seconds 3

$capHex = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
$hexWorked = $capHex -match "Ping statistics|Control-C|\^C|PS [A-Z]:\\"

if ($hexWorked) {
    Write-Pass "Literal 0x03 byte stopped ping"
} else {
    Write-Fail "Literal 0x03 byte DID NOT stop ping either."
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# ============================================================
# PART D: Edge cases
# ============================================================
Write-Host "`n=== PART D: Edge Cases ===" -ForegroundColor Cyan

# --- Test D1: join-pane with invalid source ---
Write-Host "`n[Test D1] join-pane with nonexistent source" -ForegroundColor Yellow

$S = "test230_jp"
& $PSMUX new-session -d -s $S
Start-Sleep -Seconds 3
if (-not (Wait-Session $S)) { Write-Fail "Session $S did not start"; exit 1 }

$badResult = & $PSMUX join-pane -s "nonexistent:0.0" -t "${S}:0.0" 2>&1 | Out-String
$badExit = $LASTEXITCODE
Write-Host "    Bad source result: '$($badResult.Trim())' (exit: $badExit)"

if ($badExit -ne 0 -or $badResult -match "error|not found|no such|cannot|invalid") {
    Write-Pass "join-pane with bad source returns error"
} else {
    Write-Fail "join-pane with bad source: no error (exit $badExit). Should error."
}

# --- Test D2: join-pane with no args ---
Write-Host "`n[Test D2] join-pane with no arguments" -ForegroundColor Yellow

$noArgResult = & $PSMUX join-pane 2>&1 | Out-String
$noArgExit = $LASTEXITCODE
Write-Host "    No-args result: '$($noArgResult.Trim())' (exit: $noArgExit)"

# tmux requires at least -t to know the target
if ($noArgResult -match "error|usage|missing|cannot|no " -or $noArgExit -ne 0) {
    Write-Pass "join-pane with no args returns error/usage"
} else {
    Write-Fail "join-pane with no args: no error returned. Might be silent no-op."
}

# --- Test D3: movep alias (the issue mentions it returns unknown command) ---
Write-Host "`n[Test D3] movep alias" -ForegroundColor Yellow

$movepResult = & $PSMUX movep 2>&1 | Out-String
$movepExit = $LASTEXITCODE
Write-Host "    movep result: '$($movepResult.Trim())' (exit: $movepExit)"

if ($movepResult -match "unknown command") {
    Write-Fail "movep alias not registered (returns 'unknown command'). Alias gap confirmed."
} elseif ($movepResult -match "error|usage|missing") {
    Write-Pass "movep alias exists (returned usage/error)"
} else {
    Write-Pass "movep alias accepted (response: $($movepResult.Trim()))"
}

# --- Test D4: joinp alias ---
Write-Host "`n[Test D4] joinp alias" -ForegroundColor Yellow

$joinpResult = & $PSMUX joinp 2>&1 | Out-String
$joinpExit = $LASTEXITCODE
Write-Host "    joinp result: '$($joinpResult.Trim())' (exit: $joinpExit)"

if ($joinpResult -match "unknown command") {
    Write-Fail "joinp alias not registered"
} else {
    Write-Pass "joinp alias recognized"
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# ============================================================
# CLEANUP
# ============================================================
Cleanup

Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Issue #230 Validation Results" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host ""

if ($script:TestsFailed -gt 0) {
    Write-Host "  CONCLUSION: Bug(s) from issue #230 are CONFIRMED." -ForegroundColor Red
} else {
    Write-Host "  CONCLUSION: All tested behaviors work correctly." -ForegroundColor Green
}

exit $script:TestsFailed
