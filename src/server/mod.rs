mod helpers;
mod options;
mod connection;

use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::env;
use std::net::TcpListener;

use portable_pty::native_pty_system;
use ratatui::prelude::Rect;

use crate::types::{AppState, CtrlReq, Mode, FocusDir, LayoutKind, PipePaneState, VERSION,
    WaitChannel, WaitForOp, Node, Action, Bind, PopupPty};
use crate::platform::install_console_ctrl_handler;
use crate::pane::{create_window, create_window_raw, split_active_with_command, kill_active_pane, spawn_warm_pane};
use crate::tree::{self, active_pane, active_pane_mut, resize_all_panes, kill_all_children,
    find_window_index_by_id, focus_pane_by_id, focus_pane_by_index, get_active_pane_id,
    path_exists};

use helpers::{collect_pane_paths_server, serialize_bindings_json, json_escape_string,
    list_windows_json_with_tabs, combined_data_version, TMUX_COMMANDS};
use options::{get_option_value, get_window_option_value, render_window_options, apply_set_option};

use crate::input::{send_text_to_active, send_key_to_active, send_paste_to_active, move_focus};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, move_copy_cursor, current_prompt_pos,
    yank_selection, scroll_copy_up, scroll_copy_down, switch_with_copy_save,
    capture_active_pane_text, capture_active_pane_range, capture_active_pane_styled};
use crate::layout::{dump_layout_json, dump_layout_json_fast, apply_layout, cycle_layout,
    cycle_layout_reverse};
use crate::window_ops::{toggle_zoom, remote_mouse_down, remote_mouse_drag, remote_mouse_up,
    remote_mouse_button, remote_mouse_motion, remote_scroll_up, remote_scroll_down,
    swap_pane, break_pane_to_window, unzoom_if_zoomed, resize_pane_vertical,
    resize_pane_horizontal, resize_pane_absolute, rotate_panes, respawn_active_pane};
use crate::config::{load_config, parse_key_string, format_key_binding, normalize_key_for_binding,
    parse_config_content, parse_config_line};
use crate::commands::{parse_command_to_action, format_action, parse_menu_definition};
use crate::util::{list_windows_json, list_tree_json, list_windows_tmux, base64_encode};
use crate::format::{expand_format, format_list_windows, format_list_panes, set_buffer_idx_override};
use crate::help;

fn should_spawn_warm_server(app: &AppState) -> bool {
    app.session_name != "__warm__" && !app.destroy_unattached
}

/// Spawn a standby "warm server" process that pre-loads config + shell.
/// When `psmux new-session` is run later, the CLI claims this warm server
/// via `claim-session` instead of cold-spawning, making session creation
/// nearly instant.  The warm server uses session name `__warm__`.
fn spawn_warm_server(app: &AppState) {
    // destroy-unattached means the user expects the session to be torn down
    // when the last client leaves; keeping a hidden warm server alive breaks
    // that expectation and makes exit-empty appear ineffective.
    if !should_spawn_warm_server(app) {
        return;
    }
    // Skip if a warm server already exists
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let warm_base = if let Some(ref sn) = app.socket_name {
        format!("{}____warm__", sn)
    } else {
        "__warm__".to_string()
    };
    let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
    if std::path::Path::new(&warm_port_path).exists() {
        // Check if it's actually alive
        if let Ok(port_str) = std::fs::read_to_string(&warm_port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                let addr = format!("127.0.0.1:{}", port);
                if std::net::TcpStream::connect_timeout(
                    &addr.parse().unwrap(),
                    Duration::from_millis(100),
                ).is_ok() {
                    return; // warm server already running
                }
            }
        }
        // Stale port file — remove it
        let _ = std::fs::remove_file(&warm_port_path);
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut args: Vec<String> = vec!["server".into(), "-s".into(), "__warm__".into()];
    if let Some(ref sn) = app.socket_name {
        args.push("-L".into());
        args.push(sn.clone());
    }
    // Pass current terminal dimensions so the warm server's first window
    // and warm pane are spawned at the right size.
    let area = app.last_window_area;
    if area.width > 1 && area.height > 1 {
        args.push("-x".into());
        args.push(area.width.to_string());
        args.push("-y".into());
        args.push(area.height.to_string());
    }
    #[cfg(windows)]
    { let _ = crate::platform::spawn_server_hidden(&exe, &args); }
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new(&exe);
        for a in &args { cmd.arg(a); }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let _ = cmd.spawn();
    }
}

/// Compute the effective display size from all connected clients' terminal sizes.
/// Returns None if no clients have reported sizes.
fn compute_effective_client_size(app: &AppState) -> Option<(u16, u16)> {
    if app.client_sizes.is_empty() { return None; }
    match app.window_size.as_str() {
        "smallest" => Some((
            app.client_sizes.values().map(|s| s.0).min().unwrap(),
            app.client_sizes.values().map(|s| s.1).min().unwrap(),
        )),
        "largest" => Some((
            app.client_sizes.values().map(|s| s.0).max().unwrap(),
            app.client_sizes.values().map(|s| s.1).max().unwrap(),
        )),
        _ => {
            // "latest" — use latest client's size, fall back to smallest
            if let Some(cid) = app.latest_client_id {
                if let Some(&size) = app.client_sizes.get(&cid) {
                    return Some(size);
                }
            }
            Some((
                app.client_sizes.values().map(|s| s.0).min().unwrap(),
                app.client_sizes.values().map(|s| s.1).min().unwrap(),
            ))
        }
    }
}

pub fn run_server(session_name: String, socket_name: Option<String>, initial_command: Option<String>, raw_command: Option<Vec<String>>, start_dir: Option<String>, window_name: Option<String>, init_size: Option<(u16, u16)>) -> io::Result<()> {
    // DEBUG: absolute first thing in run_server
    let home_dbg = std::env::var("USERPROFILE").unwrap_or_default();
    let _ = std::fs::write(format!("{}\\.psmux\\debug_server.log", home_dbg), format!("run_server called for '{}'\n", session_name));
    
    // Write crash info to a log file when stderr is unavailable (detached server)
    std::panic::set_hook(Box::new(|info| {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let path = format!("{}\\.psmux\\crash.log", home);
        let bt = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::write(&path, format!("{info}\n\nBacktrace:\n{bt}"));
    }));
    // Install console control handler to prevent termination on client detach
    install_console_ctrl_handler();

    let pty_system = native_pty_system();

    let mut app = AppState::new(session_name);
    app.socket_name = socket_name;
    // Server starts detached with a reasonable default window size
    app.attached_clients = 0;

    // Bind the control listener BEFORE loading config so that run-shell
    // commands spawned by load_config can connect back to the server.
    let (tx, rx) = mpsc::channel::<CtrlReq>();
    app.control_rx = Some(rx);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    app.control_port = Some(port);

    // Write port and key files IMMEDIATELY after binding, BEFORE loading
    // config or creating windows.  run-shell scripts (e.g. PPM) need the
    // port file to discover the server, and the client polls for it to know
    // the server is ready.
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let dir = format!("{}\\.psmux", home);
    let _ = std::fs::create_dir_all(&dir);

    // Generate a random session key for security
    let session_key: String = {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        let s = RandomState::new();
        let mut h = s.build_hasher();
        h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as u64);
        h.write_u64(std::process::id() as u64);
        format!("{:016x}", h.finish())
    };

    let regpath = format!("{}\\{}.port", dir, app.port_file_base());
    let _ = std::fs::write(&regpath, port.to_string());
    let keypath = format!("{}\\{}.key", dir, app.port_file_base());
    let _ = std::fs::write(&keypath, &session_key);

    // Expose the server identity via env var so that child processes spawned
    // by run-shell (from hooks, keybindings, etc.) can find this server when
    // they call `psmux set -g ...` or other CLI commands.
    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());

    // Try to set file permissions to user-only (Windows)
    #[cfg(windows)]
    {
        // Recreate key file with restricted permissions
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&keypath)
            .map(|mut f| std::io::Write::write_all(&mut f, session_key.as_bytes()));
    }

    // Start accept thread BEFORE load_config so that run-shell commands
    // (e.g. PPM plugin manager) spawned during config parsing can connect
    // to the server.  Without this, run-shell scripts fail silently because
    // there is no TCP listener accepting connections yet.
    // Initialize shared aliases empty — will be populated after load_config.
    let shared_aliases: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>> =
        std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
    let shared_aliases_main = shared_aliases.clone();

    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(stream) = conn {
                let tx = tx.clone();
                let session_key_clone = session_key.clone();
                let aliases = shared_aliases.clone();
                thread::spawn(move || {
                    connection::handle_connection(stream, tx, &session_key_clone, aliases);
                }); // end per-connection thread
            }
        }
    });

    // Load config AFTER the TCP listener is bound, port/key files are written,
    // and the accept thread is running.  This ensures that run-shell commands
    // in the config (e.g. `run '~/.psmux/plugins/ppm/ppm.ps1'`) can connect
    // back to the server to apply settings.

    // Apply initial dimensions BEFORE warm pane spawn so spawn_warm_pane()
    // uses the correct terminal size.
    if let Some((w, h)) = init_size {
        app.last_window_area = ratatui::layout::Rect { x: 0, y: 0, width: w, height: h };
    }

    // Pre-spawn a warm pane BEFORE loading config: the shell (pwsh) starts
    // loading immediately and runs in parallel with config parsing / plugin
    // initialization.  By the time create_window() consumes it, the shell
    // has had the full config-load duration (~100-500ms) as a head start.
    // Only when we have real dimensions and default shell (no custom command).
    let early_warm = if init_size.is_some() && initial_command.is_none() && raw_command.is_none() && start_dir.is_none() {
        match spawn_warm_pane(&*pty_system, &mut app) {
            Ok(wp) => Some(wp),
            Err(_) => None,
        }
    } else { None };

    load_config(&mut app);

    // If the user configured a custom default-shell in their config, the
    // early warm pane has the wrong shell — kill it so create_window falls
    // through to a cold spawn with the correct shell.
    if let Some(wp) = early_warm {
        if app.default_shell.is_empty() {
            // No custom shell — the pre-spawned default (pwsh) is correct
            app.warm_pane = Some(wp);
        } else {
            // Custom shell set by config — wrong warm pane, kill it
            let mut wp = wp;
            wp.child.kill().ok();
        }
    }

    // Update shared aliases now that config has been loaded
    if let Ok(mut w) = shared_aliases_main.write() {
        *w = app.command_aliases.clone();
    }

    // Create initial window — if a warm pane was pre-spawned above,
    // create_window's fast path transplants it instantly.
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(ref dir) = start_dir { env::set_current_dir(dir).ok(); }
    if let Some(ref raw_args) = raw_command {
        create_window_raw(&*pty_system, &mut app, raw_args)?;
    } else {
        create_window(&*pty_system, &mut app, initial_command.as_deref())?;
    }
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    // Apply window name if specified via -n
    if let Some(n) = window_name { app.windows.last_mut().map(|w| w.name = n); }
    // Replenish: spawn a warm pane for the NEXT new-window / split.
    // For detached sessions without init_size, defer until first ClientSize.
    if init_size.is_some() && app.warm_pane.is_none() {
        match spawn_warm_pane(&*pty_system, &mut app) {
            Ok(wp) => { app.warm_pane = Some(wp); }
            Err(e) => { eprintln!("psmux: warm pane pre-spawn failed: {e}"); }
        }
    }
    // Fire client-attached hooks once at startup so plugins populate initial
    // data (e.g. CPU/battery) even for detached sessions (tppanel previews).
    {
        let cmds: Vec<String> = app.hooks.get("client-attached").cloned().unwrap_or_default();
        for cmd in cmds {
            parse_config_line(&mut app, &cmd);
        }
    }
    // Spawn a warm server for the NEXT new-session when the current session
    // is allowed to keep background state alive.
    if should_spawn_warm_server(&app) {
        spawn_warm_server(&app);
    }
    let mut state_dirty = true;
    let mut cached_dump_state = String::new();
    let mut cached_data_version: u64 = 0;
    // Cached metadata JSON — windows/tree/prefix change only on structural
    // mutations, so we rebuild them lazily via `meta_dirty`.
    let mut meta_dirty = true;
    let mut cached_windows_json = String::new();
    let mut cached_tree_json = String::new();
    let mut cached_prefix_str = String::new();
    let mut cached_prefix2_str = String::new();
    let mut cached_base_index: usize = 0;
    let mut cached_pred_dim: bool = false;
    let mut cached_status_style = String::new();
    let mut cached_bindings_json = String::from("[]");
    // Reusable buffer for building the combined JSON envelope.
    let mut combined_buf = String::with_capacity(32768);


    // Track when we recently sent keystrokes to the PTY.  While waiting
    // for the echo to appear we use a much shorter recv_timeout (1ms vs 5ms)
    // so that dump-state requests are served with minimal delay.  This is
    // critical for nested-shell latency (e.g. WSL inside pwsh) where the
    // echo path goes through ConPTY → pwsh → WSL → echo → ConPTY and can
    // take 10-30ms.  Without this, each "no-change" polling cycle costs up
    // to 5ms, adding cumulative latency visible as heavy input lag.
    let mut echo_pending_until: Option<Instant> = None;

    // Track when any client last requested a dump or sent input.
    // Used to ramp down the server loop frequency when truly idle.
    let mut last_client_activity = Instant::now();

    // Throttle reap_children: only check for exited processes every 250ms.
    // With hundreds of windows, calling try_wait() on every process each
    // loop iteration wastes CPU.  Exited processes are still reaped promptly
    // (250ms is imperceptible to users).
    let mut last_reap = Instant::now();

    loop {
        // Adaptive timeout: ramps from 1ms (active typing/echo) through
        // 5ms (client recently active) up to 50ms (fully idle).  This
        // dramatically reduces CPU usage when the session is idle while
        // keeping responsiveness high during interaction.
        let data_ready = crate::types::PTY_DATA_READY.swap(false, std::sync::atomic::Ordering::AcqRel);
        if data_ready {
            state_dirty = true;
        }
        let echo_active = echo_pending_until.map_or(false, |t| t.elapsed().as_millis() < 50);
        let idle_secs = last_client_activity.elapsed().as_secs();
        let timeout_ms: u64 = if echo_active || data_ready {
            1      // Active echo/data: 1ms for maximum responsiveness
        } else if idle_secs < 2 {
            5      // Recently active: 5ms (200 Hz)
        } else if crate::types::has_frame_receivers() {
            16     // Push clients attached: 16ms (~60 Hz) so PTY data
                   // is detected and pushed within one vsync period.
        } else {
            50     // No clients: 50ms (20 Hz) — saves CPU
        };
        if let Some(rx) = app.control_rx.as_ref() {
            if let Ok(req) = rx.recv_timeout(Duration::from_millis(timeout_ms)) {
                last_client_activity = Instant::now();
                let mut pending = vec![req];
                // Drain any additional queued messages without blocking
                while let Ok(r) = rx.try_recv() {
                    pending.push(r);
                }
                // Also check if fresh PTY output arrived while we were
                // waiting – mark state dirty so DumpState produces a full
                // frame instead of "NC".
                if crate::types::PTY_DATA_READY.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    state_dirty = true;
                }
                // Process key/command inputs BEFORE dump-state requests.
                // This ensures ConPTY receives keystrokes before we serialize
                // the screen, reducing stale-frame responses.
                pending.sort_by_key(|r| match r {
                    CtrlReq::DumpState(..) => 1,
                    CtrlReq::DumpLayout(_) => 1,
                    _ => 0,
                });
                // Track temporary -t focus: save (active_idx, active_path) when
                // FocusWindowTemp/FocusPaneTemp is seen, restore after next
                // non-temp command so the user's view doesn't jump.
                let mut temp_focus_restore: Option<(usize, Vec<usize>)> = None;
                for req in pending {
                    let mutates_state = !matches!(&req,
                        CtrlReq::DumpState(..)
                        | CtrlReq::SendText(_)
                        | CtrlReq::SendKey(_)
                        | CtrlReq::SendPaste(_)
                    );
                    let is_temp_focus = matches!(&req,
                        CtrlReq::FocusWindowTemp(_) | CtrlReq::FocusPaneTemp(_) | CtrlReq::FocusPaneByIndexTemp(_));
                    let mut hook_event: Option<&str> = None;
                    // Track active_idx changes for debugging window-switch issues
                    let _prev_active_idx = app.active_idx;
                    let _req_tag: &str = match &req {
                        CtrlReq::NextWindow => "NextWindow",
                        CtrlReq::PrevWindow => "PrevWindow",
                        CtrlReq::SelectWindow(_) => "SelectWindow",
                        CtrlReq::FocusWindow(_) => "FocusWindow",
                        CtrlReq::FocusWindowTemp(_) => "FocusWindowTemp",
                        CtrlReq::FocusWindowCmd(_) => "FocusWindowCmd",
                        CtrlReq::LastWindow => "LastWindow",
                        CtrlReq::MouseDown(..) => "MouseDown",
                        CtrlReq::MouseDownRight(..) => "MouseDownRight",
                        CtrlReq::MouseDownMiddle(..) => "MouseDownMiddle",
                        CtrlReq::FocusPane(_) => "FocusPane",
                        CtrlReq::FocusPaneTemp(_) => "FocusPaneTemp",
                        CtrlReq::NewWindow(..) => "NewWindow",
                        CtrlReq::KillWindow => "KillWindow",
                        CtrlReq::KillPane => "KillPane",
                        CtrlReq::BreakPane => "BreakPane",
                        CtrlReq::JoinPane(_) => "JoinPane",
                        CtrlReq::MoveWindow(..) => "MoveWindow",
                        CtrlReq::SwapWindow(_) => "SwapWindow",
                        _ => "",
                    };
                    match req {
                CtrlReq::NewWindow(cmd, name, detached, start_dir) => {
                    let prev_idx = app.active_idx;
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    // Hide the warm pane when an explicit start dir is requested
                    // so create_window spawns a fresh shell in the correct CWD.
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = create_window(&*pty_system, &mut app, cmd.as_deref()) {
                        eprintln!("psmux: new-window error: {e}");
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
                    if let Some(n) = name { app.windows.last_mut().map(|w| w.name = n); }
                    if detached { app.active_idx = prev_idx; }
                    // Replenish warm pane pool for next new-window
                    if app.warm_pane.is_none() {
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(wp) => { app.warm_pane = Some(wp); }
                            Err(_) => {}
                        }
                    }
                    resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-new-window");
                }
                CtrlReq::NewWindowPrint(cmd, name, detached, start_dir, format_str, resp) => {
                    let prev_idx = app.active_idx;
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = create_window(&*pty_system, &mut app, cmd.as_deref()) {
                        eprintln!("psmux: new-window error: {e}");
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
                    if let Some(n) = name { app.windows.last_mut().map(|w| w.name = n); }
                    // Use full format engine for -P output (tmux compatible)
                    let new_win_idx = app.windows.len() - 1;
                    let fmt = format_str.as_deref().unwrap_or("#{session_name}:#{window_index}");
                    let pane_info = crate::format::expand_format_for_window(fmt, &app, new_win_idx);
                    if detached { app.active_idx = prev_idx; }
                    let _ = resp.send(pane_info);
                    // Replenish warm pane pool for next new-window
                    if app.warm_pane.is_none() {
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(wp) => { app.warm_pane = Some(wp); }
                            Err(_) => {}
                        }
                    }
                    resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-new-window");
                }
                CtrlReq::SplitWindow(k, cmd, detached, start_dir, size_pct, resp) => {
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    let prev_path = app.windows[app.active_idx].active_path.clone();
                    // Hide warm pane when explicit start_dir is given (wrong CWD)
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = split_active_with_command(&mut app, k, cmd.as_deref(), Some(&*pty_system)) {
                        let _ = resp.send(format!("psmux: split-window: {e}"));
                    } else {
                        let _ = resp.send(String::new());
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    // Apply size if specified (as percentage)
                    if let Some(_pct) = size_pct {
                        // Size will be applied by resize_all_panes using the layout ratios
                    }
                    if detached {
                        // Revert focus to the previously active pane.
                        // After split, prev_path now points to a Split node;
                        // the original pane is child [0] of that Split.
                        let mut revert_path = prev_path;
                        revert_path.push(0);
                        app.windows[app.active_idx].active_path = revert_path;
                    }
                    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
                    // Replenish warm pane for the next new-window/split
                    if app.warm_pane.is_none() {
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(wp) => { app.warm_pane = Some(wp); }
                            Err(_) => {}
                        }
                    }
                    resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-split-window");
                }
                CtrlReq::SplitWindowPrint(k, cmd, detached, start_dir, size_pct, format_str, resp) => {
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    let prev_path = app.windows[app.active_idx].active_path.clone();
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = split_active_with_command(&mut app, k, cmd.as_deref(), Some(&*pty_system)) {
                        eprintln!("psmux: split-window error: {e}");
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    if let Some(_pct) = size_pct { }
                    // Use full format engine for -P output (tmux compatible)
                    let fmt = format_str.as_deref().unwrap_or("#{session_name}:#{window_index}.#{pane_index}");
                    let pane_info = crate::format::expand_format_for_window(fmt, &app, app.active_idx);
                    if detached {
                        let mut revert_path = prev_path;
                        revert_path.push(0);
                        app.windows[app.active_idx].active_path = revert_path;
                    }
                    let _ = resp.send(pane_info);
                    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
                    // Replenish warm pane
                    if app.warm_pane.is_none() {
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(wp) => { app.warm_pane = Some(wp); }
                            Err(_) => {}
                        }
                    }
                    resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-split-window");
                }
                CtrlReq::KillPane => { let _ = kill_active_pane(&mut app); resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-kill-pane"); }
                CtrlReq::CapturePane(resp) => {
                    if let Some(text) = capture_active_pane_text(&mut app)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneStyled(resp, s, e) => {
                    if let Some(text) = capture_active_pane_styled(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneRange(resp, s, e) => {
                    if let Some(text) = capture_active_pane_range(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::FocusWindow(wid) => {
                    // wid is a display index (same as tmux window number), convert to internal array index
                    if wid >= app.window_base_index {
                        let internal_idx = wid - app.window_base_index;
                        if internal_idx < app.windows.len() && internal_idx != app.active_idx {
                            switch_with_copy_save(&mut app, |app| {
                                app.last_window_idx = app.active_idx;
                                app.active_idx = internal_idx;
                            });
                            // Lazily resize panes in the newly-focused window
                            resize_all_panes(&mut app);
                        }
                    }
                    meta_dirty = true;
                }
                CtrlReq::FocusPane(pid) => {
                    let old_path = app.windows[app.active_idx].active_path.clone();
                    switch_with_copy_save(&mut app, |app| { focus_pane_by_id(app, pid); });
                    if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(&mut app); }
                    meta_dirty = true;
                }
                CtrlReq::FocusPaneByIndex(idx) => {
                    let old_path = app.windows[app.active_idx].active_path.clone();
                    switch_with_copy_save(&mut app, |app| { focus_pane_by_index(app, idx); });
                    if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(&mut app); }
                    meta_dirty = true;
                }
                // ── Temporary focus variants for -t targeting ────────────
                // These switch active_idx/active_path so the NEXT command
                // in the batch operates on the correct window/pane.
                // After the entire pending batch is processed, we restore
                // the original focus (see temp_focus_restore below).
                CtrlReq::FocusWindowTemp(wid) => {
                    if temp_focus_restore.is_none() {
                        temp_focus_restore = Some((app.active_idx, app.windows[app.active_idx].active_path.clone()));
                    }
                    if wid >= app.window_base_index {
                        let internal_idx = wid - app.window_base_index;
                        if internal_idx < app.windows.len() {
                            app.active_idx = internal_idx;
                        }
                    }
                }
                CtrlReq::FocusPaneTemp(pid) => {
                    if temp_focus_restore.is_none() {
                        temp_focus_restore = Some((app.active_idx, app.windows[app.active_idx].active_path.clone()));
                    }
                    focus_pane_by_id(&mut app, pid);
                }
                CtrlReq::FocusPaneByIndexTemp(idx) => {
                    if temp_focus_restore.is_none() {
                        temp_focus_restore = Some((app.active_idx, app.windows[app.active_idx].active_path.clone()));
                    }
                    focus_pane_by_index(&mut app, idx);
                }
                CtrlReq::SessionInfo(resp) => {
                    let attached = if app.attached_clients > 0 { " (attached)" } else { "" };
                    let windows = app.windows.len();
                    let created = app.created_at.format("%a %b %e %H:%M:%S %Y");
                    let line = format!("{}: {} windows (created {}){}\n", app.session_name, windows, created, attached);
                    let _ = resp.send(line);
                }
                CtrlReq::ClientAttach(cid) => { app.attached_clients = app.attached_clients.saturating_add(1); app.latest_client_id = Some(cid); hook_event = Some("client-attached"); }
                CtrlReq::ClientDetach(cid) => {
                    app.attached_clients = app.attached_clients.saturating_sub(1);
                    app.client_sizes.remove(&cid);
                    if app.latest_client_id == Some(cid) {
                        app.latest_client_id = None;
                    }
                    // Recompute effective size from remaining clients
                    if let Some((w, h)) = compute_effective_client_size(&app) {
                        app.last_window_area = Rect { x: 0, y: 0, width: w, height: h };
                        resize_all_panes(&mut app);
                    }
                    hook_event = Some("client-detached");
                    if app.attached_clients == 0 && app.destroy_unattached {
                        for win in app.windows.iter_mut() {
                            kill_all_children(&mut win.root);
                        }
                        if let Some(mut wp) = app.warm_pane.take() {
                            wp.child.kill().ok();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                        let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                        let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                        let _ = std::fs::remove_file(&regpath);
                        let _ = std::fs::remove_file(&keypath);
                        crate::types::shutdown_persistent_streams();
                        std::process::exit(0);
                    }
                }
                CtrlReq::DumpLayout(resp) => {
                    let json = dump_layout_json(&mut app)?;
                    let _ = resp.send(json);
                }
                CtrlReq::DumpState(resp, allow_nc) => {
                    // ── Automatic rename: resolve foreground process ──
                    {
                        let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
                        if app.automatic_rename && !in_copy {
                            for win in app.windows.iter_mut() {
                                if win.manual_rename { continue; }
                                if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                                    if p.dead { continue; }
                                    if p.last_title_check.elapsed().as_millis() < 1000 { continue; }
                                    p.last_title_check = std::time::Instant::now();
                                    if p.child_pid.is_none() {
                                        p.child_pid = crate::platform::mouse_inject::get_child_pid(&*p.child);
                                    }
                                    let new_name = if let Some(pid) = p.child_pid {
                                        crate::platform::process_info::get_foreground_process_name(pid)
                                            .unwrap_or_else(|| "shell".into())
                                    } else if !p.title.is_empty() {
                                        p.title.clone()
                                    } else {
                                        continue;
                                    };
                                    if !new_name.is_empty() && win.name != new_name {
                                        win.name = new_name;
                                        meta_dirty = true;
                                        state_dirty = true;
                                    }
                                }
                            }
                        }
                    }
                    // Fast-path: nothing changed at all → 2-byte "NC" marker
                    // instead of cloning 50-100KB of JSON.
                    // Only allowed for persistent connections that already have
                    // the previous frame; one-shot connections always need full state.
                    // Skip the fast-path while any pane in the active window still
                    // has a default placeholder title — we need layout serialisation
                    // to keep running infer_title_from_prompt until a real title is
                    // resolved.
                    let has_placeholder_title = app.windows.get(app.active_idx)
                        .and_then(|w| crate::tree::active_pane(&w.root, &w.active_path))
                        .map_or(false, |p| p.title.starts_with("pane %"));
                    if allow_nc
                        && !state_dirty
                        && !has_placeholder_title
                        && !cached_dump_state.is_empty()
                        && cached_data_version == combined_data_version(&app)
                    {
                        let _ = resp.send("NC".to_string());
                        continue;
                    }
                    // Rebuild metadata cache if structural changes happened.
                    if meta_dirty {
                        cached_windows_json = list_windows_json_with_tabs(&app)?;
                        cached_tree_json = list_tree_json(&app)?;
                        cached_prefix_str = format_key_binding(&app.prefix_key);
                        cached_prefix2_str = app.prefix2_key.as_ref().map(|k| format_key_binding(k)).unwrap_or_default();
                        cached_base_index = app.window_base_index;
                        cached_pred_dim = app.prediction_dimming;
                        cached_status_style = app.status_style.clone();
                        cached_bindings_json = serialize_bindings_json(&app);
                        meta_dirty = false;
                    }
                    let _t_layout = std::time::Instant::now();
                    let layout_json = dump_layout_json_fast(&mut app)?;
                    let _layout_ms = _t_layout.elapsed().as_micros();
                    combined_buf.clear();
                    let ss_escaped = json_escape_string(&cached_status_style);
                    let sl_expanded = json_escape_string(&expand_format(&app.status_left, &app));
                    let sr_expanded = json_escape_string(&expand_format(&app.status_right, &app));
                    let pbs_escaped = json_escape_string(&app.pane_border_style);
                    let pabs_escaped = json_escape_string(&app.pane_active_border_style);
                    let wsf_escaped = json_escape_string(&app.window_status_format);
                    let wscf_escaped = json_escape_string(&app.window_status_current_format);
                    let wss_escaped = json_escape_string(&app.window_status_separator);
                    let ws_style_escaped = json_escape_string(&app.window_status_style);
                    let wsc_style_escaped = json_escape_string(&app.window_status_current_style);
                    let mode_style_escaped = json_escape_string(&app.mode_style);
                    let status_position_escaped = json_escape_string(&app.status_position);
                    let status_justify_escaped = json_escape_string(&app.status_justify);
                    // Build status_format JSON array for multi-line status bar
                    let status_format_json = {
                        let mut sf = String::from("[");
                        for (i, fmt_str) in app.status_format.iter().enumerate() {
                            if i > 0 { sf.push(','); }
                            sf.push('"');
                            sf.push_str(&json_escape_string(&expand_format(fmt_str, &app)));
                            sf.push('"');
                        }
                        sf.push(']');
                        sf
                    };
                    let cursor_style_code = crate::rendering::configured_cursor_code();
                    let _ = std::fmt::Write::write_fmt(&mut combined_buf, format_args!(
                        "{{\"layout\":{},\"windows\":{},\"prefix\":\"{}\",\"prefix2\":\"{}\",\"tree\":{},\"base_index\":{},\"prediction_dimming\":{},\"status_style\":\"{}\",\"status_left\":\"{}\",\"status_right\":\"{}\",\"pane_border_style\":\"{}\",\"pane_active_border_style\":\"{}\",\"wsf\":\"{}\",\"wscf\":\"{}\",\"wss\":\"{}\",\"ws_style\":\"{}\",\"wsc_style\":\"{}\",\"clock_mode\":{},\"bindings\":{},\"status_left_length\":{},\"status_right_length\":{},\"status_lines\":{},\"status_format\":{},\"mode_style\":\"{}\",\"status_position\":\"{}\",\"status_justify\":\"{}\",\"cursor_style_code\":{},\"status_visible\":{},\"repeat_time\":{}}}",
                        layout_json, cached_windows_json, cached_prefix_str, cached_prefix2_str, cached_tree_json, cached_base_index, cached_pred_dim, ss_escaped, sl_expanded, sr_expanded, pbs_escaped, pabs_escaped, wsf_escaped, wscf_escaped, wss_escaped, ws_style_escaped, wsc_style_escaped,
                        matches!(app.mode, Mode::ClockMode), cached_bindings_json,
                        app.status_left_length, app.status_right_length, app.status_lines, status_format_json,
                        mode_style_escaped, status_position_escaped, status_justify_escaped,
                        cursor_style_code, app.status_visible, app.repeat_time_ms,
                    ));
                    cached_dump_state.clear();
                    cached_dump_state.push_str(&combined_buf);
                    // Inject one-shot clipboard data for OSC 52 delivery to
                    // the client.  Only the *response* includes this field;
                    // the cached copy does not, so subsequent NC frames won't
                    // re-trigger clipboard emission on the client.
                    if let Some(clip_text) = app.clipboard_osc52.take() {
                        let clip_b64 = base64_encode(&clip_text);
                        // Replace trailing '}' with the extra field
                        if combined_buf.ends_with('}') {
                            combined_buf.pop();
                            combined_buf.push_str(",\"clipboard_osc52\":\"");
                            combined_buf.push_str(&clip_b64);
                            combined_buf.push_str("\"}");
                        }
                    }
                    cached_data_version = combined_data_version(&app);
                    state_dirty = false;
                    // Timing log: dump-state build time
                    if std::env::var("PSMUX_LATENCY_LOG").unwrap_or_default() == "1" {
                        let total_us = _t_layout.elapsed().as_micros();
                        use std::io::Write as _;
                        static SRV_LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();
                        let log = SRV_LOG.get_or_init(|| {
                            let p = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\gj".into())).join("psmux_server_latency.log");
                            std::sync::Mutex::new(std::fs::File::create(p).expect("create latency log"))
                        });
                        if let Ok(mut f) = log.lock() {
                            let _ = writeln!(f, "[SRV] dump: layout={}us total={}us json_len={}", _layout_ms, total_us, combined_buf.len());
                        }
                    }
                    // Push the newly-built frame to ALL persistent clients so
                    // that other attached sessions see the update immediately,
                    // even if they are idle and not polling dump-state.
                    // Without this, the DumpState handler clears state_dirty,
                    // and the bottom-of-loop push section never fires for frames
                    // already served to the requesting client.
                    crate::types::push_frame(&cached_dump_state);
                    let _ = resp.send(combined_buf.clone());
                }
                CtrlReq::SendText(s) => { send_text_to_active(&mut app, &s)?; echo_pending_until = Some(Instant::now()); }
                CtrlReq::SendKey(k) => { send_key_to_active(&mut app, &k)?; echo_pending_until = Some(Instant::now()); }
                CtrlReq::SendPaste(s) => { send_paste_to_active(&mut app, &s)?; echo_pending_until = Some(Instant::now()); }
                CtrlReq::ZoomPane => { toggle_zoom(&mut app); hook_event = Some("after-resize-pane"); }
                CtrlReq::CopyEnter => { enter_copy_mode(&mut app); }
                CtrlReq::CopyEnterPageUp => {
                    enter_copy_mode(&mut app);
                    let half = app.windows.get(app.active_idx)
                        .and_then(|w| active_pane(&w.root, &w.active_path))
                        .map(|p| p.last_rows as usize).unwrap_or(20);
                    scroll_copy_up(&mut app, half);
                }
                CtrlReq::ClockMode => { app.mode = Mode::ClockMode; }
                CtrlReq::CopyMove(dx, dy) => { move_copy_cursor(&mut app, dx, dy); }
                CtrlReq::CopyAnchor => { if let Some((r,c)) = current_prompt_pos(&mut app) { app.copy_anchor = Some((r,c)); app.copy_anchor_scroll_offset = app.copy_scroll_offset; app.copy_pos = Some((r,c)); } }
                CtrlReq::CopyYank => { let _ = yank_selection(&mut app); exit_copy_mode(&mut app); }
                CtrlReq::CopyRectToggle => {
                    app.copy_selection_mode = match app.copy_selection_mode {
                        crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                        _ => crate::types::SelectionMode::Rect,
                    };
                }
                CtrlReq::ClientSize(cid, w, h) => { 
                    app.client_sizes.insert(cid, (w, h));
                    app.latest_client_id = Some(cid);
                    let (ew, eh) = compute_effective_client_size(&app).unwrap_or((w, h));
                    app.last_window_area = Rect { x: 0, y: 0, width: ew, height: eh }; 
                    resize_all_panes(&mut app);
                    // Respawn warm pane at the new terminal dimensions so
                    // the next new-window gets a pane whose parser grid
                    // already matches the display — no resize reflow needed,
                    // prompt appears pixel-perfect on the first frame.
                    let need_respawn = app.warm_pane.as_ref().map_or(true, |wp| wp.rows != eh || wp.cols != ew);
                    if need_respawn {
                        if let Some(mut old) = app.warm_pane.take() { old.child.kill().ok(); }
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(wp) => { app.warm_pane = Some(wp); }
                            Err(_) => {}
                        }
                    }
                }
                CtrlReq::FocusPaneCmd(pid) => {
                    let old_path = app.windows[app.active_idx].active_path.clone();
                    switch_with_copy_save(&mut app, |app| { focus_pane_by_id(app, pid); });
                    if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(&mut app); }
                    meta_dirty = true;
                }
                CtrlReq::FocusWindowCmd(wid) => { switch_with_copy_save(&mut app, |app| { if let Some(idx) = find_window_index_by_id(app, wid) { app.active_idx = idx; } }); resize_all_panes(&mut app); meta_dirty = true; }
                CtrlReq::MouseDown(x,y) => { if app.mouse_enabled { remote_mouse_down(&mut app, x, y); state_dirty = true; meta_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseDownRight(x,y) => { if app.mouse_enabled { remote_mouse_button(&mut app, x, y, 2, true); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseDownMiddle(x,y) => { if app.mouse_enabled { remote_mouse_button(&mut app, x, y, 1, true); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseDrag(x,y) => { if app.mouse_enabled { remote_mouse_drag(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseUp(x,y) => { if app.mouse_enabled { remote_mouse_up(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseUpRight(x,y) => { if app.mouse_enabled { remote_mouse_button(&mut app, x, y, 2, false); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseUpMiddle(x,y) => { if app.mouse_enabled { remote_mouse_button(&mut app, x, y, 1, false); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseMove(x,y) => { if app.mouse_enabled { remote_mouse_motion(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::ScrollUp(x, y) => { if app.mouse_enabled { remote_scroll_up(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::ScrollDown(x, y) => { if app.mouse_enabled { remote_scroll_down(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::NextWindow => { if !app.windows.is_empty() { switch_with_copy_save(&mut app, |app| { app.last_window_idx = app.active_idx; app.active_idx = (app.active_idx + 1) % app.windows.len(); }); resize_all_panes(&mut app); } meta_dirty = true; hook_event = Some("after-select-window"); }
                CtrlReq::PrevWindow => { if !app.windows.is_empty() { switch_with_copy_save(&mut app, |app| { app.last_window_idx = app.active_idx; app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); }); resize_all_panes(&mut app); } meta_dirty = true; hook_event = Some("after-select-window"); }
                CtrlReq::RenameWindow(name) => { let win = &mut app.windows[app.active_idx]; win.name = name; win.manual_rename = true; meta_dirty = true; hook_event = Some("after-rename-window"); }
                CtrlReq::ListWindows(resp) => { let json = list_windows_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ListWindowsTmux(resp) => { let text = list_windows_tmux(&app); let _ = resp.send(text); }
                CtrlReq::ListWindowsFormat(resp, fmt) => { let text = format_list_windows(&app, &fmt); let _ = resp.send(text); }
                CtrlReq::ListTree(resp) => { let json = list_tree_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ToggleSync => { app.sync_input = !app.sync_input; }
                CtrlReq::SetPaneTitle(title) => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { p.title = title; }
                }
                CtrlReq::SetPaneStyle(style) => {
                    // Per-pane styling (e.g. "bg=default,fg=blue") matching
                    // tmux's `-P` flag which sets window-style + window-active-style.
                    // Store on the pane for API compatibility; ConPTY rendering
                    // doesn't support per-pane fg/bg tinting yet.
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                        p.pane_style = Some(style);
                    }
                }
                CtrlReq::SendKeys(keys, literal) => {
                    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
                    if in_copy {
                        // In copy/search mode — route through mode-aware handlers
                        if literal {
                            send_text_to_active(&mut app, &keys)?;
                        } else {
                            let parts: Vec<&str> = keys.split_whitespace().collect();
                            for key in parts.iter() {
                                let key_upper = key.to_uppercase();
                                let normalized = match key_upper.as_str() {
                                    "ENTER" => "enter",
                                    "TAB" => "tab",
                                    "BTAB" | "BACKTAB" => "btab",
                                    "ESCAPE" | "ESC" => "esc",
                                    "SPACE" => "space",
                                    "BSPACE" | "BACKSPACE" => "backspace",
                                    "UP" => "up",
                                    "DOWN" => "down",
                                    "RIGHT" => "right",
                                    "LEFT" => "left",
                                    "HOME" => "home",
                                    "END" => "end",
                                    "PAGEUP" | "PPAGE" => "pageup",
                                    "PAGEDOWN" | "NPAGE" => "pagedown",
                                    "DELETE" | "DC" => "delete",
                                    "INSERT" | "IC" => "insert",
                                    _ => "",
                                };
                                if !normalized.is_empty() {
                                    send_key_to_active(&mut app, normalized)?;
                                } else if key_upper.starts_with("C-") || key_upper.starts_with("M-") || (key_upper.starts_with("F") && key_upper.len() >= 2 && key_upper[1..].chars().all(|c| c.is_ascii_digit())) {
                                    send_key_to_active(&mut app, &key.to_lowercase())?;
                                } else {
                                    // Plain text char — route through send_text_to_active (handles copy mode chars)
                                    send_text_to_active(&mut app, key)?;
                                }
                            }
                        }
                    } else if literal {
                        send_text_to_active(&mut app, &keys)?;
                    } else {
                        let parts: Vec<&str> = keys.split_whitespace().collect();
                        for (i, key) in parts.iter().enumerate() {
                            let key_upper = key.to_uppercase();
                            let _is_special = matches!(key_upper.as_str(), 
                                "ENTER" | "TAB" | "BTAB" | "BACKTAB" | "ESCAPE" | "ESC" | "SPACE" | "BSPACE" | "BACKSPACE" |
                                "UP" | "DOWN" | "RIGHT" | "LEFT" | "HOME" | "END" |
                                "PAGEUP" | "PPAGE" | "PAGEDOWN" | "NPAGE" | "DELETE" | "DC" | "INSERT" | "IC" |
                                "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
                            ) || key_upper.starts_with("C-") || key_upper.starts_with("M-") || key_upper.starts_with("S-");
                            
                            match key_upper.as_str() {
                                "ENTER" => send_text_to_active(&mut app, "\r")?,
                                "TAB" => send_text_to_active(&mut app, "\t")?,
                                "BTAB" | "BACKTAB" => send_text_to_active(&mut app, "\x1b[Z")?,
                                "ESCAPE" | "ESC" => send_text_to_active(&mut app, "\x1b")?,
                                "SPACE" => send_text_to_active(&mut app, " ")?,
                                "BSPACE" | "BACKSPACE" => send_text_to_active(&mut app, "\x7f")?,
                                "UP" => send_text_to_active(&mut app, "\x1b[A")?,
                                "DOWN" => send_text_to_active(&mut app, "\x1b[B")?,
                                "RIGHT" => send_text_to_active(&mut app, "\x1b[C")?,
                                "LEFT" => send_text_to_active(&mut app, "\x1b[D")?,
                                "HOME" => send_text_to_active(&mut app, "\x1b[H")?,
                                "END" => send_text_to_active(&mut app, "\x1b[F")?,
                                "PAGEUP" | "PPAGE" => send_text_to_active(&mut app, "\x1b[5~")?,
                                "PAGEDOWN" | "NPAGE" => send_text_to_active(&mut app, "\x1b[6~")?,
                                "DELETE" | "DC" => send_text_to_active(&mut app, "\x1b[3~")?,
                                "INSERT" | "IC" => send_text_to_active(&mut app, "\x1b[2~")?,
                                "F1" => send_text_to_active(&mut app, "\x1bOP")?,
                                "F2" => send_text_to_active(&mut app, "\x1bOQ")?,
                                "F3" => send_text_to_active(&mut app, "\x1bOR")?,
                                "F4" => send_text_to_active(&mut app, "\x1bOS")?,
                                "F5" => send_text_to_active(&mut app, "\x1b[15~")?,
                                "F6" => send_text_to_active(&mut app, "\x1b[17~")?,
                                "F7" => send_text_to_active(&mut app, "\x1b[18~")?,
                                "F8" => send_text_to_active(&mut app, "\x1b[19~")?,
                                "F9" => send_text_to_active(&mut app, "\x1b[20~")?,
                                "F10" => send_text_to_active(&mut app, "\x1b[21~")?,
                                "F11" => send_text_to_active(&mut app, "\x1b[23~")?,
                                "F12" => send_text_to_active(&mut app, "\x1b[24~")?,
                                // Modifier + special key combos (C-Left, S-Right, C-M-Up, etc.)
                                // must be checked BEFORE the generic C-x / M-x single-char handlers.
                                s if crate::input::parse_modified_special_key(s).is_some() => {
                                    let seq = crate::input::parse_modified_special_key(s).unwrap();
                                    send_text_to_active(&mut app, &seq)?;
                                }
                                s if s.starts_with("C-M-") || s.starts_with("C-m-") => {
                                    if let Some(c) = key.chars().nth(4) {
                                        let ctrl = (c.to_ascii_lowercase() as u8) & 0x1F;
                                        send_text_to_active(&mut app, &format!("\x1b{}", ctrl as char))?;
                                    }
                                }
                                s if s.starts_with("C-") => {
                                    if let Some(c) = s.chars().nth(2) {
                                        let ctrl = (c.to_ascii_lowercase() as u8) & 0x1F;
                                        send_text_to_active(&mut app, &String::from(ctrl as char))?;

                                    }
                                }
                                s if s.starts_with("M-") => {
                                    if let Some(c) = key.chars().nth(2) {
                                        send_text_to_active(&mut app, &format!("\x1b{}", c))?;
                                    }
                                }
                                _ => {
                                    send_text_to_active(&mut app, key)?;
                                    if i + 1 < parts.len() {
                                        let next_upper = parts[i + 1].to_uppercase();
                                        let next_is_special = matches!(next_upper.as_str(),
                                            "ENTER" | "TAB" | "BTAB" | "BACKTAB" | "ESCAPE" | "ESC" | "SPACE" | "BSPACE" | "BACKSPACE" |
                                            "UP" | "DOWN" | "RIGHT" | "LEFT" | "HOME" | "END" |
                                            "PAGEUP" | "PPAGE" | "PAGEDOWN" | "NPAGE" | "DELETE" | "DC" | "INSERT" | "IC" |
                                            "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
                                        ) || next_upper.starts_with("C-") || next_upper.starts_with("M-") || next_upper.starts_with("S-");
                                        if !next_is_special {
                                            send_text_to_active(&mut app, " ")?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    echo_pending_until = Some(Instant::now());
                }
                CtrlReq::SendKeysX(cmd) => {
                    // send-keys -X: dispatch copy-mode commands by name
                    // This is the primary mechanism used by tmux-yank and other plugins
                    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
                    if !in_copy {
                        // Auto-enter copy mode for commands that require it
                        enter_copy_mode(&mut app);
                    }
                    match cmd.as_str() {
                        "cancel" => {
                            app.mode = Mode::Passthrough;
                            app.copy_anchor = None;
                            app.copy_pos = None;
                            app.copy_scroll_offset = 0;
                            let win = &mut app.windows[app.active_idx];
                            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                                if let Ok(mut parser) = p.term.lock() {
                                    parser.screen_mut().set_scrollback(0);
                                }
                            }
                        }
                        "begin-selection" => {
                            if let Some((r,c)) = crate::copy_mode::get_copy_pos(&mut app) {
                                app.copy_anchor = Some((r,c));
                                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                                app.copy_pos = Some((r,c));
                                app.copy_selection_mode = crate::types::SelectionMode::Char;
                            }
                        }
                        "select-line" => {
                            if let Some((r,c)) = crate::copy_mode::get_copy_pos(&mut app) {
                                app.copy_anchor = Some((r,c));
                                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                                app.copy_pos = Some((r,c));
                                app.copy_selection_mode = crate::types::SelectionMode::Line;
                            }
                        }
                        "rectangle-toggle" => {
                            app.copy_selection_mode = match app.copy_selection_mode {
                                crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                                _ => crate::types::SelectionMode::Rect,
                            };
                        }
                        "copy-selection" => {
                            let _ = yank_selection(&mut app);
                        }
                        "copy-selection-and-cancel" => {
                            let _ = yank_selection(&mut app);
                            app.mode = Mode::Passthrough;
                            app.copy_scroll_offset = 0;
                            app.copy_pos = None;
                        }
                        "copy-selection-no-clear" => {
                            let _ = yank_selection(&mut app);
                        }
                        s if s.starts_with("copy-pipe-and-cancel") || s.starts_with("copy-pipe") => {
                            // copy-pipe[-and-cancel] [command] — yank + pipe to command
                            let _ = yank_selection(&mut app);
                            // Extract pipe command from argument if present
                            let cancel = s.contains("cancel");
                            let pipe_cmd = cmd.strip_prefix("copy-pipe-and-cancel")
                                .or_else(|| cmd.strip_prefix("copy-pipe"))
                                .unwrap_or("")
                                .trim();
                            if !pipe_cmd.is_empty() {
                                if let Some(text) = app.paste_buffers.first().cloned() {
                                    // Pipe yanked text to the command's stdin
                                    if let Ok(mut child) = std::process::Command::new(if cfg!(windows) { "pwsh" } else { "sh" })
                                        .args(if cfg!(windows) { vec!["-NoProfile", "-Command", pipe_cmd] } else { vec!["-c", pipe_cmd] })
                                        .stdin(std::process::Stdio::piped())
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .spawn() {
                                        if let Some(mut stdin) = child.stdin.take() {
                                            use std::io::Write;
                                            let _ = stdin.write_all(text.as_bytes());
                                        }
                                        let _ = child.wait();
                                    }
                                }
                            }
                            if cancel {
                                app.mode = Mode::Passthrough;
                                app.copy_scroll_offset = 0;
                                app.copy_pos = None;
                            }
                        }
                        "cursor-up" => { move_copy_cursor(&mut app, 0, -1); }
                        "cursor-down" => { move_copy_cursor(&mut app, 0, 1); }
                        "cursor-left" => { move_copy_cursor(&mut app, -1, 0); }
                        "cursor-right" => { move_copy_cursor(&mut app, 1, 0); }
                        "start-of-line" => { crate::copy_mode::move_to_line_start(&mut app); }
                        "end-of-line" => { crate::copy_mode::move_to_line_end(&mut app); }
                        "back-to-indentation" => { crate::copy_mode::move_to_first_nonblank(&mut app); }
                        "next-word" => { crate::copy_mode::move_word_forward(&mut app); }
                        "previous-word" => { crate::copy_mode::move_word_backward(&mut app); }
                        "next-word-end" => { crate::copy_mode::move_word_end(&mut app); }
                        "next-space" => { crate::copy_mode::move_word_forward_big(&mut app); }
                        "previous-space" => { crate::copy_mode::move_word_backward_big(&mut app); }
                        "next-space-end" => { crate::copy_mode::move_word_end_big(&mut app); }
                        "top-line" => { crate::copy_mode::move_to_screen_top(&mut app); }
                        "middle-line" => { crate::copy_mode::move_to_screen_middle(&mut app); }
                        "bottom-line" => { crate::copy_mode::move_to_screen_bottom(&mut app); }
                        "history-top" => { crate::copy_mode::scroll_to_top(&mut app); }
                        "history-bottom" => { crate::copy_mode::scroll_to_bottom(&mut app); }
                        "halfpage-up" => {
                            let half = app.windows.get(app.active_idx)
                                .and_then(|w| active_pane(&w.root, &w.active_path))
                                .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                            scroll_copy_up(&mut app, half);
                        }
                        "halfpage-down" => {
                            let half = app.windows.get(app.active_idx)
                                .and_then(|w| active_pane(&w.root, &w.active_path))
                                .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                            scroll_copy_down(&mut app, half);
                        }
                        "page-up" => { scroll_copy_up(&mut app, 20); }
                        "page-down" => { scroll_copy_down(&mut app, 20); }
                        "scroll-up" => { scroll_copy_up(&mut app, 1); }
                        "scroll-down" => { scroll_copy_down(&mut app, 1); }
                        "search-forward" | "search-forward-incremental" => {
                            app.mode = Mode::CopySearch { input: String::new(), forward: true };
                        }
                        "search-backward" | "search-backward-incremental" => {
                            app.mode = Mode::CopySearch { input: String::new(), forward: false };
                        }
                        "search-again" => { crate::copy_mode::search_next(&mut app); }
                        "search-reverse" => { crate::copy_mode::search_prev(&mut app); }
                        "copy-end-of-line" => { let _ = crate::copy_mode::copy_end_of_line(&mut app); app.mode = Mode::Passthrough; app.copy_scroll_offset = 0; app.copy_pos = None; }
                        "select-word" => {
                            // Select the word under cursor
                            crate::copy_mode::move_word_backward(&mut app);
                            if let Some((r,c)) = crate::copy_mode::get_copy_pos(&mut app) {
                                app.copy_anchor = Some((r,c));
                                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                                app.copy_selection_mode = crate::types::SelectionMode::Char;
                            }
                            crate::copy_mode::move_word_end(&mut app);
                        }
                        "other-end" => {
                            if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                                app.copy_anchor = Some(p);
                                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                                app.copy_pos = Some(a);
                            }
                        }
                        "clear-selection" => {
                            app.copy_anchor = None;
                            app.copy_selection_mode = crate::types::SelectionMode::Char;
                        }
                        "append-selection" => {
                            // Append to existing buffer instead of replacing
                            let _ = yank_selection(&mut app);
                            if app.paste_buffers.len() >= 2 {
                                let appended = format!("{}{}", app.paste_buffers[1], app.paste_buffers[0]);
                                app.paste_buffers[0] = appended;
                            }
                        }
                        "append-selection-and-cancel" => {
                            let _ = yank_selection(&mut app);
                            if app.paste_buffers.len() >= 2 {
                                let appended = format!("{}{}", app.paste_buffers[1], app.paste_buffers[0]);
                                app.paste_buffers[0] = appended;
                            }
                            app.mode = Mode::Passthrough;
                            app.copy_scroll_offset = 0;
                            app.copy_pos = None;
                        }
                        "copy-line" => {
                            // Select entire current line and yank
                            if let Some((r, _)) = crate::copy_mode::get_copy_pos(&mut app) {
                                app.copy_anchor = Some((r, 0));
                                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                                app.copy_selection_mode = crate::types::SelectionMode::Line;
                                let cols = app.windows.get(app.active_idx)
                                    .and_then(|w| active_pane(&w.root, &w.active_path))
                                    .map(|p| p.last_cols).unwrap_or(80);
                                app.copy_pos = Some((r, cols.saturating_sub(1)));
                                let _ = yank_selection(&mut app);
                            }
                            app.mode = Mode::Passthrough;
                            app.copy_scroll_offset = 0;
                            app.copy_pos = None;
                        }
                        s if s.starts_with("goto-line") => {
                            // goto-line <N> — jump to line N in scrollback
                            let n = s.strip_prefix("goto-line").unwrap_or("").trim()
                                .parse::<u16>().unwrap_or(0);
                            app.copy_pos = Some((n, 0));
                        }
                        "jump-forward" => { app.copy_find_char_pending = Some(0); }
                        "jump-backward" => { app.copy_find_char_pending = Some(1); }
                        "jump-to-forward" => { app.copy_find_char_pending = Some(2); }
                        "jump-to-backward" => { app.copy_find_char_pending = Some(3); }
                        "jump-again" => {
                            // Repeat last find-char in same direction
                            // We'd need to store last char; for now emit the pending
                        }
                        "jump-reverse" => {
                            // Repeat last find-char in reverse direction
                        }
                        "next-paragraph" => {
                            crate::copy_mode::move_next_paragraph(&mut app);
                        }
                        "previous-paragraph" => {
                            crate::copy_mode::move_prev_paragraph(&mut app);
                        }
                        "next-matching-bracket" => {
                            crate::copy_mode::move_matching_bracket(&mut app);
                        }
                        "stop-selection" => {
                            // Keep cursor position but stop extending selection
                            app.copy_anchor = None;
                        }
                        _ => {} // ignore unknown copy-mode commands
                    }
                }
                CtrlReq::SelectPane(dir) => {
                    // Auto-unzoom when navigating to another pane (tmux behavior).
                    // For directional nav: unzoom first so compute_rects uses
                    // real geometry, then re-zoom only if focus didn't change.
                    // For other cases: only unzoom if focus actually changes.
                    // (fixes #46)
                    match dir.as_str() {
                        "U" | "D" | "L" | "R" => {
                            let focus_dir = match dir.as_str() {
                                "U" => FocusDir::Up, "D" => FocusDir::Down,
                                "L" => FocusDir::Left, _ => FocusDir::Right,
                            };
                            let was_zoomed = unzoom_if_zoomed(&mut app);
                            let old_path = app.windows[app.active_idx].active_path.clone();
                            switch_with_copy_save(&mut app, |app| {
                                move_focus(app, focus_dir);
                            });
                            if app.windows[app.active_idx].active_path != old_path {
                                // Focus changed — stay unzoomed (tmux behavior)
                                app.last_pane_path = old_path;
                            } else if was_zoomed {
                                // No pane in that direction — re-zoom
                                toggle_zoom(&mut app);
                            }
                        }
                        "last" => {
                            // select-pane -l: switch to last active pane
                            let old_path = app.windows[app.active_idx].active_path.clone();
                            switch_with_copy_save(&mut app, |app| {
                                let win = &mut app.windows[app.active_idx];
                                if !app.last_pane_path.is_empty() {
                                    let tmp = win.active_path.clone();
                                    win.active_path = app.last_pane_path.clone();
                                    app.last_pane_path = tmp;
                                }
                            });
                            if app.windows[app.active_idx].active_path != old_path {
                                unzoom_if_zoomed(&mut app);
                            }
                        }
                        "mark" => {
                            // select-pane -m: mark the current pane
                            let win = &app.windows[app.active_idx];
                            if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                                app.marked_pane = Some((app.active_idx, pid));
                            }
                        }
                        "next" => {
                            // select-pane next: cycle to next pane (like Prefix+o / tmux -t :.+)
                            let old_path = app.windows[app.active_idx].active_path.clone();
                            switch_with_copy_save(&mut app, |app| {
                                let win = &app.windows[app.active_idx];
                                let mut pane_paths = Vec::new();
                                let mut path = Vec::new();
                                collect_pane_paths_server(&win.root, &mut path, &mut pane_paths);
                                if let Some(cur) = pane_paths.iter().position(|p| *p == win.active_path) {
                                    let next = (cur + 1) % pane_paths.len();
                                    let new_path = pane_paths[next].clone();
                                    let win = &mut app.windows[app.active_idx];
                                    app.last_pane_path = win.active_path.clone();
                                    win.active_path = new_path;
                                }
                            });
                            if app.windows[app.active_idx].active_path != old_path {
                                unzoom_if_zoomed(&mut app);
                            }
                        }
                        "prev" => {
                            // select-pane prev: cycle to previous pane (tmux -t :.-)
                            let old_path = app.windows[app.active_idx].active_path.clone();
                            switch_with_copy_save(&mut app, |app| {
                                let win = &app.windows[app.active_idx];
                                let mut pane_paths = Vec::new();
                                let mut path = Vec::new();
                                collect_pane_paths_server(&win.root, &mut path, &mut pane_paths);
                                if let Some(cur) = pane_paths.iter().position(|p| *p == win.active_path) {
                                    let prev = (cur + pane_paths.len() - 1) % pane_paths.len();
                                    let new_path = pane_paths[prev].clone();
                                    let win = &mut app.windows[app.active_idx];
                                    app.last_pane_path = win.active_path.clone();
                                    win.active_path = new_path;
                                }
                            });
                            if app.windows[app.active_idx].active_path != old_path {
                                unzoom_if_zoomed(&mut app);
                            }
                        }
                        "unmark" => {
                            // select-pane -M: clear the marked pane
                            app.marked_pane = None;
                        }
                        _ => {}
                    }
                    meta_dirty = true;
                    hook_event = Some("after-select-pane");
                }
                CtrlReq::SelectWindow(idx) => {
                    if idx >= app.window_base_index {
                        let internal_idx = idx - app.window_base_index;
                        if internal_idx < app.windows.len() && internal_idx != app.active_idx {
                            switch_with_copy_save(&mut app, |app| {
                                app.last_window_idx = app.active_idx;
                                app.active_idx = internal_idx;
                            });
                            resize_all_panes(&mut app);
                        }
                    }
                    meta_dirty = true;
                    hook_event = Some("after-select-window");
                }
                CtrlReq::ListPanes(resp) => {
                    let mut output = String::new();
                    let win = &app.windows[app.active_idx];
                    fn collect_panes(node: &Node, panes: &mut Vec<(usize, u16, u16, vt100::MouseProtocolMode, vt100::MouseProtocolEncoding, bool)>) {
                        match node {
                            Node::Leaf(p) => {
                                let (mode, enc, alt) = match p.term.lock() {
                                    Ok(term) => {
                                        let screen = term.screen();
                                        (screen.mouse_protocol_mode(), screen.mouse_protocol_encoding(), screen.alternate_screen())
                                    }
                                    Err(_) => {
                                        // Mutex poisoned — reader thread panicked.  Use safe defaults.
                                        (vt100::MouseProtocolMode::None, vt100::MouseProtocolEncoding::Default, false)
                                    }
                                };
                                panes.push((p.id, p.last_cols, p.last_rows, mode, enc, alt));
                            }
                            Node::Split { children, .. } => {
                                for c in children { collect_panes(c, panes); }
                            }
                        }
                    }
                    let mut panes = Vec::new();
                    collect_panes(&win.root, &mut panes);
                    let active_pane_id = crate::tree::get_active_pane_id(&win.root, &win.active_path);
                    for (pos, (id, cols, rows, _mode, _enc, _alt)) in panes.iter().enumerate() {
                        let idx = pos + app.pane_base_index;
                        let active_marker = if active_pane_id == Some(*id) { " (active)" } else { "" };
                        output.push_str(&format!("{}: [{}x{}] [history {}/{}, 0 bytes] %{}{}\n", idx, cols, rows, app.history_limit, app.history_limit, id, active_marker));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListPanesFormat(resp, fmt) => {
                    let text = format_list_panes(&app, &fmt, app.active_idx);
                    let _ = resp.send(text);
                }
                CtrlReq::ListAllPanes(resp) => {
                    let mut output = String::new();
                    fn collect_all_panes(node: &Node, panes: &mut Vec<(usize, u16, u16)>) {
                        match node {
                            Node::Leaf(p) => { panes.push((p.id, p.last_cols, p.last_rows)); }
                            Node::Split { children, .. } => { for c in children { collect_all_panes(c, panes); } }
                        }
                    }
                    for (wi, win) in app.windows.iter().enumerate() {
                        let mut panes = Vec::new();
                        collect_all_panes(&win.root, &mut panes);
                        for (id, cols, rows) in panes {
                            output.push_str(&format!("{}:{}: %{} [{}x{}]\n", app.session_name, wi + app.window_base_index, id, cols, rows));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListAllPanesFormat(resp, fmt) => {
                    let mut lines = Vec::new();
                    for wi in 0..app.windows.len() {
                        lines.push(format_list_panes(&app, &fmt, wi));
                    }
                    let _ = resp.send(lines.join("\n"));
                }
                CtrlReq::KillWindow => {
                    if app.windows.len() > 1 {
                        let mut win = app.windows.remove(app.active_idx);
                        kill_all_children(&mut win.root);
                        if app.active_idx >= app.windows.len() { app.active_idx = app.windows.len() - 1; }
                    } else {
                        // Last window: kill all children; reaper will detect empty session and exit
                        kill_all_children(&mut app.windows[0].root);
                    }
                    hook_event = Some("window-closed");
                }
                CtrlReq::KillSession => {
                    // Kill all child processes in all windows before exiting
                    for win in app.windows.iter_mut() {
                        kill_all_children(&mut win.root);
                    }
                    // Kill warm pane's child (process::exit skips Drop)
                    if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                    // Brief delay to let child processes fully terminate
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                    let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                    let _ = std::fs::remove_file(&regpath);
                    let _ = std::fs::remove_file(&keypath);
                    crate::types::shutdown_persistent_streams();
                    std::process::exit(0);
                }
                CtrlReq::HasSession(resp) => {
                    let _ = resp.send(true);
                }
                CtrlReq::RenameSession(name) => {
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let old_path = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                    let old_keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                    // Compute new port file base with socket_name prefix
                    let new_base = if let Some(ref sn) = app.socket_name {
                        format!("{}__{}" , sn, name)
                    } else {
                        name.clone()
                    };
                    let new_path = format!("{}\\.psmux\\{}.port", home, new_base);
                    let new_keypath = format!("{}\\.psmux\\{}.key", home, new_base);
                    if let Some(port) = app.control_port {
                        let _ = std::fs::remove_file(&old_path);
                        let _ = std::fs::write(&new_path, port.to_string());
                        if let Ok(key) = std::fs::read_to_string(&old_keypath) {
                            let _ = std::fs::remove_file(&old_keypath);
                            let _ = std::fs::write(&new_keypath, key);
                        }
                    }
                    app.session_name = name;
                    hook_event = Some("after-rename-session");
                }
                CtrlReq::ClaimSession(name, resp) => {
                    // Same as RenameSession but with a synchronous response
                    // so the CLI knows the rename completed before attaching.
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let old_path = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                    let old_keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                    let new_base = if let Some(ref sn) = app.socket_name {
                        format!("{}__{}" , sn, name)
                    } else {
                        name.clone()
                    };
                    let new_path = format!("{}\\.psmux\\{}.port", home, new_base);
                    let new_keypath = format!("{}\\.psmux\\{}.key", home, new_base);
                    if let Some(port) = app.control_port {
                        let _ = std::fs::remove_file(&old_path);
                        let _ = std::fs::write(&new_path, port.to_string());
                        if let Ok(key) = std::fs::read_to_string(&old_keypath) {
                            let _ = std::fs::remove_file(&old_keypath);
                            let _ = std::fs::write(&new_keypath, key);
                        }
                    }
                    app.session_name = name;
                    let _ = resp.send("OK\n".to_string());
                    // Spawn a replacement warm server for the NEXT new-session
                    spawn_warm_server(&app);
                    hook_event = Some("after-rename-session");
                }
                CtrlReq::SwapPane(dir) => {
                    match dir.as_str() {
                        "U" => { swap_pane(&mut app, FocusDir::Up); }
                        "D" => { swap_pane(&mut app, FocusDir::Down); }
                        _ => { swap_pane(&mut app, FocusDir::Down); }
                    }
                    hook_event = Some("after-swap-pane");
                }
                CtrlReq::ResizePane(dir, amount) => {
                    match dir.as_str() {
                        "U" | "D" => { resize_pane_vertical(&mut app, if dir == "U" { -(amount as i16) } else { amount as i16 }); }
                        "L" | "R" => { resize_pane_horizontal(&mut app, if dir == "L" { -(amount as i16) } else { amount as i16 }); }
                        _ => {}
                    }
                    hook_event = Some("after-resize-pane");
                }
                CtrlReq::SetBuffer(content) => {
                    app.paste_buffers.insert(0, content);
                    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
                }
                CtrlReq::ListBuffers(resp) => {
                    let mut output = String::new();
                    for (i, buf) in app.paste_buffers.iter().enumerate() {
                        let preview: String = buf.chars().take(50).collect();
                        output.push_str(&format!("buffer{}: {} bytes: \"{}\"\n", i, buf.len(), preview));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListBuffersFormat(resp, fmt) => {
                    let mut output = Vec::new();
                    for (i, _buf) in app.paste_buffers.iter().enumerate() {
                        set_buffer_idx_override(Some(i));
                        output.push(expand_format(&fmt, &app));
                        set_buffer_idx_override(None);
                    }
                    let _ = resp.send(output.join("\n"));
                }
                CtrlReq::ShowBuffer(resp) => {
                    let content = app.paste_buffers.first().cloned().unwrap_or_default();
                    let _ = resp.send(content);
                }
                CtrlReq::ShowBufferAt(resp, idx) => {
                    let content = app.paste_buffers.get(idx).cloned().unwrap_or_default();
                    let _ = resp.send(content);
                }
                CtrlReq::DeleteBuffer => {
                    if !app.paste_buffers.is_empty() { app.paste_buffers.remove(0); }
                }
                CtrlReq::DisplayMessage(resp, fmt) => {
                    let result = expand_format(&fmt, &app);
                    let _ = resp.send(result);
                }
                CtrlReq::LastWindow => {
                    if app.windows.len() > 1 && app.last_window_idx < app.windows.len() {
                        switch_with_copy_save(&mut app, |app| {
                            let tmp = app.active_idx;
                            app.active_idx = app.last_window_idx;
                            app.last_window_idx = tmp;
                        });
                    }
                    meta_dirty = true;
                    hook_event = Some("after-select-window");
                }
                CtrlReq::LastPane => {
                    switch_with_copy_save(&mut app, |app| {
                        let win = &mut app.windows[app.active_idx];
                        if !app.last_pane_path.is_empty() && path_exists(&win.root, &app.last_pane_path) {
                            let tmp = win.active_path.clone();
                            win.active_path = app.last_pane_path.clone();
                            app.last_pane_path = tmp;
                        } else if !win.active_path.is_empty() {
                            let last = win.active_path.last_mut();
                            if let Some(idx) = last {
                                *idx = (*idx + 1) % 2;
                            }
                        }
                    });
                    meta_dirty = true;
                }
                CtrlReq::RotateWindow(reverse) => {
                    rotate_panes(&mut app, reverse);
                    hook_event = Some("after-rotate-window");
                }
                CtrlReq::DisplayPanes => {
                    app.mode = Mode::PaneChooser { opened_at: std::time::Instant::now() };
                    state_dirty = true;
                }
                CtrlReq::BreakPane => {
                    break_pane_to_window(&mut app);
                    hook_event = Some("after-break-pane");
                    meta_dirty = true;
                }
                CtrlReq::JoinPane(target_win) => {
                    // Real join-pane: extract active pane from current window and
                    // graft it as a vertical split into the target window.
                    let src_idx = app.active_idx;
                    if target_win < app.windows.len() && target_win != src_idx {
                        let src_path = app.windows[src_idx].active_path.clone();
                        let src_root = std::mem::replace(&mut app.windows[src_idx].root,
                            Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
                        let (remaining, extracted) = tree::extract_node(src_root, &src_path);
                        if let Some(pane_node) = extracted {
                            let src_empty = remaining.is_none();
                            if let Some(rem) = remaining {
                                app.windows[src_idx].root = rem;
                                app.windows[src_idx].active_path = tree::first_leaf_path(&app.windows[src_idx].root);
                            }
                            // Adjust target index if source window will be removed
                            let tgt = if src_empty && target_win > src_idx { target_win - 1 } else { target_win };
                            if src_empty {
                                app.windows.remove(src_idx);
                                if app.active_idx >= app.windows.len() {
                                    app.active_idx = app.windows.len().saturating_sub(1);
                                }
                            }
                            // Graft pane into target window
                            if tgt < app.windows.len() {
                                let tgt_path = app.windows[tgt].active_path.clone();
                                tree::replace_leaf_with_split(&mut app.windows[tgt].root, &tgt_path, LayoutKind::Vertical, pane_node);
                                app.active_idx = tgt;
                            }
                            resize_all_panes(&mut app);
                            meta_dirty = true;
                            hook_event = Some("after-join-pane");
                        } else {
                            // Extraction failed — restore
                            if let Some(rem) = remaining {
                                app.windows[src_idx].root = rem;
                            }
                        }
                    }
                }
                CtrlReq::RespawnPane => {
                    respawn_active_pane(&mut app, Some(&*pty_system))?;
                    hook_event = Some("after-respawn-pane");
                }
                CtrlReq::BindKey(table_name, key, command, repeat) => {
                    if let Some(kc) = parse_key_string(&key) {
                        let kc = normalize_key_for_binding(kc);
                        // Support `\;` chaining in server-side bind-key
                        let sub_cmds = crate::config::split_chained_commands_pub(&command);
                        let action = if sub_cmds.len() > 1 {
                            Some(Action::CommandChain(sub_cmds))
                        } else {
                            parse_command_to_action(&command)
                        };
                        if let Some(act) = action {
                            let table = app.key_tables.entry(table_name).or_default();
                            table.retain(|b| b.key != kc);
                            table.push(Bind { key: kc, action: act, repeat });
                        }
                    }
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::UnbindKey(key) => {
                    if let Some(kc) = parse_key_string(&key) {
                        let kc = normalize_key_for_binding(kc);
                        for table in app.key_tables.values_mut() {
                            table.retain(|b| b.key != kc);
                        }
                    }
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::ListKeys(resp) => {
                    // Build list-keys output from the canonical help module
                    let user_iter = app.key_tables.iter().flat_map(|(table_name, binds)| {
                        binds.iter().map(move |bind| {
                            let key_str = format_key_binding(&bind.key);
                            let action_str = format_action(&bind.action);
                            (table_name.as_str(), key_str, action_str, bind.repeat)
                        })
                    });
                    let output = help::build_list_keys_output(user_iter);
                    let _ = resp.send(output);
                }
                CtrlReq::SetOption(option, value) => {
                    apply_set_option(&mut app, &option, &value, false);
                    // Update shared aliases if command-alias changed
                    if option == "command-alias" {
                        if let Ok(mut map) = shared_aliases_main.write() {
                            *map = app.command_aliases.clone();
                        }
                    }
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::SetOptionQuiet(option, value, quiet) => {
                    apply_set_option(&mut app, &option, &value, quiet);
                    // Update shared aliases if command-alias changed
                    if option == "command-alias" {
                        if let Ok(mut map) = shared_aliases_main.write() {
                            *map = app.command_aliases.clone();
                        }
                    }
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::SetOptionUnset(option) => {
                    // Reset option to default or remove @user-option
                    if option.starts_with('@') {
                        app.environment.remove(&option);
                    } else {
                        match option.as_str() {
                            "status-left" => { app.status_left = "psmux:#I".to_string(); }
                            "status-right" => { app.status_right = "#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y".to_string(); }
                            "mouse" => { app.mouse_enabled = true; }
                            "escape-time" => { app.escape_time_ms = 500; }
                            "history-limit" => { app.history_limit = 2000; }
                            "display-time" => { app.display_time_ms = 750; }
                            "mode-keys" => { app.mode_keys = "emacs".to_string(); }
                            "status" => { app.status_visible = true; }
                            "status-position" => { app.status_position = "bottom".to_string(); }
                            "status-style" => { app.status_style = String::new(); }
                            "renumber-windows" => { app.renumber_windows = false; }
                            "remain-on-exit" => { app.remain_on_exit = false; }
                            "destroy-unattached" => { app.destroy_unattached = false; }
                            "exit-empty" => { app.exit_empty = true; }
                            "automatic-rename" => { app.automatic_rename = true; }
                            "pane-border-style" => { app.pane_border_style = String::new(); }
                            "pane-active-border-style" => { app.pane_active_border_style = "fg=green".to_string(); }
                            "window-status-format" => { app.window_status_format = "#I:#W#{?window_flags,#{window_flags}, }".to_string(); }
                            "window-status-current-format" => { app.window_status_current_format = "#I:#W#{?window_flags,#{window_flags}, }".to_string(); }
                            "window-status-separator" => { app.window_status_separator = " ".to_string(); }
                            "cursor-style" => { std::env::set_var("PSMUX_CURSOR_STYLE", "bar"); }
                            "cursor-blink" => { std::env::set_var("PSMUX_CURSOR_BLINK", "1"); }
                            _ => {}
                        }
                    }
                }
                CtrlReq::SetOptionAppend(option, value) => {
                    // Append to existing option value
                    if option.starts_with('@') {
                        let existing = app.environment.get(&option).cloned().unwrap_or_default();
                        app.environment.insert(option, format!("{}{}", existing, value));
                    } else {
                        match option.as_str() {
                            "status-left" => { app.status_left.push_str(&value); }
                            "status-right" => { app.status_right.push_str(&value); }
                            "status-style" => { app.status_style.push_str(&value); }
                            "pane-border-style" => { app.pane_border_style.push_str(&value); }
                            "pane-active-border-style" => { app.pane_active_border_style.push_str(&value); }
                            "window-status-format" => { app.window_status_format.push_str(&value); }
                            "window-status-current-format" => { app.window_status_current_format.push_str(&value); }
                            _ => {}
                        }
                    }
                }
                CtrlReq::ShowOptions(resp) => {
                    let mut output = String::new();
                    output.push_str(&format!("prefix {}\n", format_key_binding(&app.prefix_key)));
                    if let Some(ref p2) = app.prefix2_key {
                        output.push_str(&format!("prefix2 {}\n", format_key_binding(p2)));
                    }
                    output.push_str(&format!("base-index {}\n", app.window_base_index));
                    output.push_str(&format!("pane-base-index {}\n", app.pane_base_index));
                    output.push_str(&format!("escape-time {}\n", app.escape_time_ms));
                    output.push_str(&format!("mouse {}\n", if app.mouse_enabled { "on" } else { "off" }));
                    output.push_str(&format!("status {}\n", if app.status_visible { "on" } else { "off" }));
                    output.push_str(&format!("status-position {}\n", app.status_position));
                    output.push_str(&format!("status-left \"{}\"\n", app.status_left));
                    output.push_str(&format!("status-right \"{}\"\n", app.status_right));
                    output.push_str(&format!("history-limit {}\n", app.history_limit));
                    output.push_str(&format!("display-time {}\n", app.display_time_ms));
                    output.push_str(&format!("display-panes-time {}\n", app.display_panes_time_ms));
                    output.push_str(&format!("mode-keys {}\n", app.mode_keys));
                    output.push_str(&format!("focus-events {}\n", if app.focus_events { "on" } else { "off" }));
                    output.push_str(&format!("renumber-windows {}\n", if app.renumber_windows { "on" } else { "off" }));
                    output.push_str(&format!("automatic-rename {}\n", if app.automatic_rename { "on" } else { "off" }));
                    output.push_str(&format!("monitor-activity {}\n", if app.monitor_activity { "on" } else { "off" }));
                    output.push_str(&format!("synchronize-panes {}\n", if app.sync_input { "on" } else { "off" }));
                    output.push_str(&format!("remain-on-exit {}\n", if app.remain_on_exit { "on" } else { "off" }));
                    output.push_str(&format!("destroy-unattached {}\n", if app.destroy_unattached { "on" } else { "off" }));
                    output.push_str(&format!("exit-empty {}\n", if app.exit_empty { "on" } else { "off" }));
                    output.push_str(&format!("set-titles {}\n", if app.set_titles { "on" } else { "off" }));
                    if !app.set_titles_string.is_empty() {
                        output.push_str(&format!("set-titles-string \"{}\"\n", app.set_titles_string));
                    }
                    output.push_str(&format!(
                        "prediction-dimming {}\n",
                        if app.prediction_dimming { "on" } else { "off" }
                    ));
                    output.push_str(&format!("cursor-style {}\n", std::env::var("PSMUX_CURSOR_STYLE").unwrap_or_else(|_| "bar".to_string())));
                    output.push_str(&format!("cursor-blink {}\n", if std::env::var("PSMUX_CURSOR_BLINK").unwrap_or_else(|_| "1".to_string()) != "0" { "on" } else { "off" }));
                    if !app.default_shell.is_empty() {
                        output.push_str(&format!("default-shell {}\n", app.default_shell));
                    }
                    output.push_str(&format!("word-separators \"{}\"\n", app.word_separators));
                    if !app.pane_border_style.is_empty() {
                        output.push_str(&format!("pane-border-style \"{}\"\n", app.pane_border_style));
                    }
                    if !app.pane_active_border_style.is_empty() {
                        output.push_str(&format!("pane-active-border-style \"{}\"\n", app.pane_active_border_style));
                    }
                    if !app.status_style.is_empty() {
                        output.push_str(&format!("status-style \"{}\"\n", app.status_style));
                    }
                    if !app.status_left_style.is_empty() {
                        output.push_str(&format!("status-left-style \"{}\"\n", app.status_left_style));
                    }
                    if !app.status_right_style.is_empty() {
                        output.push_str(&format!("status-right-style \"{}\"\n", app.status_right_style));
                    }
                    output.push_str(&format!("status-interval {}\n", app.status_interval));
                    output.push_str(&format!("status-justify {}\n", app.status_justify));
                    output.push_str(&format!("window-status-format \"{}\"\n", app.window_status_format));
                    output.push_str(&format!("window-status-current-format \"{}\"\n", app.window_status_current_format));
                    if !app.window_status_style.is_empty() {
                        output.push_str(&format!("window-status-style \"{}\"\n", app.window_status_style));
                    }
                    if !app.window_status_current_style.is_empty() {
                        output.push_str(&format!("window-status-current-style \"{}\"\n", app.window_status_current_style));
                    }
                    if !app.window_status_activity_style.is_empty() {
                        output.push_str(&format!("window-status-activity-style \"{}\"\n", app.window_status_activity_style));
                    }
                    if !app.message_style.is_empty() {
                        output.push_str(&format!("message-style \"{}\"\n", app.message_style));
                    }
                    if !app.message_command_style.is_empty() {
                        output.push_str(&format!("message-command-style \"{}\"\n", app.message_command_style));
                    }
                    if !app.mode_style.is_empty() {
                        output.push_str(&format!("mode-style \"{}\"\n", app.mode_style));
                    }
                    // Include @user-options (used by plugins)
                    for (key, val) in &app.environment {
                        if key.starts_with('@') {
                            output.push_str(&format!("{} \"{}\"\n", key, val));
                        }
                    }
                    // New options
                    output.push_str(&format!("main-pane-width {}\n", app.main_pane_width));
                    output.push_str(&format!("main-pane-height {}\n", app.main_pane_height));
                    output.push_str(&format!("status-left-length {}\n", app.status_left_length));
                    output.push_str(&format!("status-right-length {}\n", app.status_right_length));
                    output.push_str(&format!("window-size {}\n", app.window_size));
                    output.push_str(&format!("allow-passthrough {}\n", app.allow_passthrough));
                    output.push_str(&format!("set-clipboard {}\n", app.set_clipboard));
                    if !app.copy_command.is_empty() {
                        output.push_str(&format!("copy-command \"{}\"\n", app.copy_command));
                    }
                    for (alias, expansion) in &app.command_aliases {
                        output.push_str(&format!("command-alias \"{}={}\"\n", alias, expansion));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::SourceFile(path) => {
                    // Use config helper for standard source-file behavior (-F support,
                    // nested parse context). Keep direct glob handling for wildcard sources.
                    let is_format_expand = path.starts_with("-F ") || path.starts_with("-F\t");
                    let path_for_glob = if is_format_expand { path[3..].trim() } else { &path };
                    if !is_format_expand && (path_for_glob.contains('*') || path_for_glob.contains('?')) {
                        let expanded = if path_for_glob.starts_with('~') {
                            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                            path_for_glob.replacen('~', &home, 1)
                        } else {
                            path_for_glob.to_string()
                        };
                        if let Ok(entries) = glob::glob(&expanded) {
                            for entry in entries.flatten() {
                                if let Ok(contents) = std::fs::read_to_string(&entry) {
                                    parse_config_content(&mut app, &contents);
                                }
                            }
                        }
                    } else {
                        crate::config::source_file(&mut app, &path);
                    }
                }
                CtrlReq::MoveWindow(target) => {
                    if let Some(t) = target {
                        if t < app.windows.len() && app.active_idx != t {
                            let win = app.windows.remove(app.active_idx);
                            let insert_idx = if t > app.active_idx { t - 1 } else { t };
                            app.windows.insert(insert_idx.min(app.windows.len()), win);
                            app.active_idx = insert_idx.min(app.windows.len() - 1);
                        }
                    }
                }
                CtrlReq::SwapWindow(target) => {
                    if target < app.windows.len() && app.active_idx != target {
                        app.windows.swap(app.active_idx, target);
                    }
                }
                CtrlReq::LinkWindow(_target) => {}
                CtrlReq::UnlinkWindow => {
                    if app.windows.len() > 1 {
                        let mut win = app.windows.remove(app.active_idx);
                        kill_all_children(&mut win.root);
                        if app.active_idx >= app.windows.len() {
                            app.active_idx = app.windows.len() - 1;
                        }
                    }
                }
                CtrlReq::FindWindow(resp, pattern) => {
                    let mut output = String::new();
                    for (i, win) in app.windows.iter().enumerate() {
                        if win.name.contains(&pattern) {
                            output.push_str(&format!("{}: {} []\n", i + app.window_base_index, win.name));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::MovePane(target_win) => {
                    // move-pane is an alias for join-pane
                    let src_idx = app.active_idx;
                    if target_win < app.windows.len() && target_win != src_idx {
                        let src_path = app.windows[src_idx].active_path.clone();
                        let src_root = std::mem::replace(&mut app.windows[src_idx].root,
                            Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
                        let (remaining, extracted) = tree::extract_node(src_root, &src_path);
                        if let Some(pane_node) = extracted {
                            let src_empty = remaining.is_none();
                            if let Some(rem) = remaining {
                                app.windows[src_idx].root = rem;
                                app.windows[src_idx].active_path = tree::first_leaf_path(&app.windows[src_idx].root);
                            }
                            let tgt = if src_empty && target_win > src_idx { target_win - 1 } else { target_win };
                            if src_empty {
                                app.windows.remove(src_idx);
                                if app.active_idx >= app.windows.len() {
                                    app.active_idx = app.windows.len().saturating_sub(1);
                                }
                            }
                            if tgt < app.windows.len() {
                                let tgt_path = app.windows[tgt].active_path.clone();
                                tree::replace_leaf_with_split(&mut app.windows[tgt].root, &tgt_path, LayoutKind::Vertical, pane_node);
                                app.active_idx = tgt;
                            }
                            resize_all_panes(&mut app);
                            meta_dirty = true;
                        } else {
                            if let Some(rem) = remaining {
                                app.windows[src_idx].root = rem;
                            }
                        }
                    }
                }
                CtrlReq::PipePane(cmd, stdin, stdout, toggle) => {
                    let win = &app.windows[app.active_idx];
                    let pane_id = get_active_pane_id(&win.root, &win.active_path).unwrap_or(0);
                    let has_existing = app.pipe_panes.iter().any(|p| p.pane_id == pane_id);
                    
                    if cmd.is_empty() {
                        // No command: close any existing pipe on this pane
                        if let Some(idx) = app.pipe_panes.iter().position(|p| p.pane_id == pane_id) {
                            if let Some(ref mut proc) = app.pipe_panes[idx].process {
                                let _ = proc.kill();
                            }
                            app.pipe_panes.remove(idx);
                        }
                    } else if toggle && has_existing {
                        // -o flag with existing pipe: close it (toggle off), don't start new
                        if let Some(idx) = app.pipe_panes.iter().position(|p| p.pane_id == pane_id) {
                            if let Some(ref mut proc) = app.pipe_panes[idx].process {
                                let _ = proc.kill();
                            }
                            app.pipe_panes.remove(idx);
                        }
                    } else {
                        // Close any existing pipe first (replace)
                        if let Some(idx) = app.pipe_panes.iter().position(|p| p.pane_id == pane_id) {
                            if let Some(ref mut proc) = app.pipe_panes[idx].process {
                                let _ = proc.kill();
                            }
                            app.pipe_panes.remove(idx);
                        }
                        // Start new pipe
                        #[cfg(windows)]
                        let process = std::process::Command::new("pwsh")
                            .args(["-NoProfile", "-Command", &cmd])
                            .stdin(if stdout { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                            .stdout(if stdin { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .ok();
                        #[cfg(not(windows))]
                        let process = std::process::Command::new("sh")
                            .args(["-c", &cmd])
                            .stdin(if stdout { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                            .stdout(if stdin { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .ok();
                        
                        app.pipe_panes.push(PipePaneState {
                            pane_id,
                            process,
                            stdin,
                            stdout,
                        });
                    }
                }
                CtrlReq::SelectLayout(layout) => {
                    apply_layout(&mut app, &layout);
                    state_dirty = true;
                }
                CtrlReq::NextLayout => {
                    cycle_layout(&mut app);
                    state_dirty = true;
                }
                CtrlReq::ListClients(resp) => {
                    let mut output = String::new();
                    output.push_str(&format!("/dev/pts/0: {}: {} [{}x{}] (utf8)\n", 
                        app.session_name, 
                        app.windows[app.active_idx].name,
                        app.last_window_area.width,
                        app.last_window_area.height
                    ));
                    let _ = resp.send(output);
                }
                CtrlReq::SwitchClient(_target) => {}
                CtrlReq::SwitchClientTable(table) => {
                    app.current_key_table = Some(table);
                    state_dirty = true;
                }
                CtrlReq::ListCommands(resp) => {
                    let cmds = TMUX_COMMANDS.join("\n");
                    let _ = resp.send(cmds);
                }
                CtrlReq::LockClient => {}
                CtrlReq::RefreshClient => { state_dirty = true; meta_dirty = true; }
                CtrlReq::SuspendClient => {}
                CtrlReq::CopyModePageUp => {
                    enter_copy_mode(&mut app);
                    move_copy_cursor(&mut app, 0, -20);
                }
                CtrlReq::ClearHistory => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                        if let Ok(mut parser) = p.term.lock() {
                            *parser = vt100::Parser::new(p.last_rows, p.last_cols, app.history_limit);
                        }
                    }
                }
                CtrlReq::SaveBuffer(path) => {
                    if let Some(content) = app.paste_buffers.first() {
                        let _ = std::fs::write(&path, content);
                    }
                }
                CtrlReq::LoadBuffer(path) => {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        app.paste_buffers.insert(0, content);
                        if app.paste_buffers.len() > 10 {
                            app.paste_buffers.pop();
                        }
                    }
                }
                CtrlReq::SetEnvironment(key, value) => {
                    app.environment.insert(key.clone(), value.clone());
                    env::set_var(&key, &value);
                }
                CtrlReq::ShowEnvironment(resp) => {
                    let mut output = String::new();
                    // Show psmux/tmux-specific environment vars
                    for (key, value) in &app.environment {
                        output.push_str(&format!("{}={}\n", key, value));
                    }
                    // Also show inherited PSMUX_/TMUX_ vars from process env
                    for (key, value) in env::vars() {
                        if (key.starts_with("PSMUX") || key.starts_with("TMUX")) && !app.environment.contains_key(&key) {
                            output.push_str(&format!("{}={}\n", key, value));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::SetHook(hook, cmd) => {
                    app.hooks.entry(hook).or_insert_with(Vec::new).push(cmd);
                }
                CtrlReq::ShowHooks(resp) => {
                    let mut output = String::new();
                    for (name, commands) in &app.hooks {
                        for cmd in commands {
                            output.push_str(&format!("{} -> {}\n", name, cmd));
                        }
                    }
                    if output.is_empty() {
                        output.push_str("(no hooks)\n");
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::RemoveHook(hook) => {
                    app.hooks.remove(&hook);
                }
                CtrlReq::KillServer => {
                    // Kill all child processes in all windows before exiting
                    for win in app.windows.iter_mut() {
                        kill_all_children(&mut win.root);
                    }
                    // Kill warm pane's child (process::exit skips Drop)
                    if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                    // Brief delay to let child processes fully terminate
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                    let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                    let _ = std::fs::remove_file(&regpath);
                    let _ = std::fs::remove_file(&keypath);
                    crate::types::shutdown_persistent_streams();
                    std::process::exit(0);
                }
                CtrlReq::WaitFor(channel, op) => {
                    match op {
                        WaitForOp::Lock => {
                            let entry = app.wait_channels.entry(channel).or_insert_with(|| WaitChannel {
                                locked: false,
                                waiters: Vec::new(),
                            });
                            entry.locked = true;
                        }
                        WaitForOp::Unlock => {
                            if let Some(ch) = app.wait_channels.get_mut(&channel) {
                                ch.locked = false;
                                for waiter in ch.waiters.drain(..) {
                                    let _ = waiter.send(());
                                }
                            }
                        }
                        WaitForOp::Signal => {
                            if let Some(ch) = app.wait_channels.get_mut(&channel) {
                                for waiter in ch.waiters.drain(..) {
                                    let _ = waiter.send(());
                                }
                            }
                        }
                        WaitForOp::Wait => {
                            app.wait_channels.entry(channel).or_insert_with(|| WaitChannel {
                                locked: false,
                                waiters: Vec::new(),
                            });
                        }
                    }
                }
                CtrlReq::DisplayMenu(menu_def, x, y) => {
                    let menu = parse_menu_definition(&menu_def, x, y);
                    if !menu.items.is_empty() {
                        app.mode = Mode::MenuMode { menu };
                    }
                }
                CtrlReq::DisplayPopup(command, width, height, close_on_exit) => {
                    if !command.is_empty() {
                        // Try to spawn with PTY for interactive programs (fzf, etc.)
                        let pty_result = Some(portable_pty::native_pty_system())
                            .and_then(|pty_sys| {
                                let pty_size = portable_pty::PtySize { rows: height.saturating_sub(2), cols: width.saturating_sub(2), pixel_width: 0, pixel_height: 0 };
                                let pair = pty_sys.openpty(pty_size).ok()?;
                                let mut cmd_builder = portable_pty::CommandBuilder::new(if cfg!(windows) { "pwsh" } else { "sh" });
                                if let Ok(dir) = std::env::current_dir() { cmd_builder.cwd(dir); }
                                if cfg!(windows) { cmd_builder.args(["-NoProfile", "-Command", &command]); } else { cmd_builder.args(["-c", &command]); }
                                let child = pair.slave.spawn_command(cmd_builder).ok()?;
                                // Close the slave handle immediately – required for ConPTY.
                                drop(pair.slave);
                                let term = std::sync::Arc::new(std::sync::Mutex::new(vt100::Parser::new(pty_size.rows, pty_size.cols, 0)));
                                let term_reader = term.clone();
                                if let Ok(mut reader) = pair.master.try_clone_reader() {
                                    std::thread::spawn(move || {
                                        let mut buf = [0u8; 8192];
                                        loop {
                                            match reader.read(&mut buf) {
                                                Ok(n) if n > 0 => { if let Ok(mut p) = term_reader.lock() { p.process(&buf[..n]); } }
                                                _ => break,
                                            }
                                        }
                                    });
                                }
                                let mut pty_writer = pair.master.take_writer().ok()?;
                                crate::pane::conpty_preemptive_dsr_response(&mut *pty_writer);
                                Some(PopupPty { master: pair.master, writer: pty_writer, child, term })
                            });
                        
                        app.mode = Mode::PopupMode {
                            command: command.clone(),
                            output: String::new(),
                            process: None,
                            width,
                            height,
                            close_on_exit,
                            popup_pty: pty_result,
                        };
                    } else {
                        app.mode = Mode::PopupMode {
                            command: String::new(),
                            output: "Press 'q' or Escape to close\n".to_string(),
                            process: None,
                            width,
                            height,
                            close_on_exit: true,
                            popup_pty: None,
                        };
                    }
                }
                CtrlReq::ConfirmBefore(prompt, cmd) => {
                    let prompt_text = if prompt.is_empty() {
                        format!("Confirm: {} (y/n)?", cmd)
                    } else {
                        format!("{} (y/n)?", prompt)
                    };
                    app.mode = Mode::ConfirmMode {
                        prompt: prompt_text,
                        command: cmd,
                        input: String::new(),
                    };
                }
                CtrlReq::ResizePaneAbsolute(axis, size) => {
                    resize_pane_absolute(&mut app, &axis, size);
                }
                CtrlReq::ResizePanePercent(axis, pct) => {
                    // Convert percentage to absolute size based on current window dimensions
                    let area = app.last_window_area;
                    let total = if axis == "x" { area.width } else { area.height };
                    let abs_size = ((total as u32) * (pct as u32) / 100).max(1) as u16;
                    resize_pane_absolute(&mut app, &axis, abs_size);
                }
                CtrlReq::ShowOptionValue(resp, name) => {
                    let val = get_option_value(&app, &name);
                    let _ = resp.send(val);
                }
                CtrlReq::ShowWindowOptionValue(resp, name) => {
                    let val = get_window_option_value(&app, &name);
                    let _ = resp.send(val);
                }
                CtrlReq::ShowWindowOptions(resp) => {
                    let _ = resp.send(render_window_options(&app));
                }
                CtrlReq::ChooseBuffer(resp) => {
                    let mut output = String::new();
                    for (i, buf) in app.paste_buffers.iter().enumerate() {
                        let preview: String = buf.chars().take(50).collect();
                        let preview = preview.replace('\n', "\\n").replace('\r', "");
                        output.push_str(&format!("buffer{}: {} bytes: \"{}\"\n", i, buf.len(), preview));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ServerInfo(resp) => {
                    let info = format!(
                        "psmux {} (Windows)\npid: {}\nsession: {}\nwindows: {}\nuptime: {}s\nsocket: {}",
                        VERSION,
                        std::process::id(),
                        app.session_name,
                        app.windows.len(),
                        (chrono::Local::now() - app.created_at).num_seconds(),
                        {
                            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                            format!("{}\\.psmux\\{}.port", home, app.port_file_base())
                        }
                    );
                    let _ = resp.send(info);
                }
                CtrlReq::SendPrefix => {
                    // Send the prefix key to the active pane as if typed
                    let prefix = app.prefix_key;
                    let encoded: Vec<u8> = match prefix.0 {
                        crossterm::event::KeyCode::Char(c) if prefix.1.contains(crossterm::event::KeyModifiers::CONTROL) => {
                            vec![(c.to_ascii_lowercase() as u8) & 0x1F]
                        }
                        crossterm::event::KeyCode::Char(c) => format!("{}", c).into_bytes(),
                        _ => vec![],
                    };
                    if !encoded.is_empty() {
                        let win = &mut app.windows[app.active_idx];
                        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                            let _ = p.writer.write_all(&encoded);
                            let _ = p.writer.flush();
                        }
                    }
                }
                CtrlReq::PrevLayout => {
                    cycle_layout_reverse(&mut app);
                    state_dirty = true;
                }
                CtrlReq::FocusIn => {
                    if app.focus_events {
                        // Forward focus-in escape sequence to all panes in active window
                        let win = &mut app.windows[app.active_idx];
                        fn send_focus_seq(node: &mut Node, seq: &[u8]) {
                            match node {
                                Node::Leaf(p) => { let _ = p.writer.write_all(seq); let _ = p.writer.flush(); }
                                Node::Split { children, .. } => { for c in children { send_focus_seq(c, seq); } }
                            }
                        }
                        send_focus_seq(&mut win.root, b"\x1b[I");
                    }
                    hook_event = Some("pane-focus-in");
                }
                CtrlReq::FocusOut => {
                    if app.focus_events {
                        let win = &mut app.windows[app.active_idx];
                        fn send_focus_seq(node: &mut Node, seq: &[u8]) {
                            match node {
                                Node::Leaf(p) => { let _ = p.writer.write_all(seq); let _ = p.writer.flush(); }
                                Node::Split { children, .. } => { for c in children { send_focus_seq(c, seq); } }
                            }
                        }
                        send_focus_seq(&mut win.root, b"\x1b[O");
                    }
                    hook_event = Some("pane-focus-out");
                }
                CtrlReq::CommandPrompt(initial) => {
                    app.mode = Mode::CommandPrompt { input: initial.clone(), cursor: initial.len() };
                    state_dirty = true;
                }
                CtrlReq::ShowMessages(resp) => {
                    // Return message log (tmux stores recent log messages)
                    let _ = resp.send(String::new());
                }
                CtrlReq::ResizeWindow(_dim, _size) => {
                    // On Windows, window size is controlled by the terminal emulator;
                    // resize-window is a no-op since we adapt to the terminal size.
                }
                CtrlReq::RespawnWindow => {
                    // Kill all panes in the active window and respawn	
                    respawn_active_pane(&mut app, Some(&*pty_system))?;
                    state_dirty = true;
                }
            }
            // Log any active_idx change for debugging window-switch issues
            if app.active_idx != _prev_active_idx && crate::debug_log::server_log_enabled() {
                crate::debug_log::server_log("switch", &format!(
                    "active_idx changed {} -> {} by req={} hook={:?}",
                    _prev_active_idx, app.active_idx, _req_tag, hook_event));
            }
            // Fire any hooks registered for the event that just occurred
            if let Some(event) = hook_event {
                let _pre_hook_idx = app.active_idx;
                let cmds: Vec<String> = app.hooks.get(event).cloned().unwrap_or_default();
                for cmd in cmds {
                    parse_config_line(&mut app, &cmd);
                }
                // Check if the hook itself changed active_idx
                if app.active_idx != _pre_hook_idx && crate::debug_log::server_log_enabled() {
                    crate::debug_log::server_log("switch", &format!(
                        "active_idx changed {} -> {} by HOOK event={}",
                        _pre_hook_idx, app.active_idx, event));
                }
            }
            // Restore temporary -t focus after non-temp command completes
            if !is_temp_focus {
                if let Some((restore_idx, restore_path)) = temp_focus_restore.take() {
                    if restore_idx < app.windows.len() {
                        app.active_idx = restore_idx;
                        app.windows[restore_idx].active_path = restore_path;
                    }
                }
            }
            if mutates_state {
                state_dirty = true;
            }
        }
                // Clean up any trailing temp focus at end of batch
                if let Some((restore_idx, restore_path)) = temp_focus_restore {
                    if restore_idx < app.windows.len() {
                        app.active_idx = restore_idx;
                        app.windows[restore_idx].active_path = restore_path;
                    }
                }
            }
        }
        // ── Server-push: proactively send frames to attached clients ──
        // Instead of waiting for clients to poll dump-state, serialize
        // and push whenever state changed (PTY output, new window, key
        // echo, etc.).  This gives event-driven rendering like wezterm:
        // frames arrive within 1-5ms of ConPTY output instead of waiting
        // for the next client poll cycle (up to 50ms).
        if (state_dirty || meta_dirty) && crate::types::has_frame_receivers() {
            // Rebuild metadata cache if structural changes happened.
            if meta_dirty {
                cached_windows_json = list_windows_json_with_tabs(&app)?;
                cached_tree_json = list_tree_json(&app)?;
                cached_prefix_str = format_key_binding(&app.prefix_key);
                cached_prefix2_str = app.prefix2_key.as_ref().map(|k| format_key_binding(k)).unwrap_or_default();
                cached_base_index = app.window_base_index;
                cached_pred_dim = app.prediction_dimming;
                cached_status_style = app.status_style.clone();
                cached_bindings_json = serialize_bindings_json(&app);
                meta_dirty = false;
            }
            let layout_json = dump_layout_json_fast(&mut app)?;
            combined_buf.clear();
            let ss_escaped = json_escape_string(&cached_status_style);
            let sl_expanded = json_escape_string(&expand_format(&app.status_left, &app));
            let sr_expanded = json_escape_string(&expand_format(&app.status_right, &app));
            let pbs_escaped = json_escape_string(&app.pane_border_style);
            let pabs_escaped = json_escape_string(&app.pane_active_border_style);
            let wsf_escaped = json_escape_string(&app.window_status_format);
            let wscf_escaped = json_escape_string(&app.window_status_current_format);
            let wss_escaped = json_escape_string(&app.window_status_separator);
            let ws_style_escaped = json_escape_string(&app.window_status_style);
            let wsc_style_escaped = json_escape_string(&app.window_status_current_style);
            let mode_style_escaped = json_escape_string(&app.mode_style);
            let status_position_escaped = json_escape_string(&app.status_position);
            let status_justify_escaped = json_escape_string(&app.status_justify);
            let status_format_json = {
                let mut sf = String::from("[");
                for (i, fmt_str) in app.status_format.iter().enumerate() {
                    if i > 0 { sf.push(','); }
                    sf.push('"');
                    sf.push_str(&json_escape_string(&expand_format(fmt_str, &app)));
                    sf.push('"');
                }
                sf.push(']');
                sf
            };
            let cursor_style_code = crate::rendering::configured_cursor_code();
            let _ = std::fmt::Write::write_fmt(&mut combined_buf, format_args!(
                "{{\"layout\":{},\"windows\":{},\"prefix\":\"{}\",\"prefix2\":\"{}\",\"tree\":{},\"base_index\":{},\"prediction_dimming\":{},\"status_style\":\"{}\",\"status_left\":\"{}\",\"status_right\":\"{}\",\"pane_border_style\":\"{}\",\"pane_active_border_style\":\"{}\",\"wsf\":\"{}\",\"wscf\":\"{}\",\"wss\":\"{}\",\"ws_style\":\"{}\",\"wsc_style\":\"{}\",\"clock_mode\":{},\"bindings\":{},\"status_left_length\":{},\"status_right_length\":{},\"status_lines\":{},\"status_format\":{},\"mode_style\":\"{}\",\"status_position\":\"{}\",\"status_justify\":\"{}\",\"cursor_style_code\":{},\"status_visible\":{},\"repeat_time\":{}}}",
                layout_json, cached_windows_json, cached_prefix_str, cached_prefix2_str, cached_tree_json, cached_base_index, cached_pred_dim, ss_escaped, sl_expanded, sr_expanded, pbs_escaped, pabs_escaped, wsf_escaped, wscf_escaped, wss_escaped, ws_style_escaped, wsc_style_escaped,
                matches!(app.mode, Mode::ClockMode), cached_bindings_json,
                app.status_left_length, app.status_right_length, app.status_lines, status_format_json,
                mode_style_escaped, status_position_escaped, status_justify_escaped,
                cursor_style_code, app.status_visible, app.repeat_time_ms,
            ));
            // Inject clipboard data if pending
            if let Some(clip_text) = app.clipboard_osc52.take() {
                let clip_b64 = base64_encode(&clip_text);
                if combined_buf.ends_with('}') {
                    combined_buf.pop();
                    combined_buf.push_str(",\"clipboard_osc52\":\"");
                    combined_buf.push_str(&clip_b64);
                    combined_buf.push_str("\"}");
                }
            }
            cached_dump_state.clear();
            cached_dump_state.push_str(&combined_buf);
            cached_data_version = combined_data_version(&app);
            state_dirty = false;
            crate::types::push_frame(&combined_buf);
        }
        // ── Status-interval timer: fire hooks periodically ──
        if app.status_interval > 0 {
            let elapsed = app.last_status_interval_fire.elapsed().as_secs();
            if elapsed >= app.status_interval {
                app.last_status_interval_fire = std::time::Instant::now();
                let _pre_status_idx = app.active_idx;
                let cmds: Vec<String> = app.hooks.get("status-interval").cloned().unwrap_or_default();
                for cmd in cmds {
                    parse_config_line(&mut app, &cmd);
                }
                if app.active_idx != _pre_status_idx && crate::debug_log::server_log_enabled() {
                    crate::debug_log::server_log("switch", &format!(
                        "active_idx changed {} -> {} by status-interval hook",
                        _pre_status_idx, app.active_idx));
                }
            }
        }
        // Check if all windows/panes have exited (throttled to every 250ms)
        if last_reap.elapsed() >= Duration::from_millis(250) {
            last_reap = Instant::now();
            let (all_empty, any_pruned) = tree::reap_children(&mut app)?;
            if any_pruned {
                // A pane exited naturally - resize remaining panes to fill the space
                resize_all_panes(&mut app);
                state_dirty = true;
                meta_dirty = true;
            }
            if app.exit_empty && all_empty {
                // Kill warm pane's child (process::exit skips Drop)
                if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                let _ = std::fs::remove_file(&regpath);
                let _ = std::fs::remove_file(&keypath);
                crate::types::shutdown_persistent_streams();
                std::process::exit(0);
            }
        }
        // recv_timeout already handles the wait; no additional sleep needed.
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::should_spawn_warm_server;
    use crate::types::AppState;

    #[test]
    fn warm_server_is_disabled_for_destroy_unattached_sessions() {
        let mut app = AppState::new("demo".to_string());
        app.destroy_unattached = true;
        assert!(!should_spawn_warm_server(&app));
    }

    #[test]
    fn warm_server_is_disabled_for_warm_session_itself() {
        let app = AppState::new("__warm__".to_string());
        assert!(!should_spawn_warm_server(&app));
    }

    #[test]
    fn warm_server_is_allowed_for_normal_sessions() {
        let app = AppState::new("demo".to_string());
        assert!(should_spawn_warm_server(&app));
    }
}
