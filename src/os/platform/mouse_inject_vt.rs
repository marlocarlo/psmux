#[allow(unused_imports)]
use super::*;

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
#[cfg(windows)]
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
#[cfg(windows)]
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
        // Encode paste text as UTF-16, normalizing \n -> \r for the
        // console input buffer (Windows apps expect CR for line breaks;
        // PSReadLine and other readline implementations treat \r as Enter).
        let mut prev_cr = false;
        for c in text.chars() {
            if c == '\n' {
                if !prev_cr {
                    // Bare \n -> \r
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
        // console input injection, only key-down events carry characters).
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
