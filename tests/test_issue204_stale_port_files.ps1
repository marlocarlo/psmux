# Issue #204: Stale .port files left behind when pane command fails to spawn
# Tests that port/key files are cleaned up when the initial pane command
# cannot be spawned, preventing ghost sessions in "psmux ls".

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
        Start-Sleep -Milliseconds 300
        Remove-Item "$psmuxDir\$s.port" -Force -EA SilentlyContinue
        Remove-Item "$psmuxDir\$s.key" -Force -EA SilentlyContinue
    }
}

Write-Host "`n=== Issue #204 Tests: Stale Port File Cleanup ===" -ForegroundColor Cyan

function Start-Process-Timeout {
    param([string]$FilePath, [string[]]$ArgumentList, [int]$TimeoutSec = 10)
    $proc = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -WindowStyle Hidden -PassThru
    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
    }
}

# === TEST 1: Nonexistent binary leaves no stale port file ===
Write-Host "`n[Test 1] Nonexistent binary: no stale port file after server exits" -ForegroundColor Yellow
$SESSION1 = "stale_test_204_1"
Cleanup @($SESSION1)
Start-Sleep -Milliseconds 500

# Launch with a nonexistent binary
Start-Process-Timeout -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION1,"C:\nonexistent_path_204\binary.exe" -TimeoutSec 10
Start-Sleep -Seconds 3

# Check WITHOUT running any other psmux command (to avoid cleanup_stale_port_files)
$portExists = Test-Path "$psmuxDir\$SESSION1.port"
$keyExists = Test-Path "$psmuxDir\$SESSION1.key"

if (-not $portExists) { Write-Pass "No stale .port file for nonexistent binary" }
else {
    # Port file exists; check if server is actually alive
    $port = (Get-Content "$psmuxDir\$SESSION1.port" -Raw).Trim()
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1",[int]$port)
        $tcp.Close()
        Write-Pass "Server is alive (session running with dead pane, exit-empty will clean up)"
    } catch {
        Write-Fail "Stale .port file left behind (server dead, port file orphaned)"
    }
}

if (-not $keyExists) { Write-Pass "No stale .key file for nonexistent binary" }
else { Write-Fail "Stale .key file left behind" }


# === TEST 2: psmux ls does not show ghost session ===
Write-Host "`n[Test 2] psmux ls does not list ghost session" -ForegroundColor Yellow
$SESSION2 = "stale_test_204_2"
Cleanup @($SESSION2)
Start-Sleep -Milliseconds 500

Start-Process-Timeout -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION2,"C:\nonexistent_path_204\ghost.exe" -TimeoutSec 10
Start-Sleep -Seconds 3

$lsOutput = & $PSMUX ls 2>&1 | Out-String
if ($lsOutput -notmatch $SESSION2) { Write-Pass "Ghost session not listed in 'psmux ls'" }
else { Write-Fail "Ghost session '$SESSION2' appears in 'psmux ls': $($lsOutput.Trim())" }


# === TEST 3: has-session returns exit 1 for failed session ===
Write-Host "`n[Test 3] has-session returns exit 1 for failed spawn" -ForegroundColor Yellow
$SESSION3 = "stale_test_204_3"
Cleanup @($SESSION3)
Start-Sleep -Milliseconds 500

Start-Process-Timeout -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION3,"C:\nonexistent_path_204\check.exe" -TimeoutSec 10
Start-Sleep -Seconds 3

& $PSMUX has-session -t $SESSION3 2>$null
if ($LASTEXITCODE -ne 0) { Write-Pass "has-session correctly reports session does not exist" }
else { Write-Fail "has-session reports session exists (should not)" }


# === TEST 4: Normal session still works (no regression) ===
Write-Host "`n[Test 4] Normal session creation still works" -ForegroundColor Yellow
$SESSION4 = "stale_test_204_4"
Cleanup @($SESSION4)
Start-Sleep -Milliseconds 500

& $PSMUX new-session -d -s $SESSION4
Start-Sleep -Seconds 3

& $PSMUX has-session -t $SESSION4 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "Normal session created successfully" }
else { Write-Fail "Normal session creation failed (regression!)" }

$portFile4 = "$psmuxDir\$SESSION4.port"
if (Test-Path $portFile4) {
    $port4 = (Get-Content $portFile4 -Raw).Trim()
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1",[int]$port4)
        $tcp.Close()
        Write-Pass "Normal session server is reachable via TCP"
    } catch {
        Write-Fail "Normal session server not reachable"
    }
} else {
    Write-Fail "Normal session port file missing"
}


# === TEST 5: Consistency between ls and has-session ===
Write-Host "`n[Test 5] ls and has-session are consistent for failed spawn" -ForegroundColor Yellow
$SESSION5 = "stale_test_204_5"
Cleanup @($SESSION5)
Start-Sleep -Milliseconds 500

Start-Process-Timeout -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION5,"C:\nonexistent_path_204\consist.exe" -TimeoutSec 10
Start-Sleep -Seconds 3

$lsOutput5 = & $PSMUX ls 2>&1 | Out-String
$lsShows = $lsOutput5 -match $SESSION5

& $PSMUX has-session -t $SESSION5 2>$null
$hasSession = ($LASTEXITCODE -eq 0)

if ($lsShows -eq $hasSession) { Write-Pass "ls and has-session are consistent (both say: $(if ($lsShows) { 'exists' } else { 'not found' }))" }
else { Write-Fail "INCONSISTENCY: ls says $(if ($lsShows) { 'exists' } else { 'not found' }), has-session says $(if ($hasSession) { 'exists' } else { 'not found' })" }


# === TEST 6: Race window test (port file should not appear between spawn and ls) ===
Write-Host "`n[Test 6] No ghost session visible at any point after failed spawn" -ForegroundColor Yellow
$SESSION6 = "stale_test_204_6"
Cleanup @($SESSION6)
Start-Sleep -Milliseconds 500

Start-Process -FilePath $PSMUX -ArgumentList "new-session","-d","-s",$SESSION6,"C:\nonexistent_path_204\race.exe" -WindowStyle Hidden

# Poll aggressively for the port file
$portSeen = $false
$serverDead = $false
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 100
    if (Test-Path "$psmuxDir\$SESSION6.port") {
        $portSeen = $true
        $p = (Get-Content "$psmuxDir\$SESSION6.port" -Raw -EA SilentlyContinue)
        if ($p) {
            $p = $p.Trim()
            try { $t = [System.Net.Sockets.TcpClient]::new("127.0.0.1",[int]$p); $t.Close() } catch { $serverDead = $true }
        }
        if ($serverDead) { break }
    }
}
Start-Sleep -Seconds 2
$finalExists = Test-Path "$psmuxDir\$SESSION6.port"

if (-not $finalExists) { Write-Pass "Port file cleaned up (no stale file remains)" }
else { Write-Fail "Stale port file persists after failed spawn" }

if ($portSeen -and $serverDead) {
    Write-Fail "Port file was visible while server was dead (race window detected)"
} elseif ($portSeen -and -not $serverDead) {
    Write-Pass "Port file was transient but server was alive during that window (exit-empty handles cleanup)"
} else {
    Write-Pass "Port file was never visible (immediately cleaned up on spawn failure)"
}


# === TEARDOWN ===
Cleanup @($SESSION1, $SESSION2, $SESSION3, $SESSION4, $SESSION5, $SESSION6)

Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed: $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
exit $script:TestsFailed
