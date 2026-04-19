using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// Burst benchmark v2: scans FULL visible console window for changes
// instead of tracking from cursor position. Works reliably for both
// psmux TUI and direct PowerShell.
//
// Usage: burst_bench2.exe <PID> <text> <intra_char_ms> <inter_word_ms>

class BurstBench2 {
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
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool ReadConsoleOutputCharacter(IntPtr h, StringBuilder sb, uint len, COORD coord, out uint read);
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

    // Read the entire visible window content as a single string
    static string ReadFullScreen(IntPtr hOut, CSBI csbi) {
        int width = csbi.dwSize.X;
        int visTop = csbi.srWindow.Top;
        int visBot = csbi.srWindow.Bottom;
        var all = new StringBuilder();
        for (int row = visTop; row <= visBot; row++) {
            var sb = new StringBuilder(width);
            uint read;
            ReadConsoleOutputCharacter(hOut, sb, (uint)width, new COORD(0, (short)row), out read);
            all.Append(sb.ToString(0, (int)read).TrimEnd());
            all.Append('\n');
        }
        return all.ToString();
    }

    // Count non-whitespace chars in screen content
    static int CountChars(string screen) {
        int c = 0;
        foreach (char ch in screen) {
            if (ch != ' ' && ch != '\n' && ch != '\r' && ch != '\0') c++;
        }
        return c;
    }

    static void Main(string[] args) {
        if (args.Length < 4) {
            Console.Error.WriteLine("Usage: burst_bench2.exe <PID> <text> <intra_ms> <inter_ms>");
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

        // Read baseline screen content
        string baseScreen = ReadFullScreen(hOut, csbi);
        int baseChars = CountChars(baseScreen);

        var samples = new List<long[]>(); // ts, charCount, delta, gap
        int prevChars = baseChars;
        long firstMs = 0, lastMs = 0, lastChangeMs = 0;
        int maxGap = 0, stallCount = 0, burstDetections = 0;
        var sw = Stopwatch.StartNew();

        // Monitor thread: scans full screen every 5ms
        Thread monitor = new Thread(() => {
            while (!injDone || sw.ElapsedMilliseconds < (injDone ? sw.ElapsedMilliseconds + 3000 : 60000)) {
                CSBI curCsbi;
                GetConsoleScreenBufferInfo(hOut, out curCsbi);
                string screen = ReadFullScreen(hOut, curCsbi);
                int chars = CountChars(screen);
                long ts = sw.ElapsedMilliseconds;

                if (chars > prevChars) {
                    int delta = chars - prevChars;
                    if (firstMs == 0) firstMs = ts;
                    lastMs = ts;
                    int gap = 0;
                    if (lastChangeMs > 0) {
                        gap = (int)(ts - lastChangeMs);
                        if (gap > maxGap) maxGap = gap;
                        if (gap > 150) stallCount++;
                        if (delta > 8) burstDetections++;
                    }
                    samples.Add(new long[] { ts, chars, delta, gap });
                    lastChangeMs = ts;
                    prevChars = chars;
                }

                if (injDone && (ts - lastMs) > 3000) break;
                Thread.Sleep(5);
            }
        });
        monitor.IsBackground = true;
        monitor.Start();

        Thread.Sleep(50); // let monitor start

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

        // Wait for render to catch up
        Thread.Sleep(4000);
        injDone = true;
        monitor.Join(5000);

        CloseHandle(hIn);
        CloseHandle(hOut);
        FreeConsole();

        // Percentiles
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

        // Output
        Console.WriteLine("TS_MS,CHARS,DELTA,GAP_MS");
        foreach (var s in samples) {
            Console.WriteLine("{0},{1},{2},{3}", s[0], s[1], s[2], s[3]);
        }
        Console.WriteLine("SUMMARY chars={0} inject_ms={1} render_ms={2} first_ms={3} last_ms={4} samples={5} stalls={6} bursts={7} max_gap={8} avg_gap={9} p50={10} p90={11} p95={12} p99={13}",
            text.Length, injDuration, renderSpan, firstMs, lastMs,
            samples.Count, stallCount, burstDetections, maxGap, avg, p50, p90, p95, p99);
    }
}
