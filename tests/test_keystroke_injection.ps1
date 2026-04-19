# test_keystroke_injection.ps1
# Experiment: WriteConsoleInput-based keystroke injection into psmux
#
# keybd_event injects into the hardware input queue (foreground window).
# Console apps read from the console input buffer via ReadConsoleInput.
# WriteConsoleInput writes directly to that buffer - the correct API.
# Requires AttachConsole(pid) from a SEPARATE PROCESS.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$SESSION = "kbi_test"
$psmuxDir = "$env:USERPROFILE\.psmux"
$TEMP = [System.IO.Path]::GetTempPath()
$script:Pass = 0
$script:Fail = 0

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:Pass++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:Fail++ }

# ============================================================
# STEP 1: Compile the C# injector exe
# ============================================================
Write-Host "`n=== Step 1: Compile WriteConsoleInput injector ===" -ForegroundColor Cyan

$csharpSource = @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

class ConsoleKeyInjector
{
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FreeConsole();

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(uint dwProcessId);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int nStdHandle);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool WriteConsoleInput(
        IntPtr hConsoleInput,
        INPUT_RECORD[] lpBuffer,
        uint nLength,
        out uint lpNumberOfEventsWritten);

    [DllImport("user32.dll")]
    static extern uint MapVirtualKeyW(uint uCode, uint uMapType);

    const int STD_INPUT_HANDLE = -10;
    const ushort KEY_EVENT = 0x0001;
    const uint LEFT_CTRL_PRESSED = 0x0008;
    const uint SHIFT_PRESSED = 0x0010;

    [StructLayout(LayoutKind.Sequential)]
    struct KEY_EVENT_RECORD
    {
        public int bKeyDown;
        public ushort wRepeatCount;
        public ushort wVirtualKeyCode;
        public ushort wVirtualScanCode;
        public char UnicodeChar;
        public uint dwControlKeyState;
    }

    [StructLayout(LayoutKind.Explicit)]
    struct INPUT_RECORD
    {
        [FieldOffset(0)] public ushort EventType;
        [FieldOffset(4)] public KEY_EVENT_RECORD KeyEvent;
    }

    static INPUT_RECORD MakeKey(bool down, ushort vk, char ch, uint ctrl)
    {
        var r = new INPUT_RECORD();
        r.EventType = KEY_EVENT;
        r.KeyEvent.bKeyDown = down ? 1 : 0;
        r.KeyEvent.wRepeatCount = 1;
        r.KeyEvent.wVirtualKeyCode = vk;
        r.KeyEvent.wVirtualScanCode = (ushort)MapVirtualKeyW(vk, 0);
        r.KeyEvent.UnicodeChar = ch;
        r.KeyEvent.dwControlKeyState = ctrl;
        return r;
    }

    static bool SendKey(IntPtr h, ushort vk, char ch, uint ctrl, List<string> log)
    {
        var recs = new INPUT_RECORD[] {
            MakeKey(true, vk, ch, ctrl),
            MakeKey(false, vk, ch, ctrl)
        };
        uint written;
        bool ok = WriteConsoleInput(h, recs, 2, out written);
        log.Add(string.Format("  Key '{0}' VK=0x{1:X2} ctrl=0x{2:X8} -> ok={3} written={4}",
            ch == '\0' ? "\\0" : ch.ToString(), vk, ctrl, ok, written));
        return ok && written == 2;
    }

    static bool SendCtrlCombo(IntPtr h, char letter, List<string> log)
    {
        ushort vk = (ushort)char.ToUpper(letter);
        char ctrlChar = (char)(char.ToUpper(letter) - 'A' + 1);

        var recs = new INPUT_RECORD[] {
            MakeKey(true,  0x11, '\0',     LEFT_CTRL_PRESSED),  // Ctrl down
            MakeKey(true,  vk,   ctrlChar, LEFT_CTRL_PRESSED),  // key down
            MakeKey(false, vk,   ctrlChar, LEFT_CTRL_PRESSED),  // key up
            MakeKey(false, 0x11, '\0',     0)                   // Ctrl up
        };
        uint written;
        bool ok = WriteConsoleInput(h, recs, 4, out written);
        log.Add(string.Format("  Ctrl+{0} (char=0x{1:X2}) -> ok={2} written={3}",
            letter, (int)ctrlChar, ok, written));
        return ok && written == 4;
    }

    static int Main(string[] args)
    {
        var log = new List<string>();
        string logFile = Path.Combine(Path.GetTempPath(), "psmux_inject.log");

        if (args.Length < 2)
        {
            File.WriteAllText(logFile, "Usage: injector.exe <pid> <keys>\nKeys: chars, ^x=Ctrl+x, {ENTER}, {ESC}, {SLEEP:ms}");
            return 99;
        }

        uint pid;
        if (!uint.TryParse(args[0], out pid))
        {
            File.WriteAllText(logFile, "Invalid PID: " + args[0]);
            return 98;
        }

        // Join remaining args as the key spec (allows spaces)
        string keys = string.Join(" ", args, 1, args.Length - 1);
        log.Add("PID: " + pid);
        log.Add("Keys: " + keys);

        // Detach from parent console
        bool freed = FreeConsole();
        log.Add("FreeConsole: " + freed + " (err=" + (freed ? "none" : Marshal.GetLastWin32Error().ToString()) + ")");

        // Attach to target console
        bool attached = AttachConsole(pid);
        int attachErr = attached ? 0 : Marshal.GetLastWin32Error();
        log.Add("AttachConsole(" + pid + "): " + attached + " (err=" + (attached ? "none" : attachErr.ToString()) + ")");

        if (!attached)
        {
            File.WriteAllText(logFile, string.Join("\n", log));
            return 2;
        }

        // Get console input handle
        IntPtr handle = GetStdHandle(STD_INPUT_HANDLE);
        log.Add("Handle: " + handle);

        if (handle == IntPtr.Zero || handle == new IntPtr(-1))
        {
            log.Add("FAILED: Invalid console input handle");
            File.WriteAllText(logFile, string.Join("\n", log));
            FreeConsole();
            return 3;
        }

        // Parse and inject keys
        int injected = 0;
        int i = 0;
        while (i < keys.Length)
        {
            if (keys[i] == '^' && i + 1 < keys.Length)
            {
                // Ctrl+letter
                char c = keys[i + 1];
                SendCtrlCombo(handle, c, log);
                i += 2;
                injected++;
                Thread.Sleep(50);
            }
            else if (keys[i] == '{')
            {
                int end = keys.IndexOf('}', i);
                if (end > i)
                {
                    string token = keys.Substring(i + 1, end - i - 1);
                    if (token == "ENTER")
                    {
                        SendKey(handle, 0x0D, '\r', 0, log);
                        injected++;
                    }
                    else if (token == "ESC" || token == "ESCAPE")
                    {
                        SendKey(handle, 0x1B, (char)0x1B, 0, log);
                        injected++;
                    }
                    else if (token.StartsWith("SLEEP:"))
                    {
                        int ms = int.Parse(token.Substring(6));
                        Thread.Sleep(ms);
                        log.Add("  SLEEP " + ms + "ms");
                    }
                    i = end + 1;
                    Thread.Sleep(30);
                }
                else { i++; }
            }
            else
            {
                char c = keys[i];
                ushort vk;
                uint ctrl = 0;

                if (c >= 'a' && c <= 'z') vk = (ushort)(0x41 + c - 'a');
                else if (c >= 'A' && c <= 'Z') { vk = (ushort)(0x41 + c - 'A'); ctrl = SHIFT_PRESSED; }
                else if (c >= '0' && c <= '9') vk = (ushort)(0x30 + c - '0');
                else if (c == ' ') vk = 0x20;
                else if (c == '-') vk = 0xBD;
                else if (c == '=') vk = 0xBB;
                else if (c == '.') vk = 0xBE;
                else if (c == ',') vk = 0xBC;
                else if (c == '/') vk = 0xBF;
                else if (c == '_') { vk = 0xBD; ctrl = SHIFT_PRESSED; }
                else if (c == ':') { vk = 0xBA; ctrl = SHIFT_PRESSED; }
                else if (c == '"') { vk = 0xDE; ctrl = SHIFT_PRESSED; }
                else if (c == '[') vk = 0xDB;
                else if (c == ']') vk = 0xDD;
                else if (c == '\\') vk = 0xDC;
                else if (c == ';') vk = 0xBA;
                else if (c == '\'') vk = 0xDE;
                else vk = (ushort)c;

                SendKey(handle, vk, c, ctrl, log);
                i++;
                injected++;
                Thread.Sleep(30);
            }
        }

        log.Add("Total injected: " + injected);

        FreeConsole();
        File.WriteAllText(logFile, string.Join("\n", log));
        return 0;
    }
}
'@

$csFile = Join-Path $TEMP "psmux_injector.cs"
$exeFile = Join-Path $TEMP "psmux_injector.exe"
$logFile = Join-Path $TEMP "psmux_inject.log"

# Find csc.exe
$cscPath = Join-Path ([Runtime.InteropServices.RuntimeEnvironment]::GetRuntimeDirectory()) "csc.exe"
if (-not (Test-Path $cscPath)) {
    # Fallback: search .NET Framework directories
    $cscPath = Get-ChildItem "C:\Windows\Microsoft.NET\Framework64\v4*\csc.exe" -EA SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $cscPath -or -not (Test-Path $cscPath)) {
    Write-Host "FATAL: Cannot find csc.exe" -ForegroundColor Red
    exit 1
}

Write-Host "  Using csc: $cscPath"
$csharpSource | Set-Content -Path $csFile -Encoding UTF8

$compileOut = & $cscPath /nologo /optimize /out:$exeFile $csFile 2>&1
if (-not (Test-Path $exeFile)) {
    Write-Host "FATAL: Compilation failed:" -ForegroundColor Red
    $compileOut | ForEach-Object { Write-Host "  $_" }
    exit 1
}
Write-Host "  Compiled: $exeFile" -ForegroundColor Green


# ============================================================
# STEP 2: Launch psmux attached session
# ============================================================
Write-Host "`n=== Step 2: Launch psmux attached session ===" -ForegroundColor Cyan

# Cleanup any stale session
& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue

# Launch attached (visible window)
$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$SESSION -PassThru
Write-Host "  Launched PID: $($proc.Id)"

# Wait for session ready
$ready = $false
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 250
    if (Test-Path "$psmuxDir\$SESSION.port") {
        $port = (Get-Content "$psmuxDir\$SESSION.port" -Raw).Trim()
        try {
            $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
            $tcp.Close()
            $ready = $true
            break
        } catch {}
    }
}

if (-not $ready) {
    Write-Host "FATAL: Session never became ready" -ForegroundColor Red
    Stop-Process -Id $proc.Id -Force -EA SilentlyContinue
    exit 1
}
Write-Host "  Session ready (port $port)" -ForegroundColor Green

# Wait for shell prompt
Start-Sleep -Seconds 3
Write-Host "  Shell should be initialized"


# ============================================================
# STEP 3: Test Approach A - WriteConsoleInput (character injection)
# ============================================================
Write-Host "`n=== Test A: Character injection via WriteConsoleInput ===" -ForegroundColor Cyan

# Clear pane first
& $PSMUX send-keys -t $SESSION "clear" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1

# Inject "echo WCI_MARKER" + Enter via the compiled exe
Write-Host "  Injecting: echo WCI_MARKER + Enter"
$injectProc = Start-Process -FilePath $exeFile `
    -ArgumentList "$($proc.Id)", "echo WCI_MARKER{ENTER}" `
    -Wait -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $TEMP "inject_stdout.txt") `
    -RedirectStandardError (Join-Path $TEMP "inject_stderr.txt")

$exitCode = $injectProc.ExitCode
Write-Host "  Injector exit code: $exitCode"

# Show log
if (Test-Path $logFile) {
    Write-Host "  --- Injector log ---" -ForegroundColor DarkGray
    Get-Content $logFile | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
    Write-Host "  --- end log ---" -ForegroundColor DarkGray
}

if ($exitCode -ne 0) {
    Write-Fail "Injector returned exit code $exitCode"
    # Show stderr if any
    $stderr = Join-Path $TEMP "inject_stderr.txt"
    if ((Test-Path $stderr) -and (Get-Item $stderr).Length -gt 0) {
        Write-Host "  stderr:" -ForegroundColor Red
        Get-Content $stderr | ForEach-Object { Write-Host "    $_" -ForegroundColor Red }
    }
} else {
    # Wait for the command to execute and check capture-pane
    Start-Sleep -Seconds 2
    $captured = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($captured -match "WCI_MARKER") {
        Write-Pass "Character injection WORKS - 'WCI_MARKER' found in pane"
    } else {
        Write-Fail "Character injection FAILED - 'WCI_MARKER' not found in pane"
        Write-Host "  Captured pane content:" -ForegroundColor DarkGray
        $captured.Split("`n") | Select-Object -First 15 | ForEach-Object {
            Write-Host "    |$_|" -ForegroundColor DarkGray
        }
    }
}


# ============================================================
# STEP 4: Test Approach A - Ctrl+B prefix key
# ============================================================
Write-Host "`n=== Test B: Ctrl+B prefix via WriteConsoleInput ===" -ForegroundColor Cyan

# First, split the window so there's something to zoom
& $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 2

# Check initial zoom state
$zoomBefore = (& $PSMUX display-message -t $SESSION -p '#{window_zoomed_flag}' 2>&1).Trim()
Write-Host "  Zoom flag before: $zoomBefore"

# Inject Ctrl+B (prefix) then z (zoom toggle)
Write-Host "  Injecting: Ctrl+B then z"
$injectProc = Start-Process -FilePath $exeFile `
    -ArgumentList "$($proc.Id)", "^b{SLEEP:300}z" `
    -Wait -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $TEMP "inject_stdout2.txt") `
    -RedirectStandardError (Join-Path $TEMP "inject_stderr2.txt")

$exitCode = $injectProc.ExitCode
Write-Host "  Injector exit code: $exitCode"

if (Test-Path $logFile) {
    Write-Host "  --- Injector log ---" -ForegroundColor DarkGray
    Get-Content $logFile | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
    Write-Host "  --- end log ---" -ForegroundColor DarkGray
}

if ($exitCode -ne 0) {
    Write-Fail "Prefix key injector returned exit code $exitCode"
} else {
    Start-Sleep -Seconds 1
    $zoomAfter = (& $PSMUX display-message -t $SESSION -p '#{window_zoomed_flag}' 2>&1).Trim()
    Write-Host "  Zoom flag after: $zoomAfter"

    if ($zoomBefore -eq "0" -and $zoomAfter -eq "1") {
        Write-Pass "Prefix + zoom WORKS - zoom toggled 0 -> 1"
    } elseif ($zoomBefore -ne $zoomAfter) {
        Write-Pass "Prefix + zoom WORKS - zoom changed ($zoomBefore -> $zoomAfter)"
    } else {
        Write-Fail "Prefix + zoom FAILED - zoom unchanged ($zoomBefore -> $zoomAfter)"
    }
}


# ============================================================
# STEP 5: Test command prompt (prefix + : + type command)
# ============================================================
Write-Host "`n=== Test C: Command prompt via WriteConsoleInput ===" -ForegroundColor Cyan

# Unzoom first
& $PSMUX resize-pane -Z -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

# Get window count before
$winsBefore = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1).Trim()
Write-Host "  Windows before: $winsBefore"

# Inject: prefix + : + "new-window" + Enter
Write-Host "  Injecting: Ctrl+B, :, 'new-window', Enter"
$injectProc = Start-Process -FilePath $exeFile `
    -ArgumentList "$($proc.Id)", "^b{SLEEP:300}:{SLEEP:500}new-window{ENTER}" `
    -Wait -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $TEMP "inject_stdout3.txt") `
    -RedirectStandardError (Join-Path $TEMP "inject_stderr3.txt")

$exitCode = $injectProc.ExitCode
Write-Host "  Injector exit code: $exitCode"

if (Test-Path $logFile) {
    Write-Host "  --- Injector log ---" -ForegroundColor DarkGray
    Get-Content $logFile | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
    Write-Host "  --- end log ---" -ForegroundColor DarkGray
}

if ($exitCode -ne 0) {
    Write-Fail "Command prompt injector returned exit code $exitCode"
} else {
    Start-Sleep -Seconds 3
    $winsAfter = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1).Trim()
    Write-Host "  Windows after: $winsAfter"

    if ([int]$winsAfter -gt [int]$winsBefore) {
        Write-Pass "Command prompt WORKS - window count $winsBefore -> $winsAfter"
    } else {
        Write-Fail "Command prompt FAILED - window count unchanged ($winsBefore -> $winsAfter)"
    }
}


# ============================================================
# STEP 6: Test copy mode (prefix + [ + navigation)
# ============================================================
Write-Host "`n=== Test D: Copy mode via WriteConsoleInput ===" -ForegroundColor Cyan

# Put text in pane
& $PSMUX send-keys -t $SESSION "echo COPY_TARGET_123" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 1

# Inject: prefix + [ (enter copy mode)
Write-Host "  Injecting: Ctrl+B, ["
$injectProc = Start-Process -FilePath $exeFile `
    -ArgumentList "$($proc.Id)", "^b{SLEEP:300}[" `
    -Wait -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $TEMP "inject_stdout4.txt") `
    -RedirectStandardError (Join-Path $TEMP "inject_stderr4.txt")

$exitCode = $injectProc.ExitCode
Write-Host "  Injector exit code: $exitCode"

if ($exitCode -ne 0) {
    Write-Fail "Copy mode injector returned exit code $exitCode"
} else {
    Start-Sleep -Seconds 1
    # Check if copy mode is active via dump-state
    $port = (Get-Content "$psmuxDir\$SESSION.port" -Raw).Trim()
    $key = (Get-Content "$psmuxDir\$SESSION.key" -Raw).Trim()
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
        $tcp.NoDelay = $true; $tcp.ReceiveTimeout = 5000
        $stream = $tcp.GetStream()
        $writer = [System.IO.StreamWriter]::new($stream)
        $reader = [System.IO.StreamReader]::new($stream)
        $writer.Write("AUTH $key`n"); $writer.Flush()
        $null = $reader.ReadLine()
        $writer.Write("dump-state`n"); $writer.Flush()
        $best = $null
        $tcp.ReceiveTimeout = 3000
        for ($j = 0; $j -lt 50; $j++) {
            try { $line = $reader.ReadLine() } catch { break }
            if ($null -eq $line) { break }
            if ($line -ne "NC" -and $line.Length -gt 100) { $best = $line }
            if ($best) { $tcp.ReceiveTimeout = 50 }
        }
        $tcp.Close()

        if ($best) {
            $json = $best | ConvertFrom-Json
            $mode = $json.mode
            Write-Host "  Current mode: $mode"
            if ($mode -match "copy|CopyMode") {
                Write-Pass "Copy mode entered via keystroke injection"
            } else {
                Write-Fail "Expected CopyMode, got: $mode"
            }
        } else {
            Write-Fail "Could not get dump-state"
        }
    } catch {
        Write-Fail "TCP error: $_"
    }

    # Exit copy mode with q
    $injectProc = Start-Process -FilePath $exeFile `
        -ArgumentList "$($proc.Id)", "q" `
        -Wait -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $TEMP "inject_stdout4b.txt") `
        -RedirectStandardError (Join-Path $TEMP "inject_stderr4b.txt")
    Start-Sleep -Milliseconds 500
}


# ============================================================
# STEP 7: Test keybinding (prefix + c = new window)
# ============================================================
Write-Host "`n=== Test E: Keybinding prefix+c via WriteConsoleInput ===" -ForegroundColor Cyan

$winsBefore2 = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1).Trim()
Write-Host "  Windows before: $winsBefore2"

# Inject: prefix + c (default binding for new-window)
Write-Host "  Injecting: Ctrl+B, c"
$injectProc = Start-Process -FilePath $exeFile `
    -ArgumentList "$($proc.Id)", "^b{SLEEP:300}c" `
    -Wait -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $TEMP "inject_stdout5.txt") `
    -RedirectStandardError (Join-Path $TEMP "inject_stderr5.txt")

$exitCode = $injectProc.ExitCode

if ($exitCode -ne 0) {
    Write-Fail "Keybinding injector returned exit code $exitCode"
} else {
    Start-Sleep -Seconds 3
    $winsAfter2 = (& $PSMUX display-message -t $SESSION -p '#{session_windows}' 2>&1).Trim()
    Write-Host "  Windows after: $winsAfter2"

    if ([int]$winsAfter2 -gt [int]$winsBefore2) {
        Write-Pass "Keybinding prefix+c WORKS - window count $winsBefore2 -> $winsAfter2"
    } else {
        Write-Fail "Keybinding prefix+c FAILED - window count unchanged"
    }
}


# ============================================================
# CLEANUP
# ============================================================
Write-Host "`n=== Cleanup ===" -ForegroundColor Cyan
& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue

# Clean temp files
Remove-Item (Join-Path $TEMP "inject_std*.txt") -Force -EA SilentlyContinue


# ============================================================
# RESULTS
# ============================================================
Write-Host "`n" -NoNewline
Write-Host ("=" * 60)
Write-Host "KEYSTROKE INJECTION EXPERIMENT RESULTS"
Write-Host ("=" * 60)
Write-Host "  Passed: $($script:Pass)" -ForegroundColor Green
Write-Host "  Failed: $($script:Fail)" -ForegroundColor $(if ($script:Fail -gt 0) { "Red" } else { "Green" })
Write-Host ""

if ($script:Pass -gt 0 -and $script:Fail -eq 0) {
    Write-Host "  WriteConsoleInput injection is FULLY WORKING!" -ForegroundColor Green
    Write-Host "  This can replace keybd_event in tui_helper.ps1" -ForegroundColor Green
} elseif ($script:Pass -gt 0) {
    Write-Host "  WriteConsoleInput PARTIALLY works" -ForegroundColor Yellow
    Write-Host "  Character injection works but some key combos may need tuning" -ForegroundColor Yellow
} else {
    Write-Host "  WriteConsoleInput approach FAILED" -ForegroundColor Red
    Write-Host "  Console attachment may not work under current terminal host" -ForegroundColor Red
}

exit $script:Fail
