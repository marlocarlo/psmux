using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

class Injector
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
            MakeKey(false, vk, ch, 0)
        };
        uint written;
        bool ok = WriteConsoleInput(h, recs, 2, out written);
        int err = ok ? 0 : Marshal.GetLastWin32Error();
        log.Add(string.Format("  '{0}' vk=0x{1:X2} ok={2} w={3} e={4}",
            ch == '\0' ? "NUL" : ch.ToString(), vk, ok, written, err));
        return ok && written == 2;
    }

    static bool SendCtrlCombo(IntPtr h, char letter, List<string> log)
    {
        ushort vk = (ushort)char.ToUpper(letter);
        char ctrlChar = (char)(char.ToUpper(letter) - 'A' + 1);
        var recs = new INPUT_RECORD[] {
            MakeKey(true,  0x11, '\0',     LEFT_CTRL_PRESSED),
            MakeKey(true,  vk,   ctrlChar, LEFT_CTRL_PRESSED),
            MakeKey(false, vk,   ctrlChar, LEFT_CTRL_PRESSED),
            MakeKey(false, 0x11, '\0',     0)
        };
        uint written;
        bool ok = WriteConsoleInput(h, recs, 4, out written);
        int err = ok ? 0 : Marshal.GetLastWin32Error();
        log.Add(string.Format("  Ctrl+{0} ok={1} w={2} e={3}", letter, ok, written, err));
        return ok && written == 4;
    }

    static int Main(string[] args)
    {
        var log = new List<string>();
        string logFile = Path.Combine(Path.GetTempPath(), "psmux_inject.log");

        if (args.Length < 2)
        {
            File.WriteAllText(logFile, "Usage: injector.exe <pid> <keys>\n" +
                "Keys: chars, ^x=Ctrl+x, {ENTER}, {ESC}, {SLEEP:ms}");
            return 99;
        }

        uint pid;
        if (!uint.TryParse(args[0], out pid))
        {
            File.WriteAllText(logFile, "Invalid PID: " + args[0]);
            return 98;
        }

        string keys = string.Join(" ", args, 1, args.Length - 1);
        log.Add("PID=" + pid + " Keys=" + keys);

        // Detach from our console, attach to target
        FreeConsole();
        if (!AttachConsole(pid))
        {
            log.Add("AttachConsole FAILED err=" + Marshal.GetLastWin32Error());
            File.WriteAllText(logFile, string.Join("\n", log));
            return 2;
        }

        // Open the console input buffer directly
        IntPtr handle = CreateFileW("CONIN$", 0xC0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        if (handle == new IntPtr(-1))
        {
            log.Add("CreateFile(CONIN$) FAILED err=" + Marshal.GetLastWin32Error());
            FreeConsole();
            File.WriteAllText(logFile, string.Join("\n", log));
            return 3;
        }
        log.Add("Handle=" + handle);

        int injected = 0;
        int i = 0;
        while (i < keys.Length)
        {
            if (keys[i] == '^' && i + 1 < keys.Length)
            {
                if (SendCtrlCombo(handle, keys[i + 1], log)) injected++;
                i += 2;
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
                        if (SendKey(handle, 0x0D, '\r', 0, log)) injected++;
                    }
                    else if (token == "ESC" || token == "ESCAPE")
                    {
                        if (SendKey(handle, 0x1B, (char)0x1B, 0, log)) injected++;
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
                else if (c == '_') { vk = 0xBD; ctrl = SHIFT_PRESSED; }
                else if (c == ':') { vk = 0xBA; ctrl = SHIFT_PRESSED; }
                else if (c == '.') vk = 0xBE;
                else if (c == ',') vk = 0xBC;
                else if (c == '/') vk = 0xBF;
                else if (c == '\\') vk = 0xDC;
                else if (c == '[') vk = 0xDB;
                else if (c == ']') vk = 0xDD;
                else if (c == '"') { vk = 0xDE; ctrl = SHIFT_PRESSED; }
                else if (c == '\'') vk = 0xDE;
                else if (c == ';') vk = 0xBA;
                else if (c == '=') vk = 0xBB;
                else if (c == '(') { vk = 0x39; ctrl = SHIFT_PRESSED; }
                else if (c == ')') { vk = 0x30; ctrl = SHIFT_PRESSED; }
                else if (c == '%') { vk = 0x35; ctrl = SHIFT_PRESSED; }
                else if (c == '#') { vk = 0x33; ctrl = SHIFT_PRESSED; }
                else if (c == '@') { vk = 0x32; ctrl = SHIFT_PRESSED; }
                else if (c == '!') { vk = 0x31; ctrl = SHIFT_PRESSED; }
                else if (c == '&') { vk = 0x37; ctrl = SHIFT_PRESSED; }
                else if (c == '*') { vk = 0x38; ctrl = SHIFT_PRESSED; }
                else if (c == '+') { vk = 0xBB; ctrl = SHIFT_PRESSED; }
                else vk = (ushort)c;

                if (SendKey(handle, vk, c, ctrl, log)) injected++;
                i++;
                Thread.Sleep(30);
            }
        }

        log.Add("Injected=" + injected);
        FreeConsole();
        File.WriteAllText(logFile, string.Join("\n", log));
        return 0;
    }
}
