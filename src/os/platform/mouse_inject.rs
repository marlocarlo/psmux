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

#[cfg(windows)]
pub(crate) use std::ffi::c_void;

pub(crate) const GENERIC_READ: u32  = 0x80000000;
pub(crate) const GENERIC_WRITE: u32 = 0x40000000;
pub(crate) const FILE_SHARE_READ: u32  = 0x00000001;
pub(crate) const FILE_SHARE_WRITE: u32 = 0x00000002;
pub(crate) const OPEN_EXISTING: u32 = 3;
pub(crate) const INVALID_HANDLE: isize = -1;

const MOUSE_EVENT: u16 = 0x0002;
pub(crate) const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;

// dwButtonState flags
pub const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;
pub const RIGHTMOST_BUTTON_PRESSED: u32     = 0x0002;
pub const FROM_LEFT_2ND_BUTTON_PRESSED: u32 = 0x0004; // middle button

// dwEventFlags
pub const MOUSE_MOVED: u32       = 0x0001;
pub const MOUSE_WHEELED: u32     = 0x0004;

use std::sync::Mutex;
use std::time::{Duration, Instant};
static LAST_DRAG_INJECT: Mutex<Option<Instant>> = Mutex::new(None);
const DRAG_THROTTLE: Duration = Duration::from_millis(16); // ~60fps

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct COORD {
    pub(crate) x: i16,
    pub(crate) y: i16,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct MOUSE_EVENT_RECORD {
    pub(crate) mouse_position: COORD,
    pub(crate) button_state: u32,
    pub(crate) control_key_state: u32,
    pub(crate) event_flags: u32,
}

#[repr(C)]
pub(crate) struct INPUT_RECORD {
    pub(crate) event_type: u16,
    pub(crate) _padding: u16,
    pub(crate) event: MOUSE_EVENT_RECORD,
}

#[link(name = "kernel32")]
extern "system" {
    pub(crate) fn FreeConsole() -> i32;
    pub(crate) fn AttachConsole(process_id: u32) -> i32;
    pub(crate) fn GetConsoleWindow() -> isize;
    pub(crate) fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: *const c_void,
    ) -> isize;
    pub(crate) fn WriteConsoleInputW(
        console_input: isize,
        buffer: *const INPUT_RECORD,
        length: u32,
        events_written: *mut u32,
    ) -> i32;
    pub(crate) fn CloseHandle(handle: isize) -> i32;
    pub(crate) fn GetProcessId(process: isize) -> u32;
    pub(crate) fn GetLastError() -> u32;
}

/// Console input mode flags
pub(crate) const ENABLE_MOUSE_INPUT: u32         = 0x0010;
pub(crate) const ENABLE_EXTENDED_FLAGS: u32      = 0x0080;
pub(crate) const ENABLE_QUICK_EDIT_MODE: u32     = 0x0040;
pub(crate) const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

#[inline]
pub(crate) fn debug_log(msg: &str) {
    // Write to mouse_debug.log when PSMUX_MOUSE_DEBUG=1 is set.
    use std::sync::atomic::{AtomicBool, Ordering};
    static CHECKED: AtomicBool = AtomicBool::new(false);
    static ENABLED: AtomicBool = AtomicBool::new(false);

    if !CHECKED.swap(true, Ordering::Relaxed) {
        let on = std::env::var("PSMUX_MOUSE_DEBUG").map_or(false, |v| v == "1" || v == "true");
        ENABLED.store(on, Ordering::Relaxed);
    }
    if !ENABLED.load(Ordering::Relaxed) { return; }

    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let path = format!("{}/.psmux/mouse_debug.log", home);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "[platform] {}", msg);
    }
}

/// Extract the process ID from a portable_pty::Child trait object.
///
/// Uses the `Child::process_id()` trait method provided by portable-pty 0.9+.
pub fn get_child_pid(child: &dyn portable_pty::Child) -> Option<u32> {
    child.process_id()
}

/// Query whether the child process's console input has
/// ENABLE_VIRTUAL_TERMINAL_INPUT (0x0200) set.
///
/// When this flag is ON, the process uses VT-based input processing
/// (crossterm, ratatui apps).  VT mouse sequences written to the ConPTY
/// input pipe are passed through as KEY_EVENT records, and the app's VT
/// parser handles them.  If the flag is OFF (e.g. Node.js libuv raw mode
/// which sets only ENABLE_WINDOW_INPUT), VT mouse sequences should NOT
/// be written because the app cannot parse them and they appear as garbage.
pub fn query_vti_enabled(child_pid: u32) -> Option<bool> {
    unsafe {
        let had_console = GetConsoleWindow() != 0;
        FreeConsole();

        if AttachConsole(child_pid) == 0 {
            debug_log(&format!("query_vti_enabled: AttachConsole({}) FAILED", child_pid));
            if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
            return None;
        }

        let conin: [u16; 7] = [
            'C' as u16, 'O' as u16, 'N' as u16,
            'I' as u16, 'N' as u16, '$' as u16, 0,
        ];
        let handle = CreateFileW(
            conin.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null(),
        );

        if handle == INVALID_HANDLE || handle == 0 {
            debug_log("query_vti_enabled: CreateFileW(CONIN$) FAILED");
            FreeConsole();
            if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
            return None;
        }

        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
        }
        let mut mode: u32 = 0;
        let ok = GetConsoleMode(handle as *mut c_void, &mut mode);

        CloseHandle(handle);
        FreeConsole();
        if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }

        if ok == 0 {
            debug_log("query_vti_enabled: GetConsoleMode FAILED");
            return None;
        }

        let vti = (mode & ENABLE_VIRTUAL_TERMINAL_INPUT) != 0;
        debug_log(&format!("query_vti_enabled: pid={} mode=0x{:04X} VTI={}", child_pid, mode, vti));
        Some(vti)
    }
}

/// Inject a mouse event into a child process's console input buffer.
///
/// Performs the full cycle: FreeConsole → AttachConsole(pid) → open CONIN$
/// → WriteConsoleInputW → CloseHandle → FreeConsole.
///
/// Console handles are pseudo-handles that are invalidated by FreeConsole,
/// so we must do the entire cycle atomically for each event.
///
/// `reattach`: if true, re-attaches to original console after injection
/// (needed for app/standalone mode where crossterm uses the console).
/// Server mode should pass false to avoid conhost cycling.
pub fn send_mouse_event(
    child_pid: u32,
    col: i16,
    row: i16,
    button_state: u32,
    event_flags: u32,
    reattach: bool,
) -> bool {
    // Throttle drag events to ~60fps to avoid excessive console attach/detach cycling
    if event_flags & MOUSE_MOVED != 0 {
        if let Ok(mut guard) = LAST_DRAG_INJECT.lock() {
            if let Some(t) = *guard {
                if t.elapsed() < DRAG_THROTTLE {
                    return false;
                }
            }
            *guard = Some(Instant::now());
        }
    }

    unsafe {
        // Check if we currently own a console (app mode yes, server mode no after first call)
        let had_console = reattach && GetConsoleWindow() != 0;

        // Detach from current console (no-op if already detached)
        FreeConsole();

        // Attach to child's pseudo-console
        if AttachConsole(child_pid) == 0 {
            let err = GetLastError();
            debug_log(&format!("send_mouse_event: AttachConsole({}) FAILED err={}", child_pid, err));
            if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
            return false;
        }

        // Open the console input buffer
        let conin: [u16; 7] = [
            'C' as u16, 'O' as u16, 'N' as u16,
            'I' as u16, 'N' as u16, '$' as u16, 0,
        ];
        let handle = CreateFileW(
            conin.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null(),
        );

        if handle == INVALID_HANDLE || handle == 0 {
            let err = GetLastError();
            debug_log(&format!("send_mouse_event: CreateFileW(CONIN$) FAILED err={}", err));
            FreeConsole();
            if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
            return false;
        }

        // Temporarily ensure ENABLE_MOUSE_INPUT is set on the console so
        // mouse events are delivered to the foreground process.  Save and
        // restore original mode to prevent polluting the child's console
        // state (which would confuse query_mouse_input_enabled).
        {
            // Re-use the top-level GetConsoleMode/SetConsoleMode declarations
            // (they use *mut c_void for the handle parameter).
            #[link(name = "kernel32")]
            extern "system" {
                fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
                fn SetConsoleMode(hConsoleHandle: *mut c_void, dwMode: u32) -> i32;
            }
            let mut mode: u32 = 0;
            let h = handle as *mut c_void;
            if GetConsoleMode(h, &mut mode) != 0 {
                let desired = (mode | ENABLE_MOUSE_INPUT | ENABLE_EXTENDED_FLAGS)
                              & !ENABLE_QUICK_EDIT_MODE;
                if desired != mode {
                    SetConsoleMode(h, desired);
                }
            }
        }

        // Write the mouse event
        let record = INPUT_RECORD {
            event_type: MOUSE_EVENT,
            _padding: 0,
            event: MOUSE_EVENT_RECORD {
                mouse_position: COORD { x: col, y: row },
                button_state,
                control_key_state: 0,
                event_flags,
            },
        };
        let mut written: u32 = 0;
        let result = WriteConsoleInputW(handle, &record, 1, &mut written);
        let write_err = GetLastError();

        debug_log(&format!("send_mouse_event: pid={} ({},{}) btn=0x{:X} flags=0x{:X} => ok={} written={} err={}",
            child_pid, col, row, button_state, event_flags, result, written, write_err));

        // Clean up: close handle, detach from child's console
        CloseHandle(handle);
        FreeConsole();
        // Only re-attach if we had our own console (app/standalone mode)
        // Server mode: leave detached to avoid conhost cycling
        if had_console {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }

        result != 0
    }
}

/// Query whether the child process's console input has
/// ENABLE_MOUSE_INPUT (0x0010) set.
///
/// When this flag is ON, the child uses ReadConsoleInputW to read
/// MOUSE_EVENT INPUT_RECORDs (crossterm/ratatui apps).  When OFF, the
/// child reads input as text (ReadConsole/ReadFile) and expects VT
/// mouse sequences delivered as KEY_EVENT records (nvim, vim).
pub fn query_mouse_input_enabled(child_pid: u32) -> Option<bool> {
    unsafe {
        let had_console = GetConsoleWindow() != 0;
        FreeConsole();

        if AttachConsole(child_pid) == 0 {
            debug_log(&format!("query_mouse_input_enabled: AttachConsole({}) FAILED", child_pid));
            if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
            return None;
        }

        let conin: [u16; 7] = [
            'C' as u16, 'O' as u16, 'N' as u16,
            'I' as u16, 'N' as u16, '$' as u16, 0,
        ];
        let handle = CreateFileW(
            conin.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null(),
        );

        if handle == INVALID_HANDLE || handle == 0 {
            debug_log("query_mouse_input_enabled: CreateFileW(CONIN$) FAILED");
            FreeConsole();
            if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
            return None;
        }

        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
        }
        let mut mode: u32 = 0;
        let ok = GetConsoleMode(handle as *mut c_void, &mut mode);

        CloseHandle(handle);
        FreeConsole();
        if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }

        if ok == 0 {
            debug_log("query_mouse_input_enabled: GetConsoleMode FAILED");
            return None;
        }

        let mouse_input = (mode & ENABLE_MOUSE_INPUT) != 0;
        debug_log(&format!("query_mouse_input_enabled: pid={} mode=0x{:04X} ENABLE_MOUSE_INPUT={}", child_pid, mode, mouse_input));
        Some(mouse_input)
    }
}

/// Convenience: inject Alt+key event.
#[cfg(windows)]
pub fn send_alt_key_event(child_pid: u32, ch: char) -> bool {
    send_modified_key_event(child_pid, ch, false, true, false)
}
