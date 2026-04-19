# Issue #100 - C-Space prefix parsed as C-s
# Tests that multi-character key names (Space, Enter, Tab, etc.) are correctly
# parsed when combined with modifiers (C-, M-, S-) in both config file and
# runtime set-option contexts.
#
# https://github.com/psmux/psmux/issues/100
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue100_key_names.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found. Build first: cargo build --release"; exit 1 }
Write-Info "Using: $PSMUX"

$confPath = "$env:USERPROFILE\.psmux.conf"
$confBackup = $null

# ============================================================
# SETUP
# ============================================================
Write-Info "Backing up config and cleaning up..."
if (Test-Path $confPath) {
    $confBackup = Get-Content $confPath -Raw
}
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host ("=" * 60)
Write-Host "  ISSUE #100: C-Space AND MULTI-CHAR KEY NAMES"
Write-Host ("=" * 60)

# ============================================================
# Test 1: Config file - set -g prefix C-Space
# ============================================================
Write-Host ""
Write-Test "1. Config: set -g prefix C-Space"

Set-Content -Path $confPath -Value 'set -g prefix C-Space'
$session = "issue100_test1"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
Write-Info "  show-options prefix: '$result'"
if ($result -eq "C-Space") {
    Write-Pass "C-Space parsed correctly from config file"
} elseif ($result -match "C-s$") {
    Write-Fail "C-Space parsed as C-s (bug #100 still present)"
} else {
    Write-Fail "C-Space parsed as '$result'"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 2: Config file - set -g prefix C-space (lowercase)
# ============================================================
Write-Host ""
Write-Test "2. Config: set -g prefix C-space (lowercase)"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value 'set -g prefix C-space'
$session = "issue100_test2"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
Write-Info "  show-options prefix: '$result'"
if ($result -eq "C-Space") {
    Write-Pass "C-space (lowercase) parsed correctly from config file"
} elseif ($result -match "C-$") {
    Write-Fail "C-space parsed as 'C-' (bug #100 still present)"
} else {
    Write-Fail "C-space parsed as '$result'"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 3: Runtime - set -g prefix C-Space
# ============================================================
Write-Host ""
Write-Test "3. Runtime: set -g prefix C-Space"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item $confPath -Force -ErrorAction SilentlyContinue

$session = "issue100_test3"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3
& $PSMUX set-option -g prefix C-Space -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 1

$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
Write-Info "  show-options prefix: '$result'"
if ($result -eq "C-Space") {
    Write-Pass "C-Space set correctly at runtime"
} else {
    Write-Fail "C-Space runtime set produced '$result'"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 4: Config file - bind C-Space send-prefix
# ============================================================
Write-Host ""
Write-Test "4. Config: bind C-Space send-prefix"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value @"
set -g prefix C-Space
bind C-Space send-prefix
"@
$session = "issue100_test4"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Check prefix
$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
Write-Info "  prefix: '$result'"
if ($result -eq "C-Space") {
    Write-Pass "prefix C-Space with bind works"
} else {
    Write-Fail "prefix is '$result' instead of C-Space"
}

# Check that the binding exists
$bindings = (& $PSMUX list-keys -t $session 2>&1) | Out-String
if ($bindings -match "C-Space.*send-prefix") {
    Write-Pass "bind C-Space send-prefix registered"
} else {
    Write-Fail "C-Space binding not found in list-keys"
    Write-Info "  Bindings containing 'Space' or 'send-prefix':"
    $bindings -split "`n" | Where-Object { $_ -match "Space|send-prefix" } | ForEach-Object { Write-Info "    $_" }
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 5: Config file - C-Enter, C-Tab, C-Escape
# ============================================================
Write-Host ""
Write-Test "5. Config: C-Enter, C-Tab, C-Escape bindings"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value @"
bind -T prefix C-Enter new-window
bind -T prefix C-Tab next-window
bind -T prefix C-Escape copy-mode
"@
$session = "issue100_test5"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$bindings = (& $PSMUX list-keys -t $session 2>&1) | Out-String
$passCount = 0
$failCount = 0

if ($bindings -match "C-Enter.*new-window") {
    Write-Pass "C-Enter binding registered"
    $passCount++
} else {
    Write-Fail "C-Enter binding not found"
    $failCount++
}

if ($bindings -match "C-Tab.*next-window") {
    Write-Pass "C-Tab binding registered"
    $passCount++
} else {
    Write-Fail "C-Tab binding not found"
    $failCount++
}

if ($bindings -match "C-Escape.*copy-mode") {
    Write-Pass "C-Escape binding registered"
    $passCount++
} else {
    Write-Fail "C-Escape binding not found"
    $failCount++
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 6: Config file - M-Space, M-Enter (Alt+named keys)
# ============================================================
Write-Host ""
Write-Test "6. Config: M-Space, M-Enter bindings"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value @"
bind -T prefix M-Space next-layout
bind -T prefix M-Enter new-window
"@
$session = "issue100_test6"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$bindings = (& $PSMUX list-keys -t $session 2>&1) | Out-String

if ($bindings -match "M-Space.*next-layout") {
    Write-Pass "M-Space binding registered"
} else {
    Write-Fail "M-Space binding not found"
}

if ($bindings -match "M-Enter.*new-window") {
    Write-Pass "M-Enter binding registered"
} else {
    Write-Fail "M-Enter binding not found"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 7: source-file also parses C-Space correctly
# ============================================================
Write-Host ""
Write-Test "7. source-file: C-Space prefix"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item $confPath -Force -ErrorAction SilentlyContinue

$session = "issue100_test7"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

# Write a temp config and source it
$tmpConf = "$env:TEMP\psmux_test100.conf"
Set-Content -Path $tmpConf -Value 'set -g prefix C-Space'
& $PSMUX source-file $tmpConf -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 1

$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
Write-Info "  show-options prefix after source-file: '$result'"
if ($result -eq "C-Space") {
    Write-Pass "source-file correctly parses C-Space"
} else {
    Write-Fail "source-file produced '$result' instead of C-Space"
}

Remove-Item $tmpConf -Force -ErrorAction SilentlyContinue
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 8: Regression - C-a, C-b still work
# ============================================================
Write-Host ""
Write-Test "8. Regression: C-a and C-b prefixes still work"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value 'set -g prefix C-a'
$session = "issue100_test8"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
if ($result -eq "C-a") {
    Write-Pass "C-a prefix still works"
} else {
    Write-Fail "C-a prefix broken (got '$result')"
}

# Now test C-b via runtime
& $PSMUX set-option -g prefix C-b -t $session 2>&1 | Out-Null
Start-Sleep -Seconds 1
$result = (& $PSMUX show-options -v prefix -t $session 2>&1) | Out-String
$result = $result.Trim()
if ($result -eq "C-b") {
    Write-Pass "C-b prefix still works"
} else {
    Write-Fail "C-b prefix broken (got '$result')"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 9: C-Up, C-Down, C-Left, C-Right (modifier + arrow)
# ============================================================
Write-Host ""
Write-Test "9. Config: C-Up, C-Down, C-Left, C-Right bindings"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value @"
bind -T prefix C-Up resize-pane -U
bind -T prefix C-Down resize-pane -D
bind -T prefix C-Left resize-pane -L
bind -T prefix C-Right resize-pane -R
"@
$session = "issue100_test9"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$bindings = (& $PSMUX list-keys -t $session 2>&1) | Out-String

foreach ($dir in @("Up", "Down", "Left", "Right")) {
    $flag = $dir[0].ToString().ToUpper()
    if ($bindings -match "C-$dir.*resize-pane") {
        Write-Pass "C-$dir binding registered"
    } else {
        Write-Fail "C-$dir binding not found"
    }
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# Test 10: C-F1 through C-F12 (modifier + function key)
# ============================================================
Write-Host ""
Write-Test "10. Config: C-F1 binding"

& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue

Set-Content -Path $confPath -Value 'bind -T prefix C-F1 new-window'
$session = "issue100_test10"
& $PSMUX new-session -d -s $session 2>&1 | Out-Null
Start-Sleep -Seconds 3

$bindings = (& $PSMUX list-keys -t $session 2>&1) | Out-String
if ($bindings -match "C-F1.*new-window") {
    Write-Pass "C-F1 binding registered"
} else {
    Write-Fail "C-F1 binding not found"
}
& $PSMUX kill-session -t $session 2>$null | Out-Null
Start-Sleep -Seconds 1

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Info "Cleaning up..."
& $PSMUX kill-server 2>$null | Out-Null
Start-Sleep -Seconds 1

if ($confBackup) {
    Set-Content -Path $confPath -Value $confBackup
    Write-Info "Restored original config"
} else {
    Remove-Item $confPath -Force -ErrorAction SilentlyContinue
    Write-Info "Removed test config"
}

# ============================================================
# RESULTS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed"
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) {
    Write-Host "Some tests FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All tests PASSED" -ForegroundColor Green
    exit 0
}
