# Issue #230: TUI Visual Verification Proof
# Launches REAL visible psmux windows and verifies join-pane and C-c
# via CLI commands (not screen scraping)

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

function Cleanup {
    @("tui230_join", "tui230_sig", "tui230_donor", "tui230_target") | ForEach-Object {
        & $PSMUX kill-session -t $_ 2>&1 | Out-Null
    }
    Start-Sleep -Milliseconds 1000
    @("tui230_join", "tui230_sig", "tui230_donor", "tui230_target") | ForEach-Object {
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

Cleanup
Write-Host "`n" + ("=" * 60) -ForegroundColor Cyan
Write-Host "  Issue #230: Win32 TUI VISUAL VERIFICATION" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

# ============================================================
# TUI TEST 1: join-pane in visible window
# ============================================================
Write-Host "`n[TUI Test 1] join-pane in visible attached window" -ForegroundColor Yellow

$SESSION = "tui230_join"
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
Start-Sleep -Seconds 5

if (-not (Wait-Session $SESSION)) {
    Write-Fail "TUI session did not start"
} else {
    # Create second window with a split
    & $PSMUX new-window -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    # Check initial state
    & $PSMUX select-window -t "${SESSION}:0" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $w0Before = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1).Trim()
    Write-Host "    Window 0 panes before join: $w0Before"

    # Attempt join-pane from window 1 to window 0
    $joinOut = & $PSMUX join-pane -h -s "${SESSION}:1.0" -t "${SESSION}:0.0" 2>&1 | Out-String
    Write-Host "    join-pane result: '$($joinOut.Trim())'"
    Start-Sleep -Seconds 2

    & $PSMUX select-window -t "${SESSION}:0" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $w0After = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1).Trim()
    Write-Host "    Window 0 panes after join: $w0After"

    if ([int]$w0After -gt [int]$w0Before) {
        Write-Pass "TUI: join-pane moved pane in visible window ($w0Before -> $w0After)"
    } else {
        Write-Fail "TUI: join-pane DID NOT work in visible window. Panes unchanged ($w0Before -> $w0After). BUG CONFIRMED."
    }

    # Verify TUI is still functional (can respond to commands)
    $sessName = (& $PSMUX display-message -t $SESSION -p '#{session_name}' 2>&1).Trim()
    if ($sessName -eq $SESSION) {
        Write-Pass "TUI: session still responsive after join-pane attempt"
    } else {
        Write-Fail "TUI: session not responding correctly (got: $sessName)"
    }
}

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Start-Sleep -Seconds 1

# ============================================================
# TUI TEST 2: send-keys C-c in visible window
# ============================================================
Write-Host "`n[TUI Test 2] send-keys C-c in visible attached window" -ForegroundColor Yellow

$SESSION = "tui230_sig"
$proc2 = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
Start-Sleep -Seconds 5

if (-not (Wait-Session $SESSION)) {
    Write-Fail "TUI session did not start"
} else {
    # Start a long-running command in the visible pane
    & $PSMUX send-keys -t $SESSION "ping -t 127.0.0.1" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    # Verify ping is running
    $capBefore = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    $pingRunning = $capBefore -match "Reply from|Pinging|bytes="
    Write-Host "    Ping running in TUI: $pingRunning"

    # Send C-c
    & $PSMUX send-keys -t $SESSION C-c 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # Verify ping stopped
    $capAfter = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    $pingStopped = $capAfter -match "Ping statistics|Control-C|\^C|Approximate"
    $promptBack = $capAfter -match "PS [A-Z]:\\|C:\\.*>"

    if ($pingStopped -or $promptBack) {
        Write-Pass "TUI: send-keys C-c stopped ping in visible window"
    } else {
        # Send Enter to see if prompt returns
        & $PSMUX send-keys -t $SESSION "" Enter 2>&1 | Out-Null
        Start-Sleep -Seconds 2
        $capFinal = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
        
        if ($capFinal -match "Reply from 127\.0\.0\.1") {
            Write-Fail "TUI: send-keys C-c DID NOT stop ping in visible window. BUG CONFIRMED."
            Write-Host "    === Last 10 lines of capture ===" -ForegroundColor DarkGray
            $lines = ($capFinal -split "`n") | Select-Object -Last 10
            $lines | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
        } else {
            Write-Pass "TUI: ping appears stopped (no more replies after C-c)"
        }
    }

    # Verify TUI is still functional
    $sessName2 = (& $PSMUX display-message -t $SESSION -p '#{session_name}' 2>&1).Trim()
    if ($sessName2 -eq $SESSION) {
        Write-Pass "TUI: session still responsive after C-c test"
    } else {
        Write-Fail "TUI: session not responding correctly (got: $sessName2)"
    }
}

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
try { Stop-Process -Id $proc2.Id -Force -EA SilentlyContinue } catch {}
Start-Sleep -Seconds 1

# ============================================================
# TUI TEST 3: join-pane cross-session in TUI
# ============================================================
Write-Host "`n[TUI Test 3] join-pane cross-session from visible TUI" -ForegroundColor Yellow

$DONOR = "tui230_donor"
$TARGET = "tui230_target"

# Launch target as visible TUI window
$proc3 = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$TARGET -PassThru
Start-Sleep -Seconds 4

# Create donor as detached
& $PSMUX new-session -d -s $DONOR
Start-Sleep -Seconds 3

if ((Wait-Session $TARGET) -and (Wait-Session $DONOR)) {
    # Split donor so it has 2 panes
    & $PSMUX split-window -h -t $DONOR 2>&1 | Out-Null
    Start-Sleep -Seconds 2

    $donorBefore = (& $PSMUX display-message -t $DONOR -p '#{window_panes}' 2>&1).Trim()
    $targetBefore = (& $PSMUX display-message -t $TARGET -p '#{window_panes}' 2>&1).Trim()
    Write-Host "    Donor panes: $donorBefore, Target panes: $targetBefore"

    # Cross-session join
    $joinCross = & $PSMUX join-pane -h -s "${DONOR}:0.1" -t "${TARGET}:0.0" 2>&1 | Out-String
    Write-Host "    Cross-session join result: '$($joinCross.Trim())'"
    Start-Sleep -Seconds 2

    $donorAfter = (& $PSMUX display-message -t $DONOR -p '#{window_panes}' 2>&1).Trim()
    $targetAfter = (& $PSMUX display-message -t $TARGET -p '#{window_panes}' 2>&1).Trim()
    Write-Host "    Donor after: $donorAfter, Target after: $targetAfter"

    if ([int]$targetAfter -gt [int]$targetBefore) {
        Write-Pass "TUI: cross-session join-pane worked in visible window"
    } else {
        Write-Fail "TUI: cross-session join-pane FAILED. Target unchanged ($targetBefore -> $targetAfter). BUG CONFIRMED."
    }
} else {
    Write-Fail "TUI: could not start both sessions"
}

& $PSMUX kill-session -t $DONOR 2>&1 | Out-Null
& $PSMUX kill-session -t $TARGET 2>&1 | Out-Null
try { Stop-Process -Id $proc3.Id -Force -EA SilentlyContinue } catch {}
Start-Sleep -Seconds 1

# ============================================================
# TUI TEST 4: Verify basic TUI operations still work (regression check)
# ============================================================
Write-Host "`n[TUI Test 4] Regression: basic split-window and zoom in TUI" -ForegroundColor Yellow

$SESSION = "tui230_join"
$proc4 = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
Start-Sleep -Seconds 4

if (Wait-Session $SESSION) {
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $panes = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1).Trim()
    if ($panes -eq "2") { Write-Pass "TUI: split-window created 2 panes" }
    else { Write-Fail "TUI: expected 2 panes, got $panes" }

    & $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $zoom = (& $PSMUX display-message -t $SESSION -p '#{window_zoomed_flag}' 2>&1).Trim()
    if ($zoom -eq "1") { Write-Pass "TUI: resize-pane -Z zoomed" }
    else { Write-Fail "TUI: zoom expected 1, got $zoom" }
} else {
    Write-Fail "TUI: regression session did not start"
}

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
try { Stop-Process -Id $proc4.Id -Force -EA SilentlyContinue } catch {}

# ============================================================
Cleanup

Write-Host "`n" + ("=" * 60) -ForegroundColor Cyan
Write-Host "  TUI Visual Verification Results" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })

if ($script:TestsFailed -gt 0) {
    Write-Host "`n  CONCLUSION: Bug(s) from issue #230 are visible in live TUI." -ForegroundColor Red
} else {
    Write-Host "`n  CONCLUSION: All TUI behaviors work correctly." -ForegroundColor Green
}

exit $script:TestsFailed
