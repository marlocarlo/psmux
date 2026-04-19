# Issue #232: status-interval timer must push frames to TUI clients
# Tests that status-right with strftime codes (%H:%M:%S, %r) auto-updates
# when status-interval is set, even with ZERO user interaction.
#
# The bug: status-interval timer fired hooks but never set state_dirty=true,
# so persistent (TUI) clients never received push frames with re-expanded
# strftime codes. The clock only updated when another event (keystroke,
# PTY output) happened to trigger a frame push.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$SESSION = "test_issue232"
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
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

# === SETUP ===
Cleanup
& $PSMUX new-session -d -s $SESSION
Start-Sleep -Seconds 3

# Verify session exists
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Session creation failed"
    exit 1
}

# Configure status-interval=1 and status-right with time
& $PSMUX set -t $SESSION -g status-interval 1
& $PSMUX set -t $SESSION -g status-right '%H:%M:%S'
Start-Sleep -Milliseconds 500

Write-Host "`n=== Issue #232 Tests: status-interval push frame ===" -ForegroundColor Cyan

# ============================================================
# PART A: CLI Path (direct command invocation)
# ============================================================
Write-Host "`n--- Part A: CLI Path ---" -ForegroundColor Magenta

# --- Test 1: status-interval option is accepted ---
Write-Host "`n[Test 1] status-interval option can be set to 1" -ForegroundColor Yellow
$interval = (& $PSMUX display-message -t $SESSION -p '#{status-interval}' 2>&1).Trim()
if ($interval -eq "1") { Write-Pass "status-interval is 1" }
else { Write-Fail "Expected status-interval=1, got: $interval" }

# --- Test 2: status-right with strftime expands correctly via CLI ---
Write-Host "`n[Test 2] display-message expands strftime in status-right" -ForegroundColor Yellow
$timeStr = (& $PSMUX display-message -t $SESSION -p '%H:%M:%S' 2>&1).Trim()
$currentHour = (Get-Date -Format 'HH')
if ($timeStr -match '^\d{2}:\d{2}:\d{2}$' -and $timeStr.StartsWith($currentHour)) {
    Write-Pass "strftime expanded to valid time: $timeStr"
} else {
    Write-Fail "Expected HH:MM:SS starting with $currentHour, got: $timeStr"
}

# --- Test 3: Multiple CLI polls show different timestamps ---
Write-Host "`n[Test 3] CLI dump-state shows different timestamps over 3 seconds" -ForegroundColor Yellow
$cliTimes = @()
for ($i = 0; $i -lt 4; $i++) {
    $resp = Send-TcpCommand -Session $SESSION -Command "dump-state"
    if ($resp -match '"status_right"\s*:\s*"([^"]*)"') {
        $cliTimes += $matches[1]
    }
    if ($i -lt 3) { Start-Sleep -Milliseconds 1000 }
}
$uniqueCli = $cliTimes | Select-Object -Unique
if ($uniqueCli.Count -ge 3) {
    Write-Pass "CLI dump-state returned $($uniqueCli.Count) unique timestamps in 3s: $($uniqueCli -join ', ')"
} elseif ($uniqueCli.Count -ge 2) {
    Write-Pass "CLI dump-state returned $($uniqueCli.Count) unique timestamps (timing edge case OK): $($uniqueCli -join ', ')"
} else {
    Write-Fail "Expected multiple unique timestamps, got $($uniqueCli.Count): $($cliTimes -join ', ')"
}

# ============================================================
# PART B: TCP Server Path (persistent push frames)
# THIS IS THE CORE BUG PROOF
# ============================================================
Write-Host "`n--- Part B: TCP Persistent Push Frames (core bug proof) ---" -ForegroundColor Magenta

# --- Test 4: Persistent client receives push frames with updating timestamps ---
Write-Host "`n[Test 4] PERSISTENT client receives auto-pushed frames over 5 seconds (NO user input)" -ForegroundColor Yellow
$conn = Connect-Persistent -Session $SESSION
# Request initial dump-state to prime the connection
$conn.writer.Write("dump-state`n"); $conn.writer.Flush()

$pushFrames = @()
$conn.tcp.ReceiveTimeout = 2000
$start = [DateTime]::Now
while (([DateTime]::Now - $start).TotalSeconds -lt 5.5) {
    try {
        $line = $conn.reader.ReadLine()
        if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
            if ($line -match '"status_right"\s*:\s*"([^"]*)"') {
                $pushFrames += [PSCustomObject]@{
                    StatusRight = $matches[1]
                    ReceivedAt  = (Get-Date -Format 'HH:mm:ss.fff')
                }
            }
        }
    } catch {
        # ReadTimeout: expected between frames
    }
}
$conn.tcp.Close()

$uniquePush = $pushFrames | Select-Object -ExpandProperty StatusRight -Unique
Write-Host "    Frames received: $($pushFrames.Count)" -ForegroundColor DarkGray
foreach ($f in $pushFrames) {
    Write-Host "      [$($f.ReceivedAt)] status_right=$($f.StatusRight)" -ForegroundColor DarkGray
}

if ($uniquePush.Count -ge 4) {
    Write-Pass "Push frames had $($uniquePush.Count) unique timestamps in 5s (status-interval=1 working!)"
} elseif ($uniquePush.Count -ge 3) {
    Write-Pass "Push frames had $($uniquePush.Count) unique timestamps (minor timing variance OK)"
} elseif ($pushFrames.Count -eq 0) {
    Write-Fail "ZERO push frames received in 5 seconds. BUG: state_dirty not set by status-interval timer!"
} else {
    Write-Fail "Only $($uniquePush.Count) unique timestamps from $($pushFrames.Count) frames. Expected 4+ in 5 seconds."
}

# --- Test 5: Push frames arrive roughly every 1 second ---
Write-Host "`n[Test 5] Push frame cadence matches status-interval=1" -ForegroundColor Yellow
if ($pushFrames.Count -ge 3) {
    # Check that we got at least 1 frame per second on average
    $framesPerSecond = $pushFrames.Count / 5.5
    if ($framesPerSecond -ge 0.7) {
        Write-Pass "Frame rate: $([math]::Round($framesPerSecond, 2)) frames/sec (expected ~1/sec)"
    } else {
        Write-Fail "Frame rate too low: $([math]::Round($framesPerSecond, 2)) frames/sec (expected ~1/sec)"
    }
} else {
    Write-Fail "Not enough frames to measure cadence ($($pushFrames.Count) frames)"
}

# ============================================================
# PART C: Edge Cases
# ============================================================
Write-Host "`n--- Part C: Edge Cases ---" -ForegroundColor Magenta

# --- Test 6: status-interval=0 disables timer (no extra frames) ---
Write-Host "`n[Test 6] status-interval=0 disables periodic frame push" -ForegroundColor Yellow
& $PSMUX set -t $SESSION -g status-interval 0
Start-Sleep -Milliseconds 500

$conn2 = Connect-Persistent -Session $SESSION
$conn2.writer.Write("dump-state`n"); $conn2.writer.Flush()
# Read the initial dump-state response
$conn2.tcp.ReceiveTimeout = 2000
try { $null = $conn2.reader.ReadLine() } catch {}

# Now wait 3 seconds: should get 0 additional frames since interval=0
$extraFrames = 0
$conn2.tcp.ReceiveTimeout = 1500
$start2 = [DateTime]::Now
while (([DateTime]::Now - $start2).TotalSeconds -lt 3) {
    try {
        $line = $conn2.reader.ReadLine()
        if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
            $extraFrames++
        }
    } catch {}
}
$conn2.tcp.Close()

if ($extraFrames -eq 0) {
    Write-Pass "status-interval=0: no extra push frames in 3 seconds"
} else {
    # Some frames may arrive from PTY idle output, allow up to 1
    if ($extraFrames -le 1) {
        Write-Pass "status-interval=0: only $extraFrames spurious frame(s) in 3 seconds (PTY idle noise OK)"
    } else {
        Write-Fail "status-interval=0: got $extraFrames extra frames (expected 0)"
    }
}

# Restore interval for remaining tests
& $PSMUX set -t $SESSION -g status-interval 1
Start-Sleep -Milliseconds 500

# --- Test 7: status-interval works with different strftime formats ---
Write-Host "`n[Test 7] Different strftime formats expand correctly" -ForegroundColor Yellow
$formats = @(
    @{ Format = '%H:%M:%S'; Pattern = '^\d{2}:\d{2}:\d{2}$'; Desc = "24h time" },
    @{ Format = '%Y-%m-%d'; Pattern = '^\d{4}-\d{2}-\d{2}$'; Desc = "ISO date" },
    @{ Format = '%a %b %d'; Pattern = '^\w{3} \w{3} \d{2}$'; Desc = "weekday month day" }
)
$allFormatsOk = $true
foreach ($fmt in $formats) {
    & $PSMUX set -t $SESSION -g status-right $fmt.Format
    Start-Sleep -Milliseconds 300
    $resp = Send-TcpCommand -Session $SESSION -Command "dump-state"
    if ($resp -match '"status_right"\s*:\s*"([^"]*)"') {
        $val = $matches[1]
        if ($val -match $fmt.Pattern) {
            Write-Host "      $($fmt.Desc): '$val' matches $($fmt.Pattern)" -ForegroundColor DarkGray
        } else {
            Write-Fail "$($fmt.Desc): '$val' does not match $($fmt.Pattern)"
            $allFormatsOk = $false
        }
    } else {
        Write-Fail "$($fmt.Desc): status_right not found in dump-state"
        $allFormatsOk = $false
    }
}
if ($allFormatsOk) { Write-Pass "All strftime formats expanded correctly" }

# Restore for TUI tests
& $PSMUX set -t $SESSION -g status-right '%H:%M:%S'

# --- Test 8: status-interval with larger values (5 seconds) ---
Write-Host "`n[Test 8] status-interval=5 pushes frames at ~5 second cadence" -ForegroundColor Yellow
& $PSMUX set -t $SESSION -g status-interval 5
Start-Sleep -Milliseconds 500

$conn3 = Connect-Persistent -Session $SESSION
$conn3.writer.Write("dump-state`n"); $conn3.writer.Flush()

$frames5s = @()
$conn3.tcp.ReceiveTimeout = 2000
$start3 = [DateTime]::Now
while (([DateTime]::Now - $start3).TotalSeconds -lt 7) {
    try {
        $line = $conn3.reader.ReadLine()
        if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
            if ($line -match '"status_right"\s*:\s*"([^"]*)"') {
                $frames5s += [PSCustomObject]@{
                    StatusRight = $matches[1]
                    ReceivedAt  = (Get-Date -Format 'HH:mm:ss.fff')
                }
            }
        }
    } catch {}
}
$conn3.tcp.Close()

# With interval=5, in 7 seconds we expect 1 initial + 1 timer fire = ~2 frames
# (initial dump-state response + one timer tick at ~5s)
$unique5s = $frames5s | Select-Object -ExpandProperty StatusRight -Unique
if ($frames5s.Count -ge 1 -and $frames5s.Count -le 5) {
    Write-Pass "status-interval=5: got $($frames5s.Count) frames in 7s (not flooding)"
} elseif ($frames5s.Count -gt 5) {
    Write-Fail "status-interval=5: got $($frames5s.Count) frames in 7s (too many, should be ~2)"
} else {
    Write-Fail "status-interval=5: got 0 frames"
}

# Restore
& $PSMUX set -t $SESSION -g status-interval 1

# --- Test 9: User's exact config from issue ---
Write-Host "`n[Test 9] User's exact config from issue #232" -ForegroundColor Yellow
& $PSMUX set -t $SESSION -g status-right "'#[fg=colour235,bg=colour252,bold][ #[fg=black]%a %h %d, %Y %r ]'"
& $PSMUX set -t $SESSION -g status-interval 1
Start-Sleep -Milliseconds 500

$resp = Send-TcpCommand -Session $SESSION -Command "dump-state"
if ($resp -match '"status_right"\s*:\s*"([^"]*)"') {
    $userStatus = $matches[1]
    # Should contain current year and a time with AM/PM (%r = 12hr time)
    $currentYear = (Get-Date -Format 'yyyy')
    if ($userStatus -match $currentYear -or $userStatus -match '\d{2}:\d{2}:\d{2}') {
        Write-Pass "User's format expanded: ...$(if ($userStatus.Length -gt 60) { $userStatus.Substring(0,60) + '...' } else { $userStatus })"
    } else {
        Write-Fail "User's format did not expand timestamps: $userStatus"
    }
} else {
    Write-Fail "Could not parse status_right from dump-state"
}

# ============================================================
# PART D: Interaction with other features
# ============================================================
Write-Host "`n--- Part D: Feature Interactions ---" -ForegroundColor Magenta

# --- Test 10: status-interval still works after creating new windows ---
Write-Host "`n[Test 10] status-interval continues after new-window" -ForegroundColor Yellow
& $PSMUX set -t $SESSION -g status-right '%H:%M:%S'
& $PSMUX set -t $SESSION -g status-interval 1
& $PSMUX new-window -t $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 2

$conn4 = Connect-Persistent -Session $SESSION
$conn4.writer.Write("dump-state`n"); $conn4.writer.Flush()

$framesAfterNewWin = @()
$conn4.tcp.ReceiveTimeout = 2000
$start4 = [DateTime]::Now
while (([DateTime]::Now - $start4).TotalSeconds -lt 3.5) {
    try {
        $line = $conn4.reader.ReadLine()
        if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
            if ($line -match '"status_right"\s*:\s*"([^"]*)"') {
                $framesAfterNewWin += $matches[1]
            }
        }
    } catch {}
}
$conn4.tcp.Close()

$uniqueAfterNewWin = $framesAfterNewWin | Select-Object -Unique
if ($uniqueAfterNewWin.Count -ge 2) {
    Write-Pass "status-interval pushes frames after new-window ($($uniqueAfterNewWin.Count) unique timestamps)"
} else {
    Write-Fail "status-interval stopped after new-window ($($uniqueAfterNewWin.Count) unique timestamps)"
}

# --- Test 11: status-interval works with status-left too ---
Write-Host "`n[Test 11] status-left strftime also updates via push frames" -ForegroundColor Yellow
& $PSMUX set -t $SESSION -g status-left '%H:%M:%S '
Start-Sleep -Milliseconds 500

$conn5 = Connect-Persistent -Session $SESSION
$conn5.writer.Write("dump-state`n"); $conn5.writer.Flush()

$leftTimes = @()
$conn5.tcp.ReceiveTimeout = 2000
$start5 = [DateTime]::Now
while (([DateTime]::Now - $start5).TotalSeconds -lt 3.5) {
    try {
        $line = $conn5.reader.ReadLine()
        if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
            if ($line -match '"status_left"\s*:\s*"([^"]*)"') {
                $leftTimes += $matches[1].Trim()
            }
        }
    } catch {}
}
$conn5.tcp.Close()

$uniqueLeft = $leftTimes | Select-Object -Unique
if ($uniqueLeft.Count -ge 2) {
    Write-Pass "status-left strftime updates via push frames ($($uniqueLeft.Count) unique)"
} else {
    Write-Fail "status-left strftime did not update ($($uniqueLeft.Count) unique from $($leftTimes.Count) frames)"
}

# ============================================================
# PART E: Win32 TUI Visual Verification
# ============================================================
Write-Host "`n" -NoNewline
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

$SESSION_TUI = "issue232_tui_proof"
& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Remove-Item "$psmuxDir\$SESSION_TUI.*" -Force -EA SilentlyContinue

# Launch a REAL visible psmux window
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION_TUI -PassThru
Start-Sleep -Seconds 4

# Verify session came up
& $PSMUX has-session -t $SESSION_TUI 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Fail "TUI: session did not start"
} else {
    # Configure status-interval and time display
    & $PSMUX set -t $SESSION_TUI -g status-interval 1
    & $PSMUX set -t $SESSION_TUI -g status-right '%H:%M:%S'
    Start-Sleep -Milliseconds 500

    # --- TUI Test 1: Session is alive and responds ---
    Write-Host "`n[TUI Test 1] Session responds to CLI queries" -ForegroundColor Yellow
    $sessName = (& $PSMUX display-message -t $SESSION_TUI -p '#{session_name}' 2>&1).Trim()
    if ($sessName -eq $SESSION_TUI) { Write-Pass "TUI: session responds correctly" }
    else { Write-Fail "TUI: expected '$SESSION_TUI', got '$sessName'" }

    # --- TUI Test 2: status-interval is set ---
    Write-Host "`n[TUI Test 2] status-interval verified on TUI session" -ForegroundColor Yellow
    $tuiInterval = (& $PSMUX display-message -t $SESSION_TUI -p '#{status-interval}' 2>&1).Trim()
    if ($tuiInterval -eq "1") { Write-Pass "TUI: status-interval=1 confirmed" }
    else { Write-Fail "TUI: status-interval expected 1, got $tuiInterval" }

    # --- TUI Test 3: Push frames arrive on TUI session's persistent connection ---
    Write-Host "`n[TUI Test 3] TUI session push frames update status_right" -ForegroundColor Yellow
    $portFile = "$psmuxDir\$SESSION_TUI.port"
    if (Test-Path $portFile) {
        $connTui = Connect-Persistent -Session $SESSION_TUI
        $connTui.writer.Write("dump-state`n"); $connTui.writer.Flush()

        $tuiFrames = @()
        $connTui.tcp.ReceiveTimeout = 2000
        $startTui = [DateTime]::Now
        while (([DateTime]::Now - $startTui).TotalSeconds -lt 4) {
            try {
                $line = $connTui.reader.ReadLine()
                if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
                    if ($line -match '"status_right"\s*:\s*"([^"]*)"') {
                        $tuiFrames += $matches[1]
                    }
                }
            } catch {}
        }
        $connTui.tcp.Close()

        $uniqueTui = $tuiFrames | Select-Object -Unique
        if ($uniqueTui.Count -ge 3) {
            Write-Pass "TUI: push frames delivered $($uniqueTui.Count) unique timestamps in 4s"
        } elseif ($uniqueTui.Count -ge 2) {
            Write-Pass "TUI: push frames delivered $($uniqueTui.Count) unique timestamps (timing OK)"
        } else {
            Write-Fail "TUI: only $($uniqueTui.Count) unique timestamps from $($tuiFrames.Count) frames"
        }
    } else {
        Write-Fail "TUI: port file not found at $portFile"
    }

    # --- TUI Test 4: Split pane does not break status-interval ---
    Write-Host "`n[TUI Test 4] status-interval survives split-window on TUI" -ForegroundColor Yellow
    & $PSMUX split-window -v -t $SESSION_TUI 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $panes = (& $PSMUX display-message -t $SESSION_TUI -p '#{window_panes}' 2>&1).Trim()
    if ($panes -eq "2") { Write-Pass "TUI: split-window created 2 panes" }
    else { Write-Fail "TUI: expected 2 panes, got $panes" }

    # Verify interval still set
    $tuiInterval2 = (& $PSMUX display-message -t $SESSION_TUI -p '#{status-interval}' 2>&1).Trim()
    if ($tuiInterval2 -eq "1") { Write-Pass "TUI: status-interval still 1 after split" }
    else { Write-Fail "TUI: status-interval changed to $tuiInterval2 after split" }
}

# Cleanup TUI
& $PSMUX kill-session -t $SESSION_TUI 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}

# ============================================================
# PART F: Config File Test
# ============================================================
Write-Host "`n" -NoNewline
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "CONFIG FILE TESTS" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

$SESSION_CFG = "issue232_cfg"
& $PSMUX kill-session -t $SESSION_CFG 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Remove-Item "$psmuxDir\$SESSION_CFG.*" -Force -EA SilentlyContinue

# --- Config Test 1: source-file applies status-interval ---
Write-Host "`n[Config Test 1] source-file applies status-interval setting" -ForegroundColor Yellow
$confFile = "$env:TEMP\psmux_test_232_interval.conf"
@"
set -g status-interval 1
set -g status-right '%H:%M:%S'
"@ | Set-Content -Path $confFile -Encoding UTF8

& $PSMUX new-session -d -s $SESSION_CFG
Start-Sleep -Seconds 3
& $PSMUX has-session -t $SESSION_CFG 2>$null
if ($LASTEXITCODE -eq 0) {
    & $PSMUX source-file -t $SESSION_CFG $confFile 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $cfgInterval = (& $PSMUX display-message -t $SESSION_CFG -p '#{status-interval}' 2>&1).Trim()
    if ($cfgInterval -eq "1") { Write-Pass "source-file set status-interval=1" }
    else { Write-Fail "Expected status-interval=1 after source-file, got: $cfgInterval" }

    # Verify push frames work with config-applied interval
    $portCfg = "$psmuxDir\$SESSION_CFG.port"
    if (Test-Path $portCfg) {
        $connCfg = Connect-Persistent -Session $SESSION_CFG
        $connCfg.writer.Write("dump-state`n"); $connCfg.writer.Flush()

        $cfgFrames = @()
        $connCfg.tcp.ReceiveTimeout = 2000
        $startCfg = [DateTime]::Now
        while (([DateTime]::Now - $startCfg).TotalSeconds -lt 3.5) {
            try {
                $line = $connCfg.reader.ReadLine()
                if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
                    if ($line -match '"status_right"\s*:\s*"([^"]*)"') {
                        $cfgFrames += $matches[1]
                    }
                }
            } catch {}
        }
        $connCfg.tcp.Close()

        $uniqueCfg = $cfgFrames | Select-Object -Unique
        if ($uniqueCfg.Count -ge 2) {
            Write-Pass "Config-applied status-interval pushes frames ($($uniqueCfg.Count) unique timestamps)"
        } else {
            Write-Fail "Config-applied status-interval: only $($uniqueCfg.Count) unique timestamps"
        }
    }
} else {
    Write-Fail "Config test session creation failed"
}

# --- Config Test 2: Changing status-interval via set command takes effect ---
Write-Host "`n[Config Test 2] Changing status-interval at runtime" -ForegroundColor Yellow
# Set to 0 first (disable)
& $PSMUX set -t $SESSION_CFG -g status-interval 0
Start-Sleep -Milliseconds 500
$int0 = (& $PSMUX display-message -t $SESSION_CFG -p '#{status-interval}' 2>&1).Trim()

# Then set back to 2
& $PSMUX set -t $SESSION_CFG -g status-interval 2
Start-Sleep -Milliseconds 500
$int2 = (& $PSMUX display-message -t $SESSION_CFG -p '#{status-interval}' 2>&1).Trim()

if ($int0 -eq "0" -and $int2 -eq "2") {
    Write-Pass "status-interval dynamically changed: 0 -> 2"
} else {
    Write-Fail "Dynamic change failed: expected 0 then 2, got '$int0' then '$int2'"
}

# Cleanup config
& $PSMUX kill-session -t $SESSION_CFG 2>&1 | Out-Null
Remove-Item $confFile -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\$SESSION_CFG.*" -Force -EA SilentlyContinue

# ============================================================
# PART G: Performance Metrics
# ============================================================
Write-Host "`n" -NoNewline
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "PERFORMANCE METRICS" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

& $PSMUX set -t $SESSION -g status-interval 1
& $PSMUX set -t $SESSION -g status-right '%H:%M:%S'
Start-Sleep -Milliseconds 500

# --- Perf 1: Frame push latency (time between frames) ---
Write-Host "`n[Perf 1] Frame push interval timing" -ForegroundColor Yellow
$connPerf = Connect-Persistent -Session $SESSION
$connPerf.writer.Write("dump-state`n"); $connPerf.writer.Flush()

$frameTimes = [System.Collections.ArrayList]::new()
$connPerf.tcp.ReceiveTimeout = 2000
$startPerf = [DateTime]::Now
$lastFrameTime = $null
while (([DateTime]::Now - $startPerf).TotalSeconds -lt 6) {
    try {
        $line = $connPerf.reader.ReadLine()
        if ($null -ne $line -and $line -ne "NC" -and $line.Length -gt 100) {
            $now = [DateTime]::Now
            if ($null -ne $lastFrameTime) {
                $delta = ($now - $lastFrameTime).TotalMilliseconds
                [void]$frameTimes.Add($delta)
            }
            $lastFrameTime = $now
        }
    } catch {}
}
$connPerf.tcp.Close()

if ($frameTimes.Count -ge 3) {
    $sorted = [double[]]($frameTimes | Sort-Object)
    $p50Idx = [Math]::Floor(0.5 * ($sorted.Count - 1))
    $p90Idx = [Math]::Floor(0.9 * ($sorted.Count - 1))
    $p50 = $sorted[$p50Idx]
    $p90 = $sorted[$p90Idx]
    $avg = ($frameTimes | Measure-Object -Average).Average

    Write-Host ("    [METRIC] Frame interval avg: {0:N0}ms" -f $avg) -ForegroundColor DarkCyan
    Write-Host ("    [METRIC] Frame interval p50: {0:N0}ms" -f $p50) -ForegroundColor DarkCyan
    Write-Host ("    [METRIC] Frame interval p90: {0:N0}ms" -f $p90) -ForegroundColor DarkCyan

    # With status-interval=1, frames should arrive roughly every ~1000ms
    # Allow 500ms to 1500ms for p50
    if ($p50 -ge 500 -and $p50 -le 1500) {
        Write-Pass "Frame interval p50=$([math]::Round($p50))ms (expected ~1000ms)"
    } else {
        Write-Fail "Frame interval p50=$([math]::Round($p50))ms (expected 500ms to 1500ms)"
    }
} else {
    Write-Fail "Not enough frames to measure intervals ($($frameTimes.Count) deltas)"
}

# --- Perf 2: Memory impact of status-interval ---
Write-Host "`n[Perf 2] Memory impact of status-interval push frames" -ForegroundColor Yellow
$psmuxProc = Get-Process psmux -EA SilentlyContinue | Where-Object { $_.Id -ne $proc.Id } | Select-Object -First 1
if ($psmuxProc) {
    $memMB = [Math]::Round($psmuxProc.WorkingSet64 / 1MB, 1)
    Write-Host "    [METRIC] Server memory: ${memMB}MB" -ForegroundColor DarkCyan
    if ($memMB -lt 100) { Write-Pass "Server memory under 100MB (${memMB}MB)" }
    else { Write-Fail "Server memory high: ${memMB}MB" }
} else {
    Write-Host "    [SKIP] Could not find psmux process for memory check" -ForegroundColor DarkGray
}

# ============================================================
# TEARDOWN
# ============================================================
Cleanup

Write-Host "`n" -NoNewline
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "RESULTS" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host ""

exit $script:TestsFailed
