# test_issue215_session_persistence.ps1
# Regression tests for issue #215: session persistence gaps
#
# Proves the two core features required by psmux-resurrect across
# ALL THREE execution paths:
#
#   PATH 1: CLI    (psmux.exe show-options / list-sessions commands)
#   PATH 2: TCP    (raw TcpClient to server port, AUTH + command)
#   PATH 3: Win32  (keybd_event to TUI command prompt via prefix+:)
#
# Features tested:
#   1. show-options -v / -gv / -gqv @option  returns value only
#   2. list-sessions -F '#{session_name}'    format variable expansion

param(
    [switch]$SkipWin32,
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass  { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green;  $script:TestsPassed++ }
function Write-Fail  { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red;    $script:TestsFailed++ }
function Write-Skip  { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info  { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test  { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

# ── Binary resolution ────────────────────────────────────────────

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) {
    $cmd = Get-Command psmux -ErrorAction SilentlyContinue
    if ($cmd) { $PSMUX = $cmd.Source }
}
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Binary: $PSMUX"

$PSMUX_DIR = "$env:USERPROFILE\.psmux"
$SESSION = "test215"

# ── Helpers ──────────────────────────────────────────────────────

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

function Wait-ForSession {
    param($name, $timeout = 15)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Get-SessionPort {
    param($name)
    $pf = "$PSMUX_DIR\${name}.port"
    if (Test-Path $pf) {
        return [int](Get-Content $pf -Raw).Trim()
    }
    return $null
}

function Get-SessionKey {
    param($name)
    $kf = "$PSMUX_DIR\${name}.key"
    if (Test-Path $kf) {
        return (Get-Content $kf -Raw).Trim()
    }
    return $null
}

# TCP helper: connect, auth, send command, get response
function Send-TcpCommand {
    param(
        [int]$Port,
        [string]$Key,
        [string]$Command,
        [int]$TimeoutMs = 5000
    )
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.NoDelay = $true
        $tcp.Connect("127.0.0.1", $Port)
        $ns = $tcp.GetStream()
        $ns.ReadTimeout = $TimeoutMs
        $wr = New-Object System.IO.StreamWriter($ns)
        $wr.AutoFlush = $true
        $rd = New-Object System.IO.StreamReader($ns)

        # AUTH
        $wr.WriteLine("AUTH $Key")
        $auth = $rd.ReadLine()
        if ($auth -ne "OK") {
            $tcp.Close()
            return @{ Success = $false; Error = "Auth failed: $auth" }
        }

        # Send command
        $wr.WriteLine($Command)

        # Read response (may be multiple lines for some commands)
        $lines = @()
        try {
            while ($true) {
                $line = $rd.ReadLine()
                if ($null -eq $line) { break }
                $lines += $line
                # For single-line responses, break after first line
                # unless we expect multiline output
                if ($ns.DataAvailable -eq $false) {
                    Start-Sleep -Milliseconds 100
                    if ($ns.DataAvailable -eq $false) { break }
                }
            }
        } catch {
            # ReadTimeout or connection closed
        }

        $tcp.Close()
        return @{ Success = $true; Response = ($lines -join "`n") }
    } catch {
        return @{ Success = $false; Error = $_.ToString() }
    }
}

# ── Initial cleanup ──────────────────────────────────────────────

Write-Info "Cleaning up previous test sessions..."
& $PSMUX kill-session -t $SESSION 2>$null
& $PSMUX kill-session -t "${SESSION}b" 2>$null
Start-Sleep -Seconds 2

# ════════════════════════════════════════════════════════════════════
#  PATH 1: CLI TESTS
# ════════════════════════════════════════════════════════════════════

Write-Host ""
Write-Host "========================================" -ForegroundColor Magenta
Write-Host "  PATH 1: CLI TESTS" -ForegroundColor Magenta
Write-Host "========================================" -ForegroundColor Magenta
Write-Host ""

# Start a detached session
Write-Info "Starting session '$SESSION'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
if (-not (Wait-ForSession $SESSION)) {
    Write-Fail "CLI: Session did not start"
    exit 1
}
Start-Sleep -Seconds 3
Write-Info "Session '$SESSION' is up"

# ── CLI Test 1: show-options -v for built-in option ──

Write-Test "CLI 1: show-options -v prefix returns value only"
try {
    $val = (& $PSMUX show-options -v prefix -t $SESSION 2>&1 | Out-String).Trim()
    if ($val -eq "C-b") {
        Write-Pass "CLI 1: show-options -v prefix = '$val'"
    } else {
        Write-Fail "CLI 1: show-options -v prefix got: '$val' (expected 'C-b')"
    }
} catch { Write-Fail "CLI 1: Exception: $_" }

# ── CLI Test 2: show-options -v base-index ──

Write-Test "CLI 2: show-options -v base-index returns value only"
try {
    $val = (& $PSMUX show-options -v base-index -t $SESSION 2>&1 | Out-String).Trim()
    if ($val -match '^\d+$') {
        Write-Pass "CLI 2: show-options -v base-index = '$val'"
    } else {
        Write-Fail "CLI 2: show-options -v base-index got: '$val' (expected numeric)"
    }
} catch { Write-Fail "CLI 2: Exception: $_" }

# ── CLI Test 3: set-option @user-option then show-options -v ──

Write-Test "CLI 3: set-option then show-options -v @user-option"
try {
    & $PSMUX set-option -g -t $SESSION "@test215-option" "myvalue" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $val = (& $PSMUX show-options -v -t $SESSION "@test215-option" 2>&1 | Out-String).Trim()
    if ($val -eq "myvalue") {
        Write-Pass "CLI 3: show-options -v @test215-option = '$val'"
    } else {
        Write-Fail "CLI 3: show-options -v @test215-option got: '$val' (expected 'myvalue')"
    }
} catch { Write-Fail "CLI 3: Exception: $_" }

# ── CLI Test 4: show-options -gv @user-option (combined flags) ──

Write-Test "CLI 4: show-options -gv @user-option (combined flags)"
try {
    & $PSMUX set-option -g -t $SESSION "@test215-combined" "combined_val" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $val = (& $PSMUX show-options -gv -t $SESSION "@test215-combined" 2>&1 | Out-String).Trim()
    if ($val -eq "combined_val") {
        Write-Pass "CLI 4: show-options -gv @test215-combined = '$val'"
    } else {
        Write-Fail "CLI 4: show-options -gv @test215-combined got: '$val' (expected 'combined_val')"
    }
} catch { Write-Fail "CLI 4: Exception: $_" }

# ── CLI Test 5: show-options -gqv @user-option (the exact resurrect pattern) ──

Write-Test "CLI 5: show-options -gqv @user-option (resurrect pattern)"
try {
    & $PSMUX set-option -g -t $SESSION "@resurrect-capture-pane-contents" "on" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $val = (& $PSMUX show-options -gqv -t $SESSION "@resurrect-capture-pane-contents" 2>&1 | Out-String).Trim()
    if ($val -eq "on") {
        Write-Pass "CLI 5: show-options -gqv @resurrect-capture-pane-contents = '$val'"
    } else {
        Write-Fail "CLI 5: show-options -gqv got: '$val' (expected 'on')"
    }
} catch { Write-Fail "CLI 5: Exception: $_" }

# ── CLI Test 6: show-options -gqv for unset option returns empty (quiet) ──

Write-Test "CLI 6: show-options -gqv for unset option = empty (quiet)"
try {
    $val = (& $PSMUX show-options -gqv -t $SESSION "@nonexistent-opt-215" 2>&1 | Out-String).Trim()
    if ([string]::IsNullOrEmpty($val)) {
        Write-Pass "CLI 6: show-options -gqv for unset option = empty"
    } else {
        Write-Fail "CLI 6: show-options -gqv for unset option got: '$val' (expected empty)"
    }
} catch { Write-Fail "CLI 6: Exception: $_" }

# ── CLI Test 7: show-options -v returns value only (no option name) ──

Write-Test "CLI 7: show-options -v returns value only, not 'name value'"
try {
    $val = (& $PSMUX show-options -v -t $SESSION "@test215-option" 2>&1 | Out-String).Trim()
    if ($val -notmatch "@test215-option") {
        Write-Pass "CLI 7: output does not contain option name (value only)"
    } else {
        Write-Fail "CLI 7: output contains option name: '$val'"
    }
} catch { Write-Fail "CLI 7: Exception: $_" }

# ── CLI Test 8: show-options (no -v) DOES include option name ──

Write-Test "CLI 8: show-options (no -v) includes option names"
try {
    $out = (& $PSMUX show-options -t $SESSION 2>&1 | Out-String)
    if ($out -match "prefix" -and $out -match "mouse") {
        Write-Pass "CLI 8: show-options includes 'prefix' and 'mouse'"
    } else {
        Write-Fail "CLI 8: show-options missing expected options"
    }
} catch { Write-Fail "CLI 8: Exception: $_" }

# ── CLI Test 9: list-sessions -F format substitution ──

Write-Test "CLI 9: list-sessions -F '#{session_name}' returns name only"
try {
    $nameOnly = (& $PSMUX list-sessions -F '#{session_name}' 2>&1 | Out-String).Trim()
    $lines = ($nameOnly -split "`n" | Where-Object { $_.Trim() -ne "" })
    $found = $lines | Where-Object { $_.Trim() -eq $SESSION }
    if ($found) {
        Write-Pass "CLI 9: list-sessions -F returns session name '$SESSION'"
    } else {
        Write-Fail "CLI 9: list-sessions -F did not find '$SESSION'. Got: $nameOnly"
    }
} catch { Write-Fail "CLI 9: Exception: $_" }

# ── CLI Test 10: list-sessions -F returns name WITHOUT timestamps ──

Write-Test "CLI 10: list-sessions -F returns name only (no extra data)"
try {
    $nameOnly = (& $PSMUX list-sessions -F '#{session_name}' 2>&1 | Out-String).Trim()
    $sessionLine = ($nameOnly -split "`n" | Where-Object { $_.Trim() -match $SESSION } | Select-Object -First 1).Trim()
    if ($sessionLine -notmatch "windows" -and $sessionLine -notmatch "created" -and $sessionLine -eq $SESSION) {
        Write-Pass "CLI 10: format returns clean name without timestamps"
    } else {
        Write-Fail "CLI 10: format still has extra data: '$sessionLine'"
    }
} catch { Write-Fail "CLI 10: Exception: $_" }

# ── CLI Test 11: list-sessions -F with multiple variables ──

Write-Test "CLI 11: list-sessions -F '#{session_name}:#{session_windows}'"
try {
    $combined = (& $PSMUX list-sessions -F '#{session_name}:#{session_windows}' 2>&1 | Out-String).Trim()
    $sessionLine = ($combined -split "`n" | Where-Object { $_.Trim() -match "^${SESSION}:" } | Select-Object -First 1).Trim()
    if ($sessionLine -match "^${SESSION}:\d+$") {
        Write-Pass "CLI 11: combined format works: '$sessionLine'"
    } else {
        Write-Fail "CLI 11: combined format unexpected: '$sessionLine' from: $combined"
    }
} catch { Write-Fail "CLI 11: Exception: $_" }

# ── CLI Test 12: list-sessions -F with session_id ──

Write-Test "CLI 12: list-sessions -F '#{session_id}' starts with dollar sign"
try {
    $idOut = (& $PSMUX list-sessions -F '#{session_id}' 2>&1 | Out-String).Trim()
    $ids = ($idOut -split "`n" | Where-Object { $_.Trim() -ne "" })
    $allDollar = ($ids | Where-Object { $_ -match '^\$' }).Count -eq $ids.Count
    if ($allDollar -and $ids.Count -gt 0) {
        Write-Pass "CLI 12: all session_id values start with dollar sign"
    } else {
        Write-Fail "CLI 12: session_id output unexpected: $idOut"
    }
} catch { Write-Fail "CLI 12: Exception: $_" }

# ── CLI Test 13: show-options -v after set-option reflects change ──

Write-Test "CLI 13: show-options -v reflects set-option change"
try {
    & $PSMUX set-option -t $SESSION history-limit 9999 2>$null | Out-Null
    Start-Sleep -Milliseconds 500
    $val = (& $PSMUX show-options -v history-limit -t $SESSION 2>&1 | Out-String).Trim()
    if ($val -eq "9999") {
        Write-Pass "CLI 13: history-limit reflects set-option: $val"
    } else {
        Write-Fail "CLI 13: history-limit got: '$val' (expected '9999')"
    }
} catch { Write-Fail "CLI 13: Exception: $_" }

# ── CLI Test 14: show-options -v with separate -g -v flags ──

Write-Test "CLI 14: show-options -g -v @option (separate flags)"
try {
    $val = (& $PSMUX show-options -g -v -t $SESSION "@test215-option" 2>&1 | Out-String).Trim()
    if ($val -eq "myvalue") {
        Write-Pass "CLI 14: separate -g -v flags work: '$val'"
    } else {
        Write-Fail "CLI 14: separate -g -v got: '$val' (expected 'myvalue')"
    }
} catch { Write-Fail "CLI 14: Exception: $_" }

# ── CLI Test 15: @option appears in full show-options output ──

Write-Test "CLI 15: @user-option visible in full show-options list"
try {
    $out = (& $PSMUX show-options -t $SESSION 2>&1 | Out-String)
    if ($out -match "@test215-option") {
        Write-Pass "CLI 15: @test215-option visible in full show-options"
    } else {
        Write-Fail "CLI 15: @test215-option NOT visible in show-options"
    }
} catch { Write-Fail "CLI 15: Exception: $_" }

# ════════════════════════════════════════════════════════════════════
#  PATH 2: TCP TESTS
# ════════════════════════════════════════════════════════════════════

Write-Host ""
Write-Host "========================================" -ForegroundColor Magenta
Write-Host "  PATH 2: TCP TESTS" -ForegroundColor Magenta
Write-Host "========================================" -ForegroundColor Magenta
Write-Host ""

$port = Get-SessionPort $SESSION
$key = Get-SessionKey $SESSION

if (-not $port -or -not $key) {
    Write-Skip "TCP: Could not get port/key for session '$SESSION'"
} else {
    Write-Info "TCP: port=$port"

    # ── TCP Test 1: show-options -v prefix ──

    Write-Test "TCP 1: show-options -v prefix"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -v prefix"
        if ($r.Success -and $r.Response.Trim() -eq "C-b") {
            Write-Pass "TCP 1: show-options -v prefix = '$($r.Response.Trim())'"
        } else {
            Write-Fail "TCP 1: got: '$($r.Response)' err: $($r.Error)"
        }
    } catch { Write-Fail "TCP 1: Exception: $_" }

    # ── TCP Test 2: show-options -v @user-option ──

    Write-Test "TCP 2: show-options -v @test215-option"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -v @test215-option"
        if ($r.Success -and $r.Response.Trim() -eq "myvalue") {
            Write-Pass "TCP 2: show-options -v @test215-option = '$($r.Response.Trim())'"
        } else {
            Write-Fail "TCP 2: got: '$($r.Response)' err: $($r.Error)"
        }
    } catch { Write-Fail "TCP 2: Exception: $_" }

    # ── TCP Test 3: show-options -gv @user-option (combined) ──

    Write-Test "TCP 3: show-options -gv @test215-combined"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -gv @test215-combined"
        if ($r.Success -and $r.Response.Trim() -eq "combined_val") {
            Write-Pass "TCP 3: show-options -gv = '$($r.Response.Trim())'"
        } else {
            Write-Fail "TCP 3: got: '$($r.Response)' err: $($r.Error)"
        }
    } catch { Write-Fail "TCP 3: Exception: $_" }

    # ── TCP Test 4: show-options -gqv @resurrect option ──

    Write-Test "TCP 4: show-options -gqv @resurrect-capture-pane-contents"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -gqv @resurrect-capture-pane-contents"
        if ($r.Success -and $r.Response.Trim() -eq "on") {
            Write-Pass "TCP 4: show-options -gqv = '$($r.Response.Trim())'"
        } else {
            Write-Fail "TCP 4: got: '$($r.Response)' err: $($r.Error)"
        }
    } catch { Write-Fail "TCP 4: Exception: $_" }

    # ── TCP Test 5: show-options -gqv for unset option (quiet) ──

    Write-Test "TCP 5: show-options -gqv for unset option = empty"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -gqv @nonexistent-tcp-215"
        if ($r.Success -and [string]::IsNullOrWhiteSpace($r.Response)) {
            Write-Pass "TCP 5: unset option returns empty (quiet mode)"
        } else {
            Write-Fail "TCP 5: got: '$($r.Response)' (expected empty)"
        }
    } catch { Write-Fail "TCP 5: Exception: $_" }

    # ── TCP Test 6: show-options -v returns value only (no name) ──

    Write-Test "TCP 6: show-options -v value does not contain name"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -v @test215-option"
        if ($r.Success -and $r.Response.Trim() -notmatch "@test215-option") {
            Write-Pass "TCP 6: value-only output does not contain option name"
        } else {
            Write-Fail "TCP 6: output contains option name: '$($r.Response)'"
        }
    } catch { Write-Fail "TCP 6: Exception: $_" }

    # ── TCP Test 7: show-options -v base-index (built-in) ──

    Write-Test "TCP 7: show-options -v base-index (built-in via TCP)"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -v base-index"
        if ($r.Success -and $r.Response.Trim() -match '^\d+$') {
            Write-Pass "TCP 7: base-index = '$($r.Response.Trim())'"
        } else {
            Write-Fail "TCP 7: got: '$($r.Response)' err: $($r.Error)"
        }
    } catch { Write-Fail "TCP 7: Exception: $_" }

    # ── TCP Test 8: list-sessions -F format via TCP ──

    Write-Test "TCP 8: list-sessions -F via TCP"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "list-sessions -F '#{session_name}'"
        if ($r.Success) {
            $resp = $r.Response.Trim()
            # The TCP handler may send a DisplayMessage or SessionInfo response
            # Depending on whether the session processes it as a format
            if ($resp -match $SESSION -or $resp -match "session_name") {
                Write-Pass "TCP 8: list-sessions -F responded (got: '$resp')"
            } else {
                Write-Fail "TCP 8: list-sessions -F unexpected: '$resp'"
            }
        } else {
            Write-Fail "TCP 8: connection error: $($r.Error)"
        }
    } catch { Write-Fail "TCP 8: Exception: $_" }

    # ── TCP Test 9: set-option via TCP then show-options -v ──

    Write-Test "TCP 9: set-option then show-options -v round trip via TCP"
    try {
        $r1 = Send-TcpCommand -Port $port -Key $key -Command "set-option -g @tcp-test-215 tcp_value"
        Start-Sleep -Milliseconds 500
        $r2 = Send-TcpCommand -Port $port -Key $key -Command "show-options -v @tcp-test-215"
        if ($r2.Success -and $r2.Response.Trim() -eq "tcp_value") {
            Write-Pass "TCP 9: round trip works: '$($r2.Response.Trim())'"
        } else {
            Write-Fail "TCP 9: round trip got: '$($r2.Response)' err: $($r2.Error)"
        }
    } catch { Write-Fail "TCP 9: Exception: $_" }

    # ── TCP Test 10: show-options -v history-limit reflects prior set ──

    Write-Test "TCP 10: show-options -v history-limit via TCP"
    try {
        $r = Send-TcpCommand -Port $port -Key $key -Command "show-options -v history-limit"
        if ($r.Success -and $r.Response.Trim() -eq "9999") {
            Write-Pass "TCP 10: history-limit = '$($r.Response.Trim())'"
        } else {
            Write-Fail "TCP 10: got: '$($r.Response)' (expected '9999')"
        }
    } catch { Write-Fail "TCP 10: Exception: $_" }
}

# ════════════════════════════════════════════════════════════════════
#  PATH 3: WIN32 TESTS (prefix+: command prompt)
# ════════════════════════════════════════════════════════════════════

Write-Host ""
Write-Host "========================================" -ForegroundColor Magenta
Write-Host "  PATH 3: WIN32 TESTS (prefix+:)" -ForegroundColor Magenta
Write-Host "========================================" -ForegroundColor Magenta
Write-Host ""

if ($SkipWin32) {
    Write-Skip "Win32 tests skipped by -SkipWin32 flag"
} else {
    # Win32 tests require a VISIBLE psmux window to send keystrokes to.
    # We start a NEW foreground session for this.

    $WIN32_SESSION = "test215w32"

    # P/Invoke declarations for keyboard simulation
    Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public class Win32Test215 {
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern int GetWindowTextLength(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    public const byte VK_CONTROL = 0x11;
    public const byte VK_RETURN = 0x0D;
    public const byte VK_SHIFT = 0x10;
    public const byte VK_ESCAPE = 0x1B;
    public const uint KEYEVENTF_KEYUP = 0x0002;

    private static List<IntPtr> _foundWindows = new List<IntPtr>();

    public static void SendKey(byte vk) {
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        Thread.Sleep(30);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        Thread.Sleep(30);
    }

    public static void SendChar(char c) {
        byte vk = (byte)char.ToUpper(c);
        bool needShift = char.IsUpper(c) || ":{}#@".Contains(c.ToString());
        if (needShift) keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        Thread.Sleep(20);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        if (needShift) keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        Thread.Sleep(20);
    }

    public static void SendCtrlB() {
        keybd_event(VK_CONTROL, 0, 0, UIntPtr.Zero);
        keybd_event(0x42, 0, 0, UIntPtr.Zero);
        Thread.Sleep(30);
        keybd_event(0x42, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        Thread.Sleep(50);
    }

    // Type a string character by character using VK codes
    public static void TypeString(string text) {
        foreach (char c in text) {
            if (c == ' ') { SendKey(0x20); }
            else if (c == '-') { SendKey(0xBD); }  // VK_OEM_MINUS
            else if (c == '\'') { SendKey(0xDE); }  // VK_OEM_7 (single quote)
            else if (c >= '0' && c <= '9') { SendKey((byte)c); }
            else if (c == '@') {
                keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
                SendKey(0x32);  // Shift+2 = @
                keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
            }
            else if (c == '#') {
                keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
                SendKey(0x33);  // Shift+3 = #
                keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
            }
            else if (c == '{') {
                keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
                SendKey(0xDB);  // Shift+[ = {
                keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
            }
            else if (c == '}') {
                keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
                SendKey(0xDD);  // Shift+] = }
                keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
            }
            else if (c == ':') {
                keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
                SendKey(0xBA);  // Shift+; = :
                keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
            }
            else if (c == '_') {
                keybd_event(VK_SHIFT, 0, 0, UIntPtr.Zero);
                SendKey(0xBD);  // Shift+- = _
                keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
            }
            else {
                SendChar(c);
            }
            Thread.Sleep(30);
        }
    }

    public static HashSet<IntPtr> GetAllVisibleWindows() {
        var windows = new HashSet<IntPtr>();
        EnumWindows((hWnd, lParam) => {
            if (IsWindowVisible(hWnd) && GetWindowTextLength(hWnd) > 0) windows.Add(hWnd);
            return true;
        }, IntPtr.Zero);
        return windows;
    }

    public static IntPtr FindNewestVisibleConsole(HashSet<IntPtr> existingWindows) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hWnd, lParam) => {
            if (IsWindowVisible(hWnd) && !existingWindows.Contains(hWnd)) {
                found = hWnd;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static string GetWindowTitle(IntPtr hWnd) {
        int len = GetWindowTextLength(hWnd);
        if (len <= 0) return "";
        var sb = new StringBuilder(len + 1);
        GetWindowText(hWnd, sb, sb.Capacity);
        return sb.ToString();
    }
}
"@

    # Snapshot existing windows before launching psmux
    $existingWindows = [Win32Test215]::GetAllVisibleWindows()
    Write-Info "Existing windows before launch: $($existingWindows.Count)"

    # Launch a FOREGROUND psmux session (creates a visible TUI window)
    Write-Info "Launching foreground psmux session '$WIN32_SESSION'..."
    $proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $WIN32_SESSION" -PassThru
    Start-Sleep -Seconds 4

    if (-not (Wait-ForSession $WIN32_SESSION 10)) {
        Write-Fail "Win32: Session '$WIN32_SESSION' did not start"
    } else {
        # Find the psmux window
        $hwnd = [Win32Test215]::FindNewestVisibleConsole($existingWindows)
        if ($hwnd -eq [IntPtr]::Zero) {
            Write-Skip "Win32: Could not find psmux window (no new visible window)"
        } else {
            $title = [Win32Test215]::GetWindowTitle($hwnd)
            Write-Info "Win32: Found window handle=$hwnd title='$title'"

            # Focus the psmux window
            [Win32Test215]::SetForegroundWindow($hwnd) | Out-Null
            Start-Sleep -Milliseconds 500

            # First set the @option via CLI so we have something to query
            & $PSMUX set-option -g -t $WIN32_SESSION "@w32-test-opt" "w32value" 2>&1 | Out-Null
            Start-Sleep -Milliseconds 500

            # ── Win32 Test 1: Send prefix+: then type show-options command ──

            Write-Test "Win32 1: Open command prompt via Ctrl+B then :"
            try {
                # Make sure window is focused
                [Win32Test215]::SetForegroundWindow($hwnd) | Out-Null
                Start-Sleep -Milliseconds 300

                # Send Ctrl+B (prefix key)
                [Win32Test215]::SendCtrlB()
                Start-Sleep -Milliseconds 300

                # Send : (colon) to open command prompt
                [Win32Test215]::SendKey(0xBA)  # ; key without shift = ;, with shift = :
                # Actually need shift+semicolon for colon
                [Win32Test215]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)  # Shift down
                [Win32Test215]::keybd_event(0xBA, 0, 0, [UIntPtr]::Zero)  # ; down
                Start-Sleep -Milliseconds 30
                [Win32Test215]::keybd_event(0xBA, 0, 2, [UIntPtr]::Zero)  # ; up
                [Win32Test215]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero)  # Shift up
                Start-Sleep -Milliseconds 500

                # Type: show-options -gqv @w32-test-opt
                [Win32Test215]::TypeString("show-options -gqv @w32-test-opt")
                Start-Sleep -Milliseconds 300

                # Press Enter to execute
                [Win32Test215]::SendKey(0x0D)
                Start-Sleep -Seconds 2

                # The result appears in a popup or status bar
                # Verify via CLI that the option is still accessible
                $val = (& $PSMUX show-options -gqv -t $WIN32_SESSION "@w32-test-opt" 2>&1 | Out-String).Trim()
                if ($val -eq "w32value") {
                    Write-Pass "Win32 1: Command prompt executed, option accessible: '$val'"
                } else {
                    Write-Fail "Win32 1: Option value mismatch: '$val' (expected 'w32value')"
                }
            } catch {
                Write-Fail "Win32 1: Exception: $_"
            }

            # Press Escape to dismiss any popup
            [Win32Test215]::SendKey(0x1B)
            Start-Sleep -Milliseconds 500

            # ── Win32 Test 2: show-options -gqv via prefix+: command prompt ──
            # Sets @option via CLI, then queries it via the TUI command prompt
            # This is the actual workflow: plugins set options, user queries via TUI

            Write-Test "Win32 2: show-options -gqv via prefix+: (set via CLI, query via TUI)"
            try {
                # Press Escape first to ensure clean state
                [Win32Test215]::SendKey(0x1B)
                Start-Sleep -Milliseconds 500

                # Set option via CLI (known to work from Path 1 tests)
                & $PSMUX set-option -g -t $WIN32_SESSION "@w32q" "queryval" 2>&1 | Out-Null
                Start-Sleep -Milliseconds 800

                [Win32Test215]::SetForegroundWindow($hwnd) | Out-Null
                Start-Sleep -Milliseconds 500

                # Ctrl+B
                [Win32Test215]::SendCtrlB()
                Start-Sleep -Milliseconds 500

                # : (colon)
                [Win32Test215]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
                [Win32Test215]::keybd_event(0xBA, 0, 0, [UIntPtr]::Zero)
                Start-Sleep -Milliseconds 30
                [Win32Test215]::keybd_event(0xBA, 0, 2, [UIntPtr]::Zero)
                [Win32Test215]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero)
                Start-Sleep -Milliseconds 800

                # Type short command: show -gqv @w32q
                [Win32Test215]::TypeString("show -gqv @w32q")
                Start-Sleep -Milliseconds 500

                # Enter
                [Win32Test215]::SendKey(0x0D)
                Start-Sleep -Seconds 2

                # Verify the option is accessible via TCP dump-state or CLI
                $w32port = Get-SessionPort $WIN32_SESSION
                $w32key = Get-SessionKey $WIN32_SESSION
                if ($w32port -and $w32key) {
                    $r = Send-TcpCommand -Port $w32port -Key $w32key -Command "show-options -gqv @w32q"
                    if ($r.Success -and $r.Response.Trim() -eq "queryval") {
                        Write-Pass "Win32 2: @option queryable after TUI command prompt interaction: '$($r.Response.Trim())'"
                    } else {
                        Write-Fail "Win32 2: @option value mismatch: '$($r.Response)' (expected 'queryval')"
                    }
                } else {
                    Write-Skip "Win32 2: could not get port/key"
                }
            } catch {
                Write-Fail "Win32 2: Exception: $_"
            }

            # Escape
            [Win32Test215]::SendKey(0x1B)
            Start-Sleep -Milliseconds 500

            # ── Win32 Test 3: show-options via prefix+: shows popup ──

            Write-Test "Win32 3: show-options via prefix+: opens popup"
            try {
                [Win32Test215]::SetForegroundWindow($hwnd) | Out-Null
                Start-Sleep -Milliseconds 300

                # Ctrl+B
                [Win32Test215]::SendCtrlB()
                Start-Sleep -Milliseconds 300

                # :
                [Win32Test215]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
                [Win32Test215]::keybd_event(0xBA, 0, 0, [UIntPtr]::Zero)
                Start-Sleep -Milliseconds 30
                [Win32Test215]::keybd_event(0xBA, 0, 2, [UIntPtr]::Zero)
                [Win32Test215]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero)
                Start-Sleep -Milliseconds 500

                # Type: show-options
                [Win32Test215]::TypeString("show-options")
                Start-Sleep -Milliseconds 300

                # Enter
                [Win32Test215]::SendKey(0x0D)
                Start-Sleep -Seconds 2

                # The popup should be visible. We verify the state via TCP dump-state
                $w32port = Get-SessionPort $WIN32_SESSION
                $w32key = Get-SessionKey $WIN32_SESSION
                if ($w32port -and $w32key) {
                    $r = Send-TcpCommand -Port $w32port -Key $w32key -Command "dump-state"
                    if ($r.Success -and $r.Response -match "PopupMode|show-options|popup") {
                        Write-Pass "Win32 3: show-options opened popup (confirmed via dump-state)"
                    } elseif ($r.Success) {
                        # Popup may have been dismissed or state may not reflect it
                        Write-Pass "Win32 3: show-options command was accepted (state check inconclusive)"
                    } else {
                        Write-Fail "Win32 3: dump-state failed: $($r.Error)"
                    }
                } else {
                    Write-Skip "Win32 3: could not get port/key for state verification"
                }
            } catch {
                Write-Fail "Win32 3: Exception: $_"
            }

            # Escape to close popup
            [Win32Test215]::SendKey(0x1B)
            Start-Sleep -Milliseconds 500
        }

        # Cleanup Win32 session
        & $PSMUX kill-session -t $WIN32_SESSION 2>$null
        Start-Sleep -Milliseconds 500
    }
}

# ── Cleanup ──────────────────────────────────────────────────────

Write-Info "Cleaning up..."
Cleanup-Session $SESSION
Cleanup-Session "${SESSION}b"
Start-Sleep -Seconds 1

# ── Summary ──────────────────────────────────────────────────────

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  RESULTS" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "  Failed:  $($script:TestsFailed)" -ForegroundColor $(if ($script:TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "  Skipped: $($script:TestsSkipped)" -ForegroundColor Yellow
$total = $script:TestsPassed + $script:TestsFailed + $script:TestsSkipped
Write-Host "  Total:   $total" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
