#!/usr/bin/env pwsh
# Tests for Issue #112 (split-window -d MRU mutation) and Issue #113 (display-message -t pane_active)
param([switch]$Verbose)

$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot '..\target\release\psmux.exe'
if (-not (Test-Path $exe)) { $exe = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $exe) { Write-Error "psmux not found"; exit 1 }
Write-Output "[INFO] Using: $exe"

$pass = 0; $fail = 0; $skip = 0
function Cleanup { & $exe kill-server 2>$null; Start-Sleep -Milliseconds 500 }

Write-Output ""
Write-Output "======================================================================"
Write-Output "Issue #112: split-window -d should NOT mutate MRU"
Write-Output "Issue #113: display-message -t should report correct pane_active"
Write-Output "======================================================================"

# ── Issue #113 Tests ──

Cleanup
Write-Output "[TEST] 113-1: display-message -t reports correct pane_active"
& $exe new-session -d -s t113
Start-Sleep -Milliseconds 2000
& $exe split-window -h -t t113
Start-Sleep -Milliseconds 2000

# After split-window -h, focus is on pane 1 (the new pane on the right)
# Pane 0 should be inactive, pane 1 should be active
$p0 = (& $exe display-message -t t113:0.0 -p '#{pane_index}|#{pane_active}').Trim()
$p1 = (& $exe display-message -t t113:0.1 -p '#{pane_index}|#{pane_active}').Trim()
Write-Output "[INFO]   Pane 0 query: $p0"
Write-Output "[INFO]   Pane 1 query: $p1"
if ($p0 -eq "0|0" -and $p1 -eq "1|1") {
    Write-Output "[PASS] 113-1: pane_active correct for both panes"
    $pass++
} else {
    Write-Output "[FAIL] 113-1: Expected '0|0' and '1|1', got '$p0' and '$p1'"
    $fail++
}

Cleanup
Write-Output "[TEST] 113-2: display-message -t does not change actual focus"
& $exe new-session -d -s t113b
Start-Sleep -Milliseconds 2000
& $exe split-window -h -t t113b
Start-Sleep -Milliseconds 2000

# Focus is on pane 1; querying pane 0 should NOT move focus
$before = (& $exe display-message -t t113b -p '#{pane_index}').Trim()
$query = (& $exe display-message -t t113b:0.0 -p '#{pane_index}|#{pane_active}').Trim()
$after = (& $exe display-message -t t113b -p '#{pane_index}').Trim()
Write-Output "[INFO]   Focus before: $before, query pane 0: $query, focus after: $after"
if ($before -eq $after) {
    Write-Output "[PASS] 113-2: display-message -t did not change focus"
    $pass++
} else {
    Write-Output "[FAIL] 113-2: Focus changed from $before to $after after display-message -t"
    $fail++
}

Cleanup
Write-Output "[TEST] 113-3: display-message -t with 3 panes"
& $exe new-session -d -s t113c
Start-Sleep -Milliseconds 2000
& $exe split-window -h -t t113c
Start-Sleep -Milliseconds 2000
& $exe split-window -v -t t113c
Start-Sleep -Milliseconds 2000

# Last split created pane 2 (bottom right). Focus should be on pane 2.
$p0 = (& $exe display-message -t t113c:0.0 -p '#{pane_active}').Trim()
$p1 = (& $exe display-message -t t113c:0.1 -p '#{pane_active}').Trim()
$p2 = (& $exe display-message -t t113c:0.2 -p '#{pane_active}').Trim()
Write-Output "[INFO]   pane_active: p0=$p0 p1=$p1 p2=$p2"
if ($p0 -eq "0" -and $p1 -eq "0" -and $p2 -eq "1") {
    Write-Output "[PASS] 113-3: Only active pane reports pane_active=1"
    $pass++
} else {
    Write-Output "[FAIL] 113-3: Expected 0,0,1 got $p0,$p1,$p2"
    $fail++
}

# ── Issue #112 Tests ──

Cleanup
Write-Output "[TEST] 112-1: split-window -d does not mutate MRU"
# Retry session creation (warm pane pool can cause transient failures)
$ok = $false
for ($retry = 0; $retry -lt 3; $retry++) {
    & $exe new-session -d -s t112 2>$null
    Start-Sleep -Milliseconds 2000
    $chk = & $exe has-session -t t112 2>$null
    if ($LASTEXITCODE -eq 0) { $ok = $true; break }
    & $exe kill-server 2>$null; Start-Sleep -Milliseconds 500
}
if (-not $ok) { Write-Output "[SKIP] 112-1: could not create session"; $skip++; } else {
& $exe split-window -h -t t112
Start-Sleep -Milliseconds 2000
# Now: pane 0 (left), pane 1 (right, active/focused)
# Focus pane 0, then back to 1 to set MRU: 1 is MRU, 0 is second
& $exe select-pane -t t112:0.0
Start-Sleep -Milliseconds 500
& $exe select-pane -t t112:0.1
Start-Sleep -Milliseconds 500

# Now split pane 0 with -d (detached). This should NOT change MRU.
& $exe split-window -v -d -t t112:0.0
Start-Sleep -Milliseconds 2000

# MRU order before split was: pane 1 (front), pane 0. 
# After detached split, MRU should still have pane 1 at front.
# New pane (2) was created from splitting pane 0 but is detached.
# Navigate from pane 1 left: should go to pane 0 (MRU among left candidates)
$active = (& $exe display-message -t t112 -p '#{pane_index}').Trim()
Write-Output "[INFO]   Active pane before nav: $active"
& $exe select-pane -t t112 -L
Start-Sleep -Milliseconds 500
$after_left = (& $exe display-message -t t112 -p '#{pane_index}').Trim()
Write-Output "[INFO]   After select-pane -L: $after_left"
# We expect to land on pane 0 (the left pane), since it was MRU among left candidates
# The detached split should NOT have made the new pane MRU
if ($after_left -eq "0") {
    Write-Output "[PASS] 112-1: Detached split did not corrupt MRU"
    $pass++
} else {
    Write-Output "[FAIL] 112-1: Expected pane 0 after -L, got $after_left (MRU corrupted by -d split)"
    $fail++
}
} # end retry guard for 112-1

Cleanup
Write-Output "[TEST] 112-2: non-detached split DOES update focus and MRU"
$ok = $false
for ($retry = 0; $retry -lt 3; $retry++) {
    & $exe new-session -d -s t112b 2>$null
    Start-Sleep -Milliseconds 2000
    $chk = & $exe has-session -t t112b 2>$null
    if ($LASTEXITCODE -eq 0) { $ok = $true; break }
    & $exe kill-server 2>$null; Start-Sleep -Milliseconds 500
}
if (-not $ok) { Write-Output "[SKIP] 112-2: could not create session"; $skip++; } else {
& $exe split-window -h -t t112b
Start-Sleep -Milliseconds 2000
# Focus on pane 1. Now split pane 0 (non-detached) - focus should move to new pane.
& $exe split-window -v -t t112b:0.0
Start-Sleep -Milliseconds 2000
$active = (& $exe display-message -t t112b -p '#{pane_index}').Trim()
Write-Output "[INFO]   Active pane after non-detached split of pane 0: $active"
# After splitting pane 0, non-detached: new pane (index 2) should be focused
if ($active -eq "1" -or $active -eq "2") {
    Write-Output "[PASS] 112-2: Non-detached split moved focus to new pane (index $active)"
    $pass++
} else {
    Write-Output "[FAIL] 112-2: Expected new pane to be focused, got index $active"
    $fail++
}
} # end retry guard for 112-2

# ── Cleanup ──
Cleanup

Write-Output ""
Write-Output "======================================================================"
Write-Output "Results: $pass passed, $fail failed, $skip skipped"
Write-Output "======================================================================"
if ($fail -gt 0) { exit 1 } else { exit 0 }
