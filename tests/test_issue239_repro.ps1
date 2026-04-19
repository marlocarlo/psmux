# Issue #239: Powershell modules are not properly loading when entering psmux
# Reproduction test: Prove whether PSReadLine PredictionViewStyle / PredictionSource 
# are wiped inside psmux sessions.
#
# Related: Issue #165 (same root cause area - warm pane PSReadLine init)
# The user reports: CompletionPredictor module + ListView not working inside psmux.
# Their profile has:
#   Set-PSReadLineOption -EditMode Emacs
#   Set-PSReadLineOption -PredictionSource HistoryAndPlugin
#   Set-PSReadLineOption -PredictionViewStyle ListView
#   Set-PSReadlineKeyHandler -Key Tab -Function MenuComplete

$ErrorActionPreference = "Continue"
$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = (Get-Command psmux -EA Stop).Source }
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
        Start-Sleep -Milliseconds 250
    }
    return $false
}

$allSessions = @("test239_nopred", "test239_pred", "test239_newwin", "test239_split")

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Issue #239 Reproduction: PSReadLine Predictions in psmux"  -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# === First: Establish baseline — what does the CURRENT host session have? ===
Write-Host "`n--- Baseline: Current host PSReadLine settings ---" -ForegroundColor Yellow
$hostPredSource = try { (Get-PSReadLineOption).PredictionSource } catch { "N/A" }
$hostPredView   = try { (Get-PSReadLineOption).PredictionViewStyle } catch { "N/A" }
Write-Host "  Host PredictionSource:    $hostPredSource"
Write-Host "  Host PredictionViewStyle: $hostPredView"

# ================================================================
# TEST A: Session WITHOUT allow-predictions (the DEFAULT)
# This is what the #239 reporter almost certainly has.
# ================================================================
Write-Host "`n=== TEST A: allow-predictions OFF (default) ===" -ForegroundColor Cyan
Cleanup -Sessions @("test239_nopred")

# Create a config with NO allow-predictions (or explicitly off)
$confNoPred = "$env:TEMP\psmux_test239_nopred.conf"
"# No allow-predictions" | Set-Content -Path $confNoPred -Encoding UTF8

$env:PSMUX_CONFIG_FILE = $confNoPred
& $PSMUX new-session -d -s test239_nopred
$env:PSMUX_CONFIG_FILE = $null

if (-not (Wait-Session "test239_nopred")) {
    Write-Fail "Session test239_nopred never came alive"
    exit 1
}
Write-Host "  Session test239_nopred is alive"

# Wait for shell to initialize
Start-Sleep -Seconds 5

# Query PSReadLine settings inside the psmux session
& $PSMUX send-keys -t test239_nopred '(Get-PSReadLineOption).PredictionSource' Enter
Start-Sleep -Seconds 2
$capSrc1 = & $PSMUX capture-pane -t test239_nopred -p 2>&1 | Out-String

& $PSMUX send-keys -t test239_nopred '(Get-PSReadLineOption).PredictionViewStyle' Enter
Start-Sleep -Seconds 2
$capView1 = & $PSMUX capture-pane -t test239_nopred -p 2>&1 | Out-String

Write-Host "`n[Test A1] PredictionSource with allow-predictions OFF:" -ForegroundColor Yellow
Write-Host "  Captured output:"
$capSrc1.Split("`n") | Where-Object { $_ -match "PredictionSource|None|History|Plugin" } | ForEach-Object { Write-Host "    $_" }

if ($capSrc1 -match "(?m)^None\s*$") {
    Write-Pass "PredictionSource is None (EXPECTED: psmux forces it off by default via PSRL_FIX)"
} elseif ($capSrc1 -match "(?m)^(History|HistoryAndPlugin)\s*$") {
    Write-Fail "PredictionSource is NOT None — PSRL_FIX didn't run or didn't take effect"
} else {
    Write-Host "  [INFO] Could not parse PredictionSource from capture" -ForegroundColor DarkYellow
    Write-Host "  Raw capture:" -ForegroundColor DarkGray
    Write-Host $capSrc1
}

Write-Host "`n[Test A2] PredictionViewStyle with allow-predictions OFF:" -ForegroundColor Yellow
$capView1.Split("`n") | Where-Object { $_ -match "PredictionViewStyle|InlineView|ListView" } | ForEach-Object { Write-Host "    $_" }

if ($capView1 -match "(?m)^InlineView\s*$") {
    Write-Pass "PredictionViewStyle is InlineView (EXPECTED: PSRL_FIX forces InlineView)"
} elseif ($capView1 -match "(?m)^ListView\s*$") {
    Write-Fail "PredictionViewStyle is ListView — PSRL_FIX didn't override it"
} else {
    Write-Host "  [INFO] Could not parse PredictionViewStyle from capture" -ForegroundColor DarkYellow
    Write-Host $capView1
}

# ================================================================
# TEST B: Session WITH allow-predictions ON
# This is the fix from #165 — should preserve user's PSReadLine settings
# ================================================================
Write-Host "`n=== TEST B: allow-predictions ON ===" -ForegroundColor Cyan
Cleanup -Sessions @("test239_pred")

$confPred = "$env:TEMP\psmux_test239_pred.conf"
"set -g allow-predictions on" | Set-Content -Path $confPred -Encoding UTF8

$env:PSMUX_CONFIG_FILE = $confPred
& $PSMUX new-session -d -s test239_pred
$env:PSMUX_CONFIG_FILE = $null

if (-not (Wait-Session "test239_pred")) {
    Write-Fail "Session test239_pred never came alive"
} else {
    Write-Host "  Session test239_pred is alive"
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t test239_pred '(Get-PSReadLineOption).PredictionSource' Enter
    Start-Sleep -Seconds 2
    $capSrc2 = & $PSMUX capture-pane -t test239_pred -p 2>&1 | Out-String

    & $PSMUX send-keys -t test239_pred '(Get-PSReadLineOption).PredictionViewStyle' Enter
    Start-Sleep -Seconds 2
    $capView2 = & $PSMUX capture-pane -t test239_pred -p 2>&1 | Out-String

    Write-Host "`n[Test B1] PredictionSource with allow-predictions ON:" -ForegroundColor Yellow
    $capSrc2.Split("`n") | Where-Object { $_ -match "PredictionSource|None|History|Plugin" } | ForEach-Object { Write-Host "    $_" }

    if ($capSrc2 -match "(?m)^(History|HistoryAndPlugin)\s*$") {
        Write-Pass "PredictionSource is restored (allow-predictions ON works!)"
    } elseif ($capSrc2 -match "(?m)^None\s*$") {
        Write-Fail "BUG: PredictionSource is still None even with allow-predictions ON"
    } else {
        Write-Host "  [INFO] Could not parse PredictionSource" -ForegroundColor DarkYellow
        Write-Host $capSrc2
    }

    Write-Host "`n[Test B2] PredictionViewStyle with allow-predictions ON:" -ForegroundColor Yellow
    $capView2.Split("`n") | Where-Object { $_ -match "PredictionViewStyle|InlineView|ListView" } | ForEach-Object { Write-Host "    $_" }

    # With allow-predictions ON, PSRL_PRED_RESTORE restores PredictionSource but
    # NEVER touches PredictionViewStyle. So if user's profile sets ListView, it should survive.
    # If user's profile does NOT set it, it stays at whatever default pwsh has (InlineView).
    if ($capView2 -match "(?m)^ListView\s*$") {
        Write-Pass "PredictionViewStyle is ListView (user's profile setting preserved!)"
    } elseif ($capView2 -match "(?m)^InlineView\s*$") {
        # This is OK if the current profile doesn't set ListView
        Write-Host "  [INFO] PredictionViewStyle is InlineView" -ForegroundColor DarkYellow
        Write-Host "  (This is expected if your profile doesn't set ListView)" -ForegroundColor DarkGray
    } else {
        Write-Host "  [INFO] Could not parse PredictionViewStyle" -ForegroundColor DarkYellow
        Write-Host $capView2
    }
}

# ================================================================
# TEST C: Verify show-options includes allow-predictions
# ================================================================
Write-Host "`n=== TEST C: show-options output ===" -ForegroundColor Cyan

Write-Host "[Test C1] show-options for session WITH allow-predictions:" -ForegroundColor Yellow
$opts = & $PSMUX show-options -g -t test239_pred 2>&1 | Out-String
if ($opts -match "allow-predictions\s+on") {
    Write-Pass "show-options shows 'allow-predictions on'"
} elseif ($opts -match "allow-predictions") {
    Write-Fail "allow-predictions found but not 'on': $($opts -split "`n" | Select-String 'allow-predictions')"
} else {
    Write-Fail "allow-predictions NOT in show-options output"
}

Write-Host "[Test C2] show-options for session WITHOUT allow-predictions:" -ForegroundColor Yellow
$opts2 = & $PSMUX show-options -g -t test239_nopred 2>&1 | Out-String
if ($opts2 -match "allow-predictions\s+off") {
    Write-Pass "show-options shows 'allow-predictions off'"
} else {
    Write-Host "  [INFO] Output: $($opts2 -split "`n" | Select-String 'allow-predictions')" -ForegroundColor DarkYellow
}

# ================================================================
# TEST D: Check what PSRL init string is actually sent (via process cmdline)
# ================================================================
Write-Host "`n=== TEST D: Verify actual init commands ===" -ForegroundColor Cyan

Write-Host "[Test D1] Check pwsh process command lines:" -ForegroundColor Yellow
$procs = Get-CimInstance Win32_Process -Filter "Name='pwsh.exe'" | Select-Object ProcessId, CommandLine
foreach ($p in $procs) {
    if ($p.CommandLine -match "PredictionSource None") {
        if ($p.CommandLine -match "PSRL_CRASH_GUARD|__psmux_origPred") {
            Write-Host "  PID $($p.ProcessId): CRASH_GUARD path (allow-predictions ON)" -ForegroundColor DarkCyan
        } else {
            Write-Host "  PID $($p.ProcessId): PSRL_FIX path (allow-predictions OFF)" -ForegroundColor DarkCyan
        }
    }
}

# ================================================================
# TEST E: New windows/panes in allow-predictions session
# This is critical: does a NEW window created AFTER session start 
# also get the correct init string?
# ================================================================
Write-Host "`n=== TEST E: New window in allow-predictions ON session ===" -ForegroundColor Cyan

& $PSMUX new-window -t test239_pred 2>&1 | Out-Null
Start-Sleep -Seconds 5

& $PSMUX send-keys -t test239_pred '(Get-PSReadLineOption).PredictionSource' Enter
Start-Sleep -Seconds 2
$capSrc3 = & $PSMUX capture-pane -t test239_pred -p 2>&1 | Out-String

Write-Host "[Test E1] PredictionSource in NEW window (allow-predictions ON):" -ForegroundColor Yellow
$capSrc3.Split("`n") | Where-Object { $_ -match "PredictionSource|None|History|Plugin" } | ForEach-Object { Write-Host "    $_" }

if ($capSrc3 -match "(?m)^(History|HistoryAndPlugin)\s*$") {
    Write-Pass "New window PredictionSource restored"
} elseif ($capSrc3 -match "(?m)^None\s*$") {
    Write-Fail "BUG: New window PredictionSource is None even with allow-predictions ON"
} else {
    Write-Host "  [INFO] Could not parse" -ForegroundColor DarkYellow
    Write-Host $capSrc3
}

# ================================================================
# TEST F: Split pane in allow-predictions session
# ================================================================
Write-Host "`n=== TEST F: Split pane in allow-predictions ON session ===" -ForegroundColor Cyan

& $PSMUX split-window -v -t test239_pred 2>&1 | Out-Null
Start-Sleep -Seconds 5

& $PSMUX send-keys -t test239_pred '(Get-PSReadLineOption).PredictionSource' Enter
Start-Sleep -Seconds 2
$capSrc4 = & $PSMUX capture-pane -t test239_pred -p 2>&1 | Out-String

Write-Host "[Test F1] PredictionSource in SPLIT PANE (allow-predictions ON):" -ForegroundColor Yellow
$capSrc4.Split("`n") | Where-Object { $_ -match "PredictionSource|None|History|Plugin" } | ForEach-Object { Write-Host "    $_" }

if ($capSrc4 -match "(?m)^(History|HistoryAndPlugin)\s*$") {
    Write-Pass "Split pane PredictionSource restored"
} elseif ($capSrc4 -match "(?m)^None\s*$") {
    Write-Fail "BUG: Split pane PredictionSource is None even with allow-predictions ON"
} else {
    Write-Host "  [INFO] Could not parse" -ForegroundColor DarkYellow
    Write-Host $capSrc4
}

# ================================================================
# CLEANUP
# ================================================================
Write-Host "`n=== Cleanup ===" -ForegroundColor DarkGray
Cleanup -Sessions $allSessions
Remove-Item "$env:TEMP\psmux_test239_*" -Force -EA SilentlyContinue

# ================================================================
# VERDICT
# ================================================================
Write-Host "`n============================================================" -ForegroundColor Cyan
Write-Host "  RESULTS" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })

Write-Host "`n--- ANALYSIS ---" -ForegroundColor Yellow
Write-Host @"
Issue #239 reporter config:
  Set-PSReadLineOption -EditMode Emacs
  Set-PSReadLineOption -PredictionSource HistoryAndPlugin
  Set-PSReadLineOption -PredictionViewStyle ListView
  Set-PSReadlineKeyHandler -Key Tab -Function MenuComplete

They do NOT mention 'allow-predictions on' in psmux.conf.

By default (allow-predictions OFF), PSRL_FIX forces:
  PredictionSource = None
  PredictionViewStyle = InlineView
  Removes F2 key handler

This COMPLETELY wipes their PSReadLine prediction settings.
The fix from #165 added 'allow-predictions on' config option,
but users must KNOW to add it.

VERDICT: If all tests above pass as expected, this is NOT a code bug.
It's a CONFIGURATION issue — the user needs 'set -g allow-predictions on'.
"@

exit $script:TestsFailed
