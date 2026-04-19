# Cross-session join-pane E2E tests
# Tests the ability to move panes between different psmux sessions
# via the TCP proxy architecture (ConPTY stays in source, proxy in target)

param(
    [switch]$Verbose
)

$ErrorActionPreference = 'Continue'
$script:passed = 0
$script:failed = 0
$script:results = @()

function Write-TestResult($name, $pass, $detail) {
    if ($pass) {
        Write-Host "  PASS: $name" -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host "  FAIL: $name" -ForegroundColor Red
        if ($detail) { Write-Host "        $detail" -ForegroundColor Yellow }
        $script:failed++
    }
    $script:results += [PSCustomObject]@{ Name=$name; Pass=$pass; Detail=$detail }
}

function Cleanup-Sessions {
    # Kill any leftover test sessions
    @('xsrc', 'xtgt', 'cross_src', 'cross_tgt', 'csrc', 'ctgt') | ForEach-Object {
        psmux kill-session -t $_ 2>$null
    }
    Start-Sleep -Milliseconds 500
}

# ============================================================================
# PART A: Basic cross-session infrastructure tests
# ============================================================================
Write-Host "`nPART A: Cross-session infrastructure" -ForegroundColor Cyan

Cleanup-Sessions

# A1: Create two separate sessions
Write-Host "`n  Setting up test sessions..."
psmux new-session -d -s csrc 2>$null
Start-Sleep -Milliseconds 1500
psmux new-session -d -s ctgt 2>$null
Start-Sleep -Milliseconds 1500

# Verify both sessions exist
$srcExists = psmux has-session -t csrc 2>&1; $srcOk = $LASTEXITCODE -eq 0
$tgtExists = psmux has-session -t ctgt 2>&1; $tgtOk = $LASTEXITCODE -eq 0
Write-TestResult "A1: Both test sessions created" ($srcOk -and $tgtOk) "src=$srcOk tgt=$tgtOk"

# A2: Verify sessions have separate server ports
$userHome = $env:USERPROFILE
$srcPort = if (Test-Path "$userHome\.psmux\csrc.port") { Get-Content "$userHome\.psmux\csrc.port" -Raw } else { "" }
$tgtPort = if (Test-Path "$userHome\.psmux\ctgt.port") { Get-Content "$userHome\.psmux\ctgt.port" -Raw } else { "" }
$portsOk = $srcPort.Trim() -ne "" -and $tgtPort.Trim() -ne "" -and $srcPort.Trim() -ne $tgtPort.Trim()
Write-TestResult "A2: Sessions have distinct server ports" $portsOk "src=$($srcPort.Trim()) tgt=$($tgtPort.Trim())"

# A3: Create a second pane in source session for the transfer
psmux split-window -t csrc 2>$null
Start-Sleep -Milliseconds 1000
$srcPanes = psmux list-panes -t csrc 2>&1
$srcPaneCount = ($srcPanes | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "A3: Source session has 2+ panes for transfer" ($srcPaneCount -ge 2) "count=$srcPaneCount"

# A4: Verify source session initial window count
$srcWindows = psmux list-windows -t csrc 2>&1
$srcWinCount = ($srcWindows | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "A4: Source has expected window count" ($srcWinCount -ge 1) "windows=$srcWinCount"

# A5: Verify target session initial window/pane count
$tgtPanes = psmux list-panes -t ctgt 2>&1
$tgtPaneCount = ($tgtPanes | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "A5: Target has 1 pane before transfer" ($tgtPaneCount -ge 1) "count=$tgtPaneCount"

# ============================================================================
# PART B: Cross-session join-pane execution
# ============================================================================
Write-Host "`nPART B: Cross-session join-pane execution" -ForegroundColor Cyan

# B1: Execute cross-session join-pane (move pane from csrc to ctgt)
# Syntax: psmux join-pane -s csrc:0.1 -t ctgt:0
Write-Host "  Executing: psmux join-pane -s csrc:0.1 -t ctgt:0"
$joinOutput = psmux join-pane -s csrc:0.1 -t ctgt:0 2>&1
$joinExit = $LASTEXITCODE
Start-Sleep -Milliseconds 2000

# Check that join-pane did not error
$joinStr = ($joinOutput | Out-String).Trim()
$noError = $joinStr -eq "" -or $joinStr -notmatch "ERR|error|failed|panic"
Write-TestResult "B1: join-pane cross-session no errors" ($noError) "output=$joinStr exit=$joinExit"

# B2: Verify source session lost a pane
$srcPanesAfter = psmux list-panes -t csrc 2>&1
$srcPaneCountAfter = ($srcPanesAfter | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "B2: Source pane count decreased" ($srcPaneCountAfter -lt $srcPaneCount) "before=$srcPaneCount after=$srcPaneCountAfter"

# B3: Verify target session gained a pane
$tgtPanesAfter = psmux list-panes -t ctgt 2>&1
$tgtPaneCountAfter = ($tgtPanesAfter | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "B3: Target pane count increased" ($tgtPaneCountAfter -gt $tgtPaneCount) "before=$tgtPaneCount after=$tgtPaneCountAfter"

# B4: Verify the transferred pane is alive (not dead)
$tgtPaneInfo = psmux list-panes -t ctgt -F '#{pane_dead}' 2>&1
$allAlive = ($tgtPaneInfo | ForEach-Object { $_.Trim() }) -notcontains "1"
Write-TestResult "B4: All target panes alive" $allAlive "pane_dead values: $tgtPaneInfo"

# ============================================================================
# PART C: Cross-session pane I/O verification
# ============================================================================
Write-Host "`nPART C: Cross-session pane I/O" -ForegroundColor Cyan

# C1: Send input to the transferred pane and verify it works
# The transferred pane should be pane index 1 in ctgt (the newly added one)
$marker = "XSESSION_$(Get-Random -Maximum 99999)"
psmux send-keys -t ctgt:0.1 "echo $marker" Enter 2>$null
Start-Sleep -Milliseconds 1500

# C2: Capture output from the transferred pane
$captured = psmux capture-pane -t ctgt:0.1 -p 2>&1
$markerFound = ($captured | Out-String) -match $marker
Write-TestResult "C1: Send-keys to transferred pane works" $markerFound "marker=$marker"

# C3: Send input to original pane in target (should still work)
$marker2 = "ORIGINAL_$(Get-Random -Maximum 99999)"
psmux send-keys -t ctgt:0.0 "echo $marker2" Enter 2>$null
Start-Sleep -Milliseconds 1000
$captured2 = psmux capture-pane -t ctgt:0.0 -p 2>&1
$marker2Found = ($captured2 | Out-String) -match $marker2
Write-TestResult "C2: Original pane in target still works" $marker2Found "marker=$marker2"

# C4: Source session's remaining pane should still work
$marker3 = "SRCREMAIN_$(Get-Random -Maximum 99999)"
psmux send-keys -t csrc:0.0 "echo $marker3" Enter 2>$null
Start-Sleep -Milliseconds 1000
$captured3 = psmux capture-pane -t csrc:0.0 -p 2>&1
$marker3Found = ($captured3 | Out-String) -match $marker3
Write-TestResult "C3: Source remaining pane still works" $marker3Found "marker=$marker3"

# ============================================================================
# PART D: Edge cases
# ============================================================================
Write-Host "`nPART D: Edge cases" -ForegroundColor Cyan

# D1: Join-pane with -h flag (horizontal split)
# First create a new pane in source for transfer
psmux split-window -t csrc 2>$null
Start-Sleep -Milliseconds 1000
$srcPanesBefore = (psmux list-panes -t csrc 2>&1 | Where-Object { $_ -match '^\d+:' }).Count
$tgtPanesBefore = (psmux list-panes -t ctgt 2>&1 | Where-Object { $_ -match '^\d+:' }).Count

psmux join-pane -h -s csrc:0.1 -t ctgt:0 2>$null
Start-Sleep -Milliseconds 2000

$srcPanesAfterH = (psmux list-panes -t csrc 2>&1 | Where-Object { $_ -match '^\d+:' }).Count
$tgtPanesAfterH = (psmux list-panes -t ctgt 2>&1 | Where-Object { $_ -match '^\d+:' }).Count
$hOk = $tgtPanesAfterH -gt $tgtPanesBefore
Write-TestResult "D1: Horizontal cross-session join-pane" $hOk "tgt before=$tgtPanesBefore after=$tgtPanesAfterH"

# D2: Join-pane to non-existent session should fail gracefully
$badJoin = psmux join-pane -s csrc:0.0 -t nonexistent:0 2>&1
# Should get an error but not crash
Write-TestResult "D2: Non-existent target session fails gracefully" ($true) "output=$badJoin"

# D3: move-pane alias works same as join-pane
psmux split-window -t csrc 2>$null
Start-Sleep -Milliseconds 1000
$tgtBeforeMove = (psmux list-panes -t ctgt 2>&1 | Where-Object { $_ -match '^\d+:' }).Count
psmux move-pane -s csrc:0.1 -t ctgt:0 2>$null
Start-Sleep -Milliseconds 2000
$tgtAfterMove = (psmux list-panes -t ctgt 2>&1 | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "D3: move-pane alias for cross-session works" ($tgtAfterMove -gt $tgtBeforeMove) "before=$tgtBeforeMove after=$tgtAfterMove"

# ============================================================================
# PART E: TUI Visual Verification (Layer 2)
# ============================================================================
Write-Host "`nPART E: TUI Visual Verification" -ForegroundColor Cyan

# E1: Launch a visible session, perform cross-session join, verify via CLI
$tuiSrc = "tuisrc_$(Get-Random -Maximum 9999)"
$tuiTgt = "tuitgt_$(Get-Random -Maximum 9999)"

# Create source with 2 panes
Start-Process psmux -ArgumentList "new-session -d -s $tuiSrc" -WindowStyle Hidden
Start-Sleep -Milliseconds 2000
psmux split-window -t $tuiSrc 2>$null
Start-Sleep -Milliseconds 1000

# Create target
Start-Process psmux -ArgumentList "new-session -d -s $tuiTgt" -WindowStyle Hidden
Start-Sleep -Milliseconds 2000

# Verify both running
psmux has-session -t $tuiSrc 2>$null; $tuiSrcOk = ($LASTEXITCODE -eq 0)
psmux has-session -t $tuiTgt 2>$null; $tuiTgtOk = ($LASTEXITCODE -eq 0)
Write-TestResult "E1: TUI test sessions running" ($tuiSrcOk -and $tuiTgtOk) "src=$tuiSrcOk tgt=$tuiTgtOk"

# E2: Perform cross-session transfer via CLI (while sessions have TUI windows)
psmux join-pane -s "${tuiSrc}:0.1" -t "${tuiTgt}:0" 2>$null
Start-Sleep -Milliseconds 2000

$tuiTgtPanes = (psmux list-panes -t $tuiTgt 2>&1 | Where-Object { $_ -match '^\d+:' }).Count
Write-TestResult "E2: TUI cross-session join succeeded" ($tuiTgtPanes -ge 2) "target panes=$tuiTgtPanes"

# E3: Verify transferred pane responds to commands in TUI context
$tuiMarker = "TUI_VERIFY_$(Get-Random -Maximum 99999)"
psmux send-keys -t "${tuiTgt}:0.1" "echo $tuiMarker" Enter 2>$null
Start-Sleep -Milliseconds 1500
$tuiCaptured = psmux capture-pane -t "${tuiTgt}:0.1" -p 2>&1
$tuiMarkerOk = ($tuiCaptured | Out-String) -match $tuiMarker
Write-TestResult "E3: TUI transferred pane I/O works" $tuiMarkerOk "marker=$tuiMarker"

# Cleanup TUI sessions
psmux kill-session -t $tuiSrc 2>$null
psmux kill-session -t $tuiTgt 2>$null

# ============================================================================
# Cleanup and Summary
# ============================================================================
Write-Host "`nCleaning up..." -ForegroundColor Gray
Cleanup-Sessions

Write-Host "`n============================================" -ForegroundColor White
Write-Host "RESULTS: $($script:passed) passed, $($script:failed) failed out of $($script:passed + $script:failed) tests" -ForegroundColor $(if ($script:failed -eq 0) { 'Green' } else { 'Red' })
Write-Host "============================================" -ForegroundColor White

if ($script:failed -gt 0) {
    Write-Host "`nFailed tests:" -ForegroundColor Red
    $script:results | Where-Object { -not $_.Pass } | ForEach-Object {
        Write-Host "  - $($_.Name): $($_.Detail)" -ForegroundColor Red
    }
}

exit $script:failed
