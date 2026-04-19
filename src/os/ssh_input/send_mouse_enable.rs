#[allow(unused_imports)]

use std::io;
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};

/// Explicitly (re-)send the VT mouse-enable escape sequences to stdout.
///
/// Over SSH, ConPTY may consume DECSET 1000/1002/1003/1006 from the output
/// stream and NOT forward them to sshd.  This tries several approaches:
///  1. `WriteFile` on the raw console output handle (may bypass ConPTY VT
///     processing in some Windows builds).
///  2. A regular `write_all` to stdout (belt-and-suspenders).
///
/// Call this **after** crossterm's `EnableMouseCapture` and `InputSource::new`.
#[cfg(windows)]
use super::*;

pub fn send_mouse_enable() {
    // The DEC private mode escape sequences for mouse reporting:
    //   1000 = basic mouse tracking
    //   1002 = button-event tracking (drag)
    //   1003 = any-event tracking (motion)
    //   1006 = SGR extended mouse format
    const MOUSE_ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";

    ssh_debug_log("send_mouse_enable: writing mouse-enable VT sequences to stdout");

    // Approach 1: WriteFile on the raw output handle.
    // This uses the Win32 file I/O path rather than WriteConsole, which
    // may behave differently under ConPTY.
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
            fn WriteFile(
                hFile: *mut std::ffi::c_void,
                lpBuffer: *const u8,
                nNumberOfBytesToWrite: u32,
                lpNumberOfBytesWritten: *mut u32,
                lpOverlapped: *mut std::ffi::c_void,
            ) -> i32;
        }
        const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if !h.is_null() && h != (-1isize) as *mut std::ffi::c_void {
            let mut written: u32 = 0;
            let ok = WriteFile(
                h,
                MOUSE_ENABLE.as_ptr(),
                MOUSE_ENABLE.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
            ssh_debug_log(&format!(
                "send_mouse_enable: WriteFile ok={} written={}",
                ok, written,
            ));
        } else {
            ssh_debug_log("send_mouse_enable: GetStdHandle(STDOUT) failed");
        }
    }

    // Approach 2: standard Rust stdout write (goes through ConPTY normally).
    use std::io::Write;
    let mut out = io::stdout().lock();
    let _ = out.write_all(MOUSE_ENABLE);
    let _ = out.flush();
    ssh_debug_log("send_mouse_enable: stdout write_all done");

    // Approach 3: Also send a Device Status Report (DSR) probe.
    // If ConPTY is in VT pass-through mode, the query \x1b[5n should reach
    // the client terminal, which responds with \x1b[0n.  If we later see
    // that response in our reader thread (as KEY_EVENT chars: ESC [ 0 n),
    // it proves output→client→input roundtrip works through ConPTY.
    // If we don't see it, ConPTY is consuming VT queries (Windows 10).
    const DSR_PROBE: &[u8] = b"\x1b[5n";
    let _ = out.write_all(DSR_PROBE);
    let _ = out.flush();
    ssh_debug_log("send_mouse_enable: DSR probe \\x1b[5n sent (expect \\x1b[0n response)");

    // Also log the stdout console mode for diagnostics.
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
            fn GetConsoleMode(h: *mut std::ffi::c_void, mode: *mut u32) -> i32;
            fn SetConsoleMode(h: *mut std::ffi::c_void, mode: u32) -> i32;
        }
        const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
        const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if !h.is_null() && h != (-1isize) as *mut std::ffi::c_void {
            let mut mode: u32 = 0;
            if GetConsoleMode(h, &mut mode) != 0 {
                let vtp = mode & 0x0004 != 0; // ENABLE_VIRTUAL_TERMINAL_PROCESSING
                ssh_debug_log(&format!(
                    "stdout console mode: 0x{:04X} VTP={} (pass-through={})",
                    mode, vtp, if vtp { "likely" } else { "NO" },
                ));
            }
        }
        // Verify and restore VTI + MOUSE_INPUT on stdin — these can be
        // cleared by crossterm's raw_mode toggle or ConPTY internal resets.
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        if !hin.is_null() && hin != (-1isize) as *mut std::ffi::c_void {
            let mut mode: u32 = 0;
            if GetConsoleMode(hin, &mut mode) != 0 {
                let vti = mode & 0x0200 != 0;
                let mouse = mode & 0x0010 != 0;
                ssh_debug_log(&format!(
                    "stdin console mode: 0x{:04X} VTI={} MOUSE={}",
                    mode, vti, mouse,
                ));
                if !vti || !mouse {
                    let fixed = mode | 0x0200 | 0x0010; // VTI + ENABLE_MOUSE_INPUT
                    SetConsoleMode(hin, fixed);
                    ssh_debug_log(&format!(
                        "stdin mode restored: 0x{:04X} -> 0x{:04X}",
                        mode, fixed,
                    ));
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn send_mouse_enable() {
    // On Unix, crossterm's EnableMouseCapture already works correctly.
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Returns `true` when the current process appears to run inside an SSH session.
pub fn is_ssh_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_CLIENT").is_some()
        || std::env::var_os("SSH_TTY").is_some()
}

/// Returns `true` when the terminal sends VT mouse sequences through ConPTY
/// input instead of native MOUSE_EVENT INPUT_RECORDs.
///
/// JetBrains IDEs (IntelliJ, Rider, etc.) use JediTerm, which writes VT
/// mouse escape sequences to the ConPTY input pipe.  ConPTY does NOT
/// translate these into MOUSE_EVENT records, so crossterm's
/// ReadConsoleInputW-based reader never sees them as mouse events.  The raw
/// VT bytes leak through as KEY_EVENT records and end up echoed as garbled
/// text in the active pane.
///
/// The fix: use the same VT input parser as SSH sessions to properly decode
/// X10/SGR mouse sequences from stdin.
pub fn needs_vt_input() -> bool {
    is_ssh_session()
        || std::env::var("TERMINAL_EMULATOR")
            .map_or(false, |v| v.contains("JetBrains"))
}

/// Returns the Windows build number (e.g. 19045 for Win10 22H2, 22631 for
/// Win11 23H2).  Returns `None` on non-Windows or if the query fails.
#[cfg(windows)]
pub fn windows_build_number() -> Option<u32> {
    #[repr(C)]
    struct OSVERSIONINFOW {
        os_version_info_size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform_id: u32,
        sz_csd_version: [u16; 128],
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OSVERSIONINFOW) -> i32;
    }
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.os_version_info_size = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    let status = unsafe { RtlGetVersion(&mut info) };
    if status == 0 { Some(info.build) } else { None }
}

#[cfg(not(windows))]
pub fn windows_build_number() -> Option<u32> {
    None
}

/// Unified input source — abstracts over crossterm (local) and SSH VT (remote).
///
/// # Usage
/// ```ignore
/// let input = InputSource::new(is_ssh)?;
/// loop {
///     if let Some(evt) = input.read_timeout(Duration::from_millis(50))? {
///         match evt { /* … */ }
///     }
/// }
/// ```
pub enum InputSource {
    /// Local terminal — delegates to `crossterm::event`.
    Crossterm,
    /// SSH session on Windows — reads via a background thread + VT parser.
    #[cfg(windows)]
    Ssh {
        rx: std::sync::mpsc::Receiver<Event>,
    },
}

impl InputSource {
    /// Create a new input source.
    ///
    /// When `ssh == true` **and** running on Windows, spawns the SSH VT reader
    /// thread with raw console input.  Otherwise wraps `crossterm::event`
    /// with zero overhead.
    pub fn new(ssh: bool) -> io::Result<Self> {
        if !ssh {
            return Ok(InputSource::Crossterm);
        }

        #[cfg(windows)]
        {
            match start_ssh_reader() {
                Ok(rx) => Ok(InputSource::Ssh { rx }),
                Err(e) => {
                    // Log to file instead of stderr (raw mode garbles eprintln).
                    ssh_debug_log(&format!("SSH VT input init failed: {}; falling back to crossterm", e));
                    Ok(InputSource::Crossterm)
                }
            }
        }

        #[cfg(not(windows))]
        {
            // On Unix, crossterm already reads raw VT bytes and handles mouse.
            let _ = ssh;
            Ok(InputSource::Crossterm)
        }
    }

    /// Read one event, blocking up to `timeout`.  Returns `None` on timeout.
    #[inline]
    pub fn read_timeout(&self, timeout: Duration) -> io::Result<Option<Event>> {
        match self {
            InputSource::Crossterm => {
                if crossterm::event::poll(timeout)? {
                    Ok(Some(crossterm::event::read()?))
                } else {
                    Ok(None)
                }
            }
            #[cfg(windows)]
            InputSource::Ssh { rx } => match rx.recv_timeout(timeout) {
                Ok(evt) => Ok(Some(evt)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(None),
            },
        }
    }

    /// Try to read one event without blocking.
    #[inline]
    pub fn try_read(&self) -> io::Result<Option<Event>> {
        match self {
            InputSource::Crossterm => {
                if crossterm::event::poll(Duration::ZERO)? {
                    Ok(Some(crossterm::event::read()?))
                } else {
                    Ok(None)
                }
            }
            #[cfg(windows)]
            InputSource::Ssh { rx } => match rx.try_recv() {
                Ok(evt) => Ok(Some(evt)),
                Err(_) => Ok(None),
            },
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Construct a press `Event::Key` with the given code and modifiers.
#[inline(always)]
pub(crate) fn make_key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    })
}

/// Decode CSI modifier parameter (1 = none, 2 = Shift, 3 = Alt, …).
#[inline]
pub(crate) fn decode_modifiers(n: u16) -> KeyModifiers {
    let m = n.saturating_sub(1);
    let mut mods = KeyModifiers::empty();
    if m & 1 != 0 {
        mods |= KeyModifiers::SHIFT;
    }
    if m & 2 != 0 {
        mods |= KeyModifiers::ALT;
    }
    if m & 4 != 0 {
        mods |= KeyModifiers::CONTROL;
    }
    mods
}

/// Decode a UTF-16 code unit, combining surrogate pairs.
#[inline]
pub(crate) fn decode_utf16_unit(unit: u16, high_surrogate: &mut Option<u16>) -> Option<char> {
    if (0xD800..=0xDBFF).contains(&unit) {
        *high_surrogate = Some(unit);
        return None;
    }
    if (0xDC00..=0xDFFF).contains(&unit) {
        if let Some(hi) = high_surrogate.take() {
            let cp = 0x10000 + ((hi as u32 - 0xD800) << 10) + (unit as u32 - 0xDC00);
            return char::from_u32(cp);
        }
        return None; // orphan low surrogate
    }
    *high_surrogate = None;
    char::from_u32(unit as u32)
}

// ─── VT Input Parser ─────────────────────────────────────────────────────────
//
// Compact state machine that decodes a raw VT character stream into terminal
// events.  Handles SGR mouse, X10 mouse, CSI keyboard sequences, SS3 function
// keys, bracketed paste, Alt+key, plain characters, and control codes.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PS {
    Ground,
    Escape,     // received \x1b
    CsiEntry,   // received \x1b[
    CsiParam,   // accumulating CSI parameters
    X10Mouse,   // received \x1b[M — reading 3 raw bytes
    Ss3,        // received \x1bO
    Paste,      // inside \x1b[200~ … \x1b[201~
    PasteEsc,   // received \x1b inside paste
    PasteBrk,   // received \x1b[ inside paste
    PasteNum,   // accumulating digits inside paste CSI
    /// Post-paste-flush drain: absorbs residual close-sequence characters
    /// (especially `~`) after a paste timeout flush.  Transitions to Ground
    /// on the next non-residue character or timeout tick.
    PasteDrain,
    Osc,        // inside \x1b] … waiting for ST (\x07 or \x1b\\)
    OscEsc,     // received \x1b inside OSC — might be ST
}

pub(crate) struct VtParser {
    pub(crate) state: PS,
    /// CSI numeric parameters (semicolon-separated).
    pub(crate) params: [u16; 8],
    /// Index of the *next* parameter slot (i.e. number of completed params).
    pub(crate) pidx: u8,
    /// Accumulator for the current (incomplete) numeric parameter.
    pub(crate) cur: u16,
    /// True if at least one digit has been seen for the current param.
    pub(crate) has_digit: bool,
    /// Private-mode indicator character (`<` for SGR mouse, `?` for DEC).
    pub(crate) priv_ch: u8,
    /// X10 mouse — bytes received so far (0–2).
    pub(crate) x10_n: u8,
    pub(crate) x10_buf: [u8; 3],
    /// Bracketed-paste text accumulator.
    pub(crate) paste: String,
    /// Timestamp when the parser entered Paste state.  Used to detect a
    /// missing close sequence (`\x1b[201~`) and force-flush after a timeout
    /// so the terminal does not hang forever (issue #197).
    pub(crate) paste_start: Option<std::time::Instant>,
    /// Set to `true` when the parser transitions into Paste state.
    /// The reader thread checks this flag and re-verifies VTI (Virtual
    /// Terminal Input mode) is still enabled.  ConPTY or other processes
    /// can clear VTI, which causes the close sequence (`\x1b[201~`) to be
    /// interpreted as a CSI sequence instead of passed through as raw
    /// bytes, leading to a lost close marker and terminal hang.
    pub(crate) needs_vti_recheck: bool,
    /// OSC sequence accumulator (e.g. for OSC 52 clipboard responses).
    pub(crate) osc: String,
    /// Pending high surrogate for UTF-16 decoding.
    pub(crate) hi_sur: Option<u16>,
}
