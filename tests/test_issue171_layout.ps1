# Integration test for issue #171: layout system bugs
# Tests: resize-pane -x/-y, split-window -l, select-layout tiled
# Requires: psmux built and installed

param([switch]$Verbose)

$ErrorActionPreference = "Stop"
$pass = 0
$fail = 0
$sessName = "test171_$(Get-Random -Maximum 9999)"

function Log($msg) { if ($Verbose) { Write-Host "  [DBG] $msg" -ForegroundColor DarkGray } }
function Pass($msg) { $script:pass++; Write-Host "  PASS: $msg" -ForegroundColor Green }
function Fail($msg) { $script:fail++; Write-Host "  FAIL: $msg" -ForegroundColor Red }

# Start a fresh session
Write-Host "`nStarting test session: $sessName" -ForegroundColor Cyan
$env:PSMUX_TARGET_SESSION = $sessName
psmux new-session -d -s $sessName
Start-Sleep -Milliseconds 800

try {
    # ──────────────────────────────────────────────
    #  Test 1: resize-pane -x should change width
    # ──────────────────────────────────────────────
    Write-Host "`n[Test 1] resize-pane -x/-y" -ForegroundColor Yellow

    # Create a horizontal split
    psmux split-window -h -d
    Start-Sleep -Milliseconds 500

    # Get initial pane widths
    $before = psmux list-panes -F "#{pane_width}"
    Log "Before resize: $($before -join ', ')"

    # Resize first pane to 30 columns
    psmux resize-pane -t %0 -x 30
    Start-Sleep -Milliseconds 300

    $after = psmux list-panes -F "#{pane_width}"
    Log "After resize-pane -x 30: $($after -join ', ')"

    $widths_before = ($before | ForEach-Object { [int]$_.Trim() })
    $widths_after = ($after | ForEach-Object { [int]$_.Trim() })

    if ($widths_before.Count -ge 2 -and $widths_after.Count -ge 2) {
        if ($widths_after[0] -ne $widths_before[0]) {
            Pass "resize-pane -x changed pane width (was $($widths_before[0]), now $($widths_after[0]))"
        } else {
            Fail "resize-pane -x did not change pane width (still $($widths_before[0]))"
        }
    } else {
        Fail "Could not parse pane widths"
    }

    # Test -y too
    psmux select-layout even-horizontal
    Start-Sleep -Milliseconds 200
    psmux split-window -v -d
    Start-Sleep -Milliseconds 500

    $beforeY = psmux list-panes -F "#{pane_height}"
    Log "Before resize-y: $($beforeY -join ', ')"
    psmux resize-pane -y 10
    Start-Sleep -Milliseconds 300
    $afterY = psmux list-panes -F "#{pane_height}"
    Log "After resize-pane -y 10: $($afterY -join ', ')"

    $h_before = ($beforeY | ForEach-Object { [int]$_.Trim() })
    $h_after = ($afterY | ForEach-Object { [int]$_.Trim() })
    if ($h_before.Count -ge 2 -and $h_after.Count -ge 2) {
        # At least one height should have changed
        $changed = $false
        for ($i = 0; $i -lt $h_after.Count; $i++) {
            if ($h_after[$i] -ne $h_before[$i]) { $changed = $true; break }
        }
        if ($changed) {
            Pass "resize-pane -y changed pane height"
        } else {
            Fail "resize-pane -y did not change pane height"
        }
    } else {
        Fail "Could not parse pane heights"
    }

    # ──────────────────────────────────────────────
    #  Test 2: split-window -l vs -p
    # ──────────────────────────────────────────────
    Write-Host "`n[Test 2] split-window -l (cell count) vs -p (percentage)" -ForegroundColor Yellow

    # Kill extra panes first, start fresh
    psmux kill-pane 2>$null
    psmux kill-pane 2>$null
    Start-Sleep -Milliseconds 300

    # Get window width
    $winWidth = psmux display-message -p "#{window_width}"
    $winW = [int]($winWidth.Trim())
    Log "Window width: $winW"

    # Split with -p 30 (should give new pane ~30% of space)
    psmux split-window -h -d -p 30
    Start-Sleep -Milliseconds 500
    $pctWidths = psmux list-panes -F "#{pane_width}"
    Log "After split -p 30: $($pctWidths -join ', ')"
    $pw = ($pctWidths | ForEach-Object { [int]$_.Trim() })
    # New pane (second) should be roughly 30% of window width
    if ($pw.Count -ge 2) {
        $ratio = [math]::Round(($pw[1] / ($pw[0] + $pw[1] + 1)) * 100)
        Log "New pane percentage: ${ratio}%"
        if ($ratio -ge 20 -and $ratio -le 40) {
            Pass "split-window -p 30 created pane at ~${ratio}% (expected ~30%)"
        } else {
            Fail "split-window -p 30 created pane at ~${ratio}% (expected ~30%)"
        }
    }

    # Kill the pane and try -l with a specific cell count
    psmux kill-pane
    Start-Sleep -Milliseconds 300

    # -l 20 should give new pane exactly ~20 columns (NOT 20%)
    psmux split-window -h -d -l 20
    Start-Sleep -Milliseconds 500
    $cellWidths = psmux list-panes -F "#{pane_width}"
    Log "After split -l 20: $($cellWidths -join ', ')"
    $cw = ($cellWidths | ForEach-Object { [int]$_.Trim() })
    if ($cw.Count -ge 2) {
        $newPaneW = $cw[1]
        # With -l 20, the new pane should be around 20 cells (some variance due to rounding)
        if ($newPaneW -ge 10 -and $newPaneW -le 35) {
            Pass "split-window -l 20 created pane with $newPaneW cols (expected ~20 cells)"
        } else {
            Fail "split-window -l 20 created pane with $newPaneW cols (expected ~20, got $newPaneW which suggests % interpretation)"
        }
    }

    # ──────────────────────────────────────────────
    #  Test 3: select-layout tiled redistributes
    # ──────────────────────────────────────────────
    Write-Host "`n[Test 3] select-layout tiled" -ForegroundColor Yellow

    # Kill extra panes and create 4 panes with unequal sizes
    psmux kill-pane 2>$null
    Start-Sleep -Milliseconds 200

    psmux split-window -h -d -p 80
    Start-Sleep -Milliseconds 400
    psmux split-window -v -d -p 80
    Start-Sleep -Milliseconds 400
    psmux select-pane -t %0
    Start-Sleep -Milliseconds 100
    psmux split-window -v -d -p 80
    Start-Sleep -Milliseconds 400

    $beforeTiled = psmux list-panes -F "#{pane_width}x#{pane_height}"
    Log "Before tiled: $($beforeTiled -join ', ')"

    psmux select-layout tiled
    Start-Sleep -Milliseconds 500

    $afterTiled = psmux list-panes -F "#{pane_width}x#{pane_height}"
    Log "After tiled: $($afterTiled -join ', ')"

    # After tiled layout, pane sizes should be more equal
    $beforeSizes = $beforeTiled | ForEach-Object { $_.Trim() }
    $afterSizes = $afterTiled | ForEach-Object { $_.Trim() }

    if ($afterSizes.Count -ge 3) {
        # Check if sizes changed at all
        $sizeChanged = $false
        for ($i = 0; $i -lt [Math]::Min($beforeSizes.Count, $afterSizes.Count); $i++) {
            if ($beforeSizes[$i] -ne $afterSizes[$i]) { $sizeChanged = $true; break }
        }
        if ($sizeChanged) {
            Pass "select-layout tiled redistributed pane sizes"
        } else {
            Fail "select-layout tiled did NOT change pane sizes"
        }

        # Check that sizes changed (redistribution happened)
        $widths = $afterSizes | ForEach-Object { [int]($_ -split 'x')[0] }
        $heights = $afterSizes | ForEach-Object { [int]($_ -split 'x')[1] }
        # For 3+ panes, tiled may have one full-width pane on top and two below
        # So we check heights are roughly balanced instead (within half)
        $maxH = ($heights | Measure-Object -Maximum).Maximum
        $minH = ($heights | Measure-Object -Minimum).Minimum
        Log "Height range: $minH to $maxH"
        if ($maxH -le ($minH * 3)) {
            Pass "tiled layout has balanced dimensions"
        } else {
            Fail "tiled layout has very unbalanced dimensions (min height $minH, max height $maxH)"
        }
    } else {
        Fail "Not enough panes for tiled test"
    }

} finally {
    # Cleanup
    Write-Host "`nCleaning up session $sessName..." -ForegroundColor Cyan
    psmux kill-session -t $sessName 2>$null
    $env:PSMUX_TARGET_SESSION = $null
}

Write-Host "`n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
if ($fail -gt 0) { exit 1 }
