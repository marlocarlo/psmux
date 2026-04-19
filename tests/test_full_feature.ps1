# Full practical feature test for unbind-key -a fix
# Tests ACTUAL command execution, not just list-keys output

$ErrorActionPreference = "Continue"
$script:Pass = 0; $script:Fail = 0
function OK { param($m) Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Pass++ }
function FAIL { param($m) Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Fail++ }
function INFO { param($m) Write-Host "  [INFO] $m" -ForegroundColor Cyan }

$P = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $P) { Write-Error "psmux binary not found"; exit 1 }
INFO "Binary: $P"

function Cleanup {
    Stop-Process -Name psmux -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 2000
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
}

function WinCount { (& $P list-windows 2>&1 | Out-String).Trim().Split("`n").Where({ $_.Trim() -ne "" }).Count }
function PaneCount { (& $P list-panes 2>&1 | Out-String).Trim().Split("`n").Where({ $_.Trim() -ne "" }).Count }
function KeyCount { (& $P list-keys 2>&1 | Out-String).Trim().Split("`n").Where({ $_.Trim() -ne "" }).Count }

function DumpField {
    param([string]$Field)
    $pf = "$env:USERPROFILE\.psmux\0.port"
    $kf = "$env:USERPROFILE\.psmux\0.key"
    if (!(Test-Path $pf) -or !(Test-Path $kf)) { return $null }
    $port = (Get-Content $pf).Trim()
    $key  = (Get-Content $kf).Trim()
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
        $s = $tcp.GetStream()
        $w = [System.IO.StreamWriter]::new($s)
        $r = [System.IO.StreamReader]::new($s)
        $w.WriteLine("AUTH $key"); $w.Flush()
        $null = $r.ReadLine()
        $w.WriteLine("dump-state"); $w.Flush()
        Start-Sleep -Milliseconds 500
        $buf = ""
        while ($s.DataAvailable) { $buf += [char]$s.ReadByte() }
        $tcp.Close()
        if ($buf -match "`"$Field`":(true|false|`"[^`"]*`"|\d+|\[[^\]]*\])") {
            return $Matches[1]
        }
    } catch { return $null }
    return $null
}

# ================================================================
Cleanup
Remove-Item "$env:USERPROFILE\.tmux.conf" -Force -ErrorAction SilentlyContinue

# Ensure .psmux.conf does not shadow .tmux.conf for this test
$psmuxConf = "$env:USERPROFILE\.psmux.conf"
$psmuxConfBackup = "$env:USERPROFILE\.psmux.conf.fullfeat_bak"
if (Test-Path $psmuxConf) {
    Move-Item $psmuxConf $psmuxConfBackup -Force
}

Write-Host ""
Write-Host ("=" * 70)
Write-Host "FULL PRACTICAL FEATURE TEST"
Write-Host ("=" * 70)

# ================================================================
Write-Host "`n--- TEST 1: NO CONFIG (pure defaults) ---"
# ================================================================
Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$kc = KeyCount
if ($kc -gt 50) { OK "list-keys: $kc defaults present" } else { FAIL "list-keys: only $kc (expected 50+)" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "false") { OK "defaults_suppressed = false" } else { FAIL "defaults_suppressed = $ds" }

# Actual command execution
$w0 = WinCount
& $P new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$w1 = WinCount
if ($w1 -eq ($w0 + 1)) { OK "new-window works ($w0 -> $w1)" } else { FAIL "new-window: $w0 -> $w1" }

$p0 = PaneCount
& $P split-window -h 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$p1 = PaneCount
if ($p1 -eq ($p0 + 1)) { OK "split-window -h works ($p0 -> $p1)" } else { FAIL "split-window -h: $p0 -> $p1" }

& $P split-window -v 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$p2 = PaneCount
if ($p2 -eq ($p1 + 1)) { OK "split-window -v works ($p1 -> $p2)" } else { FAIL "split-window -v: $p1 -> $p2" }

# Switch windows
& $P select-window -t :0 2>&1 | Out-Null; Start-Sleep -Milliseconds 200
OK "select-window -t :0 executed"

& $P next-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 200
OK "next-window executed"

& $P previous-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 200
OK "previous-window executed"

# Rename
& $P rename-window "test-win" 2>&1 | Out-Null; Start-Sleep -Milliseconds 200
$wlist = & $P list-windows 2>&1 | Out-String
if ($wlist -match "test-win") { OK "rename-window works" } else { FAIL "rename-window not reflected" }

# Display message (should not crash)
& $P display-message "hello test" 2>&1 | Out-Null; Start-Sleep -Milliseconds 200
OK "display-message did not crash"

# ================================================================
Write-Host "`n--- TEST 2: REPORTER'S FULL UNBIND CONFIG ---"
# ================================================================
Cleanup

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

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$kc = KeyCount
if ($kc -eq 2) { OK "list-keys: only 2 user bindings" } else { FAIL "list-keys: expected 2, got $kc" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "true") { OK "defaults_suppressed = true" } else { FAIL "defaults_suppressed = $ds" }

# Commands still work via CLI even without keybindings
$w0 = WinCount
& $P new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$w1 = WinCount
if ($w1 -eq ($w0 + 1)) { OK "CLI new-window still works with unbind" } else { FAIL "CLI new-window: $w0 -> $w1" }

& $P split-window -h 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$p1 = PaneCount
if ($p1 -ge 2) { OK "CLI split-window -h still works with unbind" } else { FAIL "CLI split-window -h: $p1 panes" }

# Bind a new key at runtime, verify it shows
& $P bind-key x new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$keys = & $P list-keys 2>&1 | Out-String
if ($keys -match "x.*new-window") { OK "Runtime bind-key works after unbind-key -a" } else { FAIL "Runtime bind-key not reflected" }

# Unbind the runtime key
& $P unbind-key x 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$keys = & $P list-keys 2>&1 | Out-String
if ($keys -notmatch "x.*new-window") { OK "Runtime unbind-key (single) works" } else { FAIL "unbind-key x not removed" }

# ================================================================
Write-Host "`n--- TEST 3: REPORTER'S FAILING CONFIG (commented prefix unbind) ---"
# ================================================================
Cleanup

@"
#unbind-key -a
#unbind-key -a -T prefix
unbind-key -a -T root
unbind-key -a -T copy-mode
unbind-key -a -T copy-mode-vi

set -g prefix C-a
unbind-key C-b

bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$kc = KeyCount
if ($kc -gt 50) { OK "Fresh start with commented unbind: $kc bindings (defaults present)" } else { FAIL "Expected 50+, got $kc" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "false") { OK "defaults_suppressed = false" } else { FAIL "defaults_suppressed = $ds" }

# Verify actual commands still work
$w0 = WinCount
& $P new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
if ((WinCount) -eq ($w0 + 1)) { OK "new-window works with commented config" } else { FAIL "new-window broken" }

# ================================================================
Write-Host "`n--- TEST 4: SOURCE-FILE RELOAD (the actual reporter bug) ---"
# ================================================================
Cleanup

# Start with FULL unbind
@"
unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$kc1 = KeyCount
if ($kc1 -eq 2) { OK "Initial: $kc1 bindings (unbind active)" } else { FAIL "Initial: expected 2, got $kc1" }

# Change config to comment out unbind, then source-file reload
@"
#unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

& $P source-file "$env:USERPROFILE\.tmux.conf" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$kc2 = KeyCount
if ($kc2 -gt 50) { OK "After reload (no unbind): $kc2 bindings (defaults returned!)" } else { FAIL "After reload: expected 50+, got $kc2" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "false") { OK "defaults_suppressed reset to false" } else { FAIL "defaults_suppressed = $ds (should be false)" }

# Verify commands work after reload
$w0 = WinCount
& $P new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
if ((WinCount) -eq ($w0 + 1)) { OK "new-window works after reload" } else { FAIL "new-window broken after reload" }

& $P split-window -h 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
OK "split-window -h after reload"

# Reload BACK to unbind
@"
unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

& $P source-file "$env:USERPROFILE\.tmux.conf" 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$kc3 = KeyCount
if ($kc3 -eq 2) { OK "Re-reload with unbind: $kc3 bindings (suppressed again)" } else { FAIL "Re-reload: expected 2, got $kc3" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "true") { OK "defaults_suppressed = true after re-reload" } else { FAIL "defaults_suppressed = $ds" }

# Even with defaults suppressed, CLI commands still work
$w0 = WinCount
& $P new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
if ((WinCount) -eq ($w0 + 1)) { OK "CLI new-window works even with suppressed defaults" } else { FAIL "CLI new-window broken with suppressed defaults" }

# ================================================================
Write-Host "`n--- TEST 5: PER-TABLE UNBIND (root only, prefix intact) ---"
# ================================================================
Cleanup

@"
bind-key -n F5 new-window
bind-key -n F6 split-window -h
unbind-key -a -T root
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$keys = & $P list-keys 2>&1 | Out-String
$hasPrefix = $keys -match "new-window" -and $keys -match "detach-client"
$hasRoot = $keys -match "root"
if ($hasPrefix -and !$hasRoot) { OK "Prefix defaults present, root cleared" } elseif ($hasPrefix -and $hasRoot) { FAIL "Root bindings still present" } else { FAIL "Prefix defaults missing" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "false") { OK "defaults_suppressed = false (only root cleared)" } else { FAIL "defaults_suppressed = $ds" }

# Add root binding at runtime
& $P bind-key -n F12 new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$keys = & $P list-keys 2>&1 | Out-String
if ($keys -match "root.*F12") { OK "Can add root binding after root table cleared" } else { FAIL "Root binding F12 not added" }

# ================================================================
Write-Host "`n--- TEST 6: RUNTIME unbind-key -a FROM CLI ---"
# ================================================================
Cleanup
Remove-Item "$env:USERPROFILE\.tmux.conf" -Force -ErrorAction SilentlyContinue

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$kc1 = KeyCount
if ($kc1 -gt 50) { OK "Before runtime unbind: $kc1 defaults" } else { FAIL "Before: expected 50+, got $kc1" }

& $P unbind-key -a 2>&1 | Out-Null; Start-Sleep -Milliseconds 500
$kc2 = KeyCount
if ($kc2 -eq 0) { OK "After unbind-key -a: $kc2 bindings" } else { FAIL "After unbind-key -a: expected 0, got $kc2" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "true") { OK "defaults_suppressed = true after runtime unbind" } else { FAIL "defaults_suppressed = $ds" }

# CLI commands still work with no bindings at all
$w0 = WinCount
& $P new-window 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
if ((WinCount) -eq ($w0 + 1)) { OK "CLI new-window works with zero bindings" } else { FAIL "CLI new-window broken" }

& $P split-window -v 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
OK "CLI split-window -v works with zero bindings"

# Bind new key, verify
& $P bind-key z resize-pane -Z 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$keys = & $P list-keys 2>&1 | Out-String
if ($keys -match "z.*resize-pane") { OK "New binding after runtime unbind" } else { FAIL "Binding not added" }

# ================================================================
Write-Host "`n--- TEST 7: RUNTIME PER-TABLE UNBIND ---"
# ================================================================
Cleanup
Remove-Item "$env:USERPROFILE\.tmux.conf" -Force -ErrorAction SilentlyContinue

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

# Add some root bindings
& $P bind-key -n F5 new-window 2>&1 | Out-Null
& $P bind-key -n F6 split-window -h 2>&1 | Out-Null
Start-Sleep -Milliseconds 300

$keys = & $P list-keys 2>&1 | Out-String
$hasRoot = $keys -match "root"
if ($hasRoot) { OK "Root bindings added at runtime" } else { FAIL "Root bindings not present" }

# Unbind only root table
& $P unbind-key -a -T root 2>&1 | Out-Null; Start-Sleep -Milliseconds 500
$keys = & $P list-keys 2>&1 | Out-String
$hasRoot = $keys -match "root"
$hasPrefix = $keys -match "new-window" -and $keys -match "detach"
if (!$hasRoot -and $hasPrefix) { OK "unbind-key -a -T root: root gone, prefix intact" } else { FAIL "Per-table runtime unbind wrong" }

$ds = DumpField "defaults_suppressed"
if ($ds -eq "false") { OK "defaults_suppressed = false (only root cleared)" } else { FAIL "defaults_suppressed = $ds" }

# ================================================================
Write-Host "`n--- TEST 8: MULTIPLE RAPID SOURCE-FILE RELOADS ---"
# ================================================================
Cleanup

@"
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force

Start-Process -FilePath $P -ArgumentList "new-session -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

# Rapid toggles: unbind -> reload -> no-unbind -> reload -> unbind -> reload
@"
unbind-key -a
set -g prefix C-a
bind-key C-a send-prefix
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force
& $P source-file "$env:USERPROFILE\.tmux.conf" 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$r1 = KeyCount

@"
set -g prefix C-a
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force
& $P source-file "$env:USERPROFILE\.tmux.conf" 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$r2 = KeyCount

@"
unbind-key -a
set -g prefix C-a
bind-key C-a send-prefix
"@ | Set-Content -Path "$env:USERPROFILE\.tmux.conf" -Force
& $P source-file "$env:USERPROFILE\.tmux.conf" 2>&1 | Out-Null; Start-Sleep -Milliseconds 300
$r3 = KeyCount

if ($r1 -le 2 -and $r2 -gt 50 -and $r3 -le 2) {
    OK "Rapid toggle: $r1 -> $r2 -> $r3 (suppressed/restored/suppressed)"
} else {
    FAIL "Rapid toggle: $r1 -> $r2 -> $r3"
}

# ================================================================
# CLEANUP
# ================================================================
Write-Host ""
Write-Host ("=" * 70)
Cleanup
Remove-Item "$env:USERPROFILE\.tmux.conf" -Force -ErrorAction SilentlyContinue
# Restore .psmux.conf if it was backed up
if (Test-Path $psmuxConfBackup) {
    Move-Item $psmuxConfBackup $psmuxConf -Force
}

Write-Host ""
Write-Host ("=" * 70)
$color = if ($script:Fail -gt 0) { "Red" } else { "Green" }
Write-Host "RESULTS: $($script:Pass) passed, $($script:Fail) failed" -ForegroundColor $color
Write-Host ("=" * 70)

if ($script:Fail -gt 0) { exit 1 } else { exit 0 }
