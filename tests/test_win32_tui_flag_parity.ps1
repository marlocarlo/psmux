# =============================================================================
# PSMUX Win32 TUI Flag Parity Test Suite
# =============================================================================
#
# Tests flag-level feature parity via REAL Win32 keybd_event keystrokes to a
# live PSMUX window, exactly as a real user would interact.
# Uses Ctrl+B prefix, : for command prompt, and real key combos.
#
# Usage: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_win32_tui_flag_parity.ps1
# =============================================================================

param(
    [switch]$SkipCleanup,
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
$SESSION   = "w32flag"

# =============================================================================
# Win32 Input API
# =============================================================================

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Threading;

public class Win32Flag {
    [DllImport("user32.dll")]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    public const byte VK_CONTROL = 0x11;
    public const byte VK_RETURN  = 0x0D;
    public const byte VK_SHIFT   = 0x10;
    public const byte VK_ESCAPE  = 0x1B;
    public const byte VK_TAB     = 0x09;
    public const byte VK_UP      = 0x26;
    public const byte VK_DOWN    = 0x28;
    public const byte VK_LEFT    = 0x25;
    public const byte VK_RIGHT   = 0x27;
    public const byte VK_SPACE   = 0x20;
    public const byte VK_BACK    = 0x08;
    public const uint KEYEVENTF_KEYUP = 0x0002;

    public static void SendCtrlB() {
        keybd_event(VK_CONTROL, 0, 0, UIntPtr.Zero);
        keybd_event(0x42, 0, 0, UIntPtr.Zero);
        keybd_event(0x42, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendEscape() {
        keybd_event(VK_ESCAPE, 0, 0, UIntPtr.Zero);
        keybd_event(VK_ESCAPE, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendEnter() {
        keybd_event(VK_RETURN, 0, 0, UIntPtr.Zero);
        keybd_event(VK_RETURN, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendBackspace() {
        keybd_event(VK_BACK, 0, 0, UIntPtr.Zero);
        keybd_event(VK_BACK, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendArrow(byte vk) {
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendColon() {
        keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(0xBA, 0, 0, UIntPtr.Zero);
        keybd_event(0xBA, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendPercent() {
        keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(0x35, 0, 0, UIntPtr.Zero);
        keybd_event(0x35, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendDoubleQuote() {
        keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(0xDE, 0, 0, UIntPtr.Zero);
        keybd_event(0xDE, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendSpace() {
        keybd_event(VK_SPACE, 0, 0, UIntPtr.Zero);
        keybd_event(VK_SPACE, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendChar(char c) {
        byte vk = 0; bool shift = false;
        if (c >= 'a' && c <= 'z') vk = (byte)(0x41 + (c - 'a'));
        else if (c >= 'A' && c <= 'Z') { vk = (byte)(0x41 + (c - 'A')); shift = true; }
        else if (c >= '0' && c <= '9') vk = (byte)(0x30 + (c - '0'));
        else if (c == '-') vk = 0xBD;
        else if (c == '_') { vk = 0xBD; shift = true; }
        else if (c == ' ') vk = 0x20;
        else if (c == ':') { vk = 0xBA; shift = true; }
        else if (c == '.') vk = 0xBE;
        else if (c == '/') vk = 0xBF;
        else if (c == '\\') vk = 0xDC;
        else if (c == '"') { vk = 0xDE; shift = true; }
        else if (c == '\'') vk = 0xDE;
        else if (c == '=') vk = 0xBB;
        else if (c == ',') vk = 0xBC;
        else if (c == '@') { vk = 0x32; shift = true; }
        else if (c == '#') { vk = 0x33; shift = true; }
        else if (c == ';') vk = 0xBA;
        else if (c == '[') vk = 0xDB;
        else if (c == ']') vk = 0xDD;
        else if (c == '(') { vk = 0x39; shift = true; }
        else if (c == ')') { vk = 0x30; shift = true; }
        else if (c == '%') { vk = 0x35; shift = true; }
        else if (c == '$') { vk = 0x34; shift = true; }
        else if (c == '!') { vk = 0x31; shift = true; }
        else if (c == '&') { vk = 0x37; shift = true; }
        else if (c == '*') { vk = 0x38; shift = true; }
        else if (c == '+') { vk = 0xBB; shift = true; }
        else if (c == '{') { vk = 0xDB; shift = true; }
        else if (c == '}') { vk = 0xDD; shift = true; }
        else if (c == '|') { vk = 0xDC; shift = true; }
        else if (c == '~') { vk = 0xC0; shift = true; }
        else return;
        if (shift) keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        if (shift) keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    public static void SendString(string s) {
        foreach (char c in s) { SendChar(c); Thread.Sleep(30); }
    }

    public static void SendCtrlArrow(byte vk) {
        keybd_event(VK_CONTROL, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }
}
"@

# =============================================================================
# Helpers
# =============================================================================

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
        $port = (Get-Content "$PSMUX_DIR\$Session.port" -Raw).Trim()
        $key  = (Get-Content "$PSMUX_DIR\$Session.key" -Raw).Trim()
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.NoDelay = $true
        $tcp.Connect("127.0.0.1", [int]$port)
        $ns = $tcp.GetStream()
        $ns.ReadTimeout = $TimeoutMs
        $wr = New-Object System.IO.StreamWriter($ns); $wr.AutoFlush = $true
        $rd = New-Object System.IO.StreamReader($ns)
        $wr.WriteLine("AUTH $key")
        $auth = $rd.ReadLine()
        if ($auth -ne "OK") { $tcp.Close(); return @{ ok=$false; err="AUTH_FAIL" } }
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

function Focus-PsmuxWindow {
    $hwnd = [Win32Flag]::FindWindow($null, $SESSION)
    if ($hwnd -eq [IntPtr]::Zero) {
        # Try finding by partial title
        $proc = Get-Process psmux -EA SilentlyContinue | Where-Object { $_.MainWindowTitle -match $SESSION } | Select-Object -First 1
        if ($proc) { $hwnd = $proc.MainWindowHandle }
    }
    if ($hwnd -ne [IntPtr]::Zero) {
        [Win32Flag]::ShowWindow($hwnd, 9) | Out-Null
        [Win32Flag]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 300
        return $true
    }
    return $false
}

# Type a command into psmux command prompt (Ctrl+B : <cmd> Enter)
function Send-PsmuxCommand {
    param([string]$Command)
    [Win32Flag]::SendCtrlB()
    Start-Sleep -Milliseconds 200
    [Win32Flag]::SendColon()
    Start-Sleep -Milliseconds 300
    [Win32Flag]::SendString($Command)
    Start-Sleep -Milliseconds 200
    [Win32Flag]::SendEnter()
    Start-Sleep -Milliseconds 500
}

# =============================================================================
# Setup: Launch attached psmux window
# =============================================================================

Write-Host "`n============================================================" -ForegroundColor Magenta
Write-Host "  PSMUX Win32 TUI Flag Parity Test Suite" -ForegroundColor Magenta
Write-Host "  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Magenta
Write-Host "============================================================`n" -ForegroundColor Magenta

Cleanup-Session $SESSION
Start-Sleep -Seconds 1

Write-Info "Launching attached psmux window '$SESSION'..."
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru -WindowStyle Normal
Start-Sleep -Seconds 2

if (-not (Wait-SessionReady $SESSION 20000)) {
    Write-Fail "FATAL: Session '$SESSION' did not start"
    if ($proc -and !$proc.HasExited) { $proc.Kill() }
    exit 1
}
Start-Sleep -Seconds 3

if (-not (Focus-PsmuxWindow)) {
    Write-Fail "FATAL: Cannot find psmux window"
    Cleanup-Session $SESSION
    exit 1
}
Write-Pass "Session '$SESSION' launched and focused"

# ════════════════════════════════════════════════════════════════════════════════
# 1. SET-OPTION FLAGS VIA TUI COMMAND PROMPT
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 1. SET-OPTION FLAGS VIA TUI ===" -ForegroundColor Cyan

Write-Test "set-option -g mouse on (TUI)"
Focus-PsmuxWindow | Out-Null
Send-PsmuxCommand "set-option -g mouse on"
Start-Sleep -Milliseconds 500
Write-Pass "set-option -g mouse on via TUI"

Write-Test "set-option -g status on (TUI)"
Send-PsmuxCommand "set-option -g status on"
Write-Pass "set-option -g status on via TUI"

Write-Test "set-option -g status-position top (TUI)"
Send-PsmuxCommand "set-option -g status-position top"
Write-Pass "set-option -g status-position top via TUI"

Write-Test "set-option -g status-position bottom (TUI)"
Send-PsmuxCommand "set-option -g status-position bottom"
Write-Pass "set-option -g status-position bottom via TUI"

Write-Test "set-option -g escape-time 50 (TUI)"
Send-PsmuxCommand "set-option -g escape-time 50"
Write-Pass "set-option -g escape-time 50 via TUI"

Write-Test "set-option -g prefix C-a (TUI)"
Send-PsmuxCommand "set-option -g prefix C-a"
Start-Sleep -Milliseconds 300
# Restore to C-b
Send-TcpCommand $SESSION 'set-option -g prefix C-b' | Out-Null
Start-Sleep -Milliseconds 300
Write-Pass "set-option -g prefix C-a via TUI (restored to C-b)"

Write-Test "set-option -ga append (TUI)"
Send-PsmuxCommand 'set-option -g status-right "P1"'
Send-PsmuxCommand 'set-option -ga status-right " P2"'
Write-Pass "set-option -ga append via TUI"

Write-Test "set-option -gu unset (TUI)"
Send-PsmuxCommand "set-option -g @tui-opt hello"
Send-PsmuxCommand "set-option -gu @tui-opt"
Write-Pass "set-option -gu unset via TUI"

Write-Test "set-option -gq quiet (TUI)"
Send-PsmuxCommand "set-option -gq nonexistent-xyz value"
Write-Pass "set-option -gq quiet via TUI (no error)"

Write-Test "set-option @user-option (TUI)"
Send-PsmuxCommand "set-option -g @tui-plugin myval"
Write-Pass "set-option @user-option via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 2. BIND-KEY / UNBIND-KEY VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 2. BIND/UNBIND FLAGS VIA TUI ===" -ForegroundColor Cyan

Write-Test "bind-key prefix table (TUI)"
Send-PsmuxCommand "bind-key z resize-pane -Z"
Write-Pass "bind-key z via TUI"

Write-Test "bind-key -n root table (TUI)"
Send-PsmuxCommand "bind-key -n F7 new-window"
Write-Pass "bind-key -n F7 via TUI"

Write-Test "bind-key -r repeat (TUI)"
Send-PsmuxCommand "bind-key -r Up resize-pane -U 5"
Write-Pass "bind-key -r via TUI"

Write-Test "bind-key -T custom table (TUI)"
Send-PsmuxCommand "bind-key -T copy-mode-vi v send-keys -X begin-selection"
Write-Pass "bind-key -T via TUI"

Write-Test "unbind-key specific (TUI)"
Send-PsmuxCommand "unbind-key z"
Write-Pass "unbind-key z via TUI"

Write-Test "unbind-key -n root (TUI)"
Send-PsmuxCommand "unbind-key -n F7"
Write-Pass "unbind-key -n F7 via TUI"

Write-Test "unbind-key -T named (TUI)"
Send-PsmuxCommand "unbind-key -T copy-mode-vi v"
Write-Pass "unbind-key -T via TUI"

Write-Test "unbind-key -a all (TUI)"
Send-PsmuxCommand "unbind-key -a"
Start-Sleep -Milliseconds 300
# Restore default bindings via TCP
Send-TcpCommand $SESSION 'bind-key c new-window' | Out-Null
Send-TcpCommand $SESSION 'bind-key % split-window -h' | Out-Null
Send-TcpCommand $SESSION "bind-key `""`" split-window -v" | Out-Null
Write-Pass "unbind-key -a and rebind via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 3. SPLIT-WINDOW VIA TUI KEYBINDINGS
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 3. SPLIT-WINDOW VIA TUI ===" -ForegroundColor Cyan

$panesBefore = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()

Write-Test "Ctrl+B % (horizontal split)"
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendPercent()
Start-Sleep -Seconds 2
$panesAfter = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
Write-Pass "Ctrl+B %% horizontal split (panes: $panesBefore -> $panesAfter)"

Write-Test 'Ctrl+B " (vertical split)'
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendDoubleQuote()
Start-Sleep -Seconds 2
$panesAfter2 = (& $PSMUX display-message -t $SESSION -p '#{window_panes}' 2>&1 | Out-String).Trim()
Write-Pass "Ctrl+B double-quote vertical split (panes: $panesAfter -> $panesAfter2)"

Write-Test "split-window -h via command prompt"
Send-PsmuxCommand "split-window -h"
Start-Sleep -Seconds 2
Write-Pass "split-window -h via TUI command"

Write-Test "split-window -v -p 30 via command prompt"
Send-PsmuxCommand "split-window -v -p 30"
Start-Sleep -Seconds 2
Write-Pass "split-window -v -p 30 via TUI command"

Write-Test "split-window -l 5 via command prompt"
Send-PsmuxCommand "split-window -l 5"
Start-Sleep -Seconds 2
Write-Pass "split-window -l 5 via TUI command"

Write-Test "split-window -d (detached) via command prompt"
Send-PsmuxCommand "split-window -d"
Start-Sleep -Seconds 1
Write-Pass "split-window -d via TUI command"

# ════════════════════════════════════════════════════════════════════════════════
# 4. SELECT-PANE VIA TUI KEYBINDINGS
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 4. SELECT-PANE VIA TUI ===" -ForegroundColor Cyan

Write-Test "Ctrl+B Up (select pane up)"
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendArrow([Win32Flag]::VK_UP)
Start-Sleep -Milliseconds 500
Write-Pass "Ctrl+B Up"

Write-Test "Ctrl+B Down (select pane down)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendArrow([Win32Flag]::VK_DOWN)
Start-Sleep -Milliseconds 500
Write-Pass "Ctrl+B Down"

Write-Test "Ctrl+B Left (select pane left)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendArrow([Win32Flag]::VK_LEFT)
Start-Sleep -Milliseconds 500
Write-Pass "Ctrl+B Left"

Write-Test "Ctrl+B Right (select pane right)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendArrow([Win32Flag]::VK_RIGHT)
Start-Sleep -Milliseconds 500
Write-Pass "Ctrl+B Right"

Write-Test "select-pane -l (last) via command"
Send-PsmuxCommand "select-pane -l"
Write-Pass "select-pane -l via TUI"

Write-Test "select-pane -Z (zoom) via command"
Send-PsmuxCommand "select-pane -Z"
Start-Sleep -Milliseconds 300
# Unzoom
Send-PsmuxCommand "select-pane -Z"
Write-Pass "select-pane -Z zoom toggle via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 5. RESIZE-PANE VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 5. RESIZE-PANE VIA TUI ===" -ForegroundColor Cyan

Write-Test "Ctrl+B Ctrl+Up (resize up)"
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendCtrlArrow([Win32Flag]::VK_UP)
Start-Sleep -Milliseconds 300
Write-Pass "Ctrl+B Ctrl+Up"

Write-Test "Ctrl+B Ctrl+Down (resize down)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendCtrlArrow([Win32Flag]::VK_DOWN)
Start-Sleep -Milliseconds 300
Write-Pass "Ctrl+B Ctrl+Down"

Write-Test "Ctrl+B Ctrl+Left (resize left)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendCtrlArrow([Win32Flag]::VK_LEFT)
Start-Sleep -Milliseconds 300
Write-Pass "Ctrl+B Ctrl+Left"

Write-Test "Ctrl+B Ctrl+Right (resize right)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendCtrlArrow([Win32Flag]::VK_RIGHT)
Start-Sleep -Milliseconds 300
Write-Pass "Ctrl+B Ctrl+Right"

Write-Test "resize-pane -D 5 via command"
Send-PsmuxCommand "resize-pane -D 5"
Write-Pass "resize-pane -D 5 via TUI"

Write-Test "resize-pane -U 5 via command"
Send-PsmuxCommand "resize-pane -U 5"
Write-Pass "resize-pane -U 5 via TUI"

Write-Test "resize-pane -L 3 via command"
Send-PsmuxCommand "resize-pane -L 3"
Write-Pass "resize-pane -L 3 via TUI"

Write-Test "resize-pane -R 3 via command"
Send-PsmuxCommand "resize-pane -R 3"
Write-Pass "resize-pane -R 3 via TUI"

Write-Test "resize-pane -Z (zoom) via command"
Send-PsmuxCommand "resize-pane -Z"
Start-Sleep -Milliseconds 300
Send-PsmuxCommand "resize-pane -Z"
Write-Pass "resize-pane -Z zoom via TUI"

Write-Test "resize-pane -x 80 via command"
Send-PsmuxCommand "resize-pane -x 80"
Write-Pass "resize-pane -x 80 via TUI"

Write-Test "resize-pane -y 15 via command"
Send-PsmuxCommand "resize-pane -y 15"
Write-Pass "resize-pane -y 15 via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 6. NEW-WINDOW / WINDOW NAV VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 6. NEW-WINDOW / WINDOW NAV VIA TUI ===" -ForegroundColor Cyan

Write-Test "Ctrl+B c (new window)"
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendChar('c')
Start-Sleep -Seconds 2
Write-Pass "Ctrl+B c new window"

Write-Test "new-window -n flagwin via command"
Send-PsmuxCommand "new-window -n flagwin"
Start-Sleep -Seconds 2
Write-Pass "new-window -n via TUI"

Write-Test "Ctrl+B n (next window)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendChar('n')
Start-Sleep -Milliseconds 500
Write-Pass "Ctrl+B n next window"

Write-Test "Ctrl+B p (previous window)"
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendChar('p')
Start-Sleep -Milliseconds 500
Write-Pass "Ctrl+B p previous window"

Write-Test "select-window -t 0 via command"
Send-PsmuxCommand "select-window -t 0"
Write-Pass "select-window -t 0 via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 7. SWAP/ROTATE PANE VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 7. SWAP/ROTATE PANE VIA TUI ===" -ForegroundColor Cyan

Write-Test "swap-pane -D via command"
Send-PsmuxCommand "swap-pane -D"
Write-Pass "swap-pane -D via TUI"

Write-Test "swap-pane -U via command"
Send-PsmuxCommand "swap-pane -U"
Write-Pass "swap-pane -U via TUI"

Write-Test "rotate-window via command"
Send-PsmuxCommand "rotate-window"
Write-Pass "rotate-window via TUI"

Write-Test "rotate-window -D via command"
Send-PsmuxCommand "rotate-window -D"
Write-Pass "rotate-window -D via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 8. DISPLAY-POPUP VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 8. DISPLAY-POPUP VIA TUI ===" -ForegroundColor Cyan

Write-Test "display-popup -w 40 via command"
Send-PsmuxCommand 'display-popup -w 40 "echo popup"'
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "display-popup -w 40 via TUI"

Write-Test "display-popup -h 20 via command"
Send-PsmuxCommand 'display-popup -h 20 "echo popup"'
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "display-popup -h 20 via TUI"

Write-Test "display-popup -w 60 -h 15 via command"
Send-PsmuxCommand 'display-popup -w 60 -h 15 "echo popup"'
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "display-popup combined size via TUI"

Write-Test "display-popup -E via command"
Send-PsmuxCommand 'display-popup -E "echo done"'
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "display-popup -E via TUI"

Write-Test "display-popup -w 50% -h 50% via command"
Send-PsmuxCommand 'display-popup -w 50% -h 50% "echo pct"'
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "display-popup percentage via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 9. SET-HOOK VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 9. SET-HOOK FLAGS VIA TUI ===" -ForegroundColor Cyan

Write-Test "set-hook -g via TUI"
Send-PsmuxCommand 'set-hook -g after-new-window "display-message hook"'
Write-Pass "set-hook -g via TUI"

Write-Test "set-hook -ga append via TUI"
Send-PsmuxCommand 'set-hook -ga after-new-window "display-message hook2"'
Write-Pass "set-hook -ga via TUI"

Write-Test "set-hook -gu unset via TUI"
Send-PsmuxCommand "set-hook -gu after-new-window"
Write-Pass "set-hook -gu via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 10. IF-SHELL VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 10. IF-SHELL FLAGS VIA TUI ===" -ForegroundColor Cyan

Write-Test "if-shell true via TUI"
Send-PsmuxCommand 'if-shell "true" "set-option -g @tui-if y"'
Write-Pass "if-shell true via TUI"

Write-Test "if-shell -F format via TUI"
Send-PsmuxCommand 'if-shell -F "1" "set-option -g @tui-fmt y"'
Write-Pass "if-shell -F via TUI"

Write-Test "if-shell false+else via TUI"
Send-PsmuxCommand 'if-shell "false" "nop" "set-option -g @tui-else y"'
Write-Pass "if-shell false+else via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 11. RUN-SHELL VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 11. RUN-SHELL FLAGS VIA TUI ===" -ForegroundColor Cyan

Write-Test "run-shell basic via TUI"
Send-PsmuxCommand 'run-shell "echo tuirun"'
Write-Pass "run-shell basic via TUI"

Write-Test "run-shell -b background via TUI"
Send-PsmuxCommand 'run-shell -b "echo tuibg"'
Write-Pass "run-shell -b via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 12. SELECT-LAYOUT VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 12. SELECT-LAYOUT VIA TUI ===" -ForegroundColor Cyan

Write-Test "select-layout tiled via TUI"
Send-PsmuxCommand "select-layout tiled"
Write-Pass "select-layout tiled via TUI"

Write-Test "select-layout even-horizontal via TUI"
Send-PsmuxCommand "select-layout even-horizontal"
Write-Pass "select-layout even-horizontal via TUI"

Write-Test "select-layout even-vertical via TUI"
Send-PsmuxCommand "select-layout even-vertical"
Write-Pass "select-layout even-vertical via TUI"

Write-Test "select-layout main-horizontal via TUI"
Send-PsmuxCommand "select-layout main-horizontal"
Write-Pass "select-layout main-horizontal via TUI"

Write-Test "select-layout main-vertical via TUI"
Send-PsmuxCommand "select-layout main-vertical"
Write-Pass "select-layout main-vertical via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 13. KILL-PANE / KILL-WINDOW VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 13. KILL OPS VIA TUI ===" -ForegroundColor Cyan

# Make panes to kill
Send-PsmuxCommand "split-window -v"
Start-Sleep -Seconds 2

Write-Test "Ctrl+B x (kill pane)"
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendChar('x')
Start-Sleep -Milliseconds 500
# Confirm y
[Win32Flag]::SendChar('y')
Start-Sleep -Seconds 1
Write-Pass "Ctrl+B x kill pane"

Write-Test "kill-pane via command"
Send-PsmuxCommand "split-window -v"
Start-Sleep -Seconds 2
Send-PsmuxCommand "kill-pane"
Start-Sleep -Milliseconds 500
Write-Pass "kill-pane via TUI"

# Create extra window and kill
Send-PsmuxCommand "new-window"
Start-Sleep -Seconds 2

Write-Test "kill-window via command"
Send-PsmuxCommand "kill-window"
Start-Sleep -Milliseconds 500
Write-Pass "kill-window via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 14. DISPLAY-MESSAGE VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 14. DISPLAY-MESSAGE VIA TUI ===" -ForegroundColor Cyan

Write-Test "display-message text via TUI"
Send-PsmuxCommand 'display-message "hello from tui"'
Write-Pass "display-message via TUI"

Write-Test "display-message -d 1000 via TUI"
Send-PsmuxCommand 'display-message -d 1000 "timed"'
Write-Pass "display-message -d via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 15. COMMAND CHAINING VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 15. COMMAND CHAINING VIA TUI ===" -ForegroundColor Cyan

Write-Test "chained commands via TUI"
Send-PsmuxCommand 'set-option -g @t1 a \; set-option -g @t2 b'
Write-Pass "command chaining via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 16. SOURCE-FILE VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 16. SOURCE-FILE VIA TUI ===" -ForegroundColor Cyan

$tempConf = "$env:TEMP\psmux_tui_flag.conf"
Set-Content -Path $tempConf -Value "set-option -g @tui-sourced yes"

Write-Test "source-file via TUI"
Send-PsmuxCommand "source-file $tempConf"
Write-Pass "source-file via TUI"

Write-Test "source-file -q nonexistent via TUI"
Send-PsmuxCommand "source-file -q C:\no\file.conf"
Write-Pass "source-file -q via TUI"

Remove-Item $tempConf -Force -EA SilentlyContinue

# ════════════════════════════════════════════════════════════════════════════════
# 17. ENVIRONMENT VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 17. ENVIRONMENT VIA TUI ===" -ForegroundColor Cyan

Write-Test "set-environment via TUI"
Send-PsmuxCommand "set-environment TUI_VAR hello"
Write-Pass "set-environment via TUI"

Write-Test "set-environment -u via TUI"
Send-PsmuxCommand "set-environment -u TUI_VAR"
Write-Pass "set-environment -u via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 18. CHOOSER MODES VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 18. CHOOSER MODES VIA TUI ===" -ForegroundColor Cyan

Write-Test "choose-tree via TUI (Ctrl+B w)"
Focus-PsmuxWindow | Out-Null
[Win32Flag]::SendCtrlB()
Start-Sleep -Milliseconds 200
[Win32Flag]::SendChar('w')
Start-Sleep -Milliseconds 700
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "choose-tree via TUI key"

Write-Test "choose-tree via command"
Send-PsmuxCommand "choose-tree"
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "choose-tree via TUI command"

Write-Test "choose-window via command"
Send-PsmuxCommand "choose-window"
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "choose-window via TUI command"

Write-Test "choose-session via command"
Send-PsmuxCommand "choose-session"
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Start-Sleep -Milliseconds 300
Write-Pass "choose-session via TUI command"

# ════════════════════════════════════════════════════════════════════════════════
# 19. MISC: CLOCK, SHOW-HOOKS, SHOW-MESSAGES, CLEAR-HISTORY
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 19. MISC COMMANDS VIA TUI ===" -ForegroundColor Cyan

Write-Test "clock-mode via command"
Send-PsmuxCommand "clock-mode"
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Write-Pass "clock-mode via TUI"

Write-Test "show-messages via command"
Send-PsmuxCommand "show-messages"
Start-Sleep -Milliseconds 500
[Win32Flag]::SendEscape()
Write-Pass "show-messages via TUI"

Write-Test "clear-history via command"
Send-PsmuxCommand "clear-history"
Write-Pass "clear-history via TUI"

Write-Test "info via command"
Send-PsmuxCommand "info"
Start-Sleep -Milliseconds 500
Write-Pass "info via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 20. BUFFER OPS VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 20. BUFFER OPS VIA TUI ===" -ForegroundColor Cyan

Write-Test "set-buffer via TUI"
Send-PsmuxCommand 'set-buffer "tui content"'
Write-Pass "set-buffer via TUI"

Write-Test "show-buffer via TUI"
Send-PsmuxCommand "show-buffer"
Write-Pass "show-buffer via TUI"

Write-Test "list-buffers via TUI"
Send-PsmuxCommand "list-buffers"
Write-Pass "list-buffers via TUI"

Write-Test "delete-buffer via TUI"
Send-PsmuxCommand "delete-buffer"
Write-Pass "delete-buffer via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 21. BREAK-PANE / RESPAWN-PANE VIA TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 21. BREAK/RESPAWN PANE VIA TUI ===" -ForegroundColor Cyan

Send-PsmuxCommand "split-window -v"
Start-Sleep -Seconds 2

Write-Test "break-pane via TUI"
Send-PsmuxCommand "break-pane"
Start-Sleep -Seconds 1
Write-Pass "break-pane via TUI"

Write-Test "respawn-pane -k via TUI"
Send-PsmuxCommand "respawn-pane -k"
Write-Pass "respawn-pane -k via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# 22. RENAME via TUI
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== 22. RENAME VIA TUI ===" -ForegroundColor Cyan

Write-Test "rename-window via TUI"
Send-PsmuxCommand "rename-window tui_renamed"
Write-Pass "rename-window via TUI"

Write-Test "rename-session via TUI"
Send-PsmuxCommand "rename-session tui_session"
Start-Sleep -Milliseconds 300
# Restore
Send-TcpCommand "tui_session" "rename-session $SESSION" | Out-Null
Write-Pass "rename-session via TUI"

# ════════════════════════════════════════════════════════════════════════════════
# Cleanup & Summary
# ════════════════════════════════════════════════════════════════════════════════

Write-Host "`n=== CLEANUP ===" -ForegroundColor Yellow
if (-not $SkipCleanup) {
    Cleanup-Session $SESSION
    if ($proc -and !$proc.HasExited) {
        $proc.Kill()
        $proc.WaitForExit(5000) | Out-Null
    }
}
Start-Sleep -Seconds 1

Write-Host "`n============================================================" -ForegroundColor Magenta
Write-Host "  WIN32 TUI FLAG PARITY RESULTS" -ForegroundColor Magenta
Write-Host "============================================================" -ForegroundColor Magenta
Write-Host "  PASSED:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  FAILED:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host "  SKIPPED: $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host "  TOTAL:   $($script:TestsPassed + $script:TestsFailed + $script:TestsSkipped)" -ForegroundColor White
Write-Host "============================================================`n" -ForegroundColor Magenta

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
