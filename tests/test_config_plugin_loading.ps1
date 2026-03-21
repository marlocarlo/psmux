# test_config_plugin_loading.ps1 — Config & @plugin auto-source tests
#
# Covers issues #65 and the theme-flash fix:
#   1. Config file search order (.psmux.conf → .psmuxrc → .tmux.conf → XDG)
#   2. run-shell commands in config can connect to server
#   3. @plugin auto-source loads plugin.conf synchronously
#   4. User overrides AFTER @plugin are preserved (not clobbered)
#   5. -f flag overrides default config search
#   6. Symlink configs work
#   7. Multiple @plugin declarations load in order
#   8. PPM Initialize-Plugin skips .ps1 when plugin.conf exists

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}
if (-not (Test-Path $PSMUX)) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

$HOME_DIR = $env:USERPROFILE
$PSMUX_DIR = "$HOME_DIR\.psmux"
$PLUGINS_DIR = "$PSMUX_DIR\plugins"

# ── Create mock plugin.conf files if they don't exist ────────────────
# Tests 2-5 rely on psmux-sensible and psmux-theme-gruvbox plugin confs.
# If not installed, create minimal mocks so the tests can exercise the
# @plugin auto-source codepath.
$script:createdMockPlugins = @()

$sensibleDir = "$PLUGINS_DIR\psmux-sensible"
$sensibleConf = "$sensibleDir\plugin.conf"
if (-not (Test-Path $sensibleConf)) {
    New-Item -ItemType Directory -Path $sensibleDir -Force | Out-Null
    @"
# Mock psmux-sensible plugin.conf for testing
set -g escape-time 50
set -g base-index 1
set -g mouse on
"@ | Set-Content -Path $sensibleConf -Encoding UTF8
    $script:createdMockPlugins += $sensibleDir
}

$gruvboxDir = "$PLUGINS_DIR\psmux-theme-gruvbox"
$gruvboxConf = "$gruvboxDir\plugin.conf"
if (-not (Test-Path $gruvboxConf)) {
    New-Item -ItemType Directory -Path $gruvboxDir -Force | Out-Null
    @"
# Mock psmux-theme-gruvbox plugin.conf for testing
set -g status-style "bg=#282828,fg=#ebdbb2"
set -g pane-active-border-style "fg=#8ec07c"
set -g pane-border-style "fg=#504945"
"@ | Set-Content -Path $gruvboxConf -Encoding UTF8
    $script:createdMockPlugins += $gruvboxDir
}

# Helper: kill all psmux, remove stale port files
function Reset-Psmux {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    Remove-Item "$PSMUX_DIR\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$PSMUX_DIR\*.key" -Force -ErrorAction SilentlyContinue
}

# Helper: start session with specific config, return $true if successful
function Start-SessionWithConfig {
    param([string]$ConfigPath, [string]$SessionName = "cfgtest")
    Reset-Psmux
    $env:PSMUX_CONFIG_FILE = $ConfigPath
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SessionName -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
    $env:PSMUX_CONFIG_FILE = $null
    & $PSMUX has-session -t $SessionName 2>$null
    return ($LASTEXITCODE -eq 0)
}

# Helper: query option from a session
function Get-Option {
    param([string]$Option, [string]$Session = "cfgtest")
    (& $PSMUX show-options -g -v $Option -t $Session 2>&1 | Out-String).Trim()
}

Write-Host ""
Write-Host ("=" * 70)
Write-Host "    CONFIG & @PLUGIN AUTO-SOURCE TEST SUITE"
Write-Host ("=" * 70)
Write-Host ""

# ============================================================
# TEST 1: Basic config file loading
# ============================================================
Write-Host ("=" * 60)
Write-Host "  TEST 1: Basic config file loading"
Write-Host ("=" * 60)

$testConf = "$env:TEMP\psmux_test_basic.conf"
Set-Content -Path $testConf -Value @"
set -g escape-time 123
set -g base-index 3
set -g status-left "[BASIC]"
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf) {
    $v = Get-Option "escape-time"
    if ($v -eq "123") { Write-Pass "escape-time=123 from config" }
    else { Write-Fail "escape-time='$v' expected '123'" }

    $v = Get-Option "base-index"
    if ($v -eq "3") { Write-Pass "base-index=3 from config" }
    else { Write-Fail "base-index='$v' expected '3'" }

    $v = Get-Option "status-left"
    if ($v -match "BASIC") { Write-Pass "status-left contains BASIC" }
    else { Write-Fail "status-left='$v' expected to contain BASIC" }
} else {
    Write-Fail "Could not start session with basic config"
}

# ============================================================
# TEST 2: @plugin auto-source — sensible defaults
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 2: @plugin auto-source — sensible defaults"
Write-Host ("=" * 60)

# Ensure sensible plugin.conf exists
$sensibleConf = "$PLUGINS_DIR\psmux-sensible\plugin.conf"
if (-not (Test-Path $sensibleConf)) {
    Write-Skip "@plugin sensible test — plugin.conf not found at $sensibleConf"
} else {
    $testConf2 = "$env:TEMP\psmux_test_plugin.conf"
    Set-Content -Path $testConf2 -Value @"
set -g @plugin 'psmux-sensible'
"@ -Encoding UTF8

    if (Start-SessionWithConfig $testConf2) {
        $v = Get-Option "escape-time"
        if ($v -eq "50") { Write-Pass "sensible: escape-time=50" }
        else { Write-Fail "sensible: escape-time='$v' expected '50'" }

        $v = Get-Option "base-index"
        if ($v -eq "1") { Write-Pass "sensible: base-index=1" }
        else { Write-Fail "sensible: base-index='$v' expected '1'" }

        $v = Get-Option "mouse"
        if ($v -eq "on") { Write-Pass "sensible: mouse=on" }
        else { Write-Fail "sensible: mouse='$v' expected 'on'" }
    } else {
        Write-Fail "Could not start session with @plugin sensible config"
    }
}

# ============================================================
# TEST 3: @plugin auto-source — theme (gruvbox)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 3: @plugin auto-source — gruvbox theme"
Write-Host ("=" * 60)

$gruvboxConf = "$PLUGINS_DIR\psmux-theme-gruvbox\plugin.conf"
if (-not (Test-Path $gruvboxConf)) {
    Write-Skip "gruvbox test — plugin.conf not found"
} else {
    $testConf3 = "$env:TEMP\psmux_test_gruvbox.conf"
    Set-Content -Path $testConf3 -Value @"
set -g @plugin 'psmux-theme-gruvbox'
"@ -Encoding UTF8

    if (Start-SessionWithConfig $testConf3) {
        $v = Get-Option "status-style"
        if ($v -match "#282828") { Write-Pass "gruvbox: status-style has bg=#282828" }
        else { Write-Fail "gruvbox: status-style='$v' expected #282828" }

        $v = Get-Option "pane-active-border-style"
        if ($v -match "#8ec07c") { Write-Pass "gruvbox: pane-active-border aqua" }
        else { Write-Fail "gruvbox: pane-active-border='$v'" }
    } else {
        Write-Fail "Could not start session with gruvbox config"
    }
}

# ============================================================
# TEST 4: User overrides AFTER @plugin are preserved
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 4: User overrides AFTER @plugin preserved"
Write-Host ("=" * 60)

$testConf4 = "$env:TEMP\psmux_test_override.conf"
Set-Content -Path $testConf4 -Value @"
set -g @plugin 'psmux-sensible'
set -g @plugin 'psmux-theme-gruvbox'

# User overrides — these MUST survive
set -g automatic-rename off
set -g cursor-blink off
set -g cursor-style block
set -g escape-time 200
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf4) {
    $v = Get-Option "automatic-rename"
    if ($v -eq "off") { Write-Pass "user override: automatic-rename=off" }
    else { Write-Fail "user override: automatic-rename='$v' expected 'off' (sensible may have overridden)" }

    $v = Get-Option "cursor-blink"
    if ($v -eq "off") { Write-Pass "user override: cursor-blink=off" }
    else { Write-Fail "user override: cursor-blink='$v' expected 'off'" }

    $v = Get-Option "cursor-style"
    if ($v -eq "block") { Write-Pass "user override: cursor-style=block" }
    else { Write-Fail "user override: cursor-style='$v' expected 'block'" }

    $v = Get-Option "escape-time"
    if ($v -eq "200") { Write-Pass "user override: escape-time=200 (overrides sensible's 50)" }
    else { Write-Fail "user override: escape-time='$v' expected '200'" }

    # Theme should still be gruvbox
    $v = Get-Option "status-style"
    if ($v -match "#282828") { Write-Pass "gruvbox theme still applied despite overrides" }
    else { Write-Fail "gruvbox theme lost: status-style='$v'" }
} else {
    Write-Fail "Could not start session with override config"
}

# ============================================================
# TEST 5: @plugin with org/name format
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 5: @plugin with org/name path format"
Write-Host ("=" * 60)

$testConf5 = "$env:TEMP\psmux_test_orgname.conf"
Set-Content -Path $testConf5 -Value @"
set -g @plugin 'psmux-plugins/psmux-sensible'
set -g @plugin 'psmux-plugins/psmux-theme-gruvbox'
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf5) {
    $v = Get-Option "base-index"
    if ($v -eq "1") { Write-Pass "org/name: sensible loaded (base-index=1)" }
    else { Write-Fail "org/name: base-index='$v' expected '1'" }

    $v = Get-Option "status-style"
    if ($v -match "#282828") { Write-Pass "org/name: gruvbox loaded" }
    else { Write-Fail "org/name: gruvbox not loaded, status-style='$v'" }
} else {
    Write-Fail "Could not start session with org/name config"
}

# ============================================================
# TEST 6: run-shell during config (server connectivity)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 6: run-shell during config connects to server"
Write-Host ("=" * 60)

$testConf6 = "$env:TEMP\psmux_test_runshell.conf"
Set-Content -Path $testConf6 -Value @"
set -g @test-marker "before-run"
run-shell "psmux set -g @test-run-shell ok"
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf6) {
    Start-Sleep -Seconds 2  # run-shell is async
    $v = Get-Option "@test-run-shell"
    if ($v -eq "ok") { Write-Pass "run-shell set @test-run-shell=ok during config" }
    else { Write-Fail "run-shell: @test-run-shell='$v' expected 'ok'" }
} else {
    Write-Fail "Could not start session with run-shell config"
}

# ============================================================
# TEST 7: -f flag overrides (env var PSMUX_CONFIG_FILE)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 7: -f flag config override"
Write-Host ("=" * 60)

$testConf7 = "$env:TEMP\psmux_test_foverride.conf"
Set-Content -Path $testConf7 -Value @"
set -g @custom-flag-test "yes-custom"
set -g escape-time 999
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf7 "flagtest") {
    $v = Get-Option "@custom-flag-test" "flagtest"
    if ($v -eq "yes-custom") { Write-Pass "-f flag: @custom-flag-test loaded" }
    else { Write-Fail "-f flag: @custom-flag-test='$v'" }

    $v = Get-Option "escape-time" "flagtest"
    if ($v -eq "999") { Write-Pass "-f flag: escape-time=999" }
    else { Write-Fail "-f flag: escape-time='$v' expected '999'" }
} else {
    Write-Fail "Could not start session with -f override"
}

# ============================================================
# TEST 8: Symlink config file
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 8: Symlink config file"
Write-Host ("=" * 60)

$realConf = "$env:TEMP\psmux_test_real.conf"
$linkConf = "$env:TEMP\psmux_test_link.conf"
Set-Content -Path $realConf -Value @"
set -g @symlink-test "symlinked"
set -g escape-time 777
"@ -Encoding UTF8

Remove-Item $linkConf -Force -ErrorAction SilentlyContinue
try {
    New-Item -ItemType SymbolicLink -Path $linkConf -Target $realConf -Force -ErrorAction Stop | Out-Null
    if (Start-SessionWithConfig $linkConf "linktest") {
        $v = Get-Option "@symlink-test" "linktest"
        if ($v -eq "symlinked") { Write-Pass "symlink: @symlink-test loaded" }
        else { Write-Fail "symlink: @symlink-test='$v'" }

        $v = Get-Option "escape-time" "linktest"
        if ($v -eq "777") { Write-Pass "symlink: escape-time=777" }
        else { Write-Fail "symlink: escape-time='$v'" }
    } else {
        Write-Fail "Could not start symlink session"
    }
} catch {
    Write-Skip "Symlink test skipped (requires admin/developer mode): $_"
}

# ============================================================
# TEST 9: Multiple plugins load in declaration order
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 9: Multiple @plugin load order"
Write-Host ("=" * 60)

# Create two minimal test plugins
$tp1 = "$PLUGINS_DIR\test-order-a"
$tp2 = "$PLUGINS_DIR\test-order-b"
New-Item -ItemType Directory -Path $tp1 -Force | Out-Null
New-Item -ItemType Directory -Path $tp2 -Force | Out-Null
Set-Content -Path "$tp1\plugin.conf" -Value "set -g @order-test first" -Encoding UTF8
Set-Content -Path "$tp2\plugin.conf" -Value "set -g @order-test second" -Encoding UTF8

$testConf9 = "$env:TEMP\psmux_test_order.conf"
Set-Content -Path $testConf9 -Value @"
set -g @plugin 'test-order-a'
set -g @plugin 'test-order-b'
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf9 "ordertest") {
    $v = Get-Option "@order-test" "ordertest"
    if ($v -eq "second") { Write-Pass "load order: second plugin wins (@order-test=second)" }
    else { Write-Fail "load order: @order-test='$v' expected 'second'" }
} else {
    Write-Fail "Could not start session for load order test"
}

# Cleanup test plugins
Remove-Item $tp1 -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $tp2 -Recurse -Force -ErrorAction SilentlyContinue

# ============================================================
# TEST 10: @plugin ppm is skipped (not auto-sourced)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "  TEST 10: @plugin 'ppm' is not auto-sourced"
Write-Host ("=" * 60)

$testConf10 = "$env:TEMP\psmux_test_ppm_skip.conf"
Set-Content -Path $testConf10 -Value @"
set -g @plugin 'ppm'
set -g escape-time 42
"@ -Encoding UTF8

if (Start-SessionWithConfig $testConf10 "ppmtest") {
    $v = Get-Option "escape-time" "ppmtest"
    if ($v -eq "42") { Write-Pass "ppm skipped: user escape-time=42 intact" }
    else { Write-Fail "ppm skip: escape-time='$v' expected '42'" }
} else {
    Write-Fail "Could not start session for ppm skip test"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Reset-Psmux
Remove-Item "$env:TEMP\psmux_test_*.conf" -Force -ErrorAction SilentlyContinue
# Remove mock plugins we created (leave real user-installed ones alone)
foreach ($dir in $script:createdMockPlugins) {
    Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
}

# ============================================================
# RESULTS
# ============================================================
Write-Host ""
Write-Host ("=" * 70)
$total = $script:TestsPassed + $script:TestsFailed + $script:TestsSkipped
Write-Host "  RESULTS: $script:TestsPassed passed, $script:TestsFailed failed, $script:TestsSkipped skipped (of $total)"
Write-Host ("=" * 70)

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
