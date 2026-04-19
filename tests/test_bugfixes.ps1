$psmux = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $psmux)) { $psmux = "$PSScriptRoot\..\target\debug\psmux.exe" }
$out = "$PSScriptRoot\..\test_bugfix_results.txt"

# Clean up
taskkill /f /im psmux.exe 2>$null
Start-Sleep 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$results = @()
$results += "=== BUG FIX VERIFICATION RESULTS ==="
$results += ""

# Create session
& $psmux new-session -s fixtest -d 2>&1 | Out-Null
Start-Sleep 3
$results += "Session 'fixtest' created."
$results += ""

# ========================================
# BUG FIX 1: prefix2 None (case-insensitive)
# ========================================
$results += "=== BUG FIX 1: prefix2 None (case-insensitive) ==="

& $psmux set-option -t fixtest prefix2 C-a 2>&1 | Out-Null
Start-Sleep 0.5
$r2 = (& $psmux show-options -t fixtest -v prefix2 2>&1) | Out-String
$r2 = $r2.Trim()
$pass2 = $r2 -eq "C-a"
$results += "Test 2: show prefix2 after set C-a => [$r2] (expect C-a) => $(if($pass2){'PASS'}else{'FAIL'})"

& $psmux set-option -t fixtest prefix2 None 2>&1 | Out-Null
Start-Sleep 0.5
$r4 = (& $psmux show-options -t fixtest -v prefix2 2>&1) | Out-String
$r4 = $r4.Trim()
$pass4 = $r4 -eq "none"
$results += "Test 4: show prefix2 after set None => [$r4] (expect none) => $(if($pass4){'PASS'}else{'FAIL'})"

& $psmux set-option -t fixtest prefix2 NONE 2>&1 | Out-Null
Start-Sleep 0.5
$r6 = (& $psmux show-options -t fixtest -v prefix2 2>&1) | Out-String
$r6 = $r6.Trim()
$pass6 = $r6 -eq "none"
$results += "Test 6: show prefix2 after set NONE => [$r6] (expect none) => $(if($pass6){'PASS'}else{'FAIL'})"
$results += ""

# ========================================
# BUG FIX 2: status numeric (multi-line)
# ========================================
$results += "=== BUG FIX 2: status numeric (multi-line) ==="

& $psmux set-option -t fixtest status 2 2>&1 | Out-Null
Start-Sleep 0.5
$r8 = (& $psmux show-options -t fixtest -v status 2>&1) | Out-String
$r8 = $r8.Trim()
$pass8 = $r8 -eq "2"
$results += "Test 8: show status after set 2 => [$r8] (expect 2) => $(if($pass8){'PASS'}else{'FAIL'})"

& $psmux set-option -t fixtest status 3 2>&1 | Out-Null
Start-Sleep 0.5
$r10 = (& $psmux show-options -t fixtest -v status 2>&1) | Out-String
$r10 = $r10.Trim()
$pass10 = $r10 -eq "3"
$results += "Test 10: show status after set 3 => [$r10] (expect 3) => $(if($pass10){'PASS'}else{'FAIL'})"

& $psmux set-option -t fixtest status on 2>&1 | Out-Null
Start-Sleep 0.5
$r12 = (& $psmux show-options -t fixtest -v status 2>&1) | Out-String
$r12 = $r12.Trim()
$pass12 = $r12 -eq "on"
$results += "Test 12: show status after set on => [$r12] (expect on) => $(if($pass12){'PASS'}else{'FAIL'})"

& $psmux set-option -t fixtest status off 2>&1 | Out-Null
Start-Sleep 0.5
$r14 = (& $psmux show-options -t fixtest -v status 2>&1) | Out-String
$r14 = $r14.Trim()
$pass14 = $r14 -eq "off"
$results += "Test 14: show status after set off => [$r14] (expect off) => $(if($pass14){'PASS'}else{'FAIL'})"
$results += ""

# ========================================
# BUG FIX 3: main-pane-width/height in show-options -v
# ========================================
$results += "=== BUG FIX 3: main-pane-width/height in show-options -v ==="

& $psmux set-option -t fixtest main-pane-width 80 2>&1 | Out-Null
Start-Sleep 0.5
$r16 = (& $psmux show-options -t fixtest -v main-pane-width 2>&1) | Out-String
$r16 = $r16.Trim()
$pass16 = $r16 -eq "80"
$results += "Test 16: show main-pane-width after set 80 => [$r16] (expect 80) => $(if($pass16){'PASS'}else{'FAIL'})"

& $psmux set-option -t fixtest main-pane-height 25 2>&1 | Out-Null
Start-Sleep 0.5
$r18 = (& $psmux show-options -t fixtest -v main-pane-height 2>&1) | Out-String
$r18 = $r18.Trim()
$pass18 = $r18 -eq "25"
$results += "Test 18: show main-pane-height after set 25 => [$r18] (expect 25) => $(if($pass18){'PASS'}else{'FAIL'})"

$r19 = (& $psmux show-options -t fixtest 2>&1) | Out-String
$hasWidth = $r19 -match "main-pane-width"
$hasHeight = $r19 -match "main-pane-height"
$pass19 = $hasWidth -and $hasHeight
$results += "Test 19: full dump has main-pane-width=$hasWidth, main-pane-height=$hasHeight => $(if($pass19){'PASS'}else{'FAIL'})"
$results += ""

# ========================================
# BUG FIX 4: command-alias in show-options -v
# ========================================
$results += "=== BUG FIX 4: command-alias in show-options -v ==="

& $psmux set-option -t fixtest command-alias "sw=split-window" 2>&1 | Out-Null
Start-Sleep 0.5
$r21 = (& $psmux show-options -t fixtest -v command-alias 2>&1) | Out-String
$r21 = $r21.Trim()
$pass21 = $r21 -match "sw=split-window"
$results += "Test 21: show command-alias => [$r21] (expect contains sw=split-window) => $(if($pass21){'PASS'}else{'FAIL'})"
$results += ""

# ========================================
# BUG FIX 5: capture-pane -S negative offset
# ========================================
$results += "=== BUG FIX 5: capture-pane -S negative offset ==="

& $psmux send-keys -t fixtest "echo LINE1" Enter 2>&1 | Out-Null
Start-Sleep 0.5
& $psmux send-keys -t fixtest "echo LINE2" Enter 2>&1 | Out-Null
Start-Sleep 0.5
& $psmux send-keys -t fixtest "echo LINE3" Enter 2>&1 | Out-Null
Start-Sleep 1

$r22raw = (& $psmux capture-pane -t fixtest -p -S 0 -E 5 2>&1) | Out-String
$r22lines = @($r22raw -split "`n" | Where-Object { $_ -ne "" -and $_.Trim() -ne "" })
$r22count = $r22lines.Count
# With -S 0 -E 5 we expect exactly 6 lines
$pass22 = $r22count -eq 6
$results += "Test 22: capture-pane -S 0 -E 5 => $r22count lines (expect 6) => $(if($pass22){'PASS'}else{'FAIL'})"
$results += "  Content: $($r22raw.Trim().Substring(0, [Math]::Min(200, $r22raw.Trim().Length)))"

$r23raw = (& $psmux capture-pane -t fixtest -p -S -5 2>&1) | Out-String
$r23all = @($r23raw -split "`n" | Where-Object { $_ -ne $null })
$r23count = $r23all.Count
# Key check: -S -5 should return at most ~6 lines (NOT the full 30-row screen).
# The bottom rows may be empty if content hasn't scrolled that far.
$pass23 = ($r23count -ge 0) -and ($r23count -le 7)
$results += "Test 23: capture-pane -S -5 => $r23count lines (expect <=7, NOT 30) => $(if($pass23){'PASS'}else{'FAIL'})"
$results += "  Content: $($r23raw.Trim().Substring(0, [Math]::Min(200, $r23raw.Trim().Length)))"

$r24raw = (& $psmux capture-pane -t fixtest -p -S -3 2>&1) | Out-String
$r24all = @($r24raw -split "`n" | Where-Object { $_ -ne $null })
$r24count = $r24all.Count
# Key check: -S -3 should return at most ~4 lines (NOT the full 30-row screen).
$pass24 = ($r24count -ge 0) -and ($r24count -le 5)
$results += "Test 24: capture-pane -S -3 => $r24count lines (expect <=5, NOT 30) => $(if($pass24){'PASS'}else{'FAIL'})"
$results += "  Content: $($r24raw.Trim().Substring(0, [Math]::Min(200, $r24raw.Trim().Length)))"
$results += ""

# ========================================
# BUG FIX 6: SelectLayout/NextLayout state_dirty
# ========================================
$results += "=== BUG FIX 6: SelectLayout/NextLayout state_dirty ==="

$split_out = & $psmux split-window -t fixtest -h 2>&1
$ec25 = $LASTEXITCODE
Start-Sleep 1
$pass25 = $ec25 -eq 0
$results += "Test 25: split-window -h exit code => $ec25 (expect 0) => $(if($pass25){'PASS'}else{'FAIL'})"

$next_out = & $psmux next-layout -t fixtest 2>&1
$ec26 = $LASTEXITCODE
$pass26 = $ec26 -eq 0
$results += "Test 26: next-layout exit code => $ec26 (expect 0) => $(if($pass26){'PASS'}else{'FAIL'})"

$sel_out = & $psmux select-layout -t fixtest even-vertical 2>&1
$ec27 = $LASTEXITCODE
$pass27 = $ec27 -eq 0
$results += "Test 27: select-layout even-vertical exit code => $ec27 (expect 0) => $(if($pass27){'PASS'}else{'FAIL'})"
$results += ""

# ========================================
# Cleanup
# ========================================
& $psmux kill-session -t fixtest 2>&1 | Out-Null

# Count results
$total = 0
$passed = 0
$failed = 0
foreach ($line in $results) {
    if ($line -match "=> PASS$") { $total++; $passed++ }
    elseif ($line -match "=> FAIL$") { $total++; $failed++ }
}
$results += "========================================"
$results += "SUMMARY: $passed/$total PASSED, $failed FAILED"
$results += "========================================"

# Write to file and print
$results | Set-Content $out
$results | ForEach-Object { Write-Host $_ }
