#[allow(unused_imports)]
// ---------------------------------------------------------------------------
// CREATE_NO_WINDOW for background subprocesses
// ---------------------------------------------------------------------------

/// Windows `CREATE_NO_WINDOW` flag (0x08000000).
///
/// When set on `CreateProcess`, the child process does not get a console
/// window allocated by conhost.  This is the correct flag for *helper*
/// subprocesses (format `#()` expansion, `run-shell`, `if-shell`, clipboard
/// pipes, plugin scripts) that only need stdin/stdout/stderr pipes.
///
/// **Important:** PTY/ConPTY child processes and psmux server processes must
/// NOT use this flag because they need a real console session.  Those use
/// `spawn_server_hidden()` (with `CREATE_NEW_CONSOLE` + `SW_HIDE`) instead.
///
/// On non-Windows platforms this is a no-op.
#[cfg(windows)]
use super::*;

#[cfg(not(windows))]
pub mod process_info {
    pub fn get_process_name(_pid: u32) -> Option<String> { None }
    pub fn get_process_cwd(_pid: u32) -> Option<String> { None }
    pub fn get_foreground_process_name(_pid: u32) -> Option<String> { None }
    pub fn get_foreground_cwd(_pid: u32) -> Option<String> { None }
    pub fn has_vt_bridge_descendant(_root_pid: u32) -> bool { false }
}

// ─── UTF-16 Console Writer (Windows) ────────────────────────────────────
//
// On Windows, Rust's `Stdout::write()` uses `WriteFile` which sends raw
// bytes to the console.  The console interprets those bytes according to
// the *output code page* (typically 437 or 1252, **not** UTF-8).  Even
// after calling `SetConsoleOutputCP(65001)`, ConPTY has incomplete support
// for multi-byte UTF-8 sequences delivered through `WriteFile`, causing
// characters like ▶ (U+25B6, 3 bytes: E2 96 B6) to render as mojibake
// (e.g. `â¶`).
//
// The fix is to bypass `WriteFile` entirely and use `WriteConsoleW`, which
// accepts UTF-16 wide strings and renders them correctly regardless of
// the console codepage.  This wrapper converts incoming UTF-8 bytes to
// UTF-16 on the fly and writes them with `WriteConsoleW`.

/// A [`std::io::Write`] implementation that renders Unicode correctly on
/// Windows by converting UTF-8 → UTF-16 and calling `WriteConsoleW`.
///
/// Crucially, this buffers incomplete trailing UTF-8 sequences between
/// `write()` calls.  `write_all()` may split a buffer at any byte
/// boundary — including in the middle of a multi-byte character like
/// `▶` (U+25B6, bytes E2 96 B6).  Without buffering, each orphaned byte
/// would be emitted as a Latin-1 code point (`â`, `¶`), producing the
/// exact garbling the user sees.
#[cfg(windows)]
pub struct Utf16ConsoleWriter {
    pub(crate) handle: *mut std::ffi::c_void,
    /// Frame buffer: accumulates all `write()` output so that `flush()`
    /// can emit the complete frame as a single `WriteConsoleW` call.
    /// This eliminates the visible top-to-bottom "curtain" repaint that
    /// occurs when ratatui's many small per-cell writes are each sent to
    /// the console individually.
    pub(crate) frame_buf: Vec<u8>,
}

#[cfg(windows)]
unsafe impl Send for Utf16ConsoleWriter {}

#[cfg(windows)]
impl Utf16ConsoleWriter {
    pub fn new() -> Self {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
        }
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        // Pre-allocate ~128KB for the frame buffer — large enough for a
        // typical full-screen frame's escape sequences without reallocation.
        Self { handle, frame_buf: Vec::with_capacity(131072) }
    }

    /// Write a valid UTF-8 string via `WriteConsoleW`.
    pub(crate) fn write_wide(&self, s: &str) -> std::io::Result<()> {
        if s.is_empty() {
            return Ok(());
        }

        #[link(name = "kernel32")]
        extern "system" {
            fn WriteConsoleW(
                hConsoleOutput: *mut std::ffi::c_void,
                lpBuffer: *const u16,
                nNumberOfCharsToWrite: u32,
                lpNumberOfCharsWritten: *mut u32,
                lpReserved: *mut std::ffi::c_void,
            ) -> i32;
        }

        let wide: Vec<u16> = s.encode_utf16().collect();
        let mut total: u32 = 0;
        let len = wide.len() as u32;
        while total < len {
            let mut written: u32 = 0;
            let ok = unsafe {
                WriteConsoleW(
                    self.handle,
                    wide.as_ptr().add(total as usize),
                    len - total,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if written == 0 {
                break;
            }
            total += written;
        }
        Ok(())
    }
}

#[cfg(windows)]
impl std::io::Write for Utf16ConsoleWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Append to the frame buffer — actual console output is deferred
        // until flush(), so all of ratatui's per-cell writes within a
        // single draw() call are batched into one atomic WriteConsoleW.
        self.frame_buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.frame_buf.is_empty() {
            return Ok(());
        }

        // Convert the buffered UTF-8 to a valid string, handling any
        // incomplete trailing multi-byte sequence.
        let (valid, remainder) = match std::str::from_utf8(&self.frame_buf) {
            Ok(s) => (s.len(), 0),
            Err(e) => {
                let valid_end = e.valid_up_to();
                // If error_len is None, trailing bytes are an incomplete
                // sequence — they'll be completed by the next write.
                // If it's Some, those bytes are genuinely invalid — skip.
                let skip = e.error_len().unwrap_or(0);
                (valid_end, self.frame_buf.len() - valid_end - skip)
            }
        };

        if valid > 0 {
            // Safety: we just validated this range is valid UTF-8.
            let s = unsafe { std::str::from_utf8_unchecked(&self.frame_buf[..valid]) };
            self.write_wide(s)?;
        }

        // Keep any incomplete trailing bytes for the next flush.
        if remainder > 0 {
            let start = self.frame_buf.len() - remainder;
            // Rotate trailing bytes to front.
            let mut i = 0;
            while i < remainder {
                self.frame_buf[i] = self.frame_buf[start + i];
                i += 1;
            }
            self.frame_buf.truncate(remainder);
        } else {
            self.frame_buf.clear();
        }

        Ok(())
    }
}

/// Platform-independent writer type for the TUI backend.
///
/// On Windows this uses [`Utf16ConsoleWriter`] (WriteConsoleW) so that
/// multi-byte UTF-8 characters render correctly.  On other platforms it
/// is simply [`std::io::Stdout`].
#[cfg(windows)]
pub type PsmuxWriter = Utf16ConsoleWriter;

#[cfg(not(windows))]
pub type PsmuxWriter = std::io::Stdout;

/// Create a new [`PsmuxWriter`].
pub fn create_writer() -> PsmuxWriter {
    #[cfg(windows)]
    { Utf16ConsoleWriter::new() }
    #[cfg(not(windows))]
    { std::io::stdout() }
}

// ---------------------------------------------------------------------------
// Win32 System Caret — Accessibility / Speech-to-Text support
// ---------------------------------------------------------------------------
// Speech-to-text tools like Wispr Flow use GetGUIThreadInfo() to locate the
// system caret.  When psmux enters raw mode + alternate screen, the default
// console caret is hidden and accessibility tools lose track of the text
// insertion point.
//
// By creating a Win32 caret on the console window and updating its position
// every frame, accessibility tools can detect the active text input context
// and inject transcribed text.
//
// These functions are safe to call on all platforms; non-Windows builds are
// no-ops.  SSH sessions should skip calling these (no local console window).
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod caret {
    use std::sync::atomic::{AtomicBool, Ordering};

    static CARET_CREATED: AtomicBool = AtomicBool::new(false);

    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleWindow() -> isize;
        fn GetCurrentConsoleFontEx(
            hConsoleOutput: *mut std::ffi::c_void,
            bMaximumWindow: i32,
            lpConsoleCurrentFontEx: *mut CONSOLE_FONT_INFOEX,
        ) -> i32;
        fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
    }

    #[link(name = "user32")]
    extern "system" {
        fn CreateCaret(hWnd: isize, hBitmap: isize, nWidth: i32, nHeight: i32) -> i32;
        fn SetCaretPos(x: i32, y: i32) -> i32;
        fn ShowCaret(hWnd: isize) -> i32;
        fn DestroyCaret() -> i32;
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct CONSOLE_FONT_INFOEX {
        cbSize: u32,
        nFont: u32,
        dwFontSize_X: i16,
        dwFontSize_Y: i16,
        FontFamily: u32,
        FontWeight: u32,
        FaceName: [u16; 32],
    }

    /// Query the current console font cell size in pixels.
    /// Returns (cell_width, cell_height).  Falls back to (8, 16) on failure.
    fn console_cell_size() -> (i32, i32) {
        const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle.is_null() || handle == (-1isize) as *mut std::ffi::c_void {
                return (8, 16);
            }
            let mut info: CONSOLE_FONT_INFOEX = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<CONSOLE_FONT_INFOEX>() as u32;
            if GetCurrentConsoleFontEx(handle, 0, &mut info) != 0 {
                let w = if info.dwFontSize_X > 0 { info.dwFontSize_X as i32 } else { 8 };
                let h = if info.dwFontSize_Y > 0 { info.dwFontSize_Y as i32 } else { 16 };
                (w, h)
            } else {
                (8, 16)
            }
        }
    }

    /// Create the system caret on the console window (if not already created)
    /// and update its position to the given terminal cell coordinates.
    ///
    /// `col` and `row` are 0-based terminal cell coordinates (the same values
    /// used for VT CUP positioning).
    pub fn update(col: u16, row: u16) {
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd == 0 {
                return;
            }
            if !CARET_CREATED.load(Ordering::Relaxed) {
                let (cw, ch) = console_cell_size();
                if CreateCaret(hwnd, 0, cw.max(1), ch.max(1)) != 0 {
                    CARET_CREATED.store(true, Ordering::Relaxed);
                    ShowCaret(hwnd);
                }
            }
            let (cw, ch) = console_cell_size();
            SetCaretPos(col as i32 * cw, row as i32 * ch);
        }
    }

    /// Hide and destroy the system caret.  Call on exit.
    pub fn destroy() {
        if CARET_CREATED.swap(false, Ordering::Relaxed) {
            unsafe { DestroyCaret(); }
        }
    }
}

#[cfg(not(windows))]
pub mod caret {
    pub fn update(_col: u16, _row: u16) {}
    pub fn destroy() {}
}

/// On Windows ConPTY, Shift+Enter is misreported by crossterm:
///
/// VS Code's xterm.js sends `\x1b\r` (ESC + CR) for Shift+Enter.
/// ConPTY interprets the ESC prefix as Alt, so crossterm reports
/// `KeyModifiers::ALT` instead of `KeyModifiers::SHIFT`.
///
/// This function polls the physical keyboard state to detect the real
/// modifiers and remaps accordingly.
#[cfg(windows)]
pub fn augment_enter_shift(key: &mut crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};

    if !matches!(key.code, KeyCode::Enter) {
        return;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        return;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(vKey: i32) -> i16;
    }

    const VK_SHIFT: i32 = 0x10;
    const VK_CONTROL: i32 = 0x11;
    const VK_MENU: i32 = 0x12; // Alt

    unsafe {
        let shift_down = GetAsyncKeyState(VK_SHIFT) < 0;
        let ctrl_down = GetAsyncKeyState(VK_CONTROL) < 0;
        let alt_down = GetAsyncKeyState(VK_MENU) < 0;

        if shift_down {
            key.modifiers.insert(KeyModifiers::SHIFT);
            // Windows Terminal + crossterm sometimes reports a phantom CONTROL
            // modifier on the Press event for Shift+Enter while the physical
            // Ctrl key is not held.  Remove it.
            if !ctrl_down && key.modifiers.contains(KeyModifiers::CONTROL) {
                key.modifiers.remove(KeyModifiers::CONTROL);
            }
            if !alt_down && key.modifiers.contains(KeyModifiers::ALT) {
                key.modifiers.remove(KeyModifiers::ALT);
            }
        } else if !shift_down && !ctrl_down && !alt_down {
            // No physical modifiers held; ConPTY may have injected a phantom
            // ALT from ESC+CR.  Already handled by the early return for SHIFT
            // above, but guard plain Enter too.
        } else if !shift_down && alt_down {
            // Physical Alt is held, leave as is.
        }
    }
}
