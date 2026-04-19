# Win32 TUI Helper Module for PSMUX Tests
#
# Dot-source this file to get Win32 TUI primitives for visual verification.
# Usage: . "$PSScriptRoot\tui_helper.ps1"
#
# Provides:
#   [TUI_H] class       - Win32 APIs (window discovery, keybd_event, focus)
#   Launch-PsmuxWindow   - Launch psmux, discover window, wait for session
#   Cleanup-PsmuxWindow  - Kill test session and process
#   Ensure-TuiFocus      - Ensure the psmux window has foreground focus
#   Send-PrefixKey       - Send Ctrl+B (prefix key) via keybd_event
#   Send-TuiKeys         - Type a string via keybd_event
#   Send-TuiKey          - Send a single VK key via keybd_event
#   Safe-TuiQuery        - Query session via display-message
#   TUI-CapturePane      - Capture pane content (what user sees)

if (-not ([System.Management.Automation.PSTypeName]'TUI_H').Type) {
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class TUI_H {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);

    public const byte VK_MENU    = 0x12;
    public const byte VK_CONTROL = 0x11;
    public const byte VK_SHIFT   = 0x10;
    public const byte VK_RETURN  = 0x0D;
    public const byte VK_ESCAPE  = 0x1B;
    public const byte VK_BACK    = 0x08;
    public const byte VK_TAB     = 0x09;
    public const byte VK_SPACE   = 0x20;
    public const byte VK_LEFT    = 0x25;
    public const byte VK_UP      = 0x26;
    public const byte VK_RIGHT   = 0x27;
    public const byte VK_DOWN    = 0x28;
    public const byte VK_DELETE  = 0x2E;
    public const byte VK_HOME    = 0x24;
    public const byte VK_END     = 0x23;
    public const byte VK_PRIOR   = 0x21;  // Page Up
    public const byte VK_NEXT    = 0x22;  // Page Down
    public const byte VK_F1      = 0x70;
    public const byte VK_F2      = 0x71;
    public const byte VK_F3      = 0x72;
    public const byte VK_F4      = 0x73;
    public const byte VK_F5      = 0x74;
    public const uint KEYUP      = 0x0002;

    public static HashSet<IntPtr> Snapshot() {
        var s = new HashSet<IntPtr>();
        EnumWindows((h, l) => { if (IsWindowVisible(h)) s.Add(h); return true; }, IntPtr.Zero);
        return s;
    }

    public static IntPtr FindNewest(HashSet<IntPtr> before) {
        IntPtr f = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (IsWindowVisible(h) && !before.Contains(h) && GetWindowTextLength(h) > 0) {
                var sb2 = new StringBuilder(256);
                GetWindowText(h, sb2, 256);
                string t = sb2.ToString();
                if (!t.Contains("Visual Studio Code") && !t.Contains("Code -")) {
                    f = h; return false;
                }
            }
            return true;
        }, IntPtr.Zero);
        return f;
    }

    public static IntPtr FindByTitle(string needle) {
        IntPtr f = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (IsWindowVisible(h) && GetWindowTextLength(h) > 0) {
                var sb2 = new StringBuilder(512);
                GetWindowText(h, sb2, 512);
                string t = sb2.ToString();
                if (t.IndexOf(needle, StringComparison.OrdinalIgnoreCase) >= 0
                    && !t.Contains("Visual Studio Code") && !t.Contains("Code -")) {
                    f = h; return false;
                }
            }
            return true;
        }, IntPtr.Zero);
        return f;
    }

    public static string Title(IntPtr h) {
        int len = GetWindowTextLength(h); if (len <= 0) return "";
        var sb = new StringBuilder(len + 1); GetWindowText(h, sb, sb.Capacity); return sb.ToString();
    }

    public static bool Focus(IntPtr h) {
        keybd_event(VK_MENU, 0, 0, UIntPtr.Zero);
        ShowWindow(h, 9);
        BringWindowToTop(h);
        SetForegroundWindow(h);
        keybd_event(VK_MENU, 0, KEYUP, UIntPtr.Zero);
        System.Threading.Thread.Sleep(300);
        return GetForegroundWindow() == h;
    }

    public static void Key(byte vk, bool shift) {
        if (shift) keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(30);
        keybd_event(vk, 0, KEYUP, UIntPtr.Zero);
        if (shift) { System.Threading.Thread.Sleep(10); keybd_event(VK_SHIFT, 0, KEYUP, UIntPtr.Zero); }
    }

    public static void CtrlKey(byte vk) {
        keybd_event(VK_CONTROL, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(20);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(40);
        keybd_event(vk, 0, KEYUP, UIntPtr.Zero);
        System.Threading.Thread.Sleep(10);
        keybd_event(VK_CONTROL, 0, KEYUP, UIntPtr.Zero);
    }

    public static void Enter() { Key(VK_RETURN, false); }
    public static void Escape() { Key(VK_ESCAPE, false); }
    public static void Tab() { Key(VK_TAB, false); }

    public static void TypeChar(char c) {
        byte vk = 0; bool shift = false;
        if      (c >= 'a' && c <= 'z') vk = (byte)(0x41 + (c - 'a'));
        else if (c >= 'A' && c <= 'Z') { vk = (byte)(0x41 + (c - 'A')); shift = true; }
        else if (c >= '0' && c <= '9') vk = (byte)(0x30 + (c - '0'));
        else if (c == '-') vk = 0xBD;
        else if (c == ' ') vk = 0x20;
        else if (c == ':') { vk = 0xBA; shift = true; }
        else if (c == ';') vk = 0xBA;
        else if (c == '.') vk = 0xBE;
        else if (c == '/') vk = 0xBF;
        else if (c == '\\') vk = 0xDC;
        else if (c == '=') vk = 0xBB;
        else if (c == ',') vk = 0xBC;
        else if (c == '%') { vk = 0x35; shift = true; }
        else if (c == '!') { vk = 0x31; shift = true; }
        else if (c == '@') { vk = 0x32; shift = true; }
        else if (c == '#') { vk = 0x33; shift = true; }
        else if (c == '_') { vk = 0xBD; shift = true; }
        else if (c == '\'') vk = 0xDE;
        else if (c == '"') { vk = 0xDE; shift = true; }
        else if (c == '[') vk = 0xDB;
        else if (c == ']') vk = 0xDD;
        else if (c == '{') { vk = 0xDB; shift = true; }
        else if (c == '}') { vk = 0xDD; shift = true; }
        else if (c == '(') { vk = 0x39; shift = true; }
        else if (c == ')') { vk = 0x30; shift = true; }
        else if (c == '+') { vk = 0xBB; shift = true; }
        else if (c == '|') { vk = 0xDC; shift = true; }
        else if (c == '<') { vk = 0xBC; shift = true; }
        else if (c == '>') { vk = 0xBE; shift = true; }
        else return;
        Key(vk, shift);
    }

    public static void TypeString(string s) {
        foreach (char c in s) {
            TypeChar(c);
            System.Threading.Thread.Sleep(30);
        }
    }
}
"@
}

# ── State variables for the calling script ──
$script:TUI_HWND = [IntPtr]::Zero
$script:TUI_PROC = $null
$script:TUI_SESSION = ""
$script:TUI_PSMUX = (Get-Command psmux -EA SilentlyContinue)?.Source
if (-not $script:TUI_PSMUX) { $script:TUI_PSMUX = "psmux" }

function Launch-PsmuxWindow {
    param(
        [string]$Session,
        [int]$TimeoutMs = 20000
    )
    $script:TUI_SESSION = $Session

    # Pre-cleanup
    & $script:TUI_PSMUX kill-session -t $Session 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Snapshot windows
    $snap = [TUI_H]::Snapshot()
    Write-Host "  [TUI] $($snap.Count) windows before launch" -ForegroundColor DarkGray

    # Launch visible (non-detached) psmux session
    $script:TUI_PROC = Start-Process -FilePath $script:TUI_PSMUX -ArgumentList "new-session","-s",$Session -PassThru
    Start-Sleep -Seconds 2

    # Discover the new window
    $script:TUI_HWND = [TUI_H]::FindNewest($snap)
    if ($script:TUI_HWND -eq [IntPtr]::Zero) {
        # Fallback: find by psmux.exe in title
        Start-Sleep -Seconds 1
        $script:TUI_HWND = [TUI_H]::FindByTitle("psmux")
        if ($script:TUI_HWND -eq [IntPtr]::Zero) {
            $script:TUI_HWND = [TUI_H]::FindByTitle($script:TUI_PSMUX)
        }
    }

    if ($script:TUI_HWND -eq [IntPtr]::Zero) {
        Write-Host "  [TUI] WARNING: Could not find psmux window" -ForegroundColor Yellow
        return $false
    }

    $title = [TUI_H]::Title($script:TUI_HWND)
    Write-Host "  [TUI] Found window: '$title' (hwnd=$($script:TUI_HWND))" -ForegroundColor DarkGray

    # Wait for session to be ready
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $out = & $script:TUI_PSMUX has-session -t $Session 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host "  [TUI] Session '$Session' ready" -ForegroundColor DarkGray
            return $true
        }
        Start-Sleep -Milliseconds 200
    }
    Write-Host "  [TUI] WARNING: Session '$Session' not ready within timeout" -ForegroundColor Yellow
    return $false
}

function Cleanup-PsmuxWindow {
    param([string]$Session = $script:TUI_SESSION)
    if ($Session) { & $script:TUI_PSMUX kill-session -t $Session 2>&1 | Out-Null }
    if ($script:TUI_PROC -and -not $script:TUI_PROC.HasExited) {
        $script:TUI_PROC.Kill()
        $script:TUI_PROC.WaitForExit(3000)
    }
    $script:TUI_HWND = [IntPtr]::Zero
    $script:TUI_PROC = $null
}

function Ensure-TuiFocus {
    if ($script:TUI_HWND -eq [IntPtr]::Zero) { return $false }
    for ($i = 0; $i -lt 5; $i++) {
        if ([TUI_H]::Focus($script:TUI_HWND)) { return $true }
        Start-Sleep -Milliseconds 300
    }
    Write-Host "  [TUI] WARNING: Could not focus psmux window" -ForegroundColor Yellow
    return $false
}

function Send-PrefixKey {
    # Ctrl+B (default prefix)
    if (-not (Ensure-TuiFocus)) { return $false }
    [TUI_H]::CtrlKey(0x42)  # 0x42 = 'B'
    Start-Sleep -Milliseconds 200
    return $true
}

function Send-TuiKeys {
    param([string]$Text)
    if (-not (Ensure-TuiFocus)) { return $false }
    [TUI_H]::TypeString($Text)
    return $true
}

function Send-TuiKey {
    param(
        [byte]$VK,
        [switch]$Shift,
        [switch]$Ctrl
    )
    if (-not (Ensure-TuiFocus)) { return $false }
    if ($Ctrl) { [TUI_H]::CtrlKey($VK) }
    else { [TUI_H]::Key($VK, $Shift.IsPresent) }
    return $true
}

function Send-TuiEnter { Ensure-TuiFocus | Out-Null; [TUI_H]::Enter() }
function Send-TuiEscape { Ensure-TuiFocus | Out-Null; [TUI_H]::Escape() }

function Safe-TuiQuery {
    param([string]$Fmt, [string]$Session = $script:TUI_SESSION)
    $r = & $script:TUI_PSMUX display-message -t $Session -p $Fmt 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return $null }
    return $r.Trim()
}

function TUI-CapturePane {
    param(
        [string]$Session = $script:TUI_SESSION,
        [string]$StartLine = "",
        [string]$EndLine = ""
    )
    $args_ = @("capture-pane", "-t", $Session, "-p")
    if ($StartLine) { $args_ += "-S"; $args_ += $StartLine }
    if ($EndLine) { $args_ += "-E"; $args_ += $EndLine }
    $r = & $script:TUI_PSMUX @args_ 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return $null }
    return $r
}
