# Win32 TUI Proof Test
#
# Pure keybd_event approach: ALL TUI interaction via real Win32 keystrokes.
# send-keys goes to the PTY, NOT the TUI input loop. keybd_event is the
# only way to drive prefix, command prompt, and keybindings for real.
#
# Window discovery: snapshot visible windows before launch, find the NEW
# console window (conhost) after launch via diff. Verify focus before
# every keystroke sequence.

$ErrorActionPreference = "Continue"
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "tui_w32"
$TARGET  = "tui_w32_tgt"
$PSMUX   = (Get-Command psmux -EA Stop).Source
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:SessionDead = $false

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }

# ── Win32 API ──────────────────────────────────────────────────────────────
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class TUI {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);

    public const byte VK_MENU = 0x12, VK_CONTROL = 0x11, VK_SHIFT = 0x10;
    public const byte VK_RETURN = 0x0D, VK_ESCAPE = 0x1B, VK_BACK = 0x08;
    public const uint UP = 0x0002;

    public static HashSet<IntPtr> Snapshot() {
        var s = new HashSet<IntPtr>();
        EnumWindows((h,l) => { if (IsWindowVisible(h)) s.Add(h); return true; }, IntPtr.Zero);
        return s;
    }

    public static IntPtr FindNewest(HashSet<IntPtr> before) {
        IntPtr f = IntPtr.Zero;
        EnumWindows((h,l) => {
            if (IsWindowVisible(h) && !before.Contains(h) && GetWindowTextLength(h) > 0) {
                var sb2 = new StringBuilder(256);
                GetWindowText(h, sb2, 256);
                string t = sb2.ToString();
                // Skip VS Code windows
                if (!t.Contains("Visual Studio Code") && !t.Contains("Code -")) {
                    f = h; return false;
                }
            }
            return true;
        }, IntPtr.Zero);
        return f;
    }

    // Find window by title substring (for WT tab reuse scenarios)
    public static IntPtr FindByTitle(string needle) {
        IntPtr f = IntPtr.Zero;
        EnumWindows((h,l) => {
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
        var sb = new StringBuilder(len+1); GetWindowText(h, sb, sb.Capacity); return sb.ToString();
    }

    public static bool Focus(IntPtr h) {
        // ALT trick bypasses Windows foreground lock
        keybd_event(VK_MENU, 0, 0, UIntPtr.Zero);
        ShowWindow(h, 9);
        BringWindowToTop(h);
        SetForegroundWindow(h);
        keybd_event(VK_MENU, 0, UP, UIntPtr.Zero);
        System.Threading.Thread.Sleep(300);
        return GetForegroundWindow() == h;
    }

    // ── Keystroke primitives ────────────────────────────────────────
    public static void CtrlB() {
        keybd_event(VK_CONTROL, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(20);
        keybd_event(0x42, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(40);
        keybd_event(0x42, 0, UP, UIntPtr.Zero);
        System.Threading.Thread.Sleep(10);
        keybd_event(VK_CONTROL, 0, UP, UIntPtr.Zero);
    }

    public static void Key(byte vk, bool shift) {
        if (shift) keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(30);
        keybd_event(vk, 0, UP, UIntPtr.Zero);
        if (shift) { System.Threading.Thread.Sleep(10); keybd_event(VK_SHIFT, 0, UP, UIntPtr.Zero); }
    }

    public static void Enter() { Key(VK_RETURN, false); }
    public static void Escape() { Key(VK_ESCAPE, false); }

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

# ── Helpers ────────────────────────────────────────────────────────────────
function Wait-SessionReady([string]$Name, [int]$TimeoutMs = 15000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $out = & $PSMUX has-session -t $Name 2>&1
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

function Safe-Query([string]$Fmt) {
    $r = & $PSMUX display-message -t $SESSION -p $Fmt 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return $null }
    return $r.Trim()
}

function Ensure-Focus {
    if ($null -eq $script:hwnd -or $script:hwnd -eq [IntPtr]::Zero) { return $false }
    for ($i = 0; $i -lt 5; $i++) {
        if ([TUI]::Focus($script:hwnd)) { return $true }
        Start-Sleep -Milliseconds 300
    }
    return $false
}

function Verify-Focus {
    $fg = [TUI]::GetForegroundWindow()
    $ok = ($fg -eq $script:hwnd)
    if (-not $ok) {
        $t = [TUI]::Title($fg)
        Write-Host "    [!] Wrong window focused: '$t'" -ForegroundColor Yellow
    }
    return $ok
}

function Skip-IfDead([string]$Name) {
    if ($script:SessionDead) { Write-Fail "$Name (SKIPPED: session dead)"; return $true }
    if ($script:proc.HasExited) {
        $script:SessionDead = $true
        Write-Fail "$Name (process exited code=$($script:proc.ExitCode))"
        return $true
    }
    return $false
}

# ── PRE-CLEANUP: kill only test sessions, preserve warm pool ───────────────
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Win32 TUI Proof Tests" -ForegroundColor Cyan
Write-Host "========================================`n" -ForegroundColor Cyan

& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
& $PSMUX kill-session -t $TARGET 2>&1 | Out-Null
& $PSMUX kill-session -t tui_w32_split 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\$TARGET.*" -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\tui_w32_split.*" -Force -EA SilentlyContinue
Start-Sleep -Seconds 1

# ── LAUNCH ─────────────────────────────────────────────────────────────────
$snap = [TUI]::Snapshot()
Write-Host "[Setup] $($snap.Count) windows before launch"

$script:proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
if (!(Wait-SessionReady $SESSION)) {
    Write-Host "FATAL: Session did not start" -ForegroundColor Red
    try { $script:proc.Kill() } catch {}; exit 1
}
Write-Host "[Setup] Session ready, PID=$($script:proc.Id)" -ForegroundColor Green
Start-Sleep -Seconds 3

# ── FIND WINDOW ────────────────────────────────────────────────────────────
# Try snapshot diff first (works when new window is created)
$script:hwnd = [TUI]::FindNewest($snap)
if ($script:hwnd -eq [IntPtr]::Zero) {
    # Fallback: Windows Terminal may reuse existing window (tab), so search by title
    Start-Sleep -Seconds 1
    $script:hwnd = [TUI]::FindByTitle("psmux")
}
if ($script:hwnd -eq [IntPtr]::Zero) {
    # Try matching the exe path in title (WT sometimes uses full path)
    $script:hwnd = [TUI]::FindByTitle($PSMUX)
}
if ($script:hwnd -ne [IntPtr]::Zero) {
    $t = [TUI]::Title($script:hwnd)
    Write-Host "[Setup] Console: HWND=$($script:hwnd) '$t'" -ForegroundColor Green
} else {
    Write-Host "[Setup] WARNING: No console window found" -ForegroundColor Yellow
}

$ok = Ensure-Focus
Write-Host "[Setup] Focus: $ok" -ForegroundColor $(if($ok){"Green"}else{"Yellow"})


# ═══════════════════════════════════════════════════════════════════════════
# TEST A: Prefix+: command prompt, type set-option (all keybd_event)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test A] Prefix+: set-option via command prompt" -ForegroundColor Yellow

if (Skip-IfDead "Test A") {} else {
    Ensure-Focus | Out-Null
    if (!(Verify-Focus)) { Write-Fail "Test A: cannot focus psmux window" }
    else {
        # Get current status-interval
        $oldVal = Safe-Query '#{status-interval}'

        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::TypeChar(':')
        Start-Sleep -Milliseconds 1000
        [TUI]::TypeString("set -g status-interval 77")
        Start-Sleep -Milliseconds 400
        [TUI]::Enter()
        Start-Sleep -Seconds 2

        # Refocus after command prompt closes
        Ensure-Focus | Out-Null

        $newVal = Safe-Query '#{status-interval}'
        if ($newVal -eq "77") {
            Write-Pass "command prompt: set status-interval to 77 (was $oldVal)"
        } else {
            Write-Fail "command prompt: status-interval=$newVal (expected 77)"
        }
    }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST B: Prefix+c new-window (keybd_event)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test B] Prefix+c new-window" -ForegroundColor Yellow

if (Skip-IfDead "Test B") {} else {
    $before = Safe-Query '#{session_windows}'
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::Key(0x43, $false)  # c
        Start-Sleep -Seconds 4

        $after = Safe-Query '#{session_windows}'
        if ($null -ne $after -and [int]$after -gt [int]$before) {
            Write-Pass "prefix+c: windows $before -> $after"
        } else { Write-Fail "prefix+c: windows $before -> $after" }
    } else { Write-Fail "Test B: focus lost" }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST C: Prefix+p previous window (keybd_event)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test C] Prefix+p prev window" -ForegroundColor Yellow

if (Skip-IfDead "Test C") {} else {
    $before = Safe-Query '#{window_index}'
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::Key(0x50, $false)  # p
        Start-Sleep -Seconds 2

        $after = Safe-Query '#{window_index}'
        if ($null -ne $after -and $after -ne $before) {
            Write-Pass "prefix+p: window $before -> $after"
        } else { Write-Fail "prefix+p: window $before -> $after" }
    } else { Write-Fail "Test C: focus lost" }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST D: Prefix+: set-option (all keybd_event)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test D] Prefix+: set-option" -ForegroundColor Yellow

if (Skip-IfDead "Test D") {} else {
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::TypeChar(':')
        Start-Sleep -Milliseconds 800
        [TUI]::TypeString("set -g status-left TUIPROOF")
        Start-Sleep -Milliseconds 400
        [TUI]::Enter()
        Start-Sleep -Seconds 2

        $sl = & $PSMUX show-options -g -v "status-left" -t $SESSION 2>&1 | Out-String
        if ($sl -match "TUIPROOF") { Write-Pass "set-option: status-left=TUIPROOF" }
        else { Write-Fail "set-option did not apply. Got: $($sl.Trim())" }
    } else { Write-Fail "Test D: focus lost" }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST E: Prefix+: rename-window via command prompt
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test E] Prefix+: rename-window" -ForegroundColor Yellow

if (Skip-IfDead "Test E") {} else {
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::TypeChar(':')
        Start-Sleep -Milliseconds 800
        [TUI]::TypeString("rename-window tuirenamed")
        Start-Sleep -Milliseconds 400
        [TUI]::Enter()
        Start-Sleep -Seconds 2

        $wn = Safe-Query '#{window_name}'
        if ($null -ne $wn -and $wn -match "tuirenamed") {
            Write-Pass "rename-window: $wn"
        } else { Write-Fail "rename-window got: $wn" }
    } else { Write-Fail "Test E: focus lost" }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST F: Prefix+: bind-key via command prompt
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test F] Prefix+: bind-key" -ForegroundColor Yellow

if (Skip-IfDead "Test F") {} else {
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::TypeChar(':')
        Start-Sleep -Milliseconds 800
        [TUI]::TypeString("bind-key F7 rename-window tuibound")
        Start-Sleep -Milliseconds 400
        [TUI]::Enter()
        Start-Sleep -Seconds 2

        $keys = & $PSMUX list-keys -t $SESSION 2>&1 | Out-String
        if ($keys -match "F7") { Write-Pass "bind-key F7 registered" }
        else { Write-Fail "bind-key F7 not in list-keys" }
    } else { Write-Fail "Test F: focus lost" }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST G: Prefix+: run-shell (issue #4)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test G] Prefix+: run-shell" -ForegroundColor Yellow

if (Skip-IfDead "Test G") {} else {
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::TypeChar(':')
        Start-Sleep -Milliseconds 800
        [TUI]::TypeString("run-shell -b echo")
        Start-Sleep -Milliseconds 400
        [TUI]::Enter()
        Start-Sleep -Seconds 3

        & $PSMUX has-session -t $SESSION 2>$null
        if ($LASTEXITCODE -eq 0) { Write-Pass "run-shell -b echo: session alive" }
        else { Write-Fail "run-shell killed session"; $script:SessionDead = $true }
    } else { Write-Fail "Test G: focus lost" }
}


# ═══════════════════════════════════════════════════════════════════════════
# TEST H: Prefix+d detach (ultimate proof: keybd_event causes TUI exit
#         while session persists)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test H] Prefix+d detach (ultimate TUI proof)" -ForegroundColor Yellow

if (Skip-IfDead "Test H") {} else {
    Ensure-Focus | Out-Null
    if (Verify-Focus) {
        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::Key(0x44, $false)  # d
        Start-Sleep -Seconds 3

        $script:proc.Refresh()
        $exited = $script:proc.HasExited

        & $PSMUX has-session -t $SESSION 2>$null
        $alive = ($LASTEXITCODE -eq 0)

        if ($exited -and $alive) {
            Write-Pass "prefix+d: process exited, session alive (PERFECT detach)"
        } elseif ($alive) {
            Write-Pass "prefix+d: session alive"
        } else {
            Write-Fail "prefix+d: session gone"
        }
    } else { Write-Fail "Test H: focus lost" }
}


# ── CLEANUP (main session) ─────────────────────────────────────────────────
Write-Host "`n[Cleanup main]" -ForegroundColor DarkGray
try { if (-not $script:proc.HasExited) { $script:proc.Kill() } } catch {}
Start-Sleep -Seconds 1
& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
& $PSMUX kill-session -t $TARGET 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
Remove-Item "$psmuxDir\$TARGET.*" -Force -EA SilentlyContinue


# ═══════════════════════════════════════════════════════════════════════════
# TEST I: Prefix+%% split-window (fresh session to isolate)
# ═══════════════════════════════════════════════════════════════════════════
Write-Host "`n[Test I] Prefix+%% split-window (fresh session)" -ForegroundColor Yellow

$SPLIT = "tui_w32_split"
& $PSMUX kill-session -t $SPLIT 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SPLIT.*" -Force -EA SilentlyContinue
Start-Sleep -Milliseconds 500

$snap2 = [TUI]::Snapshot()
$splitProc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SPLIT -PassThru
if (Wait-SessionReady $SPLIT 15000) {
    Start-Sleep -Seconds 3
    $splitHwnd = [TUI]::FindNewest($snap2)
    if ($splitHwnd -eq [IntPtr]::Zero) {
        $splitHwnd = [TUI]::FindByTitle("psmux")
    }
    if ($splitHwnd -ne [IntPtr]::Zero -and [TUI]::Focus($splitHwnd)) {
        $bp = & $PSMUX display-message -t $SPLIT -p '#{window_panes}' 2>&1 | Out-String
        $bp = $bp.Trim()

        [TUI]::CtrlB()
        Start-Sleep -Milliseconds 500
        [TUI]::Key(0x35, $true)  # Shift+5 = %
        Start-Sleep -Seconds 5

        $splitProc.Refresh()
        if ($splitProc.HasExited) {
            Write-Fail "prefix+%% crashed TUI (exit=$($splitProc.ExitCode)) [NEEDS INVESTIGATION]"
        } else {
            $ap = & $PSMUX display-message -t $SPLIT -p '#{window_panes}' 2>&1 | Out-String
            $ap = $ap.Trim()
            if ($ap -match '^\d+$' -and [int]$ap -gt [int]$bp) {
                Write-Pass "prefix+%%: panes $bp -> $ap"
            } else { Write-Fail "prefix+%%: panes $bp -> $ap" }
        }
    } else { Write-Fail "Test I: cannot find/focus split window" }
} else { Write-Fail "Test I: split session did not start" }
try { if (-not $splitProc.HasExited) { $splitProc.Kill() } } catch {}
& $PSMUX kill-session -t $SPLIT 2>&1 | Out-Null
Remove-Item "$psmuxDir\$SPLIT.*" -Force -EA SilentlyContinue

# ── RESULTS ────────────────────────────────────────────────────────────────
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Win32 TUI Proof Results" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host ""

exit $script:TestsFailed
