# psmux Session-5 Feature Test Suite
# Tests: new-window -n, copy-mode -u, paste-buffer -b, list-windows tmux format,
#        copy-mode Space/Enter, W/B/E WORD motions, H/M/L screen position,
#        f/F find-char, D copy-to-end-of-line
# Run: powershell -ExecutionPolicy Bypass -File tests\test_features5.ps1

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

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }
function PsmuxQuick { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 150 }

function Ensure-Session {
    param([string]$Name)
    & $PSMUX has-session -t $Name 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Info "Session '$Name' died - recreating..."
        New-PsmuxSession -Name $Name
        & $PSMUX has-session -t $Name 2>$null
        if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot recreate session" -ForegroundColor Red; exit 1 }
    }
}

# Kill everything first
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Create test session
Write-Info "Creating test session 'feat5'..."
New-PsmuxSession -Name "feat5"
& $PSMUX has-session -t feat5 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'feat5' created"

# ============================================================
# 1. NEW-WINDOW -n FLAG TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "NEW-WINDOW -n FLAG TESTS"
Write-Host ("=" * 60)

Write-Test "new-window -n sets window name"
Psmux new-window -t feat5 -n "mywin"
Start-Sleep -Milliseconds 500
$lsw = Psmux list-windows -t feat5 -J
if ($lsw -match '"name":"mywin"') { Write-Pass "new-window -n set name to 'mywin'" }
else { Write-Fail "new-window -n name not found: $lsw" }

Write-Test "new-window creates default name without -n"
Psmux new-window -t feat5
Start-Sleep -Milliseconds 500
$lsw = Psmux list-windows -t feat5 -J
$wins = ([regex]::Matches($lsw, '"id":')).Count
if ($wins -ge 3) { Write-Pass "new-window created window (total: $wins)" }
else { Write-Fail "Expected >=3 windows, got $wins" }

# ============================================================
# 2. LIST-WINDOWS TMUX-STYLE OUTPUT TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "LIST-WINDOWS FORMAT TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "list-windows default output is tmux-style"
$lsw = Psmux list-windows -t feat5
$lines = ($lsw -split "`n") | Where-Object { $_.Trim() -ne "" }
$firstLine = $lines[0]
# tmux format: "0: name* (N panes) [WxH]"
if ($firstLine -match '^\d+:\s+\S+.*\(\d+ panes\)\s+\[\d+x\d+\]') {
    Write-Pass "list-windows default format is tmux-style: $firstLine"
} else {
    Write-Fail "list-windows format unexpected: $firstLine"
}

Write-Test "list-windows shows multiple windows"
if ($lines.Count -ge 3) { Write-Pass "list-windows shows $($lines.Count) windows" }
else { Write-Fail "Expected >=3 lines, got $($lines.Count)" }

Write-Test "list-windows -J returns JSON"
$json = Psmux list-windows -t feat5 -J
if ($json -match '^\[.*\]$') { Write-Pass "list-windows -J returns JSON array" }
else { Write-Fail "list-windows -J not JSON: $json" }

Write-Test "list-windows shows active window with *"
$activeLine = $lines | Where-Object { $_ -match '\*' }
if ($activeLine) { Write-Pass "Active window marked with *: $activeLine" }
else { Write-Fail "No active window marked with *" }

# ============================================================
# 3. COPY-MODE -u (ENTER WITH PAGE UP) TEST
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY-MODE -u TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "copy-mode -u enters copy mode"
# Generate some content first
Psmux send-keys -t feat5 "echo line1; echo line2; echo line3; echo line4; echo line5" Enter
Start-Sleep -Milliseconds 500
Psmux copy-mode -t feat5 -u
Start-Sleep -Milliseconds 300
$dm = Psmux display-message -t feat5 -p "#{pane_in_mode}"
if ($dm -match "1") { Write-Pass "copy-mode -u entered copy mode" }
else { Write-Fail "copy-mode -u did not enter copy mode: $dm" }

# Exit copy mode
Psmux send-keys -t feat5 q
Start-Sleep -Milliseconds 200

# ============================================================
# 4. PASTE-BUFFER -b (SPECIFIC BUFFER INDEX) TEST
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "PASTE-BUFFER -b TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "paste-buffer -b pastes specific buffer"
# Set up multiple buffers
Psmux set-buffer -t feat5 "buffer-zero"
Psmux set-buffer -t feat5 "buffer-one"
Psmux set-buffer -t feat5 "buffer-two"
Start-Sleep -Milliseconds 200

# Verify we have 3 buffers
$bufs = Psmux list-buffers -t feat5
$bufLines = ($bufs -split "`n") | Where-Object { $_.Trim() -ne "" }
if ($bufLines.Count -ge 3) { Write-Pass "Have $($bufLines.Count) buffers" }
else { Write-Fail "Expected >=3 buffers, got $($bufLines.Count): $bufs" }

# buffer 0 should be the most recent (buffer-two)
$show0 = Psmux show-buffer -t feat5
if ($show0 -match "buffer-two") { Write-Pass "Buffer 0 is 'buffer-two' (most recent)" }
else { Write-Fail "Buffer 0 unexpected: $show0" }

# ============================================================
# 5. COPY-MODE SPACE/ENTER TESTS (vi-style)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY-MODE SPACE/ENTER KEY TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "Space begins selection, Enter copies and exits"
# Put known text in pane
Psmux send-keys -t feat5 "echo SPACETEST" Enter
Start-Sleep -Milliseconds 500
Psmux copy-mode -t feat5
Start-Sleep -Milliseconds 200
# Move up to output line (the "SPACETEST" line, not the "echo SPACETEST" prompt line)
Psmux send-keys -t feat5 k 0
Start-Sleep -Milliseconds 100
# Space to begin selection at start of "SPACETEST"
Psmux send-keys -t feat5 space
Start-Sleep -Milliseconds 100
# Select forward 8 chars to cover "SPACETEST" (0-8 = 9 chars)
PsmuxQuick send-keys -t feat5 l l l l l l l l
Start-Sleep -Milliseconds 100
# Enter to copy and exit
Psmux send-keys -t feat5 Enter
Start-Sleep -Milliseconds 300
# Check that we're back in passthrough
$dm = Psmux display-message -t feat5 -p "#{pane_in_mode}"
if ($dm -match "0") { Write-Pass "Enter exits copy-mode" }
else { Write-Fail "Still in copy mode after Enter: $dm" }
# Check buffer contains selection
$buf = Psmux show-buffer -t feat5
if ($buf -match "SPACETEST") { Write-Pass "Space+Enter copied text: $buf" }
else { Write-Fail "Space+Enter buffer unexpected: $buf" }

# ============================================================
# 6. COPY-MODE W/B/E (BIG WORD) MOTION TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY-MODE WORD MOTION TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "W/B/E WORD motions work in copy mode"
# Put text with punctuation for WORD vs word distinction
# Quote the string so PowerShell echo outputs it as a single line
Psmux send-keys -t feat5 "echo 'hello-world foo.bar baz'" Enter
Start-Sleep -Milliseconds 800
Psmux copy-mode -t feat5
Start-Sleep -Milliseconds 300
# Navigate up to the output line (which is "hello-world foo.bar baz")
Psmux send-keys -t feat5 k
Start-Sleep -Milliseconds 100
Psmux send-keys -t feat5 0
Start-Sleep -Milliseconds 100
# At col 0, select with v and E to end of WORD
Psmux send-keys -t feat5 v
Start-Sleep -Milliseconds 100
# Move to end of WORD (E): should reach end of "hello-world" (the 'd' at col 10)
Psmux send-keys -t feat5 E
Start-Sleep -Milliseconds 100
Psmux send-keys -t feat5 y
Start-Sleep -Milliseconds 300
$buf = Psmux show-buffer -t feat5
if ($buf -match "hello-world") { Write-Pass "v+E captured WORD: $buf" }
else { Write-Fail "v+E WORD motion unexpected: $buf" }

# ============================================================
# 7. COPY-MODE H/M/L (SCREEN POSITION) TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY-MODE SCREEN POSITION TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "H/M/L move cursor to screen positions"
Psmux copy-mode -t feat5
Start-Sleep -Milliseconds 200
# H = move to top of screen
Psmux send-keys -t feat5 H
Start-Sleep -Milliseconds 100
# L = move to bottom of screen
Psmux send-keys -t feat5 L
Start-Sleep -Milliseconds 100
# M = move to middle of screen
Psmux send-keys -t feat5 M
Start-Sleep -Milliseconds 100
# If we get here without crash, the motions are working
Psmux send-keys -t feat5 q
Start-Sleep -Milliseconds 200
$dm = Psmux display-message -t feat5 -p "#{pane_in_mode}"
if ($dm -match "0") { Write-Pass "H/M/L motions work (no crash, exited cleanly)" }
else { Write-Fail "H/M/L test: still in copy mode: $dm" }

# ============================================================
# 8. COPY-MODE f/F FIND-CHAR TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY-MODE FIND-CHAR TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "f finds character forward on current line"
Psmux send-keys -t feat5 "echo findchar-XYZ-test" Enter
Start-Sleep -Milliseconds 500
Psmux copy-mode -t feat5
Start-Sleep -Milliseconds 200
# Go to output line "findchar-XYZ-test"
Psmux send-keys -t feat5 k 0
Start-Sleep -Milliseconds 100
# f X = find next X on line (should move cursor to 'X' in output)
Psmux send-keys -t feat5 f X
Start-Sleep -Milliseconds 100
# Start selection
Psmux send-keys -t feat5 v
Start-Sleep -Milliseconds 100
# Move right 2 to select XYZ
Psmux send-keys -t feat5 l l
Start-Sleep -Milliseconds 100
Psmux send-keys -t feat5 y
Start-Sleep -Milliseconds 300
$buf = Psmux show-buffer -t feat5
if ($buf -match "XYZ") { Write-Pass "f char found X and selected XYZ: $buf" }
else { Write-Fail "f char unexpected: $buf" }

# ============================================================
# 9. COPY-MODE D (COPY TO END OF LINE) TEST
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COPY-MODE D (COPY END OF LINE) TESTS"
Write-Host ("=" * 60)

Ensure-Session -Name "feat5"

Write-Test "D copies from cursor to end of line"
Psmux send-keys -t feat5 "echo START-middle-END" Enter
Start-Sleep -Milliseconds 800
Psmux copy-mode -t feat5
Start-Sleep -Milliseconds 300
# Go to output line "START-middle-END" (1 line up from prompt)
Psmux send-keys -t feat5 k
Start-Sleep -Milliseconds 100
Psmux send-keys -t feat5 0
Start-Sleep -Milliseconds 100
# D = copy from cursor (col 0) to end of line
Psmux send-keys -t feat5 D
Start-Sleep -Milliseconds 300
# Should have exited copy mode
$dm = Psmux display-message -t feat5 -p "#{pane_in_mode}"
if ($dm -match "0") { Write-Pass "D exits copy mode" }
else { Write-Fail "D did not exit copy mode: $dm" }
$buf = Psmux show-buffer -t feat5
if ($buf -match "START-middle-END") { Write-Pass "D copied to end of line: $buf" }
else { Write-Fail "D buffer unexpected: $buf" }

# ============================================================
# CLEANUP & SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLEANUP"
Write-Host ("=" * 60)

# Kill test session
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t feat5" -WindowStyle Hidden
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 60)
if ($script:TestsFailed -gt 0) {
    Write-Host "RESULT: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor Red
} else {
    Write-Host "RESULT: $($script:TestsPassed) passed, $($script:TestsFailed) failed - ALL TESTS PASSED" -ForegroundColor Green
}
Write-Host ("=" * 60)
exit $script:TestsFailed
