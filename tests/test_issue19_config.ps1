# =============================================================================
# ISSUE #19 DEEP TEST: Config file bind-key
# Tests that bind-key from config files actually works end-to-end
# Uses real USERPROFILE paths (backs up and restores any existing config)
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
# TEST 1: Config file with bind-key commands is loaded
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 1: Config file bind-key loading" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

# Create a test config at the real USERPROFILE location
$testConfig = "$env:USERPROFILE\.psmux.conf"
@"
# Test config - Issue #19
# Key bindings
bind-key r split-window -h
bind-key - split-window -v
bind | split-window -h
bind h select-pane -L
bind j select-pane -D
bind k select-pane -U
bind l select-pane -R

# Status bar (to verify config loads at all)
set -g status-right 'TEST19OK'
set -g window-status-format ' #I:#W '
"@ | Set-Content -Path $testConfig -Encoding UTF8 -NoNewline
Write-Info "Created test config: $testConfig"
Write-Info "Config contents:"
Get-Content $testConfig | ForEach-Object { Write-Info "  $_" }

$S1 = "cfg_bind_test_$(Get-Random)"
Write-Test "Start session with config file present"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S1 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$ls = (& $PSMUX ls 2>&1) -join "`n"
if ($ls -match [regex]::Escape($S1)) {
    Write-Pass "Session '$S1' started with config"
} else {
    Write-Fail "Could not start session! ls output: $ls"
    # Try to diagnose
    Write-Info "Checking if port file was created..."
    $portFiles = Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue
    Write-Info "Port files: $($portFiles | ForEach-Object { $_.Name })"
}

Write-Test "List keys — check for custom bindings from config"
$keys = & $PSMUX list-keys -t $S1 2>&1
$keysText = ($keys -join "`n")
Write-Info "list-keys full output:"
$keys | ForEach-Object { Write-Info "  $_" }

$bindChecks = @(
    @{ Key = "r"; Pattern = "split-window.*-h|split.*horizontal"; Desc = "bind r split-window -h" },
    @{ Key = "-"; Pattern = "split-window.*-v|split.*vertical"; Desc = 'bind - split-window -v' },
    @{ Key = "h"; Pattern = "select-pane.*-L"; Desc = "bind h select-pane -L" },
    @{ Key = "j"; Pattern = "select-pane.*-D"; Desc = "bind j select-pane -D" },
    @{ Key = "k"; Pattern = "select-pane.*-U"; Desc = "bind k select-pane -U" },
    @{ Key = "l"; Pattern = "select-pane.*-R"; Desc = "bind l select-pane -R" }
)

foreach ($bc in $bindChecks) {
    if ($keysText -match $bc.Pattern) {
        Write-Pass "Config binding found: $($bc.Desc)"
    } else {
        Write-Fail "Config binding MISSING: $($bc.Desc)"
    }
}

# ═══════════════════════════════════════════════════════════════════════
# TEST 2: Runtime bind-key command (prefix+: then bind-key)
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 2: Runtime bind-key via CLI" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

Write-Test "Add a new binding at runtime"
$result = & $PSMUX bind-key -t $S1 "v" "split-window -v" 2>&1
Write-Info "bind-key v result: $result"

Start-Sleep -Milliseconds 500
$keys2 = & $PSMUX list-keys -t $S1 2>&1
$keys2Text = ($keys2 -join "`n")

if ($keys2Text -match "v.*split-window") {
    Write-Pass "Runtime bind-key 'v' -> split-window registered"
} else {
    Write-Fail "Runtime bind-key 'v' not found"
    Write-Info "list-keys after runtime bind:"
    $keys2 | ForEach-Object { Write-Info "  $_" }
}

# ═══════════════════════════════════════════════════════════════════════
# TEST 3: Verify bound action actually WORKS (split-window -h)
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 3: Bound action execution" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

Write-Test "Verify split-window -h works via direct CLI"
$panesBefore = & $PSMUX list-panes -t $S1 2>&1
$panesBeforeCount = ($panesBefore | Measure-Object -Line).Lines
Write-Info "Panes before split: $panesBeforeCount"

& $PSMUX split-window -h -t $S1 2>&1 | Out-Null
Start-Sleep -Seconds 1

$panesAfter = & $PSMUX list-panes -t $S1 2>&1
$panesAfterCount = ($panesAfter | Measure-Object -Line).Lines
Write-Info "Panes after split: $panesAfterCount"

if ($panesAfterCount -gt $panesBeforeCount) {
    Write-Pass "split-window -h created a new pane ($panesBeforeCount -> $panesAfterCount)"
} else {
    Write-Fail "split-window did NOT create a new pane"
}

# ═══════════════════════════════════════════════════════════════════════
# TEST 4: source-file command (load config at runtime)
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 4: source-file command" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

# Create a separate config to source
$sourceConfig = "$env:TEMP\psmux_source_test.conf"
@"
# Source-file test
bind-key g new-window
set -g status-left '[SRC]'
"@ | Set-Content -Path $sourceConfig -Encoding UTF8

Write-Test "source-file loads additional bindings"
$srcResult = & $PSMUX source-file -t $S1 $sourceConfig 2>&1
Write-Info "source-file result: $srcResult"
Start-Sleep -Milliseconds 500

$keys3 = & $PSMUX list-keys -t $S1 2>&1
$keys3Text = ($keys3 -join "`n")
if ($keys3Text -match "g.*new-window") {
    Write-Pass "source-file loaded binding: g -> new-window"
} else {
    Write-Fail "source-file binding not found"
    Write-Info "list-keys after source-file:"
    $keys3 | ForEach-Object { Write-Info "  $_" }
}

Remove-Item $sourceConfig -Force -ErrorAction SilentlyContinue

# ═══════════════════════════════════════════════════════════════════════
# TEST 5: Config with tmux-style binding variants
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 5: tmux-style binding variants" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

# Kill the session and restart with a new config
& $PSMUX kill-session -t $S1 2>&1 | Out-Null
Start-Sleep -Seconds 2

# Write a more complex tmux-style config
@"
# tmux-compatible config for testing
set-option -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key -r H resize-pane -L 5
bind-key -r J resize-pane -D 5
bind-key -r K resize-pane -U 5
bind-key -r L resize-pane -R 5
bind-key -n C-h select-pane -L
bind-key -n C-j select-pane -D
bind-key -n C-k select-pane -U
bind-key -n C-l select-pane -R
bind 0 select-window -t :=0
bind 1 select-window -t :=1
bind 2 select-window -t :=2
set -g mouse on
set -g status-right '#H | %H:%M'
"@ | Set-Content -Path $testConfig -Encoding UTF8 -NoNewline
Write-Info "Updated config with tmux-style bindings"

$S5 = "tmux_style_test_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S5 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$ls5 = (& $PSMUX ls 2>&1) -join "`n"
if ($ls5 -match [regex]::Escape($S5)) {
    Write-Pass "Session started with tmux-style config"

    Write-Test "Check prefix changed to C-a"
    $opts5 = & $PSMUX show-options -t $S5 2>&1
    $opts5Text = ($opts5 -join "`n")
    if ($opts5Text -match "prefix.*C-a|prefix.*\u0001") {
        Write-Pass "Custom prefix C-a is set"
    } else {
        Write-Fail "prefix C-a not reflected in show-options"
        Write-Info "show-options: $opts5Text"
    }

    Write-Test "Check root-table bindings (-n flag)"
    $keys5 = & $PSMUX list-keys -t $S5 2>&1
    $keys5Text = ($keys5 -join "`n")
    if ($keys5Text -match "root.*C-h|C-h.*select-pane.*-L") {
        Write-Pass "Root-table binding (bind -n C-h) found"
    } else {
        Write-Fail "Root-table binding (bind -n C-h) missing"
    }

    Write-Test "Check repeatable bindings (-r flag)"
    if ($keys5Text -match "H.*resize-pane") {
        Write-Pass "Repeatable binding (bind -r H resize-pane) found"
    } else {
        Write-Fail "Repeatable binding (bind -r H resize-pane) missing"
    }

    Write-Test "Check window select bindings (bind 0 select-window -t :=0)"
    if ($keys5Text -match "0.*select-window|select-window.*0") {
        Write-Pass "Window select binding found"
    } else {
        Write-Fail "Window select binding missing"
    }

    $keys5 | ForEach-Object { Write-Info "  $_" }
} else {
    Write-Fail "Could not start session with tmux-style config"
}

& $PSMUX kill-session -t $S5 2>&1 | Out-Null
Start-Sleep -Seconds 1

# ═══════════════════════════════════════════════════════════════════════
# TEST 6: Config with chained commands (bind x split-window \; select-pane -D)
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Magenta
Write-Host "  TEST 6: Chained command bindings" -ForegroundColor Magenta
Write-Host ("=" * 70) -ForegroundColor Magenta

@"
# Chained command test
bind-key m split-window -h \; select-pane -R
bind-key n next-window
set -g status-right 'CHAIN_TEST'
"@ | Set-Content -Path $testConfig -Encoding UTF8 -NoNewline

$S6 = "chain_test_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S6 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$ls6 = (& $PSMUX ls 2>&1) -join "`n"
if ($ls6 -match [regex]::Escape($S6)) {
    $keys6 = & $PSMUX list-keys -t $S6 2>&1
    $keys6Text = ($keys6 -join "`n")
    Write-Info "Chained binding keys:"
    $keys6 | ForEach-Object { Write-Info "  $_" }

    if ($keys6Text -match "m.*split") {
        Write-Pass "Chained command binding found for 'm'"
    } else {
        Write-Fail "Chained command binding missing for 'm'"
    }
} else {
    Write-Fail "Could not start session for chained bindings test"
}

& $PSMUX kill-session -t $S6 2>&1 | Out-Null
Start-Sleep -Seconds 1

# ═══════════════════════════════════════════════════════════════════════
# CLEANUP: Restore original config files
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Yellow
Write-Host "  CLEANUP" -ForegroundColor Yellow
Write-Host ("=" * 70) -ForegroundColor Yellow

# Remove our test config
Remove-Item $testConfig -Force -ErrorAction SilentlyContinue

# Restore backed up configs
foreach ($entry in $backedUp.GetEnumerator()) {
    Copy-Item $entry.Value $entry.Key -Force
    Remove-Item $entry.Value -Force
    Write-Info "Restored: $($entry.Key)"
}
Write-Info "Original config files restored"

# Kill any remaining test sessions
& $PSMUX kill-server 2>&1 | Out-Null
taskkill /f /im psmux.exe 2>$null | Out-Null

# ═══════════════════════════════════════════════════════════════════════
# SUMMARY
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  ISSUE #19 DEEP TEST RESULTS" -ForegroundColor White
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host ("=" * 70) -ForegroundColor White

if ($script:TestsFailed -gt 0) {
    Write-Host ""
    Write-Host "  *** ISSUE #19 BUGS DETECTED ***" -ForegroundColor Red
    exit 1
} else {
    Write-Host ""
    Write-Host "  All Issue #19 tests passed!" -ForegroundColor Green
    exit 0
}
