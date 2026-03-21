# =============================================================================
# PRECISE BUG DETECTION TESTS - Issues found in GitHub #19 and #25
# Tests specific code-level bugs discovered by code analysis
# =============================================================================
$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "  [INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "  [TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { Write-Host "[FATAL] Binary not found" -ForegroundColor Red; exit 1 }

# Kill existing
taskkill /f /im psmux.exe 2>$null | Out-Null
Start-Sleep -Seconds 2
$psmuxDir = "$env:USERPROFILE\.psmux"
if (Test-Path $psmuxDir) {
    Get-ChildItem "$psmuxDir\*.port" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    Get-ChildItem "$psmuxDir\*.key" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
}

# Backup existing config
$existingConfig = "$env:USERPROFILE\.psmux.conf"
$existingBackup = ""
if (Test-Path $existingConfig) { $existingBackup = "${existingConfig}.bak_$(Get-Random)"; Copy-Item $existingConfig $existingBackup -Force; Remove-Item $existingConfig -Force }
$existingRc = "$env:USERPROFILE\.psmuxrc"
$existingRcBackup = ""
if (Test-Path $existingRc) { $existingRcBackup = "${existingRc}.bak_$(Get-Random)"; Copy-Item $existingRc $existingRcBackup -Force; Remove-Item $existingRc -Force }
$existingTmux = "$env:USERPROFILE\.tmux.conf"
$existingTmuxBackup = ""
if (Test-Path $existingTmux) { $existingTmuxBackup = "${existingTmux}.bak_$(Get-Random)"; Copy-Item $existingTmux $existingTmuxBackup -Force; Remove-Item $existingTmux -Force }

# ═══════════════════════════════════════════════════════════════════════
# BUG 1: Config parser treats '-' (dash) key as a flag
# In parse_bind_key(), `if p.starts_with('-')` eats the '-' key as a flag
# Result: `bind - split-window -v` is silently ignored in config files
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Red
Write-Host "  BUG 1: Config parser treats dash key as flag" -ForegroundColor Red
Write-Host ("=" * 70) -ForegroundColor Red

@"
# Bug 1 test config
bind-key - split-window -v
bind-key r split-window -h
set -g status-right 'BUG1TEST'
"@ | Set-Content -Path $existingConfig -Encoding UTF8 -NoNewline

$S1 = "bug1_test_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S1 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$keys1 = & $PSMUX list-keys -t $S1 2>&1
$keys1Text = ($keys1 -join "`n")

Write-Test "BUG1a: bind-key r split-window -h should be registered (control)"
if ($keys1Text -match "bind-key -T prefix r split-window -h") {
    Write-Pass "BUG1a: r -> split-window -h found (working correctly)"
} else {
    Write-Fail "BUG1a: r -> split-window -h NOT found"
}

Write-Test "BUG1b: bind-key - split-window -v from config file"
# Look specifically for a binding with '-' as the key for split-window -v
$dashBindFound = $false
foreach ($line in $keys1) {
    $l = "$line".Trim()
    # We need to find a line like: bind-key -T prefix - split-window -v
    # But NOT the default: bind-key -T prefix " split-window -v
    if ($l -match 'bind-key -T prefix - split-window -v') {
        $dashBindFound = $true
    }
}
if ($dashBindFound) {
    Write-Pass "BUG1b: dash key binding found"
} else {
    Write-Fail "BUG1b: dash key binding MISSING - config parser ate '-' as flag!"
    Write-Info "This confirms the bug: parse_bind_key() in config.rs treats '-' as a flag"
}

& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 3

# ═══════════════════════════════════════════════════════════════════════
# BUG 1b: Same dash key via RUNTIME bind-key (should work since TCP 
# handler uses exact matching, not starts_with)
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Yellow
Write-Host "  BUG 1b: Runtime bind-key with dash key (TCP handler)" -ForegroundColor Yellow  
Write-Host ("=" * 70) -ForegroundColor Yellow

# Use empty config
@"
# Empty config for runtime test
set -g status-right 'BUG1RUNTIME'
"@ | Set-Content -Path $existingConfig -Encoding UTF8 -NoNewline

$S1b = "bug1b_test_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S1b -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

Write-Test "BUG1b-runtime: Add bind-key - at runtime via CLI"
& $PSMUX bind-key -t $S1b "-" "split-window -v" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$keys1b = & $PSMUX list-keys -t $S1b 2>&1
$keys1bText = ($keys1b -join "`n")

$dashRuntimeFound = $false
foreach ($line in $keys1b) {
    $l = "$line".Trim()
    if ($l -match 'bind-key -T prefix - split-window -v') {
        $dashRuntimeFound = $true
    }
}
if ($dashRuntimeFound) {
    Write-Pass "BUG1b-runtime: dash binding registered via TCP handler (works!)"
} else {
    Write-Fail "BUG1b-runtime: dash binding missing even via TCP handler"
}

& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 3

# ═══════════════════════════════════════════════════════════════════════
# BUG 2: list-keys shows duplicates for overridden default keys
# When user overrides 'l' (default: last-window), both the default
# and custom binding appear in list-keys output
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Red
Write-Host "  BUG 2: Duplicate entries in list-keys for overridden defaults" -ForegroundColor Red
Write-Host ("=" * 70) -ForegroundColor Red

@"
# Bug 2 test - override 'l' with select-pane -R
bind l select-pane -R
set -g status-right 'BUG2TEST'
"@ | Set-Content -Path $existingConfig -Encoding UTF8 -NoNewline

$S2 = "bug2_test_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S2 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$keys2 = & $PSMUX list-keys -t $S2 2>&1
$keys2Text = ($keys2 -join "`n")

Write-Test "BUG2: Count how many times 'l' appears in list-keys"
$lBindings = $keys2 | Where-Object { "$_".Trim() -match 'bind-key -T prefix l ' }
$lCount = ($lBindings | Measure-Object).Count
Write-Info "Lines with 'bind-key -T prefix l':"
$lBindings | ForEach-Object { Write-Info "  $_" }

if ($lCount -eq 1) {
    Write-Pass "BUG2: Only one 'l' binding shown (no duplicate)"
} elseif ($lCount -gt 1) {
    Write-Fail "BUG2: DUPLICATE - 'l' appears $lCount times in list-keys!"
    Write-Info "User binding should replace the default, not co-exist"
} else {
    Write-Fail "BUG2: No 'l' binding found at all"
}

& $PSMUX kill-server 2>&1 | Out-Null
Start-Sleep -Seconds 3

# ═══════════════════════════════════════════════════════════════════════
# BUG 3: Client hardcoded bindings shadow user bindings
# When user binds 'n' to something other than next-window, the
# hardcoded client handler still runs next-window instead
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Red
Write-Host "  BUG 3: Client hardcoded bindings shadow user bindings" -ForegroundColor Red
Write-Host ("=" * 70) -ForegroundColor Red

Write-Info "This bug can only be fully tested interactively (client key dispatch)"
Write-Info "But we can verify the REGISTRATION side works correctly"

@"
# Bug 3 test - rebind hardcoded keys
bind n split-window -h
bind l select-pane -R
bind c kill-pane
set -g status-right 'BUG3TEST'
"@ | Set-Content -Path $existingConfig -Encoding UTF8 -NoNewline

$S3 = "bug3_test_$(Get-Random)"
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $S3 -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 4

$keys3 = & $PSMUX list-keys -t $S3 2>&1
$keys3Text = ($keys3 -join "`n")

# Check that the user bindings are registered (even if client ignores them)
Write-Test "BUG3a: User binding 'n -> split-window -h' registered on server"
$nBindings = $keys3 | Where-Object { "$_".Trim() -match 'bind-key -T prefix n ' }
Write-Info "All 'n' bindings:"
$nBindings | ForEach-Object { Write-Info "  $_" }

$hasCustomN = $nBindings | Where-Object { "$_" -match 'split-window' }
if ($hasCustomN) {
    Write-Pass "BUG3a: Custom 'n -> split-window -h' registered on server"
} else {
    Write-Fail "BUG3a: Custom 'n' binding not registered"
}

$hasDefaultN = $nBindings | Where-Object { "$_" -match 'next-window' }
if ($hasDefaultN) {
    Write-Fail "BUG3a-dup: DEFAULT 'n -> next-window' is ALSO listed (will shadow custom)"
} else {
    Write-Pass "BUG3a-dup: Default 'n -> next-window' NOT listed (correctly replaced)"
}

Write-Test "BUG3b: User binding 'l -> select-pane -R' registered"
$lBindings3 = $keys3 | Where-Object { "$_".Trim() -match 'bind-key -T prefix l ' }
Write-Info "All 'l' bindings:"
$lBindings3 | ForEach-Object { Write-Info "  $_" }

$hasCustomL = $lBindings3 | Where-Object { "$_" -match 'select-pane -R' }
if ($hasCustomL) {
    Write-Pass "BUG3b: Custom 'l -> select-pane -R' registered"
} else {
    Write-Fail "BUG3b: Custom 'l' binding not registered"
}

$hasDefaultL = $lBindings3 | Where-Object { "$_" -match 'last-window' }
if ($hasDefaultL) {
    Write-Fail "BUG3b-dup: DEFAULT 'l -> last-window' is ALSO listed (client will use THIS instead!)"
    Write-Info "Client hardcoded handler at client.rs:552 intercepts 'l' before checking synced_bindings"
} else {
    Write-Pass "BUG3b-dup: Default correctly replaced"
}

# Test that the HARDCODED keys that the user overrode still execute the DEFAULT action
# We can test this via CLI: the bug only manifests when the USER presses the key interactively
Write-Test "BUG3c: Via CLI, commands should use the correct user-defined action"
& $PSMUX new-window -t $S3 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
& $PSMUX select-window -t "${S3}:1" 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

$panesBefore = & $PSMUX list-panes -t $S3 2>&1
$panesBeforeCount = ($panesBefore | Measure-Object -Line).Lines

# Execute the user-bound action directly (not via key, but via command)
# This tests that the command itself works, even if the key binding dispatch is broken
& $PSMUX split-window -h -t $S3 2>&1 | Out-Null
Start-Sleep -Seconds 1

$panesAfter = & $PSMUX list-panes -t $S3 2>&1
$panesAfterCount = ($panesAfter | Measure-Object -Line).Lines
if ($panesAfterCount -gt $panesBeforeCount) {
    Write-Pass "BUG3c: The command itself works (split-window -h)"
    Write-Info "But pressing prefix+'n' interactively would run next-window instead!"
} else {
    Write-Fail "BUG3c: split-window -h failed"
}

& $PSMUX kill-session -t $S3 2>&1 | Out-Null
Start-Sleep -Seconds 1

# ═══════════════════════════════════════════════════════════════════════
# CLEANUP
# ═══════════════════════════════════════════════════════════════════════
Remove-Item $existingConfig -Force -ErrorAction SilentlyContinue
if ($existingBackup -and (Test-Path $existingBackup)) { Copy-Item $existingBackup $existingConfig -Force; Remove-Item $existingBackup -Force }
if ($existingRcBackup -and (Test-Path $existingRcBackup)) { Copy-Item $existingRcBackup $existingRc -Force; Remove-Item $existingRcBackup -Force }
if ($existingTmuxBackup -and (Test-Path $existingTmuxBackup)) { Copy-Item $existingTmuxBackup $existingTmux -Force; Remove-Item $existingTmuxBackup -Force }

& $PSMUX kill-server 2>&1 | Out-Null
taskkill /f /im psmux.exe 2>$null | Out-Null

# ═══════════════════════════════════════════════════════════════════════
# SUMMARY
# ═══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  BUG DETECTION RESULTS" -ForegroundColor White
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host ("=" * 70) -ForegroundColor White

if ($script:TestsFailed -gt 0) {
    Write-Host ""
    Write-Host "  *** BUGS CONFIRMED ***" -ForegroundColor Red
    Write-Host "  BUG 1: Config parser parse_bind_key() treats '-' as flag" -ForegroundColor Red
    Write-Host "  BUG 2: list-keys shows both default and custom for same key" -ForegroundColor Red
    Write-Host "  BUG 3: Client hardcoded bindings shadow user config bindings" -ForegroundColor Red
    exit 1
} else {
    Write-Host ""
    Write-Host "  All bugs have been fixed!" -ForegroundColor Green
    exit 0
}
