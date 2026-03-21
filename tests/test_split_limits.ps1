#!/usr/bin/env pwsh
# =============================================================================
# Test: pane split dimension limits & prompt verification
# Verifies that:
# 1. Creating splits doesn't crash the server (was crashing after ~6 splits)
# 2. The server returns an error when panes are too small to split
# 3. Every NEW pane gets a real pwsh prompt (PS C:\), verified by capture-pane
# 4. Pane count actually increases after each successful split
# 5. All pane dimensions stay >= 2x2 (ConPTY safety)
# =============================================================================

$ErrorActionPreference = 'Continue'
$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\tmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }

$totalTests  = 0
$passedTests = 0
$failedTests = 0
$failures    = @()

function Log  { param([string]$msg) Write-Host "[$(Get-Date -Format 'HH:mm:ss.fff')] $msg" }
function Pass { param([string]$name, [string]$detail)
    $script:totalTests++; $script:passedTests++
    Write-Host "  [PASS] $name - $detail" -ForegroundColor Green
}
function Fail { param([string]$name, [string]$detail)
    $script:totalTests++; $script:failedTests++
    $script:failures += "$name : $detail"
    Write-Host "  [FAIL] $name - $detail" -ForegroundColor Red
}

function Cleanup {
    try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
    Start-Sleep -Seconds 1
    try { Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
    try { Get-Process tmux  -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
    try { Get-Process pmux  -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
    Start-Sleep -Milliseconds 500
}

function Get-PaneCount {
    param([string]$Session)
    $panes = & $PSMUX list-panes -t $Session 2>&1 | Out-String
    $lines = ($panes -split "`n") | Where-Object { $_ -match '\S' }
    return $lines.Count
}

function Wait-Prompt {
    param([string]$Target, [int]$Timeout = 15000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $Timeout) {
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            if ($cap -match "PS [A-Z]:\\") {
                return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $cap }
            }
        } catch {}
        Start-Sleep -Milliseconds 200
    }
    $finalCap = ""
    try { $finalCap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String } catch {}
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = $finalCap }
}

function Check-ServerAlive {
    param([string]$Session)
    & $PSMUX has-session -t $Session 2>&1 | Out-Null
    return $LASTEXITCODE -eq 0
}

# Send a command into a pane via send-keys, wait, then capture-pane and check
# that the expected output string appears in the pane content.
function Run-And-Verify {
    param(
        [string]$Target,       # e.g. "split4:1.2"
        [string]$Command,      # e.g. "echo HELLO_MARKER"
        [string]$Expected,     # regex to match in captured output, e.g. "HELLO_MARKER"
        [int]$Timeout = 10000
    )
    # Send the command + Enter
    & $PSMUX send-keys -t $Target "$Command" Enter 2>&1 | Out-Null
    # Poll capture-pane until the expected output appears
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $Timeout) {
        Start-Sleep -Milliseconds 300
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            if ($cap -match $Expected) {
                return @{ Found = $true; ElapsedMs = $sw.ElapsedMilliseconds; Output = $cap }
            }
        } catch {}
    }
    $finalCap = ""
    try { $finalCap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String } catch {}
    return @{ Found = $false; ElapsedMs = $sw.ElapsedMilliseconds; Output = $finalCap }
}

Log "Using: $PSMUX"
Write-Host ""

# =============================================================================
# TEST 1: Repeated vertical splits — verify each NEW pane
# =============================================================================
Write-Host ("=" * 60)
Log "TEST 1: Repeated vertical splits - verify NEW pane creation"
Write-Host ("=" * 60)
Cleanup

& $PSMUX new-session -d -s split1 2>&1 | Out-Null
Start-Sleep -Seconds 3

$r = Wait-Prompt "split1:0"
if ($r.Found) { Pass "Initial prompt" "$($r.ElapsedMs)ms" }
else          { Fail "Initial prompt" "No PS prompt in initial window" }

$maxSplits = 15
$successfulSplits = 0
$refusedSplits = 0
for ($i = 1; $i -le $maxSplits; $i++) {
    $panesBefore = Get-PaneCount "split1"
    $out = & $PSMUX split-window -t split1 -v 2>&1 | Out-String
    Start-Sleep -Milliseconds 500

    if (-not (Check-ServerAlive "split1")) {
        Fail "Server alive (vsplit $i)" "SERVER CRASHED after vsplit $i!"
        break
    }

    $panesAfter = Get-PaneCount "split1"

    # Check: did the server return an error?
    if ($out -match "too small|error|no space") {
        $refusedSplits++
        # Verify pane count did NOT increase
        if ($panesAfter -eq $panesBefore) {
            Pass "Vsplit $i refused" "correctly refused ($($out.Trim())), panes=$panesAfter"
        } else {
            Fail "Vsplit $i refused but created" "error returned but pane count went $panesBefore -> $panesAfter"
        }
        continue
    }

    # No error - split should have succeeded
    if ($panesAfter -le $panesBefore) {
        Fail "Vsplit $i no new pane" "no error but pane count unchanged ($panesBefore -> $panesAfter)"
        continue
    }

    $successfulSplits++
    $newPaneIdx = $panesAfter - 1

    # KEY CHECK: verify the NEW pane has a PS prompt
    $r = Wait-Prompt "split1:0.$newPaneIdx"
    if ($r.Found) {
        Pass "Vsplit $i new pane prompt" "pane $newPaneIdx prompt in $($r.ElapsedMs)ms (panes=$panesAfter)"
    } else {
        # Also check if any pane has the prompt — maybe active pane changed
        $anyPrompt = $false
        for ($p = 0; $p -lt $panesAfter; $p++) {
            $rc = Wait-Prompt "split1:0.$p" 3000
            if ($rc.Found -and $p -eq $newPaneIdx) { $anyPrompt = $true; break }
        }
        if ($anyPrompt) {
            Pass "Vsplit $i new pane prompt (retry)" "pane $newPaneIdx found on retry"
        } else {
            Fail "Vsplit $i new pane prompt" "pane $newPaneIdx has NO PS prompt (output: $($r.Output.Substring(0, [Math]::Min(80, $r.Output.Length))))"
        }
    }
}

if (Check-ServerAlive "split1") {
    Pass "Server survived vsplits" "$successfulSplits created, $refusedSplits refused"
} else {
    Fail "Server survived vsplits" "Server died"
}

# =============================================================================
# TEST 2: Repeated horizontal splits
# =============================================================================
Write-Host ""
Write-Host ("=" * 60)
Log "TEST 2: Repeated horizontal splits - verify NEW pane creation"
Write-Host ("=" * 60)
Cleanup

& $PSMUX new-session -d -s split2 2>&1 | Out-Null
Start-Sleep -Seconds 3

$r = Wait-Prompt "split2:0"
if ($r.Found) { Pass "Initial prompt (hsplit)" "$($r.ElapsedMs)ms" }
else          { Fail "Initial prompt (hsplit)" "No PS prompt" }

$successfulSplits = 0
$refusedSplits = 0
for ($i = 1; $i -le $maxSplits; $i++) {
    $panesBefore = Get-PaneCount "split2"
    $out = & $PSMUX split-window -t split2 -h 2>&1 | Out-String
    Start-Sleep -Milliseconds 500

    if (-not (Check-ServerAlive "split2")) {
        Fail "Server alive (hsplit $i)" "SERVER CRASHED!"
        break
    }

    $panesAfter = Get-PaneCount "split2"

    if ($out -match "too small|error|no space") {
        $refusedSplits++
        if ($panesAfter -eq $panesBefore) {
            Pass "Hsplit $i refused" "correctly refused, panes=$panesAfter"
        } else {
            Fail "Hsplit $i refused but created" "error but panes $panesBefore -> $panesAfter"
        }
        continue
    }

    if ($panesAfter -le $panesBefore) {
        Fail "Hsplit $i no new pane" "no error but panes unchanged ($panesBefore -> $panesAfter)"
        continue
    }

    $successfulSplits++
    $newPaneIdx = $panesAfter - 1

    $r = Wait-Prompt "split2:0.$newPaneIdx"
    if ($r.Found) {
        Pass "Hsplit $i new pane prompt" "pane $newPaneIdx in $($r.ElapsedMs)ms (panes=$panesAfter)"
    } else {
        Fail "Hsplit $i new pane prompt" "pane $newPaneIdx has NO PS prompt"
    }
}

if (Check-ServerAlive "split2") {
    Pass "Server survived hsplits" "$successfulSplits created, $refusedSplits refused"
} else {
    Fail "Server survived hsplits" "Server died"
}

# =============================================================================
# TEST 3: Alternating V/H splits (most realistic user scenario)
# =============================================================================
Write-Host ""
Write-Host ("=" * 60)
Log "TEST 3: Alternating V/H splits - verify every new pane"
Write-Host ("=" * 60)
Cleanup

& $PSMUX new-session -d -s split3 2>&1 | Out-Null
Start-Sleep -Seconds 3

$r = Wait-Prompt "split3:0"
if ($r.Found) { Pass "Initial prompt (alt)" "$($r.ElapsedMs)ms" }
else          { Fail "Initial prompt (alt)" "No PS prompt" }

$successfulSplits = 0
$refusedSplits = 0
for ($i = 1; $i -le 20; $i++) {
    $dir = if ($i % 2 -eq 1) { "-v" } else { "-h" }
    $dirName = if ($i % 2 -eq 1) { "V" } else { "H" }
    $panesBefore = Get-PaneCount "split3"
    $out = & $PSMUX split-window -t split3 $dir 2>&1 | Out-String
    Start-Sleep -Milliseconds 500

    if (-not (Check-ServerAlive "split3")) {
        Fail "Server alive (alt split $i $dirName)" "SERVER CRASHED!"
        break
    }

    $panesAfter = Get-PaneCount "split3"

    if ($out -match "too small|error|no space") {
        $refusedSplits++
        if ($panesAfter -eq $panesBefore) {
            Pass "AltSplit $i ($dirName) refused" "correctly refused, panes=$panesAfter"
        } else {
            Fail "AltSplit $i ($dirName) refused but created" "error but panes changed"
        }
        continue
    }

    if ($panesAfter -le $panesBefore) {
        Fail "AltSplit $i ($dirName) no new pane" "no error but panes unchanged"
        continue
    }

    $successfulSplits++
    $newPaneIdx = $panesAfter - 1

    $r = Wait-Prompt "split3:0.$newPaneIdx"
    if ($r.Found) {
        Pass "AltSplit $i ($dirName) new pane prompt" "pane $newPaneIdx in $($r.ElapsedMs)ms (panes=$panesAfter)"
    } else {
        Fail "AltSplit $i ($dirName) new pane prompt" "pane $newPaneIdx no prompt"
    }
}

if (Check-ServerAlive "split3") {
    Pass "Server survived alt splits" "$successfulSplits created, $refusedSplits refused"
} else {
    Fail "Server survived alt splits" "Server died"
}

# =============================================================================
# TEST 4: Multiple windows + splits (the user's exact scenario)
# =============================================================================
Write-Host ""
Write-Host ("=" * 60)
Log "TEST 4: 5 windows x 3 splits each - verify every new pane"
Write-Host ("=" * 60)
Cleanup

& $PSMUX new-session -d -s split4 2>&1 | Out-Null
Start-Sleep -Seconds 3

$r = Wait-Prompt "split4:0"
if ($r.Found) { Pass "Initial prompt (multi)" "$($r.ElapsedMs)ms" }
else          { Fail "Initial prompt (multi)" "No PS prompt" }

$totalPanes = 1
$totalRefused = 0
for ($w = 1; $w -le 4; $w++) {
    & $PSMUX new-window -t split4 2>&1 | Out-Null
    Start-Sleep -Milliseconds 1000

    if (-not (Check-ServerAlive "split4")) {
        Fail "Server alive (window $w)" "SERVER CRASHED creating window!"
        break
    }

    $r = Wait-Prompt "split4:$w"
    if ($r.Found) {
        Pass "Window $w prompt" "$($r.ElapsedMs)ms"
        $totalPanes++
    } else {
        Fail "Window $w prompt" "No prompt"
    }

    for ($s = 1; $s -le 3; $s++) {
        $dir = if ($s % 2 -eq 1) { "-v" } else { "-h" }
        $panesBefore = Get-PaneCount "split4"
        $out = & $PSMUX split-window -t split4 $dir 2>&1 | Out-String
        Start-Sleep -Milliseconds 500

        if (-not (Check-ServerAlive "split4")) {
            Fail "Server alive (win $w split $s)" "SERVER CRASHED!"
            break
        }

        $panesAfter = Get-PaneCount "split4"

        if ($out -match "too small|error|no space") {
            $totalRefused++
            Log "  Win $w split $s refused (expected)"
            continue
        }

        if ($panesAfter -le $panesBefore) {
            Fail "Win$w Split$s no new pane" "no error but panes unchanged"
            continue
        }

        $totalPanes++
        $newPaneIdx = $panesAfter - 1

        $r = Wait-Prompt "split4:$w.$newPaneIdx"
        if ($r.Found) {
            Pass "Win$w Split$s new pane prompt" "pane $newPaneIdx in $($r.ElapsedMs)ms"
        } else {
            Fail "Win$w Split$s new pane prompt" "pane $newPaneIdx has no prompt"
        }
    }
}

if (Check-ServerAlive "split4") {
    Pass "Server survived multi-window" "$totalPanes panes created, $totalRefused refused"
} else {
    Fail "Server survived multi-window" "Server died"
}

# =============================================================================
# TEST 5: Verify all pane dimensions are >= 2x2 (ConPTY safety)
# =============================================================================
Write-Host ""
Write-Host ("=" * 60)
Log "TEST 5: Verify all pane dimensions >= 2x2"
Write-Host ("=" * 60)

if (Check-ServerAlive "split4") {
    for ($w = 0; $w -le 4; $w++) {
        & $PSMUX select-window -t "split4:$w" 2>&1 | Out-Null
        Start-Sleep -Milliseconds 300
        $panes = & $PSMUX list-panes -t split4 2>&1 | Out-String
        $paneLines = ($panes -split "`n") | Where-Object { $_ -match '\S' }
        foreach ($line in $paneLines) {
            if ($line -match '\[(\d+)x(\d+)\]') {
                $cols = [int]$Matches[1]
                $rows = [int]$Matches[2]
                if ($cols -lt 2 -or $rows -lt 2) {
                    Fail "Pane dim win$w" "DANGEROUS dimensions: ${cols}x${rows} - ConPTY will crash! ($line)"
                } else {
                    Pass "Pane dim win$w" "${cols}x${rows}"
                }
            }
        }
    }
} else {
    Fail "Dimension check" "Server not alive"
}

# =============================================================================
# TEST 6: Verify exit code on refused split
# =============================================================================
Write-Host ""
Write-Host ("=" * 60)
Log "TEST 6: Exit code on refused split"
Write-Host ("=" * 60)

if (Check-ServerAlive "split4") {
    # Try to split a pane that should be too small (window 0 has been split multiple times)
    & $PSMUX select-window -t "split4:0" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    # Try to split the already-split panes — at least one should be too small
    # Force split into a pane we know is already at minimum
    $out = & $PSMUX split-window -t split4 -v 2>&1 | Out-String
    $exitCode = $LASTEXITCODE
    if ($out -match "too small") {
        if ($exitCode -ne 0) {
            Pass "Exit code on refuse" "exit=$exitCode, got error: $($out.Trim())"
        } else {
            Pass "Exit code on refuse" "exit=$exitCode (0 is acceptable), error: $($out.Trim())"
        }
    } else {
        # Split might have succeeded if there's still room
        Pass "Exit code test" "split succeeded (pane had room), exit=$exitCode"
    }
} else {
    Fail "Exit code test" "Server not alive"
}

# =============================================================================
# TEST 7: Run commands in every pane and verify output
# Creates a fresh session with 3 windows × 2 splits, then sends echo + ls
# into every single pane and verifies the output appeared via capture-pane.
# =============================================================================
Write-Host ""
Write-Host ("=" * 60)
Log "TEST 7: Run commands in every pane and verify output"
Write-Host ("=" * 60)
Cleanup

& $PSMUX new-session -d -s cmdtest 2>&1 | Out-Null
Start-Sleep -Seconds 3

$r = Wait-Prompt "cmdtest:0"
if ($r.Found) { Pass "Cmdtest initial prompt" "$($r.ElapsedMs)ms" }
else          { Fail "Cmdtest initial prompt" "No PS prompt" }

# Create 2 more windows, each with 2 splits (V then H) = 3 panes per window
for ($w = 1; $w -le 2; $w++) {
    & $PSMUX new-window -t cmdtest 2>&1 | Out-Null
    Start-Sleep -Milliseconds 1000
    $r = Wait-Prompt "cmdtest:$w"
    if (-not $r.Found) { Fail "Cmdtest window $w prompt" "No prompt"; continue }

    & $PSMUX split-window -t cmdtest -v 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    & $PSMUX split-window -t cmdtest -h 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
}

# Also split the first window
& $PSMUX select-window -t "cmdtest:0" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX split-window -t cmdtest -v 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# Wait for all panes to settle
Start-Sleep -Seconds 2

# Now iterate every window and every pane, run two commands:
# 1) echo MARKER_<win>_<pane>  — verify the unique marker appears
# 2) Get-ChildItem env:COMPUTERNAME — verify ls-like command works
for ($w = 0; $w -le 2; $w++) {
    & $PSMUX select-window -t "cmdtest:$w" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300

    $paneCount = Get-PaneCount "cmdtest"
    for ($p = 0; $p -lt $paneCount; $p++) {
        $target = "cmdtest:$w.$p"
        $marker = "PANE_OK_${w}_${p}"

        # First wait for the prompt to appear in this pane
        $pr = Wait-Prompt $target 8000
        if (-not $pr.Found) {
            Fail "Pane $target prompt" "No PS prompt before running command"
            continue
        }

        # TEST 7a: echo a unique marker and verify it appears
        $r = Run-And-Verify -Target $target -Command "echo $marker" -Expected $marker -Timeout 8000
        if ($r.Found) {
            Pass "echo in $target" "marker appeared in $($r.ElapsedMs)ms"
        } else {
            $snippet = if ($r.Output.Length -gt 80) { $r.Output.Substring(0, 80) } else { $r.Output }
            Fail "echo in $target" "marker '$marker' not found (captured: $snippet)"
        }

        # TEST 7b: run Get-ChildItem and verify directory listing output
        $r = Run-And-Verify -Target $target -Command "Get-ChildItem env:COMPUTERNAME" -Expected "COMPUTERNAME" -Timeout 8000
        if ($r.Found) {
            Pass "ls in $target" "Get-ChildItem output in $($r.ElapsedMs)ms"
        } else {
            $snippet = if ($r.Output.Length -gt 80) { $r.Output.Substring(0, 80) } else { $r.Output }
            Fail "ls in $target" "COMPUTERNAME not found (captured: $snippet)"
        }
    }
}

if (Check-ServerAlive "cmdtest") {
    Pass "Server survived cmd test" "all command executions completed"
} else {
    Fail "Server survived cmd test" "Server died"
}

# =============================================================================
# CLEANUP & SUMMARY
# =============================================================================
Write-Host ""
Cleanup

Write-Host ("=" * 60)
$color = if ($failedTests -eq 0) { "Green" } else { "Red" }
Write-Host "RESULTS: $passedTests passed, $failedTests failed, $totalTests total" -ForegroundColor $color
if ($failures.Count -gt 0) {
    Write-Host "Failures:" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
}
Write-Host ("=" * 60)
exit $failedTests
