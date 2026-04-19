using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;

// Batch keystroke injector for issue #237.
// Sends ALL characters in a SINGLE WriteConsoleInput call so they arrive
// in the same console input buffer read cycle. This triggers psmux's
// stage2 paste heuristic (>=3 chars in <20ms).
//
// Usage: injector_batch.exe <pid> <chars>
// Example: injector_batch.exe 1234 ABCDEFGHIJ
//
// Unlike injector.cs, there is NO Thread.Sleep between characters.
// All KEY_EVENT records are batched into one WriteConsoleInput call.

class BatchInjector
{
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FreeConsole();

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(uint pid);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFileW(string name, uint access, uint share,
        IntPtr sec, uint disp, uint flags, IntPtr tmpl);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool WriteConsoleInput(IntPtr h, INPUT_RECORD[] buf, uint len, out uint written);

    [DllImport("user32.dll")]
    static extern uint MapVirtualKeyW(uint code, uint mapType);

    const ushort KEY_EVENT = 1;
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

    static int Main(string[] args)
    {
        var log = new List<string>();
        string logFile = Path.Combine(Path.GetTempPath(), "psmux_batch_inject.log");

        if (args.Length < 2)
        {
            File.WriteAllText(logFile, "Usage: injector_batch.exe <pid> <chars>\n");
            return 99;
        }

        uint pid;
        if (!uint.TryParse(args[0], out pid))
        {
            File.WriteAllText(logFile, "Invalid PID: " + args[0]);
            return 98;
        }

        string chars = string.Join(" ", args, 1, args.Length - 1);
        log.Add("PID=" + pid + " Chars=" + chars + " Count=" + chars.Length);

        FreeConsole();
        if (!AttachConsole(pid))
        {
            log.Add("AttachConsole FAILED err=" + Marshal.GetLastWin32Error());
            File.WriteAllText(logFile, string.Join("\n", log));
            return 2;
        }

        IntPtr handle = CreateFileW("CONIN$", 0xC0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        if (handle == new IntPtr(-1))
        {
            log.Add("CreateFile(CONIN$) FAILED err=" + Marshal.GetLastWin32Error());
            FreeConsole();
            File.WriteAllText(logFile, string.Join("\n", log));
            return 3;
        }
        log.Add("Handle=" + handle);

        // Build ALL key events in one array (press+release for each char)
        var records = new List<INPUT_RECORD>();
        foreach (char c in chars)
        {
            ushort vk;
            uint ctrl = 0;

            if (c >= 'a' && c <= 'z') vk = (ushort)(0x41 + c - 'a');
            else if (c >= 'A' && c <= 'Z') { vk = (ushort)(0x41 + c - 'A'); ctrl = SHIFT_PRESSED; }
            else if (c >= '0' && c <= '9') vk = (ushort)(0x30 + c - '0');
            else if (c == ' ') vk = 0x20;
            else if (c == '-') vk = 0xBD;
            else if (c == '_') { vk = 0xBD; ctrl = SHIFT_PRESSED; }
            else if (c == '.') vk = 0xBE;
            else if (c == '\r' || c == '\n') vk = 0x0D;
            else vk = (ushort)c;

            char outChar = (c == '\n') ? '\r' : c;
            records.Add(MakeKey(true, vk, outChar, ctrl));
            records.Add(MakeKey(false, vk, outChar, 0));
        }

        // Send ALL records in ONE WriteConsoleInput call
        var arr = records.ToArray();
        uint written;
        bool ok = WriteConsoleInput(handle, arr, (uint)arr.Length, out written);
        int err = ok ? 0 : Marshal.GetLastWin32Error();
        log.Add(string.Format("Batch write: {0} records, ok={1}, written={2}, err={3}",
            arr.Length, ok, written, err));

        FreeConsole();
        File.WriteAllText(logFile, string.Join("\n", log));
        return ok ? 0 : 1;
    }
}
