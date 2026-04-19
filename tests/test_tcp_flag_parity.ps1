# =============================================================================
# PSMUX TCP Flag Parity Test Suite
# =============================================================================
#
# Tests EVERY flag of EVERY command via raw TCP socket to the PSMUX server,
# ensuring the server/connection.rs correctly handles all flag combinations.
#
# Usage: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_tcp_flag_parity.ps1
# =============================================================================

param(
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass  { param($msg) Write-Host "  [PASS] $msg" -ForegroundColor Green;  $script:TestsPassed++ }
function Write-Fail  { param($msg) Write-Host "  [FAIL] $msg" -ForegroundColor Red;    $script:TestsFailed++ }
function Write-Skip  { param($msg) Write-Host "  [SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info  { param($msg) Write-Host "  [INFO] $msg" -ForegroundColor Cyan }
function Write-Test  { param($msg) Write-Host "  [TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -EA SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -EA SilentlyContinue).Path }
if (-not $PSMUX) { $cmd = Get-Command psmux -EA SilentlyContinue; if ($cmd) { $PSMUX = $cmd.Source } }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Binary: $PSMUX"

$PSMUX_DIR = "$env:USERPROFILE\.psmux"
$SESSION = "tcpflag"

function Cleanup-Session {
    param([string]$Name)
    & $PSMUX kill-session -t $Name 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Remove-Item "$PSMUX_DIR\$Name.*" -Force -EA SilentlyContinue
}

function Wait-SessionReady {
    param([string]$Name, [int]$TimeoutMs = 15000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $pf = "$PSMUX_DIR\$Name.port"
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw).Trim()
            if ($port -match '^\d+$') {
                try {
                    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
                    $tcp.Close()
                    return $true
                } catch {}
            }
        }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

function Send-TcpCommand {
    param([string]$Session, [string]$Command, [int]$TimeoutMs = 5000)
    try {
        $portFile = "$PSMUX_DIR\$Session.port"
        $keyFile  = "$PSMUX_DIR\$Session.key"
        if (-not (Test-Path $portFile)) { return @{ ok=$false; err="NO_PORT_FILE" } }
        if (-not (Test-Path $keyFile))  { return @{ ok=$false; err="NO_KEY_FILE" } }
        $rawPort = Get-Content $portFile -Raw
        $rawKey  = Get-Content $keyFile -Raw
        if (-not $rawPort) { return @{ ok=$false; err="EMPTY_PORT_FILE" } }
        if (-not $rawKey)  { return @{ ok=$false; err="EMPTY_KEY_FILE" } }
        $port = $rawPort.Trim()
        $key  = $rawKey.Trim()
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.NoDelay = $true
        $tcp.Connect("127.0.0.1", [int]$port)
        $ns = $tcp.GetStream()
        $ns.ReadTimeout = $TimeoutMs
        $wr = New-Object System.IO.StreamWriter($ns); $wr.AutoFlush = $true
        $rd = New-Object System.IO.StreamReader($ns)
        $wr.WriteLine("AUTH $key")
        $auth = $rd.ReadLine()
        if ($auth -ne "OK") { $tcp.Close(); return @{ ok=$false; err="AUTH_FAIL: $auth" } }
        $wr.WriteLine($Command)
        $lines = @()
        try {
            while ($true) {
                $line = $rd.ReadLine()
                if ($null -eq $line) { break }
                $lines += $line
                if ($ns.DataAvailable -eq $false) {
                    Start-Sleep -Milliseconds 100
                    if ($ns.DataAvailable -eq $false) { break }
                }
            }
        } catch {}
        $tcp.Close()
        return @{ ok=$true; resp=($lines -join "`n"); lines=$lines }
    } catch { return @{ ok=$false; err=$_.Exception.Message } }
}

function Send-TcpAndVerify {
    param([string]$Label, [string]$Command, [switch]$ExpectOutput)
    $r = Send-TcpCommand $SESSION $Command
    if ($r.ok) {
        if ($ExpectOutput -and $r.lines.Count -eq 0) {
            Write-Fail "$Label (no output)"
        } else {
            Write-Pass "$Label"
        }
    } else {
        Write-Fail "$Label ($($r.err))"
    }
    return $r
}

# =============================================================================
# Setup
# =============================================================================

Write-Host "`n============================================================" -ForegroundColor Magenta
Write-Host "  PSMUX TCP Flag Parity Test Suite" -ForegroundColor Magenta
Write-Host "  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Magenta
Write-Host "============================================================`n" -ForegroundColor Magenta

Cleanup-Session $SESSION
Start-Sleep -Seconds 1
& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
if (-not (Wait-SessionReady $SESSION)) {
    Write-Fail "FATAL: Session did not start"
    exit 1
}
Start-Sleep -Seconds 3
Write-Pass "Session '$SESSION' ready"

# ════════════════════════════════════════════════════════════════════════════════
# 1. SET-OPTION: ALL flags -g -u -a -q -o -w -F and combinations
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 1. SET-OPTION FLAG MATRIX ===" -ForegroundColor Cyan

Send-TcpAndVerify "set-option -g mouse on" 'set-option -g mouse on'
Send-TcpAndVerify "set-option -g mouse off" 'set-option -g mouse off'
Send-TcpAndVerify "set-option -g status on" 'set-option -g status on'
Send-TcpAndVerify "set-option -g status off" 'set-option -g status off'
Send-TcpAndVerify "set-option -g escape-time 50" 'set-option -g escape-time 50'
Send-TcpAndVerify "set-option -g history-limit 10000" 'set-option -g history-limit 10000'
Send-TcpAndVerify "set-option -g base-index 0" 'set-option -g base-index 0'
Send-TcpAndVerify "set-option -g base-index 1" 'set-option -g base-index 1'
Send-TcpAndVerify "set-option -g pane-base-index 1" 'set-option -g pane-base-index 1'
Send-TcpAndVerify "set-option -g status-position top" 'set-option -g status-position top'
Send-TcpAndVerify "set-option -g status-position bottom" 'set-option -g status-position bottom'
Send-TcpAndVerify "set-option -g prefix C-b" 'set-option -g prefix C-b'
Send-TcpAndVerify "set-option -g prefix C-a" 'set-option -g prefix C-a'
Send-TcpAndVerify "set-option -g mode-keys vi" 'set-option -g mode-keys vi'
Send-TcpAndVerify "set-option -g mode-keys emacs" 'set-option -g mode-keys emacs'
Send-TcpAndVerify "set-option -g repeat-time 500" 'set-option -g repeat-time 500'
Send-TcpAndVerify "set-option -g display-time 2000" 'set-option -g display-time 2000'
Send-TcpAndVerify "set-option -g focus-events on" 'set-option -g focus-events on'
Send-TcpAndVerify "set-option -g set-clipboard on" 'set-option -g set-clipboard on'
Send-TcpAndVerify "set-option -g renumber-windows on" 'set-option -g renumber-windows on'
Send-TcpAndVerify "set-option -g aggressive-resize on" 'set-option -g aggressive-resize on'
Send-TcpAndVerify "set-option -g detach-on-destroy on" 'set-option -g detach-on-destroy on'
Send-TcpAndVerify "set-option -g default-shell pwsh" 'set-option -g default-shell pwsh'
Send-TcpAndVerify 'set-option -g word-separators " -_@"' 'set-option -g word-separators " -_@"'
Send-TcpAndVerify "set-option -g scroll-enter-copy-mode on" 'set-option -g scroll-enter-copy-mode on'
Send-TcpAndVerify 'set-option -g status-left "[S]"' 'set-option -g status-left "[S]"'
Send-TcpAndVerify 'set-option -g status-right "%H:%M"' 'set-option -g status-right "%H:%M"'
Send-TcpAndVerify 'set-option -g status-style "bg=blue"' 'set-option -g status-style "bg=blue"'
Send-TcpAndVerify 'set-option -g pane-border-style "fg=green"' 'set-option -g pane-border-style "fg=green"'
Send-TcpAndVerify 'set-option -g pane-active-border-style "fg=cyan"' 'set-option -g pane-active-border-style "fg=cyan"'

# Flag combinations
Send-TcpAndVerify "set-option -ga append" 'set-option -g status-right "PART1"'
Send-TcpAndVerify "set-option -ga status-right append" 'set-option -ga status-right " PART2"'
Send-TcpAndVerify "set-option -gu unset" 'set-option -gu status-right'
Send-TcpAndVerify "set-option -gq quiet unknown" 'set-option -gq nonexistent-xyz value'
Send-TcpAndVerify "set-option -go only-if-unset" 'set-option -go escape-time 999'
Send-TcpAndVerify "set-option -w window scope" 'set-option -w mouse on'
Send-TcpAndVerify "set-option @user-option" 'set-option -g @tcp-test value123'
Send-TcpAndVerify "set alias" 'set -g status on'
Send-TcpAndVerify "setw alias" 'setw -g mouse on'

# ════════════════════════════════════════════════════════════════════════════════
# 2. SHOW-OPTIONS: flags -v -g -q -A -w
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 2. SHOW-OPTIONS FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "show-options (all)" 'show-options' -ExpectOutput
Send-TcpAndVerify "show-options specific key" 'show-options mouse'
Send-TcpAndVerify "show alias" 'show mouse'
Send-TcpAndVerify "showw alias" 'showw mouse'

# ════════════════════════════════════════════════════════════════════════════════
# 3. BIND-KEY: flags -n -r -T
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 3. BIND-KEY FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "bind-key prefix table" 'bind-key z resize-pane -Z'
Send-TcpAndVerify "bind-key -n root table" 'bind-key -n F8 new-window'
Send-TcpAndVerify "bind-key -r repeat" 'bind-key -r Up resize-pane -U 5'
Send-TcpAndVerify "bind-key -T custom table" 'bind-key -T copy-mode-vi v send-keys -X begin-selection'
Send-TcpAndVerify "bind-key -nr combined" 'bind-key -nr M-Up resize-pane -U'
Send-TcpAndVerify "bind-key C-x (ctrl)" 'bind-key C-x kill-pane'
Send-TcpAndVerify "bind-key M-h (alt)" 'bind-key M-h select-pane -L'
Send-TcpAndVerify "bind alias" 'bind c new-window'

# ════════════════════════════════════════════════════════════════════════════════
# 4. UNBIND-KEY: flags -a -n -T
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 4. UNBIND-KEY FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "unbind-key specific" 'unbind-key z'
Send-TcpAndVerify "unbind-key -n root" 'unbind-key -n F8'
Send-TcpAndVerify "unbind-key -T named table" 'unbind-key -T copy-mode-vi v'
Send-TcpAndVerify "unbind-key -a (all)" 'unbind-key -a'
Send-TcpAndVerify "unbind alias" 'unbind c'

# Restore bindings
Send-TcpCommand $SESSION 'bind-key c new-window' | Out-Null

# ════════════════════════════════════════════════════════════════════════════════
# 5. LIST-KEYS: flags -T
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 5. LIST-KEYS FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "list-keys (all)" 'list-keys' -ExpectOutput
Send-TcpAndVerify "lsk alias" 'lsk'

# ════════════════════════════════════════════════════════════════════════════════
# 6. SET-HOOK: flags -g -a -u (combined -ga -gu -ag -ug)
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 6. SET-HOOK FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "set-hook -g basic" 'set-hook -g after-new-window "display-message hook1"'
Send-TcpAndVerify "set-hook -ga append" 'set-hook -ga after-new-window "display-message hook2"'
Send-TcpAndVerify "set-hook -ag append (reversed)" 'set-hook -ag after-split-window "display-message hook3"'
Send-TcpAndVerify "set-hook -gu unset" 'set-hook -gu after-new-window'
Send-TcpAndVerify "set-hook -ug unset (reversed)" 'set-hook -ug after-split-window'
Send-TcpAndVerify "set-hook overwrite" 'set-hook -g after-kill-pane "cmd1"'
Send-TcpAndVerify "set-hook overwrite same" 'set-hook -g after-kill-pane "cmd2"'

# ════════════════════════════════════════════════════════════════════════════════
# 7. SET-ENVIRONMENT / SHOW-ENVIRONMENT: flags -g -u -r
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 7. ENVIRONMENT FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "set-environment basic" 'set-environment TCP_VAR1 value1'
Send-TcpAndVerify "set-environment empty" 'set-environment TCP_VAR2'
Send-TcpAndVerify "set-environment -u unset" 'set-environment -u TCP_VAR1'
Send-TcpAndVerify "show-environment" 'show-environment'
Send-TcpAndVerify "setenv alias" 'setenv ALIAS_VAR val'
Send-TcpAndVerify "showenv alias" 'showenv'

# ════════════════════════════════════════════════════════════════════════════════
# 8. DISPLAY-MESSAGE: flags -p -d -I -t
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 8. DISPLAY-MESSAGE FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "display-message (no args)" 'display-message'
Send-TcpAndVerify "display-message text" 'display-message "hello tcp"'
Send-TcpAndVerify "display-message -p print" 'display-message -p "tcp print"'
Send-TcpAndVerify "display-message -d duration" 'display-message -d 3000 "timed"'
Send-TcpAndVerify "display-message -I" 'display-message -I "input"'
Send-TcpAndVerify "display-message -t target" 'display-message -t 0 "to pane"'
Send-TcpAndVerify "display-message format" 'display-message -p "#{session_name}"'
Send-TcpAndVerify "display alias" 'display "via alias"'

# ════════════════════════════════════════════════════════════════════════════════
# 9. IF-SHELL: flags -b -F
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 9. IF-SHELL FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "if-shell true" 'if-shell "true" "set-option -g @iftrue y"'
Send-TcpAndVerify "if-shell false+else" 'if-shell "false" "nop" "set-option -g @ifelse y"'
Send-TcpAndVerify "if-shell -F format true" 'if-shell -F "1" "set-option -g @fmt1 y"'
Send-TcpAndVerify "if-shell -F empty=false" 'if-shell -F "" "nop" "set-option -g @fmtempty y"'
Send-TcpAndVerify "if-shell -F 0=false" 'if-shell -F "0" "nop" "set-option -g @fmtzero y"'
Send-TcpAndVerify "if-shell literal 1" 'if-shell "1" "set-option -g @lit1 y"'

# ════════════════════════════════════════════════════════════════════════════════
# 10. RUN-SHELL: flags -b
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 10. RUN-SHELL FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "run-shell basic" 'run-shell "echo tcp_run"'
Send-TcpAndVerify "run-shell -b background" 'run-shell -b "echo tcp_bg"'
Send-TcpAndVerify "run alias" 'run "echo alias"'

# ════════════════════════════════════════════════════════════════════════════════
# 11. SPLIT-WINDOW: flags -h -v -p -l -c -d -b -f -F -P -Z -e
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 11. SPLIT-WINDOW FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "split-window default (vertical)" 'split-window'
Send-TcpAndVerify "split-window -h horizontal" 'split-window -h'
Send-TcpAndVerify "split-window -v explicit vert" 'split-window -v'
Send-TcpAndVerify "split-window -p 30 percent" 'split-window -p 30'
Send-TcpAndVerify "split-window -l 5 lines" 'split-window -l 5'
Send-TcpAndVerify "split-window -d detached" 'split-window -d'
Send-TcpAndVerify "split-window -b before" 'split-window -b'
Send-TcpAndVerify "split-window -f full width" 'split-window -f'
Send-TcpAndVerify 'split-window -c dir' 'split-window -v -c "C:\"'
Send-TcpAndVerify "split-window -e env" 'split-window -e TCPVAR=1'
Send-TcpAndVerify "split-window combined -h -p 40 -d" 'split-window -h -p 40 -d'
Send-TcpAndVerify "splitw alias" 'splitw -v'

# ════════════════════════════════════════════════════════════════════════════════
# 12. NEW-WINDOW: flags -n -d -c
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 12. NEW-WINDOW FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "new-window default" 'new-window'
Send-TcpAndVerify "new-window -n name" 'new-window -n tcpwin'
Send-TcpAndVerify "new-window -d detached" 'new-window -d'
Send-TcpAndVerify 'new-window -c dir' 'new-window -c "C:\"'
Send-TcpAndVerify "neww alias" 'neww'

# ════════════════════════════════════════════════════════════════════════════════
# 13. SELECT-PANE: flags -U -D -L -R -l -t -Z
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 13. SELECT-PANE FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "select-pane -U" 'select-pane -U'
Send-TcpAndVerify "select-pane -D" 'select-pane -D'
Send-TcpAndVerify "select-pane -L" 'select-pane -L'
Send-TcpAndVerify "select-pane -R" 'select-pane -R'
Send-TcpAndVerify "select-pane -l (last)" 'select-pane -l'
Send-TcpAndVerify "select-pane -t 0" 'select-pane -t 0'
Send-TcpAndVerify "select-pane -Z zoom" 'select-pane -Z'
Send-TcpAndVerify "selectp alias" 'selectp -D'

# ════════════════════════════════════════════════════════════════════════════════
# 14. RESIZE-PANE: flags -U -D -L -R -Z -x -y N
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 14. RESIZE-PANE FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "resize-pane -D 2" 'resize-pane -D 2'
Send-TcpAndVerify "resize-pane -U 2" 'resize-pane -U 2'
Send-TcpAndVerify "resize-pane -L 3" 'resize-pane -L 3'
Send-TcpAndVerify "resize-pane -R 3" 'resize-pane -R 3'
Send-TcpAndVerify "resize-pane -Z zoom" 'resize-pane -Z'
Send-TcpAndVerify "resize-pane -x 80" 'resize-pane -x 80'
Send-TcpAndVerify "resize-pane -y 20" 'resize-pane -y 20'
Send-TcpAndVerify "resizep alias" 'resizep -D 1'

# ════════════════════════════════════════════════════════════════════════════════
# 15. SWAP-PANE: flags -U -D
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 15. SWAP-PANE FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "swap-pane -U" 'swap-pane -U'
Send-TcpAndVerify "swap-pane -D" 'swap-pane -D'
Send-TcpAndVerify "swapp alias" 'swapp -D'

# ════════════════════════════════════════════════════════════════════════════════
# 16. ROTATE-WINDOW: flags -U -D
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 16. ROTATE-WINDOW FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "rotate-window default" 'rotate-window'
Send-TcpAndVerify "rotate-window -D down" 'rotate-window -D'
Send-TcpAndVerify "rotatew alias" 'rotatew'

# ════════════════════════════════════════════════════════════════════════════════
# 17. SEND-KEYS: flags -l -t + key names
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 17. SEND-KEYS FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "send-keys Enter" 'send-keys Enter'
Send-TcpAndVerify "send-keys Space" 'send-keys Space'
Send-TcpAndVerify "send-keys Escape" 'send-keys Escape'
Send-TcpAndVerify "send-keys Tab" 'send-keys Tab'
Send-TcpAndVerify "send-keys BSpace" 'send-keys BSpace'
Send-TcpAndVerify "send-keys -l literal" 'send-keys -l "literal text"'
Send-TcpAndVerify "send-keys -t 0" 'send-keys -t 0 Enter'
Send-TcpAndVerify "send-keys text+Enter" 'send-keys "echo hi" Enter'
Send-TcpAndVerify "send alias" 'send Enter'

# ════════════════════════════════════════════════════════════════════════════════
# 18. DISPLAY-POPUP: flags -w -h -d -c -E -K
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 18. DISPLAY-POPUP FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "display-popup -w" 'display-popup -w 40 "echo pop"'
Send-TcpAndVerify "display-popup -h" 'display-popup -h 20 "echo pop"'
Send-TcpAndVerify "display-popup -w -h" 'display-popup -w 60 -h 15 "echo pop"'
Send-TcpAndVerify "display-popup -E" 'display-popup -E "echo pop"'
Send-TcpAndVerify "display-popup -w 50% -h 50%" 'display-popup -w 50% -h 50% "echo pct"'
Send-TcpAndVerify "popup alias" 'popup "echo alias"'

# Close any popup
Start-Sleep -Milliseconds 300
Send-TcpCommand $SESSION 'send-keys Escape' | Out-Null

# ════════════════════════════════════════════════════════════════════════════════
# 19. SELECT-LAYOUT: all layout types
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 19. SELECT-LAYOUT ===" -ForegroundColor Cyan

Send-TcpAndVerify "select-layout tiled" 'select-layout tiled'
Send-TcpAndVerify "select-layout even-horizontal" 'select-layout even-horizontal'
Send-TcpAndVerify "select-layout even-vertical" 'select-layout even-vertical'
Send-TcpAndVerify "select-layout main-horizontal" 'select-layout main-horizontal'
Send-TcpAndVerify "select-layout main-vertical" 'select-layout main-vertical'
Send-TcpAndVerify "selectl alias" 'selectl tiled'
Send-TcpAndVerify "next-layout" 'next-layout'

# ════════════════════════════════════════════════════════════════════════════════
# 20. WINDOW-NAVIGATION
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 20. WINDOW NAVIGATION ===" -ForegroundColor Cyan

Send-TcpAndVerify "select-window by index" 'select-window -t 0'
Send-TcpAndVerify "next-window" 'next-window'
Send-TcpAndVerify "previous-window" 'previous-window'
Send-TcpAndVerify "last-window" 'last-window'
Send-TcpAndVerify "selectw alias" 'selectw -t 0'

# ════════════════════════════════════════════════════════════════════════════════
# 21. KILL OPERATIONS
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 21. KILL OPERATIONS ===" -ForegroundColor Cyan

# Create windows/panes to kill
Send-TcpCommand $SESSION 'new-window' | Out-Null; Start-Sleep -Seconds 2
Send-TcpCommand $SESSION 'split-window -v' | Out-Null; Start-Sleep -Seconds 2

Send-TcpAndVerify "kill-pane" 'kill-pane'
Start-Sleep -Seconds 1
Send-TcpAndVerify "kill-window" 'kill-window'
Start-Sleep -Seconds 1

# Recreate
Send-TcpCommand $SESSION 'new-window' | Out-Null; Start-Sleep -Seconds 2
Send-TcpAndVerify "killw alias" 'killw'
Start-Sleep -Seconds 1

Send-TcpCommand $SESSION 'new-window' | Out-Null; Start-Sleep -Seconds 2
Send-TcpCommand $SESSION 'split-window' | Out-Null; Start-Sleep -Seconds 2
Send-TcpAndVerify "killp alias" 'killp'

# ════════════════════════════════════════════════════════════════════════════════
# 22. SWAP/MOVE/LINK WINDOW
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 22. SWAP/MOVE/LINK WINDOW ===" -ForegroundColor Cyan

# Create windows
Send-TcpCommand $SESSION 'new-window' | Out-Null; Start-Sleep -Seconds 2
Send-TcpCommand $SESSION 'new-window' | Out-Null; Start-Sleep -Seconds 2

Send-TcpAndVerify "swap-window -s -t" 'swap-window -s 0 -t 1'
Send-TcpAndVerify "move-window -s -t" 'move-window -s 0 -t 5'
Send-TcpAndVerify "swapw alias" 'swapw -s 0 -t 1'
Send-TcpAndVerify "movew alias" 'movew -s 0 -t 3'

# ════════════════════════════════════════════════════════════════════════════════
# 23. BREAK/JOIN/RESPAWN PANE
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 23. BREAK/JOIN/RESPAWN PANE ===" -ForegroundColor Cyan

Send-TcpCommand $SESSION 'split-window -v' | Out-Null; Start-Sleep -Seconds 2
Send-TcpAndVerify "break-pane" 'break-pane'
Start-Sleep -Milliseconds 500
Send-TcpAndVerify "breakp alias" 'breakp'
Send-TcpAndVerify "respawn-pane -k" 'respawn-pane -k'
Send-TcpAndVerify "respawnp alias" 'respawnp -k'

# ════════════════════════════════════════════════════════════════════════════════
# 24. CAPTURE-PANE: flags -p -e -J
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 24. CAPTURE-PANE FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "capture-pane" 'capture-pane'
Send-TcpAndVerify "capture-pane -p" 'capture-pane -p'
Send-TcpAndVerify "capture-pane -e" 'capture-pane -e'
Send-TcpAndVerify "capture-pane -J" 'capture-pane -J'
Send-TcpAndVerify "capturep alias" 'capturep'

# ════════════════════════════════════════════════════════════════════════════════
# 25. RENAME-WINDOW / RENAME-SESSION
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 25. RENAME FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "rename-window" 'rename-window tcp_renamed'
Send-TcpAndVerify "rename-session" 'rename-session tcpflag_r'
# Restore session name (use renamed session name since port/key files now use it)
Start-Sleep -Milliseconds 500
Send-TcpCommand "tcpflag_r" 'rename-session tcpflag' | Out-Null
Start-Sleep -Milliseconds 500

# ════════════════════════════════════════════════════════════════════════════════
# 26. BUFFER OPERATIONS
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 26. BUFFER OPERATIONS ===" -ForegroundColor Cyan

Send-TcpAndVerify "set-buffer" 'set-buffer "tcp content"'
Send-TcpAndVerify "show-buffer" 'show-buffer'
Send-TcpAndVerify "list-buffers" 'list-buffers'
Send-TcpAndVerify "delete-buffer" 'delete-buffer'

# ════════════════════════════════════════════════════════════════════════════════
# 27. SOURCE-FILE
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 27. SOURCE-FILE FLAGS ===" -ForegroundColor Cyan

$tempConf = "$env:TEMP\psmux_tcp_test.conf"
Set-Content -Path $tempConf -Value "set-option -g @tcp-sourced yes"

Send-TcpAndVerify "source-file" "source-file $tempConf"
Send-TcpAndVerify "source-file -q nonexistent" 'source-file -q C:\no\such\file.conf'
Send-TcpAndVerify "source alias" "source $tempConf"

Remove-Item $tempConf -Force -EA SilentlyContinue

# ════════════════════════════════════════════════════════════════════════════════
# 28. COMMAND CHAINING (\;)
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 28. COMMAND CHAINING ===" -ForegroundColor Cyan

Send-TcpAndVerify "chain 2 commands" 'set-option -g @ch1 a \; set-option -g @ch2 b'
Send-TcpAndVerify "chain 3 commands" 'set-option -g @c1 x \; set-option -g @c2 y \; set-option -g @c3 z'

# ════════════════════════════════════════════════════════════════════════════════
# 29. WAIT-FOR: flags -L -S -U
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 29. WAIT-FOR FLAGS ===" -ForegroundColor Cyan

Send-TcpAndVerify "wait-for -S signal" 'wait-for -S tcp_chan'
Send-TcpAndVerify "wait-for -U unlock" 'wait-for -U tcp_chan'

# ════════════════════════════════════════════════════════════════════════════════
# 30. CLEAR-HISTORY / SHOW-HOOKS / SHOW-MESSAGES / CLOCK / INFO
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 30. MISC COMMANDS ===" -ForegroundColor Cyan

Send-TcpAndVerify "clear-history" 'clear-history'
Send-TcpAndVerify "show-hooks" 'show-hooks'
Send-TcpAndVerify "show-messages" 'show-messages'
Send-TcpAndVerify "clock-mode" 'clock-mode'
Send-TcpAndVerify "info" 'info'

# ════════════════════════════════════════════════════════════════════════════════
# 31. LIST COMMANDS VIA TCP
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 31. LIST COMMANDS ===" -ForegroundColor Cyan

Send-TcpAndVerify "list-windows" 'list-windows' -ExpectOutput
Send-TcpAndVerify "list-panes" 'list-panes' -ExpectOutput
Send-TcpAndVerify "list-clients" 'list-clients'
Send-TcpAndVerify "list-commands" 'list-commands'
Send-TcpAndVerify "lsw alias" 'lsw'
Send-TcpAndVerify "lsp alias" 'lsp'
Send-TcpAndVerify "lscm alias" 'lscm'

# ════════════════════════════════════════════════════════════════════════════════
# 32. COMMAND-PROMPT
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 32. COMMAND-PROMPT ===" -ForegroundColor Cyan

Send-TcpAndVerify "command-prompt" 'command-prompt'
# Dismiss it
Start-Sleep -Milliseconds 200
Send-TcpCommand $SESSION 'send-keys Escape' | Out-Null

Send-TcpAndVerify "command-prompt -I" 'command-prompt -I "split-window"'
Start-Sleep -Milliseconds 200
Send-TcpCommand $SESSION 'send-keys Escape' | Out-Null

# ════════════════════════════════════════════════════════════════════════════════
# 33. CHOOSER MODES
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 33. CHOOSER MODES ===" -ForegroundColor Cyan

Send-TcpAndVerify "choose-tree" 'choose-tree'
Start-Sleep -Milliseconds 200
Send-TcpCommand $SESSION 'send-keys Escape' | Out-Null

Send-TcpAndVerify "choose-window" 'choose-window'
Start-Sleep -Milliseconds 200
Send-TcpCommand $SESSION 'send-keys Escape' | Out-Null

Send-TcpAndVerify "choose-session" 'choose-session'
Start-Sleep -Milliseconds 200
Send-TcpCommand $SESSION 'send-keys Escape' | Out-Null

# ════════════════════════════════════════════════════════════════════════════════
# Cleanup & Summary
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== CLEANUP ===" -ForegroundColor Yellow
Cleanup-Session $SESSION
Start-Sleep -Seconds 1

Write-Host "`n============================================================" -ForegroundColor Magenta
Write-Host "  TCP FLAG PARITY RESULTS" -ForegroundColor Magenta
Write-Host "============================================================" -ForegroundColor Magenta
Write-Host "  PASSED:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  FAILED:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host "  SKIPPED: $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host "  TOTAL:   $($script:TestsPassed + $script:TestsFailed + $script:TestsSkipped)" -ForegroundColor White
Write-Host "============================================================`n" -ForegroundColor Magenta

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
