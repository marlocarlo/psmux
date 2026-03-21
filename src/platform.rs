/// Spawn a server process with a hidden console window on Windows.
///
/// Uses raw `CreateProcessW` with `STARTF_USESHOWWINDOW` + `SW_HIDE` and
/// `CREATE_NEW_CONSOLE` so that ConPTY has a real console session while the
/// window remains invisible.  This replicates the behaviour of
/// `Start-Process -WindowStyle Hidden` in PowerShell.
#[cfg(windows)]
pub fn spawn_server_hidden(exe: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct STARTUPINFOW {
        cb: u32,
        lpReserved: *mut u16,
        lpDesktop: *mut u16,
        lpTitle: *mut u16,
        dwX: u32,
        dwY: u32,
        dwXSize: u32,
        dwYSize: u32,
        dwXCountChars: u32,
        dwYCountChars: u32,
        dwFillAttribute: u32,
        dwFlags: u32,
        wShowWindow: u16,
        cbReserved2: u16,
        lpReserved2: *mut u8,
        hStdInput: isize,
        hStdOutput: isize,
        hStdError: isize,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESS_INFORMATION {
        hProcess: isize,
        hThread: isize,
        dwProcessId: u32,
        dwThreadId: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateProcessW(
            lpApplicationName: *const u16,
            lpCommandLine: *mut u16,
            lpProcessAttributes: *const std::ffi::c_void,
            lpThreadAttributes: *const std::ffi::c_void,
            bInheritHandles: i32,
            dwCreationFlags: u32,
            lpEnvironment: *const std::ffi::c_void,
            lpCurrentDirectory: *const u16,
            lpStartupInfo: *const STARTUPINFOW,
            lpProcessInformation: *mut PROCESS_INFORMATION,
        ) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    const STARTF_USESHOWWINDOW: u32 = 0x00000001;
    const SW_HIDE: u16 = 0;
    const CREATE_NEW_CONSOLE: u32 = 0x00000010;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

    // Build command line: "exe" arg1 arg2 ...
    // Each argument is quoted to handle spaces.
    let mut cmdline = format!("\"{}\"", exe.display());
    for arg in args {
        if arg.contains(' ') || arg.contains('"') {
            cmdline.push_str(&format!(" \"{}\"", arg.replace('"', "\\\"")));
        } else {
            cmdline.push(' ');
            cmdline.push_str(arg);
        }
    }
    let mut cmdline_wide: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE;

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmdline_wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0, // don't inherit handles
            CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        )
    };

    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Close handles – we don't need to wait for the child.
    unsafe {
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }

    Ok(())
}

/// Enable virtual terminal processing on Windows Console Host.
/// This is required for ANSI color codes to work in conhost.exe (legacy console).
#[cfg(windows)]
pub fn enable_virtual_terminal_processing() {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    const CP_UTF8: u32 = 65001;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, dwMode: u32) -> i32;
        fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
        fn SetConsoleCP(wCodePageID: u32) -> i32;
    }

    unsafe {
        // Set console code page to UTF-8 so multi-byte Unicode characters
        // (e.g. ▶ U+25B6, ◀ U+25C0) render correctly instead of as mojibake.
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);

        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if !handle.is_null() {
            let mut mode: u32 = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn enable_virtual_terminal_processing() {
    // No-op on non-Windows platforms
}

/// Clear `ENABLE_VIRTUAL_TERMINAL_INPUT` (VTI, 0x0200) from the console stdin.
///
/// crossterm 0.28's `enable_raw_mode()` sets VTI.  When psmux runs inside a
/// ConPTY-based terminal (e.g. WezTerm), VTI tells conhost to pass VT bytes
/// through as raw KEY_EVENT records instead of properly translating them to
/// INPUT_RECORDs with virtual-key codes.  This breaks crossterm's event parser
/// because it expects translated INPUT_RECORDs for regular key events.
///
/// For local (non-SSH) sessions, we do not need VTI — crossterm reads native
/// INPUT_RECORDs via `ReadConsoleInputW`.  The SSH input path has its OWN
/// `SetConsoleMode(+VTI)` call, so this only runs for local mode.
///
/// Windows Terminal is unaffected because it IS the console host (no ConPTY
/// pipe translation).  The fix specifically helps ConPTY-hosted terminals.
#[cfg(windows)]
pub fn disable_vti_on_stdin() {
    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, dwMode: u32) -> i32;
    }

    unsafe {
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        if handle.is_null() || handle == (-1isize) as *mut std::ffi::c_void {
            return;
        }
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            let had_vti = mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0;
            crate::debug_log::input_log("console", &format!(
                "stdin mode before: 0x{:04X} VTI={}", mode, had_vti
            ));
            if had_vti {
                let new_mode = mode & !ENABLE_VIRTUAL_TERMINAL_INPUT;
                SetConsoleMode(handle, new_mode);
                crate::debug_log::input_log("console", &format!(
                    "stdin mode after: 0x{:04X} (VTI cleared)", new_mode
                ));
            }
        }
    }
}

#[cfg(not(windows))]
pub fn disable_vti_on_stdin() {
    // No-op on non-Windows platforms
}

/// Install a console control handler on Windows to prevent termination on client detach.
#[cfg(windows)]
pub fn install_console_ctrl_handler() {
    type HandlerRoutine = unsafe extern "system" fn(u32) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<HandlerRoutine>, add: i32) -> i32;
    }

    const CTRL_CLOSE_EVENT: u32 = 2;
    const CTRL_LOGOFF_EVENT: u32 = 5;
    const CTRL_SHUTDOWN_EVENT: u32 = 6;

    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        match ctrl_type {
            CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => 1,
            _ => 0,
        }
    }

    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(windows))]
pub fn install_console_ctrl_handler() {
    // No-op on non-Windows platforms
}

// ---------------------------------------------------------------------------
// Windows Console API mouse injection
// ---------------------------------------------------------------------------
// ConPTY does NOT translate VT mouse escape sequences (e.g. SGR \x1b[<0;10;5M)
// into MOUSE_EVENT INPUT_RECORDs. Writing them to the PTY master appears as
// garbage text in the child app.
//
// The solution: use WriteConsoleInput to inject native MOUSE_EVENT records
// directly into the child's console input buffer.
//
// Flow:
//   1. On first mouse event targeting a pane, lazily acquire the console handle:
//      FreeConsole() → AttachConsole(child_pid) → CreateFileW("CONIN$") → FreeConsole()
//   2. The handle remains valid after FreeConsole on modern Windows (real kernel handles).
//   3. Use WriteConsoleInputW(handle, MOUSE_EVENT record) for each mouse event.
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod mouse_inject {
    use std::ffi::c_void;

    const GENERIC_READ: u32  = 0x80000000;
    const GENERIC_WRITE: u32 = 0x40000000;
    const FILE_SHARE_READ: u32  = 0x00000001;
    const FILE_SHARE_WRITE: u32 = 0x00000002;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE: isize = -1;

    const MOUSE_EVENT: u16 = 0x0002;
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;

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
    struct COORD {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct MOUSE_EVENT_RECORD {
        mouse_position: COORD,
        button_state: u32,
        control_key_state: u32,
        event_flags: u32,
    }

    #[repr(C)]
    struct INPUT_RECORD {
        event_type: u16,
        _padding: u16,
        event: MOUSE_EVENT_RECORD,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn FreeConsole() -> i32;
        fn AttachConsole(process_id: u32) -> i32;
        fn GetConsoleWindow() -> isize;
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *const c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *const c_void,
        ) -> isize;
        fn WriteConsoleInputW(
            console_input: isize,
            buffer: *const INPUT_RECORD,
            length: u32,
            events_written: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetProcessId(process: isize) -> u32;
        fn GetLastError() -> u32;
    }

    /// Console input mode flags
    const ENABLE_MOUSE_INPUT: u32         = 0x0010;
    const ENABLE_EXTENDED_FLAGS: u32      = 0x0080;
    const ENABLE_QUICK_EDIT_MODE: u32     = 0x0040;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

    #[inline]
    fn debug_log(msg: &str) {
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

    /// Inject a VT escape sequence into a child process's console input buffer
    /// as a series of KEY_EVENT records.
    ///
    /// This bypasses ConPTY's VT input parser entirely — the raw characters of
    /// the escape sequence are delivered directly to the foreground process
    /// (e.g. wsl.exe) as keyboard input.  wsl.exe forwards them to the Linux
    /// PTY, where the terminal application (e.g. htop) interprets them as
    /// mouse events.
    ///
    /// This is more reliable than writing to the PTY master pipe because
    /// ConPTY's input engine may not correctly handle SGR mouse sequences
    /// written to hInput.
    pub fn send_vt_sequence(child_pid: u32, sequence: &[u8]) -> bool {
        unsafe {
            let had_console = GetConsoleWindow() != 0;
            FreeConsole();

            if AttachConsole(child_pid) == 0 {
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

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
                FreeConsole();
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

            // Save original console mode, temporarily set VTI for injection,
            // then restore after writing.  This prevents mode pollution which
            // would confuse the query_mouse_input_enabled() heuristic used to
            // distinguish console-API apps (crossterm) from VT apps (nvim).
            #[link(name = "kernel32")]
            extern "system" {
                fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
                fn SetConsoleMode(hConsoleHandle: *mut c_void, dwMode: u32) -> i32;
            }
            let h = handle as *mut c_void;
            let mut original_mode: u32 = 0;
            let got_mode = GetConsoleMode(h, &mut original_mode) != 0;
            if got_mode {
                let desired = (original_mode | ENABLE_EXTENDED_FLAGS | 0x0200 /*ENABLE_VIRTUAL_TERMINAL_INPUT*/)
                              & !ENABLE_QUICK_EDIT_MODE;
                if desired != original_mode {
                    SetConsoleMode(h, desired);
                }
            }

            // Build KEY_EVENT records for each byte of the VT sequence.
            // Each record is a "key down" event with the character set.
            const KEY_EVENT: u16 = 0x0001;

            #[repr(C)]
            #[derive(Copy, Clone)]
            struct KEY_EVENT_RECORD {
                key_down: i32,
                repeat_count: u16,
                virtual_key_code: u16,
                virtual_scan_code: u16,
                u_char: u16,       // UnicodeChar
                control_key_state: u32,
            }

            #[repr(C)]
            struct KEY_INPUT_RECORD {
                event_type: u16,
                _padding: u16,
                event: KEY_EVENT_RECORD,
            }

            // Build the array of input records
            let mut records: Vec<KEY_INPUT_RECORD> = Vec::with_capacity(sequence.len());
            for &byte in sequence {
                records.push(KEY_INPUT_RECORD {
                    event_type: KEY_EVENT,
                    _padding: 0,
                    event: KEY_EVENT_RECORD {
                        key_down: 1,
                        repeat_count: 1,
                        virtual_key_code: 0,
                        virtual_scan_code: 0,
                        u_char: byte as u16,
                        control_key_state: 0,
                    },
                });
            }

            let mut written: u32 = 0;
            let result = WriteConsoleInputW(
                handle,
                records.as_ptr() as *const INPUT_RECORD,
                records.len() as u32,
                &mut written,
            );

            // Restore original console mode to prevent pollution
            if got_mode {
                SetConsoleMode(h, original_mode);
            }

            CloseHandle(handle);
            FreeConsole();
            if had_console {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }

            result != 0
        }
    }

    /// Inject bracketed paste text into a child process's console input buffer.
    ///
    /// Sends `\x1b[200~` + text + `\x1b[201~` as KEY_EVENT records via
    /// WriteConsoleInputW, bypassing ConPTY's VT input parser entirely.
    /// ConPTY strips bracketed paste sequences written to the PTY master pipe,
    /// so this direct injection is the only way to deliver them to the child.
    ///
    /// The text is encoded as UTF-16 for proper Unicode support (file paths
    /// may contain non-ASCII characters).
    pub fn send_bracketed_paste(child_pid: u32, text: &str, bracket: bool) -> bool {
        unsafe {
            let had_console = GetConsoleWindow() != 0;
            FreeConsole();

            if AttachConsole(child_pid) == 0 {
                let err = GetLastError();
                debug_log(&format!("send_bracketed_paste: AttachConsole({}) FAILED err={}", child_pid, err));
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

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
                debug_log(&format!("send_bracketed_paste: CreateFileW(CONIN$) FAILED err={}", err));
                FreeConsole();
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

            const KEY_EVENT: u16 = 0x0001;

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
            struct KEY_INPUT_RECORD {
                event_type: u16,
                _padding: u16,
                event: KEY_EVENT_RECORD,
            }

            // Build bracket-open, text, bracket-close as UTF-16 chars
            let bracket_open: &[u8] = b"\x1b[200~";
            let bracket_close: &[u8] = b"\x1b[201~";

            // Collect all UTF-16 code units to send
            let mut chars: Vec<u16> = Vec::new();
            if bracket {
                for &b in bracket_open {
                    chars.push(b as u16);
                }
            }
            // Encode paste text as UTF-16, normalizing \n → \r for the
            // console input buffer (Windows apps expect CR for line breaks;
            // PSReadLine and other readline implementations treat \r as Enter).
            let mut prev_cr = false;
            for c in text.chars() {
                if c == '\n' {
                    if !prev_cr {
                        // Bare \n → \r
                        chars.push('\r' as u16);
                    }
                    // If preceded by \r, the \r was already pushed; skip this \n
                    prev_cr = false;
                    continue;
                }
                prev_cr = c == '\r';
                let mut buf = [0u16; 2];
                let encoded = c.encode_utf16(&mut buf);
                for &unit in encoded.iter() {
                    chars.push(unit);
                }
            }
            if bracket {
                for &b in bracket_close {
                    chars.push(b as u16);
                }
            }

            // Build KEY_EVENT records (key-down only; key-up not needed for
            // console input injection — only key-down events carry characters).
            let mut records: Vec<KEY_INPUT_RECORD> = Vec::with_capacity(chars.len());
            for &wch in &chars {
                records.push(KEY_INPUT_RECORD {
                    event_type: KEY_EVENT,
                    _padding: 0,
                    event: KEY_EVENT_RECORD {
                        key_down: 1,
                        repeat_count: 1,
                        virtual_key_code: 0,
                        virtual_scan_code: 0,
                        u_char: wch,
                        control_key_state: 0,
                    },
                });
            }

            // WriteConsoleInputW can perform partial writes (returns fewer
            // records than requested).  Retry in a loop so that large pastes
            // are delivered in full; without this the closing bracket sequence
            // can be silently dropped, breaking bracket paste mode in the
            // child application.
            //
            // For very large pastes, the console input buffer may fill up.
            // We limit each write to CHUNK_SIZE records and yield briefly
            // between chunks to let the consumer (PSReadLine etc.) drain.
            const CHUNK_SIZE: usize = 2048;
            let mut offset: usize = 0;
            let mut last_result: i32 = 1;
            while offset < records.len() {
                let mut written: u32 = 0;
                let remaining = (records.len() - offset).min(CHUNK_SIZE);
                last_result = WriteConsoleInputW(
                    handle,
                    records[offset..].as_ptr() as *const INPUT_RECORD,
                    remaining as u32,
                    &mut written,
                );
                if last_result == 0 || written == 0 {
                    // Brief yield and retry once (buffer may temporarily be full)
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    last_result = WriteConsoleInputW(
                        handle,
                        records[offset..].as_ptr() as *const INPUT_RECORD,
                        remaining as u32,
                        &mut written,
                    );
                    if last_result == 0 || written == 0 {
                        break;
                    }
                }
                offset += written as usize;
                // Yield between chunks to let the consumer drain the buffer
                if offset < records.len() && remaining >= CHUNK_SIZE {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }

            debug_log(&format!("send_bracketed_paste: pid={} bracket={} text_len={} records={} written={} ok={}",
                child_pid, bracket, text.len(), records.len(), offset, last_result != 0));

            CloseHandle(handle);
            FreeConsole();
            if had_console {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }

            last_result != 0 && offset == records.len()
        }
    }

    /// Send a CTRL_C_EVENT to all processes on the child's console.
    ///
    /// TUI applications (pstop, btop, etc.) often disable ENABLE_PROCESSED_INPUT
    /// on the ConPTY console and fail to restore it on exit.  When this flag is
    /// off, writing 0x03 to the ConPTY input pipe no longer generates a
    /// CTRL_C_EVENT signal — the byte is delivered as a regular key event that
    /// most programs ignore.
    ///
    /// This function works around the issue by:
    ///   1. Attaching to the child's hidden ConPTY console
    ///   2. Re-enabling ENABLE_PROCESSED_INPUT if it was cleared
    ///   3. Calling GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)
    ///
    /// The combination ensures Ctrl+C always delivers a signal regardless of
    /// what a previous TUI application did to the console mode.
    pub fn send_ctrl_c_event(child_pid: u32, reattach: bool) -> bool {
        const CTRL_C_EVENT: u32 = 0;
        const ENABLE_PROCESSED_INPUT: u32 = 0x0001;

        type HandlerRoutine = unsafe extern "system" fn(u32) -> i32;

        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<HandlerRoutine>,
                add: i32,
            ) -> i32;
            fn GenerateConsoleCtrlEvent(
                ctrl_event: u32,
                process_group_id: u32,
            ) -> i32;
            fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
            fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
        }

        // Always log to file for Ctrl+C events (critical signal path).
        fn log(msg: &str) {
            debug_log(&format!("ctrl_c: {}", msg));
        }

        unsafe {
            let had_console = reattach && GetConsoleWindow() != 0;

            FreeConsole();

            log(&format!("called: pid={} reattach={} had_console={}", child_pid, reattach, had_console));

            if AttachConsole(child_pid) == 0 {
                let err = GetLastError();
                log(&format!("AttachConsole({}) FAILED err={}", child_pid, err));
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

            // Open the console input buffer to check / fix ENABLE_PROCESSED_INPUT
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

            if handle != INVALID_HANDLE && handle != 0 {
                let mut mode: u32 = 0;
                if GetConsoleMode(handle as *mut c_void, &mut mode) != 0 {
                    log(&format!("console mode=0x{:04X} PROCESSED_INPUT={}", mode, mode & ENABLE_PROCESSED_INPUT != 0));
                    if mode & ENABLE_PROCESSED_INPUT == 0 {
                        log(&format!("re-enabling ENABLE_PROCESSED_INPUT for pid={}", child_pid));
                        SetConsoleMode(handle as *mut c_void, mode | ENABLE_PROCESSED_INPUT);
                    }
                }
                CloseHandle(handle);
            }

            // Ignore CTRL_C in our own process so GenerateConsoleCtrlEvent
            // doesn't kill psmux (we're temporarily on the child's console).
            // Passing None as handler with add=1 tells the system to ignore
            // Ctrl+C signals in this process.
            SetConsoleCtrlHandler(None, 1);

            let ok = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            let err = GetLastError();

            log(&format!("GenerateConsoleCtrlEvent => ok={} err={}", ok, err));

            // Detach from the child's console BEFORE restoring Ctrl+C handling.
            // GenerateConsoleCtrlEvent dispatches asynchronously via a new thread;
            // if we restore the default handler while still attached, the async
            // handler thread might terminate psmux.  Detaching first ensures the
            // event only targets processes that remain on the console.
            FreeConsole();

            // Brief sleep to let the async CTRL_C_EVENT handler thread finish
            // before we re-enable default handling.
            std::thread::sleep(std::time::Duration::from_millis(5));

            // Restore default Ctrl+C handling now that we're detached
            SetConsoleCtrlHandler(None, 0);

            if had_console {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }

            ok != 0
        }
    }

    /// Inject a modified key event into a child process's console input buffer.
    ///
    /// Uses WriteConsoleInputW with the appropriate control_key_state flags
    /// (LEFT_CTRL_PRESSED, LEFT_ALT_PRESSED, SHIFT_PRESSED) matching how
    /// Windows Terminal synthesises input events.
    ///
    /// This is necessary because ConPTY does NOT reassemble ESC+char into
    /// native Alt+key events — PSReadLine and other console apps receive
    /// them as separate key events.  Similarly, Ctrl+Alt+key written as
    /// ESC + control-char is not reassembled.
    ///
    /// For Ctrl+key: `u_char` = control character (ch & 0x1F), matching the
    /// Windows console convention.  For Alt+key: `u_char` = the plain char.
    /// For Ctrl+Alt: `u_char` = control character.
    ///
    /// Sends both key-down and key-up events for proper event pairing.
    ///
    /// Convenience wrapper: `send_alt_key_event` calls this with ctrl=false, alt=true, shift=false.
    pub fn send_modified_key_event(child_pid: u32, ch: char, ctrl: bool, alt: bool, shift: bool) -> bool {
        unsafe {
            let had_console = GetConsoleWindow() != 0;
            FreeConsole();

            if AttachConsole(child_pid) == 0 {
                debug_log(&format!("send_modified_key_event: AttachConsole({}) FAILED", child_pid));
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

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
                debug_log(&format!("send_modified_key_event: CreateFileW(CONIN$) FAILED"));
                FreeConsole();
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

            const KEY_EVENT: u16 = 0x0001;
            const LEFT_ALT_PRESSED: u32 = 0x0002;
            const LEFT_CTRL_PRESSED: u32 = 0x0008;
            const SHIFT_PRESSED: u32 = 0x0010;

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
            struct KEY_INPUT_RECORD {
                event_type: u16,
                _padding: u16,
                event: KEY_EVENT_RECORD,
            }

            #[link(name = "user32")]
            extern "system" {
                fn VkKeyScanW(ch: u16) -> i16;
                fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
            }

            // Build control_key_state flags (matching Windows Terminal convention)
            let mut flags: u32 = 0;
            if ctrl { flags |= LEFT_CTRL_PRESSED; }
            if alt  { flags |= LEFT_ALT_PRESSED; }
            if shift { flags |= SHIFT_PRESSED; }

            // Determine the character to send:
            // - Ctrl+key: u_char = control character (ch & 0x1F)
            // - Alt+key: u_char = plain character
            // - Ctrl+Alt+key: u_char = control character
            // - Shift+key: u_char = uppercase/shifted character
            let base_char = if shift && !ctrl {
                ch.to_ascii_uppercase()
            } else {
                ch
            };

            let u_char_value: u16 = if ctrl {
                // Control character: letter & 0x1F
                (base_char.to_ascii_lowercase() as u16) & 0x1F
            } else {
                let mut buf = [0u16; 2];
                let encoded = base_char.encode_utf16(&mut buf);
                encoded[0]
            };

            // VK code is always the unmodified letter key
            let mut buf = [0u16; 2];
            let plain_wch = ch.to_ascii_lowercase().encode_utf16(&mut buf)[0];
            let vk_result = VkKeyScanW(plain_wch);
            let vk = if vk_result == -1 { 0u16 } else { (vk_result & 0xFF) as u16 };

            // MAPVK_VK_TO_VSC = 0
            let scan = MapVirtualKeyW(vk as u32, 0) as u16;

            let records = [
                KEY_INPUT_RECORD {
                    event_type: KEY_EVENT,
                    _padding: 0,
                    event: KEY_EVENT_RECORD {
                        key_down: 1,
                        repeat_count: 1,
                        virtual_key_code: vk,
                        virtual_scan_code: scan,
                        u_char: u_char_value,
                        control_key_state: flags,
                    },
                },
                KEY_INPUT_RECORD {
                    event_type: KEY_EVENT,
                    _padding: 0,
                    event: KEY_EVENT_RECORD {
                        key_down: 0,
                        repeat_count: 1,
                        virtual_key_code: vk,
                        virtual_scan_code: scan,
                        u_char: u_char_value,
                        control_key_state: flags,
                    },
                },
            ];

            let mut written: u32 = 0;
            let result = WriteConsoleInputW(
                handle,
                records.as_ptr() as *const INPUT_RECORD,
                2,
                &mut written,
            );

            debug_log(&format!("send_modified_key_event: pid={} char='{}' ctrl={} alt={} shift={} vk=0x{:02X} scan=0x{:02X} u_char=0x{:04X} flags=0x{:04X} => ok={} written={}",
                child_pid, ch, ctrl, alt, shift, vk, scan, u_char_value, flags, result != 0, written));

            CloseHandle(handle);
            FreeConsole();
            if had_console {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }

            result != 0 && written >= 1
        }
    }

    /// Convenience: inject Alt+key event.
    pub fn send_alt_key_event(child_pid: u32, ch: char) -> bool {
        send_modified_key_event(child_pid, ch, false, true, false)
    }

    /// Inject a modified Enter (VK_RETURN) event via WriteConsoleInputW.
    ///
    /// ConPTY cannot reconstruct Shift+Enter from VT sequences (\x1b\r is
    /// misinterpreted as Alt+Enter).  Native injection delivers the exact
    /// KEY_EVENT_RECORD with the correct modifier flags, so PSReadLine and
    /// other console-API-based readers see the true Shift/Ctrl/Alt+Enter.
    pub fn send_modified_enter_event(child_pid: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
        unsafe {
            let had_console = GetConsoleWindow() != 0;
            FreeConsole();

            if AttachConsole(child_pid) == 0 {
                debug_log(&format!("send_modified_enter_event: AttachConsole({}) FAILED", child_pid));
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

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
                debug_log(&format!("send_modified_enter_event: CreateFileW(CONIN$) FAILED"));
                FreeConsole();
                if had_console { AttachConsole(ATTACH_PARENT_PROCESS); }
                return false;
            }

            const KEY_EVENT: u16 = 0x0001;
            const LEFT_ALT_PRESSED: u32 = 0x0002;
            const LEFT_CTRL_PRESSED: u32 = 0x0008;
            const SHIFT_PRESSED: u32 = 0x0010;
            const VK_RETURN: u16 = 0x0D;

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
            struct KEY_INPUT_RECORD {
                event_type: u16,
                _padding: u16,
                event: KEY_EVENT_RECORD,
            }

            #[link(name = "user32")]
            extern "system" {
                fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
            }

            let mut flags: u32 = 0;
            if ctrl  { flags |= LEFT_CTRL_PRESSED; }
            if alt   { flags |= LEFT_ALT_PRESSED; }
            if shift { flags |= SHIFT_PRESSED; }

            // MAPVK_VK_TO_VSC = 0
            let scan = MapVirtualKeyW(VK_RETURN as u32, 0) as u16;

            let records = [
                KEY_INPUT_RECORD {
                    event_type: KEY_EVENT,
                    _padding: 0,
                    event: KEY_EVENT_RECORD {
                        key_down: 1,
                        repeat_count: 1,
                        virtual_key_code: VK_RETURN,
                        virtual_scan_code: scan,
                        u_char: '\r' as u16,
                        control_key_state: flags,
                    },
                },
                KEY_INPUT_RECORD {
                    event_type: KEY_EVENT,
                    _padding: 0,
                    event: KEY_EVENT_RECORD {
                        key_down: 0,
                        repeat_count: 1,
                        virtual_key_code: VK_RETURN,
                        virtual_scan_code: scan,
                        u_char: '\r' as u16,
                        control_key_state: flags,
                    },
                },
            ];

            let mut written: u32 = 0;
            let result = WriteConsoleInputW(
                handle,
                records.as_ptr() as *const INPUT_RECORD,
                2,
                &mut written,
            );

            debug_log(&format!("send_modified_enter_event: pid={} ctrl={} alt={} shift={} scan=0x{:02X} flags=0x{:04X} => ok={} written={}",
                child_pid, ctrl, alt, shift, scan, flags, result != 0, written));

            CloseHandle(handle);
            FreeConsole();
            if had_console {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }

            result != 0 && written >= 1
        }
    }
}

#[cfg(not(windows))]
pub mod mouse_inject {
    pub fn get_child_pid(_child: &dyn portable_pty::Child) -> Option<u32> { None }
    pub fn send_mouse_event(_pid: u32, _col: i16, _row: i16, _btn: u32, _flags: u32, _reattach: bool) -> bool { false }
    pub fn send_vt_sequence(_pid: u32, _sequence: &[u8]) -> bool { false }
    pub fn query_vti_enabled(_pid: u32) -> Option<bool> { None }
    pub fn send_ctrl_c_event(_pid: u32, _reattach: bool) -> bool { false }
    pub fn query_mouse_input_enabled(_pid: u32) -> Option<bool> { None }
    pub fn send_bracketed_paste(_pid: u32, _text: &str, _bracket: bool) -> bool { false }
    pub fn send_modified_key_event(_pid: u32, _ch: char, _ctrl: bool, _alt: bool, _shift: bool) -> bool { false }
    pub fn send_alt_key_event(_pid: u32, _ch: char) -> bool { false }
    pub fn send_modified_enter_event(_pid: u32, _ctrl: bool, _alt: bool, _shift: bool) -> bool { false }
}

// ---------------------------------------------------------------------------
// Process tree killing — ensures all descendant processes are terminated
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod process_kill {
    const TH32CS_SNAPPROCESS: u32 = 0x00000002;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const INVALID_HANDLE: isize = -1;

    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> isize;
        fn Process32FirstW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> isize;
        fn TerminateProcess(h_process: isize, exit_code: u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    /// Collect all descendant PIDs of `root_pid` (children, grandchildren, etc.).
    /// Uses a breadth-first traversal of the process tree snapshot.
    fn collect_descendants(root_pid: u32) -> Vec<u32> {
        let mut descendants = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE || snap == 0 { return descendants; }

            // Build full process table from snapshot
            let mut entries: Vec<(u32, u32)> = Vec::with_capacity(256); // (pid, parent_pid)
            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe) != 0 {
                entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                while Process32NextW(snap, &mut pe) != 0 {
                    entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                }
            }
            CloseHandle(snap);

            // BFS from root_pid
            let mut queue: Vec<u32> = vec![root_pid];
            let mut head = 0;
            while head < queue.len() {
                let parent = queue[head];
                head += 1;
                for &(pid, ppid) in &entries {
                    if ppid == parent && pid != root_pid && !queue.contains(&pid) {
                        queue.push(pid);
                        descendants.push(pid);
                    }
                }
            }
        }
        descendants
    }

    /// Force-terminate a single process by PID.
    fn terminate_pid(pid: u32) {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION, 0, pid);
            if h != 0 && h != INVALID_HANDLE {
                let _ = TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }

    /// Kill an entire process tree: all descendants first (leaves → root order),
    /// then the root process itself.  Calls `child.kill()` via portable_pty as a
    /// fallback.  Does NOT call `child.wait()` so `try_wait()` still works for
    /// the reaper (`prune_exited`), which will detect the dead process and clean
    /// up the tree node.
    ///
    /// This mirrors how tmux on Linux sends SIGKILL to the pane's process group.
    pub fn kill_process_tree(child: &mut Box<dyn portable_pty::Child>) {
        // Try to get the PID
        let pid = super::mouse_inject::get_child_pid(child.as_ref());

        if let Some(root_pid) = pid {
            // Collect all descendants, kill them leaf-first (reverse order)
            let mut descs = collect_descendants(root_pid);
            descs.reverse();
            for &dpid in &descs {
                terminate_pid(dpid);
            }
            // Kill the root process
            terminate_pid(root_pid);
        }

        // Fallback: tell portable_pty to kill the direct child process.
        // Do NOT call child.wait() here — the reaper (prune_exited) needs
        // try_wait() to detect the dead process and remove the tree node.
        let _ = child.kill();
    }

    /// Kill multiple process trees using a SINGLE process snapshot.
    /// Much faster than calling `kill_process_tree` N times when
    /// killing an entire session (avoids N separate system snapshots).
    pub fn kill_process_trees_batch(children: &mut [&mut Box<dyn portable_pty::Child>]) {
        // Collect all root PIDs
        let root_pids: Vec<Option<u32>> = children.iter()
            .map(|c| super::mouse_inject::get_child_pid(c.as_ref()))
            .collect();

        // Take ONE process snapshot for all trees
        let entries = snapshot_process_table();

        // For each root PID, find descendants using the shared snapshot
        for (i, root_pid_opt) in root_pids.iter().enumerate() {
            if let Some(root_pid) = root_pid_opt {
                let mut descs = collect_descendants_from_table(&entries, *root_pid);
                descs.reverse();
                for &dpid in &descs {
                    terminate_pid(dpid);
                }
                terminate_pid(*root_pid);
            }
            let _ = children[i].kill();
        }
    }

    /// Take a system-wide process snapshot and return the process table.
    fn snapshot_process_table() -> Vec<(u32, u32)> {
        let mut entries: Vec<(u32, u32)> = Vec::with_capacity(256);
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE || snap == 0 { return entries; }

            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe) != 0 {
                entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                while Process32NextW(snap, &mut pe) != 0 {
                    entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                }
            }
            CloseHandle(snap);
        }
        entries
    }

    /// BFS from root_pid using a pre-built process table.
    fn collect_descendants_from_table(entries: &[(u32, u32)], root_pid: u32) -> Vec<u32> {
        let mut descendants = Vec::new();
        let mut queue: Vec<u32> = vec![root_pid];
        let mut head = 0;
        while head < queue.len() {
            let parent = queue[head];
            head += 1;
            for &(pid, ppid) in entries {
                if ppid == parent && pid != root_pid && !queue.contains(&pid) {
                    queue.push(pid);
                    descendants.push(pid);
                }
            }
        }
        descendants
    }
}

#[cfg(not(windows))]
pub mod process_kill {
    /// On non-Windows, fall back to simple kill (no wait — let the reaper handle it).
    pub fn kill_process_tree(child: &mut Box<dyn portable_pty::Child>) {
        let _ = child.kill();
    }

    /// Batch kill — on non-Windows, just kill each child individually.
    pub fn kill_process_trees_batch(children: &mut [&mut Box<dyn portable_pty::Child>]) {
        for child in children.iter_mut() {
            let _ = child.kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Process info queries — get CWD and process name from PID (for format vars)
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod process_info {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const PROCESS_VM_READ: u32 = 0x0010;
    const MAX_PATH: usize = 260;
    const TH32CS_SNAPPROCESS: u32 = 0x00000002;
    const INVALID_HANDLE: isize = -1;

    #[allow(non_snake_case)]
    #[repr(C)]
    struct PROCESS_BASIC_INFORMATION {
        Reserved1: isize,
        PebBaseAddress: isize, // pointer to PEB
        Reserved2: [isize; 2],
        UniqueProcessId: isize,
        Reserved3: isize,
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct UNICODE_STRING {
        Length: u16,
        MaximumLength: u16,
        Buffer: isize, // pointer to wide string
    }

    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn QueryFullProcessImageNameW(h: isize, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn ReadProcessMemory(
            h_process: isize,
            base_address: isize,
            buffer: *mut u8,
            size: usize,
            bytes_read: *mut usize,
        ) -> i32;
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> isize;
        fn Process32FirstW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process_handle: isize,
            process_information_class: u32,
            process_information: *mut u8,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    /// Get the executable name of a process by PID (e.g. "pwsh" or "vim").
    pub fn get_process_name(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h == 0 || h == -1 { return None; }
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size);
            CloseHandle(h);
            if ok == 0 { return None; }
            let full_path = OsString::from_wide(&buf[..size as usize])
                .to_string_lossy()
                .into_owned();
            let name = std::path::Path::new(&full_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())?;
            Some(name)
        }
    }

    /// Get the current working directory of a process by PID.
    /// Reads the PEB → ProcessParameters → CurrentDirectory from the target process.
    pub fn get_process_cwd(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if h == 0 || h == -1 { return None; }
            let result = read_process_cwd(h);
            CloseHandle(h);
            result
        }
    }

    /// Read CWD from a process handle via NtQueryInformationProcess + ReadProcessMemory.
    unsafe fn read_process_cwd(h: isize) -> Option<String> {
        // Step 1: Get PEB address
        let mut pbi: PROCESS_BASIC_INFORMATION = std::mem::zeroed();
        let mut ret_len: u32 = 0;
        let status = NtQueryInformationProcess(
            h,
            0, // ProcessBasicInformation
            &mut pbi as *mut _ as *mut u8,
            std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            &mut ret_len,
        );
        if status != 0 { return None; }
        let peb_addr = pbi.PebBaseAddress;
        if peb_addr == 0 { return None; }

        // Step 2: Read ProcessParameters pointer from PEB.
        // PEB layout (x64): offset 0x20 = ProcessParameters pointer
        // PEB layout (x86): offset 0x10 = ProcessParameters pointer
        let params_ptr_offset = if std::mem::size_of::<usize>() == 8 { 0x20 } else { 0x10 };
        let mut process_params_ptr: isize = 0;
        let mut bytes_read: usize = 0;
        let ok = ReadProcessMemory(
            h,
            peb_addr + params_ptr_offset,
            &mut process_params_ptr as *mut isize as *mut u8,
            std::mem::size_of::<isize>(),
            &mut bytes_read,
        );
        if ok == 0 || process_params_ptr == 0 { return None; }

        // Step 3: Read CurrentDirectory.DosPath (UNICODE_STRING) from RTL_USER_PROCESS_PARAMETERS.
        // x64 offset: 0x38 = CurrentDirectory.DosPath
        // x86 offset: 0x24 = CurrentDirectory.DosPath
        let cwd_offset = if std::mem::size_of::<usize>() == 8 { 0x38 } else { 0x24 };
        let mut cwd_ustr: UNICODE_STRING = std::mem::zeroed();
        let ok = ReadProcessMemory(
            h,
            process_params_ptr + cwd_offset,
            &mut cwd_ustr as *mut UNICODE_STRING as *mut u8,
            std::mem::size_of::<UNICODE_STRING>(),
            &mut bytes_read,
        );
        if ok == 0 || cwd_ustr.Length == 0 || cwd_ustr.Buffer == 0 { return None; }

        // Step 4: Read the actual CWD wide string
        let char_count = (cwd_ustr.Length / 2) as usize;
        let mut wchars: Vec<u16> = vec![0u16; char_count];
        let ok = ReadProcessMemory(
            h,
            cwd_ustr.Buffer,
            wchars.as_mut_ptr() as *mut u8,
            cwd_ustr.Length as usize,
            &mut bytes_read,
        );
        if ok == 0 { return None; }

        let path = OsString::from_wide(&wchars)
            .to_string_lossy()
            .into_owned();
        // Remove trailing backslash (tmux convention)
        Some(path.trim_end_matches('\\').to_string())
    }

    /// Append a line to ~/.psmux/autorename.log (first 100 entries only).
    fn autorename_log(msg: &str) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        if n > 100 { return; }
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let path = format!("{}/.psmux/autorename.log", home);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S%.3f"), msg);
        }
    }

    /// Get the name of the foreground process in the pane.
    /// Walks the process tree from the shell PID to find the deepest
    /// non-system descendant (the user's foreground command).
    pub fn get_foreground_process_name(pid: u32) -> Option<String> {
        // Walk the process tree to find the foreground child.
        let result = find_foreground_child_pid(pid);
        match result {
            Some(target) if target != pid => {
                let name = get_process_name(target);
                autorename_log(&format!("pid={} fg_child={} name={:?}", pid, target, name));
                if let Some(n) = name {
                    return Some(n);
                }
            }
            Some(_) => {
                autorename_log(&format!("pid={} fg_child=self (no children)", pid));
            }
            None => {
                autorename_log(&format!("pid={} fg_child=None (BFS found nothing)", pid));
            }
        }
        // Fallback: shell's own process name.
        let shell_name = get_process_name(pid);
        autorename_log(&format!("pid={} fallback_name={:?}", pid, shell_name));
        shell_name
    }

    /// Get the CWD of the foreground process in the pane.
    pub fn get_foreground_cwd(pid: u32) -> Option<String> {
        if let Some(target) = find_foreground_child_pid(pid) {
            if target != pid {
                if let Some(cwd) = get_process_cwd(target) {
                    return Some(cwd);
                }
            }
        }
        get_process_cwd(pid)
    }

    /// Known system/infrastructure processes that should be skipped when
    /// walking the process tree to find the user's foreground command.
    fn is_system_exe(name: &str) -> bool {
        matches!(name,
            "conhost.exe" | "csrss.exe" | "dwm.exe" | "services.exe"
            | "svchost.exe" | "wininit.exe" | "winlogon.exe"
            | "openconsole.exe" | "runtimebroker.exe"
        )
    }

    /// Walk the process tree from `root_pid` downward and return the PID of
    /// the process most likely to be the user's foreground command.
    ///
    /// Strategy: BFS all descendants, then pick the deepest non-system leaf.
    /// When multiple candidates exist at the same depth, prefer the largest
    /// PID (heuristic for "most recently created").
    fn find_foreground_child_pid(root_pid: u32) -> Option<u32> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE || snap == 0 {
                autorename_log(&format!("root={} SNAPSHOT FAILED", root_pid));
                return None;
            }

            // Collect (pid, ppid, exe_name_lower) for every process.
            let mut entries: Vec<(u32, u32, String)> = Vec::with_capacity(512);
            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe) != 0 {
                let name = exe_name_from_entry(&pe);
                entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
                while Process32NextW(snap, &mut pe) != 0 {
                    let name = exe_name_from_entry(&pe);
                    entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
                }
            }
            CloseHandle(snap);

            autorename_log(&format!("root={} snapshot_entries={}", root_pid, entries.len()));

            // Log direct children of root_pid
            let direct: Vec<_> = entries.iter()
                .filter(|(_, ppid, _)| *ppid == root_pid)
                .collect();
            for (pid, _, name) in &direct {
                autorename_log(&format!("  direct_child: pid={} name={}", pid, name));
            }

            // BFS: collect all descendants with their depth.
            // Each entry is (pid, exe_name, depth).
            let mut descendants: Vec<(u32, String, u32)> = Vec::new();
            let mut queue: Vec<(u32, u32)> = vec![(root_pid, 0)]; // (pid, depth)
            let mut head = 0;
            while head < queue.len() {
                let (parent, depth) = queue[head];
                head += 1;
                for (pid, ppid, name) in &entries {
                    if *ppid == parent && *pid != root_pid
                        && !descendants.iter().any(|(p, _, _)| p == pid)
                    {
                        descendants.push((*pid, name.clone(), depth + 1));
                        queue.push((*pid, depth + 1));
                    }
                }
            }

            autorename_log(&format!("root={} descendants={}", root_pid, descendants.len()));
            for (pid, name, depth) in &descendants {
                autorename_log(&format!("  desc: pid={} name={} depth={}", pid, name, depth));
            }

            if descendants.is_empty() {
                return None;
            }

            // A "leaf" is a descendant that has no children in our descendant set.
            let desc_pids: std::collections::HashSet<u32> =
                descendants.iter().map(|(p, _, _)| *p).collect();
            let leaves: Vec<(u32, &str, u32)> = descendants.iter()
                .filter(|(pid, _, _)| {
                    // No entry in the process table has this pid as parent
                    // while also being in our descendant set.
                    !entries.iter().any(|(ep, eppid, _)| *eppid == *pid && desc_pids.contains(ep))
                })
                .map(|(pid, name, depth)| (*pid, name.as_str(), *depth))
                .collect();

            // Choose from leaves if available, otherwise from all descendants.
            let pool: Vec<(u32, &str, u32)> = if !leaves.is_empty() {
                leaves
            } else {
                descendants.iter().map(|(p, n, d)| (*p, n.as_str(), *d)).collect()
            };

            // Prefer non-system candidates.
            let user_pool: Vec<&(u32, &str, u32)> = pool.iter()
                .filter(|(_, name, _)| !is_system_exe(name))
                .collect();

            let selection = if !user_pool.is_empty() { user_pool } else { pool.iter().collect() };

            // Deepest first, then largest PID as tiebreaker.
            let result = selection.iter()
                .max_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)))
                .map(|(pid, _, _)| *pid);

            autorename_log(&format!("root={} selected={:?}", root_pid, result));
            result
        }
    }

    /// Extract the lowercased executable name from a PROCESSENTRY32W.
    fn exe_name_from_entry(pe: &PROCESSENTRY32W) -> String {
        let nul = pe.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(pe.sz_exe_file.len());
        String::from_utf16_lossy(&pe.sz_exe_file[..nul]).to_lowercase()
    }

    /// Check if an executable name is a VT bridge process (WSL, SSH, etc.)
    /// that requires VT mouse injection instead of Win32 console injection.
    fn is_vt_bridge_exe(name: &str) -> bool {
        let stem = name.strip_suffix(".exe").unwrap_or(name);
        matches!(stem, "wsl" | "ssh" | "ubuntu" | "debian" | "kali"
                      | "fedoraremix" | "opensuse-leap" | "sles" | "arch")
            || stem.starts_with("wsl")
    }

    /// Walk the process tree from `root_pid` and check if any descendant
    /// is a VT bridge process (wsl.exe, ssh.exe, etc.).
    /// This is used for mouse injection: VT bridge processes need VT mouse
    /// sequences written to the PTY master, not Win32 MOUSE_EVENT records.
    pub fn has_vt_bridge_descendant(root_pid: u32) -> bool {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE || snap == 0 { return false; }

            let mut entries: Vec<(u32, u32, String)> = Vec::with_capacity(256);
            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe) != 0 {
                let name = exe_name_from_entry(&pe);
                entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
                while Process32NextW(snap, &mut pe) != 0 {
                    let name = exe_name_from_entry(&pe);
                    entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
                }
            }
            CloseHandle(snap);

            // BFS from root_pid to check all descendants
            let mut queue: Vec<u32> = vec![root_pid];
            let mut head = 0;
            while head < queue.len() {
                let parent = queue[head];
                head += 1;
                for (pid, ppid, name) in &entries {
                    if *ppid == parent && *pid != root_pid
                        && !queue.contains(pid)
                    {
                        if is_vt_bridge_exe(name) {
                            return true;
                        }
                        queue.push(*pid);
                    }
                }
            }
            false
        }
    }
}

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
    handle: *mut std::ffi::c_void,
    /// Frame buffer: accumulates all `write()` output so that `flush()`
    /// can emit the complete frame as a single `WriteConsoleW` call.
    /// This eliminates the visible top-to-bottom "curtain" repaint that
    /// occurs when ratatui's many small per-cell writes are each sent to
    /// the console individually.
    frame_buf: Vec<u8>,
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
    fn write_wide(&self, s: &str) -> std::io::Result<()> {
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
    const VK_MENU: i32 = 0x12; // Alt

    unsafe {
        let shift_down = GetAsyncKeyState(VK_SHIFT) < 0;
        let alt_down = GetAsyncKeyState(VK_MENU) < 0;

        if shift_down && !alt_down {
            key.modifiers.remove(KeyModifiers::ALT);
            key.modifiers.insert(KeyModifiers::SHIFT);
        } else if shift_down {
            key.modifiers.insert(KeyModifiers::SHIFT);
        }
    }
}
