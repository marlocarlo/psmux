using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// Keystroke injector + screen buffer monitor
// Injects chars at a fixed rate while polling the console screen buffer
// to measure when each character actually RENDERS on screen.
//
// Usage: typing_benchmark.exe <PID> <text> <interval_ms> <monitor_row> <monitor_col_start>
//
// Output: CSV lines with timestamp_ms, visible_char_count, delta_chars
// Final line: SUMMARY inject_ms=N render_ms=N stalls=N max_gap_ms=N chars=N

class TypingBenchmark {
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(uint pid);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FreeConsole();
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFile(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool WriteConsoleInput(IntPtr h, INPUT_RECORD[] buf, uint len, out uint written);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int nStdHandle);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool ReadConsoleOutputCharacter(IntPtr hConsoleOutput, StringBuilder lpCharacter, 
        uint nLength, COORD dwReadCoord, out uint lpNumberOfCharsRead);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleScreenBufferInfo(IntPtr hConsoleOutput, out CONSOLE_SCREEN_BUFFER_INFO lpConsoleScreenBufferInfo);

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
        public short X;
        public short Y;
        public COORD(short x, short y) { X = x; Y = y; }
    }
    [StructLayout(LayoutKind.Sequential)]
    struct SMALL_RECT {
        public short Left, Top, Right, Bottom;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct CONSOLE_SCREEN_BUFFER_INFO {
        public COORD dwSize;
        public COORD dwCursorPosition;
        public ushort wAttributes;
        public SMALL_RECT srWindow;
        public COORD dwMaximumWindowSize;
    }

    static volatile bool injectionDone = false;
    static volatile int injectedCount = 0;

    static void Main(string[] args) {
        if (args.Length < 3) {
            Console.Error.WriteLine("Usage: typing_benchmark.exe <PID> <text> <interval_ms> [monitor_row] [monitor_col_start]");
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

        IntPtr hIn = CreateFile("CONIN$", 0xC0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        IntPtr hOut = CreateFile("CONOUT$", 0x80000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        
        if (hIn == (IntPtr)(-1) || hOut == (IntPtr)(-1)) {
            Console.Error.WriteLine("CreateFile failed: " + Marshal.GetLastWin32Error());
            Environment.Exit(3);
        }

        // Get current cursor position to know where to monitor
        CONSOLE_SCREEN_BUFFER_INFO csbi;
        GetConsoleScreenBufferInfo(hOut, out csbi);
        short monitorRow = csbi.dwCursorPosition.Y;
        short monitorColStart = csbi.dwCursorPosition.X;
        int bufWidth = csbi.dwSize.X;

        // Override with args if provided
        if (args.Length > 3) monitorRow = short.Parse(args[3]);
        if (args.Length > 4) monitorColStart = short.Parse(args[4]);

        // Results storage
        var samples = new List<long[]>(); // [timestamp_ms, visible_chars, delta]
        var sw = Stopwatch.StartNew();

        // Read initial screen content at monitor row
        string initialContent = ReadRow(hOut, monitorRow, monitorColStart, bufWidth);
        int baseLen = initialContent.TrimEnd().Length;

        // Start injection thread
        var injThread = new Thread(() => {
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
                Interlocked.Increment(ref injectedCount);
                if (interval > 0) Thread.Sleep(interval);
            }
            injectionDone = true;
        });

        // Start monitor loop
        int prevVisibleLen = 0;
        long lastChangeMs = 0;
        int maxGapMs = 0;
        int stallCount = 0;
        int burstCount = 0;
        long firstCharMs = 0;
        long lastCharMs = 0;

        injThread.Start();

        // Poll screen buffer every 10ms
        int timeoutMs = (text.Length * Math.Max(interval, 1)) + 10000;
        bool allSeen = false;

        while (sw.ElapsedMilliseconds < timeoutMs && !allSeen) {
            // Read multiple rows (text may wrap)
            string visibleText = "";
            for (short row = monitorRow; row < monitorRow + 5 && row < csbi.dwSize.Y; row++) {
                string rowContent;
                if (row == monitorRow) {
                    rowContent = ReadRow(hOut, row, monitorColStart, bufWidth);
                } else {
                    rowContent = ReadRow(hOut, row, 0, bufWidth);
                }
                string trimmed = rowContent.TrimEnd();
                if (trimmed.Length > 0) {
                    visibleText += trimmed;
                }
            }

            int curLen = visibleText.Length;
            long ts = sw.ElapsedMilliseconds;

            if (curLen != prevVisibleLen) {
                int delta = curLen - prevVisibleLen;
                if (prevVisibleLen > 0) {
                    int gap = (int)(ts - lastChangeMs);
                    if (gap > maxGapMs) maxGapMs = gap;
                    if (gap > 200) stallCount++;
                    if (delta > 5) burstCount++;
                }
                if (firstCharMs == 0 && curLen > 0) firstCharMs = ts;
                lastCharMs = ts;
                lastChangeMs = ts;

                samples.Add(new long[] { ts, curLen, delta });
                prevVisibleLen = curLen;
            }

            if (curLen >= text.Length) allSeen = true;

            Thread.Sleep(10);
        }

        injThread.Join(5000);

        CloseHandle(hIn);
        CloseHandle(hOut);
        FreeConsole();

        // Output CSV
        Console.WriteLine("TIMESTAMP_MS,VISIBLE_CHARS,DELTA");
        foreach (var s in samples) {
            Console.WriteLine(s[0] + "," + s[1] + "," + s[2]);
        }

        // Compute percentiles from gaps
        var gaps = new List<int>();
        for (int i = 1; i < samples.Count; i++) {
            gaps.Add((int)(samples[i][0] - samples[i - 1][0]));
        }
        gaps.Sort();

        int p50 = gaps.Count > 0 ? gaps[gaps.Count / 2] : 0;
        int p90 = gaps.Count > 0 ? gaps[(int)(gaps.Count * 0.9)] : 0;
        int p99 = gaps.Count > 0 ? gaps[(int)(gaps.Count * 0.99)] : 0;
        int avgGap = 0;
        if (gaps.Count > 0) {
            long sum = 0; foreach (int g in gaps) sum += g; avgGap = (int)(sum / gaps.Count);
        }

        long renderSpan = lastCharMs - firstCharMs;
        long injectTime = text.Length * Math.Max(interval, 1);

        Console.WriteLine("SUMMARY chars={0} inject_ms={1} render_ms={2} first_char_ms={3} last_char_ms={4} samples={5} stalls={6} bursts={7} max_gap_ms={8} avg_gap_ms={9} p50_ms={10} p90_ms={11} p99_ms={12} all_seen={13}",
            text.Length, injectTime, renderSpan, firstCharMs, lastCharMs, 
            samples.Count, stallCount, burstCount, maxGapMs, avgGap, p50, p90, p99, allSeen);
    }

    static string ReadRow(IntPtr hOut, short row, short colStart, int bufWidth) {
        int readLen = bufWidth - colStart;
        if (readLen <= 0) return "";
        var sb = new StringBuilder(readLen);
        uint charsRead;
        ReadConsoleOutputCharacter(hOut, sb, (uint)readLen, new COORD(colStart, row), out charsRead);
        return sb.ToString(0, (int)charsRead);
    }
}
