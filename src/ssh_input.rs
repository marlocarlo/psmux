//! SSH VT Input — transparent mouse + keyboard support over SSH on Windows.
//!
//! ## Problem
//!
//! ConPTY does **not** translate VT mouse escape sequences (SGR `\x1b[<…M`,
//! X10 `\x1b[M…`) into native `MOUSE_EVENT` `INPUT_RECORD`s.  When psmux
//! runs over SSH, the remote terminal sends SGR mouse bytes through:
//!
//! ```text
//!   remote terminal → SSH client → sshd → ConPTY input pipe
//!     → ConPTY does NOT convert to MOUSE_EVENT
//!       → crossterm's ReadConsoleInputW never sees mouse events
//! ```
//!
//! ## Solution
//!
//! When an SSH session is detected, this module:
//!
//! 1. Configures the console stdin for raw input (no echo, no line edit,
//!    no Quick Edit) with `ENABLE_MOUSE_INPUT` and
//!    `ENABLE_VIRTUAL_TERMINAL_INPUT` (VTI).  VTI is **critical** — without
//!    it, ConPTY's input parser intercepts CSI sequences from the SSH data
//!    stream (including SGR mouse `\x1b[<…M`) and discards those it doesn't
//!    recognise.  With VTI, ConPTY passes raw bytes through as `KEY_EVENT`
//!    records with `u_char` set, which our VT parser reassembles.
//! 2. Spawns a dedicated reader thread that calls `ReadConsoleInputW` in a
//!    tight loop.
//! 3. Handles **two kinds** of `KEY_EVENT` records:
//!    - `u_char != 0` — character data (ConPTY passed unrecognised VT bytes
//!      through as individual characters).  Fed into a fast VT state-machine
//!      parser that decodes SGR/X10 mouse, CSI keyboard, SS3 function keys,
//!      bracketed paste, Alt+key, and plain characters.
//!    - `u_char == 0` — virtual-key events (ConPTY recognised the VT
//!      sequence and translated it, e.g. VK_UP for `\x1b[A`).  Mapped
//!      directly to `crossterm::event::Event` via VK-code lookup.
//! 4. Delivers events through a bounded `mpsc::sync_channel` — the client
//!    event loop reads via [`InputSource::read_timeout`] /
//!    [`InputSource::try_read`].
//!
//! Resize events (`WINDOW_BUFFER_SIZE_EVENT`) and native `MOUSE_EVENT`
//! records are forwarded directly.
//!
//! On non-Windows platforms (or when not under SSH), [`InputSource`] simply
//! delegates to `crossterm::event`.
//!
//! ## Debugging
//!
//! Set `PSMUX_SSH_DEBUG=1` to write a detailed trace of every INPUT_RECORD
//! and emitted event to `~/.psmux/ssh_input.log`.

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
fn make_key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    })
}

/// Decode CSI modifier parameter (1 = none, 2 = Shift, 3 = Alt, …).
#[inline]
fn decode_modifiers(n: u16) -> KeyModifiers {
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
fn decode_utf16_unit(unit: u16, high_surrogate: &mut Option<u16>) -> Option<char> {
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
enum PS {
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

struct VtParser {
    state: PS,
    /// CSI numeric parameters (semicolon-separated).
    params: [u16; 8],
    /// Index of the *next* parameter slot (i.e. number of completed params).
    pidx: u8,
    /// Accumulator for the current (incomplete) numeric parameter.
    cur: u16,
    /// True if at least one digit has been seen for the current param.
    has_digit: bool,
    /// Private-mode indicator character (`<` for SGR mouse, `?` for DEC).
    priv_ch: u8,
    /// X10 mouse — bytes received so far (0–2).
    x10_n: u8,
    x10_buf: [u8; 3],
    /// Bracketed-paste text accumulator.
    paste: String,
    /// Timestamp when the parser entered Paste state.  Used to detect a
    /// missing close sequence (`\x1b[201~`) and force-flush after a timeout
    /// so the terminal does not hang forever (issue #197).
    paste_start: Option<std::time::Instant>,
    /// Set to `true` when the parser transitions into Paste state.
    /// The reader thread checks this flag and re-verifies VTI (Virtual
    /// Terminal Input mode) is still enabled.  ConPTY or other processes
    /// can clear VTI, which causes the close sequence (`\x1b[201~`) to be
    /// interpreted as a CSI sequence instead of passed through as raw
    /// bytes, leading to a lost close marker and terminal hang.
    needs_vti_recheck: bool,
    /// OSC sequence accumulator (e.g. for OSC 52 clipboard responses).
    osc: String,
    /// Pending high surrogate for UTF-16 decoding.
    hi_sur: Option<u16>,
}

impl VtParser {
    fn new() -> Self {
        Self {
            state: PS::Ground,
            params: [0; 8],
            pidx: 0,
            cur: 0,
            has_digit: false,
            priv_ch: 0,
            x10_n: 0,
            x10_buf: [0; 3],
            paste: String::new(),
            paste_start: None,
            needs_vti_recheck: false,
            osc: String::new(),
            hi_sur: None,
        }
    }

    #[inline(always)]
    fn reset_csi(&mut self) {
        self.params = [0; 8];
        self.pidx = 0;
        self.cur = 0;
        self.has_digit = false;
        self.priv_ch = 0;
    }

    /// Feed one Unicode character into the parser, emitting events via `emit`.
    #[inline]
    fn feed<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match self.state {
            PS::Ground   => self.on_ground(ch, emit),
            PS::Escape   => self.on_escape(ch, emit),
            PS::CsiEntry => self.on_csi_entry(ch, emit),
            PS::CsiParam => self.on_csi_param(ch, emit),
            PS::X10Mouse => self.on_x10(ch, emit),
            PS::Ss3      => self.on_ss3(ch, emit),
            PS::Paste    => self.on_paste(ch, emit),
            PS::PasteEsc => self.on_paste_esc(ch, emit),
            PS::PasteBrk => self.on_paste_brk(ch, emit),
            PS::PasteNum => self.on_paste_num(ch, emit),
            PS::PasteDrain => self.on_paste_drain(ch, emit),
            PS::Osc      => self.on_osc(ch, emit),
            PS::OscEsc   => self.on_osc_esc(ch, emit),
        }
    }

    /// True when the parser holds a pending `\x1b` that might be a standalone
    /// Escape key or the start of a longer sequence.
    #[inline(always)]
    fn has_pending_escape(&self) -> bool {
        self.state == PS::Escape
    }

    /// Emit a standalone Escape key if the timeout expired mid-sequence.
    fn flush_escape<F: FnMut(Event)>(&mut self, emit: &mut F) {
        if self.state == PS::Escape {
            emit(make_key(KeyCode::Esc, KeyModifiers::empty()));
            self.state = PS::Ground;
        }
        // PasteDrain expires after a generous window (2 seconds) to absorb
        // any residual close-sequence characters that arrive late due to
        // SSH/ConPTY latency.  `paste_start` is reused as the drain
        // deadline timestamp.
        if self.state == PS::PasteDrain {
            let expired = match self.paste_start {
                Some(start) => start.elapsed().as_millis() >= 2000,
                None => true,
            };
            if expired {
                self.state = PS::Ground;
                self.paste_start = None;
            }
        }
    }

    /// Cancel a pending escape without emitting it.  Used when ConPTY has
    /// already consumed the ESC as part of a recognised VT sequence and
    /// delivered a VK event instead — the ESC in the parser is stale.
    fn cancel_escape(&mut self) {
        if self.state == PS::Escape {
            self.state = PS::Ground;
        }
    }

    /// True when the parser is inside a bracketed-paste sequence.
    #[inline(always)]
    fn is_in_paste(&self) -> bool {
        matches!(self.state, PS::Paste | PS::PasteEsc | PS::PasteBrk | PS::PasteNum)
    }

    /// Maximum paste buffer size (1 MB).  Prevents unbounded memory growth
    /// if the close sequence is never received.
    const PASTE_MAX_BYTES: usize = 1_048_576;

    /// Maximum time (in seconds) to stay in Paste state before force-flushing.
    /// If the `\x1b[201~` terminator is lost (e.g. ConPTY strips it, or sshd
    /// transforms it), this prevents the parser from being stuck forever,
    /// which would make the terminal completely unresponsive (issue #197).
    const PASTE_TIMEOUT_SECS: u64 = 2;

    /// Force-flush a stale paste if we have been in Paste state for too long
    /// or the buffer has exceeded the size limit.  Called on every timeout
    /// tick from the reader thread.
    fn flush_stale_paste<F: FnMut(Event)>(&mut self, emit: &mut F) {
        if !self.is_in_paste() { return; }

        let should_flush = if let Some(start) = self.paste_start {
            start.elapsed().as_secs() >= Self::PASTE_TIMEOUT_SECS
                || self.paste.len() >= Self::PASTE_MAX_BYTES
        } else {
            false
        };

        if should_flush {
            ssh_debug_log(&format!(
                "flush_stale_paste: forcing flush after {}ms, {} chars (state={:?})",
                self.paste_start.map(|s| s.elapsed().as_millis()).unwrap_or(0),
                self.paste.len(),
                self.state,
            ));
            // Save current state before flushing to determine the correct
            // transition for absorbing residual close-sequence characters.
            let pre_flush_state = self.state;
            let text = std::mem::take(&mut self.paste);
            if !text.is_empty() {
                emit(Event::Paste(text));
            }
            self.paste_start = None;
            // Transition to the appropriate state to absorb any remaining
            // characters of the close sequence (\x1b[201~) that may still
            // be in-flight.  Going directly to Ground would cause residual
            // characters (especially the trailing '~') to leak as visible
            // input (issue #197).
            match pre_flush_state {
                PS::Paste => {
                    // Close sequence hasn't started arriving through the
                    // VT parser.  However, ConPTY may have stripped the
                    // CSI prefix (\x1b[201) and only leaked the final `~`.
                    // Transition to PasteDrain to absorb that residue.
                    // Reuse paste_start as the drain deadline (500 ms window).
                    self.cur = 0;
                    self.paste_start = Some(std::time::Instant::now());
                    self.state = PS::PasteDrain;
                    ssh_debug_log(&format!(
                        "flush_stale_paste: transitioning to PasteDrain (pre={:?} post={:?})",
                        pre_flush_state, self.state,
                    ));
                }
                PS::PasteEsc => {
                    // Already consumed \x1b.  Transition to Escape so the
                    // remaining [201~ is processed as a normal CSI (which
                    // dispatch_tilde discards for param 201).
                    self.cur = 0;
                    self.state = PS::Escape;
                }
                PS::PasteBrk => {
                    // Consumed \x1b[.  Transition to CsiEntry.
                    self.reset_csi();
                    self.state = PS::CsiEntry;
                }
                PS::PasteNum => {
                    // Consumed \x1b[ plus digits (cur holds accumulated
                    // value).  Transition to CsiParam so the final ~
                    // dispatches via dispatch_tilde (which ignores 201).
                    let saved_cur = self.cur;
                    self.reset_csi();
                    self.cur = saved_cur;
                    self.has_digit = true;
                    self.state = PS::CsiParam;
                }
                _ => {
                    self.cur = 0;
                    self.state = PS::Ground;
                }
            }
        }
    }

    // ── Ground ───────────────────────────────────────────────────────────

    #[inline]
    fn on_ground<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '\x1b' => {
                self.state = PS::Escape;
            }
            '\r' | '\n' => emit(make_key(KeyCode::Enter, KeyModifiers::empty())),
            '\t' => emit(make_key(KeyCode::Tab, KeyModifiers::empty())),
            '\x7f' => emit(make_key(KeyCode::Backspace, KeyModifiers::empty())),
            '\x08' => emit(make_key(KeyCode::Backspace, KeyModifiers::empty())),
            '\0' => emit(make_key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            c if c as u32 >= 1 && (c as u32) <= 26 => {
                // Ctrl+A … Ctrl+Z
                let letter = (b'a' + (c as u8) - 1) as char;
                emit(make_key(KeyCode::Char(letter), KeyModifiers::CONTROL));
            }
            c if c as u32 == 28 => emit(make_key(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
            c if c as u32 == 29 => emit(make_key(KeyCode::Char(']'), KeyModifiers::CONTROL)),
            c if c as u32 == 30 => emit(make_key(KeyCode::Char('^'), KeyModifiers::CONTROL)),
            c if c as u32 == 31 => emit(make_key(KeyCode::Char('_'), KeyModifiers::CONTROL)),
            c => emit(make_key(KeyCode::Char(c), KeyModifiers::empty())),
        }
    }

    // ── Escape ───────────────────────────────────────────────────────────

    fn on_escape<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '[' => {
                self.reset_csi();
                self.state = PS::CsiEntry;
            }
            'O' => {
                self.state = PS::Ss3;
            }
            '\x1b' => {
                // Double-Esc → emit one Escape, stay in Escape state.
                emit(make_key(KeyCode::Esc, KeyModifiers::empty()));
            }
            ']' => {
                // OSC sequence start (\x1b])
                self.osc.clear();
                self.state = PS::Osc;
            }
            c if c >= ' ' && c <= '~' => {
                // Alt + printable character.
                emit(make_key(KeyCode::Char(c), KeyModifiers::ALT));
                self.state = PS::Ground;
            }
            c => {
                // Unknown after Esc — emit Esc then re-process char.
                emit(make_key(KeyCode::Esc, KeyModifiers::empty()));
                self.state = PS::Ground;
                self.on_ground(c, emit);
            }
        }
    }

    // ── CSI entry (\x1b[ received) ───────────────────────────────────────

    fn on_csi_entry<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '<' => {
                self.priv_ch = b'<';
                self.state = PS::CsiParam;
            }
            '?' => {
                self.priv_ch = b'?';
                self.state = PS::CsiParam;
            }
            '0'..='9' => {
                self.cur = (ch as u16) - (b'0' as u16);
                self.has_digit = true;
                self.state = PS::CsiParam;
            }
            ';' => {
                // Empty first param (implicitly 0).
                self.finish_param();
                self.state = PS::CsiParam;
            }
            'M' => {
                // X10 mouse: \x1b[M followed by 3 raw bytes.
                self.x10_n = 0;
                self.state = PS::X10Mouse;
            }
            // CSI with immediate final character (no params).
            c @ ('A'..='Z' | 'a'..='z' | '~') => {
                self.finish_param();
                self.dispatch_csi(c, emit);
                // dispatch_csi sets state (Ground or Paste).
            }
            '\x1b' => {
                // Abort — new escape sequence starting.
                self.state = PS::Escape;
            }
            _ => {
                // Unknown — discard and return to ground.
                self.state = PS::Ground;
            }
        }
    }

    // ── CSI parameter accumulation ───────────────────────────────────────

    fn on_csi_param<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '0'..='9' => {
                self.cur = self.cur.saturating_mul(10).saturating_add((ch as u16) - (b'0' as u16));
                self.has_digit = true;
            }
            ';' => {
                self.finish_param();
            }
            ':' => {
                // Sub-parameter separator (kitty protocol, etc.) — accumulate
                // like ';' for simplicity; sufficient for SGR mouse.
                self.finish_param();
            }
            c @ ('A'..='Z' | 'a'..='z' | '~') => {
                self.finish_param();
                self.dispatch_csi(c, emit);
                // dispatch_csi sets state (Ground or Paste).
            }
            '\x1b' => {
                self.state = PS::Escape;
            }
            _ => {
                // Unexpected intermediate byte — discard whole sequence.
                self.state = PS::Ground;
            }
        }
    }

    /// Push the current accumulator into the param array and reset.
    #[inline]
    fn finish_param(&mut self) {
        if (self.pidx as usize) < self.params.len() {
            self.params[self.pidx as usize] = self.cur;
            self.pidx += 1;
        }
        self.cur = 0;
        self.has_digit = false;
    }

    // ── CSI dispatch ─────────────────────────────────────────────────────

    /// Dispatch a complete CSI sequence.  Sets `self.state` to Ground (or
    /// Paste for `\x1b[200~`).
    fn dispatch_csi<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        // SGR mouse: \x1b[<Pb;Px;PyM/m
        if self.priv_ch == b'<' {
            self.dispatch_sgr_mouse(ch, emit);
            self.state = PS::Ground;
            return;
        }

        // DEC private-mode sequences (\x1b[?…) — ignore silently.
        if self.priv_ch == b'?' {
            self.state = PS::Ground;
            return;
        }

        // Bracketed paste start: \x1b[200~
        if ch == '~' && self.pidx >= 1 && self.params[0] == 200 {
            self.paste.clear();
            self.paste_start = Some(std::time::Instant::now());
            self.needs_vti_recheck = true;
            self.state = PS::Paste;
            return;
        }

        // Modifier — second param when present (e.g. \x1b[1;5A = Ctrl+Up).
        let mods = if self.pidx >= 2 {
            decode_modifiers(self.params[1])
        } else {
            KeyModifiers::empty()
        };

        match ch {
            'A' => emit(make_key(KeyCode::Up, mods)),
            'B' => emit(make_key(KeyCode::Down, mods)),
            'C' => emit(make_key(KeyCode::Right, mods)),
            'D' => emit(make_key(KeyCode::Left, mods)),
            'H' => emit(make_key(KeyCode::Home, mods)),
            'F' => emit(make_key(KeyCode::End, mods)),
            'P' => emit(make_key(KeyCode::F(1), mods)),
            'Q' => emit(make_key(KeyCode::F(2), mods)),
            'R' => emit(make_key(KeyCode::F(3), mods)),
            'S' => emit(make_key(KeyCode::F(4), mods)),
            'Z' => emit(make_key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            'I' if self.pidx <= 1 && self.params[0] == 0 => emit(Event::FocusGained),
            'O' if self.pidx <= 1 && self.params[0] == 0 => emit(Event::FocusLost),
            '~' => self.dispatch_tilde(mods, emit),
            _ => {} // Unknown — silently discard.
        }
        self.state = PS::Ground;
    }

    /// Dispatch CSI `~` (tilde) sequences: `\x1b[N~` or `\x1b[N;mod~`.
    fn dispatch_tilde<F: FnMut(Event)>(&self, mods: KeyModifiers, emit: &mut F) {
        let n = self.params[0];
        let code = match n {
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            11 => KeyCode::F(1),
            12 => KeyCode::F(2),
            13 => KeyCode::F(3),
            14 => KeyCode::F(4),
            15 => KeyCode::F(5),
            17 => KeyCode::F(6),
            18 => KeyCode::F(7),
            19 => KeyCode::F(8),
            20 => KeyCode::F(9),
            21 => KeyCode::F(10),
            23 => KeyCode::F(11),
            24 => KeyCode::F(12),
            _ => return,
        };
        emit(make_key(code, mods));
    }

    // ── SGR mouse ────────────────────────────────────────────────────────

    /// Decode SGR mouse: `\x1b[<Pb;Px;PyM` (press/drag) or `…m` (release).
    fn dispatch_sgr_mouse<F: FnMut(Event)>(&self, final_ch: char, emit: &mut F) {
        if self.pidx < 3 {
            return;
        }
        let pb = self.params[0];
        let px = self.params[1].saturating_sub(1); // → 0-based column
        let py = self.params[2].saturating_sub(1); // → 0-based row
        let is_release = final_ch == 'm';

        let btn_id    = pb & 0x03;
        let is_shift  = pb & 0x04 != 0;
        let is_alt    = pb & 0x08 != 0;
        let is_ctrl   = pb & 0x10 != 0;
        let is_motion = pb & 0x20 != 0;
        let is_scroll = pb & 0x40 != 0;

        let mut modifiers = KeyModifiers::empty();
        if is_shift { modifiers |= KeyModifiers::SHIFT; }
        if is_alt   { modifiers |= KeyModifiers::ALT; }
        if is_ctrl  { modifiers |= KeyModifiers::CONTROL; }

        let kind = if is_scroll {
            if btn_id == 0 {
                MouseEventKind::ScrollUp
            } else {
                MouseEventKind::ScrollDown
            }
        } else if is_release {
            let button = match btn_id {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::Left,
            };
            MouseEventKind::Up(button)
        } else if is_motion {
            if btn_id == 3 {
                MouseEventKind::Moved
            } else {
                let button = match btn_id {
                    0 => MouseButton::Left,
                    1 => MouseButton::Middle,
                    2 => MouseButton::Right,
                    _ => MouseButton::Left,
                };
                MouseEventKind::Drag(button)
            }
        } else {
            let button = match btn_id {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::Left,
            };
            MouseEventKind::Down(button)
        };

        emit(Event::Mouse(MouseEvent {
            kind,
            column: px,
            row: py,
            modifiers,
        }));
    }

    // ── X10 mouse ────────────────────────────────────────────────────────

    fn on_x10<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        let byte = (ch as u32).min(255) as u8;
        self.x10_buf[self.x10_n as usize] = byte;
        self.x10_n += 1;
        if self.x10_n < 3 {
            return;
        }
        // Got all 3 bytes: button, column+33, row+33.
        self.state = PS::Ground;
        let raw_btn = self.x10_buf[0].wrapping_sub(32);
        let col = self.x10_buf[1].wrapping_sub(33) as u16;
        let row = self.x10_buf[2].wrapping_sub(33) as u16;

        let btn_id    = raw_btn & 0x03;
        let is_motion = raw_btn & 0x20 != 0;
        let is_scroll = raw_btn & 0x40 != 0;

        let mut modifiers = KeyModifiers::empty();
        if raw_btn & 0x04 != 0 { modifiers |= KeyModifiers::SHIFT; }
        if raw_btn & 0x08 != 0 { modifiers |= KeyModifiers::ALT; }
        if raw_btn & 0x10 != 0 { modifiers |= KeyModifiers::CONTROL; }

        let kind = if is_scroll {
            if btn_id == 0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown }
        } else if is_motion {
            match btn_id {
                0 => MouseEventKind::Drag(MouseButton::Left),
                1 => MouseEventKind::Drag(MouseButton::Middle),
                2 => MouseEventKind::Drag(MouseButton::Right),
                _ => MouseEventKind::Moved,
            }
        } else if btn_id == 3 {
            // X10 "release" encoding.
            MouseEventKind::Up(MouseButton::Left)
        } else {
            let button = match btn_id {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::Left,
            };
            MouseEventKind::Down(button)
        };

        emit(Event::Mouse(MouseEvent { kind, column: col, row: row, modifiers }));
    }

    // ── SS3 (\x1bO) ─────────────────────────────────────────────────────

    fn on_ss3<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        self.state = PS::Ground;
        match ch {
            'A' => emit(make_key(KeyCode::Up, KeyModifiers::empty())),
            'B' => emit(make_key(KeyCode::Down, KeyModifiers::empty())),
            'C' => emit(make_key(KeyCode::Right, KeyModifiers::empty())),
            'D' => emit(make_key(KeyCode::Left, KeyModifiers::empty())),
            'H' => emit(make_key(KeyCode::Home, KeyModifiers::empty())),
            'F' => emit(make_key(KeyCode::End, KeyModifiers::empty())),
            'P' => emit(make_key(KeyCode::F(1), KeyModifiers::empty())),
            'Q' => emit(make_key(KeyCode::F(2), KeyModifiers::empty())),
            'R' => emit(make_key(KeyCode::F(3), KeyModifiers::empty())),
            'S' => emit(make_key(KeyCode::F(4), KeyModifiers::empty())),
            _ => {
                // Unknown SS3 — emit Alt+char as fallback.
                emit(make_key(KeyCode::Char(ch), KeyModifiers::ALT));
            }
        }
    }

    // ── Bracketed paste (\x1b[200~ … \x1b[201~) ─────────────────────────

    fn on_paste<F: FnMut(Event)>(&mut self, ch: char, _emit: &mut F) {
        if ch == '\x1b' {
            self.state = PS::PasteEsc;
        } else if self.paste.len() < Self::PASTE_MAX_BYTES {
            self.paste.push(ch);
        }
    }

    fn on_paste_esc<F: FnMut(Event)>(&mut self, ch: char, _emit: &mut F) {
        if ch == '[' {
            self.state = PS::PasteBrk;
        } else {
            self.paste.push('\x1b');
            self.paste.push(ch);
            self.state = PS::Paste;
        }
    }

    fn on_paste_brk<F: FnMut(Event)>(&mut self, ch: char, _emit: &mut F) {
        if ch.is_ascii_digit() {
            self.cur = (ch as u16) - (b'0' as u16);
            self.state = PS::PasteNum;
        } else {
            self.paste.push('\x1b');
            self.paste.push('[');
            self.paste.push(ch);
            self.state = PS::Paste;
        }
    }

    fn on_paste_num<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        if ch.is_ascii_digit() {
            self.cur = self.cur.saturating_mul(10).saturating_add((ch as u16) - (b'0' as u16));
        } else if ch == '~' && self.cur == 201 {
            // \x1b[201~ — paste end.
            let text = std::mem::take(&mut self.paste);
            self.paste_start = None;
            emit(Event::Paste(text));
            self.state = PS::Ground;
        } else {
            // Not the end marker — push partial escape into paste buffer.
            self.paste.push('\x1b');
            self.paste.push('[');
            let s = self.cur.to_string();
            self.paste.push_str(&s);
            self.paste.push(ch);
            self.cur = 0;
            self.state = PS::Paste;
        }
    }

    /// Post-paste-flush drain: absorbs residual close-sequence characters
    /// (`~`, `[`, digits, ESC) that may arrive after a paste timeout flush.
    /// ConPTY can strip the CSI prefix of `\x1b[201~` and leak only the
    /// final `~`, which would otherwise appear as a visible character.
    fn on_paste_drain<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '~' | '[' | '0'..='9' => {
                // Likely residue from a stripped close sequence — absorb.
                ssh_debug_log(&format!("PasteDrain: absorbing residue char {:?}", ch));
            }
            '\x1b' => {
                // ESC could start a new close sequence that ConPTY partially
                // passed through.  Transition to Escape to let the CSI
                // parser handle it (dispatch_tilde ignores param 201).
                self.paste_start = None;
                self.state = PS::Escape;
            }
            _ => {
                // Non-residue character: drain is done, process normally.
                self.paste_start = None;
                self.state = PS::Ground;
                self.on_ground(ch, emit);
            }
        }
    }

    // ── OSC (Operating System Command) ───────────────────────────────────
    //
    // Accumulates \x1b] ... ST where ST is \x07 (BEL) or \x1b\\.
    // Used to parse OSC 52 clipboard responses from the client terminal.

    fn on_osc<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '\x07' => {
                // ST (BEL) — dispatch OSC
                self.dispatch_osc(emit);
                self.state = PS::Ground;
            }
            '\x1b' => {
                // Possible start of ST (\x1b\\)
                self.state = PS::OscEsc;
            }
            c => {
                // Safety limit: 128 KB
                if self.osc.len() < 131072 {
                    self.osc.push(c);
                }
            }
        }
    }

    fn on_osc_esc<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        if ch == '\\' {
            // ST (\x1b\\) — dispatch OSC
            self.dispatch_osc(emit);
            self.state = PS::Ground;
        } else {
            // Not ST — abort OSC, re-process as new escape sequence
            self.osc.clear();
            self.state = PS::Escape;
            self.on_escape(ch, emit);
        }
    }

    fn dispatch_osc<F: FnMut(Event)>(&self, emit: &mut F) {
        // OSC 52 clipboard response: "52;<selection>;<base64data>"
        if let Some(rest) = self.osc.strip_prefix("52;") {
            if let Some(sc_idx) = rest.find(';') {
                let data = &rest[sc_idx + 1..];
                // Ignore queries ("?") and empty responses
                if data != "?" && !data.is_empty() {
                    if let Some(text) = crate::util::base64_decode(data) {
                        if !text.is_empty() {
                            emit(Event::Paste(text));
                        }
                    }
                }
            }
        }
        // All other OSC sequences silently discarded
    }
}

// ─── VK-code → KeyCode mapping (Windows Console API) ─────────────────────────

/// Map a Windows virtual-key code to a crossterm `KeyCode`.
/// Returns `None` for modifier-only keys (Ctrl, Shift, Alt, CapsLock, etc.)
/// and other keys we don't need to handle.
#[cfg(windows)]
fn vk_to_keycode(vk: u16) -> Option<KeyCode> {
    match vk {
        0x08 => Some(KeyCode::Backspace),   // VK_BACK
        0x09 => Some(KeyCode::Tab),         // VK_TAB
        0x0D => Some(KeyCode::Enter),       // VK_RETURN
        0x1B => Some(KeyCode::Esc),         // VK_ESCAPE
        0x20 => Some(KeyCode::Char(' ')),   // VK_SPACE
        0x21 => Some(KeyCode::PageUp),      // VK_PRIOR
        0x22 => Some(KeyCode::PageDown),    // VK_NEXT
        0x23 => Some(KeyCode::End),         // VK_END
        0x24 => Some(KeyCode::Home),        // VK_HOME
        0x25 => Some(KeyCode::Left),        // VK_LEFT
        0x26 => Some(KeyCode::Up),          // VK_UP
        0x27 => Some(KeyCode::Right),       // VK_RIGHT
        0x28 => Some(KeyCode::Down),        // VK_DOWN
        0x2D => Some(KeyCode::Insert),      // VK_INSERT
        0x2E => Some(KeyCode::Delete),      // VK_DELETE
        0x70 => Some(KeyCode::F(1)),        // VK_F1
        0x71 => Some(KeyCode::F(2)),
        0x72 => Some(KeyCode::F(3)),
        0x73 => Some(KeyCode::F(4)),
        0x74 => Some(KeyCode::F(5)),
        0x75 => Some(KeyCode::F(6)),
        0x76 => Some(KeyCode::F(7)),
        0x77 => Some(KeyCode::F(8)),
        0x78 => Some(KeyCode::F(9)),
        0x79 => Some(KeyCode::F(10)),
        0x7A => Some(KeyCode::F(11)),
        0x7B => Some(KeyCode::F(12)),       // VK_F12
        _ => None,
    }
}

/// Extract crossterm `KeyModifiers` from Win32 `dwControlKeyState`.
#[cfg(windows)]
fn vk_modifiers(state: u32) -> KeyModifiers {
    let mut m = KeyModifiers::empty();
    if state & 0x0010 != 0 { m |= KeyModifiers::SHIFT; }      // SHIFT_PRESSED
    if state & (0x0001 | 0x0002) != 0 { m |= KeyModifiers::ALT; }     // LEFT/RIGHT_ALT
    if state & (0x0004 | 0x0008) != 0 { m |= KeyModifiers::CONTROL; } // LEFT/RIGHT_CTRL
    m
}

// ─── Debug logging ───────────────────────────────────────────────────────────

/// Global log file shared across all threads (main + reader).
#[cfg(windows)]
static SSH_LOG: std::sync::LazyLock<std::sync::Mutex<Option<std::fs::File>>> =
    std::sync::LazyLock::new(|| {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        let dir = format!("{}/.psmux", home);
        let _ = std::fs::create_dir_all(&dir);
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(format!("{}/ssh_input.log", dir))
            .ok();
        std::sync::Mutex::new(f)
    });

/// Write a line to `~/.psmux/ssh_input.log`.  Always active in SSH mode;
/// set `PSMUX_SSH_DEBUG=1` for verbose per-event logging.
#[cfg(windows)]
fn ssh_debug_log(msg: &str) {
    use std::io::Write;
    if let Ok(mut guard) = SSH_LOG.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{}", msg);
            let _ = f.flush();
        }
    }
}

/// True when verbose per-event logging is enabled.
#[cfg(windows)]
fn ssh_verbose() -> bool {
    std::env::var("PSMUX_SSH_DEBUG").ok().as_deref() == Some("1")
}

// ─── Windows: SSH reader thread + Win32 FFI ──────────────────────────────────

#[cfg(windows)]
fn start_ssh_reader() -> io::Result<std::sync::mpsc::Receiver<Event>> {
    use std::ffi::c_void;
    use std::sync::mpsc;

    // ── Win32 constants ──────────────────────────────────────────────────
    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const ENABLE_WINDOW_INPUT: u32          = 0x0008;
    const ENABLE_MOUSE_INPUT: u32           = 0x0010;
    const ENABLE_EXTENDED_FLAGS: u32        = 0x0080;
    const ENABLE_LINE_INPUT: u32            = 0x0002;
    const ENABLE_ECHO_INPUT: u32            = 0x0004;
    const ENABLE_PROCESSED_INPUT: u32       = 0x0001;
    const ENABLE_QUICK_EDIT_MODE: u32       = 0x0040;

    const KEY_EVENT: u16                     = 0x0001;
    const MOUSE_EVENT: u16                   = 0x0002;
    const WINDOW_BUFFER_SIZE_EVENT: u16      = 0x0004;

    const WAIT_OBJECT_0: u32 = 0x00000000;
    const WAIT_TIMEOUT: u32  = 0x00000102;

    // ── Win32 structs ────────────────────────────────────────────────────

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct KEY_EVENT_RECORD {
        key_down: i32,
        repeat_count: u16,
        virtual_key_code: u16,
        virtual_scan_code: u16,
        u_char: u16,
        control_key_state: u32,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct MOUSE_EVENT_RECORD {
        mouse_x: i16,
        mouse_y: i16,
        button_state: u32,
        control_key_state: u32,
        event_flags: u32,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct WINDOW_BUFFER_SIZE_RECORD {
        size_x: i16,
        size_y: i16,
    }

    #[repr(C)]
    struct INPUT_RECORD {
        event_type: u16,
        _pad: u16,
        data: [u8; 16], // largest variant (KEY_EVENT_RECORD / MOUSE_EVENT_RECORD)
    }

    // ── Win32 imports ────────────────────────────────────────────────────

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
        fn ReadConsoleInputW(
            h: *mut c_void,
            buf: *mut INPUT_RECORD,
            len: u32,
            read: *mut u32,
        ) -> i32;
        fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    }

    // ── Native MOUSE_EVENT → crossterm Event conversion ──────────────────

    const FROM_LEFT_1ST: u32 = 0x0001;
    const RIGHTMOST: u32     = 0x0002;
    const FROM_LEFT_2ND: u32 = 0x0004;
    const ME_MOVED: u32      = 0x0001;
    const ME_WHEELED: u32    = 0x0004;

    fn convert_native_mouse(rec: &MOUSE_EVENT_RECORD) -> Option<Event> {
        let col = rec.mouse_x.max(0) as u16;
        let row = rec.mouse_y.max(0) as u16;
        let mods = {
            let s = rec.control_key_state;
            let mut m = KeyModifiers::empty();
            if s & 0x0010 != 0 { m |= KeyModifiers::SHIFT; } // SHIFT_PRESSED
            if s & (0x0001 | 0x0002) != 0 { m |= KeyModifiers::ALT; } // LEFT/RIGHT_ALT
            if s & (0x0004 | 0x0008) != 0 { m |= KeyModifiers::CONTROL; } // LEFT/RIGHT_CTRL
            m
        };

        if rec.event_flags & ME_WHEELED != 0 {
            let delta = (rec.button_state >> 16) as i16;
            let kind = if delta > 0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown };
            return Some(Event::Mouse(MouseEvent { kind, column: col, row, modifiers: mods }));
        }

        if rec.event_flags & ME_MOVED != 0 {
            if rec.button_state & FROM_LEFT_1ST != 0 {
                return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: col, row, modifiers: mods }));
            }
            if rec.button_state & RIGHTMOST != 0 {
                return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Right), column: col, row, modifiers: mods }));
            }
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Moved, column: col, row, modifiers: mods }));
        }

        if rec.button_state & FROM_LEFT_1ST != 0 {
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: col, row, modifiers: mods }));
        }
        if rec.button_state & RIGHTMOST != 0 {
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: col, row, modifiers: mods }));
        }
        if rec.button_state & FROM_LEFT_2ND != 0 {
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Middle), column: col, row, modifiers: mods }));
        }

        // button_state == 0  → all buttons released
        if rec.button_state == 0 && rec.event_flags == 0 {
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: col, row, modifiers: mods }));
        }

        None
    }

    // ── Setup + thread spawn ─────────────────────────────────────────────

    let (tx, rx) = mpsc::sync_channel::<Event>(1024);

    // ── Startup diagnostics ──────────────────────────────────────────────
    ssh_debug_log("=== psmux SSH input module starting ===");
    // Log Windows version
    {
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
        unsafe { RtlGetVersion(&mut info) };
        ssh_debug_log(&format!(
            "Windows {}.{} build {}",
            info.major, info.minor, info.build,
        ));
        // ConPTY mouse support requires Windows 11 build 22523+.
        // On older builds, ConPTY's VT parser discards SGR mouse input
        // sequences and does not forward DECSET to the SSH client.
        if info.build < 22523 {
            ssh_debug_log(&format!(
                "WARNING: Windows build {} < 22523 — ConPTY does NOT support \
                 mouse over SSH. Mouse clicks will not work. \
                 Upgrade to Windows 11 22H2+ for SSH mouse support.",
                info.build,
            ));
        } else {
            ssh_debug_log("ConPTY build >= 22523 — mouse over SSH should be supported");
        }
    }
    // Log SSH env vars
    for var in &["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
        if let Ok(val) = std::env::var(var) {
            ssh_debug_log(&format!("  {}={}", var, val));
        }
    }

    // Configure console stdin for VT input *before* spawning the thread so
    // any error is reported synchronously.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle.is_null() || handle == (-1isize) as *mut c_void {
        return Err(io::Error::new(io::ErrorKind::Other, "GetStdHandle(STDIN) failed"));
    }

    let mut orig_mode: u32 = 0;
    if unsafe { GetConsoleMode(handle, &mut orig_mode) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("GetConsoleMode failed (err {})", io::Error::last_os_error()),
        ));
    }

    // ENABLE_VIRTUAL_TERMINAL_INPUT (0x0200) is CRITICAL for SSH mouse.
    // Without it, ConPTY's input parser intercepts CSI sequences from the
    // SSH data stream (including SGR mouse \x1b[<…M) and discards those it
    // doesn't recognise.  With VTI, ConPTY passes raw bytes through as
    // KEY_EVENT records with u_char set, which our VT parser reassembles.
    //
    // This must run AFTER crossterm's enable_raw_mode() and
    // EnableMouseCapture so our SetConsoleMode has the final word.
    let new_mode = (orig_mode
        & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT | ENABLE_QUICK_EDIT_MODE))
        | ENABLE_VIRTUAL_TERMINAL_INPUT
        | ENABLE_WINDOW_INPUT
        | ENABLE_MOUSE_INPUT
        | ENABLE_EXTENDED_FLAGS;

    if unsafe { SetConsoleMode(handle, new_mode) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "SetConsoleMode(+VTI) failed (err {})",
                io::Error::last_os_error()
            ),
        ));
    }

    // Verify the mode actually stuck (some ConPTY implementations may
    // silently ignore VTI).
    let mut actual_mode: u32 = 0;
    if unsafe { GetConsoleMode(handle, &mut actual_mode) } != 0 {
        let vti_ok = actual_mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0;
        ssh_debug_log(&format!(
            "Console mode: orig=0x{:04X} requested=0x{:04X} actual=0x{:04X} VTI={}",
            orig_mode, new_mode, actual_mode, if vti_ok { "YES" } else { "NO" },
        ));
        if !vti_ok {
            ssh_debug_log("WARNING: VTI not set — ConPTY may swallow mouse sequences");
        }
    } else {
        ssh_debug_log("WARNING: re-read GetConsoleMode failed after SetConsoleMode");
    }

    // ── Spawn the reader thread ────────────────────────────────────────
    // The console handle is process-global and remains
    // valid for the entire process lifetime.  We pass it as usize (which is
    // Send) and cast back inside the thread.
    let handle_val = handle as usize;
    std::thread::Builder::new()
        .name("ssh-vt-input".into())
        .spawn(move || {
            let handle = handle_val as *mut c_void;
            let mut parser = VtParser::new();
            let mut records: Vec<INPUT_RECORD> = Vec::with_capacity(64);
            records.resize_with(64, || unsafe { std::mem::zeroed() });

            // Escape-timeout: 50 ms matches tmux's default.
            const ESC_TIMEOUT_MS: u32 = 50;

            let mut alive = true;
            let verbose = ssh_verbose();
            let mut total_records: u64 = 0;
            let mut key_char_count: u64 = 0;
            let mut key_vk_count: u64 = 0;
            let mut mouse_count: u64 = 0;
            let mut loop_count: u64 = 0;

            ssh_debug_log(&format!("Reader thread started (verbose={})", verbose));

            loop {
                loop_count += 1;
                // Dynamic timeout: short when the parser has a pending Esc
                // or is inside a paste (need to detect stale paste quickly).
                let wait_ms = if parser.has_pending_escape() {
                    ESC_TIMEOUT_MS
                } else if parser.is_in_paste() || parser.state == PS::PasteDrain {
                    200 // check paste timeout / drain expiry frequently
                } else {
                    500
                };
                let wait = unsafe { WaitForSingleObject(handle, wait_ms) };

                if wait == WAIT_TIMEOUT {
                    // Heartbeat every ~60 loops (≈30 s at 500 ms timeout)
                    if loop_count % 60 == 0 {
                        ssh_debug_log(&format!(
                            "heartbeat: loops={} records={} chars={} vk={} mouse={}",
                            loop_count, total_records, key_char_count, key_vk_count, mouse_count,
                        ));
                        // Verify VTI is still set — ConPTY or other processes can
                        // clear it, which silently breaks mouse input over SSH.
                        let mut cur_mode: u32 = 0;
                        if unsafe { GetConsoleMode(handle, &mut cur_mode) } != 0 {
                            if cur_mode & ENABLE_VIRTUAL_TERMINAL_INPUT == 0 {
                                ssh_debug_log("WARNING: VTI cleared! Re-enabling...");
                                let fixed = cur_mode | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_MOUSE_INPUT;
                                unsafe { SetConsoleMode(handle, fixed) };
                            }
                        }
                    }
                    // Flush pending Esc (if any) as a standalone keypress.
                    parser.flush_escape(&mut |evt| {
                        if tx.send(evt).is_err() { alive = false; }
                    });
                    // Flush stale paste if the close sequence never arrived
                    // (issue #197: prevents terminal from hanging forever).
                    parser.flush_stale_paste(&mut |evt| {
                        if tx.send(evt).is_err() { alive = false; }
                    });
                    if !alive { break; }
                    continue;
                }

                if wait != WAIT_OBJECT_0 {
                    break; // handle error / abandoned
                }

                let mut count: u32 = 0;
                let ok = unsafe {
                    ReadConsoleInputW(
                        handle,
                        records.as_mut_ptr(),
                        records.len() as u32,
                        &mut count,
                    )
                };
                if ok == 0 || count == 0 {
                    break;
                }

                for i in 0..count as usize {
                    let rec = &records[i];
                    total_records += 1;
                    match rec.event_type {
                        KEY_EVENT => {
                            let key = unsafe { &*(rec.data.as_ptr() as *const KEY_EVENT_RECORD) };
                            // Skip key-up events entirely.
                            if key.key_down == 0 { continue; }

                            if verbose {
                                ssh_debug_log(&format!(
                                    "KEY vk=0x{:04X} scan=0x{:04X} u_char=0x{:04X}({}) ctrl=0x{:08X}",
                                    key.virtual_key_code, key.virtual_scan_code,
                                    key.u_char, char::from_u32(key.u_char as u32).unwrap_or('.'),
                                    key.control_key_state,
                                ));
                            }

                            if key.u_char != 0 {
                                key_char_count += 1;
                                if let Some(ch) = decode_utf16_unit(key.u_char, &mut parser.hi_sur) {
                                    parser.feed(ch, &mut |evt| {
                                        if verbose {
                                            ssh_debug_log(&format!("  → emit(char): {:?}", evt));
                                        }
                                        // Always log mouse events (key diagnostic)
                                        if !verbose && matches!(evt, Event::Mouse(_)) {
                                            ssh_debug_log(&format!("MOUSE via VT parser: {:?}", evt));
                                        }
                                        if tx.send(evt).is_err() { alive = false; }
                                    });
                                }
                            } else {
                                key_vk_count += 1;
                                // When the parser is inside a bracketed-paste
                                // sequence, a VK_ESCAPE (u_char=0) must be fed
                                // to the VT parser as '\x1b' so the close-
                                // sequence detector can recognise \x1b[201~.
                                // ConPTY may deliver the ESC from the paste
                                // close marker as a VK event (bypassing the VT
                                // parser), which would leave the parser stuck
                                // in Paste state and cause the trailing '~' to
                                // leak as a visible character (issue #197).
                                if parser.is_in_paste() && key.virtual_key_code == 0x1B {
                                    if verbose {
                                        ssh_debug_log("  VK_ESCAPE in paste state → feeding \\x1b to parser");
                                    }
                                    parser.feed('\x1b', &mut |evt| {
                                        if verbose {
                                            ssh_debug_log(&format!("  → emit(paste-esc): {:?}", evt));
                                        }
                                        if tx.send(evt).is_err() { alive = false; }
                                    });
                                } else {
                                    parser.cancel_escape();

                                    let mods = vk_modifiers(key.control_key_state);
                                    if let Some(code) = vk_to_keycode(key.virtual_key_code) {
                                        let evt = make_key(code, mods);
                                        if verbose {
                                            ssh_debug_log(&format!("  → emit(vk): {:?}", evt));
                                        }
                                        if tx.send(evt).is_err() { alive = false; }
                                    }
                                }
                            }
                        }
                        WINDOW_BUFFER_SIZE_EVENT => {
                            let w = unsafe {
                                &*(rec.data.as_ptr() as *const WINDOW_BUFFER_SIZE_RECORD)
                            };
                            ssh_debug_log(&format!("RESIZE {}x{}", w.size_x, w.size_y));
                            let _ = tx.send(Event::Resize(w.size_x as u16, w.size_y as u16));
                        }
                        MOUSE_EVENT => {
                            mouse_count += 1;
                            let m = unsafe {
                                &*(rec.data.as_ptr() as *const MOUSE_EVENT_RECORD)
                            };
                            ssh_debug_log(&format!(
                                "NATIVE MOUSE ({},{}) btn=0x{:X} flags=0x{:X}",
                                m.mouse_x, m.mouse_y, m.button_state, m.event_flags,
                            ));
                            if let Some(evt) = convert_native_mouse(m) {
                                let _ = tx.send(evt);
                            }
                        }
                        other => {
                            if verbose {
                                ssh_debug_log(&format!("OTHER event_type={}", other));
                            }
                        }
                    }

                    if !alive { break; }
                }

                // After processing all records from this batch, flush any
                // pending escape if no more input is immediately available.
                if parser.has_pending_escape() {
                    let peek_wait = unsafe { WaitForSingleObject(handle, ESC_TIMEOUT_MS) };
                    if peek_wait == WAIT_TIMEOUT {
                        parser.flush_escape(&mut |evt| {
                            if tx.send(evt).is_err() { alive = false; }
                        });
                    }
                    // If WAIT_OBJECT_0 → more input arriving, continue loop
                    // and the escape will be resolved with the next batch.
                }

                // When the parser just entered Paste state, re-verify that
                // VTI is still enabled.  ConPTY or other processes can clear
                // it, which causes the close sequence (\x1b[201~) to be
                // interpreted as a CSI sequence instead of passed through
                // as raw bytes (issue #197).
                if parser.needs_vti_recheck {
                    parser.needs_vti_recheck = false;
                    let mut cur_mode: u32 = 0;
                    if unsafe { GetConsoleMode(handle, &mut cur_mode) } != 0 {
                        if cur_mode & ENABLE_VIRTUAL_TERMINAL_INPUT == 0 {
                            ssh_debug_log("VTI cleared at paste-start! Re-enabling...");
                            let fixed = cur_mode | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_MOUSE_INPUT;
                            unsafe { SetConsoleMode(handle, fixed) };
                        }
                    }
                }

                if !alive { break; }
            }
        })?;

    Ok(rx)
}

#[cfg(test)]
#[path = "../tests-rs/test_ssh_vt_paste.rs"]
mod tests;
