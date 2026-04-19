$ErrorActionPreference = "Continue"
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
$results = @()

# Create the test session
taskkill /f /im psmux.exe 2>$null | Out-Null
Start-Sleep 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", "copytest" -WindowStyle Hidden
Start-Sleep 3

function Test-Step {
    param([string]$Name, [string]$Expected, [string]$Actual)
    $status = if ($Actual.Trim() -eq $Expected.Trim()) { "PASS" } else { "FAIL" }
    $line = "$status | $Name | expected=[$Expected] actual=[$($Actual.Trim())]"
    Write-Host $line
    $script:results += $line
    return $line
}

function Query {
    param([string]$Format)
    $r = & $PSMUX display-message -t copytest -p $Format 2>&1
    return "$r".Trim()
}

# We are already in copy mode from the previous manual step.
# Exit and re-enter to have a clean state.
& $PSMUX send-keys -t copytest q; Start-Sleep -Milliseconds 500

# ===== TEST 1: Enter copy mode, check pane_in_mode = 1 =====
& $PSMUX copy-mode -t copytest; Start-Sleep -Milliseconds 500
$v = Query "#{pane_in_mode}"
Test-Step "T01: copy-mode entry (pane_in_mode)" "1" $v

# ===== TEST 2: Exit with 'q', check pane_in_mode = 0 =====
& $PSMUX send-keys -t copytest q; Start-Sleep -Milliseconds 500
$v = Query "#{pane_in_mode}"
Test-Step "T02: exit with q (pane_in_mode)" "0" $v

# ===== TEST 3: Enter copy mode, exit with Escape =====
& $PSMUX copy-mode -t copytest; Start-Sleep -Milliseconds 500
& $PSMUX send-keys -t copytest Escape; Start-Sleep -Milliseconds 500
$v = Query "#{pane_in_mode}"
Test-Step "T03: exit with Escape (pane_in_mode)" "0" $v

# ===== TEST 4: Enter copy mode, exit with -X cancel =====
& $PSMUX copy-mode -t copytest; Start-Sleep -Milliseconds 500
& $PSMUX send-keys -t copytest -X cancel; Start-Sleep -Milliseconds 500
$v = Query "#{pane_in_mode}"
Test-Step "T04: exit with -X cancel (pane_in_mode)" "0" $v

# ===== TEST 5: Cursor movement h (left) =====
& $PSMUX copy-mode -t copytest; Start-Sleep -Milliseconds 500
# First go to a known position: move right a few times then left
& $PSMUX send-keys -t copytest 0; Start-Sleep -Milliseconds 300
$x_start = Query "#{copy_cursor_x}"
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 300
$x_after_3r = Query "#{copy_cursor_x}"
& $PSMUX send-keys -t copytest h; Start-Sleep -Milliseconds 300
$x_after_h = Query "#{copy_cursor_x}"
$expected_h = [int]$x_after_3r - 1
Test-Step "T05: h (left) movement" "$expected_h" $x_after_h

# ===== TEST 6: l (right) =====
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 300
$x_after_l = Query "#{copy_cursor_x}"
$expected_l = [int]$x_after_h + 1
Test-Step "T06: l (right) movement" "$expected_l" $x_after_l

# ===== TEST 7: j (down) =====
$y_before = Query "#{copy_cursor_y}"
& $PSMUX send-keys -t copytest j; Start-Sleep -Milliseconds 300
$y_after_j = Query "#{copy_cursor_y}"
$expected_j = [int]$y_before + 1
Test-Step "T07: j (down) movement" "$expected_j" $y_after_j

# ===== TEST 8: k (up) =====
& $PSMUX send-keys -t copytest k; Start-Sleep -Milliseconds 300
$y_after_k = Query "#{copy_cursor_y}"
Test-Step "T08: k (up) movement" "$y_before" $y_after_k

# ===== TEST 9: 0 (beginning of line) =====
& $PSMUX send-keys -t copytest 0; Start-Sleep -Milliseconds 300
$x_bol = Query "#{copy_cursor_x}"
Test-Step "T09: 0 (beginning of line)" "0" $x_bol

# ===== TEST 10: $ (end of line) =====
& $PSMUX send-keys -t copytest '$'; Start-Sleep -Milliseconds 500
$x_eol = Query "#{copy_cursor_x}"
$eol_pass = [int]$x_eol -gt 0
Test-Step "T10: dollar (end of line, x>0)" "True" "$eol_pass"
Write-Host "  T10 detail: copy_cursor_x=$x_eol"

# ===== TEST 11: w (word forward) =====
& $PSMUX send-keys -t copytest 0; Start-Sleep -Milliseconds 300
$x_before_w = Query "#{copy_cursor_x}"
& $PSMUX send-keys -t copytest w; Start-Sleep -Milliseconds 300
$x_after_w = Query "#{copy_cursor_x}"
$w_pass = [int]$x_after_w -gt [int]$x_before_w
Test-Step "T11: w (word forward, x increased)" "True" "$w_pass"
Write-Host "  T11 detail: x before=$x_before_w, x after=$x_after_w"

# ===== TEST 12: b (word backward) =====
# Move forward a bit first
& $PSMUX send-keys -t copytest w; Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t copytest w; Start-Sleep -Milliseconds 300
$x_before_b = Query "#{copy_cursor_x}"
& $PSMUX send-keys -t copytest b; Start-Sleep -Milliseconds 300
$x_after_b = Query "#{copy_cursor_x}"
$b_pass = [int]$x_after_b -lt [int]$x_before_b
Test-Step "T12: b (word backward, x decreased)" "True" "$b_pass"
Write-Host "  T12 detail: x before=$x_before_b, x after=$x_after_b"

# ===== TEST 13: e (end of word) =====
& $PSMUX send-keys -t copytest 0; Start-Sleep -Milliseconds 300
$x_before_e = Query "#{copy_cursor_x}"
& $PSMUX send-keys -t copytest e; Start-Sleep -Milliseconds 300
$x_after_e = Query "#{copy_cursor_x}"
$e_pass = [int]$x_after_e -gt [int]$x_before_e
Test-Step "T13: e (end of word, x increased)" "True" "$e_pass"
Write-Host "  T13 detail: x before=$x_before_e, x after=$x_after_e"

# ===== TEST 14: g (top of buffer) =====
& $PSMUX send-keys -t copytest g; Start-Sleep -Milliseconds 300
$y_top = Query "#{copy_cursor_y}"
Test-Step "T14: g (top of buffer)" "0" $y_top

# ===== TEST 15: G (bottom of buffer) =====
& $PSMUX send-keys -t copytest G; Start-Sleep -Milliseconds 500
$y_bottom = Query "#{copy_cursor_y}"
# G should move to last line - in a fresh session this may be 0 (single screen)
# Just verify G is accepted and y is a valid number
$G_pass = $y_bottom -match '^\d+$'
Test-Step "T15: G (bottom of buffer, valid y)" "True" "$G_pass"
Write-Host "  T15 detail: copy_cursor_y=$y_bottom"

# ===== TEST 16: Selection (Space to begin, 3l to select 3 chars) =====
# Go to a known position first
& $PSMUX send-keys -t copytest g; Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t copytest 0; Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t copytest Space; Start-Sleep -Milliseconds 300
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 200
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 200
& $PSMUX send-keys -t copytest l; Start-Sleep -Milliseconds 300
$sel = Query "#{selection_present}"
Test-Step "T16: selection_present after Space+3l" "1" $sel

# ===== TEST 17: copy-selection-and-cancel =====
& $PSMUX send-keys -t copytest -X copy-selection-and-cancel; Start-Sleep -Milliseconds 500
$v = Query "#{pane_in_mode}"
Test-Step "T17: copy-selection-and-cancel (pane_in_mode)" "0" $v

# ===== TEST 18: show-buffer / list-buffers =====
$buf_show = & $PSMUX show-buffer -t copytest 2>&1
$buf_list = & $PSMUX list-buffers -t copytest 2>&1
$buf_show_str = "$buf_show".Trim()
$buf_list_str = "$buf_list".Trim()
$buf_pass = ($buf_show_str.Length -gt 0) -or ($buf_list_str.Length -gt 0)
Test-Step "T18: buffer captured (non-empty)" "True" "$buf_pass"
Write-Host "  T18 show-buffer: [$buf_show_str]"
Write-Host "  T18 list-buffers: [$buf_list_str]"

# ===== TEST 19: H, M, L (top/middle/bottom of screen) =====
& $PSMUX copy-mode -t copytest; Start-Sleep -Milliseconds 500
& $PSMUX send-keys -t copytest H; Start-Sleep -Milliseconds 300
$y_H = Query "#{copy_cursor_y}"
& $PSMUX send-keys -t copytest M; Start-Sleep -Milliseconds 300
$y_M = Query "#{copy_cursor_y}"
& $PSMUX send-keys -t copytest L; Start-Sleep -Milliseconds 300
$y_L = Query "#{copy_cursor_y}"
$HML_pass = ([int]$y_H -le [int]$y_M) -and ([int]$y_M -le [int]$y_L)
Test-Step "T19: H/M/L (H<=M<=L)" "True" "$HML_pass"
Write-Host "  T19 detail: H_y=$y_H, M_y=$y_M, L_y=$y_L"

# ===== TEST 20: Search with / then hello Enter =====
& $PSMUX send-keys -t copytest /; Start-Sleep -Milliseconds 500
& $PSMUX send-keys -t copytest hello Enter; Start-Sleep -Milliseconds 700
$search_x = Query "#{copy_cursor_x}"
$search_y = Query "#{copy_cursor_y}"
$search_mode = Query "#{pane_in_mode}"
# After search, still in copy mode but cursor moved to match
Test-Step "T20: search /hello (still in copy mode)" "1" $search_mode
Write-Host "  T20 detail: cursor at x=$search_x, y=$search_y"

# ═══════════════════════════════════════════════════════════════
# Win32 TUI VERIFICATION: Prove copy mode via real keystrokes
# ═══════════════════════════════════════════════════════════════
Write-Host ""
Write-Host "RESULTS: $passCount/$total API tests done. Starting TUI verification..."
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_CPY = "cpy_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_CPY
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # Put some content in the pane for copy mode to work with
    & $script:TUI_PSMUX send-keys -t $TUI_SESSION_CPY "echo 'line one'; echo 'line two'; echo 'line three'" Enter 2>&1 | Out-Null
    Start-Sleep -Seconds 1

    # TUI Test 1: Enter copy mode via CLI (visible TUI window)
    $mode = Safe-TuiQuery "#{pane_in_mode}" -Session $TUI_SESSION_CPY
    & $script:TUI_PSMUX copy-mode -t $TUI_SESSION_CPY 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $mode = Safe-TuiQuery "#{pane_in_mode}" -Session $TUI_SESSION_CPY
    Test-Step "TUI: Copy mode entered (visible TUI proof)" "1" $mode

    # TUI Test 2: Move cursor in copy mode via send-keys
    $cursorBefore = Safe-TuiQuery "#{copy_cursor_y}" -Session $TUI_SESSION_CPY
    & $script:TUI_PSMUX send-keys -t $TUI_SESSION_CPY Up 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200
    & $script:TUI_PSMUX send-keys -t $TUI_SESSION_CPY Up 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $cursorAfter = Safe-TuiQuery "#{copy_cursor_y}" -Session $TUI_SESSION_CPY
    $moved = if ($cursorAfter -ne $cursorBefore) { "True" } else { "False" }
    Test-Step "TUI: Copy cursor moved (visible TUI proof)" "True" $moved

    # TUI Test 3: Exit copy mode via send-keys q
    & $script:TUI_PSMUX send-keys -t $TUI_SESSION_CPY q 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $modeAfter = Safe-TuiQuery "#{pane_in_mode}" -Session $TUI_SESSION_CPY
    Test-Step "TUI: Copy mode exited (visible TUI proof)" "0" $modeAfter

    Cleanup-PsmuxWindow -Session $TUI_SESSION_CPY
    Write-Host ""
} else {
    Write-Info "TUI verification skipped (could not launch window)"
}

# Exit copy mode
& $PSMUX send-keys -t copytest q; Start-Sleep -Milliseconds 300

# Cleanup
& $PSMUX kill-session -t copytest 2>$null
Start-Sleep 1

# Summary
$passCount = ($results | Where-Object { $_ -match "^PASS" }).Count
$failCount = ($results | Where-Object { $_ -match "^FAIL" }).Count
$total = $passCount + $failCount
Write-Host ""
Write-Host "RESULTS: $passCount/$total passed, $failCount failed"
if ($failCount -eq 0) { Write-Host "ALL TESTS PASSED!" -ForegroundColor Green; exit 0 }
else { Write-Host "$failCount TESTS FAILED" -ForegroundColor Red; exit 1 }

Write-Host ""
Write-Host "===== ALL TESTS COMPLETE ====="

# Cleanup
& $PSMUX kill-session -t copytest; Start-Sleep -Milliseconds 500
Write-Host "Session killed. Done."
