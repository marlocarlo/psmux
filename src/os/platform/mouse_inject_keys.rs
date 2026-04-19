#[allow(unused_imports)]
use super::*;

/// Send a CTRL_C_EVENT to all processes on the child's console.
///
/// TUI applications (pstop, btop, etc.) often disable ENABLE_PROCESSED_INPUT
/// on the ConPTY console and fail to restore it on exit.  When this flag is
/// off, writing 0x03 to the ConPTY input pipe no longer generates a
/// CTRL_C_EVENT signal, the byte is delivered as a regular key event that
/// most programs ignore.
///
/// This function works around the issue by:
///   1. Attaching to the child's hidden ConPTY console
///   2. Re-enabling ENABLE_PROCESSED_INPUT if it was cleared
///   3. Calling GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)
///
/// The combination ensures Ctrl+C always delivers a signal regardless of
/// what a previous TUI application did to the console mode.
#[cfg(windows)]
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
/// For Ctrl+key: `u_char` = control character (ch & 0x1F).
/// For Alt+key: `u_char` = the plain char.
/// For Ctrl+Alt: `u_char` = control character.
///
/// Sends both key-down and key-up events for proper event pairing.
#[cfg(windows)]
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

        let mut flags: u32 = 0;
        if ctrl { flags |= LEFT_CTRL_PRESSED; }
        if alt  { flags |= LEFT_ALT_PRESSED; }
        if shift { flags |= SHIFT_PRESSED; }

        let base_char = if shift && !ctrl {
            ch.to_ascii_uppercase()
        } else {
            ch
        };

        let u_char_value: u16 = if ctrl {
            (base_char.to_ascii_lowercase() as u16) & 0x1F
        } else {
            let mut buf = [0u16; 2];
            let encoded = base_char.encode_utf16(&mut buf);
            encoded[0]
        };

        let mut buf = [0u16; 2];
        let plain_wch = ch.to_ascii_lowercase().encode_utf16(&mut buf)[0];
        let vk_result = VkKeyScanW(plain_wch);
        let vk = if vk_result == -1 { 0u16 } else { (vk_result & 0xFF) as u16 };
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

/// Inject a modified Enter key event into a child process's console input.
///
/// Sends VK_RETURN with Ctrl/Alt/Shift flags so PSReadLine and
/// other console-API-based readers see the true Shift/Ctrl/Alt+Enter.
#[cfg(windows)]
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
