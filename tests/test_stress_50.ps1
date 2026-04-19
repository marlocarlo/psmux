# Stress test: Create ~50 panes across many windows
# Goal: find what breaks after the first 2-3 windows

$ErrorActionPreference = "Continue"
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
$SESSION = "stress50"
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

function Get-PaneCount($sess) {
    $out = & $PSMUX list-panes -t $sess -a 2>&1
    if ($LASTEXITCODE -ne 0) { return -1 }
    return @($out | Where-Object { $_ -match '\S' }).Count
}

function Wait-Prompt($target, $timeoutMs = 10000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $cap = & $PSMUX capture-pane -t $target -p 2>&1
        $text = ($cap | Out-String)
        if ($text -match 'PS [A-Z]:\\') {
            return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $text }
        }
        Start-Sleep -Milliseconds 200
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
        Start-Sleep -Milliseconds 300
        $cap = & $PSMUX capture-pane -t $Target -p 2>&1
        $text = ($cap | Out-String)
        if ($text -match $Expected) {
            return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $text }
        }
    }
    $cap = & $PSMUX capture-pane -t $Target -p 2>&1
    $text = ($cap | Out-String)
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = $text }
}

function Check-ServerAlive($sess) {
    $out = & $PSMUX list-sessions 2>&1
    return ($LASTEXITCODE -eq 0) -and ($out -match $sess)
}

# =============================================================================
# STRESS TEST: Create 20 windows with 2 splits each = ~60 panes
# =============================================================================
Cleanup

Log "Starting stress test - target: 20 windows x 3 panes = 60 panes"
Write-Host ("=" * 70)

# Create session
& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 3

$r = Wait-Prompt "$SESSION`:0"
if ($r.Found) { Pass "Window 0 initial prompt" "$($r.ElapsedMs)ms" }
else { Fail "Window 0 initial prompt" "No PS prompt"; Cleanup; exit 1 }

$totalPanes = 1
$windowCount = 1

# Create windows and splits
for ($w = 1; $w -le 19; $w++) {
    Log "--- Creating window $w ---"
    
    # Check server is still alive before each window
    if (-not (Check-ServerAlive $SESSION)) {
        Fail "Server alive before window $w" "Server died!"
        break
    }
    
    # Create new window
    $out = & $PSMUX new-window -t $SESSION 2>&1
    $ec = $LASTEXITCODE
    if ($ec -ne 0) {
        Fail "new-window $w" "exit code $ec, output: $out"
        continue
    }
    Start-Sleep -Milliseconds 1500
    
    $r = Wait-Prompt "$SESSION`:$w"
    if ($r.Found) {
        Pass "Window $w prompt" "$($r.ElapsedMs)ms"
        $totalPanes++
        $windowCount++
    } else {
        Fail "Window $w prompt" "No PS prompt after $($r.ElapsedMs)ms"
        # Dump what we captured
        $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
        Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        continue
    }
    
    # Split vertically
    $out = & $PSMUX split-window -t $SESSION -v 2>&1
    $ec = $LASTEXITCODE
    if ($ec -ne 0) {
        Fail "Window $w vsplit" "exit code $ec, output: $out"
    } else {
        Start-Sleep -Milliseconds 800
        # Find the new pane - it should be pane index 1 on current window
        $r = Wait-Prompt "$SESSION`:$w.1" 8000
        if ($r.Found) {
            Pass "Window $w.1 (vsplit) prompt" "$($r.ElapsedMs)ms"
            $totalPanes++
        } else {
            Fail "Window $w.1 (vsplit) prompt" "No PS prompt after $($r.ElapsedMs)ms"
            $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
            Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        }
    }
    
    # Split horizontally
    $out = & $PSMUX split-window -t $SESSION -h 2>&1
    $ec = $LASTEXITCODE
    if ($ec -ne 0) {
        Fail "Window $w hsplit" "exit code $ec, output: $out"
    } else {
        Start-Sleep -Milliseconds 800
        $r = Wait-Prompt "$SESSION`:$w.2" 8000
        if ($r.Found) {
            Pass "Window $w.2 (hsplit) prompt" "$($r.ElapsedMs)ms"
            $totalPanes++
        } else {
            Fail "Window $w.2 (hsplit) prompt" "No PS prompt after $($r.ElapsedMs)ms"
            $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
            Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        }
    }
    
    Log "Progress: $totalPanes panes so far, $windowCount windows"
}

Write-Host ""
Write-Host ("=" * 70)
Log "Phase 1 complete: $totalPanes panes created across $windowCount windows"
Log "Pass=$($script:pass)  Fail=$($script:fail)"
Write-Host ("=" * 70)

# Now verify we can still interact with panes
# Pick a sampling: window 0, window 5, window 10, window 15, window 19
Write-Host ""
Log "Phase 2: Verify command execution in sampled panes"
Write-Host ("=" * 70)

$sampleWindows = @(0, 3, 7, 12, 17)
foreach ($w in $sampleWindows) {
    if ($w -ge $windowCount) { continue }
    
    & $PSMUX select-window -t "$SESSION`:$w" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    
    $target = "$SESSION`:$w.0"
    $marker = "STRESS_${w}_OK"
    
    $r = Run-And-Verify -Target $target -Command "echo $marker" -Expected $marker -Timeout 10000
    if ($r.Found) {
        Pass "echo in $target" "appeared in $($r.ElapsedMs)ms"
    } else {
        Fail "echo in $target" "marker not found after $($r.ElapsedMs)ms"
        $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0, 120) } else { $r.Output }
        Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
    }
}

# Final server check
if (Check-ServerAlive $SESSION) {
    Pass "Server survived stress test" "$totalPanes panes, $windowCount windows"
} else {
    Fail "Server survived stress test" "Server died!"
}

# List all panes at the end for diagnostics
Write-Host ""
Log "Final pane listing:"
$allPanes = & $PSMUX list-panes -t $SESSION -a 2>&1
$allPanes | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host ("=" * 70)
Log "RESULTS: $($script:pass) passed, $($script:fail) failed, $($script:pass + $script:fail) total"
Write-Host ("=" * 70)

if ($script:errors.Count -gt 0) {
    Write-Host ""
    Log "FAILURES:"
    $script:errors | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
}

# Cleanup
Cleanup
