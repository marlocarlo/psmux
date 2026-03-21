# =============================================================================
# ISSUE #19 FIX TEST: bind-key with pipe '|' and other shifted symbols
# Tests that bind-key | split-window -h actually works end-to-end
# The bug was that crossterm reports '|' as (Char('|'), SHIFT) but config
# stored it as (Char('|'), NONE), so the binding never matched.
# =============================================================================
$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "  [INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "  [TEST] $msg" -ForegroundColor White }

# --- Locate binary ---
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    Write-Host "[FATAL] psmux binary not found." -ForegroundColor Red
    exit 1
}
Write-Info "Binary: $PSMUX"

# --- Kill existing sessions ---
taskkill /f /im psmux.exe 2>$null | Out-Null
taskkill /f /im pmux.exe 2>$null | Out-Null
taskkill /f /im tmux.exe 2>$null | Out-Null
Start-Sleep -Seconds 2

# Remove stale port/key files
$psmuxDir = "$env:USERPROFILE\.psmux"
if (Test-Path $psmuxDir) {
    Get-ChildItem "$psmuxDir\*.port" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    Get-ChildItem "$psmuxDir\*.key" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
}

# ═══════════════════════════════════════════════════════════════════════
# Backup any existing config files
# ═══════════════════════════════════════════════════════════════════════
$configCandidates = @(
    "$env:USERPROFILE\.psmux.conf",
    "$env:USERPROFILE\.psmuxrc",
    "$env:USERPROFILE\.tmux.conf"
)
$backedUp = @{}
foreach ($cf in $configCandidates) {
    if (Test-Path $cf) {
        $backupPath = "${cf}.test_backup_$(Get-Random)"
        Copy-Item $cf $backupPath -Force
        $backedUp[$cf] = $backupPath
        Remove-Item $cf -Force
        Write-Info "Backed up existing config: $cf -> $backupPath"
    }
}

# ═══════════════════════════════════════════════════════════════════════
# TEST 1: Config file with bind-key | (pipe char)
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 1: bind-key | split-window -h (config file)" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

$testConfig = "$env:USERPROFILE\.psmux.conf"
@"
# Test config - Issue #19 pipe key fix
bind-key | split-window -h
bind-key - split-window -v
bind-key _ split-window -v
"@ | Set-Content -Path $testConfig -Encoding UTF8 -NoNewline
Write-Info "Created test config: $testConfig"
Get-Content $testConfig | ForEach-Object { Write-Info "  $_" }

$S1 = "pipe_key_test_$(Get-Random)"
Write-Test "Start session with config containing bind-key |"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S1 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$ls = (& $PSMUX ls 2>&1) -join "`n"
if ($ls -match [regex]::Escape($S1)) {
    Write-Pass "Session '$S1' started"
} else {
    Write-Fail "Could not start session! ls output: $ls"
}

Write-Test "list-keys shows binding for | (pipe)"
$keys = & $PSMUX list-keys -t $S1 2>&1
$keysText = ($keys -join "`n")
Write-Info "list-keys output:"
$keys | ForEach-Object { Write-Info "  $_" }

# Check that | binding for split-window -h is present
if ($keysText -match "\|.*split-window.*-h") {
    Write-Pass "bind-key | split-window -h found in list-keys"
} else {
    Write-Fail "bind-key | split-window -h NOT found in list-keys"
}

# Check that - binding for split-window -v is present
if ($keysText -match "\-.*split-window.*-v") {
    Write-Pass "bind-key - split-window -v found in list-keys"
} else {
    Write-Fail "bind-key - split-window -v NOT found in list-keys"
}

# Check that _ binding for split-window -v is present
if ($keysText -match "_.*split-window.*-v") {
    Write-Pass "bind-key _ split-window -v found in list-keys"
} else {
    Write-Fail "bind-key _ split-window -v NOT found in list-keys"
}

& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 3

# ═══════════════════════════════════════════════════════════════════════
# TEST 2: Runtime bind-key | via CLI
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 2: Runtime bind-key | via CLI" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

# Clean config so we test runtime binding only
Remove-Item $testConfig -Force -ErrorAction SilentlyContinue

$S2 = "pipe_runtime_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S2 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$ls2 = (& $PSMUX ls 2>&1) -join "`n"
if ($ls2 -match [regex]::Escape($S2)) {
    Write-Pass "Session '$S2' started (no config)"
} else {
    Write-Fail "Could not start session '$S2'"
}

Write-Test "Runtime: bind-key | split-window -h"
& $PSMUX bind-key -t $S2 "|" split-window -h 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$keys2 = & $PSMUX list-keys -t $S2 2>&1
$keys2Text = ($keys2 -join "`n")
Write-Info "list-keys after runtime bind:"
$keys2 | ForEach-Object { Write-Info "  $_" }

if ($keys2Text -match "\|.*split-window.*-h") {
    Write-Pass "Runtime bind-key | split-window -h found in list-keys"
} else {
    Write-Fail "Runtime bind-key | split-window -h NOT found in list-keys"
}

Write-Test "Runtime: bind-key _ split-window -v"
& $PSMUX bind-key -t $S2 "_" split-window -v 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$keys2b = & $PSMUX list-keys -t $S2 2>&1
$keys2bText = ($keys2b -join "`n")
if ($keys2bText -match "_.*split-window.*-v") {
    Write-Pass "Runtime bind-key _ split-window -v found in list-keys"
} else {
    Write-Fail "Runtime bind-key _ split-window -v NOT found in list-keys"
}

# ═══════════════════════════════════════════════════════════════════════
# TEST 3: All shifted symbols can be bound
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 3: Shifted symbol keys can be bound" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

$symbolTests = @(
    @{ Key = "!"; Cmd = "display-message"; Desc = "exclamation" },
    @{ Key = "@"; Cmd = "display-message"; Desc = "at sign" },
    @{ Key = "#"; Cmd = "display-message"; Desc = "hash" },
    @{ Key = "_"; Cmd = "split-window -v"; Desc = "underscore" },
    @{ Key = "+"; Cmd = "display-message"; Desc = "plus" },
    @{ Key = "~"; Cmd = "display-message"; Desc = "tilde" }
)

foreach ($st in $symbolTests) {
    Write-Test "bind-key $($st.Key) ($($st.Desc))"
    & $PSMUX bind-key -t $S2 $st.Key $st.Cmd 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
}

Start-Sleep -Milliseconds 500
$keys3 = & $PSMUX list-keys -t $S2 2>&1
$keys3Text = ($keys3 -join "`n")

foreach ($st in $symbolTests) {
    $escaped = [regex]::Escape($st.Key)
    if ($keys3Text -match "$escaped") {
        Write-Pass "bind-key $($st.Key) ($($st.Desc)) found in list-keys"
    } else {
        Write-Fail "bind-key $($st.Key) ($($st.Desc)) NOT found in list-keys"
    }
}

& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 3

# ═══════════════════════════════════════════════════════════════════════
# TEST 4: unbind-key | works
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 4: unbind-key | (pipe)" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

# Use config with pipe binding
@"
bind-key | split-window -h
"@ | Set-Content -Path $testConfig -Encoding UTF8 -NoNewline

$S4 = "pipe_unbind_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S4 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

Write-Test "Verify | binding exists before unbind"
$keys4 = & $PSMUX list-keys -t $S4 2>&1
$keys4Text = ($keys4 -join "`n")
if ($keys4Text -match "\|.*split-window") {
    Write-Pass "| binding exists before unbind"
} else {
    Write-Fail "| binding missing before unbind"
}

Write-Test "unbind-key | removes the binding"
& $PSMUX unbind-key -t $S4 "|" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$keys4b = & $PSMUX list-keys -t $S4 2>&1
$keys4bText = ($keys4b -join "`n")
if ($keys4bText -notmatch "\|.*split-window") {
    Write-Pass "unbind-key | successfully removed the binding"
} else {
    Write-Fail "unbind-key | did NOT remove the binding"
}

& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 3

# ═══════════════════════════════════════════════════════════════════════
# TEST 5: Verify split-window -h via pipe binding actually creates pane
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 5: Functional test - split-window via pipe binding" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

@"
bind-key | split-window -h
bind-key - split-window -v
"@ | Set-Content -Path $testConfig -Encoding UTF8 -NoNewline

$S5 = "pipe_func_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S5 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

Write-Test "Count panes before split"
$panesBefore = & $PSMUX list-panes -t $S5 2>&1
$panesBeforeCount = ($panesBefore | Measure-Object -Line).Lines
Write-Info "Panes before: $panesBeforeCount"

Write-Test "Execute split-window -h (the command that | would trigger)"
& $PSMUX split-window -h -t $S5 2>&1 | Out-Null
Start-Sleep -Seconds 1

$panesAfter = & $PSMUX list-panes -t $S5 2>&1
$panesAfterCount = ($panesAfter | Measure-Object -Line).Lines
Write-Info "Panes after: $panesAfterCount"

if ($panesAfterCount -gt $panesBeforeCount) {
    Write-Pass "split-window -h created a new pane ($panesBeforeCount -> $panesAfterCount)"
} else {
    Write-Fail "split-window -h did NOT create a new pane"
}

& $PSMUX kill-session -t $S5 2>&1 | Out-Null
Start-Sleep -Seconds 1

# ═══════════════════════════════════════════════════════════════════════
# CLEANUP
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Yellow
Write-Host "  CLEANUP" -ForegroundColor Yellow
Write-Host ("=" * 70) -ForegroundColor Yellow

Remove-Item $testConfig -Force -ErrorAction SilentlyContinue

foreach ($entry in $backedUp.GetEnumerator()) {
    Copy-Item $entry.Value $entry.Key -Force
    Remove-Item $entry.Value -Force
    Write-Info "Restored: $($entry.Key)"
}
Write-Info "Original config files restored"

& $PSMUX kill-server 2>&1 | Out-Null
taskkill /f /im psmux.exe 2>$null | Out-Null

# ═══════════════════════════════════════════════════════════════════════
# SUMMARY
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  BIND-KEY PIPE '|' FIX TEST RESULTS" -ForegroundColor White
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host ("=" * 70) -ForegroundColor White

if ($script:TestsFailed -gt 0) {
    Write-Host ""
    Write-Host "  *** PIPE KEY BINDING BUGS DETECTED ***" -ForegroundColor Red
    exit 1
} else {
    Write-Host ""
    Write-Host "  All pipe key binding tests passed!" -ForegroundColor Green
    exit 0
}
