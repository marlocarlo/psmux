use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::types::{AppState, Pane, Node, LayoutKind, Window};
use crate::tree::{replace_leaf_with_split, active_pane_mut, kill_leaf};

/// Sentinel value for cursor_shape: means "no DECSCUSR received from child yet".
/// When ConPTY passthrough mode is unavailable, DECSCUSR sequences from child
/// processes are consumed by ConPTY and never forwarded.  Using this sentinel
/// lets the rendering code skip emitting any cursor-shape override, so the
/// real terminal keeps its user-configured default cursor.
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
static CACHED_SHELL_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Get the cached shell path, resolving via `which` only on first call.
fn cached_shell() -> Option<&'static str> {
    CACHED_SHELL_PATH.get_or_init(|| {
        which::which("pwsh").ok()
            .or_else(|| which::which("cmd").ok())
            .map(|p| p.to_string_lossy().into_owned())
    }).as_deref()
}

/// Determine the default shell name for window naming (like tmux shows "bash", "zsh").
fn default_shell_name(command: Option<&str>, configured_shell: Option<&str>) -> String {
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
        let pane = Pane { master: wp.master, writer: wp.writer, child: wp.child, term: wp.term, last_rows: rows, last_cols: cols, id: wp.pane_id, title: format!("pane %{}", wp.pane_id), title_locked: false, child_pid: wp.child_pid, data_version: wp.data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape: wp.cursor_shape, bell_pending: wp.bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring: wp.output_ring };
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
    let pane = Pane { master: pair.master, writer: pty_writer, child, term, last_rows: size.rows, last_cols: size.cols, id: pane_id, title: format!("pane %{}", pane_id), title_locked: false, child_pid, data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape, bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring };
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
    let pane = Pane { master: pair.master, writer: pty_writer, child, term, last_rows: size.rows, last_cols: size.cols, id: raw_pane_id, title: format!("pane %{}", raw_pane_id), title_locked: false, child_pid, data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape, bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring };
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
const MIN_SPLIT_ROWS: u16 = 4;
/// Minimum cols for a split to be allowed.
const MIN_SPLIT_COLS: u16 = 10;

pub fn split_active_with_command(app: &mut AppState, kind: LayoutKind, command: Option<&str>, pty_system_ref: Option<&dyn portable_pty::PtySystem>, start_dir: Option<&str>) -> io::Result<()> {
    // ── Guard: refuse split if the active pane is too small ──────────
    // After splitting, each half gets roughly (dim / 2) - 1 (for the divider).
    // If that would be below MIN_PANE_DIM, deny the split to avoid crashing
    // the child process (ConPTY cannot function below ~2 rows or cols).
    {
        let win = &app.windows[app.active_idx];
        if let Some(p) = crate::tree::active_pane(&win.root, &win.active_path) {
            let (cur_rows, cur_cols) = (p.last_rows, p.last_cols);
            match kind {
                LayoutKind::Vertical => {
                    // Splitting vertically divides height; need room for 2 panes + 1 divider
                    if cur_rows < MIN_SPLIT_ROWS * 2 + 1 {
                        return Err(io::Error::new(io::ErrorKind::Other,
                            format!("pane too small to split vertically ({cur_rows} rows, need {})", MIN_SPLIT_ROWS * 2 + 1)));
                    }
                }
                LayoutKind::Horizontal => {
                    // Splitting horizontally divides width; need room for 2 panes + 1 divider
                    if cur_cols < MIN_SPLIT_COLS * 2 + 1 {
                        return Err(io::Error::new(io::ErrorKind::Other,
                            format!("pane too small to split horizontally ({cur_cols} cols, need {})", MIN_SPLIT_COLS * 2 + 1)));
                    }
                }
            }
        }
    }

    // Reuse provided PTY system or create one as fallback
    let owned_pty;
    let pty_system: &dyn portable_pty::PtySystem = if let Some(ps) = pty_system_ref {
        ps
    } else {
        owned_pty = native_pty_system();
        &*owned_pty
    };
    // Compute target pane size from the *active pane's* actual dimensions,
    // not the full window area — ensures we don't over-estimate and then
    // immediately resize to a tiny rect.
    let (pane_rows, pane_cols) = {
        let win = &app.windows[app.active_idx];
        if let Some(p) = crate::tree::active_pane(&win.root, &win.active_path) {
            (p.last_rows, p.last_cols)
        } else {
            let area = app.last_window_area;
            (if area.height > 1 { area.height } else { 30 }, if area.width > 1 { area.width } else { 120 })
        }
    };
    let (rows, cols) = match kind {
        LayoutKind::Vertical => {
            let half = (pane_rows.saturating_sub(1)) / 2; // subtract 1 for divider
            (half.max(MIN_PANE_DIM), pane_cols.max(MIN_PANE_DIM))
        }
        LayoutKind::Horizontal => {
            let half = (pane_cols.saturating_sub(1)) / 2;
            (pane_rows.max(MIN_PANE_DIM), half.max(MIN_PANE_DIM))
        }
    };
    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };

    // ── Fast path: transplant warm pane for default-shell splits ─────
    // The warm pane has its shell already loaded (~470ms for pwsh).  Even
    // though its ConPTY was created at full-window size, resizing to the
    // split dimensions only costs a ConPTY repaint (~10-50ms) vs a full
    // cold spawn (~500ms).  Net result: split feels nearly instant.
    // Skip warm pane when start_dir is set — the warm pane was spawned
    // in the server's CWD, not the requested directory (#107).
    if command.is_none() && start_dir.is_none() && app.warm_pane.is_some() {
        let wp = app.warm_pane.take().unwrap();
        // Resize ConPTY + parser to the split dimensions
        if rows != wp.rows || cols != wp.cols {
            let sz = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
            wp.master.resize(sz).ok();
            if let Ok(mut parser) = wp.term.lock() {
                parser.screen_mut().set_size(rows, cols);
            }
        }
        let epoch = std::time::Instant::now() - Duration::from_secs(2);
        let new_pane_id = wp.pane_id;
        let new_leaf = Node::Leaf(Pane { master: wp.master, writer: wp.writer, child: wp.child, term: wp.term, last_rows: rows, last_cols: cols, id: new_pane_id, title: format!("pane %{}", new_pane_id), title_locked: false, child_pid: wp.child_pid, data_version: wp.data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape: wp.cursor_shape, bell_pending: wp.bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring: wp.output_ring });
        let win = &mut app.windows[app.active_idx];
        replace_leaf_with_split(&mut win.root, &win.active_path, kind, new_leaf);
        let mut new_path = win.active_path.clone();
        new_path.push(1);
        win.active_path = new_path;
        // Add new pane to MRU (most recent)
        crate::tree::touch_mru(&mut win.pane_mru, new_pane_id);
        return Ok(());
    }

    // ── Normal path: cold-spawn a new ConPTY + shell ────────────────
    let pair = pty_system.openpty(size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    // When no explicit command is given, use the configured default-shell.
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
    let child = pair.slave.spawn_command(shell_cmd).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    // Close the slave handle immediately – see create_window() comment.
    drop(pair.slave);
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, app.history_limit)));
    let term_reader = term.clone();
    let reader = pair.master.try_clone_reader().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    let output_ring = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<u8>::new()));
    spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer, output_ring.clone());
    let child_pid = crate::platform::mouse_inject::get_child_pid(&*child);
    let mut pty_writer = pair.master.take_writer()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    conpty_preemptive_dsr_response(&mut *pty_writer);
    let epoch = std::time::Instant::now() - Duration::from_secs(2);
    let split_pane_id = app.next_pane_id;
    let new_leaf = Node::Leaf(Pane { master: pair.master, writer: pty_writer, child, term, last_rows: size.rows, last_cols: size.cols, id: split_pane_id, title: format!("pane %{}", split_pane_id), title_locked: false, child_pid, data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape, bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring });
    app.next_pane_id += 1;
    let win = &mut app.windows[app.active_idx];
    replace_leaf_with_split(&mut win.root, &win.active_path, kind, new_leaf);
    let mut new_path = win.active_path.clone();
    new_path.push(1);
    win.active_path = new_path;
    // Add new pane to MRU (most recent)
    crate::tree::touch_mru(&mut win.pane_mru, split_pane_id);
    Ok(())
}

fn kill_pane_at_path(win: &mut Window, path: &Vec<usize>) {
    // Get the ID of the pane being killed (for MRU removal)
    let killed_id = crate::tree::get_active_pane_id(&win.root, path);
    // Explicitly kill the target pane's process tree FIRST.
    // remove_node() doesn't call kill_node() when the root is a single Leaf,
    // so we must do it here to ensure no orphaned processes.
    if let Some(p) = active_pane_mut(&mut win.root, path) {
        crate::platform::process_kill::kill_process_tree(&mut p.child);
    }
    kill_leaf(&mut win.root, path);
    // Remove killed pane from MRU
    if let Some(kid) = killed_id {
        crate::tree::remove_from_mru(&mut win.pane_mru, kid);
    }
    // Focus the most recently used remaining pane (tmux parity #71).
    // Walk the MRU list and pick the first pane that still exists.
    let mru_target = win.pane_mru.iter()
        .find_map(|&id| crate::tree::find_path_by_id(&win.root, id));
    win.active_path = mru_target
        .unwrap_or_else(|| crate::tree::first_leaf_path(&win.root));
}

pub fn kill_active_pane(app: &mut AppState) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    let active_path = win.active_path.clone();
    kill_pane_at_path(win, &active_path);
    Ok(())
}

pub fn kill_pane_by_id(app: &mut AppState, pane_id: usize) -> io::Result<()> {
    let restore_idx = app.active_idx;
    let restore_path = app.windows[restore_idx].active_path.clone();
    let restore_pane_id = crate::tree::get_active_pane_id(&app.windows[restore_idx].root, &restore_path);

    let target = app.windows.iter().enumerate().find_map(|(wi, win)| {
        crate::tree::find_path_by_id(&win.root, pane_id).map(|path| (wi, path))
    });

    let Some((target_idx, target_path)) = target else {
        return Ok(());
    };

    {
        let win = &mut app.windows[target_idx];
        kill_pane_at_path(win, &target_path);
    }

    // Only restore focus when the killed pane was in a DIFFERENT window.
    // For same-window kills, kill_pane_at_path already set the correct
    // MRU-based focus.  The old restore logic used path_exists() which
    // can succeed on stale indices that now point to a different pane
    // after tree restructuring (issue #140).
    if restore_idx < app.windows.len() && target_idx != restore_idx {
        app.active_idx = restore_idx;
        let restore_win = &mut app.windows[restore_idx];
        let resolved_restore_path = restore_pane_id
            .and_then(|id| crate::tree::find_path_by_id(&restore_win.root, id))
            .unwrap_or_else(|| crate::tree::first_leaf_path(&restore_win.root));
        restore_win.active_path = resolved_restore_path;
    }

    Ok(())
}

pub fn detect_shell() -> CommandBuilder {
    build_command(None, false, false)
}

/// Set TMUX, TMUX_PANE, and PSMUX_SESSION environment variables on a CommandBuilder.
/// TMUX format: /tmp/psmux-{server_pid}/{socket_name},{port},0
/// TMUX_PANE format: %{pane_id}
/// PSMUX_SESSION: actual session name (for Claude Code / tool detection)
/// The socket_name component encodes the -L namespace for child process resolution.
pub fn set_tmux_env(builder: &mut CommandBuilder, pane_id: usize, control_port: Option<u16>, socket_name: Option<&str>, session_name: &str, fix_tty: bool, _force_interactive: bool) {
    let server_pid = std::process::id();
    let port = control_port.unwrap_or(0);
    let sn = socket_name.unwrap_or("default");
    // Format compatible with tmux: <socket_path>,<pid>,<session_idx>
    // We encode the socket name in the path component for -L namespace resolution
    builder.env("TMUX", format!("/tmp/psmux-{}/{},{},0", server_pid, sn, port));
    builder.env("TMUX_PANE", format!("%{}", pane_id));
    // Override the placeholder "1" from build_command/build_default_shell with the
    // real session name.  Tools like Claude Code can use PSMUX_SESSION for explicit
    // psmux detection (e.g. `if (process.env.PSMUX_SESSION) return 'psmux'`).
    builder.env("PSMUX_SESSION", session_name);
    // Prevent MSYS2/Git-Bash from path-mangling the TMUX value (which starts
    // with /tmp/ and would be rewritten to a Windows path otherwise).
    builder.env("MSYS2_ENV_CONV_EXCL", "TMUX");
    // Enable Claude Code agent teams feature.  The standalone binary gates
    // the entire teammate tool-set (spawnTeam, spawnTeammate, …) behind
    //   T8(): LA(process.env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS) || --agent-teams
    // Without this env var the team tools are never registered and Claude
    // always falls back to the in-process "Agent" tool.
    builder.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");

    // ── Claude Code workarounds (removable once upstream fixes land) ──
    //
    // claude-code-fix-tty (set -g claude-code-fix-tty on/off):
    //   Claude Code v2.1.71 standalone binary ignores `teammateMode` from
    //   settings.json (config schema strips the field).  The `--teammate-mode
    //   tmux` CLI flag DOES work.  We set PSMUX_CLAUDE_TEAMMATE_MODE=tmux so
    //   the PowerShell env-shim `claude` wrapper function injects the flag
    //   automatically.  Disable with: set -g claude-code-fix-tty off
    if fix_tty {
        builder.env("PSMUX_CLAUDE_TEAMMATE_MODE", "tmux");
    }

}

/// Apply user-defined environment variables (from set-environment -g) to a CommandBuilder.
/// This ensures variables set via config or runtime `set-environment` are explicitly
/// passed to every child pane, in addition to process inheritance.
pub fn apply_user_environment(builder: &mut CommandBuilder, environment: &std::collections::HashMap<String, String>) {
    for (key, value) in environment {
        builder.env(key, value);
    }
}

/// PowerShell env shim snippet — defines a `Global:env` function that translates
/// POSIX `env VAR=val ... command args` invocations into PowerShell equivalents.
/// Only defined when no native `env.exe` is found on PATH.
///
/// Key design decisions for Windows + Claude Code agent teams compatibility:
///   1. POSIX backslash-escape removal uses `\\([^\w\\])` so that escapes like
///      `\@` and `\:` (produced by shell-quote) are stripped, while Windows
///      path separators (`\U` in `C:\Users`) are preserved (letter after `\`
///      is a `\w` character, so the regex does NOT match).
///   2. Escape stripping is applied to ALL arguments (env var values, the
///      command itself, and every trailing arg), not just env-var values.
///   3. `.js` / `.mjs` files are detected and automatically executed via
///      `node` because Windows associates `.js` with WScript.exe (WSH),
///      which cannot run Node.js code and instead shows error dialogs.
const ENV_SHIM_PS: &str = concat!(
    "if(-not(Get-Command env -EA 0 -Type Application)){ ",
    "function Global:env { ",
    // _pu: POSIX-unescape helper — strips `\` before non-word, non-backslash
    // chars (e.g. \@ → @, \: → :) produced by npm shell-quote.
    // SKIPS Windows absolute paths (C:\...) where `\` is a directory
    // separator, not a POSIX escape.  On Linux paths use `/` so
    // there's never a collision; on Windows `\@` in a path like
    // `node_modules\@anthropic-ai` must be preserved.
    "function _pu($s){if($s -match '^[A-Za-z]:\\\\'){return $s}; $s -replace '\\\\([^\\w\\\\])','$1'}; ",
    // _shebang: reads the first line of a script file and extracts the
    // interpreter, mimicking Linux kernel shebang execution.
    // Handles #!/usr/bin/env node, #!/usr/bin/node, #!/usr/bin/env deno, etc.
    "function _shebang($f){ ",
    "try{ $l=(Get-Content $f -TotalCount 1 -EA Stop); ",
    "if($l -match '^#!\\s*(.+)$'){ ",
    "$p=$Matches[1].Trim(); ",
    "if($p -match '/env\\s+(.+)$'){return ($Matches[1].Trim()-split'\\s+')[0]}; ",
    "return ($p-split'/')[-1] } }catch{}; $null }; ",
    "$v=@{}; $i=0; ",
    "while($i -lt $args.Count){ ",
    "if([string]$args[$i] -match '^([A-Za-z_]\\w*)=(.*)$'){ ",
    "$v[$Matches[1]]=(_pu $Matches[2]); $i++ ",
    "} else { break } }; ",
    "if($i -lt $args.Count){ ",
    "foreach($e in $v.GetEnumerator()){[Environment]::SetEnvironmentVariable($e.Key,$e.Value,'Process')}; ",
    "$cmd=(_pu ([string]$args[$i])); $rest=@(); ",
    "if($i+1 -lt $args.Count){$rest=@($args[($i+1)..($args.Count-1)]|ForEach-Object{_pu ([string]$_)})}; ",
    // For script files (.js/.mjs/.ts/.sh/.py/etc), read the shebang line
    // to determine the interpreter — exactly like Linux kernel does.
    // Falls back to node for .js/.mjs only if no shebang is found
    // (since Windows associates .js with WScript.exe, not node).
    "$interp=$null; ",
    "$resolved=$cmd; if($cmd -match '^''(.+)''$'){$resolved=$Matches[1]}; ",
    "if(Test-Path $resolved -EA 0){$interp=(_shebang $resolved)}; ",
    "if($interp){& $interp $cmd @rest} ",
    "elseif($cmd -match '\\.m?js$'){& node $cmd @rest} ",
    "else{& $cmd @rest} ",
    "} elseif($v.Count -gt 0){ ",
    "foreach($e in $v.GetEnumerator()){[Environment]::SetEnvironmentVariable($e.Key,$e.Value,'Process')} ",
    "} else { Get-ChildItem Env:|ForEach-Object{$_.Name+'='+$_.Value} } } }; ",
    // Claude Code teammate-mode wrapper (claude-code#26244):
    // The standalone (Bun SFE) binary ignores `teammateMode` from settings.json
    // but honours the `--teammate-mode tmux` CLI flag.  The agent teams tool-set
    // is separately gated by CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS env var (set
    // above in set_tmux_env).  This wrapper auto-injects --teammate-mode when
    // PSMUX_CLAUDE_TEAMMATE_MODE is set (via `set -g claude-code-fix-tty on`).
    // Disable with: set -g claude-code-fix-tty off
    "if($env:PSMUX_CLAUDE_TEAMMATE_MODE){ ",
    "function Global:claude { ",
    "if($args -contains '--teammate-mode'){ & claude.exe @args } ",
    "else{ & claude.exe --teammate-mode $env:PSMUX_CLAUDE_TEAMMATE_MODE @args } } }",
);

/// PSReadLine prediction fix — disables predictions that crash with
/// NullReferenceException in GetHistoryItems() during ConPTY startup.
/// See https://github.com/psmux/psmux/issues/109
const PSRL_FIX: &str = concat!(
    "try { Set-PSReadLineOption -PredictionSource None -ErrorAction Stop } catch {}; ",
    "try { Set-PSReadLineOption -PredictionViewStyle InlineView -ErrorAction Stop } catch {}; ",
    "try { Remove-PSReadLineKeyHandler -Chord 'F2' -ErrorAction Stop } catch {}",
);

/// Minimal crash guard: saves the user's original PredictionSource, then
/// disables predictions to prevent the #109 NullReferenceException during
/// ConPTY startup.  Does NOT touch PredictionViewStyle or F2 so those stay
/// at whatever the system default is.  Used pre-profile when allow-predictions
/// is on (#150).
const PSRL_CRASH_GUARD: &str = concat!(
    "$Global:__psmux_origPred = try { (Get-PSReadLineOption).PredictionSource } catch { 'History' }; ",
    "try { Set-PSReadLineOption -PredictionSource None -ErrorAction Stop } catch {}",
);

/// Post-profile prediction restore: if PredictionSource is still None (meaning
/// the user's profile did not explicitly set it), restore the saved original.
/// If the profile DID set a value, we leave it alone.
/// Used post-profile when allow-predictions is on (#150).
const PSRL_PRED_RESTORE: &str = concat!(
    "if ((Get-PSReadLineOption).PredictionSource -eq 'None' -and $Global:__psmux_origPred -ne 'None') { ",
    "try { Set-PSReadLineOption -PredictionSource $Global:__psmux_origPred -ErrorAction Stop } catch {} ",
    "}",
);

/// Source all four PowerShell profile scripts in the standard order.
/// Used with -NoProfile to give us control over execution order — we disable
/// PSReadLine predictions BEFORE the profile loads (preventing the
/// GetHistoryItems NullReferenceException), then re-disable after the profile
/// in case the user's profile re-enables predictions.
const PROFILE_SOURCE: &str = concat!(
    "foreach ($__p in @(",
    "$PROFILE.AllUsersAllHosts,",
    "$PROFILE.AllUsersCurrentHost,",
    "$PROFILE.CurrentUserAllHosts,",
    "$PROFILE.CurrentUserCurrentHost",
    ")) { if ($__p -and (Test-Path $__p)) { try { . $__p } catch { Write-Warning \"psmux: profile error in ${__p}: $_\" } } }",
);

/// Sync PowerShell's $PWD to the OS-level CWD (#111).
/// PowerShell's `cd` (Set-Location) only updates `$PWD` internally and
/// does NOT call Win32 SetCurrentDirectory(). This means the process PEB
/// still shows the original spawn directory, causing #{pane_current_path}
/// to always return the initial CWD.
///
/// Instead of wrapping the `prompt` function (which conflicts with prompt
/// customizers like Starship, oh-my-posh, etc.), we wrap the three cmdlets
/// that actually change directories: Set-Location, Push-Location, and
/// Pop-Location.  This is invisible to prompt customizers and survives
/// `. $PROFILE` reloads.
const CWD_SYNC: &str = concat!(
    "if (-not (Test-Path variable:Global:__psmux_cwd_hook)) { ",
    "$Global:__psmux_cwd_hook = $true; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {}; ",
    "function Global:Set-Location { ",
    "Microsoft.PowerShell.Management\\Set-Location @args; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {} ",
    "}; ",
    "function Global:Push-Location { ",
    "Microsoft.PowerShell.Management\\Push-Location @args; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {} ",
    "}; ",
    "function Global:Pop-Location { ",
    "Microsoft.PowerShell.Management\\Pop-Location @args; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {} ",
    "} }",
);

/// Build the full interactive init string for PowerShell:
/// 1. Disable PSReadLine predictions (before profile — prevents #109 crash)
/// 2. Source the user's profile scripts
/// 3. If allow_predictions is false, re-disable predictions after the profile;
///    if allow_predictions is true, restore the saved original PredictionSource
///    only when the profile did not set one explicitly (#150)
/// 4. Install CWD sync hook (enables #{pane_current_path} — #111)
/// 5. Optionally append the env shim
fn build_psrl_init(env_shim: bool, allow_predictions: bool) -> String {
    let (pre_profile, post_profile) = if allow_predictions {
        (PSRL_CRASH_GUARD, PSRL_PRED_RESTORE)
    } else {
        (PSRL_FIX, PSRL_FIX)
    };
    let mut s = format!("{}; {}; {}; {}", pre_profile, PROFILE_SOURCE, post_profile, CWD_SYNC);
    if env_shim {
        s.push_str("; ");
        s.push_str(ENV_SHIM_PS);
    }
    s
}

pub fn build_command(command: Option<&str>, env_shim: bool, allow_predictions: bool) -> CommandBuilder {
    // Capture CWD early — portable_pty on Windows defaults to USERPROFILE
    // (home dir) when no cwd is set on CommandBuilder, so we must set it
    // explicitly to honour the caller's working directory.
    let cwd = std::env::current_dir().ok();
    if let Some(cmd) = command {
        let shell = cached_shell().map(|s| s.to_string());

        match shell {
            Some(path) => {
                let mut builder = CommandBuilder::new(&path);
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");

                let stem = std::path::Path::new(&path).file_stem()
                    .and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if stem == "pwsh" || stem == "powershell" {
                    builder.args(["-NoLogo", "-Command", cmd]);
                } else if matches!(stem.as_str(), "bash" | "sh" | "zsh" | "fish" | "dash" | "ash") {
                    builder.args(["-c", cmd]);
                } else {
                    builder.args(["/C", cmd]);
                }
                builder
            }
            None => {
                let mut builder = CommandBuilder::new("pwsh.exe");
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                builder.args(["-NoLogo", "-Command", cmd]);
                builder
            }
        }
    } else {
        let shell = cached_shell().map(|s| s.to_string());
        // PSReadLine v2.2.6+ enables PredictionSource HistoryAndPlugin by default.
        // Predictions cause display corruption in terminal multiplexers because
        // PSReadLine's VT rendering races with ConPTY output capture.
        // Issue #109: GetHistoryItems() throws NullReferenceException when
        // predictions are enabled in the profile before PSReadLine is fully
        // initialized inside ConPTY.  We use -NoProfile and source profiles
        // ourselves, sandwiching them between prediction-disable commands.
        let psrl_init = build_psrl_init(env_shim, allow_predictions);
        match shell {
            Some(path) => {
                let mut builder = CommandBuilder::new(&path);
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                if path.to_lowercase().contains("pwsh") {
                    builder.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &psrl_init]);
                }
                builder
            }
            None => {
                let mut builder = CommandBuilder::new("pwsh.exe");
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                // Apply the same -NoProfile + manual profile sourcing for
                // the fallback pwsh.exe path (previously had no PSRL fix).
                builder.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &psrl_init]);
                builder
            }
        }
    }
}

/// Cached resolved default-shell path to avoid repeated `which::which()` scans.
static CACHED_DEFAULT_SHELL: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();
static CACHED_DEFAULT_SHELL_MAP: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> = std::sync::Mutex::new(None);

/// Resolve a program name via `which`, caching the result.
fn cached_which(program: &str) -> String {
    // Fast path: check if already cached in the global OnceLock for the default
    // (most common case is always the same shell)
    let mut map = CACHED_DEFAULT_SHELL_MAP.lock().unwrap_or_else(|e| e.into_inner());
    let map = map.get_or_insert_with(std::collections::HashMap::new);
    if let Some(cached) = map.get(program) {
        return cached.clone();
    }
    let resolved = which::which(program).ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    map.insert(program.to_string(), resolved.clone());
    resolved
}

/// Split a shell config value into (program, extra_args), handling paths
/// that contain spaces (e.g. `C:/Program Files/Git/bin/bash.exe`).
///
/// Resolution order:
/// 1. If the whole string resolves to an existing executable, use it as-is.
/// 2. Otherwise, use quote-aware tokenising so that users can write
///    `"C:/Program Files/Git/bin/bash.exe" --login` with quotes.
fn resolve_shell_program(shell_path: &str) -> (String, Vec<String>) {
    // Fast path: whole string is the program (possibly with spaces in path).
    if std::path::Path::new(shell_path).is_file()
        || which::which(shell_path).is_ok()
    {
        return (shell_path.to_string(), vec![]);
    }

    // Quote-aware split (handles `"path with spaces" arg1 arg2`).
    let parsed = crate::commands::parse_command_line(shell_path);
    if parsed.is_empty() {
        return (shell_path.to_string(), vec![]);
    }
    let program = parsed[0].clone();
    let extra = parsed[1..].to_vec();
    (program, extra)
}

/// Build a CommandBuilder that launches the given shell path interactively.
/// Used when `default-shell` / `default-command` is configured.
/// Supports pwsh, powershell, cmd, and any arbitrary executable.
pub fn build_default_shell(shell_path: &str, env_shim: bool, allow_predictions: bool) -> CommandBuilder {
    let (program, extra_args) = resolve_shell_program(shell_path);

    // Resolve bare names via cached `which` — avoids repeated PATH scans.
    let resolved = cached_which(&program);

    let lower = resolved.to_lowercase();
    let mut builder = CommandBuilder::new(&resolved);
    // Set CWD explicitly — portable_pty on Windows defaults to USERPROFILE
    // (home dir) when no cwd is set on CommandBuilder.
    if let Ok(dir) = std::env::current_dir() { builder.cwd(dir); }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("PSMUX_SESSION", "1");

    // Prepend extra arguments (e.g. -NoProfile) BEFORE our -NoExit/-Command block
    // so they're interpreted as flags rather than as -Command arguments.
    if !extra_args.is_empty() {
        builder.args(extra_args.clone());
    }

    if lower.contains("pwsh") || lower.contains("powershell") {
        // Issue #109: -NoProfile + manual profile sourcing to prevent
        // PSReadLine GetHistoryItems NullReferenceException.
        // If the user already passed -NoProfile in extra_args, we still
        // add ours (PowerShell accepts duplicates harmlessly) and skip
        // profile sourcing only if they explicitly opted out.
        let has_noprofile = extra_args.iter()
            .any(|a| a.eq_ignore_ascii_case("-NoProfile"));
        let psrl_init = if has_noprofile {
            // User explicitly wants no profile — just apply PSRL fix + shim.
            let mut s = PSRL_FIX.to_string();
            if env_shim {
                s.push_str("; ");
                s.push_str(ENV_SHIM_PS);
            }
            s
        } else {
            build_psrl_init(env_shim, allow_predictions)
        };
        if !has_noprofile {
            builder.args(["-NoProfile"]);
        }
        builder.args(["-NoLogo", "-NoExit", "-Command", &psrl_init]);
    }

    builder
}

/// Build a CommandBuilder for direct execution (no shell wrapping).
/// raw_args[0] is the program, rest are its arguments.
/// Used when -- separator is specified in new-session.
pub fn build_raw_command(raw_args: &[String]) -> CommandBuilder {
    if raw_args.is_empty() {
        return build_command(None, true, false);
    }
    let program = &raw_args[0];
    let mut builder = CommandBuilder::new(program);
    // Set CWD explicitly — portable_pty on Windows defaults to USERPROFILE
    // (home dir) when no cwd is set on CommandBuilder.
    if let Ok(dir) = std::env::current_dir() { builder.cwd(dir); }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("PSMUX_SESSION", "1");
    if raw_args.len() > 1 {
        let args: Vec<&str> = raw_args[1..].iter().map(|s| s.as_str()).collect();
        builder.args(args);
    }
    builder
}

/// Spawn a dedicated PTY reader thread that processes output and updates the
/// data_version counter. Exits cleanly after 200 consecutive zero-byte reads
/// (indicating the PTY pipe is closed) or on any I/O error.
///
/// Uses an 8KB read buffer (down from 64KB) to reduce mutex hold time during
/// `parser.process()`, which improves DumpState latency under heavy output.

/// Scan raw ConPTY output for DECSCUSR cursor shape sequences (`\x1b[N q`).
/// Returns the last cursor shape value found, or None.
///
/// We accept all DECSCUSR cursor shape values (0-6) from child processes.
/// Value 0 resets to default, 1-2 = block, 3-4 = underline, 5-6 = bar.
fn scan_cursor_shape(data: &[u8]) -> Option<u8> {
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
fn scan_rmcup(data: &[u8]) -> bool {
    const RMCUP: &[u8] = b"\x1b[?1049l";
    data.windows(RMCUP.len()).any(|w| w == RMCUP)
}

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
#[path = "../tests-rs/test_issue151_strict_mode.rs"]
mod test_issue151_strict_mode;

#[cfg(test)]
#[path = "../tests-rs/test_issue155_output_rendering.rs"]
mod test_issue155_output_rendering;

#[cfg(test)]
#[path = "../tests-rs/test_issue165_prediction_view_style.rs"]
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
