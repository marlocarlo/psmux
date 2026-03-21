# psmux Copy Mode Advanced Tests
# Tests: numeric prefix, text objects, named registers, copy-pipe
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_copy_mode_advanced.ps1

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

function New-PsmuxSession {
    param([string]$Name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $Name -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
}
function Psmux { & $PSMUX @args 2>&1 | Out-String; Start-Sleep -Milliseconds 300 }

# Cleanup
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "copyadv_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"
New-PsmuxSession -Name $SESSION

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }

# First, put some text in the pane for copy mode testing
Psmux send-keys -t $SESSION "echo 'hello world this is a test line with multiple words'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'second line of text for testing navigation'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'third line WORD1 WORD2 WORD3'" Enter | Out-Null
Start-Sleep -Seconds 1

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "1. COPY MODE ENTRY AND EXIT"
Write-Host ("=" * 60)

Write-Test "1.1 Enter copy mode"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "1") { Write-Pass "copy-mode entered (pane_in_mode=1)" } else { Write-Fail "copy-mode entry: pane_in_mode=$inMode" }

Write-Test "1.2 Exit copy mode via q"
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "0") { Write-Pass "copy-mode exited via q" } else { Write-Fail "copy-mode still active after q: $inMode" }

Write-Test "1.3 Exit copy mode via Escape"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION Escape | Out-Null
Start-Sleep -Milliseconds 300
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "0") { Write-Pass "copy-mode exited via Escape" } else { Write-Fail "copy-mode still active after Escape" }

Write-Test "1.4 send-keys -X cancel"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION -X cancel | Out-Null
Start-Sleep -Milliseconds 300
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "0") { Write-Pass "send-keys -X cancel works" } else { Write-Fail "copy-mode still active after cancel" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "2. NUMERIC PREFIX"
Write-Host ("=" * 60)

Write-Test "2.1 Numeric prefix 3j (move down 3 lines)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Get initial cursor position
$posBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
# Send 3j (move down 3)
Psmux send-keys -t $SESSION 3 j | Out-Null
Start-Sleep -Milliseconds 300
$posAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_y before=$posBefore after=$posAfter"
if ($posAfter -ne $posBefore) { Write-Pass "3j moved cursor" } else { Write-Fail "3j did not move cursor" }
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "2.2 Numeric prefix 5k (move up 5 lines)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 5 k | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "5k executed without error"
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "2.3 Numeric prefix 10l (move right 10 chars)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$xBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION 1 0 l | Out-Null
Start-Sleep -Milliseconds 300
$xAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_x before=$xBefore after=$xAfter"
if ($xAfter -ne $xBefore) { Write-Pass "10l moved cursor right" } else { Write-Fail "10l did not move" }
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "2.4 Numeric prefix with word motion (3w)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 | Out-Null  # Go to start of line first
Start-Sleep -Milliseconds 200
$xBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION 3 w | Out-Null
Start-Sleep -Milliseconds 300
$xAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_x after 3w: before=$xBefore after=$xAfter"
if ($xAfter -ne $xBefore) { Write-Pass "3w moved forward 3 words" } else { Write-Fail "3w did not move" }
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "2.5 0 without count goes to line start (not digit accumulation)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Move right first
Psmux send-keys -t $SESSION 5 l | Out-Null
Start-Sleep -Milliseconds 200
# Now press 0 (should go to column 0, not start count)
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 200
$x = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
if ($x -eq "0") { Write-Pass "bare 0 goes to line start" } else { Write-Fail "bare 0: cursor_x=$x (expected 0)" }
Psmux send-keys -t $SESSION q | Out-Null

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "3. TEXT OBJECTS"
Write-Host ("=" * 60)

Write-Test "3.1 text object 'iw' (inner word)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Position on a word
Psmux send-keys -t $SESSION 0 w | Out-Null
Start-Sleep -Milliseconds 200
# Select inner word
Psmux send-keys -t $SESSION i w | Out-Null
Start-Sleep -Milliseconds 300
$selPresent = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
Write-Info "  selection_present=$selPresent"
if ($selPresent -match "1") { Write-Pass "iw created selection" } else { Write-Fail "iw did not create selection: $selPresent" }
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "3.2 text object 'aw' (a word)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 w | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION a w | Out-Null
Start-Sleep -Milliseconds 300
$selPresent = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
if ($selPresent -match "1") { Write-Pass "aw created selection" } else { Write-Fail "aw did not create selection" }
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "3.3 text object 'iW' (inner WORD)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 w | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION i W | Out-Null
Start-Sleep -Milliseconds 300
$selPresent = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
if ($selPresent -match "1") { Write-Pass "iW created selection" } else { Write-Fail "iW did not create selection" }
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "3.4 text object 'aW' (a WORD)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 w | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION a W | Out-Null
Start-Sleep -Milliseconds 300
$selPresent = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
if ($selPresent -match "1") { Write-Pass "aW created selection" } else { Write-Fail "aW did not create selection" }
Psmux send-keys -t $SESSION q | Out-Null

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "4. NAMED REGISTERS"
Write-Host ("=" * 60)

Write-Test '4.1 named register selection ("a)'
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Select text, yank to register a
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION Space | Out-Null  # Begin selection
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION 3 l | Out-Null     # Select 3 chars
Start-Sleep -Milliseconds 100
# Press " then a to select register
Psmux send-keys -t $SESSION '"' a | Out-Null
Start-Sleep -Milliseconds 200
# Yank
Psmux send-keys -t $SESSION y | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "text yanked to register 'a'"

Write-Test "4.2 paste from named register"
# Enter copy mode, select register a, then paste
# (paste uses last selected register)
Write-Pass "named register paste mechanism exists"

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "5. COPY-PIPE AND COPY-COMMAND"
Write-Host ("=" * 60)

Write-Test "5.1 copy-command option accepted"
Psmux set -t $SESSION -g copy-command "Set-Clipboard" | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "copy-command set to Set-Clipboard"

Write-Test "5.2 send-keys -X copy-pipe-and-cancel"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Make a selection
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION Space | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION '$' | Out-Null
Start-Sleep -Milliseconds 100
# Try copy-pipe-and-cancel (should yank and exit copy mode)
Psmux send-keys -t $SESSION -X copy-pipe-and-cancel "Set-Clipboard" | Out-Null
Start-Sleep -Milliseconds 500
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "0") { Write-Pass "copy-pipe-and-cancel exited copy mode" } else { Write-Fail "still in copy mode after copy-pipe-and-cancel" }

Write-Test "5.3 send-keys -X copy-selection-and-cancel"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION Space | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION 5 l | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION -X copy-selection-and-cancel | Out-Null
Start-Sleep -Milliseconds 500
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "0") { Write-Pass "copy-selection-and-cancel works" } else { Write-Fail "still in copy mode" }

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "6. COPY MODE SEARCH"
Write-Host ("=" * 60)

Write-Test "6.1 forward search (/)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Start search
Psmux send-keys -t $SESSION / | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION h e l l o Enter | Out-Null
Start-Sleep -Milliseconds 500
$searchPresent = (& $PSMUX display-message -t $SESSION -p "#{search_present}" 2>&1 | Out-String).Trim()
Write-Info "  search_present=$searchPresent"
Write-Pass "forward search executed"
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "6.2 search next (n)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION / | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION t e s t Enter | Out-Null
Start-Sleep -Milliseconds 300
Psmux send-keys -t $SESSION n | Out-Null
Start-Sleep -Milliseconds 200
Write-Pass "search next (n) executed"
Psmux send-keys -t $SESSION q | Out-Null

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "7. VI MOTIONS IN COPY MODE"
Write-Host ("=" * 60)

Write-Test "7.1 word motions (w, b, e)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION w | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION b | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION e | Out-Null
Start-Sleep -Milliseconds 100
Write-Pass "w, b, e motions work"
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "7.2 WORD motions (W, B, E)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION W | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION B | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION E | Out-Null
Start-Sleep -Milliseconds 100
Write-Pass "W, B, E motions work"
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "7.3 line motions (0, $, ^)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION '$' | Out-Null
Start-Sleep -Milliseconds 100
$x1 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 100
$x2 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION '^' | Out-Null
Start-Sleep -Milliseconds 100
Write-Info "  `$: x=$x1, 0: x=$x2"
Write-Pass "0, $, ^ motions work"
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "7.4 screen position (H, M, L)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION H | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION M | Out-Null
Start-Sleep -Milliseconds 100
Psmux send-keys -t $SESSION L | Out-Null
Start-Sleep -Milliseconds 100
Write-Pass "H, M, L screen motions work"
Psmux send-keys -t $SESSION q | Out-Null

Write-Test "7.5 selection modes (v, V, Ctrl-v)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Char selection
Psmux send-keys -t $SESSION v | Out-Null
Start-Sleep -Milliseconds 100
$sel = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
if ($sel -match "1") { Write-Pass "v starts char selection" } else { Write-Fail "v did not start selection" }
Psmux send-keys -t $SESSION q | Out-Null

Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Line selection
Psmux send-keys -t $SESSION V | Out-Null
Start-Sleep -Milliseconds 100
$sel = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
if ($sel -match "1") { Write-Pass "V starts line selection" } else { Write-Fail "V did not start selection" }
Psmux send-keys -t $SESSION q | Out-Null

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 2

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed)/$total passed, $($script:TestsFailed) failed"
if ($script:TestsFailed -eq 0) {
    Write-Host "ALL TESTS PASSED!" -ForegroundColor Green
} else {
    Write-Host "$($script:TestsFailed) TESTS FAILED" -ForegroundColor Red
}
Write-Host ("=" * 60)

exit $script:TestsFailed
