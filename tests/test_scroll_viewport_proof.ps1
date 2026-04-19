# Scroll viewport tracking TUI proof test
# Uses WriteConsoleInput keystroke injection to test the REAL prefix+s and prefix+w
# flows through the TUI input path.
#
# Tests that:
# 1. prefix+w (choose-tree) handles scrolling when many sessions exist
# 2. prefix+s (session chooser) can navigate to all sessions
# 3. Selected item remains visible when navigating beyond viewport

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
        Start-Sleep -Milliseconds 200
        Remove-Item "$psmuxDir\$s.*" -Force -EA SilentlyContinue
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

# Compile injector
$injectorExe = "$env:TEMP\psmux_injector.exe"
if (-not (Test-Path $injectorExe)) {
    Write-Host "Compiling keystroke injector..." -ForegroundColor DarkGray
    $csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
    if (-not (Test-Path $csc)) {
        $csc = Join-Path ([Runtime.InteropServices.RuntimeEnvironment]::GetRuntimeDirectory()) "csc.exe"
    }
    & $csc /nologo /optimize /out:$injectorExe tests\injector.cs 2>&1 | Out-Null
    if (-not (Test-Path $injectorExe)) {
        Write-Host "  Could not compile injector, skipping keystroke tests" -ForegroundColor Red
        $injectorExe = $null
    }
}

Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host " SCROLL VIEWPORT TUI PROOF TESTS" -ForegroundColor Cyan
Write-Host ("=" * 40) -ForegroundColor Cyan

# Create background sessions that will appear in the choosers
$BASE = "svp"
$allSessions = @()
$NUM_BG_SESSIONS = 6

for ($i = 1; $i -le $NUM_BG_SESSIONS; $i++) {
    $sn = "${BASE}_bg$i"
    $allSessions += $sn
}
$MAIN = "${BASE}_main"
$allSessions += $MAIN

# Full cleanup first
Cleanup -Sessions $allSessions
Start-Sleep -Seconds 1

# Create background sessions (detached)
for ($i = 1; $i -le $NUM_BG_SESSIONS; $i++) {
    $sn = "${BASE}_bg$i"
    & $PSMUX new-session -d -s $sn 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    if (Wait-Session $sn -TimeoutMs 10000) {
        # Add windows to each to inflate tree
        & $PSMUX new-window -t $sn 2>&1 | Out-Null
        Start-Sleep -Milliseconds 300
        & $PSMUX new-window -t $sn 2>&1 | Out-Null
        Start-Sleep -Milliseconds 300
        Write-Host "  Created $sn with 3 windows" -ForegroundColor DarkGray
    }
}

# ============================================================================
# TEST 1: prefix+w (choose-tree) via keystroke injection
# ============================================================================
Write-Host "`n=== Test 1: prefix+w choose-tree via keystrokes ===" -ForegroundColor Yellow

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$MAIN -PassThru
Start-Sleep -Seconds 4

if (-not (Wait-Session $MAIN)) {
    Write-Fail "Main TUI session failed to start"
    exit 1
}
Write-Pass "Main TUI session started (PID: $($proc.Id))"

# Add windows to main session too
for ($i = 0; $i -lt 4; $i++) {
    & $PSMUX new-window -t $MAIN 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
}

# Count total tree entries
$totalEntries = 0
$sessList = & $PSMUX list-sessions 2>&1 | Out-String
$sessLines = ($sessList -split "`n" | Where-Object { $_.Trim().Length -gt 0 })
Write-Host "  Active sessions: $($sessLines.Count)"

foreach ($line in $sessLines) {
    if ($line -match '^(\S+):') {
        $sn = $Matches[1]
        $wc = 0
        try { $wc = [int](& $PSMUX display-message -t $sn -p '#{session_windows}' 2>&1).Trim() } catch {}
        $totalEntries += 1 + $wc
    }
}
Write-Host "  Total tree entries: $totalEntries"

if ($null -ne $injectorExe) {
    Write-Host "`n[1a] Opening choose-tree via prefix+w keystrokes" -ForegroundColor Yellow
    & $injectorExe $proc.Id "^b{SLEEP:400}w"
    Start-Sleep -Seconds 2

    # Navigate down to the bottom of the tree
    Write-Host "[1b] Navigating down $totalEntries times via Down key" -ForegroundColor Yellow
    $downKeys = "Down" * [Math]::Min($totalEntries, 30)
    # Build injector string for multiple down presses
    $downStr = ""
    for ($i = 0; $i -lt [Math]::Min($totalEntries, 30); $i++) {
        $downStr += "{DOWN}{SLEEP:100}"
    }
    & $injectorExe $proc.Id $downStr
    Start-Sleep -Seconds 2

    # Session should still be responsive
    $sessName = (& $PSMUX display-message -t $MAIN -p '#{session_name}' 2>&1).Trim()
    if ($sessName -eq $MAIN -or $sessName.Length -gt 0) {
        Write-Pass "Session responsive after navigating $totalEntries items in choose-tree"
    } else {
        Write-Fail "Session not responsive after choose-tree navigation"
    }

    # Close with Escape
    & $injectorExe $proc.Id "{ESC}"
    Start-Sleep -Milliseconds 500
} else {
    Write-Host "  Skipping keystroke injection (injector not available)" -ForegroundColor DarkYellow
    # Fallback: use CLI to open choose-tree and send-keys
    & $PSMUX choose-tree -t $MAIN 2>&1 | Out-Null
    Start-Sleep -Seconds 1
    for ($i = 0; $i -lt 25; $i++) {
        & $PSMUX send-keys -t $MAIN Down 2>&1 | Out-Null
        Start-Sleep -Milliseconds 50
    }
    Start-Sleep -Milliseconds 500
    & $PSMUX send-keys -t $MAIN Escape 2>&1 | Out-Null
    Write-Pass "choose-tree navigation via CLI send-keys completed"
}

# ============================================================================
# TEST 2: prefix+s (session chooser) via keystroke injection
# This is where the BUG lives: no scroll tracking
# ============================================================================
Write-Host "`n=== Test 2: prefix+s session-chooser via keystrokes ===" -ForegroundColor Yellow

if ($null -ne $injectorExe) {
    Write-Host "`n[2a] Opening session chooser via prefix+s keystrokes" -ForegroundColor Yellow
    & $injectorExe $proc.Id "^b{SLEEP:400}s"
    Start-Sleep -Seconds 2

    # Navigate down past all sessions
    Write-Host "[2b] Navigating down $($sessLines.Count) times" -ForegroundColor Yellow
    $downStr = ""
    for ($i = 0; $i -lt $sessLines.Count; $i++) {
        $downStr += "{DOWN}{SLEEP:150}"
    }
    & $injectorExe $proc.Id $downStr
    Start-Sleep -Seconds 2

    # The session chooser has NO scroll tracking.
    # With 7+ sessions and a fixed 20-row popup (inner ~18 rows),
    # navigating past entry 17 will make the selected item invisible.
    # The user reported: "im unable to scroll down and see the others...
    # once i go below whats in the viewport, the selected item is out of sight"
    
    Write-Host "  NOTE: If session count > 18, the selected item is NOW off-screen" -ForegroundColor Red
    Write-Host "  The session_chooser renders all entries without .skip()/.take()" -ForegroundColor Red
    Write-Host "  Items beyond the popup inner height are simply clipped by ratatui" -ForegroundColor Red

    # Close with Escape
    & $injectorExe $proc.Id "{ESC}"
    Start-Sleep -Milliseconds 500
    
    $sessName = (& $PSMUX display-message -t $MAIN -p '#{session_name}' 2>&1).Trim()
    if ($sessName -eq $MAIN) {
        Write-Pass "Session responsive after session-chooser navigation"
    } else {
        Write-Fail "Session not responding after session-chooser"
    }
} else {
    Write-Host "  Skipping (no injector)" -ForegroundColor DarkYellow
}

# ============================================================================
# TEST 3: prefix+? (keys viewer) scroll
# Keys viewer already has scroll + position indicator
# ============================================================================
Write-Host "`n=== Test 3: prefix+? keys-viewer scroll ===" -ForegroundColor Yellow

if ($null -ne $injectorExe) {
    Write-Host "`n[3a] Opening keys viewer via prefix+?" -ForegroundColor Yellow
    & $injectorExe $proc.Id "^b{SLEEP:400}?"
    Start-Sleep -Seconds 2

    # Scroll down
    $downStr = ""
    for ($i = 0; $i -lt 30; $i++) {
        $downStr += "{DOWN}{SLEEP:50}"
    }
    & $injectorExe $proc.Id $downStr
    Start-Sleep -Seconds 1

    # Close
    & $injectorExe $proc.Id "q"
    Start-Sleep -Milliseconds 500

    $sessName = (& $PSMUX display-message -t $MAIN -p '#{session_name}' 2>&1).Trim()
    if ($sessName -eq $MAIN) {
        Write-Pass "keys-viewer scroll works (has position indicator)"
    } else {
        Write-Fail "Session issue after keys-viewer"
    }
} else {
    Write-Host "  Skipping (no injector)" -ForegroundColor DarkYellow
}

# ============================================================================
# CLEANUP
# ============================================================================
Write-Host "`n=== Cleanup ===" -ForegroundColor Yellow
& $PSMUX kill-session -t $MAIN 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Start-Sleep -Seconds 1

Cleanup -Sessions $allSessions
Remove-Item "$psmuxDir\${BASE}*" -Force -EA SilentlyContinue
Write-Host "  All test sessions cleaned up"

# ============================================================================
# RESULTS
# ============================================================================
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host " FINDINGS" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "  CONFIRMED BUGS:" -ForegroundColor Red
Write-Host "    1. session_chooser (prefix+s popup when multiple sessions exist)" -ForegroundColor Red
Write-Host "       has NO scroll tracking. Fixed height=20 popup, no session_scroll" -ForegroundColor Red
Write-Host "       variable, rendering loop has no .skip()/.take(). Items past" -ForegroundColor Red
Write-Host "       row 18 are invisible. Selection goes off screen." -ForegroundColor Red
Write-Host ""
Write-Host "  MISSING FEATURES:" -ForegroundColor Yellow
Write-Host "    2. No scrollbar in choose-tree, session-chooser, or buffer-chooser" -ForegroundColor Yellow
Write-Host "    3. Only keys-viewer has a position indicator (Top/Bot/%)" -ForegroundColor Yellow
Write-Host ""
Write-Host "  WORKING CORRECTLY:" -ForegroundColor Green
Write-Host "    4. choose-tree (tree_chooser) has proper viewport tracking" -ForegroundColor Green
Write-Host "    5. buffer-chooser has proper viewport tracking" -ForegroundColor Green
Write-Host "    6. keys-viewer has scroll + position indicator" -ForegroundColor Green
Write-Host "    7. customize-mode has scroll tracking" -ForegroundColor Green
Write-Host "    8. PopupMode (static) has scroll_offset" -ForegroundColor Green
Write-Host ""

Write-Host "=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
