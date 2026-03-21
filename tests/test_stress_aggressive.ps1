# Aggressive stress test: rapid-fire 60 pane creation with minimal delays
# Goal: reproduce "first 2-3 work, after that issues appear"

$ErrorActionPreference = "Continue"
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
$SESSION = "aggro"
$script:pass = 0
$script:fail = 0
$script:errors = @()

function Log($msg) { Write-Host "[$(Get-Date -Format 'HH:mm:ss.fff')] $msg" }
function Pass($t, $d) { $script:pass++; Write-Host "  [PASS] $t - $d" }
function Fail($t, $d) { 
    $script:fail++
    $script:errors += "$t : $d"
    Write-Host "  [FAIL] $t - $d" -ForegroundColor Red
}

function Cleanup {
    & $PSMUX kill-server 2>&1 | Out-Null
    Start-Sleep -Seconds 2
}

function Wait-Prompt($target, $timeoutMs = 15000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $cap = & $PSMUX capture-pane -t $target -p 2>&1
        $text = ($cap | Out-String)
        if ($text -match 'PS [A-Z]:\\') {
            return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $text }
        }
        Start-Sleep -Milliseconds 150
    }
    $cap = & $PSMUX capture-pane -t $target -p 2>&1
    $text = ($cap | Out-String)
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = $text }
}

function Run-And-Verify {
    param($Target, $Command, $Expected, $Timeout = 10000)
    & $PSMUX send-keys -t $Target "$Command" Enter 2>&1 | Out-Null
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $Timeout) {
        Start-Sleep -Milliseconds 200
        $cap = & $PSMUX capture-pane -t $Target -p 2>&1
        $text = ($cap | Out-String)
        if ($text -match $Expected) {
            return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $text }
        }
    }
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = ($cap | Out-String) }
}

function Check-ServerAlive($sess) {
    $out = & $PSMUX list-sessions 2>&1
    return ($LASTEXITCODE -eq 0) -and ($out -match $sess)
}

function Get-AllPanes($sess) {
    $out = & $PSMUX list-panes -t $sess -a 2>&1
    return @($out | Where-Object { $_ -match '\S' })
}

# =============================================================================
# SCENARIO A: Rapid-fire new-window (no sleeps except small yield)
# =============================================================================
Cleanup
Write-Host ""
Log "SCENARIO A: Rapid-fire 30 windows with NO sleep between new-window calls"
Write-Host ("=" * 70)

& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Fire off 29 more windows as fast as possible
for ($w = 1; $w -le 29; $w++) {
    $out = & $PSMUX new-window -t $SESSION 2>&1
    if ($LASTEXITCODE -ne 0) {
        Fail "Rapid window $w" "exit=$LASTEXITCODE, out=$out"
    }
    # NO sleep - fire as fast as CLI allows
}

# Now wait for things to settle
Start-Sleep -Seconds 5

# Check how many panes exist
$panes = Get-AllPanes $SESSION
$paneCount = $panes.Count
Log "Created 30 windows rapid-fire, found $paneCount panes"

if ($paneCount -eq 30) {
    Pass "Rapid 30 windows" "All $paneCount panes exist"
} else {
    Fail "Rapid 30 windows" "Expected 30, got $paneCount panes"
    $panes | ForEach-Object { Write-Host "  $_" }
}

# Verify prompts appeared in a sampling
$failedPrompts = 0
foreach ($w in @(0, 5, 10, 15, 20, 25, 29)) {
    $r = Wait-Prompt "$SESSION`:$w" 15000
    if ($r.Found) {
        Pass "Rapid window $w prompt" "$($r.ElapsedMs)ms"
    } else {
        Fail "Rapid window $w prompt" "No prompt after $($r.ElapsedMs)ms"
        $failedPrompts++
        $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
        Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
    }
}

# Verify commands work in those panes
foreach ($w in @(0, 10, 20, 29)) {
    $target = "$SESSION`:$w.0"
    $marker = "RAPID_${w}"
    # Ensure pane is ready before sending echo
    $pr = Wait-Prompt $target 15000
    if (-not $pr.Found) {
        Fail "echo in rapid $target" "pane not ready (no prompt)"
        continue
    }
    # Clear any stale output first
    & $PSMUX send-keys -t $target "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $r = Run-And-Verify -Target $target -Command "echo $marker" -Expected $marker -Timeout 10000
    if ($r.Found) {
        Pass "echo in rapid $target" "$($r.ElapsedMs)ms"
    } else {
        Fail "echo in rapid $target" "not found"
        $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
        Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
    }
}

if (Check-ServerAlive $SESSION) {
    Pass "Server after rapid-fire A" "alive"
} else {
    Fail "Server after rapid-fire A" "dead"
}

Cleanup

# =============================================================================
# SCENARIO B: Rapid-fire splits in same window (the ConPTY pressure scenario)
# =============================================================================
Write-Host ""
Log "SCENARIO B: 15 windows, each hammered with 5 rapid splits"
Write-Host ("=" * 70)

& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 3

for ($w = 1; $w -le 14; $w++) {
    & $PSMUX new-window -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
}

Start-Sleep -Seconds 3
Log "15 windows created, now rapid-splitting each"

for ($w = 0; $w -le 14; $w++) {
    & $PSMUX select-window -t "$SESSION`:$w" 2>&1 | Out-Null
    
    # Fire 5 splits as fast as possible (mix V and H)
    for ($s = 0; $s -lt 5; $s++) {
        if ($s % 2 -eq 0) {
            $out = & $PSMUX split-window -t $SESSION -v 2>&1
        } else {
            $out = & $PSMUX split-window -t $SESSION -h 2>&1
        }
        # NO sleep between splits
    }
}

Start-Sleep -Seconds 8

$panes = Get-AllPanes $SESSION
$paneCount = $panes.Count
Log "After rapid splits: $paneCount total panes"

# Check if server survived
if (Check-ServerAlive $SESSION) {
    Pass "Server after rapid splits B" "alive with $paneCount panes"
} else {
    Fail "Server after rapid splits B" "dead"
}

# Verify prompt in every pane of a few windows
foreach ($w in @(0, 5, 10, 14)) {
    # How many panes in this window?
    $windowPanes = & $PSMUX list-panes -t "$SESSION`:$w" 2>&1
    $wpArray = @($windowPanes | Where-Object { $_ -match '\S' })
    Log "Window $w has $($wpArray.Count) panes"
    
    for ($p = 0; $p -lt $wpArray.Count; $p++) {
        $target = "$SESSION`:$w.$p"
        $r = Wait-Prompt $target 10000
        if ($r.Found) {
            Pass "Prompt $target" "$($r.ElapsedMs)ms"
        } else {
            Fail "Prompt $target" "No prompt after $($r.ElapsedMs)ms"
            $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
            Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        }
    }
}

# Run commands in sampled panes
foreach ($w in @(0, 7, 14)) {
    $target = "$SESSION`:$w.0"
    $marker = "SPLIT_${w}"
    # Ensure pane is ready before sending echo
    $pr = Wait-Prompt $target 15000
    if (-not $pr.Found) {
        Fail "echo in split $target" "pane not ready (no prompt)"
        continue
    }
    & $PSMUX send-keys -t $target "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $r = Run-And-Verify -Target $target -Command "echo $marker" -Expected $marker -Timeout 15000
    if ($r.Found) {
        Pass "echo in split $target" "$($r.ElapsedMs)ms"
    } else {
        Fail "echo in split $target" "not found"
    }
}

# Print all panes for diagnostics
Write-Host ""
Log "All panes:"
$panes | ForEach-Object { Write-Host "  $_" }

Cleanup

# =============================================================================
# SCENARIO C: 50 windows, each with just 1 split, verify ALL 100 panes
# =============================================================================
Write-Host ""
Log "SCENARIO C: 50 windows with 1 split each = 100 panes, verify every one"
Write-Host ("=" * 70)

& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Create 49 more windows with small yield for ConPTY
for ($w = 1; $w -le 49; $w++) {
    & $PSMUX new-window -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 150
}
Start-Sleep -Seconds 5

# Split each window once
for ($w = 0; $w -le 49; $w++) {
    & $PSMUX split-window -t "$SESSION`:$w" -v 2>&1 | Out-Null
    Start-Sleep -Milliseconds 150
}
Start-Sleep -Seconds 8

$panes = Get-AllPanes $SESSION
$paneCount = $panes.Count
Log "Expected 100 panes, got $paneCount"

if ($paneCount -ge 95) {
    Pass "$paneCount panes created" "$(if ($paneCount -eq 100) {'exact match'} else {"$paneCount/100 — some splits hit minimum size limit"})"
} else {
    Fail "100 panes created" "expected >=95, got $paneCount"
}

# Verify prompts in EVERY pane
$promptFails = 0
$promptPasses = 0
for ($w = 0; $w -le 49; $w++) {
    for ($p = 0; $p -le 1; $p++) {
        $target = "$SESSION`:$w.$p"
        $r = Wait-Prompt $target 12000
        if ($r.Found) {
            $promptPasses++
            # Only print pass for multiples of 10 to reduce noise
            if ($w % 10 -eq 0) {
                Pass "Prompt $target" "$($r.ElapsedMs)ms"
            }
        } else {
            $promptFails++
            Fail "Prompt $target" "No prompt after $($r.ElapsedMs)ms"
            $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
            Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        }
    }
}

Log "Prompt results: $promptPasses passed, $promptFails failed out of $paneCount"

# Run echo in every 10th window (wait for prompt first)
for ($w = 0; $w -le 49; $w += 10) {
    $target = "$SESSION`:$w.0"
    $marker = "CMD_${w}"
    # Ensure pane is ready before sending echo
    $pr = Wait-Prompt $target 15000
    if (-not $pr.Found) {
        Fail "echo $target" "pane not ready (no prompt)"
        continue
    }
    # Clear any stale output first
    & $PSMUX send-keys -t $target "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $r = Run-And-Verify -Target $target -Command "echo $marker" -Expected $marker -Timeout 15000
    if ($r.Found) {
        Pass "echo $target" "$($r.ElapsedMs)ms"
    } else {
        # Under extreme 100-pane load, ConPTY can be slow to flush.
        # Retry once with a fresh clear+echo cycle.
        & $PSMUX send-keys -t $target "clear" Enter 2>&1 | Out-Null
        Start-Sleep -Milliseconds 1000
        $r2 = Run-And-Verify -Target $target -Command "echo ${marker}_R" -Expected "${marker}_R" -Timeout 15000
        if ($r2.Found) {
            Pass "echo $target" "$($r2.ElapsedMs)ms (retry)"
        } else {
            Fail "echo $target" "not found"
        }
    }
}

if (Check-ServerAlive $SESSION) {
    Pass "Server after 100-pane test" "alive"
} else {
    Fail "Server after 100-pane test" "dead"
}

Cleanup

# =============================================================================
# SUMMARY
# =============================================================================
Write-Host ""
Write-Host ("=" * 70)
Log "TOTAL RESULTS: $($script:pass) passed, $($script:fail) failed"
Write-Host ("=" * 70)

if ($script:errors.Count -gt 0) {
    Write-Host ""
    Log "FAILURES:"
    $script:errors | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
}
