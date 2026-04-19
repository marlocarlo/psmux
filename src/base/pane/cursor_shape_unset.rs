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

pub const CURSOR_SHAPE_UNSET: u8 = 255;

/// Send a preemptive cursor-position report (\x1b[1;1R) to the ConPTY input pipe.
///
/// Windows ConPTY sends a Device Status Report (\x1b[6n]) during initialization
/// and **blocks** until the host responds with a cursor-position report.  In
/// portable-pty ≤0.2 this was handled internally, but 0.9+ exposes raw handles
/// and the host must respond.  Writing the response preemptively (before the
/// reader thread even starts) is safe because the data sits in the pipe buffer
/// and ConPTY reads it when ready.
pub fn conpty_preemptive_dsr_response(writer: &mut dyn std::io::Write) {
    let _ = writer.write_all(b"\x1b[1;1R");
    let _ = writer.flush();
}

/// Cached resolved shell path to avoid repeated `which::which()` PATH scans.
/// Resolved once on first use, reused for all subsequent pane spawns.
pub(crate) static CACHED_SHELL_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Get the cached shell path, resolving via `which` only on first call.
pub fn cached_shell() -> Option<&'static str> {
    CACHED_SHELL_PATH.get_or_init(|| {
        which::which("pwsh").ok()
            .or_else(|| which::which("powershell").ok())
            .or_else(|| which::which("cmd").ok())
            .map(|p| p.to_string_lossy().into_owned())
    }).as_deref()
}

/// Determine the default shell name for window naming (like tmux shows "bash", "zsh").
pub(crate) fn default_shell_name(command: Option<&str>, configured_shell: Option<&str>) -> String {
    if let Some(cmd) = command {
        // Extract the program name from the command string (space-aware)
        let (prog, _) = resolve_shell_program(cmd);
        std::path::Path::new(&prog)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(cmd)
            .to_string()
    } else if let Some(shell) = configured_shell {
        // Use configured default-shell name (space-aware)
        let (prog, _) = resolve_shell_program(shell);
        std::path::Path::new(&prog)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(shell)
            .to_string()
    } else {
        // Default shell — use cached resolved path
        cached_shell()
            .and_then(|p| std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "shell".into())
    }
}

pub fn create_window(pty_system: &dyn portable_pty::PtySystem, app: &mut AppState, command: Option<&str>, start_dir: Option<&str>) -> io::Result<()> {
    // ── Fast path: use pre-spawned warm pane when creating a default shell ──
    // The warm pane has its shell already loaded (~470ms for pwsh), so the
    // prompt appears instantly — matching wezterm's "instant tab" feel.
    if command.is_none() && start_dir.is_none() && app.warm_pane.is_some() {
        let wp = app.warm_pane.take().unwrap();
        // Resize to current terminal dimensions if they changed since pre-spawn
        let area = app.last_window_area;
        let rows = if area.height > 1 { area.height } else { 30 }.max(MIN_PANE_DIM);
        let cols = if area.width > 1 { area.width } else { 120 }.max(MIN_PANE_DIM);
        if rows != wp.rows || cols != wp.cols {
            let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
            wp.master.resize(size).ok();
            // Resize the vt100 parser too — otherwise it stays at the
            // old warm-pane dimensions while last_rows/last_cols are
            // set to the new size, causing resize_all_panes to skip
            // it (dimensions already match) and the parser to render
            // rows/cols beyond its grid as blank spaces.
            if let Ok(mut parser) = wp.term.lock() {
                parser.screen_mut().set_size(rows, cols);
            }
        }
        let epoch = std::time::Instant::now() - Duration::from_secs(2);
        let configured_shell = if app.default_shell.is_empty() { None } else { Some(app.default_shell.as_str()) };
        let pane = Pane { master: wp.master, writer: wp.writer, child: wp.child, term: wp.term, last_rows: rows, last_cols: cols, id: wp.pane_id, title: hostname_cached(), title_locked: false, child_pid: wp.child_pid, data_version: wp.data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape: wp.cursor_shape, bell_pending: wp.bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring: wp.output_ring };
        let win_name = default_shell_name(None, configured_shell);
        let initial_pane_id = wp.pane_id;
        app.windows.push(Window { root: Node::Leaf(pane), active_path: vec![], name: win_name, id: app.next_win_id, activity_flag: false, bell_flag: false, silence_flag: false, last_output_time: std::time::Instant::now(), last_seen_version: 0, manual_rename: false, layout_index: 0, pane_mru: vec![initial_pane_id], zoom_saved: None, linked_from: None });
        app.next_win_id += 1;
        app.active_idx = app.windows.len() - 1;
        return Ok(());
    }
    // ── Normal path: spawn a new ConPTY + shell synchronously ──
    // Use actual terminal size if known, otherwise fall back to defaults
    let area = app.last_window_area;
    let rows = if area.height > 1 { area.height } else { 30 }.max(MIN_PANE_DIM);
    let cols = if area.width > 1 { area.width } else { 120 }.max(MIN_PANE_DIM);
    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system
        .openpty(size)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;

    // When no explicit command is given, use the configured default-shell
    // (from `set -g default-shell` / `default-command`).
    // Expand format variables like #{pane_current_path} at spawn time (#111).
    let expanded_shell = crate::format::expand_format(&app.default_shell, app);
    let mut shell_cmd = if command.is_some() {
        build_command(command, app.env_shim, app.allow_predictions)
    } else if !expanded_shell.is_empty() {
        build_default_shell(&expanded_shell, app.env_shim, app.allow_predictions)
    } else {
        build_command(None, app.env_shim, app.allow_predictions)
    };
    // Override CWD if -c start_dir was specified
    if let Some(dir) = start_dir {
        shell_cmd.cwd(std::path::Path::new(dir));
    }
    set_tmux_env(&mut shell_cmd, app.next_pane_id, app.control_port, app.socket_name.as_deref(), &app.session_name, app.claude_code_fix_tty, app.claude_code_force_interactive);
    apply_user_environment(&mut shell_cmd, &app.environment);
    let child = pair
        .slave
        .spawn_command(shell_cmd)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    // On Windows ConPTY the slave handle MUST be closed after spawning so the
    // child owns the sole reference to the console input pipe.  Leaving it open
    // causes "The handle is invalid" IOExceptions inside the child process.
    drop(pair.slave);

    let scrollback = app.history_limit as u32;
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, scrollback as usize)));
    let term_reader = term.clone();
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;

    let output_ring = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<u8>::new()));
    spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer, output_ring.clone());

    let configured_shell = if app.default_shell.is_empty() { None } else { Some(app.default_shell.as_str()) };
    let child_pid = crate::platform::mouse_inject::get_child_pid(&*child);
    let mut pty_writer = pair.master.take_writer()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    conpty_preemptive_dsr_response(&mut *pty_writer);
    let epoch = std::time::Instant::now() - Duration::from_secs(2);
    let pane_id = app.next_pane_id;
    let pane = Pane { master: pair.master, writer: pty_writer, child, term, last_rows: size.rows, last_cols: size.cols, id: pane_id, title: hostname_cached(), title_locked: false, child_pid, data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape, bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring };
    app.next_pane_id += 1;
    let win_name = command.map(|c| default_shell_name(Some(c), None)).unwrap_or_else(|| default_shell_name(None, configured_shell));
    app.windows.push(Window { root: Node::Leaf(pane), active_path: vec![], name: win_name, id: app.next_win_id, activity_flag: false, bell_flag: false, silence_flag: false, last_output_time: std::time::Instant::now(), last_seen_version: 0, manual_rename: false, layout_index: 0, pane_mru: vec![pane_id], zoom_saved: None, linked_from: None });
    app.next_win_id += 1;
    app.active_idx = app.windows.len() - 1;
    Ok(())
}

/// Pre-spawn a shell in the background so the next `new-window` (default shell,
/// no custom command) can transplant it instantly.  The returned `WarmPane` has
/// its reader thread already running — by the time the user creates a new window
/// (typically 500ms+), pwsh will have fully loaded its profile and the prompt
/// is ready.
pub fn spawn_warm_pane(pty_system: &dyn portable_pty::PtySystem, app: &mut AppState) -> io::Result<crate::types::WarmPane> {
    if !app.warm_enabled {
        return Err(io::Error::new(io::ErrorKind::Other, "warm panes disabled"));
    }
    let area = app.last_window_area;
    let rows = if area.height > 1 { area.height } else { 30 }.max(MIN_PANE_DIM);
    let cols = if area.width > 1 { area.width } else { 120 }.max(MIN_PANE_DIM);
    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system
        .openpty(size)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    // Expand format variables like #{pane_current_path} at spawn time (#111).
    let expanded_shell = crate::format::expand_format(&app.default_shell, app);
    let mut shell_cmd = if !expanded_shell.is_empty() {
        build_default_shell(&expanded_shell, app.env_shim, app.allow_predictions)
    } else {
        build_command(None, app.env_shim, app.allow_predictions)
    };
    let pane_id = app.next_pane_id;
    app.next_pane_id += 1;
    set_tmux_env(&mut shell_cmd, pane_id, app.control_port, app.socket_name.as_deref(), &app.session_name, app.claude_code_fix_tty, app.claude_code_force_interactive);
    apply_user_environment(&mut shell_cmd, &app.environment);
    let child = pair.slave
        .spawn_command(shell_cmd)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    drop(pair.slave);
    let scrollback = app.history_limit as u32;
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, scrollback as usize)));
    let term_reader = term.clone();
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    let reader = pair.master
        .try_clone_reader()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    let output_ring = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<u8>::new()));
    spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer, output_ring.clone());
    let child_pid = crate::platform::mouse_inject::get_child_pid(&*child);
    let mut pty_writer = pair.master.take_writer()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    conpty_preemptive_dsr_response(&mut *pty_writer);
    Ok(crate::types::WarmPane { master: pair.master, writer: pty_writer, child, term, data_version, cursor_shape, bell_pending, child_pid, pane_id, rows, cols, output_ring })
}

pub fn split_active(app: &mut AppState, kind: LayoutKind) -> io::Result<()> {
    split_active_with_command(app, kind, None, None, None)
}

/// Create a new window with a raw command (program + args, no shell wrapping)
pub fn create_window_raw(pty_system: &dyn portable_pty::PtySystem, app: &mut AppState, raw_args: &[String]) -> io::Result<()> {
    let area = app.last_window_area;
    let rows = if area.height > 1 { area.height } else { 30 };
    let cols = if area.width > 1 { area.width } else { 120 };
    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system
        .openpty(size)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;

    let mut shell_cmd = build_raw_command(raw_args);
    set_tmux_env(&mut shell_cmd, app.next_pane_id, app.control_port, app.socket_name.as_deref(), &app.session_name, app.claude_code_fix_tty, app.claude_code_force_interactive);
    apply_user_environment(&mut shell_cmd, &app.environment);
    let child = pair
        .slave
        .spawn_command(shell_cmd)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    // Close the slave handle immediately – see create_window() comment.
    drop(pair.slave);

    let scrollback = app.history_limit;
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, scrollback)));
    let term_reader = term.clone();
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;

    let output_ring = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<u8>::new()));
    spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer, output_ring.clone());

    let child_pid = crate::platform::mouse_inject::get_child_pid(&*child);
    let mut pty_writer = pair.master.take_writer()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    conpty_preemptive_dsr_response(&mut *pty_writer);
    let epoch = std::time::Instant::now() - Duration::from_secs(2);
    let raw_pane_id = app.next_pane_id;
    let pane = Pane { master: pair.master, writer: pty_writer, child, term, last_rows: size.rows, last_cols: size.cols, id: raw_pane_id, title: hostname_cached(), title_locked: false, child_pid, data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape, bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring };
    app.next_pane_id += 1;
    let win_name = std::path::Path::new(&raw_args[0]).file_stem().and_then(|s| s.to_str()).unwrap_or(&raw_args[0]).to_string();
    app.windows.push(Window { root: Node::Leaf(pane), active_path: vec![], name: win_name, id: app.next_win_id, activity_flag: false, bell_flag: false, silence_flag: false, last_output_time: std::time::Instant::now(), last_seen_version: 0, manual_rename: false, layout_index: 0, pane_mru: vec![raw_pane_id], zoom_saved: None, linked_from: None });
    app.next_win_id += 1;
    app.active_idx = app.windows.len() - 1;
    Ok(())
}

/// Minimum pane dimension (rows or cols) — ConPTY on Windows crashes
/// the child process if either dimension is less than 2.
pub const MIN_PANE_DIM: u16 = 2;

/// Minimum rows for a split to be allowed — each resulting pane needs at
/// least this many rows to run a shell prompt.
pub(crate) const MIN_SPLIT_ROWS: u16 = 2;

/// Minimum cols for a split to be allowed.
pub(crate) const MIN_SPLIT_COLS: u16 = 10;
