#[allow(unused_imports)]
use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::types::{AppState, Pane, Node, LayoutKind, Window};
use crate::tree::{replace_leaf_with_split, active_pane_mut, kill_leaf};
use crate::format::hostname_cached;

/// Sentinel value for cursor_shape: means "no DECSCUSR received from child yet".
/// When ConPTY passthrough mode is unavailable, DECSCUSR sequences from child
/// processes are consumed by ConPTY and never forwarded.  Using this sentinel
/// lets the rendering code skip emitting any cursor-shape override, so the
/// real terminal keeps its user-configured default cursor.
use super::*;

/// Scan raw ConPTY output for DECSCUSR cursor shape sequences (`\x1b[N q`).
/// Returns the last cursor shape value found, or None.
///
/// We accept all DECSCUSR cursor shape values (0-6) from child processes.
/// Value 0 resets to default, 1-2 = block, 3-4 = underline, 5-6 = bar.
pub(crate) fn scan_cursor_shape(data: &[u8]) -> Option<u8> {
    let mut last_shape: Option<u8> = None;
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x1b && i + 1 < data.len() && data[i + 1] == b'[' {
            let mut j = i + 2;
            let mut param: u8 = 0;
            while j < data.len() && data[j].is_ascii_digit() {
                param = param.saturating_mul(10).saturating_add(data[j] - b'0');
                j += 1;
            }
            // Check for SP q (space 0x20 + 'q') = DECSCUSR
            if j + 1 < data.len() && data[j] == b' ' && data[j + 1] == b'q' {
                if param <= 6 {
                    last_shape = Some(param);
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    last_shape
}

/// Returns true if `data` contains the RMCUP sequence (ESC[?1049l).
pub(crate) fn scan_rmcup(data: &[u8]) -> bool {
    const RMCUP: &[u8] = b"\x1b[?1049l";
    data.windows(RMCUP.len()).any(|w| w == RMCUP)
}

/// Spawn a dedicated PTY reader thread that processes output and updates the
/// data_version counter. Exits cleanly after 200 consecutive zero-byte reads
/// (indicating the PTY pipe is closed) or on any I/O error.
///
/// Uses an 8KB read buffer (down from 64KB) to reduce mutex hold time during
/// `parser.process()`, which improves DumpState latency under heavy output.
pub fn spawn_reader_thread(
    mut reader: Box<dyn std::io::Read + Send>,
    term_reader: Arc<Mutex<vt100::Parser>>,
    dv_writer: Arc<std::sync::atomic::AtomicU64>,
    cursor_shape: Arc<std::sync::atomic::AtomicU8>,
    bell_pending: Arc<std::sync::atomic::AtomicBool>,
    output_ring: Arc<Mutex<std::collections::VecDeque<u8>>>,
) {
    thread::spawn(move || {
        // 64KB buffer: captures most full-screen TUI paints in a single
        // read(), preventing partial-frame rendering ("curtain effect")
        // that occurs when ConPTY output is split across multiple small reads.
        let mut local = vec![0u8; 65536];
        let mut zero_reads: u32 = 0;
        loop {
            match reader.read(&mut local) {
                Ok(n) if n > 0 => {
                    zero_reads = 0;
                    // Scan for DECSCUSR cursor shape before vt100 parser consumes data.
                    if let Some(shape) = scan_cursor_shape(&local[..n]) {
                        cursor_shape.store(shape, std::sync::atomic::Ordering::Release);
                    }
                    let rmcup = scan_rmcup(&local[..n]);
                    if let Ok(mut parser) = term_reader.lock() {
                        parser.process(&local[..n]);
                        // Check for audible bells detected by the vt100
                        // parser.  The parser correctly distinguishes
                        // standalone BEL (0x07) from OSC/DCS/APC string
                        // terminators, maintaining state across chunks.
                        if parser.screen_mut().take_audible_bell() {
                            bell_pending.store(true, std::sync::atomic::Ordering::Release);
                        }
                    }
                    // Append raw output to ring buffer for control mode %output
                    if let Ok(mut ring) = output_ring.lock() {
                        const MAX_RING: usize = 65536;
                        let space = MAX_RING.saturating_sub(ring.len());
                        if n <= space {
                            ring.extend(&local[..n]);
                        } else {
                            // Drop oldest data to make room
                            let drop_count = (n - space).min(ring.len());
                            ring.drain(..drop_count);
                            ring.extend(&local[..n]);
                        }
                    }
                    // When TUI sends RMCUP, reset cursor shape so it
                    // doesn't persist from the exiting TUI app.
                    if rmcup {
                        cursor_shape.store(0, std::sync::atomic::Ordering::Release);
                    }
                    dv_writer.fetch_add(1, std::sync::atomic::Ordering::Release);
                    crate::types::PTY_DATA_READY.store(true, std::sync::atomic::Ordering::Release);
                }
                Ok(_) => {
                    zero_reads += 1;
                    if zero_reads > 10 { break; }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(_) => break,
            }
        }
        // Reader exited (child process died / pipe closed).
        // If parser is still in alt-screen the TUI crashed without
        // sending RMCUP — force cleanup now (TUI is guaranteed dead).
        if let Ok(mut parser) = term_reader.lock() {
            if parser.screen().alternate_screen() {
                parser.process(b"\x1b[?25h\x1b[?1049l");
                cursor_shape.store(0, std::sync::atomic::Ordering::Release);
                dv_writer.fetch_add(1, std::sync::atomic::Ordering::Release);
                crate::types::PTY_DATA_READY.store(true, std::sync::atomic::Ordering::Release);
            }
        }
    });
}

#[cfg(test)]
#[path = "../../../tests-rs/test_issue151_strict_mode.rs"]
mod test_issue151_strict_mode;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue155_output_rendering.rs"]
mod test_issue155_output_rendering;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue165_prediction_view_style.rs"]
mod test_issue165_prediction_view_style;

#[cfg(test)]
mod test_parser_audible_bell {
    /// Helper: create a parser, process bytes, return whether bell rang.
    fn bell_after(data: &[u8]) -> bool {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(data);
        p.screen_mut().take_audible_bell()
    }

    /// Helper: process two chunks sequentially (simulates cross-chunk reads),
    /// return whether bell rang after the second chunk.
    fn bell_after_two_chunks(chunk1: &[u8], chunk2: &[u8]) -> bool {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(chunk1);
        // Consume any bell from chunk1 so we only test chunk2
        let _ = p.screen_mut().take_audible_bell();
        p.process(chunk2);
        p.screen_mut().take_audible_bell()
    }

    #[test]
    fn bare_bel() {
        assert!(bell_after(b"\x07"));
    }

    #[test]
    fn bel_in_plain_text() {
        assert!(bell_after(b"hello\x07world"));
    }

    #[test]
    fn osc_title_with_bel_terminator() {
        // OSC BEL terminator is NOT an audible bell
        assert!(!bell_after(b"\x1b]0;My Title\x07"));
    }

    #[test]
    fn osc_title_with_st_terminator() {
        assert!(!bell_after(b"\x1b]0;My Title\x1b\\"));
    }

    #[test]
    fn osc_then_standalone_bel() {
        // OSC terminated by BEL, then a real standalone BEL
        assert!(bell_after(b"\x1b]0;title\x07\x07"));
    }

    #[test]
    fn multiple_osc_no_real_bel() {
        assert!(!bell_after(b"\x1b]0;title1\x07\x1b]2;title2\x07"));
    }

    #[test]
    fn empty_data() {
        assert!(!bell_after(b""));
    }

    #[test]
    fn no_bel_at_all() {
        assert!(!bell_after(b"just text\x1b[31m"));
    }

    #[test]
    fn powershell_prompt_title_no_bell() {
        // Simulates PowerShell: sets title via OSC, then prints prompt (no BEL)
        let data = b"\x1b]0;PS C:\\Users\\test\x07\x1b[32mPS>\x1b[0m ";
        assert!(!bell_after(data));
    }

    #[test]
    fn take_clears_flag() {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(b"\x07");
        assert!(p.screen_mut().take_audible_bell());
        // Second take should be false (consumed)
        assert!(!p.screen_mut().take_audible_bell());
    }

    #[test]
    fn cross_chunk_osc_then_real_bel() {
        // Chunk 1 starts OSC without terminator; chunk 2 has the
        // OSC terminator BEL then a real standalone BEL.
        // The parser maintains state across chunks, so this works
        // correctly (unlike the old stateless scan_standalone_bel).
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(b"\x1b]0;title");
        assert!(!p.screen_mut().take_audible_bell());
        p.process(b"\x07\x07");
        assert!(p.screen_mut().take_audible_bell());
    }

    #[test]
    fn cross_chunk_osc_no_real_bel() {
        // Chunk 1 starts OSC; chunk 2 only has the OSC terminator.
        // No real bell should fire.
        assert!(!bell_after_two_chunks(b"\x1b]0;title", b"\x07"));
    }
}

// reap_children is in tree.rs
