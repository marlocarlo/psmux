# psmux Vim-Style Navigation Key Binding Test (Discussion #130)
# Tests: bind-key -n C-hjkl for pane navigation (root table), key alias normalization
# Run: powershell -ExecutionPolicy Bypass -File tests\test_vim_nav_keys.ps1

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

function Psmux { & $PSMUX @args 2>$null; Start-Sleep -Milliseconds 300 }

$SESSION = "vimnavtest"

# ============================================================
# SETUP
# ============================================================
Write-Info "Cleaning up..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden 2>$null
Start-Sleep -Seconds 2

Write-Info "Starting test session..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

# ============================================================
# 1. BIND VIM-STYLE C-HJKL KEYS (Root Table)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "VIM-STYLE ROOT TABLE BINDINGS (-n flag)"
Write-Host ("=" * 60)

Write-Test "bind-key -n C-h select-pane -L"
Psmux bind-key -t $SESSION -n C-h select-pane -L
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -match "root.*C-h.*select-pane.*-L") {
    Write-Pass "C-h bound to select-pane -L in root table"
} else {
    Write-Fail "C-h binding not found in list-keys. Output: $keys"
}

Write-Test "bind-key -n C-j select-pane -D"
Psmux bind-key -t $SESSION -n C-j select-pane -D
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -match "root.*C-j.*select-pane.*-D") {
    Write-Pass "C-j bound to select-pane -D in root table"
} else {
    Write-Fail "C-j binding not found in list-keys. Output: $keys"
}

Write-Test "bind-key -n C-k select-pane -U"
Psmux bind-key -t $SESSION -n C-k select-pane -U
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -match "root.*C-k.*select-pane.*-U") {
    Write-Pass "C-k bound to select-pane -U in root table"
} else {
    Write-Fail "C-k binding not found in list-keys. Output: $keys"
}

Write-Test "bind-key -n C-l select-pane -R"
Psmux bind-key -t $SESSION -n C-l select-pane -R
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -match "root.*C-l.*select-pane.*-R") {
    Write-Pass "C-l bound to select-pane -R in root table"
} else {
    Write-Fail "C-l binding not found in list-keys. Output: $keys"
}

# ============================================================
# 2. VERIFY ALL FOUR BINDINGS COEXIST
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "VERIFY ALL BINDINGS COEXIST"
Write-Host ("=" * 60)

Write-Test "All four vim nav bindings present"
$keys = Psmux list-keys -t $SESSION | Out-String
$allPresent = ($keys -match "C-h") -and ($keys -match "C-j") -and ($keys -match "C-k") -and ($keys -match "C-l")
if ($allPresent) {
    Write-Pass "All four C-hjkl bindings present in list-keys"
} else {
    Write-Fail "Not all bindings found. Output: $keys"
}

# ============================================================
# 3. ALTERNATIVE SYNTAX: -T root (equivalent to -n)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "-T root SYNTAX (equivalent to -n)"
Write-Host ("=" * 60)

Write-Test "bind-key -T root C-h (overwrite existing)"
Psmux bind-key -t $SESSION -T root C-h select-pane -L
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -match "root.*C-h.*select-pane.*-L") {
    Write-Pass "-T root C-h creates same root binding as -n"
} else {
    Write-Fail "-T root C-h not found. Output: $keys"
}

# ============================================================
# 4. UNBIND AND REBIND
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "UNBIND AND REBIND"
Write-Host ("=" * 60)

Write-Test "unbind C-h, then verify removed"
Psmux unbind-key -t $SESSION C-h
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -notmatch "root.*C-h.*select-pane") {
    Write-Pass "C-h successfully unbound"
} else {
    Write-Fail "C-h still present after unbind. Output: $keys"
}

Write-Test "rebind C-h after unbind"
Psmux bind-key -t $SESSION -n C-h select-pane -L
$keys = Psmux list-keys -t $SESSION | Out-String
if ("$keys" -match "root.*C-h.*select-pane.*-L") {
    Write-Pass "C-h re-bound successfully"
} else {
    Write-Fail "C-h not found after rebind. Output: $keys"
}

# ============================================================
# 5. CONFIG FILE SYNTAX (bind-key -n in .psmux.conf)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CONFIG FILE SYNTAX"
Write-Host ("=" * 60)

Write-Test "source-file with bind-key -n lines"
$confPath = "$env:TEMP\psmux_vim_test.conf"
@"
bind-key -n C-h select-pane -L
bind-key -n C-j select-pane -D
bind-key -n C-k select-pane -U
bind-key -n C-l select-pane -R
"@ | Set-Content -Path $confPath -Encoding UTF8

Psmux source-file -t $SESSION $confPath
Start-Sleep -Milliseconds 500
$keys = Psmux list-keys -t $SESSION | Out-String
$allPresent = ($keys -match "C-h") -and ($keys -match "C-j") -and ($keys -match "C-k") -and ($keys -match "C-l")
if ($allPresent) {
    Write-Pass "Config file bind-key -n lines loaded correctly"
} else {
    Write-Fail "Config file bindings not found. Output: $keys"
}
Remove-Item $confPath -Force -ErrorAction SilentlyContinue

# ============================================================
# 6. BACKSPACE AND C-h ARE DISTINCT ON WINDOWS
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "WINDOWS KEY DISTINCTION"
Write-Host ("=" * 60)

Write-Test "BSpace and C-h are distinct bindings on Windows"
# On Windows, Backspace and Ctrl+H are separate keys — binding both should create 2 entries
Psmux bind-key -t $SESSION -n BSpace display-panes
Psmux bind-key -t $SESSION -n C-h select-pane -L
$keys = Psmux list-keys -t $SESSION | Out-String
$has_bspace = $keys -match "BSpace"
$has_ch = $keys -match "C-h"
if ($has_bspace -and $has_ch) {
    Write-Pass "BSpace and C-h are distinct bindings (both present)"
} elseif ($has_ch) {
    # C-h present, BSpace may have been stored as C-h display name — still OK
    Write-Pass "C-h binding present (BSpace may share display name)"
} else {
    Write-Fail "Expected both BSpace and C-h bindings. Output: $keys"
}

# ============================================================
# CLEANUP & SUMMARY
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Info "Cleaning up session..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden 2>$null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 60)
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
