using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// Cursor-based render latency monitor
// Tracks console cursor position movement as chars are typed.
// Cursor advances = character rendered on screen.
// Works for both psmux TUI and direct PowerShell.
//
// Usage: cursor_bench.exe <PID> <text> <intra_char_ms> <inter_word_ms>

class CursorBench {
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
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleScreenBufferInfo(IntPtr h, out CSBI info);

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
    [StructLayout(LayoutKind.Sequential)]
    struct COORD {
        public short X, Y;
        public COORD(short x, short y) { X = x; Y = y; }
    }
    [StructLayout(LayoutKind.Sequential)]
    struct SMALL_RECT { public short Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    struct CSBI {
        public COORD dwSize;
        public COORD dwCursorPosition;
        public ushort wAttributes;
        public SMALL_RECT srWindow;
        public COORD dwMaximumWindowSize;
    }

    static volatile bool injDone = false;

    // Convert cursor position to a linear offset for comparison
    static int CursorOffset(CSBI csbi) {
        return csbi.dwCursorPosition.Y * csbi.dwSize.X + csbi.dwCursorPosition.X;
    }

    static void Main(string[] args) {
        if (args.Length < 4) {
            Console.Error.WriteLine("Usage: cursor_bench.exe <PID> <text> <intra_ms> <inter_ms>");
            Environment.Exit(1);
        }
        uint pid = uint.Parse(args[0]);
        string text = args[1];
        int intraMs = int.Parse(args[2]);
        int interMs = int.Parse(args[3]);

        FreeConsole();
        if (!AttachConsole(pid)) {
            Console.Error.WriteLine("AttachConsole failed: " + Marshal.GetLastWin32Error());
            Environment.Exit(2);
        }

        IntPtr hIn = CreateFile("CONIN$", 0xC0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        IntPtr hOut = CreateFile("CONOUT$", 0x80000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        if (hIn == (IntPtr)(-1) || hOut == (IntPtr)(-1)) {
            Console.Error.WriteLine("CreateFile failed"); Environment.Exit(3);
        }

        CSBI csbi;
        GetConsoleScreenBufferInfo(hOut, out csbi);
        int baseOffset = CursorOffset(csbi);
        int bufWidth = csbi.dwSize.X;

        var samples = new List<long[]>(); // ts, charsRendered, delta, gap
        int prevRendered = 0;
        long firstMs = 0, lastMs = 0, lastChangeMs = 0;
        int maxGap = 0, stallCount = 0, burstDetections = 0;
        var sw = Stopwatch.StartNew();

        // Monitor thread: polls cursor position every 3ms (333Hz)
        Thread monitor = new Thread(() => {
            long deadline = 60000;
            while (sw.ElapsedMilliseconds < deadline) {
                CSBI cur;
                GetConsoleScreenBufferInfo(hOut, out cur);
                int curOffset = CursorOffset(cur);
                int rendered = curOffset - baseOffset;
                long ts = sw.ElapsedMilliseconds;

                if (rendered > prevRendered) {
                    int delta = rendered - prevRendered;
                    if (firstMs == 0) firstMs = ts;
                    lastMs = ts;
                    int gap = 0;
                    if (lastChangeMs > 0) {
                        gap = (int)(ts - lastChangeMs);
                        if (gap > maxGap) maxGap = gap;
                        if (gap > 100) stallCount++;
                        if (delta > 8) burstDetections++;
                    }
                    samples.Add(new long[] { ts, rendered, delta, gap });
                    lastChangeMs = ts;
                    prevRendered = rendered;
                }

                // If injection done and no change for 3s, stop
                if (injDone && lastMs > 0 && (ts - lastMs) > 3000) break;
                // If no injection done within 30s, timeout
                if (ts > 30000 && firstMs == 0) break;

                Thread.Sleep(3);
            }
        });
        monitor.IsBackground = true;
        monitor.Start();

        Thread.Sleep(30);

        // Inject with burst pattern
        long injStart = sw.ElapsedMilliseconds;
        foreach (char c in text) {
            INPUT_RECORD[] recs = new INPUT_RECORD[2];
            recs[0].EventType = 1;
            recs[0].KeyEvent.bKeyDown = 1;
            recs[0].KeyEvent.wRepeatCount = 1;
            recs[0].KeyEvent.UnicodeChar = c;
            recs[1].EventType = 1;
            recs[1].KeyEvent.bKeyDown = 0;
            recs[1].KeyEvent.wRepeatCount = 1;
            recs[1].KeyEvent.UnicodeChar = c;
            uint written;
            WriteConsoleInput(hIn, recs, 2, out written);

            if (c == ' ') {
                if (interMs > 0) Thread.Sleep(interMs);
            } else {
                if (intraMs > 0) Thread.Sleep(intraMs);
            }
        }
        long injDuration = sw.ElapsedMilliseconds - injStart;
        injDone = true;

        monitor.Join(8000);

        CloseHandle(hIn);
        CloseHandle(hOut);
        FreeConsole();

        // Compute stats
        var gaps = new List<int>();
        foreach (var s in samples) { if (s[3] > 0) gaps.Add((int)s[3]); }
        gaps.Sort();
        int cnt = gaps.Count;
        int p50 = cnt > 0 ? gaps[cnt / 2] : 0;
        int p90 = cnt > 0 ? gaps[(int)(cnt * 0.9)] : 0;
        int p95 = cnt > 0 ? gaps[Math.Min((int)(cnt * 0.95), cnt - 1)] : 0;
        int p99 = cnt > 0 ? gaps[Math.Min((int)(cnt * 0.99), cnt - 1)] : 0;
        int avg = 0;
        if (cnt > 0) { long sum = 0; foreach (int g in gaps) sum += g; avg = (int)(sum / cnt); }
        long renderSpan = lastMs - firstMs;

        Console.WriteLine("TS_MS,RENDERED,DELTA,GAP_MS");
        foreach (var s in samples) {
            Console.WriteLine("{0},{1},{2},{3}", s[0], s[1], s[2], s[3]);
        }
        Console.WriteLine("SUMMARY chars={0} inject_ms={1} render_ms={2} first_ms={3} last_ms={4} samples={5} stalls={6} bursts={7} max_gap={8} avg_gap={9} p50={10} p90={11} p95={12} p99={13} rendered={14}",
            text.Length, injDuration, renderSpan, firstMs, lastMs,
            samples.Count, stallCount, burstDetections, maxGap, avg, p50, p90, p95, p99, prevRendered);
    }
}
