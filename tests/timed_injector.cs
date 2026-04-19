using System;
using System.Runtime.InteropServices;
using System.Threading;

// Timed keystroke injector: sends chars at a specified interval (ms)
// Usage: timed_injector.exe <PID> <text> <interval_ms>
// Example: timed_injector.exe 1234 "hello world this is a long sentence" 15
class TimedInjector {
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(uint pid);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FreeConsole();
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFile(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool WriteConsoleInput(IntPtr h, INPUT_RECORD[] buf, uint len, out uint written);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr h);

    [StructLayout(LayoutKind.Explicit)]
    struct INPUT_RECORD {
        [FieldOffset(0)] public ushort EventType;
        [FieldOffset(4)] public KEY_EVENT_RECORD KeyEvent;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct KEY_EVENT_RECORD {
        public int bKeyDown;
        public ushort wRepeatCount;
        public ushort wVirtualKeyCode;
        public ushort wVirtualScanCode;
        public char UnicodeChar;
        public uint dwControlKeyState;
    }

    static void Main(string[] args) {
        if (args.Length < 3) {
            Console.Error.WriteLine("Usage: timed_injector.exe <PID> <text> <interval_ms>");
            Environment.Exit(1);
        }
        uint pid = uint.Parse(args[0]);
        string text = args[1];
        int interval = int.Parse(args[2]);

        FreeConsole();
        if (!AttachConsole(pid)) {
            Console.Error.WriteLine("AttachConsole failed: " + Marshal.GetLastWin32Error());
            Environment.Exit(2);
        }
        IntPtr h = CreateFile("CONIN$", 0xC0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        if (h == (IntPtr)(-1)) {
            Console.Error.WriteLine("CreateFile CONIN$ failed: " + Marshal.GetLastWin32Error());
            Environment.Exit(3);
        }

        int sent = 0;
        foreach (char c in text) {
            INPUT_RECORD[] recs = new INPUT_RECORD[2];
            // Key down
            recs[0].EventType = 1; // KEY_EVENT
            recs[0].KeyEvent.bKeyDown = 1;
            recs[0].KeyEvent.wRepeatCount = 1;
            recs[0].KeyEvent.UnicodeChar = c;
            recs[0].KeyEvent.wVirtualKeyCode = 0;
            // Key up
            recs[1].EventType = 1;
            recs[1].KeyEvent.bKeyDown = 0;
            recs[1].KeyEvent.wRepeatCount = 1;
            recs[1].KeyEvent.UnicodeChar = c;
            recs[1].KeyEvent.wVirtualKeyCode = 0;

            uint written;
            WriteConsoleInput(h, recs, 2, out written);
            sent++;

            if (interval > 0) {
                Thread.Sleep(interval);
            }
        }
        CloseHandle(h);
        FreeConsole();
        Console.WriteLine("OK sent=" + sent + " interval=" + interval + "ms");
    }
}
