# psmux GitHub Issues Reproduction Script
# Tests bugs reported in issues #25 and #19
#
# Bug 1 (Issue #25): Active window tab color not updating after select-window
# Bug 2 (Issue #19): bind-key from command prompt not working (flag stripping)
# Bug 3 (Issue #19): Status bar colors not configurable (hardcoded yellow)
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_github_issues.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

# Kill everything first
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "issuetest"

function New-TestSession {
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
    & $PSMUX has-session -t $SESSION 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
}

function Connect-TCP {
    $portFile = "$env:USERPROFILE\.psmux\$SESSION.port"
    $keyFile  = "$env:USERPROFILE\.psmux\$SESSION.key"
    $port = [int](Get-Content $portFile).Trim()
    $key  = (Get-Content $keyFile).Trim()

    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", $port)
    $tcp.NoDelay = $true
    $tcp.ReceiveTimeout = 10000
    $stream = $tcp.GetStream()
    $enc = [System.Text.UTF8Encoding]::new($false)
    $reader = [System.IO.StreamReader]::new($stream, $enc, $false, 131072)
    $writer = [System.IO.StreamWriter]::new($stream, $enc, 4096)
    $writer.NewLine = "`n"
    $writer.AutoFlush = $false

    $writer.WriteLine("AUTH $key"); $writer.Flush()
    $auth = $reader.ReadLine()
    if ($auth -ne "OK") { Write-Host "AUTH FAILED"; $tcp.Close(); exit 1 }

    $writer.WriteLine("PERSISTENT"); $writer.Flush()

    return @{ tcp = $tcp; reader = $reader; writer = $writer }
}

function Send-Fire($conn, $cmd) {
    $conn.writer.WriteLine($cmd)
    $conn.writer.Flush()
}

function Get-Dump($conn) {
    $conn.writer.WriteLine("dump-state")
    $conn.writer.Flush()
    return $conn.reader.ReadLine()
}

function Get-FreshDump($conn) {
    # The server pushes frames asynchronously to PERSISTENT connections
    # (event-driven rendering).  After Send-Fire commands, stale pushed
    # frames may sit in the TCP read buffer ahead of our dump-state
    # response.  The real psmux client drains all frames keeping only
    # the latest (client.rs line 582-610); we must do the same.
    #
    # Strategy: send dump-state, then drain ALL available lines (stale
    # push frames + response), keeping only the last valid frame.
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        $conn.writer.WriteLine("dump-state")
        $conn.writer.Flush()
        $best = $null
        $oldTimeout = $conn.tcp.ReceiveTimeout
        # Use a generous timeout for the first read (server needs time
        # to process), then switch to a short timeout to drain remaining
        # queued frames without blocking.
        $conn.tcp.ReceiveTimeout = 2000
        for ($j = 0; $j -lt 200; $j++) {
            try {
                $line = $conn.reader.ReadLine()
            } catch {
                break  # timeout - no more data
            }
            if ($null -eq $line) { break }
            if ($line -ne "NC" -and $line.Length -gt 100) {
                $best = $line
            }
            # After first valid frame, drain remaining quickly
            if ($best) { $conn.tcp.ReceiveTimeout = 50 }
        }
        $conn.tcp.ReceiveTimeout = $oldTimeout
        if ($best) { return $best }
        Start-Sleep -Milliseconds 100
    }
    return $null
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GITHUB ISSUE REPRODUCTION TESTS"
Write-Host ("=" * 70)

# ============================================================
# BUG 1: Issue #25 - Active window tab not updating after select-window
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "BUG 1: Active window tab not updating (Issue #25)"
Write-Host ("=" * 70)

New-TestSession
$conn = Connect-TCP

# Query base-index so we use correct window numbers regardless of config
$baseIdx = & $PSMUX show-options -t $SESSION -v base-index 2>&1
$baseIdx = [int]($baseIdx.ToString().Trim())
if ($baseIdx -lt 0 -or $baseIdx -gt 100) { $baseIdx = 0 }
Write-Info "base-index=$baseIdx (windows will be $baseIdx, $($baseIdx+1), $($baseIdx+2))"

# Create 3 windows
Send-Fire $conn "new-window"
Start-Sleep -Seconds 2
Send-Fire $conn "new-window"
Start-Sleep -Seconds 2

# Get initial state - should show last window as active (array[2])
$state1 = Get-FreshDump $conn
$json1 = $state1 | ConvertFrom-Json -ErrorAction SilentlyContinue
$activeWindows1 = @($json1.windows | Where-Object { $_.active -eq $true })

Write-Test "Three windows created, third is active"
if ($activeWindows1.Count -eq 1) {
    Write-Pass "Exactly 1 active window before switch"
} else {
    Write-Fail "Expected 1 active window, got $($activeWindows1.Count)"
}

# Switch to first window via select-window (base-index aware)
$firstWin = $baseIdx
Write-Test "select-window $firstWin updates active flag in dump-state"
Send-Fire $conn "select-window $firstWin"
Start-Sleep -Milliseconds 500

# Get new state
$state2 = Get-FreshDump $conn
$json2 = $state2 | ConvertFrom-Json -ErrorAction SilentlyContinue
$activeWindows2 = @($json2.windows | Where-Object { $_.active -eq $true })

if ($activeWindows2.Count -eq 1) {
    $activeIdx = 0
    for ($i = 0; $i -lt $json2.windows.Count; $i++) {
        if ($json2.windows[$i].active) { $activeIdx = $i }
    }
    if ($activeIdx -eq 0) {
        Write-Pass "First window (array[0]) is now active after select-window $firstWin"
    } else {
        Write-Fail "Active is array[$activeIdx], expected array[0] after select-window $firstWin"
    }
} else {
    Write-Fail "Expected 1 active window after select-window, got $($activeWindows2.Count)"
}

# Switch to second window (array index 1)
$secondWin = $baseIdx + 1
Write-Test "select-window $secondWin updates active flag"
Send-Fire $conn "select-window $secondWin"
Start-Sleep -Milliseconds 500

$state3 = Get-FreshDump $conn
$json3 = $state3 | ConvertFrom-Json -ErrorAction SilentlyContinue
$activeIdx3 = -1
for ($i = 0; $i -lt $json3.windows.Count; $i++) {
    if ($json3.windows[$i].active) { $activeIdx3 = $i }
}
if ($activeIdx3 -eq 1) {
    Write-Pass "Second window (array[1]) is active after select-window $secondWin"
} else {
    Write-Fail "Active is array[$activeIdx3], expected array[1] after select-window $secondWin"
}

try { $conn.tcp.Close() } catch {}
& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep 2

# ============================================================
# BUG 2: Issue #19 - bind-key parsing strips command flags
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "BUG 2: bind-key flag stripping bug (Issue #19)"
Write-Host ("=" * 70)

New-TestSession
$conn = Connect-TCP

# Test: bind-key with command that has flags
Write-Test "bind-key r split-window -h preserves -h flag"
Send-Fire $conn "bind-key r split-window -h"
Start-Sleep -Milliseconds 500

# Use CLI list-keys (reads all output correctly) instead of raw TCP ReadLine
$keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String

if ("$keys" -match "split-window -h" -or "$keys" -match "split-window.*-h") {
    Write-Pass "bind-key r: command includes -h flag"
} else {
    Write-Fail "bind-key r: -h flag was stripped! Got: $keys"
}

# Test: bind-key with dash as key
Write-Test "bind-key - split-window -v (dash as key)"
Send-Fire $conn "bind-key - split-window -v"
Start-Sleep -Milliseconds 500

$keys2 = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String

if ("$keys2" -match 'split-window.*-v') {
    Write-Pass "bind-key -: dash key is recognized"
} else {
    Write-Fail "bind-key -: dash key was treated as a flag and dropped! Got: $keys2"
}

# Test: bind-key with -T and command flags
Write-Test "bind-key -T prefix v split-window -v"
Send-Fire $conn "bind-key -T prefix v split-window -v"
Start-Sleep -Milliseconds 500

$keys3 = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String

if ("$keys3" -match "split-window -v" -or "$keys3" -match "split-window.*-v") {
    Write-Pass "bind-key with -T: command -v flag preserved"
} else {
    Write-Fail "bind-key with -T: -v flag was stripped! Got: $keys3"
}

try { $conn.tcp.Close() } catch {}
& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep 2

# ============================================================
# BUG 3: Issue #19 - Status bar colors stuck on yellow/green
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "BUG 3: Status bar colors not configurable (Issue #19)"
Write-Host ("=" * 70)

New-TestSession
$conn = Connect-TCP

# Get initial dump-state to check what style fields are available
$state = Get-FreshDump $conn
$json = $state | ConvertFrom-Json -ErrorAction SilentlyContinue

Write-Test "dump-state includes window-status-current-style"
$hasWscStyle = $json.PSObject.Properties.Name -contains "wsc_style"
$hasWsStyle = $json.PSObject.Properties.Name -contains "ws_style"
$hasWscstyle2 = $json.PSObject.Properties.Name -contains "window_status_current_style"
if ($hasWscStyle -or $hasWscstyle2) {
    Write-Pass "dump-state has window-status-current-style field"
} else {
    Write-Fail "dump-state MISSING window-status-current-style field (client can't style tabs!)"
    Write-Info "Available fields: $($json.PSObject.Properties.Name -join ', ')"
}

Write-Test "dump-state includes window-status-style"
if ($hasWsStyle -or ($json.PSObject.Properties.Name -contains "window_status_style")) {
    Write-Pass "dump-state has window-status-style field"
} else {
    Write-Fail "dump-state MISSING window-status-style field"
}

# Test setting status-style and checking it appears
Write-Test "set status-style is reflected in dump-state"
Send-Fire $conn 'set status-style "bg=colour235 fg=colour136"'
Start-Sleep -Milliseconds 500
$state2 = Get-FreshDump $conn
$json2 = $state2 | ConvertFrom-Json -ErrorAction SilentlyContinue
if ($json2.status_style -match "colour235" -or $json2.status_style -match "235") {
    Write-Pass "status-style updated in dump-state: $($json2.status_style)"
} else {
    Write-Fail "status-style NOT updated in dump-state. Got: $($json2.status_style)"
}

try { $conn.tcp.Close() } catch {}
& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep 2

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Final cleanup..."
& $PSMUX kill-server 2>$null
Start-Sleep 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "GITHUB ISSUES TEST SUMMARY" -ForegroundColor White
Write-Host ("=" * 70)
Write-Host "Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "Total:  $($script:TestsPassed + $script:TestsFailed)"
Write-Host ""
Write-Host "Bugs identified:" -ForegroundColor Yellow
Write-Host "  1. SelectWindow handler missing meta_dirty=true -> stale tab colors"
Write-Host "  2. bind-key TCP parser strips ALL '-' prefixed args including command flags"
Write-Host "  3. window-status-current-style not sent in dump-state -> client hardcodes yellow"
Write-Host ("=" * 70)

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
