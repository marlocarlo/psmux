use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{PtySize, native_pty_system};
use ratatui::prelude::*;

use crate::types::{AppState, Mode, Pane, Node, LayoutKind, DragState, Window, FocusDir};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, get_split_mut, resize_all_panes};
use crate::pane::{detect_shell, build_default_shell, set_tmux_env};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, scroll_copy_up, scroll_copy_down, yank_selection};
use crate::platform::mouse_inject;

/// Mouse debug logger — writes to ~/.psmux/mouse_debug.log when
/// PSMUX_MOUSE_DEBUG=1 is set.
fn mouse_log(msg: &str) {
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
fn pane_inner_cell_0based(area: Rect, abs_x: u16, abs_y: u16) -> (i16, i16) {
    let col = abs_x as i16 - area.x as i16;
    let row = abs_y as i16 - area.y as i16;
    (col, row)
}

/// Convert screen coordinates to 1-based pane-local coordinates.
fn pane_inner_cell(area: Rect, abs_x: u16, abs_y: u16) -> (u16, u16) {
    let col = abs_x.saturating_sub(area.x) + 1;
    let row = abs_y.saturating_sub(area.y) + 1;
    (col, row)
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
fn inject_mouse(pane: &mut Pane, col: i16, row: i16, button_state: u32, event_flags: u32) -> bool {
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
fn is_vt_bridge(name: &str) -> bool {
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
fn detect_vt_bridge(pane: &mut Pane) -> bool {
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
fn detect_mouse_input(pane: &mut Pane) -> bool {
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
fn inject_sgr_mouse(pane: &mut Pane, col: i16, row: i16, vt_button: u8, press: bool) -> bool {
    let vt_col = (col + 1).max(1) as u16;
    let vt_row = (row + 1).max(1) as u16;
    let ch = if press { 'M' } else { 'm' };
    let sgr_seq = format!("\x1b[<{};{};{}{}", vt_button, vt_col, vt_row, ch);
    mouse_log(&format!("  -> Console VT injection (KEY_EVENTs): seq={:?}", sgr_seq));
    if pane.child_pid.is_none() {
        pane.child_pid = mouse_inject::get_child_pid(&*pane.child);
    }
    if let Some(pid) = pane.child_pid {
        let ok = mouse_inject::send_vt_sequence(pid, sgr_seq.as_bytes());
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
fn write_mouse_to_pty(pane: &mut Pane, col: i16, row: i16, vt_button: u8, press: bool) {
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

/// Temporarily unzoom for an operation, saving the zoom state so it can be
/// restored via `pop_zoom()` afterwards (tmux push/pop semantics).
/// Returns true if zoom was active and was suspended.
pub fn push_zoom(app: &mut AppState) -> bool {
    if app.windows[app.active_idx].zoom_saved.is_some() {
        // Mark that we had zoom active, unzoom, but DON'T clear zoom_saved
        // — we move it to a temp slot so pop_zoom can re-apply it.
        unzoom_if_zoomed(app);
        true
    } else {
        false
    }
}

/// Re-apply zoom after a push_zoom operation (tmux push/pop semantics).
/// Only re-zooms if `was_zoomed` is true.
pub fn pop_zoom(app: &mut AppState, was_zoomed: bool) {
    if was_zoomed && app.windows[app.active_idx].zoom_saved.is_none() {
        toggle_zoom(app);
    }
}

/// If zoom is currently active, unzoom (restore saved sizes) and resize panes.
/// Returns true if zoom was active and was cancelled.
pub fn unzoom_if_zoomed(app: &mut AppState) -> bool {
    if let Some(saved) = app.windows[app.active_idx].zoom_saved.take() {
        let win = &mut app.windows[app.active_idx];
        for (p, sz) in saved.into_iter() {
            if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) { *sizes = sz; }
        }
        resize_all_panes(app);
        true
    } else {
        false
    }
}

pub fn toggle_zoom(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if win.zoom_saved.is_none() {
        let mut saved: Vec<(Vec<usize>, Vec<u16>)> = Vec::new();
        for depth in 0..win.active_path.len() {
            let p = win.active_path[..depth].to_vec();
            if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) {
                let idx = win.active_path.get(depth).copied().unwrap_or(0);
                saved.push((p.clone(), sizes.clone()));
                for i in 0..sizes.len() { sizes[i] = if i == idx { 100 } else { 0 }; }
            }
        }
        win.zoom_saved = Some(saved);
    } else {
        if let Some(saved) = app.windows[app.active_idx].zoom_saved.take() {
            let win = &mut app.windows[app.active_idx];
            for (p, sz) in saved.into_iter() {
                if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) { *sizes = sz; }
            }
        }
    }
    // Resize all panes so child PTYs are notified of the new dimensions.
    // Without this, zoomed panes keep their pre-zoom size and child apps
    // (neovim, bottom, etc.) render in only half the screen. (issue #35)
    resize_all_panes(app);
}

/// Compute tab positions on the server side to match the client's status bar layout.
/// The client renders: "[session_name] idx: window_name idx: window_name ..."
pub fn update_tab_positions(app: &mut AppState) {
    let mut tab_pos: Vec<(usize, u16, u16)> = Vec::new();
    let mut cursor_x: u16 = 0;
    // Session label: "[session_name] "
    let session_label_len = app.session_name.len() as u16 + 3; // '[' + name + ']' + ' '
    cursor_x += session_label_len;
    // Window tabs: "idx: window_name " for each window
    for (i, w) in app.windows.iter().enumerate() {
        let display_idx = i + app.window_base_index;
        let label = format!("{}: {} ", display_idx, w.name);
        let start_x = cursor_x;
        cursor_x += label.len() as u16;
        tab_pos.push((i, start_x, cursor_x));
    }
    app.tab_positions = tab_pos;
}

pub fn remote_mouse_down(app: &mut AppState, x: u16, y: u16) {
    // Recompute tab positions to match client rendering
    update_tab_positions(app);

    // Check tab click on status bar
    let status_row = app.last_window_area.y + app.last_window_area.height;
    if y == status_row {
        for &(win_idx, x_start, x_end) in app.tab_positions.iter() {
            if x >= x_start && x < x_end && win_idx < app.windows.len() {
                if win_idx != app.active_idx {
                    crate::debug_log::server_log("switch", &format!(
                        "TAB CLICK: active_idx {} -> {} x={} y={} status_row={} tab_range={}..{}",
                        app.active_idx, win_idx, x, y, status_row, x_start, x_end));
                }
                app.last_window_idx = app.active_idx;
                app.active_idx = win_idx;
                return;
            }
        }
        return;
    }

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    let mut active_area: Option<Rect> = None;
    for (path, area) in rects.iter() {
        if area.contains(ratatui::layout::Position { x, y }) {
            win.active_path = path.clone();
            // Update MRU for clicked pane (tmux parity #70)
            if let Some(pid) = crate::tree::get_active_pane_id(&win.root, path) {
                crate::tree::touch_mru(&mut win.pane_mru, pid);
            }
            active_area = Some(*area);
        }
    }

    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        if let Some(area) = active_area {
            let (row, col) = copy_cell_for_area(area, x, y);
            // Single click positions cursor, clears selection (tmux parity).
            // Selection only starts on drag.
            app.copy_anchor = None;
            app.copy_pos = Some((row, col));
        }
        return;
    }

    let mut on_border = false;
    // Skip border detection when zoomed — no visible borders (#82)
    let mut borders: Vec<(Vec<usize>, LayoutKind, usize, u16, u16)> = Vec::new();
    if win.zoom_saved.is_none() {
        compute_split_borders(&win.root, app.last_window_area, &mut borders);
    }
    let tol = 1u16;
    for (path, kind, idx, pos, total_px) in borders.iter() {
        match kind {
            LayoutKind::Horizontal => {
                if x >= pos.saturating_sub(tol) && x <= pos + tol { if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) { app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: *pos, start_y: y, left_initial: left, _right_initial: right, total_pixels: *total_px }); } on_border = true; break; }
            }
            LayoutKind::Vertical => {
                if y >= pos.saturating_sub(tol) && y <= pos + tol { if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) { app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: x, start_y: *pos, left_initial: left, _right_initial: right, total_pixels: *total_px }); } on_border = true; break; }
            }
        }
    }

    // Forward left-click only when active pane wants mouse input.
    if !on_border {
        if let Some(area) = active_area {
            let (col, row) = pane_inner_cell_0based(area, x, y);
            let win_name = win.name.clone();
            if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                if pane_wants_mouse(active) {
                    inject_mouse_combined(active, col, row, 0, true,
                        mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED, 0, &win_name);
                }
            }
        }
    }
}

pub fn remote_mouse_drag(app: &mut AppState, x: u16, y: u16) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);

    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x, y })) {
            win.active_path = path.clone();
            let (row, col) = copy_cell_for_area(*area, x, y);
            if app.copy_anchor.is_none() {
                app.copy_anchor = Some((row, col));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
            app.copy_pos = Some((row, col));
        }
        return;
    }

    if let Some(d) = &app.drag {
        adjust_split_sizes(&mut win.root, d, x, y);
    } else {
        // Forward drag only when active pane wants mouse input.
        if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
            let (col, row) = pane_inner_cell_0based(area, x, y);
            let win_name = win.name.clone();
            if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                if pane_wants_mouse(active) {
                    inject_mouse_combined(active, col, row, 32, true,
                        mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED, mouse_inject::MOUSE_MOVED, &win_name);
                }
            }
        }
    }
}

pub fn remote_mouse_up(app: &mut AppState, x: u16, y: u16) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);

    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x, y })) {
            win.active_path = path.clone();
            let (row, col) = copy_cell_for_area(*area, x, y);
            if app.copy_anchor.is_none() {
                app.copy_anchor = Some((row, col));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
            }
            app.copy_pos = Some((row, col));
        }
        // Auto-yank if selection exists (anchor != pos)
        if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
            if a != p {
                let _ = yank_selection(app);
            }
        }
        return;
    }

    // If we were dragging a border, resize all panes to match new layout
    let was_dragging = app.drag.is_some();
    app.drag = None;
    if was_dragging {
        resize_all_panes(app);
        return;
    }

    // Forward mouse release only when active pane wants mouse input.
    if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
        let (col, row) = pane_inner_cell_0based(area, x, y);
        let win_name = win.name.clone();
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if pane_wants_mouse(active) {
                inject_mouse_combined(active, col, row, 0, false,
                    0, 0, &win_name);
            }
        }
    }
}

/// Forward a non-left mouse button press/release to the child.
pub fn remote_mouse_button(app: &mut AppState, x: u16, y: u16, button: u8, press: bool) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
        let (col, row) = pane_inner_cell_0based(area, x, y);
        let win_name = win.name.clone();
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if pane_wants_mouse(active) {
                let sgr_btn = match button {
                    1 => 1u8, // middle
                    2 => 2u8, // right
                    _ => 0u8,
                };
                let button_state = if press {
                    match button {
                        1 => mouse_inject::FROM_LEFT_2ND_BUTTON_PRESSED,
                        2 => mouse_inject::RIGHTMOST_BUTTON_PRESSED,
                        _ => 0,
                    }
                } else {
                    0
                };
                inject_mouse_combined(active, col, row, sgr_btn, press,
                    button_state, 0, &win_name);
            }
        }
    }
}

/// Forward bare mouse motion (hover) to the child PTY.
///
/// Only forwarded when the active pane explicitly wants mouse input
/// (`pane_wants_mouse`).  Shell prompts and ClaudeCode-style inputs are
/// excluded because they do not enable mouse tracking, and sending raw SGR
/// motion bytes (ESC[<35;...) would appear as visible garbage.
///
/// SGR button 35 = bare motion with no button held (WT parity).
/// Windows Terminal encodes hover as WM_MOUSEMOVE -> button 3 + 0x20 = 35.
///
/// Same-coordinate events are suppressed (Windows Terminal parity: the
/// terminal only sends motion when coordinates actually change).
pub fn remote_mouse_motion(app: &mut AppState, x: u16, y: u16) {
    // WT parity: suppress same-coordinate duplicates
    if app.last_hover_pos == Some((x, y)) {
        return;
    }
    app.last_hover_pos = Some((x, y));

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);

    // Forward hover only when the active pane explicitly wants mouse input.
    // This avoids leaking raw SGR motion bytes (ESC[<35;...) into shell-style
    // prompts such as claudecode input boxes.
    mouse_log(&format!("remote_mouse_motion: x={} y={}", x, y));

    if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
        let (col, row) = pane_inner_cell_0based(area, x, y);
        let win_name = win.name.clone();
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if pane_wants_mouse(active) {
                inject_mouse_combined(active, col, row, 35, true,
                    0, mouse_inject::MOUSE_MOVED, &win_name);
            }
        }
    }
}

fn wheel_cell_for_area(area: Rect, x: u16, y: u16) -> (u16, u16) {
    // Convert global terminal coordinates to 1-based pane-local coordinates (no border offset).
    let col = x.saturating_sub(area.x).min(area.width.saturating_sub(1)).saturating_add(1);
    let row = y.saturating_sub(area.y).min(area.height.saturating_sub(1)).saturating_add(1);
    (col, row)
}

fn copy_cell_for_area(area: Rect, x: u16, y: u16) -> (u16, u16) {
    // Convert global terminal coordinates to 0-based pane-local coordinates (no border offset).
    let col = x.saturating_sub(area.x).min(area.width.saturating_sub(1));
    let row = y.saturating_sub(area.y).min(area.height.saturating_sub(1));
    (row, col)
}

fn remote_scroll_wheel(app: &mut AppState, x: u16, y: u16, up: bool) {
    let mode_str = match &app.mode {
        Mode::Passthrough => "Passthrough",
        Mode::CopyMode => "CopyMode",
        Mode::CopySearch { .. } => "CopySearch",
        _ => "Other",
    };
    mouse_log(&format!("remote_scroll_wheel: x={} y={} up={} mode={}", x, y, up, mode_str));

    // Ignore scroll in popup mode — don't enter copy-mode (#110)
    if matches!(app.mode, Mode::PopupMode { .. }) {
        mouse_log("  -> popup mode, ignoring scroll");
        return;
    }

    // Handle scroll while already in copy mode
    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        mouse_log("  -> already in copy mode, scrolling within");
        if up {
            scroll_copy_up(app, 3);
        } else {
            scroll_copy_down(app, 3);
            // Auto-exit copy mode when scrolled back to live output
            if app.copy_scroll_offset == 0 && app.copy_anchor.is_none() {
                exit_copy_mode(app);
            }
        }
        return;
    }

    // Determine target pane, switch focus, and check if child is in alternate screen.
    //
    // IMPORTANT (tmux parity): For scroll events, we ONLY check alternate_screen()
    // to decide whether to forward to the child or enter copy mode.
    //
    // We do NOT use:
    //   - pane_wants_mouse() / mouse_protocol_mode(): PSReadLine on ConPTY
    //     spuriously enables AnyMotion mouse tracking.
    //   - is_fullscreen_tui() heuristic: A shell prompt after `ls` / `dir` can
    //     fill the last rows + leave the cursor at the bottom, causing a false
    //     positive that prevents scroll-to-copy-mode.
    //
    // alternate_screen() is reliable: all modern TUI apps (nvim, htop, vim,
    // opencode) correctly report alternate screen through ConPTY.  Testing
    // confirms nvim shows alternate_on=1.  Shell prompts always show 0.
    let (child_in_alt_screen, target_area_opt, sgr_btn, button_state) = {
        let win = &mut app.windows[app.active_idx];
        let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
        compute_rects(&win.root, app.last_window_area, &mut rects);

        let mut target_area: Option<Rect> = None;
        for (path, area) in &rects {
            if area.contains(ratatui::layout::Position { x, y }) {
                win.active_path = path.clone();
                target_area = Some(*area);
                break;
            }
        }
        if target_area.is_none() {
            target_area = rects
                .iter()
                .find(|(path, _)| *path == win.active_path)
                .map(|(_, area)| *area);
        }

        let alt = active_pane(&win.root, &win.active_path)
            .map_or(false, |p| {
                if let Ok(parser) = p.term.lock() {
                    return parser.screen().alternate_screen();
                }
                false
            });
        let sgr_btn: u8 = if up { 64 } else { 65 };
        let wheel_delta: i16 = if up { 120 } else { -120 };
        let bs = ((wheel_delta as i32) << 16) as u32;
        (alt, target_area, sgr_btn, bs)
    };

    mouse_log(&format!("  -> alt_screen={}", child_in_alt_screen));

    if child_in_alt_screen {
        // Forward scroll to child TUI app (alternate screen = real TUI)
        mouse_log("  -> forwarding scroll to child TUI (alt screen)");
        let win = &mut app.windows[app.active_idx];
        let (col, row) = target_area_opt.map_or((0, 0), |area| pane_inner_cell_0based(area, x, y));
        let win_name = win.name.clone();
        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
            inject_mouse_combined(p, col, row, sgr_btn, true,
                button_state, mouse_inject::MOUSE_WHEELED, &win_name);
        }
    } else if up {
        // Shell prompt — auto-enter copy mode and scroll up (tmux parity)
        mouse_log("  -> entering copy mode (shell scroll-up)");
        enter_copy_mode(app);
        scroll_copy_up(app, 3);
    } else {
        mouse_log("  -> scroll-down at shell (no-op)");
    }
    // Scroll down at shell prompt without copy mode is a no-op
}

pub fn remote_scroll_up(app: &mut AppState, x: u16, y: u16) { remote_scroll_wheel(app, x, y, true); }
pub fn remote_scroll_down(app: &mut AppState, x: u16, y: u16) { remote_scroll_wheel(app, x, y, false); }

pub fn swap_pane(app: &mut AppState, dir: FocusDir) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    
    let mut active_idx = None;
    for (i, (path, _)) in rects.iter().enumerate() { 
        if *path == win.active_path { active_idx = Some(i); break; } 
    }
    let Some(ai) = active_idx else { return; };
    let (_, arect) = &rects[ai];
    
    // Collect pane IDs for MRU-based tie-breaking (issue #70)
    let pane_ids: Vec<usize> = rects.iter().map(|(path, _)| {
        crate::tree::get_active_pane_id(&win.root, path).unwrap_or(usize::MAX)
    }).collect();
    // Try direct neighbour first, then wrap to opposite edge (tmux parity #61)
    let target = crate::input::find_best_pane_in_direction(&rects, ai, arect, dir, &pane_ids, &win.pane_mru)
        .or_else(|| crate::input::find_wrap_target(&rects, ai, arect, dir, &pane_ids, &win.pane_mru));
    if let Some(ni) = target {
        if let Some(new_pane_id) = pane_ids.get(ni) {
            crate::tree::touch_mru(&mut win.pane_mru, *new_pane_id);
        }
        win.active_path = rects[ni].0.clone();
    }
}

pub fn resize_pane_vertical(app: &mut AppState, amount: i16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == LayoutKind::Vertical {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    let new_size = (sizes[idx] as i16 + amount).max(1) as u16;
                    let diff = new_size as i16 - sizes[idx] as i16;
                    sizes[idx] = new_size;
                    if idx + 1 < sizes.len() {
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        sizes[idx - 1] = (sizes[idx - 1] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

pub fn resize_pane_horizontal(app: &mut AppState, amount: i16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == LayoutKind::Horizontal {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    let new_size = (sizes[idx] as i16 + amount).max(1) as u16;
                    let diff = new_size as i16 - sizes[idx] as i16;
                    sizes[idx] = new_size;
                    if idx + 1 < sizes.len() {
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        sizes[idx - 1] = (sizes[idx - 1] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

/// Absolute resize: set the active pane's share to an exact size.
/// axis is "x" (width/horizontal) or "y" (height/vertical).
pub fn resize_pane_absolute(app: &mut AppState, axis: &str, target: u16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    let target_kind = if axis == "x" { LayoutKind::Horizontal } else { LayoutKind::Vertical };
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == target_kind {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    let old = sizes[idx];
                    let new = target.max(1);
                    let diff = new as i16 - old as i16;
                    sizes[idx] = new;
                    // Absorb the difference from a neighbour
                    if idx + 1 < sizes.len() {
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        sizes[idx - 1] = (sizes[idx - 1] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

pub fn rotate_panes(app: &mut AppState, reverse: bool) {
    let win = &mut app.windows[app.active_idx];
    match &mut win.root {
        Node::Split { children, .. } if children.len() >= 2 => {
            if reverse {
                // Rotate counter-clockwise: first element goes to end
                let first = children.remove(0);
                children.push(first);
            } else {
                // Rotate clockwise: last element goes to front
                let last = children.pop().unwrap();
                children.insert(0, last);
            }
        }
        _ => {}
    }
}

pub fn break_pane_to_window(app: &mut AppState) {
    let src_idx = app.active_idx;
    let src_path = app.windows[src_idx].active_path.clone();
    
    // Extract the active pane from the current window using tree operations
    let src_root = std::mem::replace(&mut app.windows[src_idx].root,
        Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
    let (remaining, extracted) = crate::tree::extract_node(src_root, &src_path);
    
    if let Some(pane_node) = extracted {
        let src_empty = remaining.is_none();
        if let Some(rem) = remaining {
            app.windows[src_idx].root = rem;
            app.windows[src_idx].active_path = crate::tree::first_leaf_path(&app.windows[src_idx].root);
        }
        
        // Determine the window name from the pane
        let win_name = match &pane_node {
            Node::Leaf(p) => p.title.clone(),
            _ => format!("win {}", app.windows.len() + 1),
        };
        
        // Create new window containing the extracted pane
        let initial_mru = crate::tree::collect_pane_ids(&pane_node);
        app.windows.push(Window {
            root: pane_node,
            active_path: vec![],
            name: win_name,
            id: app.next_win_id,
            activity_flag: false,
            bell_flag: false,
            silence_flag: false,
            last_output_time: std::time::Instant::now(),
            last_seen_version: 0,
            manual_rename: false,
            layout_index: 0,
            pane_mru: initial_mru,
            zoom_saved: None,
        });
        app.next_win_id += 1;
        
        if src_empty {
            app.windows.remove(src_idx);
        }
        
        // Switch to the new window
        app.active_idx = app.windows.len() - 1;
    } else {
        // Extraction failed — restore
        if let Some(rem) = remaining {
            app.windows[src_idx].root = rem;
        }
    }
}

pub fn respawn_active_pane(app: &mut AppState, pty_system_ref: Option<&dyn portable_pty::PtySystem>) -> io::Result<()> {
    // Reuse provided PTY system or create one as fallback
    let owned_pty;
    let pty_system: &dyn portable_pty::PtySystem = if let Some(ps) = pty_system_ref {
        ps
    } else {
        owned_pty = native_pty_system();
        &*owned_pty
    };
    // Expand format variables like #{pane_current_path} at spawn time (#111).
    // Must happen before the mutable borrow of app.windows below.
    let expanded_shell = crate::format::expand_format(&app.default_shell, &app);

    let win = &mut app.windows[app.active_idx];
    let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) else { return Ok(()); };
    let pane_id = pane.id;
    
    let size = PtySize { rows: pane.last_rows, cols: pane.last_cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    let mut shell_cmd = if !expanded_shell.is_empty() {
        build_default_shell(&expanded_shell, app.env_shim)
    } else {
        detect_shell()
    };
    set_tmux_env(&mut shell_cmd, pane_id, app.control_port, app.socket_name.as_deref(), &app.session_name, app.claude_code_fix_tty, app.claude_code_force_interactive);
    crate::pane::apply_user_environment(&mut shell_cmd, &app.environment);
    let child = pair.slave.spawn_command(shell_cmd).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    // Close the slave handle immediately – required for ConPTY.
    drop(pair.slave);
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, app.history_limit)));
    let term_reader = term.clone();
    let reader = pair.master.try_clone_reader().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(crate::pane::CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    
    crate::pane::spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer);
    
    let mut pty_writer = pair.master.take_writer().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    crate::pane::conpty_preemptive_dsr_response(&mut *pty_writer);
    
    pane.master = pair.master;
    pane.writer = pty_writer;
    pane.child = child;
    pane.term = term;
    pane.data_version = data_version;
    pane.cursor_shape = cursor_shape;
    pane.bell_pending = bell_pending;
    pane.child_pid = None;
    pane.vt_bridge_cache = None;
    pane.vti_mode_cache = None;
    pane.mouse_input_cache = None;
    pane.dead = false;
    
    Ok(())
}
