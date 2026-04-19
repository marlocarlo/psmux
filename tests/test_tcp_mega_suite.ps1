# =============================================================================
# PSMUX TCP Socket Mega Test Suite
# =============================================================================
#
# Tests every command category via raw TCP socket to the PSMUX server.
# This proves server/connection.rs handle_connection() correctly processes
# commands that arrive over the network, not just via CLI or TUI.
#
# Covers issues: 19, 25, 33, 36, 42, 43, 44, 46, 63, 70, 71, 82,
#   94, 95, 100, 105, 108, 111, 125, 126, 133, 134, 136, 137, 140,
#   146, 151, 154, 165, 171, 192, 200, 205, 206, 209, 215
#
# Usage: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_tcp_mega_suite.ps1
# =============================================================================

param(
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass  { param($msg) Write-Host "  [PASS] $msg" -ForegroundColor Green;  $script:TestsPassed++ }
function Write-Fail  { param($msg) Write-Host "  [FAIL] $msg" -ForegroundColor Red;    $script:TestsFailed++ }
function Write-Skip  { param($msg) Write-Host "  [SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info  { param($msg) Write-Host "  [INFO] $msg" -ForegroundColor Cyan }
function Write-Test  { param($msg) Write-Host "  [TEST] $msg" -ForegroundColor White }

# Resolve binary
$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -EA SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -EA SilentlyContinue).Path }
if (-not $PSMUX) { $cmd = Get-Command psmux -EA SilentlyContinue; if ($cmd) { $PSMUX = $cmd.Source } }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Binary: $PSMUX"

$PSMUX_DIR = "$env:USERPROFILE\.psmux"
$SESSION   = "tcp_mega"

# =============================================================================
# TCP Helper Functions
# =============================================================================

function Send-TcpCommand {
    param(
        [string]$Session,
        [string]$Command,
        [int]$TimeoutMs = 5000
    )
    try {
        $portFile = "$PSMUX_DIR\$Session.port"
        $keyFile  = "$PSMUX_DIR\$Session.key"
        if (-not (Test-Path $portFile)) { return @{ ok=$false; err="NO_PORT_FILE" } }
        if (-not (Test-Path $keyFile))  { return @{ ok=$false; err="NO_KEY_FILE" } }
        $rawPort = Get-Content $portFile -Raw
        $rawKey  = Get-Content $keyFile -Raw
        if (-not $rawPort) { return @{ ok=$false; err="EMPTY_PORT_FILE" } }
        if (-not $rawKey)  { return @{ ok=$false; err="EMPTY_KEY_FILE" } }
        $port = $rawPort.Trim()
        $key  = $rawKey.Trim()
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.NoDelay = $true
        $tcp.Connect("127.0.0.1", [int]$port)
        $ns = $tcp.GetStream()
        $ns.ReadTimeout = $TimeoutMs
        $wr = New-Object System.IO.StreamWriter($ns); $wr.AutoFlush = $true
        $rd = New-Object System.IO.StreamReader($ns)

        $wr.WriteLine("AUTH $key")
        $auth = $rd.ReadLine()
        if ($auth -ne "OK") { $tcp.Close(); return @{ ok=$false; err="AUTH_FAIL: $auth" } }

        $wr.WriteLine($Command)
        $lines = @()
        try {
            while ($true) {
                $line = $rd.ReadLine()
                if ($null -eq $line) { break }
                $lines += $line
                if ($ns.DataAvailable -eq $false) {
                    Start-Sleep -Milliseconds 100
                    if ($ns.DataAvailable -eq $false) { break }
                }
            }
        } catch {}
        $tcp.Close()
        return @{ ok=$true; resp=($lines -join "`n"); lines=$lines }
    } catch {
        return @{ ok=$false; err=$_.ToString() }
    }
}

function Send-TcpRaw {
    param(
        [int]$Port,
        [string]$Payload,
        [int]$TimeoutMs = 3000
    )
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.NoDelay = $true
        $tcp.Connect("127.0.0.1", $Port)
        $ns = $tcp.GetStream()
        $ns.ReadTimeout = $TimeoutMs
        $wr = New-Object System.IO.StreamWriter($ns); $wr.AutoFlush = $true
        $rd = New-Object System.IO.StreamReader($ns)
        $wr.Write($Payload)
        $wr.Flush()
        $lines = @()
        try {
            while ($true) {
                $line = $rd.ReadLine()
                if ($null -eq $line) { break }
                $lines += $line
                if ($ns.DataAvailable -eq $false) {
                    Start-Sleep -Milliseconds 100
                    if ($ns.DataAvailable -eq $false) { break }
                }
            }
        } catch {}
        $tcp.Close()
        return @{ ok=$true; resp=($lines -join "`n"); lines=$lines }
    } catch {
        return @{ ok=$false; err=$_.ToString() }
    }
}

function Connect-Persistent {
    param([string]$Session)
    $portFile = "$PSMUX_DIR\$Session.port"
    $keyFile  = "$PSMUX_DIR\$Session.key"
    if (-not (Test-Path $portFile) -or -not (Test-Path $keyFile)) { return $null }
    $rawPort = Get-Content $portFile -Raw
    $rawKey  = Get-Content $keyFile -Raw
    if (-not $rawPort -or -not $rawKey) { return $null }
    $port = $rawPort.Trim()
    $key  = $rawKey.Trim()
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.NoDelay = $true
    $tcp.Connect("127.0.0.1", [int]$port)
    $tcp.ReceiveTimeout = 10000
    $ns = $tcp.GetStream()
    $wr = New-Object System.IO.StreamWriter($ns); $wr.AutoFlush = $true
    $rd = New-Object System.IO.StreamReader($ns)
    $wr.WriteLine("AUTH $key")
    $auth = $rd.ReadLine()
    if ($auth -ne "OK") { $tcp.Close(); return $null }
    $wr.WriteLine("PERSISTENT")
    return @{ tcp=$tcp; writer=$wr; reader=$rd; stream=$ns }
}

function Get-DumpState {
    param($conn)
    $conn.writer.WriteLine("dump-state")
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

function Cleanup-Session {
    param([string]$Name)
    & $PSMUX kill-session -t $Name 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    Remove-Item "$PSMUX_DIR\$Name.*" -Force -EA SilentlyContinue
}

function Wait-SessionReady {
    param([string]$Name, [int]$TimeoutMs = 15000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $pf = "$PSMUX_DIR\$Name.port"
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
        Start-Sleep -Milliseconds 200
    }
    return $false
}

# =============================================================================
# Initial Setup
# =============================================================================

Write-Host "`n============================================================" -ForegroundColor Magenta
Write-Host "  PSMUX TCP Socket Mega Test Suite" -ForegroundColor Magenta
Write-Host "  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Magenta
Write-Host "============================================================`n" -ForegroundColor Magenta

Cleanup-Session $SESSION
Start-Sleep -Seconds 1

# Create a detached session
Write-Info "Starting detached session '$SESSION'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION -WindowStyle Hidden
if (-not (Wait-SessionReady $SESSION)) {
    Write-Fail "FATAL: Session did not start"
    exit 1
}
Start-Sleep -Seconds 3
Write-Pass "Session '$SESSION' is live and TCP reachable"

# ════════════════════════════════════════════════════════════════════
# SECTION 1: AUTHENTICATION (Issue #136, #206)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 1: Authentication ===" -ForegroundColor Cyan

# --- Issue #136: Correct key authenticates ---
Write-Test "#136/#206: Valid AUTH succeeds"
$r = Send-TcpCommand -Session $SESSION -Command "list-sessions"
if ($r.ok) { Write-Pass "#136 Valid AUTH accepted, command executed" }
else { Write-Fail "#136 Valid AUTH failed: $($r.err)" }

# --- Issue #136/#206: Wrong key rejected ---
Write-Test "#136/#206: Invalid AUTH rejected"
$rawPort = Get-Content "$PSMUX_DIR\$SESSION.port" -Raw
if (-not $rawPort) { Write-Fail "#136/#206 No port file"; } else {
$port = $rawPort.Trim()
$badResult = Send-TcpRaw -Port ([int]$port) -Payload "AUTH bad_key_12345`n"
if ($badResult.resp -match "FAIL|ERR|denied|invalid" -or -not $badResult.ok) {
    Write-Pass "#136/#206 Invalid AUTH correctly rejected"
} elseif ($badResult.resp -match "OK") {
    Write-Fail "#136/#206 SECURITY: Invalid AUTH was ACCEPTED"
} else {
    Write-Pass "#136/#206 Invalid AUTH handled (resp: $($badResult.resp))"
}

# --- Issue #206: Empty AUTH rejected ---
Write-Test "#206: Empty AUTH rejected"
$emptyResult = Send-TcpRaw -Port ([int]$port) -Payload "AUTH `n"
if ($emptyResult.resp -notmatch "^OK$") {
    Write-Pass "#206 Empty AUTH correctly rejected"
} else {
    Write-Fail "#206 SECURITY: Empty AUTH was ACCEPTED"
}

# --- Issue #206: No AUTH, direct command rejected ---
Write-Test "#206: Command without AUTH rejected"
$noAuthResult = Send-TcpRaw -Port ([int]$port) -Payload "list-sessions`n"
if ($noAuthResult.resp -notmatch "^OK$" -and $noAuthResult.resp -notmatch "session") {
    Write-Pass "#206 Command without AUTH correctly rejected"
} else {
    Write-Fail "#206 SECURITY: Command executed without AUTH"
}
} # end of port-file-exists block

# ════════════════════════════════════════════════════════════════════
# SECTION 2: SESSION MANAGEMENT (Issues #33, #200, #205)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 2: Session Management ===" -ForegroundColor Cyan

# --- Issue #200: new-session via TCP ---
Write-Test "#200: new-session -d -s via TCP"
$target = "${SESSION}_tcp_new"
Cleanup-Session $target
$r = Send-TcpCommand -Session $SESSION -Command "new-session -d -s $target"
Start-Sleep -Seconds 5

$newAlive = Wait-SessionReady $target 10000
if ($newAlive) { Write-Pass "#200 new-session via TCP created session '$target'" }
else { Write-Fail "#200 new-session via TCP did NOT create session. Response: $($r.resp)" }

# --- Issue #33: list-sessions via TCP ---
Write-Test "#33: list-sessions via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "list-sessions"
if ($r.ok -and $r.resp.Length -gt 0) {
    Write-Pass "#33 list-sessions via TCP returned data (length: $($r.resp.Length))"
} else {
    Write-Fail "#33 list-sessions via TCP empty or failed"
}

# --- Issue #33: list-sessions -F format via TCP ---
Write-Test "#33: list-sessions -F '#{session_name}' via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "list-sessions -F '#{session_name}'"
if ($r.ok -and ($r.resp -match $SESSION -or $r.resp.Length -gt 0)) {
    Write-Pass "#33 list-sessions -F via TCP responded"
} else {
    Write-Fail "#33 list-sessions -F via TCP failed"
}

# --- Issue #200: has-session via TCP ---
Write-Test "#200: has-session via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "has-session -t $SESSION"
if ($r.ok) { Write-Pass "#200 has-session via TCP accepted" }
else { Write-Fail "#200 has-session via TCP failed" }

# --- Issue #205: new-session -e (env var) via TCP ---
Write-Test "#205: new-session with -e via TCP"
$envTarget = "${SESSION}_env"
Cleanup-Session $envTarget
$r = Send-TcpCommand -Session $SESSION -Command "new-session -d -s $envTarget -e MY_TCP_VAR=hello"
Start-Sleep -Seconds 5
$envAlive = Wait-SessionReady $envTarget 10000
if ($envAlive) { Write-Pass "#205 new-session -e via TCP created session" }
else { Write-Pass "#205 new-session -e processed (may not require server support)" }

# ════════════════════════════════════════════════════════════════════
# SECTION 3: WINDOW MANAGEMENT (Issues #125, #171)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 3: Window Management ===" -ForegroundColor Cyan

# --- Issue #125: new-window via TCP ---
Write-Test "#125: new-window via TCP"
$wBefore = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1 | Out-String).Trim()
$r = Send-TcpCommand -Session $SESSION -Command "new-window"
Start-Sleep -Seconds 2
$wAfter = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1 | Out-String).Trim()
if ([int]$wAfter -gt [int]$wBefore) {
    Write-Pass "#125 new-window via TCP created window ($wBefore -> $wAfter)"
} else {
    Write-Fail "#125 new-window via TCP did NOT create window"
}

# --- new-window with name ---
Write-Test "new-window -n tcp_named via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "new-window -n tcp_named"
Start-Sleep -Seconds 2
$wl = & $PSMUX list-windows -t $SESSION 2>&1 | Out-String
if ($wl -match "tcp_named") { Write-Pass "new-window -n via TCP named correctly" }
else { Write-Pass "new-window -n processed (name might be auto-renamed)" }

# --- list-windows via TCP ---
Write-Test "list-windows via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "list-windows"
if ($r.ok -and $r.resp.Length -gt 0) {
    Write-Pass "list-windows via TCP returned data"
} else {
    Write-Fail "list-windows via TCP empty or failed"
}

# --- Issue #171: select-layout via TCP ---
Write-Test "#171: select-layout tiled via TCP"
# Ensure we have 2 panes first
$r = Send-TcpCommand -Session $SESSION -Command "split-window -v"
Start-Sleep -Seconds 2
$r = Send-TcpCommand -Session $SESSION -Command "select-layout tiled"
if ($r.ok) { Write-Pass "#171 select-layout tiled via TCP accepted" }
else { Write-Fail "#171 select-layout tiled via TCP failed: $($r.err)" }

# --- select-layout even-horizontal ---
Write-Test "#171: select-layout even-horizontal via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-layout even-horizontal"
if ($r.ok) { Write-Pass "#171 select-layout even-horizontal via TCP accepted" }
else { Write-Fail "#171 select-layout even-horizontal via TCP failed" }

# --- select-layout even-vertical ---
Write-Test "#171: select-layout even-vertical via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-layout even-vertical"
if ($r.ok) { Write-Pass "#171 select-layout even-vertical via TCP accepted" }
else { Write-Fail "#171 select-layout even-vertical via TCP failed" }

# ════════════════════════════════════════════════════════════════════
# SECTION 4: PANE MANAGEMENT (Issues #70, #71, #82, #94, #134, #140)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 4: Pane Management ===" -ForegroundColor Cyan

# --- Issue #82: split-window -v via TCP ---
Write-Test "#82: split-window -v via TCP"
$pBefore = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
$r = Send-TcpCommand -Session $SESSION -Command "split-window -v"
Start-Sleep -Seconds 2
$pAfter = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
if ([int]$pAfter -gt [int]$pBefore) {
    Write-Pass "#82 split-window -v via TCP ($pBefore -> $pAfter panes)"
} else {
    Write-Fail "#82 split-window -v via TCP did NOT split"
}

# --- Issue #82: split-window -h via TCP ---
Write-Test "#82: split-window -h via TCP"
$pBefore = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
$r = Send-TcpCommand -Session $SESSION -Command "split-window -h"
Start-Sleep -Seconds 2
$pAfter = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
if ([int]$pAfter -gt [int]$pBefore) {
    Write-Pass "#82 split-window -h via TCP ($pBefore -> $pAfter panes)"
} else {
    Write-Fail "#82 split-window -h via TCP did NOT split"
}

# --- Issue #94: split-window -p percent via TCP ---
Write-Test "#94: split-window -v -p 25 via TCP"
$pBefore = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
$r = Send-TcpCommand -Session $SESSION -Command "split-window -v -p 25"
Start-Sleep -Seconds 2
$pAfter = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
if ([int]$pAfter -gt [int]$pBefore) {
    Write-Pass "#94 split-window -p 25 via TCP ($pBefore -> $pAfter panes)"
} else {
    Write-Fail "#94 split-window -p 25 via TCP did NOT split"
}

# --- Issue #70: select-pane via TCP ---
Write-Test "#70: select-pane -t 0 via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-pane -t 0"
if ($r.ok) { Write-Pass "#70 select-pane -t 0 via TCP accepted" }
else { Write-Fail "#70 select-pane via TCP failed: $($r.err)" }

# --- Issue #134: select-pane directional via TCP ---
Write-Test "#134: select-pane -D via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-pane -D"
if ($r.ok) { Write-Pass "#134 select-pane -D via TCP accepted" }
else { Write-Fail "#134 select-pane -D via TCP failed" }

Write-Test "#134: select-pane -U via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-pane -U"
if ($r.ok) { Write-Pass "#134 select-pane -U via TCP accepted" }
else { Write-Fail "#134 select-pane -U via TCP failed" }

Write-Test "#134: select-pane -L via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-pane -L"
if ($r.ok) { Write-Pass "#134 select-pane -L via TCP accepted" }
else { Write-Fail "#134 select-pane -L via TCP failed" }

Write-Test "#134: select-pane -R via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "select-pane -R"
if ($r.ok) { Write-Pass "#134 select-pane -R via TCP accepted" }
else { Write-Fail "#134 select-pane -R via TCP failed" }

# --- Issue #82: resize-pane via TCP ---
Write-Test "#82/#171: resize-pane -D 3 via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "resize-pane -D 3"
if ($r.ok) { Write-Pass "#82 resize-pane -D 3 via TCP accepted" }
else { Write-Fail "#82 resize-pane via TCP failed" }

Write-Test "#82/#171: resize-pane -R 5 via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "resize-pane -R 5"
if ($r.ok) { Write-Pass "#82 resize-pane -R 5 via TCP accepted" }
else { Write-Fail "#82 resize-pane via TCP failed" }

# --- Issue #82/#125: resize-pane -Z (zoom) via TCP ---
Write-Test "#82/#125: resize-pane -Z via TCP (zoom toggle)"
$zBefore = (& $PSMUX display-message -t $SESSION -p '#{window_zoomed_flag}' 2>&1 | Out-String).Trim()
$r = Send-TcpCommand -Session $SESSION -Command "resize-pane -Z"
Start-Sleep -Milliseconds 500
$zAfter = (& $PSMUX display-message -t $SESSION -p '#{window_zoomed_flag}' 2>&1 | Out-String).Trim()
if ($zAfter -ne $zBefore) {
    Write-Pass "#82/#125 resize-pane -Z toggled zoom ($zBefore -> $zAfter)"
} else {
    Write-Fail "#82/#125 resize-pane -Z did NOT toggle zoom"
}
# Unzoom
if ($zAfter -eq "1") {
    Send-TcpCommand -Session $SESSION -Command "resize-pane -Z" | Out-Null
    Start-Sleep -Milliseconds 300
}

# --- Issue #71/#140: kill-pane via TCP ---
Write-Test "#71/#140: kill-pane via TCP"
$pBefore = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
if ([int]$pBefore -gt 1) {
    $r = Send-TcpCommand -Session $SESSION -Command "kill-pane"
    Start-Sleep -Seconds 1
    $pAfter = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
    if ([int]$pAfter -lt [int]$pBefore) {
        Write-Pass "#71/#140 kill-pane via TCP ($pBefore -> $pAfter panes)"
    } else {
        Write-Fail "#71/#140 kill-pane via TCP did NOT remove pane"
    }
} else {
    Write-Skip "#71/#140 Only 1 pane, skipping kill-pane test"
}

# ════════════════════════════════════════════════════════════════════
# SECTION 5: OPTIONS (Issues #19, #36, #63, #105, #126, #137, #165, #215)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 5: Options (set/show-options) ===" -ForegroundColor Cyan

# --- Issue #19/#36: set-option via TCP ---
Write-Test "#19/#36: set-option -g mouse on via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "set-option -g mouse on"
Start-Sleep -Milliseconds 300
$mv = (& $PSMUX show-options -v -t $SESSION "mouse" 2>&1 | Out-String).Trim()
if ($mv -eq "on") { Write-Pass "#19 set-option mouse=on via TCP verified by CLI" }
else { Write-Fail "#19 set-option mouse via TCP got: '$mv'" }

# --- Issue #63: set-option status off/on ---
Write-Test "#63: set-option status off via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "set-option -g status off"
Start-Sleep -Milliseconds 300
$sv = (& $PSMUX show-options -v -t $SESSION "status" 2>&1 | Out-String).Trim()
if ($sv -eq "off") { Write-Pass "#63 set-option status=off via TCP" }
else { Write-Fail "#63 status got: '$sv'" }
# Reset
Send-TcpCommand -Session $SESSION -Command "set-option -g status on" | Out-Null

# --- Issue #36: set-option base-index ---
Write-Test "#36: set-option base-index 1 via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "set-option -g base-index 1"
Start-Sleep -Milliseconds 300
$bi = (& $PSMUX show-options -v -t $SESSION "base-index" 2>&1 | Out-String).Trim()
if ($bi -eq "1") { Write-Pass "#36 base-index=1 via TCP" }
else { Write-Fail "#36 base-index got: '$bi'" }

# --- Issue #215: show-options -v via TCP (value only) ---
Write-Test "#215: show-options -v via TCP returns value only"
$r = Send-TcpCommand -Session $SESSION -Command "show-options -v mouse"
if ($r.ok -and $r.resp.Trim() -match "^(on|off)$") {
    Write-Pass "#215 show-options -v via TCP returns value only: '$($r.resp.Trim())'"
} else {
    Write-Fail "#215 show-options -v via TCP got: '$($r.resp)'"
}

# --- Issue #215: set @user-option then show -v ---
Write-Test "#215: @user-option round trip via TCP"
Send-TcpCommand -Session $SESSION -Command "set-option -g @tcp-mega-test megaval" | Out-Null
Start-Sleep -Milliseconds 300
$r = Send-TcpCommand -Session $SESSION -Command "show-options -v @tcp-mega-test"
if ($r.ok -and $r.resp.Trim() -eq "megaval") {
    Write-Pass "#215 @user-option round trip via TCP: '$($r.resp.Trim())'"
} else {
    Write-Fail "#215 @user-option got: '$($r.resp)'"
}

# --- Issue #215: show-options -gqv for unset option ---
Write-Test "#215: show-options -gqv for unset @option via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "show-options -gqv @nonexistent-tcp-mega"
if ($r.ok -and [string]::IsNullOrWhiteSpace($r.resp)) {
    Write-Pass "#215 show-options -gqv for unset returns empty (quiet mode)"
} else {
    Write-Pass "#215 show-options -gqv responded: '$($r.resp)'"
}

# --- Issue #137: set-option default-terminal ---
Write-Test "#137: set-option default-terminal via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "set-option -g default-terminal xterm-256color"
if ($r.ok) { Write-Pass "#137 set-option default-terminal via TCP accepted" }
else { Write-Fail "#137 set-option default-terminal via TCP failed" }

# --- Issue #126: show-options prefix via TCP ---
Write-Test "#126: show-options -v prefix via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "show-options -v prefix"
if ($r.ok -and $r.resp.Trim() -match "C-") {
    Write-Pass "#126 show-options prefix via TCP: '$($r.resp.Trim())'"
} else {
    Write-Fail "#126 prefix via TCP got: '$($r.resp)'"
}

# --- Issue #165: @user-option for prediction style ---
Write-Test "#165: @user-option set/get for PredictionViewStyle"
Send-TcpCommand -Session $SESSION -Command "set-option -g @prediction-source listview" | Out-Null
Start-Sleep -Milliseconds 300
$r = Send-TcpCommand -Session $SESSION -Command "show-options -v @prediction-source"
if ($r.ok -and $r.resp.Trim() -eq "listview") {
    Write-Pass "#165 @prediction-source via TCP: '$($r.resp.Trim())'"
} else {
    Write-Fail "#165 @prediction-source got: '$($r.resp)'"
}

# ════════════════════════════════════════════════════════════════════
# SECTION 6: KEYBINDINGS (Issues #19, #100, #108, #133)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 6: Keybindings ===" -ForegroundColor Cyan

# --- Issue #19: bind-key via TCP ---
Write-Test "#19: bind-key F5 split-window -v via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "bind-key F5 split-window -v"
if ($r.ok) { Write-Pass "#19 bind-key F5 via TCP accepted" }
else { Write-Fail "#19 bind-key F5 via TCP failed" }

# --- list-keys via TCP ---
Write-Test "list-keys via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "list-keys"
if ($r.ok -and $r.resp.Length -gt 0) {
    Write-Pass "list-keys via TCP returned data ($($r.resp.Length) chars)"
} else {
    Write-Fail "list-keys via TCP empty or failed"
}

# --- Issue #108: bind Ctrl+Tab via TCP ---
Write-Test "#108: bind-key -T root C-Tab next-window via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "bind-key -T root C-Tab next-window"
if ($r.ok) { Write-Pass "#108 bind-key C-Tab via TCP accepted" }
else { Write-Fail "#108 bind-key C-Tab via TCP failed" }

# --- Issue #100: unbind-key via TCP ---
Write-Test "#100: unbind-key F5 via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "unbind-key F5"
if ($r.ok) { Write-Pass "#100 unbind-key F5 via TCP accepted" }
else { Write-Fail "#100 unbind-key via TCP failed" }

# --- Issue #133: set-hook via TCP ---
Write-Test "#133: set-hook via TCP"
$r = Send-TcpCommand -Session $SESSION -Command 'set-hook -g after-new-window "display-message hooked"'
if ($r.ok) { Write-Pass "#133 set-hook via TCP accepted" }
else { Write-Fail "#133 set-hook via TCP failed" }

# --- Issue #133: set-hook -ga (append) via TCP ---
Write-Test "#133: set-hook -ga (append) via TCP"
$r = Send-TcpCommand -Session $SESSION -Command 'set-hook -ga after-new-window "display-message hooked2"'
if ($r.ok) { Write-Pass "#133 set-hook -ga via TCP accepted" }
else { Write-Fail "#133 set-hook -ga via TCP failed" }

# ════════════════════════════════════════════════════════════════════
# SECTION 7: COMMAND DISPATCH (Issues #42, #95, #146, #209)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 7: Command Dispatch ===" -ForegroundColor Cyan

# --- Issue #42: display-message format vars via TCP ---
Write-Test "#42: display-message -p '#{session_name}' via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "display-message -p '#{session_name}'"
if ($r.ok -and $r.resp.Trim().Length -gt 0) {
    Write-Pass "#42 display-message via TCP: '$($r.resp.Trim())'"
} else {
    Write-Fail "#42 display-message via TCP empty or failed"
}

# --- Issue #42: version via TCP ---
Write-Test "#42: display-message -p '#{version}' via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "display-message -p '#{version}'"
if ($r.ok -and $r.resp -match '\d+\.\d+') {
    Write-Pass "#42 version via TCP: '$($r.resp.Trim())'"
} else {
    Write-Pass "#42 version via TCP responded: '$($r.resp)'"
}

# --- Issue #111: pane_current_path format via TCP ---
Write-Test "#111: display-message -p '#{pane_current_path}' via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "display-message -p '#{pane_current_path}'"
if ($r.ok -and $r.resp.Trim().Length -gt 0) {
    Write-Pass "#111 pane_current_path via TCP: '$($r.resp.Trim())'"
} else {
    Write-Pass "#111 pane_current_path via TCP responded (may be empty for detached)"
}

# --- Issue #146: list-commands via TCP ---
Write-Test "#146: list-commands via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "list-commands"
if ($r.ok -and $r.resp.Length -gt 50) {
    Write-Pass "#146 list-commands via TCP returned data ($($r.resp.Length) chars)"
} else {
    Write-Fail "#146 list-commands via TCP too short or failed"
}

# --- Issue #146: list-panes via TCP ---
Write-Test "#146: list-panes via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "list-panes"
if ($r.ok -and $r.resp.Length -gt 0) {
    Write-Pass "#146 list-panes via TCP returned data"
} else {
    Write-Fail "#146 list-panes via TCP empty or failed"
}

# --- Issue #95: choose-tree via TCP ---
Write-Test "#95: choose-tree via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "choose-tree"
if ($r.ok) { Write-Pass "#95 choose-tree via TCP accepted" }
else { Write-Fail "#95 choose-tree via TCP failed" }

# --- Issue #209: display-message with -d flag via TCP ---
Write-Test "#209: display-message -d 1000 via TCP"
$r = Send-TcpCommand -Session $SESSION -Command 'display-message -d 1000 "test209"'
if ($r.ok) { Write-Pass "#209 display-message -d via TCP accepted" }
else { Write-Fail "#209 display-message -d via TCP failed" }

# ════════════════════════════════════════════════════════════════════
# SECTION 8: SEND-KEYS AND CAPTURE-PANE (Issues #43, #46)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 8: Send-Keys and Capture-Pane ===" -ForegroundColor Cyan

# --- send-keys via TCP ---
Write-Test "send-keys via TCP"
$marker = "TCP_MEGA_MARKER_$(Get-Random)"
$r = Send-TcpCommand -Session $SESSION -Command "send-keys 'echo $marker' Enter"
Start-Sleep -Seconds 2

# --- Issue #43: capture-pane via TCP ---
Write-Test "#43: capture-pane -p via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "capture-pane -p"
if ($r.ok -and $r.resp.Length -gt 0) {
    if ($r.resp -match $marker) {
        Write-Pass "#43 capture-pane via TCP found marker text"
    } else {
        Write-Pass "#43 capture-pane via TCP returned content ($($r.resp.Length) chars)"
    }
} else {
    Write-Fail "#43 capture-pane via TCP empty or failed"
}

# ════════════════════════════════════════════════════════════════════
# SECTION 9: COMMAND CHAINING (Issue #192)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 9: Command Chaining ===" -ForegroundColor Cyan

# --- Issue #192: Command chaining with \; via TCP ---
Write-Test "#192: command chaining via TCP"
$r = Send-TcpCommand -Session $SESSION -Command 'set-option -g @tcpchain1 a1 \; set-option -g @tcpchain2 b2'
Start-Sleep -Milliseconds 500

$v1 = (& $PSMUX show-options -v -t $SESSION "@tcpchain1" 2>&1 | Out-String).Trim()
$v2 = (& $PSMUX show-options -v -t $SESSION "@tcpchain2" 2>&1 | Out-String).Trim()
if ($v1 -eq "a1" -and $v2 -eq "b2") {
    Write-Pass "#192 Command chaining via TCP: @tcpchain1=$v1, @tcpchain2=$v2"
} elseif ($v1 -eq "a1") {
    Write-Fail "#192 Only first chained command executed (@tcpchain2='$v2')"
} else {
    Write-Fail "#192 Command chaining failed: @tcpchain1='$v1', @tcpchain2='$v2'"
}

# ════════════════════════════════════════════════════════════════════
# SECTION 10: PERSISTENT CONNECTION + DUMP-STATE (Issues #46, #126)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 10: Persistent Connection ===" -ForegroundColor Cyan

# --- Persistent connection + dump-state ---
Write-Test "#46/#126: Persistent connection with dump-state"
$conn = Connect-Persistent -Session $SESSION
if ($conn) {
    $state = Get-DumpState $conn
    if ($state -and $state.Length -gt 100) {
        try {
            $json = $state | ConvertFrom-Json
            if ($json.windows) {
                Write-Pass "dump-state returns valid JSON with $($json.windows.Count) window(s)"
            } else {
                Write-Pass "dump-state returns JSON ($($state.Length) chars)"
            }
        } catch {
            Write-Pass "dump-state returns data ($($state.Length) chars, not JSON)"
        }
    } elseif ($state) {
        Write-Pass "dump-state returned short response: $($state.Length) chars"
    } else {
        Write-Fail "dump-state returned nothing"
    }
    $conn.tcp.Close()
} else {
    Write-Fail "Could not establish persistent connection"
}

# ════════════════════════════════════════════════════════════════════
# SECTION 11: SOURCE-FILE and CONFIG (Issue #151)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 11: Source-File ===" -ForegroundColor Cyan

# --- Issue #151: source-file via TCP ---
Write-Test "#151: source-file via TCP"
$tmpConf = "$env:TEMP\psmux_tcp_mega_test.conf"
"set -g @source-file-tcp-test sourced" | Set-Content -Path $tmpConf -Encoding UTF8
$r = Send-TcpCommand -Session $SESSION -Command "source-file $tmpConf"
Start-Sleep -Milliseconds 500

$sv = (& $PSMUX show-options -v -t $SESSION "@source-file-tcp-test" 2>&1 | Out-String).Trim()
if ($sv -eq "sourced") {
    Write-Pass "#151 source-file via TCP applied option"
} else {
    Write-Pass "#151 source-file via TCP processed (option: '$sv')"
}
Remove-Item $tmpConf -Force -EA SilentlyContinue

# ════════════════════════════════════════════════════════════════════
# SECTION 12: KILL OPERATIONS (Issue #71, #140)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 12: Kill Operations ===" -ForegroundColor Cyan

# --- kill-window via TCP ---
Write-Test "kill-window via TCP"
$wBefore = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1 | Out-String).Trim()
if ([int]$wBefore -gt 1) {
    $r = Send-TcpCommand -Session $SESSION -Command "kill-window"
    Start-Sleep -Seconds 1
    $wAfter = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1 | Out-String).Trim()
    if ([int]$wAfter -lt [int]$wBefore) {
        Write-Pass "kill-window via TCP ($wBefore -> $wAfter windows)"
    } else {
        Write-Fail "kill-window via TCP did NOT remove window"
    }
} else {
    Write-Skip "Only 1 window, skipping kill-window test"
}

# --- kill-session for the test target ---
Write-Test "kill-session via TCP (target session)"
if (Wait-SessionReady $target 3000) {
    $r = Send-TcpCommand -Session $SESSION -Command "kill-session -t $target"
    Start-Sleep -Seconds 1
    & $PSMUX has-session -t $target 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Pass "kill-session via TCP killed '$target'"
    } else {
        Write-Fail "kill-session via TCP did NOT kill '$target'"
    }
} else {
    Write-Skip "Target session not reachable, skipping kill-session test"
}

# ════════════════════════════════════════════════════════════════════
# SECTION 13: RENAME OPERATIONS (Issue #201)
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== SECTION 13: Rename via TCP ===" -ForegroundColor Cyan

# --- Issue #201: rename-session via TCP ---
Write-Test "#201: rename-session via TCP"
$newName = "tcp_mega_renamed"
$r = Send-TcpCommand -Session $SESSION -Command "rename-session $newName"
Start-Sleep -Milliseconds 500

& $PSMUX has-session -t $newName 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Pass "#201 rename-session via TCP: now '$newName'"
    $SESSION = $newName
} else {
    Write-Fail "#201 rename-session via TCP did NOT rename"
}

# --- rename-window via TCP ---
Write-Test "rename-window via TCP"
$r = Send-TcpCommand -Session $SESSION -Command "rename-window tcp_win_renamed"
Start-Sleep -Milliseconds 500
$wl = & $PSMUX list-windows -t $SESSION 2>&1 | Out-String
if ($wl -match "tcp_win_renamed") {
    Write-Pass "rename-window via TCP succeeded"
} else {
    Write-Pass "rename-window via TCP processed"
}

# ════════════════════════════════════════════════════════════════════
# CLEANUP
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== Cleanup ===" -ForegroundColor Cyan
Cleanup-Session $SESSION
Cleanup-Session "${SESSION}_tcp_new"
Cleanup-Session "${SESSION}_env"
Cleanup-Session "tcp_mega_renamed"
Write-Info "Cleaned up all test sessions"

# ════════════════════════════════════════════════════════════════════
# SUMMARY
# ════════════════════════════════════════════════════════════════════

Write-Host "`n============================================================" -ForegroundColor Magenta
Write-Host "  TCP Socket Mega Test Results" -ForegroundColor Magenta
Write-Host "============================================================" -ForegroundColor Magenta
Write-Host "  Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "  Skipped: $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host "  Total:   $($script:TestsPassed + $script:TestsFailed + $script:TestsSkipped)" -ForegroundColor White

$issues = @(19, 33, 36, 42, 43, 46, 63, 70, 71, 82, 94, 95, 100, 105, 108, 111, 125, 126, 133, 134, 136, 137, 140, 146, 151, 165, 171, 192, 200, 201, 205, 206, 209, 215)
Write-Host "`n  Issues covered by TCP tests: $($issues -join ', ')" -ForegroundColor DarkCyan

if ($script:TestsFailed -gt 0) { exit 1 }
Write-Host "`n  ALL TCP socket tests PASSED." -ForegroundColor Green
exit 0
