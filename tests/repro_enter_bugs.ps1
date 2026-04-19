## Script to reproduce the four Shift+Enter bugs using SendInput injection.
## Launches enter_diag in the specified terminal, injects physical keypresses,
## then reads the log file to see raw crossterm events.

param(
    [ValidateSet("wt","wezterm")]
    [string]$Terminal = "wt"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;

public class NativeInput {
    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT {
        public uint type_;
        public KEYBDINPUT ki;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct KEYBDINPUT {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public IntPtr dwExtraInfo;
        public uint padding1;
        public uint padding2;
    }

    public const uint INPUT_KEYBOARD = 1;
    public const uint KEYEVENTF_KEYUP = 0x0002;
    public const ushort VK_SHIFT = 0x10;
    public const ushort VK_CONTROL = 0x11;
    public const ushort VK_MENU = 0x12;
    public const ushort VK_RETURN = 0x0D;

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    public static void SendShiftEnter() {
        INPUT[] inputs = new INPUT[4];
        int size = Marshal.SizeOf(typeof(INPUT));
        inputs[0].type_ = INPUT_KEYBOARD; inputs[0].ki.wVk = VK_SHIFT;
        inputs[1].type_ = INPUT_KEYBOARD; inputs[1].ki.wVk = VK_RETURN;
        inputs[2].type_ = INPUT_KEYBOARD; inputs[2].ki.wVk = VK_RETURN; inputs[2].ki.dwFlags = KEYEVENTF_KEYUP;
        inputs[3].type_ = INPUT_KEYBOARD; inputs[3].ki.wVk = VK_SHIFT; inputs[3].ki.dwFlags = KEYEVENTF_KEYUP;
        SendInput(4, inputs, size);
    }

    public static void SendPlainEnter() {
        INPUT[] inputs = new INPUT[2];
        int size = Marshal.SizeOf(typeof(INPUT));
        inputs[0].type_ = INPUT_KEYBOARD; inputs[0].ki.wVk = VK_RETURN;
        inputs[1].type_ = INPUT_KEYBOARD; inputs[1].ki.wVk = VK_RETURN; inputs[1].ki.dwFlags = KEYEVENTF_KEYUP;
        SendInput(2, inputs, size);
    }

    public static void SendCtrlC() {
        INPUT[] inputs = new INPUT[4];
        int size = Marshal.SizeOf(typeof(INPUT));
        inputs[0].type_ = INPUT_KEYBOARD; inputs[0].ki.wVk = VK_CONTROL;
        inputs[1].type_ = INPUT_KEYBOARD; inputs[1].ki.wVk = 0x43;
        inputs[2].type_ = INPUT_KEYBOARD; inputs[2].ki.wVk = 0x43; inputs[2].ki.dwFlags = KEYEVENTF_KEYUP;
        inputs[3].type_ = INPUT_KEYBOARD; inputs[3].ki.wVk = VK_CONTROL; inputs[3].ki.dwFlags = KEYEVENTF_KEYUP;
        SendInput(4, inputs, size);
    }
}
"@

$diagExe = "C:\Users\uniqu\Documents\workspace\psmux\target\release\examples\enter_diag.exe"
$logFile = "$env:USERPROFILE\.psmux\enter_diag_raw.log"

# Remove old log
if (Test-Path $logFile) { Remove-Item $logFile -Force }

Write-Host "=== Launching enter_diag in $Terminal ===" -ForegroundColor Cyan

if ($Terminal -eq "wt") {
    Start-Process wt -ArgumentList "--title", "EnterDiag", "--", $diagExe
} else {
    Start-Process wezterm -ArgumentList "start", "--", $diagExe
}

Write-Host "Waiting 4 seconds for terminal to start..." -ForegroundColor Yellow
Start-Sleep -Seconds 4

Write-Host "Sending 3x Shift+Enter..." -ForegroundColor Green
for ($i = 0; $i -lt 3; $i++) {
    [NativeInput]::SendShiftEnter()
    Start-Sleep -Milliseconds 500
}

Write-Host "Sending 1x plain Enter..." -ForegroundColor Green
[NativeInput]::SendPlainEnter()
Start-Sleep -Milliseconds 500

Write-Host "Sending Ctrl+C to exit..." -ForegroundColor Green
[NativeInput]::SendCtrlC()
Start-Sleep -Seconds 2

Write-Host ""
Write-Host "=== Raw crossterm events from $Terminal ===" -ForegroundColor Cyan
if (Test-Path $logFile) {
    Get-Content $logFile
} else {
    Write-Host "ERROR: Log file not found at $logFile" -ForegroundColor Red
    Write-Host "The terminal may still be running. Check manually." -ForegroundColor Yellow
}
