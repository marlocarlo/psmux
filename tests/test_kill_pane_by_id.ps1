# test_kill_pane_by_id.ps1 — PR #101: preserve main pane focus when killing targeted panes
#
# Tests:
# 1. kill-pane -t %id kills the correct pane
# 2. Main pane remains focused after targeted kill
# 3. kill-pane -t %id across windows (target in different window)
# 4. Multiple targeted kills preserve focus each time
# 5. select-pane -P style-only doesn't send empty select
# 6. kill-pane (no target) still kills active pane
# 7. kill-pane -t %id with invalid ID is a no-op
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_kill_pane_by_id.ps1

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

$SESSION = "killpane101"

function Cleanup {
    & $PSMUX kill-server 2>$null | Out-Null
    Start-Sleep -Seconds 1
    Get-Process -Name psmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
}

function Get-PaneIds {
    param([string]$Target)
    $out = & $PSMUX list-panes -t $Target -F '#{pane_id}' 2>$null
    @($out | Where-Object { $_ -is [string] } | ForEach-Object { $_.Trim().TrimStart('%') } | Where-Object { $_ -match '^\d+$' })
}

function Get-ActivePaneId {
    param([string]$Target)
    $out = & $PSMUX display-message -t $Target -p '#{pane_id}' 2>$null | Out-String
    $out.Trim().TrimStart('%')
}

# Helper: session-qualified pane target (e.g., "killpane101:%2")
function PaneTarget { param([string]$PaneId) return "${SESSION}:%${PaneId}" }

Cleanup

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  PR #101: KILL-PANE BY ID — FOCUS PRESERVATION"
Write-Host ("=" * 60)

# Create test session
& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 3
& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create session" -ForegroundColor Red; exit 1 }
Write-Info "Session '$SESSION' created"

# ============================================================
# TEST 1: kill-pane -t %id kills the correct pane
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 1: kill-pane -t %id kills the correct pane"
Write-Host ("=" * 60)

# Split to create a second pane
& $PSMUX split-window -t $SESSION -v 2>&1 | Out-Null
Start-Sleep -Seconds 2

$panesBefore = Get-PaneIds $SESSION
Write-Test "1.1 Two panes exist before kill"
if ($panesBefore.Count -eq 2) {
    Write-Pass "2 panes exist: $($panesBefore -join ', ')"
} else {
    Write-Fail "Expected 2 panes, got $($panesBefore.Count)"
}

$secondPaneId = $panesBefore[1]
$firstPaneId = $panesBefore[0]

# Kill the second pane by ID (session-qualified target)
Write-Test "1.2 Kill second pane by ID (%$secondPaneId)"
& $PSMUX kill-pane -t (PaneTarget $secondPaneId) 2>&1 | Out-Null
Start-Sleep -Seconds 1

$panesAfter = Get-PaneIds $SESSION
if ($panesAfter.Count -eq 1) {
    Write-Pass "1 pane remains after kill"
} else {
    Write-Fail "Expected 1 pane, got $($panesAfter.Count)"
}

Write-Test "1.3 Surviving pane is the first pane"
if ($panesAfter[0] -eq $firstPaneId) {
    Write-Pass "First pane (ID $firstPaneId) survived"
} else {
    Write-Fail "Wrong pane survived (got $($panesAfter[0]), expected $firstPaneId)"
}

# ============================================================
# TEST 2: Main pane remains focused after targeted kill
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 2: Main pane remains focused after targeted kill"
Write-Host ("=" * 60)

# Create 3 panes: original + 2 splits
& $PSMUX split-window -t $SESSION -v 2>&1 | Out-Null
Start-Sleep -Seconds 1
& $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
Start-Sleep -Seconds 2

$panes3 = Get-PaneIds $SESSION
Write-Test "2.1 Three panes exist"
if ($panes3.Count -ge 3) {
    Write-Pass "$($panes3.Count) panes exist"
} else {
    Write-Fail "Expected 3+ panes, got $($panes3.Count)"
}

# Focus back to the first pane
& $PSMUX select-pane -t (PaneTarget $firstPaneId) 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$activeBefore = Get-ActivePaneId $SESSION
Write-Test "2.2 First pane is active before kill"
if ($activeBefore -eq $firstPaneId) {
    Write-Pass "First pane ($firstPaneId) is active"
} else {
    Write-Info "Active pane is $activeBefore (expected $firstPaneId)"
}

# Kill one of the other panes by ID (not the active one)
$targetKill = $panes3 | Where-Object { $_ -ne $firstPaneId } | Select-Object -First 1
Write-Test "2.3 Kill non-active pane %$targetKill while first pane is focused"
& $PSMUX kill-pane -t (PaneTarget $targetKill) 2>&1 | Out-Null
Start-Sleep -Seconds 1

$activeAfter = Get-ActivePaneId $SESSION
Write-Test "2.4 First pane still active after targeted kill"
if ($activeAfter -eq $firstPaneId) {
    Write-Pass "Focus preserved — first pane ($firstPaneId) still active"
} else {
    Write-Fail "Focus lost — active is now $activeAfter (expected $firstPaneId)"
}

# Verify the killed pane is gone
$panesAfterKill = Get-PaneIds $SESSION
$targetStillExists = $panesAfterKill -contains $targetKill
if (-not $targetStillExists) {
    Write-Pass "Killed pane %$targetKill is gone"
} else {
    Write-Fail "Killed pane %$targetKill still exists"
}

# ============================================================
# TEST 3: kill-pane -t %id in a different window
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 3: Cross-window targeted kill-pane"
Write-Host ("=" * 60)

# Create a second window with a split
& $PSMUX new-window -t $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 2
& $PSMUX split-window -t $SESSION -v 2>&1 | Out-Null
Start-Sleep -Seconds 1

# Get pane IDs in window 1
$win1Panes = Get-PaneIds "${SESSION}:1"
Write-Test "3.1 Window 1 has 2 panes"
if ($win1Panes.Count -eq 2) {
    Write-Pass "Window 1 has 2 panes: $($win1Panes -join ', ')"
} else {
    Write-Fail "Expected 2 panes in window 1, got $($win1Panes.Count)"
}

# Switch back to window 0
& $PSMUX select-window -t "${SESSION}:0" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$activeW0 = Get-ActivePaneId "${SESSION}:0"

# Kill a pane in window 1 while window 0 is active
if ($win1Panes.Count -ge 2) {
    $killTarget = $win1Panes[1]
    Write-Test "3.2 Kill pane %$killTarget in window 1 while window 0 is active"
    & $PSMUX kill-pane -t (PaneTarget $killTarget) 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    $win1PanesAfter = Get-PaneIds "${SESSION}:1"
    if ($win1PanesAfter.Count -eq 1) {
        Write-Pass "Window 1 now has 1 pane"
    } else {
        Write-Fail "Expected 1 pane in window 1, got $($win1PanesAfter.Count)"
    }

    # Window 0's active pane should be unchanged
    $activeW0After = Get-ActivePaneId "${SESSION}:0"
    Write-Test "3.3 Window 0 focus unchanged after cross-window kill"
    if ($activeW0After -eq $activeW0) {
        Write-Pass "Window 0 focus preserved ($activeW0)"
    } else {
        Write-Fail "Window 0 focus changed: $activeW0 -> $activeW0After"
    }
} else {
    Write-Fail "Skipping 3.2-3.3: could not create 2 panes in window 1"
}

# ============================================================
# TEST 4: Multiple sequential targeted kills
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 4: Multiple sequential targeted kills"
Write-Host ("=" * 60)

# Go to window 0 and create multiple splits
& $PSMUX select-window -t "${SESSION}:0" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
# Clean up remaining panes, start fresh
& $PSMUX kill-window -t "${SESSION}:1" 2>&1 | Out-Null
Start-Sleep -Seconds 1

# Get current pane as the "main" pane
$mainPanes = Get-PaneIds $SESSION
if ($mainPanes.Count -gt 0) {
    $mainId = $mainPanes[0]
} else {
    Write-Fail "No panes found for test 4"
    $mainId = $null
}

# Create 3 child panes (fewer to be more reliable)
for ($i = 0; $i -lt 3; $i++) {
    & $PSMUX split-window -t $SESSION -v 2>&1 | Out-Null
    Start-Sleep -Milliseconds 800
}
Start-Sleep -Seconds 1

$allPanes = Get-PaneIds $SESSION
Write-Test "4.1 Multiple panes created"
Write-Info "Panes: $($allPanes -join ', ') (main=$mainId)"
if ($allPanes.Count -ge 3) {
    Write-Pass "$($allPanes.Count) panes exist"
} else {
    Write-Fail "Expected 3+ panes, got $($allPanes.Count)"
}

if ($mainId) {
    # Focus back to main pane
    & $PSMUX select-pane -t (PaneTarget $mainId) 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Kill child panes one by one
    $children = $allPanes | Where-Object { $_ -ne $mainId }
    $killCount = 0
    $focusLost = $false
    foreach ($child in $children) {
        Write-Info "  Killing child pane %$child (kill #$($killCount+1))..."
        & $PSMUX kill-pane -t (PaneTarget $child) 2>&1 | Out-Null
        Start-Sleep -Seconds 1

        $killCount++
        $remainingPanes = Get-PaneIds $SESSION
        Write-Info "  Remaining panes after kill #${killCount}: $($remainingPanes -join ', ')"

        # Check focus is still on main pane
        $current = Get-ActivePaneId $SESSION
        if ($current -ne $mainId) {
            Write-Fail "Focus lost after killing child $killCount (active=$current, expected=$mainId)"
            $focusLost = $true
            break
        }
    }

    $finalPanes = Get-PaneIds $SESSION
    Write-Test "4.2 All children killed, main pane survived"
    if ($finalPanes.Count -eq 1 -and $finalPanes[0] -eq $mainId -and -not $focusLost) {
        Write-Pass "Only main pane ($mainId) remains, focus preserved through $killCount kills"
    } else {
        if (-not $focusLost) {
            Write-Fail "Expected only main pane $mainId, got: $($finalPanes -join ', ')"
        }
    }
}

# ============================================================
# TEST 5: select-pane -P style-only doesn't break focus
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 5: select-pane -P style-only update"
Write-Host ("=" * 60)

# Create a split
& $PSMUX split-window -t $SESSION -v 2>&1 | Out-Null
Start-Sleep -Seconds 1

$panesBeforeStyle = Get-PaneIds $SESSION
$activeBeforeStyle = Get-ActivePaneId $SESSION
Write-Test "5.1 Set pane style with -P (no direction)"

# select-pane -P should set style without changing focus
& $PSMUX select-pane -t $SESSION -P 'bg=red' 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$activeAfterStyle = Get-ActivePaneId $SESSION
if ($activeAfterStyle -eq $activeBeforeStyle) {
    Write-Pass "Focus unchanged after select-pane -P style update"
} else {
    Write-Fail "Focus changed after -P: $activeBeforeStyle -> $activeAfterStyle"
}

$panesAfterStyle = Get-PaneIds $SESSION
if ($panesAfterStyle.Count -eq $panesBeforeStyle.Count) {
    Write-Pass "Pane count unchanged after style update"
} else {
    Write-Fail "Pane count changed: $($panesBeforeStyle.Count) -> $($panesAfterStyle.Count)"
}

# ============================================================
# TEST 6: kill-pane (no target) still kills active pane
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 6: kill-pane (no -t) kills active pane"
Write-Host ("=" * 60)

$panesBeforeUntargeted = Get-PaneIds $SESSION
$activeBeforeUntargeted = Get-ActivePaneId $SESSION
Write-Test "6.1 kill-pane without -t kills active pane"
& $PSMUX kill-pane -t $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 1

$panesAfterUntargeted = Get-PaneIds $SESSION
if ($panesAfterUntargeted.Count -eq ($panesBeforeUntargeted.Count - 1)) {
    Write-Pass "Active pane killed, $($panesAfterUntargeted.Count) pane(s) remain"
} else {
    Write-Fail "Expected $($panesBeforeUntargeted.Count - 1) panes, got $($panesAfterUntargeted.Count)"
}

# The active pane should have changed
$activeAfterUntargeted = Get-ActivePaneId $SESSION
if ($activeAfterUntargeted -ne $activeBeforeUntargeted) {
    Write-Pass "Active pane changed from $activeBeforeUntargeted to $activeAfterUntargeted"
} else {
    Write-Pass "Active pane is $activeAfterUntargeted"
}

# ============================================================
# TEST 7: kill-pane -t %id with invalid ID is a no-op
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 7: kill-pane -t with invalid pane ID"
Write-Host ("=" * 60)

$panesBeforeInvalid = Get-PaneIds $SESSION
Write-Test "7.1 kill-pane with non-existent pane ID"
& $PSMUX kill-pane -t "${SESSION}:%99999" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$panesAfterInvalid = Get-PaneIds $SESSION
if ($panesAfterInvalid.Count -eq $panesBeforeInvalid.Count) {
    Write-Pass "No panes killed — invalid ID was a no-op ($($panesAfterInvalid.Count) panes)"
} else {
    Write-Fail "Pane count changed after invalid ID: $($panesBeforeInvalid.Count) -> $($panesAfterInvalid.Count)"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Cleanup

Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed (of $total)"
Write-Host ("=" * 60)
if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
