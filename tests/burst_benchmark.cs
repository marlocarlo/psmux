using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// Burst typing benchmark: injects chars in rapid bursts (0-5ms)
// and monitors screen buffer to measure render latency.
// 
// Sends chars in "word bursts" - a cluster of chars with minimal delay
// between them (0-2ms), then a small gap between words (10-30ms).
// This mimics real fast typing where fingers hit keys in rapid succession.
//
// Usage: burst_benchmark.exe <PID> <text> <intra_char_ms> <inter_word_ms>
// Output: CSV then SUMMARY line

class BurstBenchmark {
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

    static volatile bool done = false;

    static void Main(string[] args) {
        if (args.Length < 4) {
            Console.Error.WriteLine("Usage: burst_benchmark.exe <PID> <text> <intra_char_ms> <inter_word_ms>");
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
            Console.Error.WriteLine("CreateFile failed");
            Environment.Exit(3);
        }

        CSBI csbi;
        GetConsoleScreenBufferInfo(hOut, out csbi);
        short startRow = csbi.dwCursorPosition.Y;
        short startCol = csbi.dwCursorPosition.X;
        int bufW = csbi.dwSize.X;

        // Monitor thread: polls screen buffer every 5ms
        var samples = new List<long[]>();
        int prevLen = 0;
        long firstCharMs = 0, lastCharMs = 0, lastChangeMs = 0;
        int maxGap = 0, stallCount = 0, burstDetections = 0;
        object lockObj = new object();

        var sw = Stopwatch.StartNew();

        Thread monitor = new Thread(() => {
            while (!done || sw.ElapsedMilliseconds < 2000) {
                string vis = ReadRows(hOut, startRow, startCol, bufW, 8);
                int curLen = vis.TrimEnd().Length;
                long ts = sw.ElapsedMilliseconds;

                if (curLen > prevLen) {
                    int delta = curLen - prevLen;
                    lock (lockObj) {
                        if (firstCharMs == 0) firstCharMs = ts;
                        lastCharMs = ts;
                        if (lastChangeMs > 0) {
                            int gap = (int)(ts - lastChangeMs);
                            if (gap > maxGap) maxGap = gap;
                            if (gap > 150) stallCount++;
                            if (delta > 8) burstDetections++;
                            samples.Add(new long[] { ts, curLen, delta, gap });
                        } else {
                            samples.Add(new long[] { ts, curLen, delta, 0 });
                        }
                        lastChangeMs = ts;
                    }
                    prevLen = curLen;
                }
                Thread.Sleep(5); // 5ms poll = 200Hz
            }
        });
        monitor.IsBackground = true;
        monitor.Start();

        // Inject chars with burst pattern
        // Within a word: intraMs delay. Between words (space): interMs delay.
        long injectStart = sw.ElapsedMilliseconds;
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
        long injectEnd = sw.ElapsedMilliseconds;
        long injectDuration = injectEnd - injectStart;

        // Wait for rendering to catch up
        Thread.Sleep(3000);
        done = true;
        monitor.Join(2000);

        CloseHandle(hIn);
        CloseHandle(hOut);
        FreeConsole();

        // Compute gap percentiles
        var gaps = new List<int>();
        foreach (var s in samples) {
            if (s[3] > 0) gaps.Add((int)s[3]);
        }
        gaps.Sort();
        int cnt = gaps.Count;
        int p50 = cnt > 0 ? gaps[cnt / 2] : 0;
        int p90 = cnt > 0 ? gaps[(int)(cnt * 0.9)] : 0;
        int p95 = cnt > 0 ? gaps[(int)(cnt * 0.95)] : 0;
        int p99 = cnt > 0 ? gaps[Math.Min((int)(cnt * 0.99), cnt - 1)] : 0;
        int avg = 0;
        if (cnt > 0) { long sum = 0; foreach (int g in gaps) sum += g; avg = (int)(sum / cnt); }
        long renderSpan = lastCharMs - firstCharMs;

        // CSV output
        Console.WriteLine("TS_MS,VISIBLE,DELTA,GAP_MS");
        foreach (var s in samples) {
            Console.WriteLine("{0},{1},{2},{3}", s[0], s[1], s[2], s[3]);
        }

        Console.WriteLine("SUMMARY chars={0} inject_ms={1} render_ms={2} first_ms={3} last_ms={4} samples={5} stalls={6} bursts={7} max_gap={8} avg_gap={9} p50={10} p90={11} p95={12} p99={13}",
            text.Length, injectDuration, renderSpan, firstCharMs, lastCharMs,
            samples.Count, stallCount, burstDetections, maxGap, avg, p50, p90, p95, p99);
    }

    static string ReadRows(IntPtr hOut, short startRow, short startCol, int bufW, int numRows) {
        StringBuilder all = new StringBuilder();
        for (int r = 0; r < numRows; r++) {
            short row = (short)(startRow + r);
            int readLen = (r == 0) ? bufW - startCol : bufW;
            short col = (r == 0) ? startCol : (short)0;
            if (readLen <= 0) continue;
            var sb = new StringBuilder(readLen);
            uint charsRead;
            ReadConsoleOutputCharacter(hOut, sb, (uint)readLen, new COORD(col, row), out charsRead);
            all.Append(sb.ToString(0, (int)charsRead));
        }
        return all.ToString();
    }
}
