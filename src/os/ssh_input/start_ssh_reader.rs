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

#[cfg(windows)]
pub(crate) fn start_ssh_reader() -> io::Result<std::sync::mpsc::Receiver<Event>> {
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

    // ── Setup + thread spawn ─────────────────────────────────────────────

    let (tx, rx) = mpsc::sync_channel::<Event>(1024);

    // ── Startup diagnostics ──────────────────────────────────────────────
    log_ssh_startup();

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
                                &*(rec.data.as_ptr() as *const SshMouseEventRecord)
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
