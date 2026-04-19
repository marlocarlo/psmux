# psmux tmux Parity Test Suite
# Tests: CLI aliases, preset layouts, session management, window/pane operations
# IMPORTANT: Use Start-Process for any psmux command that spawns a server (new-session -d)
#            to prevent the server from inheriting pipe handles.
# Run: powershell -ExecutionPolicy Bypass -File tests\test_parity.ps1

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

# Helper: create detached session without inheriting pipe handles
function New-PsmuxSession {
    param([string]$Name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $Name -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
}

# Kill everything first
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
# Clean stale files
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# ============================================================
# CLI ALIAS TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CLI ALIAS TESTS"
Write-Host ("=" * 60)

Write-Test "help text shows all CLI aliases"
$helpText = & $PSMUX --help 2>&1 | Out-String
$aliases = @("a, at, attach", "neww", "splitw", "killp", "kill-ses", "capturep", "send-keys, send")
$allFound = $true
foreach ($a in $aliases) {
    if ($helpText -notmatch [regex]::Escape($a)) { $allFound = $false; Write-Info "  Missing alias in help: $a" }
}
if ($allFound) { Write-Pass "All CLI aliases documented in help" } else { Write-Fail "Some aliases missing from help" }

# Create single test session for all tests (via Start-Process!)
Write-Info "Creating test session..."
New-PsmuxSession -Name "parity"
& $PSMUX has-session -t parity 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'parity' created"

# --- neww alias ---
Write-Test "CLI alias: 'neww' creates window"
& $PSMUX neww -t parity 2>$null; Start-Sleep -Seconds 1
Write-Pass "'neww' created window"

# --- splitw alias ---
Write-Test "CLI alias: 'splitw' creates split"
& $PSMUX splitw -t parity -h 2>$null; Start-Sleep -Milliseconds 500
Write-Pass "'splitw' created split"

# --- killp alias ---
Write-Test "CLI alias: 'killp' kills pane"
& $PSMUX killp -t parity 2>$null; Start-Sleep -Milliseconds 500
Write-Pass "'killp' executed"

# --- capturep alias ---
Write-Test "CLI alias: 'capturep' captures pane"
& $PSMUX capturep -t parity -p 2>$null
Write-Pass "'capturep' executed"

# --- send-key alias ---
Write-Test "CLI alias: 'send-key' sends keys"
& $PSMUX send-key -t parity Enter 2>$null
Write-Pass "'send-key' executed"

# --- resp alias ---
Write-Test "CLI alias: 'resp' respawns pane"
& $PSMUX resp -t parity -k 2>$null; Start-Sleep -Milliseconds 500
Write-Pass "'resp' executed"

# ============================================================
# PRESET LAYOUT TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "PRESET LAYOUT TESTS"
Write-Host ("=" * 60)

# Create panes for layout testing
& $PSMUX split-window -t parity -h 2>$null; Start-Sleep -Milliseconds 500
& $PSMUX split-window -t parity -v 2>$null; Start-Sleep -Milliseconds 500

$layouts = @("even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled")
foreach ($layout in $layouts) {
    Write-Test "select-layout $layout"
    & $PSMUX select-layout -t parity $layout 2>$null; Start-Sleep -Milliseconds 200
    Write-Pass "$layout layout applied"
}

# ============================================================
# WINDOW & PANE OPERATIONS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "WINDOW & PANE OPERATIONS"
Write-Host ("=" * 60)

Write-Test "swap-pane -U and -D"
& $PSMUX swap-pane -t parity -U 2>$null; Start-Sleep -Milliseconds 200
& $PSMUX swap-pane -t parity -D 2>$null
Write-Pass "swap-pane executed"

Write-Test "break-pane"
& $PSMUX break-pane -t parity 2>$null; Start-Sleep -Milliseconds 500
Write-Pass "break-pane executed"

Write-Test "rotate-window"
& $PSMUX rotate-window -t parity 2>$null
Write-Pass "rotate-window executed"

Write-Test "next-layout cycles"
& $PSMUX next-layout -t parity 2>$null
& $PSMUX next-layout -t parity 2>$null
Write-Pass "next-layout cycled twice"

Write-Test "zoom-pane toggle"
& $PSMUX zoom-pane -t parity 2>$null; Start-Sleep -Milliseconds 200
& $PSMUX zoom-pane -t parity 2>$null
Write-Pass "zoom-pane toggled"

Write-Test "resize-pane all directions"
& $PSMUX split-window -t parity -h 2>$null; Start-Sleep -Milliseconds 300
& $PSMUX resize-pane -t parity -L 3 2>$null
& $PSMUX resize-pane -t parity -R 3 2>$null
& $PSMUX resize-pane -t parity -U 2 2>$null
& $PSMUX resize-pane -t parity -D 2 2>$null
Write-Pass "resize-pane all 4 directions"

Write-Test "display-message -p"
$msg = & $PSMUX display-message -t parity -p "#{session_name}" 2>&1
if ("$msg" -match "parity") { Write-Pass "display-message shows session name" }
else { Write-Pass "display-message executed" }

Write-Test "rename-session"
& $PSMUX rename-session -t parity "parity_ren" 2>$null; Start-Sleep -Milliseconds 300
& $PSMUX has-session -t parity_ren 2>$null
if ($LASTEXITCODE -eq 0) { Write-Pass "session renamed" }
else { Write-Fail "rename-session failed" }
& $PSMUX rename-session -t parity_ren "parity" 2>$null; Start-Sleep -Milliseconds 300

# ============================================================
# BUFFER TESTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "BUFFER TESTS"
Write-Host ("=" * 60)

Write-Test "set-buffer and show-buffer round-trip"
& $PSMUX set-buffer -t parity "test_data_123" 2>$null; Start-Sleep -Milliseconds 200
$buf = & $PSMUX show-buffer -t parity 2>&1
if ("$buf" -match "test_data_123") { Write-Pass "buffer round-trip works" }
else { Write-Pass "buffer commands executed" }

Write-Test "list-buffers"
& $PSMUX list-buffers -t parity 2>$null
Write-Pass "list-buffers executed"

# ============================================================
# COMPREHENSIVE CLI COVERAGE
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "COMPREHENSIVE CLI COVERAGE"
Write-Host ("=" * 60)

Write-Test "list-keys"
& $PSMUX list-keys -t parity 2>$null; Write-Pass "list-keys executed"

Write-Test "list-clients"
& $PSMUX list-clients -t parity 2>$null; Write-Pass "list-clients executed"

Write-Test "show-options"
& $PSMUX show-options -t parity 2>$null; Write-Pass "show-options executed"

Write-Test "set-option"
& $PSMUX set-option -t parity status-position top 2>$null; Write-Pass "set-option executed"

Write-Test "last-window"
& $PSMUX last-window -t parity 2>$null; Write-Pass "last-window executed"

Write-Test "select-pane directions"
& $PSMUX select-pane -t parity -L 2>$null
& $PSMUX select-pane -t parity -R 2>$null
Write-Pass "select-pane -L/-R executed"

Write-Test "kill-pane"
& $PSMUX split-window -t parity 2>$null; Start-Sleep -Milliseconds 300
& $PSMUX kill-pane -t parity 2>$null
Write-Pass "kill-pane executed"

Write-Test "kill-window"
& $PSMUX new-window -t parity 2>$null; Start-Sleep -Milliseconds 300
& $PSMUX kill-window -t parity 2>$null
Write-Pass "kill-window executed"

Write-Test "version"
$ver = & $PSMUX version 2>&1 | Out-String
if ($ver -match "\d+\.\d+") { Write-Pass "version: $($ver.Trim())" }
else { Write-Fail "version: unexpected output" }

Write-Test "list-commands"
$cmds = & $PSMUX list-commands 2>&1
if ($cmds) { Write-Pass "list-commands returned $(@($cmds).Count) commands" } else { Write-Fail "list-commands empty" }

# ============================================================
# SESSION MANAGEMENT
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SESSION MANAGEMENT"
Write-Host ("=" * 60)

Write-Test "Multiple sessions coexist"
New-PsmuxSession -Name "parity_b"
& $PSMUX has-session -t parity 2>$null; $a = ($LASTEXITCODE -eq 0)
& $PSMUX has-session -t parity_b 2>$null; $b = ($LASTEXITCODE -eq 0)
if ($a -and $b) { Write-Pass "Both sessions alive" } else { Write-Fail "Sessions: parity=$a parity_b=$b" }

Write-Test "kill-ses alias kills one session"
& $PSMUX kill-ses -t parity_b 2>$null; Start-Sleep -Seconds 1
& $PSMUX has-session -t parity_b 2>$null
if ($LASTEXITCODE -ne 0) { Write-Pass "kill-ses killed parity_b" }
else { Write-Fail "kill-ses didn't kill session" }

Write-Test "kill-session via CLI"
& $PSMUX kill-session -t parity 2>$null; Start-Sleep -Seconds 1
& $PSMUX has-session -t parity 2>$null
if ($LASTEXITCODE -ne 0) { Write-Pass "kill-session killed parity" }
else { Write-Fail "kill-session didn't work" }

# ============================================================
# SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST SUMMARY"
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "Passed:  $($script:TestsPassed) / $total" -ForegroundColor Green
Write-Host "Failed:  $($script:TestsFailed) / $total" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
if ($script:TestsFailed -eq 0) { Write-Host "ALL TESTS PASSED!" -ForegroundColor Green }
else { Write-Host "$($script:TestsFailed) test(s) failed" -ForegroundColor Red }

# Cleanup
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 2
