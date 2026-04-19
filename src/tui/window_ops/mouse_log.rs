#[allow(unused_imports)]
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{PtySize, native_pty_system};
use ratatui::prelude::*;

use crate::types::{AppState, Mode, Pane, Node, LayoutKind, DragState, Window, FocusDir};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, get_split_mut, resize_all_panes};
use crate::pane::{detect_shell, build_default_shell, set_tmux_env};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, yank_selection};
use crate::platform::mouse_inject;

/// Mouse debug logger — writes to ~/.psmux/mouse_debug.log when
/// PSMUX_MOUSE_DEBUG=1 is set.
use super::*;

pub(crate) fn mouse_log(msg: &str) {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        std::env::var("PSMUX_MOUSE_DEBUG").unwrap_or_default() == "1"
    });
    if !*ENABLED { return; }

    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    if n > 2000 { return; }

    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let path = format!("{}/.psmux/mouse_debug.log", home);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S%.3f"), msg);
    }
}

/// Convert screen coordinates to 0-based pane-local coordinates.
/// No border offset — panes are borderless (tmux-style).
pub(crate) fn pane_inner_cell_0based(area: Rect, abs_x: u16, abs_y: u16) -> (i16, i16) {
    let col = abs_x as i16 - area.x as i16;
    let row = abs_y as i16 - area.y as i16;
    (col, row)
}

/// Convert screen coordinates to 1-based pane-local coordinates.
pub(crate) fn pane_inner_cell(area: Rect, abs_x: u16, abs_y: u16) -> (u16, u16) {
    let col = abs_x.saturating_sub(area.x) + 1;
    let row = abs_y.saturating_sub(area.y) + 1;
    (col, row)
}

/// Map mouse coordinates from a client's terminal space to the server's effective
/// layout space.  When a client's terminal is larger or smaller than the effective
/// size used for layout computation, raw pixel coordinates don't match pane boundaries.
/// This ratio-based mapping is a "good enough" fallback for any interaction not yet
/// handled by client-side semantic commands.
pub(crate) fn map_client_coords(app: &AppState, x: u16, y: u16) -> (u16, u16) {
    let cid = match app.latest_client_id {
        Some(id) => id,
        None => return (x, y),
    };
    let (cw, ch) = match app.client_sizes.get(&cid) {
        Some(&size) => size,
        None => return (x, y),
    };
    let ew = app.last_window_area.width;
    let eh = app.last_window_area.height;
    if cw == ew && ch == eh {
        return (x, y);
    }
    let mx = if cw > 0 { ((x as u32) * (ew as u32) / (cw as u32)) as u16 } else { x };
    let my = if ch > 0 { ((y as u32) * (eh as u32) / (ch as u32)) as u16 } else { y };
    (mx.min(ew.saturating_sub(1)), my.min(eh.saturating_sub(1)))
}

/// Write a mouse event to the child PTY using the encoding the child requested.
pub fn write_mouse_event_remote(master: &mut dyn std::io::Write, button: u8, col: u16, row: u16, press: bool, enc: vt100::MouseProtocolEncoding) {
    match enc {
        vt100::MouseProtocolEncoding::Sgr => {
            let ch = if press { 'M' } else { 'm' };
            let _ = write!(master, "\x1b[<{};{};{}{}", button, col, row, ch);
            let _ = master.flush();
        }
        _ => {
            if press {
                let cb = (button + 32) as u8;
                let cx = ((col as u8).min(223)) + 32;
                let cy = ((row as u8).min(223)) + 32;
                let _ = master.write_all(&[0x1b, b'[', b'M', cb, cx, cy]);
                let _ = master.flush();
            }
        }
    }
}

/// Inject a mouse event into a pane via Windows Console API (WriteConsoleInputW).
///
/// For native Windows console apps: WriteConsoleInputW injects MOUSE_EVENT records
/// that ReadConsoleInput returns.  This works for apps like pstop, Far Manager, etc.
pub(crate) fn inject_mouse(pane: &mut Pane, col: i16, row: i16, button_state: u32, event_flags: u32) -> bool {
    if pane.child_pid.is_none() {
        pane.child_pid = mouse_inject::get_child_pid(&*pane.child);
    }
    if let Some(pid) = pane.child_pid {
        mouse_inject::send_mouse_event(pid, col, row, button_state, event_flags, false)
    } else {
        false
    }
}

/// Returns true if the window's foreground process is a VT bridge (wsl, ssh)
/// that needs VT mouse injection instead of Console API mouse injection.
pub(crate) fn is_vt_bridge(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("wsl") || lower.contains("ssh")
}

/// Permissive TUI detection for hover events — matches layout.rs heuristic.
///
/// Returns true when the last row of the pane screen has non-blank content,
/// which indicates a fullscreen app (status bar, menu bar, etc.).
///
/// This is deliberately less strict than `is_fullscreen_tui()`:
///   - `is_fullscreen_tui()` also requires the cursor in the bottom 3 rows,
///     which fails for apps like opencode whose cursor sits at a mid-screen
///     text input.
///   - For hover events, false positives are harmless — shells ignore bare
///     motion (SGR button 35).  False negatives break TUI hover (opencode,
///     etc.), so we use the permissive check.
pub(crate) fn screen_has_tui_content(pane: &Pane) -> bool {
    if let Ok(parser) = pane.term.lock() {
        let screen = parser.screen();
        if screen.alternate_screen() {
            return true;
        }
        let last_row = pane.last_rows.saturating_sub(1);
        for col in 0..pane.last_cols.min(80) {
            if let Some(cell) = screen.cell(last_row, col) {
                let t = cell.contents();
                if !t.is_empty() && t != " " {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if the pane is likely running a fullscreen TUI app (htop, vim, etc.)
/// by detecting alternate screen buffer usage.
///
/// ConPTY never passes DECSET 1049h (alternate screen) to the output pipe,
/// so `screen.alternate_screen()` is always false.  Use the same heuristic
/// as layout.rs: if the last row of the screen has non-blank content, the
/// pane is running a fullscreen app.
pub(crate) fn is_fullscreen_tui(pane: &Pane) -> bool {
    if let Ok(parser) = pane.term.lock() {
        let screen = parser.screen();
        // Fast check: if the parser reports alternate screen, trust it
        if screen.alternate_screen() {
            return true;
        }
        // Heuristic: check if many of the last rows are non-blank AND the
        // cursor is near the bottom.  Fullscreen TUI apps fill the entire
        // screen and keep the cursor near the bottom (status bars, menus).
        // A shell after `dir` may have content on the last row, but the
        // cursor sits at the current prompt line — not necessarily at the
        // bottom — and the rows below the cursor are blank.
        let rows = pane.last_rows;
        if rows < 3 { return false; }
        let (cursor_row, _) = screen.cursor_position();
        let last_row = rows.saturating_sub(1);
        // Cursor must be in the bottom 3 rows for a fullscreen TUI
        if cursor_row < last_row.saturating_sub(2) {
            return false;
        }
        // Check that at least 3 of the last 4 rows have non-blank content
        let check_rows = 4u16.min(rows);
        let mut filled = 0u16;
        for r in (last_row + 1 - check_rows)..=last_row {
            let mut has_content = false;
            for col in 0..pane.last_cols.min(40) { // only check first 40 cols
                if let Some(cell) = screen.cell(r, col) {
                    let t = cell.contents();
                    if !t.is_empty() && t != " " {
                        has_content = true;
                        break;
                    }
                }
            }
            if has_content { filled += 1; }
        }
        return filled >= 3;
    }
    false
}

/// Check if the child process in this pane has enabled mouse tracking
/// (DECSET 1000/1002/1003) and therefore wants to receive scroll wheel events.
///
/// This is the same logic tmux uses: if mouse_protocol_mode != None, the
/// child app (vim, htop, less -R, etc.) handles mouse itself, so psmux
/// forwards scroll events to it.  If None (shell prompt), psmux enters
/// copy mode on scroll-up, matching tmux behavior with `set -g mouse on`.
///
/// Note: ConPTY strips DECSET mouse mode escape sequences from the output
/// stream, so for native Windows console apps `mouse_protocol_mode()` is
/// always `None`.  This is correct: native Windows TUI apps receive mouse
/// via Win32 MOUSE_EVENT injection (separate path), and shell prompts
/// (PowerShell, cmd) don't want scroll events at all — scrollback is the
/// right behavior.
///
/// For apps running through a VT bridge (WSL, SSH), the VT escape sequences
/// DO pass through, so `mouse_protocol_mode()` correctly reflects the
/// child's actual mouse tracking state.
pub(crate) fn pane_wants_mouse(pane: &Pane) -> bool {
    if let Ok(parser) = pane.term.lock() {
        let screen = parser.screen();
        // Primary check (tmux parity): did the child enable mouse protocol?
        if screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None {
            return true;
        }
        // Secondary check: alternate screen active (ConPTY may strip DECSET
        // 1000 but some builds pass DECSET 1049h through).
        if screen.alternate_screen() {
            return true;
        }
    }
    false
}

/// Detect whether a pane has a VT bridge descendant (wsl.exe, ssh.exe, etc.)
/// by walking the process tree.  Result is cached for 2 seconds per pane
/// to avoid expensive CreateToolhelp32Snapshot on every mouse event.
pub(crate) fn detect_vt_bridge(pane: &mut Pane) -> bool {
    // Check cache first (2 second TTL)
    if let Some((ts, cached)) = pane.vt_bridge_cache {
        if ts.elapsed().as_secs() < 2 {
            return cached;
        }
    }
    // Ensure child_pid is resolved
    if pane.child_pid.is_none() {
        pane.child_pid = mouse_inject::get_child_pid(&*pane.child);
    }
    let result = if let Some(pid) = pane.child_pid {
        crate::platform::process_info::has_vt_bridge_descendant(pid)
    } else {
        false
    };
    pane.vt_bridge_cache = Some((std::time::Instant::now(), result));
    result
}

/// Detect whether the child's console has ENABLE_MOUSE_INPUT (0x0010) set.
///
/// When true, the child reads MOUSE_EVENT records via ReadConsoleInputW
/// (crossterm/ratatui apps like pstop, claude).  When false, the child
/// reads input as text / VT sequences (nvim, vim, opencode).
///
/// Result is cached for 2 seconds per pane.
pub(crate) fn detect_mouse_input(pane: &mut Pane) -> bool {
    if let Some((ts, cached)) = pane.mouse_input_cache {
        if ts.elapsed().as_secs() < 2 {
            return cached;
        }
    }
    if pane.child_pid.is_none() {
        pane.child_pid = mouse_inject::get_child_pid(&*pane.child);
    }
    let result = if let Some(pid) = pane.child_pid {
        mouse_inject::query_mouse_input_enabled(pid).unwrap_or(false)
    } else {
        false
    };
    pane.mouse_input_cache = Some((std::time::Instant::now(), result));
    result
}

/// Helper: inject SGR mouse via WriteConsoleInputW KEY_EVENT records.
///
/// Used ONLY for WSL/SSH bridge children where the PTY pipe doesn't reach
/// the remote TUI.  For native ConPTY children, use write_mouse_to_pty().
pub(crate) fn inject_sgr_mouse(pane: &mut Pane, col: i16, row: i16, vt_button: u8, press: bool) -> bool {
    let vt_col = (col + 1).max(1) as u16;
    let vt_row = (row + 1).max(1) as u16;
    let ch = if press { 'M' } else { 'm' };
    let sgr_seq = format!("\x1b[<{};{};{}{}", vt_button, vt_col, vt_row, ch);
    mouse_log(&format!("  -> Console VT injection (KEY_EVENTs): seq={:?}", sgr_seq));
    if pane.child_pid.is_none() {
        pane.child_pid = mouse_inject::get_child_pid(&*pane.child);
    }
    if let Some(pid) = pane.child_pid {
        let ok = crate::platform::send_vt_sequence(pid, sgr_seq.as_bytes());
        mouse_log(&format!("  -> Console VT inject result: {}", ok));
        ok
    } else {
        false
    }
}

/// Write a SGR mouse event to the pane's PTY master pipe.
///
/// This is the same mechanism Windows Terminal uses: write VT SGR mouse
/// escape sequences directly to the ConPTY input pipe.  ConPTY/conhost
/// then automatically:
///  - Translates SGR → MOUSE_EVENT records for apps using ReadConsoleInputW
///    (crossterm/ratatui: pstop, claude, opencode, etc.)
///  - Passes VT through for apps reading text/VT input (nvim, vim)
///
/// This works universally for ALL native ConPTY children — no need to
/// distinguish between crossterm vs nvim.  (fixes #60)
pub(crate) fn write_mouse_to_pty(pane: &mut Pane, col: i16, row: i16, vt_button: u8, press: bool) {
    use std::io::Write as _;
    let vt_col = (col + 1).max(1) as u16;
    let vt_row = (row + 1).max(1) as u16;
    let ch = if press { b'M' } else { b'm' };
    // Stack-allocated buffer — avoids heap allocation per mouse event.
    // Max SGR sequence: ESC[<btn;col;rowM = ~20 bytes worst case.
    let mut buf = [0u8; 32];
    let len = {
        let mut cursor = std::io::Cursor::new(&mut buf[..]);
        let _ = write!(cursor, "\x1b[<{};{};{}{}", vt_button, vt_col, vt_row, ch as char);
        cursor.position() as usize
    };
    mouse_log(&format!("  -> PTY pipe SGR mouse: seq={:?}", std::str::from_utf8(&buf[..len]).unwrap_or("?")));
    let _ = pane.writer.write_all(&buf[..len]);
    let _ = pane.writer.flush();
}

/// Inject a mouse event into a pane using the best available method.
///
/// Architecture (mirrors Windows Terminal):
///
///   For native ConPTY children, write SGR mouse escape sequences directly
///   to the PTY master pipe (pane.writer).  This is the same mechanism
///   Windows Terminal uses.  ConPTY/conhost handles all translation:
///   - Apps using ReadConsoleInputW (crossterm/ratatui) get MOUSE_EVENT records
///   - Apps reading VT input (nvim/vim) get the SGR sequences directly
///
///   For WSL/SSH bridge children, bypass ConPTY using WriteConsoleInputW
///   with KEY_EVENT records, delivering escape sequences to the bridge
///   process (wsl.exe/ssh.exe) which relays them to the Linux PTY.
///
///   At shell prompts (no TUI), no mouse forwarding is needed — the shell
///   doesn't handle mouse events.  Callers should handle shell-level
///   behavior (right-click=paste, scroll=copy-mode) before calling this.
pub(crate) fn inject_mouse_combined(pane: &mut Pane, col: i16, row: i16, vt_button: u8, press: bool,
                          _button_state: u32, _event_flags: u32, win_name: &str) {
    let vt_bridge = detect_vt_bridge(pane);

    if vt_bridge {
        // WSL/SSH bridge — bypass ConPTY, inject as KEY_EVENT records.
        // The bridge (wsl.exe, ssh.exe) relays these to the Linux PTY.
        //
        // Gate on mouse_protocol_mode (tmux + Windows Terminal parity):
        // Only forward mouse events when the remote app has explicitly
        // enabled mouse tracking (DECSET 1000/1002/1003).  For VT bridge
        // children, VT escape sequences pass through unmodified, so
        // mouse_protocol_mode() accurately reflects the remote app's
        // actual mouse tracking state.
        //
        // Without this gate, SGR mouse sequences are injected as KEY_EVENT
        // records → ssh.exe/wsl.exe relays them as literal text → the
        // remote shell prints raw escape sequences at the prompt.
        // This is the root cause of issue #77 (mouse events leak as raw
        // text into SSH panes).
        let wants = pane.term.lock().ok()
            .map_or(false, |t| t.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None);
        if !wants {
            mouse_log(&format!("inject_mouse_combined: col={} row={} vt_btn={} press={} win={} vt_bridge=true -> SUPPRESSED (remote has no mouse tracking)",
                col, row, vt_button, press, win_name));
            return;
        }
        mouse_log(&format!("inject_mouse_combined: col={} row={} vt_btn={} press={} win={} vt_bridge=true -> WriteConsoleInputW KEY_EVENT injection",
            col, row, vt_button, press, win_name));
        inject_sgr_mouse(pane, col, row, vt_button, press);
    } else {
        // Native ConPTY child — write SGR mouse to PTY pipe.
        // This is the same mechanism Windows Terminal uses.
        // ConPTY translates SGR → MOUSE_EVENT for crossterm apps,
        // and passes VT through for nvim/vim.
        mouse_log(&format!("inject_mouse_combined: col={} row={} vt_btn={} press={} win={} -> PTY pipe SGR mouse (Windows Terminal method)",
            col, row, vt_button, press, win_name));
        write_mouse_to_pty(pane, col, row, vt_button, press);
    }
}
