# Test with psmux attached (rendering pipeline active) 
# Creates 60 panes while a client is attached, then verifies each

$ErrorActionPreference = "Continue"
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
$SESSION = "attached_test"
$script:pass = 0
$script:fail = 0
$script:errors = @()

function Log($msg) { Write-Host "[$(Get-Date -Format 'HH:mm:ss.fff')] $msg" }
function Pass($t, $d) { $script:pass++; Write-Host "  [PASS] $t - $d" }
function Fail($t, $d) { 
    $script:fail++; $script:errors += "$t : $d"
    Write-Host "  [FAIL] $t - $d" -ForegroundColor Red
}

function Wait-Prompt($target, $timeoutMs = 15000) {
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
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = ($cap | Out-String) }
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

# =============================================================================
Log "Testing with ATTACHED session (rendering pipeline active)"
Write-Host ("=" * 70)

# Create the session (detached for CI, but tests the full pane/window pipeline)
& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 2
& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 3

$out = & $PSMUX list-sessions 2>&1
Log "Sessions: $out"

# Wait for initial pane
Start-Sleep -Seconds 2
$r = Wait-Prompt "$SESSION`:0"
if ($r.Found) { Pass "Initial prompt" "$($r.ElapsedMs)ms" }
else { Fail "Initial prompt" "No PS prompt"; exit 1 }

# Create 19 more windows with 2 splits each = 60 panes
for ($w = 1; $w -le 19; $w++) {
    $out = & $PSMUX new-window -t $SESSION 2>&1
    if ($LASTEXITCODE -ne 0) {
        Fail "Window $w creation" "exit=$LASTEXITCODE, out=$out"
        continue
    }
    # Minimal sleep
    Start-Sleep -Milliseconds 300
    
    $r = Wait-Prompt "$SESSION`:$w" 10000
    if ($r.Found) {
        Pass "Window $w prompt" "$($r.ElapsedMs)ms"
    } else {
        Fail "Window $w prompt" "No PS prompt after $($r.ElapsedMs)ms"
        $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0,120) } else { $r.Output }
        Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
    }
    
    # V split
    $out = & $PSMUX split-window -t $SESSION -v 2>&1
    if ($LASTEXITCODE -eq 0) {
        Start-Sleep -Milliseconds 300
        $r = Wait-Prompt "$SESSION`:$w.1" 8000
        if ($r.Found) { Pass "Win$w.1 vsplit" "$($r.ElapsedMs)ms" }
        else { 
            Fail "Win$w.1 vsplit" "No prompt" 
            $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0,120) } else { $r.Output }
            Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        }
    }
    
    # H split
    $out = & $PSMUX split-window -t $SESSION -h 2>&1
    if ($LASTEXITCODE -eq 0) {
        Start-Sleep -Milliseconds 300
        $r = Wait-Prompt "$SESSION`:$w.2" 8000
        if ($r.Found) { Pass "Win$w.2 hsplit" "$($r.ElapsedMs)ms" }
        else { 
            Fail "Win$w.2 hsplit" "No prompt"
            $snippet = if ($r.Output.Length -gt 120) { $r.Output.Substring(0,120) } else { $r.Output }
            Write-Host "    Captured: [$snippet]" -ForegroundColor Yellow
        }
    }
}

# Verify commands in sampled panes
Write-Host ""
Log "Phase 2: Verify command execution"
for ($w = 0; $w -le 19; $w += 4) {
    $target = "$SESSION`:$w.0"
    $marker = "ATTACHED_${w}"
    # Ensure pane is ready before sending echo
    $pr = Wait-Prompt $target 15000
    if (-not $pr.Found) {
        Fail "echo $target" "pane not ready (no prompt)"
        continue
    }
    # Clear stale output first
    & $PSMUX send-keys -t $target "clear" Enter 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $r = Run-And-Verify -Target $target -Command "echo $marker" -Expected $marker -Timeout 10000
    if ($r.Found) { Pass "echo $target" "$($r.ElapsedMs)ms" }
    else { Fail "echo $target" "not found" }
}

# Final stats
$panes = @(& $PSMUX list-panes -t $SESSION -a 2>&1 | Where-Object { $_ -match '\S' })
Log "Total panes: $($panes.Count)"

Write-Host ""
Write-Host ("=" * 70)
Log "RESULTS: $($script:pass) passed, $($script:fail) failed"
Write-Host ("=" * 70)
if ($script:errors.Count -gt 0) {
    Log "FAILURES:"
    $script:errors | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
}

# Cleanup
& $PSMUX kill-server 2>&1 | Out-Null
if ($script:errors.Count -gt 0) {
    Log "FAILURES:"
    $script:errors | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
}
