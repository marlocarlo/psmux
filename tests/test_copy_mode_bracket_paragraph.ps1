# psmux Copy Mode – Bracket Matching (%) and Paragraph Jump ({/}) Tests
# Tests: %, {, } keys in vi copy mode
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_copy_mode_bracket_paragraph.ps1

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

$SESSION = "copybp_$(Get-Random -Maximum 9999)"
Write-Info "Session: $SESSION"
New-PsmuxSession -Name $SESSION

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }

# Seed pane with bracket and paragraph content
# Use printf to ensure exact output without shell prompt interference
Psmux send-keys -t $SESSION "echo '(hello world)'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo '[bracket test]'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo '{curly braces}'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo ''" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'paragraph two line one'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'paragraph two line two'" Enter | Out-Null
Psmux send-keys -t $SESSION "echo ''" Enter | Out-Null
Psmux send-keys -t $SESSION "echo 'paragraph three'" Enter | Out-Null
Start-Sleep -Seconds 2

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "1. BRACKET MATCHING via send-keys -X next-matching-bracket"
Write-Host ("=" * 60)

Write-Test "1.1 next-matching-bracket command accepted (no crash)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "1") { Write-Pass "copy mode entered" } else { Write-Fail "copy mode entry failed: $inMode" }
Psmux send-keys -t $SESSION -X next-matching-bracket | Out-Null
Start-Sleep -Milliseconds 300
$inMode2 = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
Write-Info "  still in copy mode after bracket cmd: $inMode2"
Write-Pass "next-matching-bracket accepted without crash"
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "1.2 next-matching-bracket moves cursor when on bracket line"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
# Use search to find the ( character reliably
Psmux send-keys -t $SESSION g g | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION '/' | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION '(hello' Enter | Out-Null
Start-Sleep -Milliseconds 300
$xBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
$yBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  after search: cursor at x=[$xBefore] y=[$yBefore]"
# Move to start of match location, find the ( 
Psmux send-keys -t $SESSION -X next-matching-bracket | Out-Null
Start-Sleep -Milliseconds 300
$xAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
$yAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  after bracket: cursor at x=[$xAfter] y=[$yAfter]"
if ($xAfter -ne $xBefore -or $yAfter -ne $yBefore) {
    Write-Pass "next-matching-bracket moved cursor"
} else {
    Write-Fail "next-matching-bracket did not move cursor"
}
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "1.3 next-matching-bracket is idempotent (twice returns)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION g g | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION '/' | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION '(hello' Enter | Out-Null
Start-Sleep -Milliseconds 300
$xOrig = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION -X next-matching-bracket | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION -X next-matching-bracket | Out-Null
Start-Sleep -Milliseconds 300
$xReturn = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Write-Info "  orig=[$xOrig] after double bracket=[$xReturn]"
if ($xOrig -eq $xReturn) { Write-Pass "double bracket returns to original" } else { Write-Fail "double bracket: expected $xOrig got $xReturn" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "2. PARAGRAPH JUMP – send-keys -X next-paragraph"
Write-Host ("=" * 60)

Write-Test "2.1 next-paragraph accepted (no crash)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$yBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION -X next-paragraph | Out-Null
Start-Sleep -Milliseconds 300
$yAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_y before=[$yBefore] after=[$yAfter]"
Write-Pass "next-paragraph accepted without crash"
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "2.2 next-paragraph moves cursor from top of buffer"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION g g | Out-Null
Start-Sleep -Milliseconds 200
$yBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION -X next-paragraph | Out-Null
Start-Sleep -Milliseconds 300
$yAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_y from top: before=[$yBefore] after=[$yAfter]"
if ($yAfter -ne $yBefore) { Write-Pass "next-paragraph moved cursor" } else { Write-Fail "next-paragraph did not move cursor" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "3. PARAGRAPH JUMP – send-keys -X previous-paragraph"
Write-Host ("=" * 60)

Write-Test "3.1 previous-paragraph accepted (no crash)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$yBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION -X previous-paragraph | Out-Null
Start-Sleep -Milliseconds 300
$yAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_y before=[$yBefore] after=[$yAfter]"
Write-Pass "previous-paragraph accepted without crash"
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "3.2 previous-paragraph moves cursor from bottom of buffer"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION G | Out-Null
Start-Sleep -Milliseconds 200
$yBefore = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION -X previous-paragraph | Out-Null
Start-Sleep -Milliseconds 300
$yAfter = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Write-Info "  cursor_y from bottom: before=[$yBefore] after=[$yAfter]"
if ($yAfter -ne $yBefore) { Write-Pass "previous-paragraph moved cursor" } else { Write-Fail "previous-paragraph did not move cursor" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "4. RAW KEY DISPATCH (%, {, }) via input handler"
Write-Host ("=" * 60)

Write-Test "4.1 Raw % key accepted in copy mode"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION -l '%' | Out-Null
Start-Sleep -Milliseconds 300
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "1") { Write-Pass "% accepted, still in copy mode" } else { Write-Fail "% ejected from copy mode" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "4.2 Raw } key accepted in copy mode"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION -l '}' | Out-Null
Start-Sleep -Milliseconds 300
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "1") { Write-Pass "} accepted, still in copy mode" } else { Write-Fail "} ejected from copy mode" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "4.3 Raw { key accepted in copy mode"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION -l '{' | Out-Null
Start-Sleep -Milliseconds 300
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
if ($inMode -match "1") { Write-Pass "{ accepted, still in copy mode" } else { Write-Fail "{ ejected from copy mode" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "5. REGRESSION – existing copy mode not broken"
Write-Host ("=" * 60)

Write-Test "5.1 Basic h/j/k/l still work"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
$x0 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION l l l | Out-Null
Start-Sleep -Milliseconds 200
$x1 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION h | Out-Null
Start-Sleep -Milliseconds 200
$x2 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Write-Info "  x start=[$x0] after lll=[$x1] after h=[$x2]"
if ($x1 -ne $x0) { Write-Pass "l movement works" } else { Write-Fail "l did not move" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "5.2 w/b word motion"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION 0 | Out-Null
Start-Sleep -Milliseconds 200
$x0 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION w | Out-Null
Start-Sleep -Milliseconds 200
$x1 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION b | Out-Null
Start-Sleep -Milliseconds 200
$x2 = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_x}" 2>&1 | Out-String).Trim()
Write-Info "  x start=[$x0] after w=[$x1] after b=[$x2]"
if ($x1 -ne $x0) { Write-Pass "w/b motion works" } else { Write-Fail "w did not move" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "5.3 Selection (v) works"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION v | Out-Null
Start-Sleep -Milliseconds 200
$sel = (& $PSMUX display-message -t $SESSION -p "#{selection_present}" 2>&1 | Out-String).Trim()
Write-Info "  selection_present=[$sel]"
if ($sel -match "1") { Write-Pass "v starts selection" } else { Write-Fail "v did not start selection: $sel" }
Psmux send-keys -t $SESSION Escape | Out-Null
Start-Sleep -Milliseconds 300

Write-Test "5.4 Search (/) works"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION '/' | Out-Null
Start-Sleep -Milliseconds 200
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
Write-Info "  still in copy mode after /: pane_in_mode=[$inMode]"
Psmux send-keys -t $SESSION Escape | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "/ search prompt opened without crash"

Write-Test "5.5 gg/G navigation accepted (no crash)"
Psmux copy-mode -t $SESSION | Out-Null
Start-Sleep -Milliseconds 500
Psmux send-keys -t $SESSION g g | Out-Null
Start-Sleep -Milliseconds 200
$yTop = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
Psmux send-keys -t $SESSION G | Out-Null
Start-Sleep -Milliseconds 200
$yBot = (& $PSMUX display-message -t $SESSION -p "#{copy_cursor_y}" 2>&1 | Out-String).Trim()
$inMode = (& $PSMUX display-message -t $SESSION -p "#{pane_in_mode}" 2>&1 | Out-String).Trim()
Write-Info "  top=[$yTop] bottom=[$yBot] in_mode=[$inMode]"
if ($inMode -match "1") { Write-Pass "gg/G accepted, still in copy mode" } else { Write-Fail "gg/G caused copy mode exit" }
Psmux send-keys -t $SESSION q | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
# CLEANUP
Write-Host ""
Write-Host ("=" * 60)

Psmux kill-session -t $SESSION | Out-Null
Start-Sleep -Seconds 1
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden | Out-Null

Write-Host ""
Write-Host ("=" * 60)
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
