#!/usr/bin/env pwsh
# =============================================================================
# HARDCORE STRESS TEST — psmux pane spawn reliability & performance
# Verifies that `PS C:\` prompt actually appears (not just blinking cursor)
# =============================================================================
param(
    [int]$WindowCount   = 15,     # How many windows to blast open
    [int]$SplitCount    = 5,      # How many splits in one window
    [int]$PromptTimeout = 30000,  # Max ms to wait for PS prompt per pane
    [int]$BurstDelay    = 50      # ms between rapid-fire commands
)

$ErrorActionPreference = 'Continue'
$PSMUX = Join-Path $PSScriptRoot "..\target\release\tmux.exe"
if (-not (Test-Path $PSMUX)) { Write-Error "tmux.exe not found at $PSMUX"; exit 1 }

$totalTests  = 0
$passedTests = 0
$failedTests = 0
$failures    = @()

function Log { param([string]$msg) Write-Host "[$(Get-Date -Format 'HH:mm:ss.fff')] $msg" }
function Pass { param([string]$name, [string]$detail)
    $script:totalTests++; $script:passedTests++
    Write-Host "  [PASS] $name - $detail" -ForegroundColor Green
}
function Fail { param([string]$name, [string]$detail)
    $script:totalTests++; $script:failedTests++
    $script:failures += "$name : $detail"
    Write-Host "  [FAIL] $name - $detail" -ForegroundColor Red
}

# Kill all psmux instances first
function Cleanup {
    try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
    Start-Sleep -Seconds 1
    try { Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
    try { Get-Process tmux -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
    try { Get-Process pmux -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
    Start-Sleep -Milliseconds 500
}

# Wait for PS prompt on a specific target pane. Returns hashtable with Found/ElapsedMs/Output.
function Wait-Prompt {
    param(
        [string]$Target,
        [int]$Timeout = $PromptTimeout
    )
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $Timeout) {
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            # Check for ACTUAL PS prompt — must have drive letter like PS C:\
            if ($cap -match "PS [A-Z]:\\") {
                return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $cap }
            }
        } catch {}
        Start-Sleep -Milliseconds 100
    }
    # Final capture for diagnostics
    $finalCap = ""
    try { $finalCap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String } catch {}
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = $finalCap }
}

# =============================================================================
# TEST 1: Rapid Window Creation — blast $WindowCount windows and verify ALL have PS prompts
# =============================================================================
Log "=== TEST 1: Rapid $WindowCount-Window Blast ==="
Cleanup

# Start a new session
$sw = [System.Diagnostics.Stopwatch]::StartNew()
& $PSMUX new-session -d -s stress1 2>&1 | Out-Null
$sessionTime = $sw.ElapsedMilliseconds
Log "  Session created in ${sessionTime}ms"

# Wait for initial window prompt
$r = Wait-Prompt "stress1:0"
if ($r.Found) { Pass "Session prompt" "${($r.ElapsedMs)}ms" }
else { Fail "Session prompt" "No PS prompt after $($r.ElapsedMs)ms. Got: $($r.Output.Substring(0, [Math]::Min(200, $r.Output.Length)))" }

# Blast open windows rapidly
$windowTimes = @()
for ($i = 1; $i -le $WindowCount; $i++) {
    $wSw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-window -t stress1 2>&1 | Out-Null
    $windowTimes += $wSw.ElapsedMilliseconds
    if ($BurstDelay -gt 0) { Start-Sleep -Milliseconds $BurstDelay }
}
Log "  All $WindowCount new-window commands sent (avg cmd time: $([Math]::Round(($windowTimes | Measure-Object -Average).Average, 1))ms)"

# Now verify EVERY window has a PS prompt
$aliveCount = 0
$deadCount = 0
$promptTimes = @()

for ($i = 0; $i -le $WindowCount; $i++) {
    $r = Wait-Prompt "stress1:$i"
    if ($r.Found) {
        $aliveCount++
        $promptTimes += $r.ElapsedMs
    } else {
        $deadCount++
        Fail "Window $i prompt" "No PS prompt after ${PromptTimeout}ms. Content: $($r.Output.Substring(0, [Math]::Min(200, $r.Output.Length)))"
    }
}

$total = $WindowCount + 1
if ($aliveCount -eq $total) {
    $avg = [Math]::Round(($promptTimes | Measure-Object -Average).Average, 1)
    $max = ($promptTimes | Measure-Object -Maximum).Maximum
    Pass "All $total windows alive" "avg prompt: ${avg}ms, max: ${max}ms"
} else {
    Fail "Window health" "$deadCount of $total windows DEAD (no PS prompt)"
}
Log "  Alive: $aliveCount / $total | Dead: $deadCount"

# =============================================================================
# TEST 2: Rapid Splits — open $SplitCount splits in one window, verify ALL
# =============================================================================
Log ""
Log "=== TEST 2: Rapid $SplitCount Splits in One Window ==="
Cleanup

& $PSMUX new-session -d -s stress2 2>&1 | Out-Null
$r = Wait-Prompt "stress2:0"
if (-not $r.Found) { Fail "Split base prompt" "Initial window never loaded"; }

# Alternate between vertical and horizontal splits  
$splitTimes = @()
for ($i = 0; $i -lt $SplitCount; $i++) {
    $flag = if ($i % 2 -eq 0) { "-v" } else { "-h" }
    $sSw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX split-window $flag -t stress2 2>&1 | Out-Null
    $splitTimes += $sSw.ElapsedMilliseconds
    if ($BurstDelay -gt 0) { Start-Sleep -Milliseconds $BurstDelay }
}
Log "  All $SplitCount split commands sent (avg cmd time: $([Math]::Round(($splitTimes | Measure-Object -Average).Average, 1))ms)"

# Wait for panes to load
Start-Sleep -Seconds 3

# We expect 1 initial + $SplitCount splits = $SplitCount+1 panes
$totalPanes = $SplitCount + 1
Log "  Expecting $totalPanes panes in window"

$splitAlive = 0
$splitDead = 0
for ($i = 0; $i -lt $totalPanes; $i++) {
    $r = Wait-Prompt "stress2:0.$i"
    if ($r.Found) {
        $splitAlive++
    } else {
        # Pane might not exist (split may have failed if pane too small)
        try {
            $check = & $PSMUX capture-pane -t "stress2:0.$i" -p 2>&1 | Out-String
            if ($check -and $check.Trim().Length -gt 0) {
                $splitDead++
                Fail "Pane $i prompt" "No PS prompt after ${PromptTimeout}ms in split pane"
            }
        } catch {
            # Pane doesn't exist, not a failure
        }
    }
}

if ($splitAlive -gt 0 -and $splitDead -eq 0) {
    Pass "All split panes alive" "$splitAlive panes have PS prompts"
} elseif ($splitAlive -gt 0) {
    Fail "Split health" "$splitDead panes DEAD out of $($splitAlive + $splitDead) total"
} else {
    Fail "Split health" "No panes found at all"
}

# =============================================================================
# TEST 3: Mixed barrage — windows + splits interleaved
# =============================================================================
Log ""
Log "=== TEST 3: Mixed Barrage (Windows + Splits) ==="
Cleanup

& $PSMUX new-session -d -s stress3 2>&1 | Out-Null
$r = Wait-Prompt "stress3:0"

# Create 5 windows, each with 2 splits = 15 panes total
$barrageStart = [System.Diagnostics.Stopwatch]::StartNew()
for ($w = 1; $w -le 5; $w++) {
    & $PSMUX new-window -t stress3 2>&1 | Out-Null
    Start-Sleep -Milliseconds $BurstDelay
    & $PSMUX split-window -v -t stress3 2>&1 | Out-Null
    Start-Sleep -Milliseconds $BurstDelay
    & $PSMUX split-window -h -t stress3 2>&1 | Out-Null
    Start-Sleep -Milliseconds $BurstDelay
}
$barrageMs = $barrageStart.ElapsedMilliseconds
Log "  Barrage complete in ${barrageMs}ms (6 windows × 3 panes each = 18 panes)"

# Verify all windows
$bAlive = 0
$bDead = 0
for ($w = 0; $w -le 5; $w++) {
    # Check up to 3 panes per window
    for ($p = 0; $p -lt 3; $p++) {
        $r = Wait-Prompt "stress3:$w.$p"
        if ($r.Found) { $bAlive++ }
        else {
            # Some panes might not exist (window 0 has no splits)
            # Only count as dead if capture-pane returns something (pane exists)
            try {
                $check = & $PSMUX capture-pane -t "stress3:$w.$p" -p 2>&1 | Out-String
                if ($check -and $check.Length -gt 0) {
                    $bDead++
                    Fail "Barrage w${w}p${p}" "Pane exists but no PS prompt"
                }
            } catch {}
        }
    }
}
$bTotal = $bAlive + $bDead
if ($bDead -eq 0 -and $bAlive -gt 0) {
    Pass "Barrage test" "$bAlive panes all have PS prompts (took ${barrageMs}ms)"
} else {
    Fail "Barrage test" "$bDead of $bTotal panes DEAD"
}

# =============================================================================
# TEST 4: Sustained load — keep creating windows while checking old ones
# =============================================================================
Log ""
Log "=== TEST 4: Sustained Load (create while checking) ==="
Cleanup

& $PSMUX new-session -d -s stress4 2>&1 | Out-Null
$r = Wait-Prompt "stress4:0"

$sustainedAlive = 0
$sustainedDead = 0
$sustainedTimes = @()

for ($i = 1; $i -le 10; $i++) {
    # Create a window
    $cSw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-window -t stress4 2>&1 | Out-Null
    
    # Immediately check the PREVIOUS window while the new one is loading
    if ($i -gt 1) {
        $prevCheck = Wait-Prompt "stress4:$($i-1)" -Timeout 5000
        if ($prevCheck.Found) { 
            # good, prev window is healthy 
        }
    }
    
    # Now check the just-created window
    $r = Wait-Prompt "stress4:$i"
    $elapsed = $cSw.ElapsedMilliseconds
    if ($r.Found) {
        $sustainedAlive++
        $sustainedTimes += $r.ElapsedMs
    } else {
        $sustainedDead++
        Fail "Sustained w$i" "No PS prompt after ${PromptTimeout}ms"
    }
}

if ($sustainedDead -eq 0) {
    $avg = [Math]::Round(($sustainedTimes | Measure-Object -Average).Average, 1)
    $max = ($sustainedTimes | Measure-Object -Maximum).Maximum
    Pass "Sustained 10 windows" "All alive, avg: ${avg}ms, max: ${max}ms"
} else {
    Fail "Sustained load" "$sustainedDead of 10 windows DEAD"
}

# =============================================================================
# TEST 5: Rapid kill + recreate — test cleanup doesn't leak resources
# =============================================================================
Log ""
Log "=== TEST 5: Kill + Recreate Cycle ==="
Cleanup

& $PSMUX new-session -d -s stress5 2>&1 | Out-Null
Wait-Prompt "stress5:0" | Out-Null

$cycleOk = 0
$cycleFail = 0
for ($i = 0; $i -lt 10; $i++) {
    # Create and immediately kill a window
    & $PSMUX new-window -t stress5 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
    & $PSMUX kill-pane -t stress5 2>&1 | Out-Null
    Start-Sleep -Milliseconds 100
}

# After all that churn, create one more and verify it works
& $PSMUX new-window -t stress5 2>&1 | Out-Null
$r = Wait-Prompt "stress5"
if ($r.Found) {
    Pass "Kill+Recreate cycle" "Final window alive after 10 kill cycles (${($r.ElapsedMs)}ms)"
    $cycleOk++
} else {
    Fail "Kill+Recreate cycle" "Final window DEAD after 10 kill cycles"
    $cycleFail++
}

# Also verify session still works with a split
& $PSMUX split-window -v -t stress5 2>&1 | Out-Null
Start-Sleep -Seconds 1
$splitR = Wait-Prompt "stress5"
if ($splitR.Found) { Pass "Post-cycle split" "Split works after churn" }
else { Fail "Post-cycle split" "Split DEAD after churn" }

# =============================================================================
# TEST 6: Multiple sessions simultaneously
# =============================================================================
Log ""
Log "=== TEST 6: Multiple Sessions ($([Math]::Min(5, $WindowCount)) concurrent) ==="
Cleanup

$sessCount = [Math]::Min(5, $WindowCount)
for ($i = 0; $i -lt $sessCount; $i++) {
    & $PSMUX new-session -d -s "msess$i" 2>&1 | Out-Null
    Start-Sleep -Milliseconds $BurstDelay
}

$sessAlive = 0
$sessDead = 0
for ($i = 0; $i -lt $sessCount; $i++) {
    $r = Wait-Prompt "msess${i}:0"
    if ($r.Found) { $sessAlive++ }
    else {
        $sessDead++
        Fail "Session msess$i" "No PS prompt"
    }
}

if ($sessDead -eq 0) {
    Pass "All $sessCount sessions" "All have PS prompts"
} else {
    Fail "Multi-session" "$sessDead of $sessCount sessions DEAD"
}

# Add 3 windows to each session
for ($i = 0; $i -lt $sessCount; $i++) {
    for ($w = 1; $w -le 3; $w++) {
        & $PSMUX new-window -t "msess$i" 2>&1 | Out-Null
        Start-Sleep -Milliseconds $BurstDelay
    }
}

$msAlive = 0
$msDead = 0
for ($i = 0; $i -lt $sessCount; $i++) {
    for ($w = 0; $w -le 3; $w++) {
        $r = Wait-Prompt "msess${i}:$w"
        if ($r.Found) { $msAlive++ }
        else {
            $msDead++
            Fail "msess${i}:w$w" "No PS prompt in multi-session window"
        }
    }
}
$msTotal = $msAlive + $msDead
if ($msDead -eq 0) {
    Pass "Multi-session windows" "All $msTotal windows across $sessCount sessions alive"
} else {
    Fail "Multi-session windows" "$msDead of $msTotal windows DEAD"
}

# =============================================================================
# TEST 7: Maximum pane count in single window
# =============================================================================
Log ""
Log "=== TEST 7: Max Splits in Single Window ==="
Cleanup

& $PSMUX new-session -d -s stress7 2>&1 | Out-Null
Wait-Prompt "stress7:0" | Out-Null

# Try to create many splits until panes are too small
$maxSplits = [Math]::Min(6, $SplitCount)
for ($i = 0; $i -lt $maxSplits; $i++) {
    $flag = if ($i % 2 -eq 0) { "-v" } else { "-h" }
    & $PSMUX split-window $flag -t stress7 2>&1 | Out-Null
    Start-Sleep -Milliseconds $BurstDelay
}

Start-Sleep -Seconds 3

# Expected: 1 initial + maxSplits = maxSplits+1 panes (some may fail if too small)
$expectedPanes = $maxSplits + 1
Log "  Expecting up to $expectedPanes panes"

# Check panes — some may not exist if terminal too small for that many splits
$maxAlive = 0
$maxDead = 0
for ($i = 0; $i -lt $expectedPanes; $i++) {
    $r = Wait-Prompt "stress7:0.$i" -Timeout 15000
    if ($r.Found) { $maxAlive++ }
    else {
        try {
            $check = & $PSMUX capture-pane -t "stress7:0.$i" -p 2>&1 | Out-String
            if ($check -and $check.Trim().Length -gt 0) {
                $maxDead++
            }
        } catch {}
    }
}

if ($maxAlive -gt 0 -and $maxDead -eq 0) {
    Pass "Max splits ($maxAlive panes)" "$maxAlive panes alive"
} elseif ($maxAlive -gt 0) {
    Fail "Max splits" "$maxDead panes DEAD out of $($maxAlive + $maxDead)"
} else {
    Fail "Max splits" "No panes found at all"
}

# =============================================================================
# TEST 8: Latency comparison — psmux overhead vs baseline pwsh
# =============================================================================
Log ""
Log "=== TEST 8: Latency Measurement ==="
Cleanup

# Baseline: how long does pwsh -NoProfile take to show a prompt?
$baselineTimes = @()
for ($i = 0; $i -lt 3; $i++) {
    $bSw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process pwsh -ArgumentList "-NoProfile","-NoLogo","-Command","exit 0" -PassThru -NoNewWindow
    $proc.WaitForExit(10000)
    $baselineTimes += $bSw.ElapsedMilliseconds
}
$baselineAvg = [Math]::Round(($baselineTimes | Measure-Object -Average).Average, 1)
Log "  Baseline pwsh startup: avg ${baselineAvg}ms"

# psmux: measure from new-window to PS prompt appearing
& $PSMUX new-session -d -s latency 2>&1 | Out-Null
Wait-Prompt "latency:0" | Out-Null

$psmuxTimes = @()
for ($i = 1; $i -le 5; $i++) {
    $mSw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-window -t latency 2>&1 | Out-Null
    $r = Wait-Prompt "latency:$i"
    if ($r.Found) {
        $psmuxTimes += $r.ElapsedMs
    }
}

if ($psmuxTimes.Count -gt 0) {
    $psmuxAvg = [Math]::Round(($psmuxTimes | Measure-Object -Average).Average, 1)
    $psmuxMax = ($psmuxTimes | Measure-Object -Maximum).Maximum
    $overhead = [Math]::Round($psmuxAvg - $baselineAvg, 1)
    Log "  psmux new-window to prompt: avg ${psmuxAvg}ms, max ${psmuxMax}ms"
    Log "  Overhead vs baseline: ${overhead}ms"
    if ($overhead -lt 500) {
        Pass "Latency overhead" "Only ${overhead}ms over baseline (acceptable)"
    } else {
        Fail "Latency overhead" "${overhead}ms over baseline (too high!)"
    }
} else {
    Fail "Latency measurement" "No windows got prompts"
}

# =============================================================================
# CLEANUP & SUMMARY
# =============================================================================
Log ""
Log "=== CLEANUP ==="
Cleanup
Log "Done."

Log ""
Log "================================================================"
Log "  STRESS TEST RESULTS"
Log "================================================================"
Log "  Total:  $totalTests"
Log "  Passed: $passedTests" 
Log "  Failed: $failedTests"
if ($failedTests -gt 0) {
    Log ""
    Log "  FAILURES:"
    foreach ($f in $failures) { 
        Write-Host "    - $f" -ForegroundColor Red
    }
}
Log "================================================================"

if ($failedTests -gt 0) { exit 1 } else { exit 0 }
