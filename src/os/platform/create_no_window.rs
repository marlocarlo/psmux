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

pub(crate) const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Extension trait that adds `.hide_window()` to `std::process::Command`.
///
/// Call this on any `Command` that spawns a background helper process.
/// On Windows it sets `CREATE_NO_WINDOW` so no cmd.exe / conhost.exe
/// window flashes on screen.  On other platforms it does nothing.
///
/// # Example
/// ```ignore
/// use crate::platform::HideWindowCommandExt;
/// std::process::Command::new("cmd")
///     .args(["/C", "echo hello"])
///     .hide_window()
///     .output();
/// ```
pub trait HideWindowCommandExt {
    fn hide_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl HideWindowCommandExt for std::process::Command {
    fn hide_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl HideWindowCommandExt for std::process::Command {
    fn hide_window(&mut self) -> &mut Self {
        self // no-op
    }
}

// ---------------------------------------------------------------------------

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
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;

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

    // Try with CREATE_BREAKAWAY_FROM_JOB first so the server escapes the
    // parent's Job Object (e.g. sshd's kill-on-close job).  If the job
    // disallows breakaway the call fails with ERROR_ACCESS_DENIED; in
    // that case fall back without the flag.
    let base_flags = CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP;
    let mut ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmdline_wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0, // don't inherit handles
            base_flags | CREATE_BREAKAWAY_FROM_JOB,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        )
    };

    if ok == 0 {
        // Retry without breakaway (job may disallow it)
        // Re-encode cmdline_wide since CreateProcessW may have modified it
        cmdline_wide = cmdline.encode_utf16().chain(std::iter::once(0)).collect();
        ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmdline_wide.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                base_flags,
                std::ptr::null(),
                std::ptr::null(),
                &si,
                &mut pi,
            )
        };
    }

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
