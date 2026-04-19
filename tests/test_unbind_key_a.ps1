# psmux unbind-key -a End-to-End Test Suite
# Tests that unbind-key -a truly suppresses default keybindings
# both server-side (list-keys) and client-side (actual key dispatch)

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

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

function Cleanup {
    Stop-Process -Name psmux -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 1000
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
}

function Get-WindowCount {
    $out = & $PSMUX list-windows 2>&1 | Out-String
    return ($out.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }).Count
}

function Get-PaneCount {
    $out = & $PSMUX list-panes 2>&1 | Out-String
    return ($out.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }).Count
}

function Send-KeysTcp {
    param([string]$Keys)
    $portFile = "$env:USERPROFILE\.psmux\0.port"
    $keyFile  = "$env:USERPROFILE\.psmux\0.key"
    if (!(Test-Path $portFile) -or !(Test-Path $keyFile)) { Write-Fail "No port/key files"; return }
    $port = (Get-Content $portFile).Trim()
    $key  = (Get-Content $keyFile).Trim()
    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
    $stream = $tcp.GetStream()
    $writer = [System.IO.StreamWriter]::new($stream)
    $reader = [System.IO.StreamReader]::new($stream)
    $writer.WriteLine("AUTH $key"); $writer.Flush()
    $auth = $reader.ReadLine()
    $writer.WriteLine($Keys); $writer.Flush()
    Start-Sleep -Milliseconds 300
    $tcp.Close()
}

function Get-DumpStateField {
    param([string]$FieldName)
    $portFile = "$env:USERPROFILE\.psmux\0.port"
    $keyFile  = "$env:USERPROFILE\.psmux\0.key"
    if (!(Test-Path $portFile) -or !(Test-Path $keyFile)) { return $null }
    $port = (Get-Content $portFile).Trim()
    $key  = (Get-Content $keyFile).Trim()
    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
    $stream = $tcp.GetStream()
    $writer = [System.IO.StreamWriter]::new($stream)
    $reader = [System.IO.StreamReader]::new($stream)
    $writer.WriteLine("AUTH $key"); $writer.Flush()
    $auth = $reader.ReadLine()
    $writer.WriteLine("dump-state"); $writer.Flush()
    Start-Sleep -Milliseconds 500
    $buf = ""
    while ($stream.DataAvailable) { $buf += [char]$stream.ReadByte() }
    $tcp.Close()
    if ($buf -match "`"$FieldName`":(true|false|`"[^`"]*`"|\d+|\[[^\]]*\])") {
        return $Matches[1]
    }
    return $null
}

# ============================================================
Cleanup
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST SUITE: unbind-key -a"
Write-Host ("=" * 60)

# Ensure .psmux.conf does not shadow .tmux.conf for this test
$psmuxConf = "$env:USERPROFILE\.psmux.conf"
$psmuxConfBackup = "$env:USERPROFILE\.psmux.conf.unbind_bak"
if (Test-Path $psmuxConf) {
    Move-Item $psmuxConf $psmuxConfBackup -Force
}

# ============================================================
# SCENARIO 1: WITH unbind-key -a (defaults should be suppressed)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SCENARIO 1: With unbind-key -a"
Write-Host ("=" * 60)

@"
unbind-key -a
unbind-key -a -T prefix
unbind-key -a -T root
unbind-key -a -T copy-mode
unbind-key -a -T copy-mode-vi
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Write-Info "Starting session with unbind-key -a config..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

# Test 1: list-keys should only show user bindings
Write-Test "list-keys shows only user bindings after unbind-key -a"
$keys = & $PSMUX list-keys 2>&1 | Out-String
$keyLines = $keys.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }
if ($keyLines.Count -eq 2 -and $keys -match "C-a send-prefix" -and $keys -match "C-r source-file") {
    Write-Pass "list-keys: only 2 user bindings shown ($($keyLines.Count) lines)"
} else {
    Write-Fail "list-keys: expected 2 bindings, got $($keyLines.Count). Output:`n$keys"
}

# Test 2: defaults_suppressed flag is true in DumpState
Write-Test "DumpState defaults_suppressed is true"
$val = Get-DumpStateField "defaults_suppressed"
if ($val -eq "true") {
    Write-Pass "defaults_suppressed = true in DumpState"
} else {
    Write-Fail "defaults_suppressed = '$val' (expected 'true')"
}

# Test 3: bindings array in DumpState only has user bindings
Write-Test "DumpState bindings array has only user entries"
$bindings = Get-DumpStateField "bindings"
# Two entries: C-a and C-r
$entryCount = ([regex]::Matches($bindings, '"t":')).Count
if ($bindings -match "C-a" -and $bindings -match "C-r" -and $entryCount -eq 2) {
    Write-Pass "bindings array has only 2 user entries"
} else {
    Write-Fail "bindings array unexpected ($entryCount entries): $bindings"
}

# Test 4: Server-side new-window via direct command still works
Write-Test "Direct 'new-window' command still works"
$before = Get-WindowCount
& $PSMUX new-window 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$after = Get-WindowCount
if ($after -eq ($before + 1)) {
    Write-Pass "new-window via CLI creates window ($before -> $after)"
} else {
    Write-Fail "new-window via CLI: $before -> $after"
}

# Test 5: Server-side split-window via direct command still works
Write-Test "Direct 'split-window' command still works"
$before = Get-PaneCount
& $PSMUX split-window -h 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
$after = Get-PaneCount
if ($after -eq ($before + 1)) {
    Write-Pass "split-window via CLI creates pane ($before -> $after)"
} else {
    Write-Fail "split-window via CLI: $before -> $after"
}

# Test 6: unbind-key -a -T root was independent (can still add root bindings)
Write-Test "Can add root binding after unbind-key -a -T root"
& $PSMUX bind-key -n F12 new-window 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
$keys = & $PSMUX list-keys 2>&1 | Out-String
if ($keys -match "root.*F12.*new-window") {
    Write-Pass "Root binding F12 added after table was cleared"
} else {
    Write-Fail "Root binding F12 not found in list-keys"
}

# ============================================================
# SCENARIO 2: WITHOUT unbind-key -a (defaults should work)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SCENARIO 2: Without unbind-key -a (defaults preserved)"
Write-Host ("=" * 60)

Cleanup

@"
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Write-Info "Starting session without unbind-key -a..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

# Test 7: list-keys shows defaults + user bindings
Write-Test "list-keys shows defaults + user bindings"
$keys = & $PSMUX list-keys 2>&1 | Out-String
$keyLines = $keys.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }
$hasDefaults = $keys -match "new-window" -and $keys -match "split-window" -and $keys -match "detach-client"
$hasUser = $keys -match "C-a send-prefix" -and $keys -match "C-r source-file"
if ($hasDefaults -and $hasUser -and $keyLines.Count -gt 40) {
    Write-Pass "list-keys: $($keyLines.Count) lines (defaults + user)"
} else {
    Write-Fail "list-keys: expected 40+ lines with defaults, got $($keyLines.Count)"
}

# Test 8: defaults_suppressed flag is false
Write-Test "DumpState defaults_suppressed is false"
$val = Get-DumpStateField "defaults_suppressed"
if ($val -eq "false") {
    Write-Pass "defaults_suppressed = false in DumpState"
} else {
    Write-Fail "defaults_suppressed = '$val' (expected 'false')"
}

# ============================================================
# SCENARIO 3: PER-TABLE unbind (only root cleared, prefix intact)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SCENARIO 3: Per-table unbind (only root, prefix stays)"
Write-Host ("=" * 60)

Cleanup

@"
bind-key -n F5 new-window
bind-key -n F6 split-window -h
unbind-key -a -T root
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Write-Info "Starting session with only root table cleared..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

# Test 9: Prefix defaults still present
Write-Test "Prefix defaults still shown after unbind-key -a -T root"
$keys = & $PSMUX list-keys 2>&1 | Out-String
$hasDefaults = $keys -match "new-window" -and $keys -match "detach-client"
$hasRoot = $keys -match "root"
if ($hasDefaults -and !$hasRoot) {
    Write-Pass "Prefix defaults present, root bindings gone"
} elseif ($hasDefaults -and $hasRoot) {
    Write-Fail "Root bindings still present after unbind-key -a -T root"
} else {
    Write-Fail "Prefix defaults missing. Output:`n$keys"
}

# Test 10: defaults_suppressed is false (only root was cleared, not prefix)
Write-Test "defaults_suppressed is false (only root cleared)"
$val = Get-DumpStateField "defaults_suppressed"
if ($val -eq "false") {
    Write-Pass "defaults_suppressed = false (prefix untouched)"
} else {
    Write-Fail "defaults_suppressed = '$val' (expected false since only root was cleared)"
}

# ============================================================
# SCENARIO 4: RUNTIME unbind-key -a via CLI
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SCENARIO 4: Runtime unbind-key -a via CLI"
Write-Host ("=" * 60)

Cleanup

# Start with no config (defaults active)
Remove-Item "$env:USERPROFILE\.tmux.conf" -Force -ErrorAction SilentlyContinue
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

Write-Test "Defaults present before runtime unbind"
$keys = & $PSMUX list-keys 2>&1 | Out-String
$linesBefore = ($keys.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }).Count
if ($linesBefore -gt 40) {
    Write-Pass "Before: $linesBefore default bindings present"
} else {
    Write-Fail "Before: only $linesBefore bindings (expected 40+)"
}

# Runtime unbind-key -a
& $PSMUX unbind-key -a 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "After runtime unbind-key -a, prefix defaults gone"
$keys = & $PSMUX list-keys 2>&1 | Out-String
$linesAfter = ($keys.Trim().Split("`n") | Where-Object { $_.Trim() -ne "" }).Count
if ($linesAfter -eq 0) {
    Write-Pass "After: 0 bindings (all cleared)"
} else {
    Write-Fail "After: $linesAfter bindings remaining"
}

Write-Test "defaults_suppressed is true after runtime unbind"
$val = Get-DumpStateField "defaults_suppressed"
if ($val -eq "true") {
    Write-Pass "defaults_suppressed = true after runtime unbind"
} else {
    Write-Fail "defaults_suppressed = '$val' (expected true)"
}

# Test: can still add new bindings after clearing
Write-Test "Can bind new key after runtime unbind-key -a"
& $PSMUX bind-key x split-window -h 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
$keys = & $PSMUX list-keys 2>&1 | Out-String
if ($keys -match "x.*split-window") {
    Write-Pass "New binding works after runtime unbind-key -a"
} else {
    Write-Fail "New binding not found after unbind-key -a"
}

# ============================================================
# SCENARIO 5: SOURCE-FILE RELOAD (unbind then remove unbind)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SCENARIO 5: Source-file reload toggling unbind-key -a"
Write-Host ("=" * 60)

Cleanup

# Start with full unbind config
@"
unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Write-Info "Starting session with unbind-key -a..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

Write-Test "Initial: defaults suppressed after unbind-key -a"
$c1 = (& $PSMUX list-keys 2>&1 | Measure-Object -Line).Lines
if ($c1 -eq 2) {
    Write-Pass "Initial: $c1 bindings (only user)"
} else {
    Write-Fail "Initial: expected 2 bindings, got $c1"
}

# Change config to remove unbind-key -a and reload
@"
#unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force
& $PSMUX source-file "$env:USERPROFILE\.tmux.conf"
Start-Sleep -Milliseconds 500

Write-Test "After source-file reload without unbind: defaults return"
$c2 = (& $PSMUX list-keys 2>&1 | Measure-Object -Line).Lines
if ($c2 -gt 40) {
    Write-Pass "After reload: $c2 bindings (defaults returned)"
} else {
    Write-Fail "After reload: expected 40+ bindings, got $c2"
}

Write-Test "defaults_suppressed reset to false after reload"
$val = Get-DumpStateField "defaults_suppressed"
if ($val -eq "false") {
    Write-Pass "defaults_suppressed = false after reload"
} else {
    Write-Fail "defaults_suppressed = '$val' (expected false)"
}

# Reload with unbind-key -a again to verify it re-suppresses
@"
unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force
& $PSMUX source-file "$env:USERPROFILE\.tmux.conf"
Start-Sleep -Milliseconds 500

Write-Test "Re-suppressed after reload WITH unbind-key -a"
$c3 = (& $PSMUX list-keys 2>&1 | Measure-Object -Line).Lines
if ($c3 -eq 2) {
    Write-Pass "Re-suppressed: $c3 bindings"
} else {
    Write-Fail "Re-suppressed: expected 2 bindings, got $c3"
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Cleanup
Remove-Item "$env:USERPROFILE\.tmux.conf" -Force -ErrorAction SilentlyContinue
# Restore .psmux.conf if it was backed up
if (Test-Path $psmuxConfBackup) {
    Move-Item $psmuxConfBackup $psmuxConf -Force
}

Write-Host ""
Write-Host ("=" * 60)
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
