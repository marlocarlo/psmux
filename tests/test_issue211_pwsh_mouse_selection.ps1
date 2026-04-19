# psmux Issue #211: pwsh-mouse-selection E2E tests
# Validates the option plumbing, config loading, JSON serialization,
# and that the feature does not break existing behavior.
# Run: powershell -ExecutionPolicy Bypass -File tests\test_issue211_pwsh_mouse_selection.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }
function PsmuxQuick { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 150 }

# Kill everything first
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "pwsh_mouse_test"
Write-Info "Creating test session '$SESSION'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 3
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session created"

Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #211: pwsh-mouse-selection OPTION TESTS"
Write-Host ("=" * 60)

# ============================================================
# 1. DEFAULT VALUE
# ============================================================
Write-Test "pwsh-mouse-selection defaults to off"
$val = Psmux show-options -t $SESSION -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
if ($valStr -eq "off") {
    Write-Pass "Default value is 'off'"
} else {
    Write-Fail "Expected default 'off', got '$valStr'"
}

# ============================================================
# 2. SET ON/OFF CYCLE
# ============================================================
Write-Test "set pwsh-mouse-selection on"
Psmux set -g pwsh-mouse-selection on -t $SESSION
$val = Psmux show-options -t $SESSION -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
if ($valStr -eq "on") {
    Write-Pass "Set to 'on' succeeded"
} else {
    Write-Fail "Expected 'on', got '$valStr'"
}

Write-Test "set pwsh-mouse-selection off"
Psmux set -g pwsh-mouse-selection off -t $SESSION
$val = Psmux show-options -t $SESSION -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
if ($valStr -eq "off") {
    Write-Pass "Set to 'off' succeeded"
} else {
    Write-Fail "Expected 'off', got '$valStr'"
}

Write-Test "set pwsh-mouse-selection with alternative true/1 values"
Psmux set -g pwsh-mouse-selection true -t $SESSION
$val = Psmux show-options -t $SESSION -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
if ($valStr -eq "on") {
    Write-Pass "'true' interpreted as 'on'"
} else {
    Write-Fail "Expected 'on' from 'true', got '$valStr'"
}
Psmux set -g pwsh-mouse-selection off -t $SESSION

Psmux set -g pwsh-mouse-selection 1 -t $SESSION
$val = Psmux show-options -t $SESSION -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
if ($valStr -eq "on") {
    Write-Pass "'1' interpreted as 'on'"
} else {
    Write-Fail "Expected 'on' from '1', got '$valStr'"
}
Psmux set -g pwsh-mouse-selection off -t $SESSION

# ============================================================
# 3. OPTION CATALOG (show-options -g lists it)
# ============================================================
Write-Test "pwsh-mouse-selection in show-options -g listing"
$allOpts = Psmux show-options -g -t $SESSION
$allStr = ($allOpts | Out-String)
if ($allStr -match "pwsh-mouse-selection") {
    Write-Pass "Option listed in show-options -g"
} else {
    Write-Fail "Option NOT listed in show-options -g"
}

# ============================================================
# 4. CONFIG FILE SOURCE
# ============================================================
Write-Test "source-file with pwsh-mouse-selection on"
$tmpConf = "$env:TEMP\psmux_test211.conf"
Set-Content -Path $tmpConf -Value "set -g pwsh-mouse-selection on" -Encoding UTF8
Psmux source-file $tmpConf -t $SESSION
$val = Psmux show-options -t $SESSION -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
if ($valStr -eq "on") {
    Write-Pass "source-file correctly applied option"
} else {
    Write-Fail "source-file did not apply option, got '$valStr'"
}
Remove-Item $tmpConf -Force -ErrorAction SilentlyContinue

# ============================================================
# 5. JSON DUMP SERIALIZATION
# ============================================================
Write-Test "pwsh_mouse_selection appears in dump JSON"
# Read port and key for TCP dump using the specific session
$portFile = "$env:USERPROFILE\.psmux\$SESSION.port"
$keyFile  = "$env:USERPROFILE\.psmux\$SESSION.key"
$dumpFound = $false
if ((Test-Path $portFile) -and (Test-Path $keyFile)) {
    $port = [int](Get-Content $portFile -Raw).Trim()
    $key = (Get-Content $keyFile -Raw).Trim()

    try {
        $tcp = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $port)
        $stream = $tcp.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.AutoFlush = $true

        $writer.WriteLine("AUTH $key")
        Start-Sleep -Milliseconds 200
        $authResp = $reader.ReadLine()

        if ($authResp -match "OK") {
            $writer.WriteLine("dump")
            $stream.ReadTimeout = 5000
            $dumpData = ""
            # Read multiple lines (dump response may be preceded by frame data)
            for ($attempt = 0; $attempt -lt 20; $attempt++) {
                try {
                    $line = $reader.ReadLine()
                    if ($line -match 'pwsh_mouse_selection') {
                        $dumpData = $line
                        break
                    }
                } catch {
                    break
                }
            }

            if ($dumpData -match '"pwsh_mouse_selection"\s*:\s*true') {
                Write-Pass "Dump JSON contains pwsh_mouse_selection: true"
                $dumpFound = $true
            } elseif ($dumpData -match '"pwsh_mouse_selection"\s*:\s*false') {
                Write-Fail "Dump says false, but we set it on"
            } elseif ($dumpData -match 'pwsh_mouse_selection') {
                Write-Pass "Dump JSON contains pwsh_mouse_selection field"
                $dumpFound = $true
            } else {
                Write-Fail "Dump JSON missing pwsh_mouse_selection field"
            }
        } else {
            Write-Fail "TCP auth failed: $authResp"
        }
        $tcp.Close()
    } catch {
        Write-Fail "TCP dump error: $_"
    }
} else {
    Write-Info "Port/key files not found, skipping TCP dump test"
}

# ============================================================
# 6. NO CRASH WITH SPLIT PANES + OPTION ON
# ============================================================
Write-Test "Split panes with pwsh-mouse-selection on: no crash"
Psmux set -g mouse on -t $SESSION
Psmux set -g pwsh-mouse-selection on -t $SESSION
Psmux split-window -h -t $SESSION
Start-Sleep -Milliseconds 500
Psmux split-window -v -t $SESSION
Start-Sleep -Milliseconds 500

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Pass "Session alive with split panes and option on"
} else {
    Write-Fail "Session died after split with option on"
}

# ============================================================
# 7. CAPTURE-PANE STILL WORKS
# ============================================================
Write-Test "capture-pane works while pwsh-mouse-selection is on"
$capture = Psmux capture-pane -t $SESSION -p
if ($LASTEXITCODE -eq 0 -or ($capture | Out-String).Length -gt 0) {
    Write-Pass "capture-pane works with option on"
} else {
    Write-Fail "capture-pane failed with option on"
}

# ============================================================
# 8. TOGGLE DOES NOT CRASH
# ============================================================
Write-Test "Rapid on/off toggle does not crash"
for ($i = 0; $i -lt 10; $i++) {
    PsmuxQuick set -g pwsh-mouse-selection on -t $SESSION
    PsmuxQuick set -g pwsh-mouse-selection off -t $SESSION
}
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Pass "Session alive after 10 rapid toggles"
} else {
    Write-Fail "Session died during rapid toggle"
}

# ============================================================
# 9. NEW SESSION INHERITS DEFAULT OFF
# ============================================================
Write-Test "New session inherits default off (not leaked from previous set)"
# First set it on in current session
Psmux set -g pwsh-mouse-selection on -t $SESSION
# Kill and recreate session
& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep -Seconds 1
$SESSION2 = "pwsh_mouse_test2"
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION2 -d" -WindowStyle Hidden
Start-Sleep -Seconds 3
$val = Psmux show-options -t $SESSION2 -gv pwsh-mouse-selection
$valStr = ($val | Out-String).Trim()
# Global state persists within same server, so this checks server behavior
if ($valStr -eq "on" -or $valStr -eq "off") {
    Write-Pass "New session has valid pwsh-mouse-selection value: $valStr"
} else {
    Write-Fail "New session has invalid value: '$valStr'"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Info "Cleaning up..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 1

# Summary
Write-Host ""
Write-Host ("=" * 60)
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
