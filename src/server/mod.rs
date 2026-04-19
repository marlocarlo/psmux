pub(crate) mod helpers;
pub(crate) mod options;
pub(crate) mod option_catalog;
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
    WaitChannel, WaitForOp, Node, Action, Bind};
use crate::platform::install_console_ctrl_handler;
use crate::pane::{create_window, create_window_raw, split_active_with_command, kill_active_pane, kill_pane_by_id, spawn_warm_pane};
use crate::tree::{self, active_pane, active_pane_mut, resize_all_panes, kill_all_children,
    find_window_index_by_id, focus_pane_by_id, focus_pane_by_id_no_mru, focus_pane_by_index, get_active_pane_id,
    get_split_mut, path_exists};

use helpers::{collect_pane_paths_server, serialize_bindings_json, json_escape_string,
    list_windows_json_with_tabs, combined_data_version, TMUX_COMMANDS};
use options::{get_option_value, get_window_option_value, render_window_options, apply_set_option};

use crate::input::{send_text_to_active, send_key_to_active, send_paste_to_active, move_focus, find_best_pane_in_direction, find_wrap_target};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, move_copy_cursor, current_prompt_pos,
    yank_selection, scroll_copy_up, scroll_copy_down, switch_with_copy_save,
    capture_active_pane_text, capture_active_pane_range, capture_active_pane_styled};
use crate::layout::{dump_layout_json, dump_layout_json_fast, apply_layout, cycle_layout,
    cycle_layout_reverse};
use crate::window_ops::{toggle_zoom, remote_mouse_down, remote_mouse_drag, remote_mouse_up,
    remote_mouse_button, remote_mouse_motion, remote_scroll_up, remote_scroll_down,
    swap_pane, break_pane_to_window, unzoom_if_zoomed, resize_pane_vertical,
    resize_pane_horizontal, resize_pane_absolute, rotate_panes, respawn_active_pane,
    handle_pane_mouse, handle_pane_scroll, handle_split_set_sizes, handle_split_resize_done};
use crate::config::{load_config, parse_key_string, format_key_binding, normalize_key_for_binding,
    parse_config_content};
use crate::commands::{parse_command_to_action, format_action, parse_menu_definition, execute_command_string};
use crate::util::{list_windows_json, list_tree_json, list_windows_tmux, base64_encode};
use crate::control;
use crate::format::{expand_format, format_list_windows, format_list_panes, set_buffer_idx_override};
use crate::help;

/// Build a JSON fragment with overlay state (popup, menu, confirm, display_panes).
/// Delegates popup-specific serialization to the popup module.
fn serialize_overlay_json(app: &AppState) -> String {
    use crate::server::helpers::json_escape_string;

    // Popup overlay handles PopupMode, MenuMode, ConfirmMode, PaneChooser, and default
    let mut out = crate::popup::serialize_popup_overlay(app);

    // Include status_message for display-message without -p (#110)
    if let Some((ref msg, since, per_msg_duration)) = app.status_message {
        let elapsed = since.elapsed().as_millis() as u64;
        let display_time = per_msg_duration.unwrap_or(app.display_time_ms);
        if elapsed < display_time {
            out.push_str(",\"status_message\":\"");
            out.push_str(&json_escape_string(msg));
            out.push('"');
        }
    }
    out
}

fn should_spawn_warm_server(app: &AppState) -> bool {
    app.warm_enabled && app.session_name != "__warm__" && !app.destroy_unattached
}

/// Check if the active pane is currently squelched (hiding injected cd+cls).
/// Uses the non-consuming `squelch_cleared()` so the layout serialiser can
/// still properly consume the sentinel via `take_squelch_cleared()`.
fn is_active_pane_squelched(app: &AppState) -> bool {
    if app.windows.is_empty() { return false; }
    let win = &app.windows[app.active_idx];
    if let Some(p) = active_pane(&win.root, &win.active_path) {
        if let Some(deadline) = p.squelch_until {
            let sentinel = p.term.lock()
                .map(|parser| parser.screen().squelch_cleared())
                .unwrap_or(false);
            !sentinel && Instant::now() < deadline
        } else { false }
    } else { false }
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
        // Stale port file — remove it (and matching key file)
        let _ = std::fs::remove_file(&warm_port_path);
        let warm_key_path = format!("{}\\.psmux\\{}.key", home, warm_base);
        let _ = std::fs::remove_file(&warm_key_path);
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

/// Parse a popup dimension spec: "80" (absolute) or "95%" (percentage of term_dim).
fn parse_popup_dim(spec: &str, term_dim: u16, default: u16) -> u16 {
    if let Some(pct_str) = spec.strip_suffix('%') {
        if let Ok(pct) = pct_str.parse::<u16>() {
            let pct = pct.min(100);
            (term_dim as u32 * pct as u32 / 100) as u16
        } else {
            default
        }
    } else {
        spec.parse().unwrap_or(default)
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

/// Process a single CtrlReq during the post-config plugin drain loop.
/// Handles the subset of requests that plugin scripts send (set, show, bind,
/// source-file) and silently drops others.
fn drain_plugin_req(
    app: &mut AppState,
    req: CtrlReq,
    shared_aliases: &std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
) {
    match req {
        CtrlReq::SetOption(option, value) => {
            apply_set_option(app, &option, &value, false);
            app.user_set_options.insert(option.clone());
            if option == "command-alias" {
                if let Ok(mut map) = shared_aliases.write() {
                    *map = app.command_aliases.clone();
                }
            }
        }
        CtrlReq::SetOptionQuiet(option, value, quiet) => {
            apply_set_option(app, &option, &value, quiet);
            app.user_set_options.insert(option.clone());
            if option == "command-alias" {
                if let Ok(mut map) = shared_aliases.write() {
                    *map = app.command_aliases.clone();
                }
            }
        }
        CtrlReq::SetOptionAppend(option, value) => {
            if option.starts_with('@') {
                let existing = app.user_options.get(&option).cloned().unwrap_or_default();
                app.user_options.insert(option, format!("{}{}", existing, value));
            } else {
                match option.as_str() {
                    "status-left" => app.status_left.push_str(&value),
                    "status-right" => app.status_right.push_str(&value),
                    "status-style" => app.status_style.push_str(&value),
                    _ => {}
                }
            }
        }
        CtrlReq::SetOptionUnset(option) => {
            if option.starts_with('@') {
                app.user_options.remove(&option);
            }
        }
        CtrlReq::SetOptionOnlyIfUnset(option, value) => {
            // Only set if the option hasn't been explicitly set by user/config.
            // For @-prefixed user options, check if the key exists.
            // For built-in options, check the user_set_options tracker.
            let already_set = if option.starts_with('@') {
                app.user_options.contains_key(&option)
            } else {
                app.user_set_options.contains(&option)
            };
            if !already_set {
                apply_set_option(app, &option, &value, false);
                app.user_set_options.insert(option.clone());
                if option == "command-alias" {
                    if let Ok(mut map) = shared_aliases.write() {
                        *map = app.command_aliases.clone();
                    }
                }
            }
        }
        CtrlReq::ShowOptionValue(resp, name) => {
            let val = get_option_value(app, &name);
            let _ = resp.send(val);
        }
        CtrlReq::ShowWindowOptionValue(resp, name) => {
            let val = get_window_option_value(app, &name);
            let _ = resp.send(val);
        }
        CtrlReq::ShowOptions(resp) => {
            // Minimal: just send empty to unblock the caller
            let _ = resp.send(String::new());
        }
        CtrlReq::ShowWindowOptions(resp) => {
            let _ = resp.send(render_window_options(app));
        }
        CtrlReq::BindKey(table_name, key, command, repeat) => {
            if let Some(kc) = parse_key_string(&key) {
                let kc = normalize_key_for_binding(kc);
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
        }
        CtrlReq::SourceFile(path) => {
            app.defaults_suppressed = false;
            app.key_tables.clear();
            crate::config::populate_default_bindings(app);
            crate::config::source_file(app, &path);
        }
        CtrlReq::UnbindAll => {
            app.key_tables.clear();
            app.defaults_suppressed = true;
        }
        CtrlReq::UnbindAllInTable(table) => {
            if let Some(binds) = app.key_tables.get_mut(&table) {
                binds.clear();
            }
        }
        CtrlReq::UnbindKey(key, table) => {
            if let Some(kc) = parse_key_string(&key) {
                let kc = normalize_key_for_binding(kc);
                let target = table.unwrap_or_else(|| "prefix".to_string());
                if let Some(binds) = app.key_tables.get_mut(&target) {
                    binds.retain(|b| b.key != kc);
                }
            }
        }
        // Ignore other request types during plugin drain
        _ => {}
    }
}

pub fn run_server(session_name: String, socket_name: Option<String>, initial_command: Option<String>, raw_command: Option<Vec<String>>, start_dir: Option<String>, window_name: Option<String>, init_size: Option<(u16, u16)>, group_target: Option<String>, env_vars: Vec<(String, String)>) -> io::Result<()> {
    // Write crash info to a log file when stderr is unavailable (detached server)
    // and clean up port/key files so stale entries do not linger (issue #204).
    let panic_session_name = session_name.clone();
    let panic_socket_name = socket_name.clone();
    std::panic::set_hook(Box::new(move |info| {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let path = format!("{}\\.psmux\\crash.log", home);
        let bt = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::write(&path, format!("{info}\n\nBacktrace:\n{bt}"));
        // Remove port/key files to prevent stale entries after a panic
        let base = if let Some(ref sn) = panic_socket_name {
            format!("{}__{}", sn, panic_session_name)
        } else {
            panic_session_name.clone()
        };
        let _ = std::fs::remove_file(format!("{}\\.psmux\\{}.port", home, base));
        let _ = std::fs::remove_file(format!("{}\\.psmux\\{}.key", home, base));
    }));
    // Install console control handler to prevent termination on client detach
    install_console_ctrl_handler();

    let pty_system = native_pty_system();

    let mut app = AppState::new(session_name);
    app.socket_name = socket_name;
    app.session_group = group_target;
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

    app.session_key = session_key.clone();

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

    // Apply -e environment variables BEFORE pane spawn so the first pane
    // inherits them via apply_user_environment().
    crate::util::merge_session_env_into_app(&mut app, &env_vars);

    // Pre-spawn a warm pane BEFORE loading config: the shell (pwsh) starts
    // loading immediately and runs in parallel with config parsing / plugin
    // initialization.  By the time create_window() consumes it, the shell
    // has had the full config-load duration (~100-500ms) as a head start.
    // Only when using default shell (no custom command).
    // For detached sessions without -x/-y, last_window_area defaults to
    // 120x30 which is fine for the warm pane (resized later on first attach).
    let early_warm = if initial_command.is_none() && raw_command.is_none() && start_dir.is_none() {
        match spawn_warm_pane(&*pty_system, &mut app) {
            Ok(wp) => Some(wp),
            Err(_) => None,
        }
    } else { None };

    crate::config::populate_default_bindings(&mut app);
    load_config(&mut app);

    // Execute queued plugin .ps1 scripts (e.g. theme plugins that use
    // PowerShell variables and call back to psmux via CLI).  We spawn
    // them async and then drain the CtrlReq channel in a mini-loop so
    // show-options / set requests from the scripts are handled before
    // the main UI starts.
    if !app.pending_plugin_scripts.is_empty() {
        let scripts: Vec<String> = app.pending_plugin_scripts.drain(..).collect();
        let target_session = app.port_file_base();
        let mut children: Vec<std::process::Child> = Vec::new();
        for ps1 in &scripts {
            // Resolve shell: pwsh (PS7) preferred, fall back to powershell.exe (Windows PS)
            let shell = if which::which("pwsh").is_ok() { "pwsh" } else { "powershell" };
            let mut cmd = std::process::Command::new(shell);
            cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1]);
            if !target_session.is_empty() {
                cmd.env("PSMUX_TARGET_SESSION", &target_session);
            }
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            { use crate::platform::HideWindowCommandExt; cmd.hide_window(); }
            if let Ok(child) = cmd.spawn() {
                children.push(child);
            }
        }

        // Drain CtrlReq messages until all scripts finish (max 5s).
        if !children.is_empty() {
            let deadline = Instant::now() + Duration::from_secs(5);
            // Temporarily take rx out of app to avoid borrow conflict
            if let Some(rx) = app.control_rx.take() {
                loop {
                    let all_done = children.iter_mut().all(|c| {
                        matches!(c.try_wait(), Ok(Some(_)))
                    });
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if all_done || remaining.is_zero() {
                        while let Ok(req) = rx.try_recv() {
                            drain_plugin_req(&mut app, req, &shared_aliases_main);
                        }
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(50).min(remaining)) {
                        Ok(req) => drain_plugin_req(&mut app, req, &shared_aliases_main),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
                app.control_rx = Some(rx);
            }
        }
    }

    // If the user configured a custom default-shell in their config, the
    // early warm pane has the wrong shell — kill it so create_window falls
    // through to a cold spawn with the correct shell.
    if let Some(wp) = early_warm {
        if !app.warm_enabled {
            // Config disabled warm panes — kill the early pre-spawned pane.
            let mut wp = wp;
            wp.child.kill().ok();
        } else if app.default_shell.is_empty() {
            // No custom shell — the pre-spawned default (pwsh) is correct.
            // Check if config loaded env vars that the early warm pane is missing.
            let needs_env = app.environment.iter().any(|(k, _)| {
                !k.starts_with("PSMUX_TARGET_SESSION") && k != "TMUX" && k != "TMUX_PANE"
            });
            // The early warm pane was spawned before load_config with
            // allow_predictions=false (the default).  If the config set
            // allow-predictions on, the warm pane has the wrong PSReadLine
            // init string (PSRL_FIX instead of PSRL_CRASH_GUARD), so it
            // must be respawned (#165).
            let needs_predictions_fix = app.allow_predictions;
            if needs_env || needs_predictions_fix {
                // The early warm pane was spawned before config, so it lacks
                // config-defined env vars (e.g. TERM from default-terminal)
                // or has the wrong PSReadLine prediction init (#165).
                // Kill it and respawn with the correct settings.
                let mut wp = wp;
                wp.child.kill().ok();
                match spawn_warm_pane(&*pty_system, &mut app) {
                    Ok(new_wp) => { app.warm_pane = Some(new_wp); }
                    Err(e) => { eprintln!("psmux: warm pane respawn failed: {e}"); }
                }
            } else {
                app.warm_pane = Some(wp);
            }
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
    let create_result = if let Some(ref raw_args) = raw_command {
        create_window_raw(&*pty_system, &mut app, raw_args)
    } else {
        create_window(&*pty_system, &mut app, initial_command.as_deref(), None)
    };
    if let Err(e) = create_result {
        // Clean up port and key files so stale entries are not left
        // behind when the pane command fails to spawn (issue #204).
        let _ = std::fs::remove_file(&regpath);
        let _ = std::fs::remove_file(&keypath);
        // Kill warm pane if one was pre-spawned
        if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
        return Err(e);
    }
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    // Apply window name if specified via -n
    if let Some(n) = window_name { app.windows.last_mut().map(|w| w.name = n); }
    // Replenish: spawn a warm pane for the NEXT new-window / split.
    // Always replenish when no warm pane is available.
    if app.warm_pane.is_none() {
        match spawn_warm_pane(&*pty_system, &mut app) {
            Ok(wp) => { app.warm_pane = Some(wp); }
            Err(e) => { eprintln!("psmux: warm pane pre-spawn failed: {e}"); }
        }
    }
    // Fire client-attached hooks once at startup so plugins populate initial
    // data (e.g. CPU/battery) even for detached sessions (tppanel previews).
    crate::commands::fire_hooks(&mut app, "client-attached");
    // Fire session-created hook at startup
    crate::commands::fire_hooks(&mut app, "session-created");
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

    // Persist temp_focus_restore across batch boundaries so that a
    // FocusWindowTemp/FocusPaneByIndexTemp in one batch plus the actual
    // command (e.g. CapturePane) in the next batch still works correctly.
    let mut temp_focus_restore: Option<(usize, usize)> = None;

    loop {
        // Adaptive timeout: ramps from 1ms (active typing/echo) through
        // 5ms (client recently active) up to 50ms (fully idle).  This
        // dramatically reduces CPU usage when the session is idle while
        // keeping responsiveness high during interaction.
        let data_ready = crate::types::PTY_DATA_READY.swap(false, std::sync::atomic::Ordering::AcqRel);
        if data_ready {
            state_dirty = true;
            // Drain output ring buffers and send %output notifications to control clients
            if !app.control_clients.is_empty() {
                // Collect output from all panes first, then dispatch to clients
                let mut pane_outputs: Vec<(usize, String)> = Vec::new();
                for win in &app.windows {
                    crate::tree::for_each_pane(&win.root, &mut |pane: &crate::types::Pane| {
                        if let Ok(mut ring) = pane.output_ring.lock() {
                            if !ring.is_empty() {
                                let bytes: Vec<u8> = ring.drain(..).collect();
                                let data = String::from_utf8_lossy(&bytes).to_string();
                                pane_outputs.push((pane.id, data));
                            }
                        }
                    });
                }
                // Dispatch to each control client with pause-after logic
                let now = std::time::Instant::now();
                for (pane_id, data) in &pane_outputs {
                    for client in app.control_clients.values_mut() {
                        if client.paused_panes.contains(pane_id) {
                            continue;
                        }
                        if client.output_paused_panes.contains(pane_id) {
                            // Pane is paused for this client; drop output
                            continue;
                        }
                        if let Some(pause_secs) = client.pause_after_secs {
                            // Track output timing per pane
                            let last = client.pane_last_output.entry(*pane_id).or_insert(now);
                            let age = now.duration_since(*last);
                            *last = now;
                            if age.as_secs() >= pause_secs {
                                // Client fell behind: pause this pane
                                client.output_paused_panes.insert(*pane_id);
                                let _ = client.notification_tx.try_send(
                                    crate::types::ControlNotification::Pause { pane_id: *pane_id }
                                );
                                continue;
                            }
                            // Send as extended-output with age
                            let age_ms = age.as_millis() as u64;
                            let _ = client.notification_tx.try_send(
                                crate::types::ControlNotification::ExtendedOutput {
                                    pane_id: *pane_id,
                                    age_ms,
                                    data: data.clone(),
                                }
                            );
                        } else {
                            // No pause-after: send normal %output
                            let _ = client.notification_tx.try_send(
                                crate::types::ControlNotification::Output {
                                    pane_id: *pane_id,
                                    data: data.clone(),
                                }
                            );
                        }
                    }
                }
            }
        }
        // When a popup PTY is active, always push frames so interactive
        // content (e.g. fzf, shell prompts) updates in real-time.
        if matches!(app.mode, Mode::PopupMode { .. }) {
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
                // Track temporary -t focus: save (active_idx, pane_id) when
                // FocusWindowTemp/FocusPaneTemp is seen, restore after next
                // non-temp command so the user's view doesn't jump.
                // We store the pane ID (not path) because kill-pane
                // restructures the tree, invalidating saved paths (#71).
                // NOTE: temp_focus_restore lives outside the loop so it
                // persists across batch boundaries (prevents race where
                // FocusWindowTemp and the actual command land in different
                // batches).
                for req in pending {
                    let mutates_state = !matches!(&req,
                        CtrlReq::DumpState(..)
                        | CtrlReq::SendText(_)
                        | CtrlReq::SendKey(_)
                        | CtrlReq::SendPaste(_)
                    );
                    let is_temp_focus = matches!(&req,
                        CtrlReq::FocusWindowTemp(_) | CtrlReq::FocusWindowByNameTemp(_) | CtrlReq::FocusPaneTemp(_) | CtrlReq::FocusPaneByIndexTemp(_));
                    let mut hook_event: Option<&str> = None;
                    // Track active_idx changes for debugging window-switch issues
                    let _prev_active_idx = app.active_idx;
                    let _req_tag: &str = match &req {
                        CtrlReq::NextWindow => "NextWindow",
                        CtrlReq::PrevWindow => "PrevWindow",
                        CtrlReq::SelectWindow(_) => "SelectWindow",
                        CtrlReq::FocusWindow(_) => "FocusWindow",
                        CtrlReq::FocusWindowByName(_) => "FocusWindowByName",
                        CtrlReq::FocusWindowTemp(_) => "FocusWindowTemp",
                        CtrlReq::FocusWindowByNameTemp(_) => "FocusWindowByNameTemp",
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
                        CtrlReq::KillPaneById(_) => "KillPaneById",
                        CtrlReq::BreakPane => "BreakPane",
                        CtrlReq::JoinPane { .. } => "JoinPane",
                        CtrlReq::MovePane { .. } => "MovePane",
                        CtrlReq::PaneForwardExtract(..) => "PaneForwardExtract",
                        CtrlReq::PaneForwardInject { .. } => "PaneForwardInject",
                        CtrlReq::PaneForwardResize(..) => "PaneForwardResize",
                        CtrlReq::PaneForwardStatus(..) => "PaneForwardStatus",
                        CtrlReq::PaneForwardKill(..) => "PaneForwardKill",
                        CtrlReq::MoveWindow(..) => "MoveWindow",
                        CtrlReq::SwapWindow(_) => "SwapWindow",
                        _ => "",
                    };
                    match req {
                CtrlReq::NewWindow(cmd, name, detached, start_dir) => {
                    if let Some(cmds) = app.hooks.get("before-new-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    let prev_idx = app.active_idx;
                    // Expand format variables like #{pane_current_path} (#111)
                    let start_dir = start_dir.map(|d| expand_format(&d, &app)).filter(|d| !d.is_empty());
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    // Hide the warm pane when an explicit start dir is requested
                    // so create_window spawns a fresh shell in the correct CWD.
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = create_window(&*pty_system, &mut app, cmd.as_deref(), start_dir.as_deref()) {
                        eprintln!("psmux: new-window error: {e}");
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
                    if let Some(n) = name { app.windows.last_mut().map(|w| { w.name = n; w.manual_rename = true; }); }
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
                    if let Some(cmds) = app.hooks.get("before-new-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    let prev_idx = app.active_idx;
                    let start_dir = start_dir.map(|d| expand_format(&d, &app)).filter(|d| !d.is_empty());
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = create_window(&*pty_system, &mut app, cmd.as_deref(), start_dir.as_deref()) {
                        eprintln!("psmux: new-window error: {e}");
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
                    if let Some(n) = name { app.windows.last_mut().map(|w| { w.name = n; w.manual_rename = true; }); }
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
                CtrlReq::SplitWindow(k, cmd, detached, start_dir, split_size, resp) => {
                    if let Some(cmds) = app.hooks.get("before-split-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    // tmux: split-window without -Z permanently unzooms (#82)
                    unzoom_if_zoomed(&mut app);
                    let start_dir = start_dir.map(|d| expand_format(&d, &app)).filter(|d| !d.is_empty());
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    let prev_path = app.windows[app.active_idx].active_path.clone();
                    // Hide warm pane when explicit start_dir is given (wrong CWD)
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = split_active_with_command(&mut app, k, cmd.as_deref(), Some(&*pty_system), start_dir.as_deref()) {
                        let _ = resp.send(format!("psmux: split-window: {e}"));
                    } else {
                        let _ = resp.send(String::new());
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    // Apply size if specified: (value, true) = percentage, (value, false) = cell count
                    if let Some((val, is_pct)) = split_size {
                        let pct = if is_pct {
                            val.clamp(1, 99)
                        } else {
                            // Convert cell count to percentage based on split direction
                            let area = app.last_window_area;
                            let total = if k == LayoutKind::Horizontal { area.width } else { area.height };
                            if total > 0 { ((val as u32 * 100) / total as u32).clamp(1, 99) as u16 } else { 50 }
                        };
                        let win = &mut app.windows[app.active_idx];
                        if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &prev_path) {
                            sizes[0] = 100 - pct;
                            sizes[1] = pct;
                        }
                    }
                    if detached {
                        // Capture new pane ID before reverting focus
                        let new_pane_id = crate::tree::get_active_pane_id(
                            &app.windows[app.active_idx].root,
                            &app.windows[app.active_idx].active_path,
                        );
                        // Revert focus to the previously active pane.
                        // After split, prev_path now points to a Split node;
                        // the original pane is child [0] of that Split.
                        let mut revert_path = prev_path;
                        revert_path.push(0);
                        app.windows[app.active_idx].active_path = revert_path;
                        // Detached splits never focus the new pane — remove
                        // from MRU entirely so directional nav tie-breaks by
                        // pane_index among equally-unvisited candidates (#70).
                        if let Some(nid) = new_pane_id {
                            let win = &mut app.windows[app.active_idx];
                            win.pane_mru.retain(|&id| id != nid);
                        }
                    } else {
                        // Non-detached: new pane keeps focus.
                        // Cancel temp_focus_restore so -t doesn't revert (#112).
                        temp_focus_restore = None;
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
                CtrlReq::SplitWindowPrint(k, cmd, detached, start_dir, split_size, format_str, resp) => {
                    if let Some(cmds) = app.hooks.get("before-split-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    unzoom_if_zoomed(&mut app);
                    let start_dir = start_dir.map(|d| expand_format(&d, &app)).filter(|d| !d.is_empty());
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
                    let prev_path = app.windows[app.active_idx].active_path.clone();
                    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
                    if let Err(e) = split_active_with_command(&mut app, k, cmd.as_deref(), Some(&*pty_system), start_dir.as_deref()) {
                        eprintln!("psmux: split-window error: {e}");
                    }
                    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
                    // Apply size if specified: (value, true) = percentage, (value, false) = cell count
                    if let Some((val, is_pct)) = split_size {
                        let pct = if is_pct {
                            val.clamp(1, 99)
                        } else {
                            let area = app.last_window_area;
                            let total = if k == LayoutKind::Horizontal { area.width } else { area.height };
                            if total > 0 { ((val as u32 * 100) / total as u32).clamp(1, 99) as u16 } else { 50 }
                        };
                        let win = &mut app.windows[app.active_idx];
                        if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &prev_path) {
                            sizes[0] = 100 - pct;
                            sizes[1] = pct;
                        }
                    }
                    // Use full format engine for -P output (tmux compatible)
                    let fmt = format_str.as_deref().unwrap_or("#{session_name}:#{window_index}.#{pane_index}");
                    let pane_info = crate::format::expand_format_for_window(fmt, &app, app.active_idx);
                    if detached {
                        // Capture new pane ID before reverting focus
                        let new_pane_id = crate::tree::get_active_pane_id(
                            &app.windows[app.active_idx].root,
                            &app.windows[app.active_idx].active_path,
                        );
                        let mut revert_path = prev_path;
                        revert_path.push(0);
                        app.windows[app.active_idx].active_path = revert_path;
                        // Detached splits: remove from MRU (#70 pane_index tie-break)
                        if let Some(nid) = new_pane_id {
                            let win = &mut app.windows[app.active_idx];
                            win.pane_mru.retain(|&id| id != nid);
                        }
                    } else {
                        temp_focus_restore = None;
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
                CtrlReq::KillPane => {
                    if let Some(cmds) = app.hooks.get("before-kill-pane") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    unzoom_if_zoomed(&mut app); let _ = kill_active_pane(&mut app); resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-kill-pane");
                }
                CtrlReq::KillPaneById(pid) => {
                    if let Some(cmds) = app.hooks.get("before-kill-pane") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    unzoom_if_zoomed(&mut app); let _ = kill_pane_by_id(&mut app, pid); resize_all_panes(&mut app); meta_dirty = true; hook_event = Some("after-kill-pane");
                }
                CtrlReq::CapturePane(resp) => {
                    if is_active_pane_squelched(&app) {
                        let _ = resp.send(String::new());
                    } else if let Some(text) = capture_active_pane_text(&mut app)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneStyled(resp, s, e) => {
                    if is_active_pane_squelched(&app) {
                        let _ = resp.send(String::new());
                    } else if let Some(text) = capture_active_pane_styled(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneRange(resp, s, e) => {
                    if is_active_pane_squelched(&app) {
                        let _ = resp.send(String::new());
                    } else if let Some(text) = capture_active_pane_range(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
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
                            // Clear activity/bell/silence flags on the newly-focused window
                            if let Some(win) = app.windows.get_mut(internal_idx) {
                                win.activity_flag = false;
                                win.bell_flag = false;
                                win.silence_flag = false;
                            }
                            // Lazily resize panes in the newly-focused window
                            resize_all_panes(&mut app);
                        }
                    }
                    meta_dirty = true;
                }
                CtrlReq::FocusWindowByName(ref name) => {
                    if let Some(internal_idx) = app.windows.iter().position(|w| w.name == *name) {
                        if internal_idx != app.active_idx {
                            switch_with_copy_save(&mut app, |app| {
                                app.last_window_idx = app.active_idx;
                                app.active_idx = internal_idx;
                            });
                            if let Some(win) = app.windows.get_mut(internal_idx) {
                                win.activity_flag = false;
                                win.bell_flag = false;
                                win.silence_flag = false;
                            }
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
                    // Update MRU so directional navigation remembers this focus change
                    let win = &mut app.windows[app.active_idx];
                    if let Some(pid) = crate::tree::get_active_pane_id(&win.root, &win.active_path) {
                        crate::tree::touch_mru(&mut win.pane_mru, pid);
                    }
                    meta_dirty = true;
                }
                // ── Temporary focus variants for -t targeting ────────────
                // These switch active_idx/active_path so the NEXT command
                // in the batch operates on the correct window/pane.
                // After the entire pending batch is processed, we restore
                // the original focus (see temp_focus_restore below).
                CtrlReq::FocusWindowTemp(wid) => {
                    if temp_focus_restore.is_none() {
                        let pane_id = crate::tree::get_active_pane_id(
                            &app.windows[app.active_idx].root,
                            &app.windows[app.active_idx].active_path,
                        ).unwrap_or(usize::MAX);
                        temp_focus_restore = Some((app.active_idx, pane_id));
                    }
                    if wid >= app.window_base_index {
                        let internal_idx = wid - app.window_base_index;
                        if internal_idx < app.windows.len() {
                            app.active_idx = internal_idx;
                        }
                    }
                }
                CtrlReq::FocusWindowByNameTemp(ref name) => {
                    if temp_focus_restore.is_none() {
                        let pane_id = crate::tree::get_active_pane_id(
                            &app.windows[app.active_idx].root,
                            &app.windows[app.active_idx].active_path,
                        ).unwrap_or(usize::MAX);
                        temp_focus_restore = Some((app.active_idx, pane_id));
                    }
                    if let Some(internal_idx) = app.windows.iter().position(|w| w.name == *name) {
                        app.active_idx = internal_idx;
                    }
                }
                CtrlReq::FocusPaneTemp(pid) => {
                    if temp_focus_restore.is_none() {
                        let pane_id = crate::tree::get_active_pane_id(
                            &app.windows[app.active_idx].root,
                            &app.windows[app.active_idx].active_path,
                        ).unwrap_or(usize::MAX);
                        temp_focus_restore = Some((app.active_idx, pane_id));
                    }
                    // Use no-MRU variant: temporary -t targeting should not
                    // pollute the recency list (#71 — split-window -t was
                    // incorrectly touching the target pane's MRU rank).
                    focus_pane_by_id_no_mru(&mut app, pid);
                }
                CtrlReq::FocusPaneByIndexTemp(idx) => {
                    if temp_focus_restore.is_none() {
                        let pane_id = crate::tree::get_active_pane_id(
                            &app.windows[app.active_idx].root,
                            &app.windows[app.active_idx].active_path,
                        ).unwrap_or(usize::MAX);
                        temp_focus_restore = Some((app.active_idx, pane_id));
                    }
                    focus_pane_by_index(&mut app, idx);
                }
                CtrlReq::SessionInfo(resp) => {
                    let num_attached = app.client_registry.len();
                    let attached = if num_attached > 0 { " (attached)" } else { "" };
                    let group = if let Some(ref g) = app.session_group {
                        format!(" (group {})", g)
                    } else {
                        String::new()
                    };
                    let windows = app.windows.len();
                    let created = app.created_at.format("%a %b %e %H:%M:%S %Y");
                    let line = format!("{}: {} windows (created {}){}{}\n", app.session_name, windows, created, group, attached);
                    let _ = resp.send(line);
                }
                CtrlReq::ClientAttach(cid) => {
                    app.attached_clients = app.attached_clients.saturating_add(1);
                    app.latest_client_id = Some(cid);
                    // Register in client registry if not already present
                    app.client_registry.entry(cid).or_insert_with(|| {
                        let tty = format!("/dev/pts/{}", cid);
                        crate::types::ClientInfo {
                            id: cid,
                            width: app.last_window_area.width,
                            height: app.last_window_area.height,
                            connected_at: std::time::Instant::now(),
                            last_activity: std::time::Instant::now(),
                            tty_name: tty,
                            is_control: false,
                        }
                    });
                    hook_event = Some("client-attached");
                    // update-environment: refresh env vars from the attaching client's environment
                    let update_vars = app.update_environment.clone();
                    for var_spec in &update_vars {
                        let remove = var_spec.starts_with('-');
                        let name = if remove { &var_spec[1..] } else { var_spec.as_str() };
                        if remove {
                            app.environment.remove(name);
                        } else if let Ok(val) = std::env::var(name) {
                            app.environment.insert(name.to_string(), val);
                        } else {
                            app.environment.remove(name);
                        }
                    }
                }
                CtrlReq::ClientDetach(cid) => {
                    app.attached_clients = app.attached_clients.saturating_sub(1);
                    app.client_sizes.remove(&cid);
                    app.client_registry.remove(&cid);
                    app.client_prefix_active = false;
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
                        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                        let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                        let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                        let _ = std::fs::remove_file(&regpath);
                        let _ = std::fs::remove_file(&keypath);
                        crate::types::shutdown_persistent_streams();
                        tree::kill_all_children_batch(&mut app.windows);
                        if let Some(mut wp) = app.warm_pane.take() {
                            wp.child.kill().ok();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        std::process::exit(0);
                    }
                }
                CtrlReq::DumpLayout(resp) => {
                    let json = dump_layout_json(&mut app)?;
                    let _ = resp.send(json);
                }
                CtrlReq::DumpState(resp, allow_nc) => {
                    // ── Activity / bell / silence detection ──
                    let alert_hooks = helpers::check_window_activity(&mut app);
                    for event in &alert_hooks {
                        if let Some(cmds) = app.hooks.get(*event) { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    }

                    // ── Propagate OSC 0/2 titles to pane.title ──
                    if helpers::propagate_osc_titles(&mut app) {
                        state_dirty = true;
                    }

                    // ── Automatic rename / allow-rename: resolve window names ──
                    {
                        let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
                        let auto_rename = app.automatic_rename;
                        let allow_rename = app.allow_rename;
                        if (auto_rename || allow_rename) && !in_copy {
                            for win in app.windows.iter_mut() {
                                if win.manual_rename { continue; }
                                if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                                    if p.dead { continue; }
                                    if p.last_title_check.elapsed().as_millis() < 1000 { continue; }
                                    p.last_title_check = std::time::Instant::now();
                                    if p.child_pid.is_none() {
                                        p.child_pid = crate::platform::mouse_inject::get_child_pid(&*p.child);
                                    }
                                    let new_name = if auto_rename {
                                        // automatic-rename: use foreground process name
                                        if let Some(pid) = p.child_pid {
                                            match crate::platform::process_info::get_foreground_process_name(pid) {
                                                Some(name) => name,
                                                None => {
                                                    // No foreground child found.  Keep the current
                                                    // window name to avoid flashing to the shell
                                                    // name before a child process spawns (#229).
                                                    // Once a child appears, auto-rename will pick
                                                    // it up on the next tick.
                                                    continue;
                                                }
                                            }
                                        } else if allow_rename && !p.title.is_empty() {
                                            p.title.clone()
                                        } else {
                                            continue;
                                        }
                                    } else if allow_rename {
                                        // allow-rename only: use OSC title from child
                                        if let Ok(parser) = p.term.lock() {
                                            let title = parser.screen().title();
                                            if !title.is_empty() {
                                                title.to_string()
                                            } else {
                                                continue;
                                            }
                                        } else {
                                            continue;
                                        }
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
                    let has_squelch = app.windows.get(app.active_idx)
                        .and_then(|w| crate::tree::active_pane(&w.root, &w.active_path))
                        .map_or(false, |p| p.squelch_until.is_some());
                    if allow_nc
                        && !state_dirty
                        && !app.bell_forward
                        && !has_squelch
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
                    let pbhs_escaped = json_escape_string(&app.pane_border_hover_style);
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
                        "{{\"layout\":{},\"windows\":{},\"prefix\":\"{}\",\"prefix2\":\"{}\",\"tree\":{},\"base_index\":{},\"pane_base_index\":{},\"prediction_dimming\":{},\"status_style\":\"{}\",\"status_left\":\"{}\",\"status_right\":\"{}\",\"pane_border_style\":\"{}\",\"pane_active_border_style\":\"{}\",\"pane_border_hover_style\":\"{}\",\"wsf\":\"{}\",\"wscf\":\"{}\",\"wss\":\"{}\",\"ws_style\":\"{}\",\"wsc_style\":\"{}\",\"clock_mode\":{},\"bindings\":{},\"status_left_length\":{},\"status_right_length\":{},\"status_lines\":{},\"status_format\":{},\"mode_style\":\"{}\",\"status_position\":\"{}\",\"status_justify\":\"{}\",\"cursor_style_code\":{},\"status_visible\":{},\"repeat_time\":{},\"zoomed\":{},\"defaults_suppressed\":{},\"pwsh_mouse_selection\":{}}}",
                        layout_json, cached_windows_json, cached_prefix_str, cached_prefix2_str, cached_tree_json, cached_base_index, app.pane_base_index, cached_pred_dim, ss_escaped, sl_expanded, sr_expanded, pbs_escaped, pabs_escaped, pbhs_escaped, wsf_escaped, wscf_escaped, wss_escaped, ws_style_escaped, wsc_style_escaped,
                        matches!(app.mode, Mode::ClockMode), cached_bindings_json,
                        app.status_left_length, app.status_right_length, app.status_lines, status_format_json,
                        mode_style_escaped, status_position_escaped, status_justify_escaped,
                        cursor_style_code, app.status_visible, app.repeat_time_ms,
                        app.windows.get(app.active_idx).map_or(false, |w| w.zoom_saved.is_some()),
                        app.defaults_suppressed,
                        app.pwsh_mouse_selection,
                    ));
                    // Inject overlay state (popup, menu, confirm, display_panes)
                    {
                        // Inject clock_colour if set
                        if let Some(cc) = app.user_options.get("clock-mode-colour") {
                            if combined_buf.ends_with('}') {
                                combined_buf.pop();
                                combined_buf.push_str(",\"clock_colour\":\"");
                                combined_buf.push_str(&json_escape_string(cc));
                                combined_buf.push_str("\"}");
                            }
                        }
                        // Inject pane-border-status and pane-border-format
                        if let Some(pbs) = app.user_options.get("pane-border-status") {
                            if combined_buf.ends_with('}') {
                                combined_buf.pop();
                                combined_buf.push_str(",\"pane_border_status\":\"");
                                combined_buf.push_str(&json_escape_string(pbs));
                                combined_buf.push('"');
                                if let Some(pbf) = app.user_options.get("pane-border-format") {
                                    combined_buf.push_str(",\"pane_border_format\":\"");
                                    combined_buf.push_str(&json_escape_string(pbf));
                                    combined_buf.push('"');
                                }
                                combined_buf.push('}');
                            }
                        }
                        let overlay_json = serialize_overlay_json(&app);
                        if !overlay_json.is_empty() && combined_buf.ends_with('}') {
                            combined_buf.pop();
                            combined_buf.push_str(&overlay_json);
                            combined_buf.push('}');
                        }
                    }
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
                    // Forward audible bell to client terminal
                    if app.bell_forward {
                        app.bell_forward = false;
                        if combined_buf.ends_with('}') {
                            combined_buf.pop();
                            combined_buf.push_str(",\"bell\":true}");
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
                    // Push combined_buf (not cached_dump_state) so one-shot
                    // fields like bell and clipboard reach all clients.
                    // The cached copy omits them for NC dedup safety.
                    crate::types::push_frame(&combined_buf);
                    let _ = resp.send(combined_buf.clone());
                }
                CtrlReq::SendText(s) => { app.status_message = None; send_text_to_active(&mut app, &s)?; echo_pending_until = Some(Instant::now()); }
                CtrlReq::SendKey(k) => { app.status_message = None; send_key_to_active(&mut app, &k)?; echo_pending_until = Some(Instant::now()); }
                CtrlReq::SendPaste(s) => { send_paste_to_active(&mut app, &s)?; echo_pending_until = Some(Instant::now()); }
                CtrlReq::ZoomPane => { toggle_zoom(&mut app); state_dirty = true; meta_dirty = true; hook_event = Some("after-resize-pane"); }
                CtrlReq::PrefixBegin => { app.client_prefix_active = true; state_dirty = true; }
                CtrlReq::PrefixEnd => { app.client_prefix_active = false; state_dirty = true; }
                CtrlReq::CopyEnter => { enter_copy_mode(&mut app); hook_event = Some("pane-mode-changed"); }
                CtrlReq::CopyEnterPageUp => {
                    enter_copy_mode(&mut app);
                    let half = app.windows.get(app.active_idx)
                        .and_then(|w| active_pane(&w.root, &w.active_path))
                        .map(|p| p.last_rows as usize).unwrap_or(20);
                    scroll_copy_up(&mut app, half);
                    hook_event = Some("pane-mode-changed");
                }
                CtrlReq::ClockMode => { app.mode = Mode::ClockMode; state_dirty = true; hook_event = Some("pane-mode-changed"); }
                CtrlReq::CopyMove(dx, dy) => { move_copy_cursor(&mut app, dx, dy); }
                CtrlReq::CopyAnchor => { if let Some((r,c)) = current_prompt_pos(&mut app) { app.copy_anchor = Some((r,c)); app.copy_anchor_scroll_offset = app.copy_scroll_offset; app.copy_pos = Some((r,c)); } }
                CtrlReq::CopyYank => {
                    let _ = yank_selection(&mut app);
                    if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    exit_copy_mode(&mut app);
                    hook_event = Some("pane-mode-changed");
                }
                CtrlReq::CopyRectToggle => {
                    app.copy_selection_mode = match app.copy_selection_mode {
                        crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                        _ => crate::types::SelectionMode::Rect,
                    };
                }
                CtrlReq::ClientSize(cid, w, h) => { 
                    app.client_sizes.insert(cid, (w, h));
                    app.latest_client_id = Some(cid);
                    // Update registry with new size and activity timestamp
                    if let Some(info) = app.client_registry.get_mut(&cid) {
                        info.width = w;
                        info.height = h;
                        info.last_activity = std::time::Instant::now();
                    }
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
                    hook_event = Some("client-resized");
                }
                CtrlReq::FocusPaneCmd(pid) => {
                    let old_path = app.windows[app.active_idx].active_path.clone();
                    switch_with_copy_save(&mut app, |app| { focus_pane_by_id(app, pid); });
                    if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(&mut app); }
                    meta_dirty = true;
                }
                CtrlReq::FocusWindowCmd(wid) => { switch_with_copy_save(&mut app, |app| { if let Some(idx) = find_window_index_by_id(app, wid) { app.active_idx = idx; } }); resize_all_panes(&mut app); meta_dirty = true; }
                CtrlReq::MouseDown(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_down(&mut app, x, y); state_dirty = true; meta_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseDownRight(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(&mut app, x, y, 2, true); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseDownMiddle(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(&mut app, x, y, 1, true); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseDrag(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_drag(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseUp(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_up(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseUpRight(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(&mut app, x, y, 2, false); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseUpMiddle(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(&mut app, x, y, 1, false); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::MouseMove(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_motion(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::ScrollUp(cid, x, y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_scroll_up(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::ScrollDown(cid, x, y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_scroll_down(&mut app, x, y); state_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::PaneMouse(cid, pane_id, button, col, row, press) => { if app.mouse_enabled { app.latest_client_id = Some(cid); handle_pane_mouse(&mut app, pane_id, button, col, row, press); state_dirty = true; meta_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::PaneScroll(cid, pane_id, up) => { if app.mouse_enabled { app.latest_client_id = Some(cid); handle_pane_scroll(&mut app, pane_id, up); state_dirty = true; meta_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::SplitSetSizes(cid, path, sizes) => { if app.mouse_enabled { app.latest_client_id = Some(cid); handle_split_set_sizes(&mut app, &path, &sizes); state_dirty = true; meta_dirty = true; echo_pending_until = Some(Instant::now()); } }
                CtrlReq::SplitResizeDone(cid) => { if app.mouse_enabled { app.latest_client_id = Some(cid); handle_split_resize_done(&mut app); state_dirty = true; meta_dirty = true; } }
                CtrlReq::NextWindow => {
                    if let Some(cmds) = app.hooks.get("before-select-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    if !app.windows.is_empty() { switch_with_copy_save(&mut app, |app| { app.last_window_idx = app.active_idx; app.active_idx = (app.active_idx + 1) % app.windows.len(); }); resize_all_panes(&mut app); } meta_dirty = true; hook_event = Some("after-select-window");
                }
                CtrlReq::PrevWindow => {
                    if let Some(cmds) = app.hooks.get("before-select-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    if !app.windows.is_empty() { switch_with_copy_save(&mut app, |app| { app.last_window_idx = app.active_idx; app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); }); resize_all_panes(&mut app); } meta_dirty = true; hook_event = Some("after-select-window");
                }
                CtrlReq::RenameWindow(name) => {
                    if let Some(cmds) = app.hooks.get("before-rename-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    let win = &mut app.windows[app.active_idx]; win.name = name; win.manual_rename = true; meta_dirty = true; hook_event = Some("after-rename-window");
                }
                CtrlReq::ListWindows(resp) => { helpers::propagate_osc_titles(&mut app); let json = list_windows_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ListWindowsTmux(resp) => { helpers::propagate_osc_titles(&mut app); let text = list_windows_tmux(&app); let _ = resp.send(text); }
                CtrlReq::ListWindowsFormat(resp, fmt) => { helpers::propagate_osc_titles(&mut app); let text = format_list_windows(&app, &fmt); let _ = resp.send(text); }
                CtrlReq::ListTree(resp) => { let json = list_tree_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ToggleSync => { app.sync_input = !app.sync_input; }
                CtrlReq::SetPaneTitle(title) => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                        p.title_locked = !title.is_empty();
                        p.title = title;
                    }
                    meta_dirty = true;
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
                                        // On Windows, writing 0x03 to the PTY pipe doesn't
                                        // generate CTRL_C_EVENT when ENABLE_PROCESSED_INPUT
                                        // is disabled (e.g. after a TUI app).  Fire the real
                                        // signal via the platform helper so detached/headless
                                        // send-keys C-c reliably interrupts processes.
                                        #[cfg(windows)]
                                        if ctrl == 0x03 {
                                            if let Some(win) = app.windows.get_mut(app.active_idx) {
                                                if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                                                    if p.child_pid.is_none() {
                                                        p.child_pid = crate::platform::mouse_inject::get_child_pid(&*p.child);
                                                    }
                                                    if let Some(pid) = p.child_pid {
                                                        crate::platform::mouse_inject::send_ctrl_c_event(pid, false);
                                                    }
                                                }
                                            }
                                        }
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
                            if let Some(cmds) = app.hooks.get("pane-mode-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                        }
                        "copy-selection-and-cancel" => {
                            let _ = yank_selection(&mut app);
                            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                            app.mode = Mode::Passthrough;
                            app.copy_scroll_offset = 0;
                            app.copy_pos = None;
                            if let Some(cmds) = app.hooks.get("pane-mode-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                        }
                        "copy-selection-no-clear" => {
                            let _ = yank_selection(&mut app);
                            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                        }
                        s if s.starts_with("copy-pipe-and-cancel") || s.starts_with("copy-pipe") => {
                            // copy-pipe[-and-cancel] [command] — yank + pipe to command
                            let _ = yank_selection(&mut app);
                            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                            // Extract pipe command from argument if present
                            let cancel = s.contains("cancel");
                            let pipe_cmd = cmd.strip_prefix("copy-pipe-and-cancel")
                                .or_else(|| cmd.strip_prefix("copy-pipe"))
                                .unwrap_or("")
                                .trim();
                            if !pipe_cmd.is_empty() {
                                if let Some(text) = app.paste_buffers.first().cloned() {
                                    // Pipe yanked text to the command's stdin
                                    let mut copy_pipe_cmd = std::process::Command::new(if cfg!(windows) { "pwsh" } else { "sh" });
                                    copy_pipe_cmd.args(if cfg!(windows) { vec!["-NoProfile", "-Command", pipe_cmd] } else { vec!["-c", pipe_cmd] })
                                        .stdin(std::process::Stdio::piped())
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null());
                                    { use crate::platform::HideWindowCommandExt; copy_pipe_cmd.hide_window(); }
                                    if let Ok(mut child) = copy_pipe_cmd.spawn() {
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
                                if let Some(cmds) = app.hooks.get("pane-mode-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                            if app.paste_buffers.len() >= 2 {
                                let appended = format!("{}{}", app.paste_buffers[1], app.paste_buffers[0]);
                                app.paste_buffers[0] = appended;
                            }
                        }
                        "append-selection-and-cancel" => {
                            let _ = yank_selection(&mut app);
                            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                            if app.paste_buffers.len() >= 2 {
                                let appended = format!("{}{}", app.paste_buffers[1], app.paste_buffers[0]);
                                app.paste_buffers[0] = appended;
                            }
                            app.mode = Mode::Passthrough;
                            app.copy_scroll_offset = 0;
                            app.copy_pos = None;
                            if let Some(cmds) = app.hooks.get("pane-mode-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                                if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                            }
                            app.mode = Mode::Passthrough;
                            app.copy_scroll_offset = 0;
                            app.copy_pos = None;
                            if let Some(cmds) = app.hooks.get("pane-mode-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                    if let Some(cmds) = app.hooks.get("before-select-pane") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                            if was_zoomed {
                                // Zoom-aware: check direct neighbor or wrap target (tmux parity: unzoom+wrap).
                                let win = &app.windows[app.active_idx];
                                let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
                                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                                let active_idx = rects.iter().position(|(path, _)| *path == win.active_path);
                                let has_target = if let Some(ai) = active_idx {
                                    let (_, arect) = &rects[ai];
                                    find_best_pane_in_direction(&rects, ai, arect, focus_dir, &[], &[])
                                        .or_else(|| find_wrap_target(&rects, ai, arect, focus_dir, &[], &[]))
                                        .is_some()
                                } else { false };
                                if has_target {
                                    let old_path = app.windows[app.active_idx].active_path.clone();
                                    switch_with_copy_save(&mut app, |app| {
                                        move_focus(app, focus_dir);
                                    });
                                    app.last_pane_path = old_path;
                                } else {
                                    // No reachable pane (single-pane window) — re-zoom
                                    toggle_zoom(&mut app);
                                }
                            } else {
                                let old_path = app.windows[app.active_idx].active_path.clone();
                                switch_with_copy_save(&mut app, |app| {
                                    move_focus(app, focus_dir);
                                });
                                if app.windows[app.active_idx].active_path != old_path {
                                    app.last_pane_path = old_path;
                                }
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
                                // Update MRU for the newly focused pane
                                let win = &mut app.windows[app.active_idx];
                                if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                                    crate::tree::touch_mru(&mut win.pane_mru, pid);
                                }
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
                                let win = &mut app.windows[app.active_idx];
                                if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                                    crate::tree::touch_mru(&mut win.pane_mru, pid);
                                }
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
                                let win = &mut app.windows[app.active_idx];
                                if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                                    crate::tree::touch_mru(&mut win.pane_mru, pid);
                                }
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
                    if let Some(cmds) = app.hooks.get("before-select-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                    helpers::propagate_osc_titles(&mut app);
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
                    helpers::propagate_osc_titles(&mut app);
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
                    // Fire session-closed hook before cleanup
                    if let Some(cmds) = app.hooks.get("session-closed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    // Remove port/key files FIRST so clients see the session
                    // as gone immediately, then kill processes.
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                    let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                    let _ = std::fs::remove_file(&regpath);
                    let _ = std::fs::remove_file(&keypath);
                    crate::types::shutdown_persistent_streams();
                    // Kill all child processes using a single process snapshot
                    tree::kill_all_children_batch(&mut app.windows);
                    // Kill warm pane's child (process::exit skips Drop)
                    if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                    // TerminateProcess is synchronous on Windows — processes
                    // are already dead.  Minimal delay for OS handle cleanup.
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    std::process::exit(0);
                }
                CtrlReq::HasSession(resp) => {
                    let _ = resp.send(true);
                }
                CtrlReq::RenameSession(name) => {
                    if let Some(cmds) = app.hooks.get("before-rename-session") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
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
                    // Update env so run-shell/hooks from this server target the new name
                    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());
                    hook_event = Some("after-rename-session");
                }
                CtrlReq::ClaimSession(name, client_cwd, resp) => {
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
                    // Update env so run-shell/hooks from this server target the new name
                    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());
                    // Honour the client's working directory: the warm server
                    // was spawned from a previous session whose CWD may differ
                    // from where the user ran `psmux` now.  Update the
                    // server's CWD (for future pane spawns) and silently
                    // inject `cd` into the active pane so the shell starts
                    // in the right directory.  A clear screen command is
                    // chained after cd so the user never sees the injected
                    // command or its echo.
                    if let Some(ref cwd) = client_cwd {
                        let cwd_path = std::path::Path::new(cwd);
                        if cwd_path.is_dir() {
                            let server_cwd_differs = env::current_dir()
                                .map(|cur| cur != cwd_path)
                                .unwrap_or(true);
                            if server_cwd_differs {
                                env::set_current_dir(cwd_path).ok();
                                // Inject cd + clear into the active pane so
                                // the directory change is invisible to the
                                // user.  Leading space keeps it out of shell
                                // history; the clear wipes visible traces.
                                //
                                // The vt100 parser watches for the CSI 2J
                                // that cls/clear generates, which tells the
                                // layout serialiser the clear finished
                                // (event-driven, no guessing).  A safety
                                // timeout is a fallback for unusual shells.
                                if let Some(win) = app.windows.last_mut() {
                                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                                        use std::io::Write as _;
                                        let escaped = cwd.replace('\'', "''");
                                        let clear = if cfg!(windows) { "cls" } else { "clear" };
                                        let cd_cmd = format!(" cd '{}'; {}\r", escaped, clear);
                                        // Tell the vt100 parser to watch for the
                                        // next screen-clear event (CSI 2J/3J).
                                        if let Ok(mut parser) = p.term.lock() {
                                            parser.screen_mut().set_squelch_clear_pending(true);
                                        }
                                        p.squelch_until = Some(Instant::now() + Duration::from_millis(500));
                                        let _ = p.writer.write_all(cd_cmd.as_bytes());
                                        let _ = p.writer.flush();
                                    }
                                }
                            }
                        }
                    }
                    // Update env so run-shell/hooks from this server target the new name
                    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());
                    // Re-load user config so the claimed session reflects the
                    // current config file.  The warm server loaded config at
                    // its own startup, but the user may have changed their
                    // config since then (or the warm server was spawned by a
                    // different session with a different PSMUX_CONFIG_FILE).
                    app.key_tables.clear();
                    app.defaults_suppressed = false;
                    crate::config::populate_default_bindings(&mut app);
                    load_config(&mut app);
                    // Update shared aliases after config reload
                    if let Ok(mut w) = shared_aliases_main.write() {
                        *w = app.command_aliases.clone();
                    }
                    // Fire client-session-changed hook (warm server claimed by new session)
                    if let Some(cmds) = app.hooks.get("client-session-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(&mut app, cmd); } }
                    meta_dirty = true;
                    state_dirty = true;
                    let _ = resp.send("OK\n".to_string());
                    // Spawn a replacement warm server for the NEXT new-session
                    spawn_warm_server(&app);
                    hook_event = Some("after-rename-session");
                }
                CtrlReq::SwapPane(dir) => {
                    // tmux: swap-pane without -Z permanently unzooms (#82)
                    unzoom_if_zoomed(&mut app);
                    match dir.as_str() {
                        "U" => { swap_pane(&mut app, FocusDir::Up); }
                        "D" => { swap_pane(&mut app, FocusDir::Down); }
                        _ => { swap_pane(&mut app, FocusDir::Down); }
                    }
                    hook_event = Some("after-swap-pane");
                }
                CtrlReq::ResizePane(dir, amount) => {
                    unzoom_if_zoomed(&mut app);
                    match dir.as_str() {
                        "U" | "D" => { resize_pane_vertical(&mut app, if dir == "U" { -(amount as i16) } else { amount as i16 }); }
                        "L" | "R" => { resize_pane_horizontal(&mut app, if dir == "L" { -(amount as i16) } else { amount as i16 }); }
                        _ => {}
                    }
                    resize_all_panes(&mut app); meta_dirty = true;
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
                CtrlReq::DeleteBufferAt(idx) => {
                    if idx < app.paste_buffers.len() { app.paste_buffers.remove(idx); }
                }
                CtrlReq::PasteBufferAt(idx) => {
                    if idx < app.paste_buffers.len() {
                        let text = app.paste_buffers[idx].clone();
                        let win = &mut app.windows[app.active_idx];
                        if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                            let _ = write!(p.writer, "{}", text);
                        }
                    }
                }
                CtrlReq::DisplayMessage(resp, fmt, target_pane_idx, set_status_bar, duration_ms) => {
                    // Propagate OSC titles so #{pane_title} reflects latest state
                    helpers::propagate_osc_titles(&mut app);
                    let result = if let Some(pane_idx) = target_pane_idx {
                        // -t targeting: evaluate format for the specific pane
                        // using PANE_POS_OVERRIDE so #{pane_active} reflects
                        // the REAL active pane, not the target (#113)
                        crate::format::expand_format_for_pane(&fmt, &app, app.active_idx, pane_idx)
                    } else {
                        expand_format(&fmt, &app)
                    };
                    if set_status_bar {
                        app.status_message = Some((result.clone(), Instant::now(), duration_ms));
                        state_dirty = true;
                    }
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
                CtrlReq::DisplayPaneSelect(digit) => {
                    // User pressed a digit during display-panes overlay: select the matching pane
                    let win = &app.windows[app.active_idx];
                    let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
                    crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                    for (i, (path, _)) in rects.iter().enumerate() {
                        if i >= 10 { break; }
                        let mapped = (i + app.pane_base_index) % 10;
                        if mapped == digit {
                            let new_path = path.clone();
                            let old_path = app.windows[app.active_idx].active_path.clone();
                            app.windows[app.active_idx].active_path = new_path;
                            if app.windows[app.active_idx].active_path != old_path {
                                app.last_pane_path = old_path;
                            }
                            break;
                        }
                    }
                    app.mode = Mode::Passthrough;
                    state_dirty = true;
                    meta_dirty = true;
                }
                CtrlReq::BreakPane => {
                    unzoom_if_zoomed(&mut app);
                    break_pane_to_window(&mut app);
                    hook_event = Some("after-break-pane");
                    meta_dirty = true;
                }
                CtrlReq::JoinPane { src_win, src_pane, target_win, target_pane, horizontal }
                | CtrlReq::MovePane { src_win, src_pane, target_win, target_pane, horizontal } => {
                    unzoom_if_zoomed(&mut app);
                    // Resolve source window index (default: active window)
                    let src_idx = src_win.unwrap_or(app.active_idx);
                    // Resolve target window index (default: active window, but must differ from source)
                    let raw_target_win = target_win.unwrap_or(app.active_idx);
                    if src_idx < app.windows.len() && raw_target_win < app.windows.len() && src_idx != raw_target_win {
                        // Resolve source pane path within source window
                        let src_path = if let Some(pidx) = src_pane {
                            // Get Nth pane path in DFS order
                            let mut leaves = Vec::new();
                            tree::collect_leaf_paths_pub(&app.windows[src_idx].root, &mut Vec::new(), &mut leaves);
                            if let Some((_, p)) = leaves.get(pidx) {
                                p.clone()
                            } else {
                                app.windows[src_idx].active_path.clone()
                            }
                        } else {
                            app.windows[src_idx].active_path.clone()
                        };
                        // Unzoom source window if needed
                        if let Some(saved) = app.windows[src_idx].zoom_saved.take() {
                            let win = &mut app.windows[src_idx];
                            for (p, sz) in saved.into_iter() {
                                if let Some(Node::Split { sizes, .. }) = crate::tree::get_split_mut(&mut win.root, &p) { *sizes = sz; }
                            }
                        }
                        let src_root = std::mem::replace(&mut app.windows[src_idx].root,
                            Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
                        let (remaining, extracted) = tree::extract_node(src_root, &src_path);
                        if let Some(pane_node) = extracted {
                            let src_empty = remaining.is_none();
                            if let Some(rem) = remaining {
                                app.windows[src_idx].root = rem;
                                app.windows[src_idx].active_path = tree::first_leaf_path(&app.windows[src_idx].root);
                            }
                            // Adjust target index if source window will be removed and target is after it
                            let tgt = if src_empty && raw_target_win > src_idx { raw_target_win - 1 } else { raw_target_win };
                            if src_empty {
                                app.windows.remove(src_idx);
                                if app.active_idx >= app.windows.len() {
                                    app.active_idx = app.windows.len().saturating_sub(1);
                                }
                            }
                            // Graft pane into target window
                            if tgt < app.windows.len() {
                                // Resolve target pane path
                                let tgt_path = if let Some(tpidx) = target_pane {
                                    let mut leaves = Vec::new();
                                    tree::collect_leaf_paths_pub(&app.windows[tgt].root, &mut Vec::new(), &mut leaves);
                                    if let Some((_, p)) = leaves.get(tpidx) {
                                        p.clone()
                                    } else {
                                        app.windows[tgt].active_path.clone()
                                    }
                                } else {
                                    app.windows[tgt].active_path.clone()
                                };
                                let split_kind = if horizontal { LayoutKind::Horizontal } else { LayoutKind::Vertical };
                                tree::replace_leaf_with_split(&mut app.windows[tgt].root, &tgt_path, split_kind, pane_node);
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
                // ── Cross-session pane forwarding ───────────────────────
                CtrlReq::PaneForwardExtract(win_idx, pane_idx, resp) => {
                    crate::cross_session_server::handle_pane_forward_extract(&mut app, win_idx, pane_idx, resp);
                    resize_all_panes(&mut app);
                    meta_dirty = true;
                }
                CtrlReq::PaneForwardInject {
                    source_session, source_addr, source_key,
                    forward_id, fwd_port, pid, title, rows, cols,
                    screen_b64, target_win, target_pane, horizontal,
                } => {
                    crate::cross_session_server::handle_pane_forward_inject(
                        &mut app, source_session, source_addr, source_key,
                        forward_id, fwd_port, pid, title, rows, cols,
                        screen_b64, target_win, target_pane, horizontal,
                    );
                    resize_all_panes(&mut app);
                    meta_dirty = true;
                    hook_event = Some("after-join-pane");
                }
                CtrlReq::PaneForwardResize(fwd_id, fwd_rows, fwd_cols) => {
                    if let Some(fp) = app.forwarded_panes.get(&fwd_id) {
                        let _ = fp.master.resize(portable_pty::PtySize {
                            rows: fwd_rows, cols: fwd_cols, pixel_width: 0, pixel_height: 0,
                        });
                    }
                }
                CtrlReq::PaneForwardStatus(fwd_id, resp) => {
                    let status = if let Some(fp) = app.forwarded_panes.get_mut(&fwd_id) {
                        match fp.child.try_wait() {
                            Ok(Some(_)) => "exited".to_string(),
                            Ok(None) => "running".to_string(),
                            Err(_) => "exited".to_string(),
                        }
                    } else {
                        "exited".to_string()
                    };
                    let _ = resp.send(status);
                }
                CtrlReq::PaneForwardKill(fwd_id) => {
                    if let Some(mut fp) = app.forwarded_panes.remove(&fwd_id) {
                        fp.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = fp.child.kill();
                    }
                }
                CtrlReq::RespawnPane(workdir, kill) => {
                    respawn_active_pane(&mut app, Some(&*pty_system), workdir.as_deref(), kill)?;
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
                CtrlReq::UnbindKey(key, table) => {
                    if let Some(kc) = parse_key_string(&key) {
                        let kc = normalize_key_for_binding(kc);
                        let target = table.unwrap_or_else(|| "prefix".to_string());
                        if let Some(binds) = app.key_tables.get_mut(&target) {
                            binds.retain(|b| b.key != kc);
                        }
                    }
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::UnbindAll => {
                    app.key_tables.clear();
                    app.defaults_suppressed = true;
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::UnbindAllInTable(table) => {
                    if let Some(binds) = app.key_tables.get_mut(&table) {
                        binds.clear();
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
                    let output = help::build_list_keys_output(user_iter, app.defaults_suppressed);
                    let _ = resp.send(output);
                }
                CtrlReq::SetOption(option, value) => {
                    apply_set_option(&mut app, &option, &value, false);
                    app.user_set_options.insert(option.clone());
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
                    let old_shell = app.default_shell.clone();
                    apply_set_option(&mut app, &option, &value, quiet);
                    app.user_set_options.insert(option.clone());
                    // If default-shell changed, kill the warm pane so the next
                    // new-window spawns the correct shell (fixes #99).
                    if app.default_shell != old_shell {
                        if let Some(mut wp) = app.warm_pane.take() {
                            wp.child.kill().ok();
                        }
                    }
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
                        app.user_options.remove(&option);
                    } else {
                        match option.as_str() {
                            "status-left" => { app.status_left = "psmux:#I".to_string(); }
                            "status-right" => { app.status_right = "#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y".to_string(); }
                            "mouse" => { app.mouse_enabled = true; }
                            "scroll-enter-copy-mode" => { app.scroll_enter_copy_mode = true; }
                            "pwsh-mouse-selection" => { app.pwsh_mouse_selection = false; }
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
                            "pane-border-hover-style" => { app.pane_border_hover_style = "fg=yellow".to_string(); }
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
                        let existing = app.user_options.get(&option).cloned().unwrap_or_default();
                        app.user_options.insert(option, format!("{}{}", existing, value));
                    } else {
                        match option.as_str() {
                            "status-left" => { app.status_left.push_str(&value); }
                            "status-right" => { app.status_right.push_str(&value); }
                            "status-style" => { app.status_style.push_str(&value); }
                            "pane-border-style" => { app.pane_border_style.push_str(&value); }
                            "pane-active-border-style" => { app.pane_active_border_style.push_str(&value); }
                            "pane-border-hover-style" => { app.pane_border_hover_style.push_str(&value); }
                            "window-status-format" => { app.window_status_format.push_str(&value); }
                            "window-status-current-format" => { app.window_status_current_format.push_str(&value); }
                            _ => {}
                        }
                    }
                }
                CtrlReq::SetOptionOnlyIfUnset(option, value) => {
                    let already_set = if option.starts_with('@') {
                        app.user_options.contains_key(&option)
                    } else {
                        app.user_set_options.contains(&option)
                    };
                    if !already_set {
                        apply_set_option(&mut app, &option, &value, false);
                        app.user_set_options.insert(option.clone());
                        if option == "command-alias" {
                            if let Ok(mut map) = shared_aliases_main.write() {
                                *map = app.command_aliases.clone();
                            }
                        }
                        meta_dirty = true;
                        state_dirty = true;
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
                    output.push_str(&format!("scroll-enter-copy-mode {}\n", if app.scroll_enter_copy_mode { "on" } else { "off" }));
                    output.push_str(&format!("pwsh-mouse-selection {}\n", if app.pwsh_mouse_selection { "on" } else { "off" }));
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
                    output.push_str(&format!("allow-predictions {}\n", if app.allow_predictions { "on" } else { "off" }));
                    output.push_str(&format!("cursor-style {}\n", std::env::var("PSMUX_CURSOR_STYLE").unwrap_or_else(|_| "bar".to_string())));
                    output.push_str(&format!("cursor-blink {}\n", if std::env::var("PSMUX_CURSOR_BLINK").unwrap_or_else(|_| "1".to_string()) != "0" { "on" } else { "off" }));
                    {
                        let shell_val = if app.default_shell.is_empty() {
                            crate::pane::cached_shell().unwrap_or("pwsh.exe").to_string()
                        } else {
                            app.default_shell.clone()
                        };
                        output.push_str(&format!("default-shell {}\n", shell_val));
                    }
                    output.push_str(&format!("word-separators \"{}\"\n", app.word_separators));
                    if !app.pane_border_style.is_empty() {
                        output.push_str(&format!("pane-border-style \"{}\"\n", app.pane_border_style));
                    }
                    if !app.pane_active_border_style.is_empty() {
                        output.push_str(&format!("pane-active-border-style \"{}\"\n", app.pane_active_border_style));
                    }
                    if !app.pane_border_hover_style.is_empty() {
                        output.push_str(&format!("pane-border-hover-style \"{}\"\n", app.pane_border_hover_style));
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
                    for (key, val) in &app.user_options {
                        output.push_str(&format!("{} \"{}\"\n", key, val));
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
                    output.push_str(&format!("allow-rename {}\n", if app.allow_rename { "on" } else { "off" }));
                    output.push_str(&format!("allow-set-title {}\n", if app.allow_set_title { "on" } else { "off" }));
                    output.push_str(&format!("bell-action {}\n", app.bell_action));
                    output.push_str(&format!("activity-action {}\n", app.activity_action));
                    output.push_str(&format!("silence-action {}\n", app.silence_action));
                    output.push_str(&format!("update-environment \"{}\"\n", app.update_environment.join(" ")));
                    if let Some(ref group) = app.session_group {
                        output.push_str(&format!("session-group \"{}\"\n", group));
                    }
                    for (alias, expansion) in &app.command_aliases {
                        output.push_str(&format!("command-alias \"{}={}\"\n", alias, expansion));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::SourceFile(path) => {
                    // Reset binding state so config reload gets a clean slate.
                    // If the config has unbind-key -a, it will re-set the flag.
                    app.defaults_suppressed = false;
                    app.key_tables.clear();
                    crate::config::populate_default_bindings(&mut app);
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
                    // Mark dirty so the client receives updated config
                    // (status bar, bindings, styles, etc.) on the next
                    // dump-state instead of getting an NC fast-path reply.
                    state_dirty = true;
                    meta_dirty = true;
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
                CtrlReq::LinkWindow(src_idx_opt, dst_idx_opt) => {
                    // link-window: within a single session, create a linked window
                    // referencing the source window. Since PTY handles can't be shared
                    // across windows, this spawns a new shell and marks it as linked.
                    let src = src_idx_opt.unwrap_or(app.active_idx);
                    if src < app.windows.len() {
                        let src_id = app.windows[src].id;
                        let src_name = app.windows[src].name.clone();
                        let dst = dst_idx_opt.unwrap_or(app.windows.len());
                        let pty_system = portable_pty::native_pty_system();
                        match crate::pane::create_window(&*pty_system, &mut app, None, None) {
                            Ok(()) => {
                                let new_idx = app.windows.len() - 1;
                                app.windows[new_idx].linked_from = Some(src_id);
                                app.windows[new_idx].name = src_name;
                                if dst < new_idx {
                                    let win = app.windows.remove(new_idx);
                                    app.windows.insert(dst, win);
                                    if app.active_idx > dst && app.active_idx <= new_idx {
                                        app.active_idx = app.active_idx.saturating_sub(1);
                                    }
                                }
                                resize_all_panes(&mut app);
                                meta_dirty = true;
                                hook_event = Some("window-linked");
                            }
                            Err(_e) => {
                                app.status_message = Some(("link-window: failed to create linked window".to_string(), std::time::Instant::now(), None));
                            }
                        }
                    } else {
                        app.status_message = Some(("link-window: source window not found".to_string(), std::time::Instant::now(), None));
                    }
                    state_dirty = true;
                }
                CtrlReq::UnlinkWindow => {
                    if app.windows.len() > 1 {
                        let mut win = app.windows.remove(app.active_idx);
                        kill_all_children(&mut win.root);
                        if app.active_idx >= app.windows.len() {
                            app.active_idx = app.windows.len() - 1;
                        }
                        resize_all_panes(&mut app);
                        meta_dirty = true;
                        hook_event = Some("window-unlinked");
                    }
                }
                CtrlReq::SetSessionGroup(group_name) => {
                    app.session_group = Some(group_name);
                    state_dirty = true;
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
                        let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
                        let process = {
                            let mut c = std::process::Command::new(&shell_prog);
                            for a in &shell_args { c.arg(a); }
                            c.arg(&cmd);
                            c.stdin(if stdout { std::process::Stdio::piped() } else { std::process::Stdio::null() });
                            c.stdout(if stdin { std::process::Stdio::piped() } else { std::process::Stdio::null() });
                            c.stderr(std::process::Stdio::null());
                            { use crate::platform::HideWindowCommandExt; c.hide_window(); }
                            c.spawn().ok()
                        };
                        
                        app.pipe_panes.push(PipePaneState {
                            pane_id,
                            process,
                            stdin,
                            stdout,
                        });
                    }
                }
                CtrlReq::SelectLayout(layout) => {
                    unzoom_if_zoomed(&mut app);
                    apply_layout(&mut app, &layout);
                    resize_all_panes(&mut app);
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::NextLayout => {
                    unzoom_if_zoomed(&mut app);
                    cycle_layout(&mut app);
                    resize_all_panes(&mut app);
                    meta_dirty = true;
                    state_dirty = true;
                }
                CtrlReq::ListClients(resp) => {
                    let mut output = String::new();
                    if app.client_registry.is_empty() {
                        // Fallback for backward compat when no clients registered yet
                        output.push_str(&format!("/dev/pts/0: {}: {} [{}x{}] (utf8)\n", 
                            app.session_name, 
                            app.windows[app.active_idx].name,
                            app.last_window_area.width,
                            app.last_window_area.height
                        ));
                    } else {
                        let mut clients: Vec<&crate::types::ClientInfo> = app.client_registry.values().collect();
                        clients.sort_by_key(|c| c.id);
                        for ci in &clients {
                            let activity_secs = ci.last_activity.elapsed().as_secs();
                            let kind = if ci.is_control { " (control mode)" } else { "" };
                            output.push_str(&format!("{}: {}: {} [{}x{}] (utf8){} [activity={}s ago]\n",
                                ci.tty_name,
                                app.session_name,
                                app.windows[app.active_idx].name,
                                ci.width, ci.height,
                                kind,
                                activity_secs,
                            ));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListClientsFormat(resp, fmt) => {
                    let mut output = String::new();
                    let mut clients: Vec<&crate::types::ClientInfo> = app.client_registry.values().collect();
                    clients.sort_by_key(|c| c.id);
                    for ci in &clients {
                        let activity_secs = ci.last_activity.elapsed().as_secs();
                        let line = fmt
                            .replace("#{client_name}", &ci.tty_name)
                            .replace("#{client_tty}", &ci.tty_name)
                            .replace("#{client_width}", &ci.width.to_string())
                            .replace("#{client_height}", &ci.height.to_string())
                            .replace("#{client_activity}", &activity_secs.to_string())
                            .replace("#{client_session}", &app.session_name)
                            .replace("#{session_name}", &app.session_name)
                            .replace("#{client_control_mode}", if ci.is_control { "1" } else { "0" });
                        output.push_str(&line);
                        output.push('\n');
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ForceDetachClient(target_cid) => {
                    // Force-detach a specific client by shutting down its TCP stream
                    app.client_sizes.remove(&target_cid);
                    let was_present = app.client_registry.remove(&target_cid).is_some();
                    if was_present {
                        app.attached_clients = app.attached_clients.saturating_sub(1);
                    }
                    if app.latest_client_id == Some(target_cid) {
                        app.latest_client_id = app.client_registry.keys().max().copied();
                    }
                    // Shut down the TCP stream to force disconnect
                    crate::types::shutdown_client_stream(target_cid);
                    // Recompute effective size from remaining clients
                    if let Some((w, h)) = compute_effective_client_size(&app) {
                        app.last_window_area = Rect { x: 0, y: 0, width: w, height: h };
                        resize_all_panes(&mut app);
                    }
                    // Fire detach notification
                    control::emit_notification(&app, crate::types::ControlNotification::ClientDetached {
                        client: format!("/dev/pts/{}", target_cid),
                    });
                    hook_event = Some("client-detached");
                    if app.attached_clients == 0 && app.destroy_unattached {
                        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                        let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                        let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                        let _ = std::fs::remove_file(&regpath);
                        let _ = std::fs::remove_file(&keypath);
                        crate::types::shutdown_persistent_streams();
                        tree::kill_all_children_batch(&mut app.windows);
                        if let Some(mut wp) = app.warm_pane.take() {
                            wp.child.kill().ok();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        std::process::exit(0);
                    }
                }
                CtrlReq::SwitchClient(target, flag) => {
                    // Resolve the target session name based on the flag
                    let current = app.port_file_base();
                    let all_sessions = crate::session::list_session_names();
                    let resolved = match flag {
                        't' => {
                            // Direct target: validate it exists
                            if target.is_empty() {
                                None
                            } else if all_sessions.contains(&target) {
                                Some(target.clone())
                            } else {
                                // Try partial match (prefix)
                                all_sessions.iter().find(|s| s.starts_with(&target)).cloned()
                            }
                        }
                        'n' => {
                            // Next session (alphabetically after current)
                            let pos = all_sessions.iter().position(|s| s == &current);
                            match pos {
                                Some(i) if i + 1 < all_sessions.len() => Some(all_sessions[i + 1].clone()),
                                Some(_) => all_sessions.first().cloned(), // wrap around
                                None => all_sessions.first().cloned(),
                            }
                        }
                        'p' => {
                            // Previous session (alphabetically before current)
                            let pos = all_sessions.iter().position(|s| s == &current);
                            match pos {
                                Some(0) => all_sessions.last().cloned(), // wrap around
                                Some(i) => Some(all_sessions[i - 1].clone()),
                                None => all_sessions.last().cloned(),
                            }
                        }
                        'l' => {
                            // Last session (read from last_session file)
                            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                            let last_path = format!("{}\\.psmux\\last_session", home);
                            std::fs::read_to_string(&last_path).ok()
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty() && s != &current && all_sessions.contains(s))
                        }
                        _ => None,
                    };
                    match resolved {
                        Some(ref sess) if sess != &current => {
                            // Signal the attached client to switch by sending a directive
                            if let Some(cid) = app.latest_client_id {
                                crate::types::send_directive_to_client(cid, &format!("SWITCH {}", sess));
                            } else {
                                // No specific client ID, send to all attached clients
                                crate::types::send_directive_to_all_clients(&format!("SWITCH {}", sess));
                            }
                        }
                        Some(_) => {
                            // Target is the same as current session
                            app.status_message = Some(("switch-client: already on that session".to_string(), std::time::Instant::now(), None));
                            state_dirty = true;
                        }
                        None => {
                            if flag == 't' && !target.is_empty() {
                                app.status_message = Some((format!("switch-client: session not found: {}", target), std::time::Instant::now(), None));
                            } else if flag == 'l' {
                                app.status_message = Some(("switch-client: no last session".to_string(), std::time::Instant::now(), None));
                            } else if all_sessions.len() <= 1 {
                                app.status_message = Some(("switch-client: only one session available".to_string(), std::time::Instant::now(), None));
                            } else {
                                app.status_message = Some(("switch-client: no target session".to_string(), std::time::Instant::now(), None));
                            }
                            state_dirty = true;
                        }
                    }
                }
                CtrlReq::SwitchClientTable(table) => {
                    app.current_key_table = Some(table);
                    state_dirty = true;
                }
                CtrlReq::ListCommands(resp) => {
                    let cmds = TMUX_COMMANDS.join("\n");
                    let _ = resp.send(cmds);
                }
                CtrlReq::LockClient => {
                    app.status_message = Some(("lock: not available on Windows".to_string(), std::time::Instant::now(), None));
                    state_dirty = true;
                }
                CtrlReq::RefreshClient => { state_dirty = true; meta_dirty = true; }
                CtrlReq::SuspendClient => {
                    app.status_message = Some(("suspend: not available on Windows".to_string(), std::time::Instant::now(), None));
                    state_dirty = true;
                }
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
                    // Kill the warm pane and respawn so it picks up the new
                    // env var at process level — avoids PTY echo (#137).
                    if app.warm_pane.is_some() {
                        if let Some(mut old_wp) = app.warm_pane.take() {
                            old_wp.child.kill().ok();
                        }
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(new_wp) => { app.warm_pane = Some(new_wp); }
                            Err(e) => { eprintln!("psmux: warm pane respawn (SetEnv) failed: {e}"); }
                        }
                    }
                }
                CtrlReq::UnsetEnvironment(key) => {
                    app.environment.remove(&key);
                    env::remove_var(&key);
                    // Kill the warm pane and respawn so it no longer has
                    // the removed env var — avoids PTY echo (#137).
                    if app.warm_pane.is_some() {
                        if let Some(mut old_wp) = app.warm_pane.take() {
                            old_wp.child.kill().ok();
                        }
                        match spawn_warm_pane(&*pty_system, &mut app) {
                            Ok(new_wp) => { app.warm_pane = Some(new_wp); }
                            Err(e) => { eprintln!("psmux: warm pane respawn (UnsetEnv) failed: {e}"); }
                        }
                    }
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
                    // Replace (not append) to match tmux semantics – prevents
                    // duplicate hooks on config reload (issue #133).
                    app.hooks.insert(hook, vec![cmd]);
                }
                CtrlReq::AppendHook(hook, cmd) => {
                    // -a/-ga: append to existing hook list so multiple
                    // plugins can register separate handlers (tmux semantics).
                    app.hooks.entry(hook).or_insert_with(Vec::new).push(cmd);
                }
                CtrlReq::ShowHooks(resp) => {
                    let mut output = String::new();
                    for (name, commands) in &app.hooks {
                        if commands.len() == 1 {
                            output.push_str(&format!("{} -> {}\n", name, commands[0]));
                        } else {
                            for (i, cmd) in commands.iter().enumerate() {
                                output.push_str(&format!("{}[{}] -> {}\n", name, i, cmd));
                            }
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
                    // Remove port/key files FIRST so clients see the session
                    // as gone immediately, then kill processes.
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                    let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                    let _ = std::fs::remove_file(&regpath);
                    let _ = std::fs::remove_file(&keypath);
                    crate::types::shutdown_persistent_streams();
                    // Kill all child processes using a single process snapshot
                    tree::kill_all_children_batch(&mut app.windows);
                    // Kill warm pane's child (process::exit skips Drop)
                    if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                    // TerminateProcess is synchronous on Windows — processes
                    // are already dead.  Minimal delay for OS handle cleanup.
                    std::thread::sleep(std::time::Duration::from_millis(10));
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
                        state_dirty = true;
                    }
                }
                CtrlReq::DisplayMenuDirect(menu) => {
                    if !menu.items.is_empty() {
                        app.mode = Mode::MenuMode { menu };
                        state_dirty = true;
                    }
                }
                CtrlReq::DisplayPopup(command, width_spec, height_spec, close_on_exit, start_dir) => {
                    // Resolve percentage dimensions against terminal area (#154)
                    let term_w = app.last_window_area.width;
                    let term_h = app.last_window_area.height;
                    let width = parse_popup_dim(&width_spec, term_w, 80);
                    let height = parse_popup_dim(&height_spec, term_h, 24);
                    // Expand format variables in start_dir (e.g. #{pane_current_path})
                    let start_dir = start_dir.map(|d| expand_format(&d, &app)).filter(|d| !d.is_empty());
                    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
                    if let Some(dir) = &start_dir { let _ = env::set_current_dir(dir); }
                    if !command.is_empty() {
                        // Spawn popup as a real Pane via the popup module
                        let inner_h = height.saturating_sub(2);
                        let inner_w = width.saturating_sub(2);
                        let pane_result = crate::popup::create_popup_pane(
                            &command,
                            start_dir.as_deref(),
                            inner_h,
                            inner_w,
                            app.next_pane_id,
                            &app.session_name,
                            &app.environment,
                        );
                        if let Some(prev) = saved_dir { let _ = env::set_current_dir(prev); }
                        
                        app.mode = Mode::PopupMode {
                            command: command.clone(),
                            output: String::new(),
                            process: None,
                            width,
                            height,
                            close_on_exit,
                            popup_pane: pane_result,
                            scroll_offset: 0,
                        };
                        state_dirty = true;
                    } else {
                        if let Some(prev) = saved_dir { let _ = env::set_current_dir(prev); }
                        app.mode = Mode::PopupMode {
                            command: String::new(),
                            output: "Press 'q' or Escape to close\n".to_string(),
                            process: None,
                            width,
                            height,
                            close_on_exit: true,
                            popup_pane: None,
                            scroll_offset: 0,
                        };
                        state_dirty = true;
                    }
                }
                CtrlReq::ConfirmBefore(prompt, cmd) => {
                    let prompt_text = if prompt.is_empty() {
                        format!("Confirm: {}? (y/n)", cmd)
                    } else {
                        // Don't append (y/n) if prompt already contains it
                        if prompt.contains("(y/n)") {
                            prompt.clone()
                        } else {
                            let base = prompt.trim_end_matches('?');
                            format!("{}? (y/n)", base)
                        }
                    };
                    app.mode = Mode::ConfirmMode {
                        prompt: prompt_text,
                        command: cmd,
                        input: String::new(),
                    };
                    state_dirty = true;
                }
                CtrlReq::ResizePaneAbsolute(axis, size) => {
                    unzoom_if_zoomed(&mut app);
                    resize_pane_absolute(&mut app, &axis, size);
                    resize_all_panes(&mut app);
                    hook_event = Some("after-resize-pane");
                }
                CtrlReq::ResizePanePercent(axis, pct) => {
                    unzoom_if_zoomed(&mut app);
                    // Convert percentage to absolute size based on current window dimensions
                    let area = app.last_window_area;
                    let total = if axis == "x" { area.width } else { area.height };
                    let abs_size = ((total as u32) * (pct as u32) / 100).max(1) as u16;
                    resize_pane_absolute(&mut app, &axis, abs_size);
                    resize_all_panes(&mut app);
                    hook_event = Some("after-resize-pane");
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
                    unzoom_if_zoomed(&mut app);
                    cycle_layout_reverse(&mut app);
                    resize_all_panes(&mut app);
                    meta_dirty = true;
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
                    respawn_active_pane(&mut app, Some(&*pty_system), None, true)?;
                    state_dirty = true;
                }
                CtrlReq::PopupInput(data) => {
                    if let Mode::PopupMode { ref mut popup_pane, .. } = app.mode {
                        if let Some(ref mut pty) = popup_pane {
                            // If child has exited, 'q' closes the popup
                            let child_exited = matches!(pty.child.try_wait(), Ok(Some(_)));
                            if child_exited && data == b"q" {
                                app.mode = Mode::Passthrough;
                            } else if !child_exited {
                                let _ = pty.writer.write_all(&data);
                                let _ = pty.writer.flush();
                            }
                        } else {
                            // No PTY means static popup — 'q' closes it
                            if data == b"q" {
                                app.mode = Mode::Passthrough;
                            }
                        }
                    }
                    state_dirty = true;
                }
                CtrlReq::OverlayClose => {
                    match app.mode {
                        Mode::PopupMode { .. } | Mode::MenuMode { .. } | Mode::ConfirmMode { .. } | Mode::PaneChooser { .. } | Mode::ClockMode | Mode::CustomizeMode { .. } => {
                            app.mode = Mode::Passthrough;
                            state_dirty = true;
                        }
                        _ => {}
                    }
                }
                CtrlReq::ConfirmRespond(yes) => {
                    if let Mode::ConfirmMode { ref command, .. } = app.mode {
                        let cmd = command.clone();
                        app.mode = Mode::Passthrough;
                        if yes {
                            let _ = execute_command_string(&mut app, &cmd);
                        }
                        state_dirty = true;
                    }
                }
                CtrlReq::MenuSelect(idx) => {
                    if let Mode::MenuMode { ref menu } = app.mode {
                        if let Some(item) = menu.items.get(idx) {
                            if !item.is_separator && !item.command.is_empty() {
                                let cmd = item.command.clone();
                                app.mode = Mode::Passthrough;
                                let _ = execute_command_string(&mut app, &cmd);
                                state_dirty = true;
                            }
                        }
                    }
                }
                CtrlReq::MenuNavigate(delta) => {
                    if let Mode::MenuMode { ref mut menu } = app.mode {
                        let len = menu.items.len();
                        if len > 0 {
                            if delta > 0 {
                                // Move down, skipping separators
                                let mut next = (menu.selected + 1) % len;
                                let start = next;
                                while menu.items[next].is_separator {
                                    next = (next + 1) % len;
                                    if next == start { break; }
                                }
                                menu.selected = next;
                            } else {
                                // Move up, skipping separators
                                let mut next = if menu.selected == 0 { len - 1 } else { menu.selected - 1 };
                                let start = next;
                                while menu.items[next].is_separator {
                                    next = if next == 0 { len - 1 } else { next - 1 };
                                    if next == start { break; }
                                }
                                menu.selected = next;
                            }
                            state_dirty = true;
                        }
                    }
                }
                CtrlReq::ShowTextPopup(title, content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
                    let height = (lines.len() as u16 + 2).max(5);
                    app.mode = Mode::PopupMode {
                        command: title,
                        output: content,
                        process: None,
                        width: width.min(120),
                        height,
                        close_on_exit: false,
                        popup_pane: None,
                        scroll_offset: 0,
                    };
                    state_dirty = true;
                }
                CtrlReq::StatusMessage(msg) => {
                    app.status_message = Some((msg, std::time::Instant::now(), None));
                    state_dirty = true;
                }
                CtrlReq::ClearPromptHistory => {
                    app.command_history.clear();
                    app.command_history_idx = 0;
                }
                CtrlReq::ShowPromptHistory(persistent) => {
                    if persistent {
                        let content = if app.command_history.is_empty() {
                            "(no prompt history)\n".to_string()
                        } else {
                            app.command_history.iter().enumerate()
                                .map(|(i, cmd)| format!("{}: {}", i, cmd))
                                .collect::<Vec<_>>().join("\n")
                        };
                        let lines: Vec<&str> = content.lines().collect();
                        let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
                        let height = (lines.len() as u16 + 2).max(5);
                        app.mode = Mode::PopupMode {
                            command: "show-prompt-history".to_string(),
                            output: content,
                            process: None,
                            width: width.min(120),
                            height: height.min(40),
                            close_on_exit: false,
                            popup_pane: None,
                            scroll_offset: 0,
                        };
                        state_dirty = true;
                    }
                }
                CtrlReq::ControlRegister { client_id, echo, notif_tx } => {
                    app.control_clients.insert(client_id, crate::types::ControlClient {
                        client_id,
                        cmd_counter: 0,
                        echo_enabled: echo,
                        notification_tx: notif_tx,
                        paused_panes: std::collections::HashSet::new(),
                        subscriptions: std::collections::HashMap::new(),
                        subscription_values: std::collections::HashMap::new(),
                        subscription_last_check: std::collections::HashMap::new(),
                        pause_after_secs: None,
                        output_paused_panes: std::collections::HashSet::new(),
                        pane_last_output: std::collections::HashMap::new(),
                    });
                    // Register control client in the client registry
                    let tty = format!("/dev/pts/{}", client_id);
                    app.client_registry.insert(client_id, crate::types::ClientInfo {
                        id: client_id,
                        width: app.last_window_area.width,
                        height: app.last_window_area.height,
                        connected_at: std::time::Instant::now(),
                        last_activity: std::time::Instant::now(),
                        tty_name: tty,
                        is_control: true,
                    });
                    app.attached_clients = app.attached_clients.saturating_add(1);
                }
                CtrlReq::ControlSubscribe { client_id, name, target, format } => {
                    if let Some(cc) = app.control_clients.get_mut(&client_id) {
                        cc.subscriptions.insert(name.clone(), (target, format));
                        // Clear cached value so the first check always emits
                        cc.subscription_values.remove(&name);
                        cc.subscription_last_check.remove(&name);
                    }
                }
                CtrlReq::ControlUnsubscribe { client_id, name } => {
                    if let Some(cc) = app.control_clients.get_mut(&client_id) {
                        cc.subscriptions.remove(&name);
                        cc.subscription_values.remove(&name);
                        cc.subscription_last_check.remove(&name);
                    }
                }
                CtrlReq::ControlSetPauseAfter { client_id, pause_after_secs } => {
                    if let Some(cc) = app.control_clients.get_mut(&client_id) {
                        cc.pause_after_secs = pause_after_secs;
                        if pause_after_secs.is_none() {
                            // Clear all pause state when disabling
                            cc.output_paused_panes.clear();
                            cc.pane_last_output.clear();
                        }
                    }
                }
                CtrlReq::ControlContinuePane { client_id, pane_id } => {
                    if let Some(cc) = app.control_clients.get_mut(&client_id) {
                        if cc.output_paused_panes.remove(&pane_id) {
                            let _ = cc.notification_tx.try_send(
                                crate::types::ControlNotification::Continue { pane_id }
                            );
                        }
                    }
                }
                CtrlReq::ControlDeregister { client_id } => {
                    app.control_clients.remove(&client_id);
                    app.client_registry.remove(&client_id);
                    app.attached_clients = app.attached_clients.saturating_sub(1);
                }
                CtrlReq::CustomizeMode => {
                    let options = crate::server::option_catalog::build_option_list(&app);
                    app.mode = Mode::CustomizeMode {
                        options,
                        selected: 0,
                        scroll_offset: 0,
                        editing: false,
                        edit_buffer: String::new(),
                        edit_cursor: 0,
                        filter: String::new(),
                    };
                    state_dirty = true;
                }
                CtrlReq::CustomizeNavigate(delta) => {
                    if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, editing, .. } = app.mode {
                        if !editing {
                            let visible: Vec<usize> = options.iter().enumerate()
                                .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i)
                                .collect();
                            if !visible.is_empty() {
                                let cur_pos = visible.iter().position(|&i| i == *selected).unwrap_or(0);
                                let new_pos = if delta > 0 {
                                    (cur_pos + delta as usize).min(visible.len() - 1)
                                } else {
                                    cur_pos.saturating_sub((-delta) as usize)
                                };
                                *selected = visible[new_pos];
                                // Update scroll offset to keep selection visible
                                if new_pos < *scroll_offset {
                                    *scroll_offset = new_pos;
                                } else if new_pos >= *scroll_offset + 20 {
                                    *scroll_offset = new_pos.saturating_sub(19);
                                }
                            }
                            state_dirty = true;
                        }
                    }
                }
                CtrlReq::CustomizeEdit => {
                    if let Mode::CustomizeMode { ref options, selected, ref mut editing, ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                        if !*editing {
                            if let Some((_, value, _)) = options.get(selected) {
                                *edit_buffer = value.clone();
                                *edit_cursor = edit_buffer.len();
                                *editing = true;
                                state_dirty = true;
                            }
                        }
                    }
                }
                CtrlReq::CustomizeEditUpdate(text) => {
                    if let Mode::CustomizeMode { editing, ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                        if editing {
                            *edit_buffer = text.clone();
                            *edit_cursor = edit_buffer.len();
                            state_dirty = true;
                        }
                    }
                }
                CtrlReq::CustomizeEditConfirm => {
                    if let Mode::CustomizeMode { ref mut options, selected, ref mut editing, ref edit_buffer, .. } = app.mode {
                        if *editing {
                            let name = options[selected].0.clone();
                            let value = edit_buffer.clone();
                            options[selected].1 = value.clone();
                            *editing = false;
                            options::apply_set_option(&mut app, &name, &value, true);
                            state_dirty = true;
                        }
                    }
                }
                CtrlReq::CustomizeEditCancel => {
                    if let Mode::CustomizeMode { ref mut editing, ref mut edit_buffer, .. } = app.mode {
                        if *editing {
                            *editing = false;
                            *edit_buffer = String::new();
                            state_dirty = true;
                        }
                    }
                }
                CtrlReq::CustomizeResetDefault => {
                    if let Mode::CustomizeMode { ref mut options, selected, editing, .. } = app.mode {
                        if !editing {
                            if let Some(def) = option_catalog::default_for(&options[selected].0) {
                                let name = options[selected].0.clone();
                                let value = def.to_string();
                                options[selected].1 = value.clone();
                                options::apply_set_option(&mut app, &name, &value, true);
                                state_dirty = true;
                            }
                        }
                    }
                }
                CtrlReq::CustomizeFilter(text) => {
                    if let Mode::CustomizeMode { ref mut filter, ref mut selected, ref mut scroll_offset, ref options, .. } = app.mode {
                        *filter = text;
                        // Reset selection to first matching option
                        let first_match = options.iter().enumerate()
                            .find(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                            .map(|(i, _)| i);
                        if let Some(idx) = first_match {
                            *selected = idx;
                        }
                        *scroll_offset = 0;
                        state_dirty = true;
                    }
                }
                CtrlReq::RunCommand(cmd, resp) => {
                    let result = execute_command_string(&mut app, &cmd);
                    match result {
                        Ok(()) => { let _ = resp.send("OK".to_string()); }
                        Err(e) => { let _ = resp.send(format!("error: {}", e)); }
                    }
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
                    let _ = execute_command_string(&mut app, &cmd);
                }
                // Emit control mode notifications for hook events
                if !app.control_clients.is_empty() {
                    let active_win = &app.windows[app.active_idx];
                    let win_id = active_win.id;
                    let active_pane_id = get_active_pane_id(&active_win.root, &active_win.active_path).unwrap_or(0);
                    match event {
                        "after-new-window" => {
                            control::emit_notification(&app, crate::types::ControlNotification::WindowAdd { window_id: win_id });
                        }
                        "after-kill-pane" | "window-closed" => {
                            control::emit_notification(&app, crate::types::ControlNotification::WindowClose { window_id: win_id });
                        }
                        "after-rename-window" => {
                            let name = active_win.name.clone();
                            control::emit_notification(&app, crate::types::ControlNotification::WindowRenamed { window_id: win_id, name });
                        }
                        "after-select-window" => {
                            control::emit_notification(&app, crate::types::ControlNotification::SessionWindowChanged {
                                session_id: app.session_id, window_id: win_id,
                            });
                        }
                        "after-select-pane" => {
                            control::emit_notification(&app, crate::types::ControlNotification::WindowPaneChanged {
                                window_id: win_id, pane_id: active_pane_id,
                            });
                        }
                        "after-rename-session" => {
                            let name = app.session_name.clone();
                            control::emit_notification(&app, crate::types::ControlNotification::SessionRenamed { name });
                        }
                        "client-attached" => {
                            let name = app.session_name.clone();
                            control::emit_notification(&app, crate::types::ControlNotification::SessionChanged {
                                session_id: app.session_id, name,
                            });
                        }
                        "client-detached" => {
                            control::emit_notification(&app, crate::types::ControlNotification::ClientDetached {
                                client: "client".to_string(),
                            });
                        }
                        "after-split-window" | "after-resize-pane" | "after-break-pane"
                        | "after-join-pane" | "after-rotate-window" | "after-swap-pane" => {
                            control::emit_notification(&app, crate::types::ControlNotification::LayoutChange {
                                window_id: win_id,
                                layout: format!("{}x{}", app.last_window_area.width, app.last_window_area.height),
                            });
                        }
                        "window-linked" => {
                            control::emit_notification(&app, crate::types::ControlNotification::WindowAdd { window_id: win_id });
                        }
                        "window-unlinked" => {
                            control::emit_notification(&app, crate::types::ControlNotification::WindowClose { window_id: win_id });
                        }
                        _ => {}
                    }
                }
                // Check if the hook itself changed active_idx
                if app.active_idx != _pre_hook_idx && crate::debug_log::server_log_enabled() {
                    crate::debug_log::server_log("switch", &format!(
                        "active_idx changed {} -> {} by HOOK event={}",
                        _pre_hook_idx, app.active_idx, event));
                }
            }
            // Restore temporary -t focus after non-temp command completes.
            // Use pane ID (not path) because kill-pane restructures the
            // tree and invalidates saved paths (#71).
            if !is_temp_focus {
                if let Some((restore_idx, restore_pane_id)) = temp_focus_restore.take() {
                    if restore_idx < app.windows.len() {
                        app.active_idx = restore_idx;
                        let win = &mut app.windows[restore_idx];
                        if let Some(path) = crate::tree::find_path_by_id(&win.root, restore_pane_id) {
                            win.active_path = path;
                        }
                        // If the pane was killed, keep whatever active_path
                        // kill_pane_at_path already set (MRU target).
                    }
                }
            }
            if mutates_state {
                state_dirty = true;
            }
        }
                // No trailing cleanup: temp_focus_restore persists across
                // batch boundaries so the actual command that follows in a
                // later batch can still benefit from the temp focus (and
                // will restore when it processes as a non-temp-focus req).
            }
        }
        // Drain async run-shell results (non-blocking).
        if let Some(rx) = app.run_shell_rx.as_ref() {
            while let Ok((title, text)) = rx.try_recv() {
                if !text.is_empty() {
                    let lines: Vec<&str> = text.lines().collect();
                    let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
                    let height = (lines.len() as u16 + 2).max(5);
                    app.mode = Mode::PopupMode {
                        command: title,
                        output: text,
                        process: None,
                        width: width.min(120),
                        height,
                        close_on_exit: false,
                        popup_pane: None,
                        scroll_offset: 0,
                    };
                    state_dirty = true;
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
            // Check bell/activity state for the pushed frame
            let push_alert_hooks = helpers::check_window_activity(&mut app);
            for event in &push_alert_hooks {
                crate::commands::fire_hooks(&mut app, event);
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
            let layout_json = dump_layout_json_fast(&mut app)?;
            combined_buf.clear();
            let ss_escaped = json_escape_string(&cached_status_style);
            let sl_expanded = json_escape_string(&expand_format(&app.status_left, &app));
            let sr_expanded = json_escape_string(&expand_format(&app.status_right, &app));
            let pbs_escaped = json_escape_string(&app.pane_border_style);
            let pabs_escaped = json_escape_string(&app.pane_active_border_style);
            let pbhs_escaped = json_escape_string(&app.pane_border_hover_style);
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
                "{{\"layout\":{},\"windows\":{},\"prefix\":\"{}\",\"prefix2\":\"{}\",\"tree\":{},\"base_index\":{},\"pane_base_index\":{},\"prediction_dimming\":{},\"status_style\":\"{}\",\"status_left\":\"{}\",\"status_right\":\"{}\",\"pane_border_style\":\"{}\",\"pane_active_border_style\":\"{}\",\"pane_border_hover_style\":\"{}\",\"wsf\":\"{}\",\"wscf\":\"{}\",\"wss\":\"{}\",\"ws_style\":\"{}\",\"wsc_style\":\"{}\",\"clock_mode\":{},\"bindings\":{},\"status_left_length\":{},\"status_right_length\":{},\"status_lines\":{},\"status_format\":{},\"mode_style\":\"{}\",\"status_position\":\"{}\",\"status_justify\":\"{}\",\"cursor_style_code\":{},\"status_visible\":{},\"repeat_time\":{},\"zoomed\":{},\"pwsh_mouse_selection\":{}}}",
                layout_json, cached_windows_json, cached_prefix_str, cached_prefix2_str, cached_tree_json, cached_base_index, app.pane_base_index, cached_pred_dim, ss_escaped, sl_expanded, sr_expanded, pbs_escaped, pabs_escaped, pbhs_escaped, wsf_escaped, wscf_escaped, wss_escaped, ws_style_escaped, wsc_style_escaped,
                matches!(app.mode, Mode::ClockMode), cached_bindings_json,
                app.status_left_length, app.status_right_length, app.status_lines, status_format_json,
                mode_style_escaped, status_position_escaped, status_justify_escaped,
                cursor_style_code, app.status_visible, app.repeat_time_ms,
                app.windows.get(app.active_idx).map_or(false, |w| w.zoom_saved.is_some()),
                app.pwsh_mouse_selection,
            ));
            // Inject overlay state (popup, menu, confirm, display_panes)
            {
                // Inject clock_colour if set
                if let Some(cc) = app.user_options.get("clock-mode-colour") {
                    if combined_buf.ends_with('}') {
                        combined_buf.pop();
                        combined_buf.push_str(",\"clock_colour\":\"");
                        combined_buf.push_str(&json_escape_string(cc));
                        combined_buf.push_str("\"}");
                    }
                }
                // Inject pane-border-status and pane-border-format
                if let Some(pbs) = app.user_options.get("pane-border-status") {
                    if combined_buf.ends_with('}') {
                        combined_buf.pop();
                        combined_buf.push_str(",\"pane_border_status\":\"");
                        combined_buf.push_str(&json_escape_string(pbs));
                        combined_buf.push('"');
                        if let Some(pbf) = app.user_options.get("pane-border-format") {
                            combined_buf.push_str(",\"pane_border_format\":\"");
                            combined_buf.push_str(&json_escape_string(pbf));
                            combined_buf.push('"');
                        }
                        combined_buf.push('}');
                    }
                }
                let overlay_json = serialize_overlay_json(&app);
                if !overlay_json.is_empty() && combined_buf.ends_with('}') {
                    combined_buf.pop();
                    combined_buf.push_str(&overlay_json);
                    combined_buf.push('}');
                }
            }
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
            // Inject bell AFTER caching (one-shot: should not persist in cache)
            if app.bell_forward {
                app.bell_forward = false;
                if combined_buf.ends_with('}') {
                    combined_buf.pop();
                    combined_buf.push_str(",\"bell\":true}");
                }
            }
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
                    let bg_cmd = crate::commands::ensure_background(&cmd);
                    let _ = execute_command_string(&mut app, &bg_cmd);
                }
                if app.active_idx != _pre_status_idx && crate::debug_log::server_log_enabled() {
                    crate::debug_log::server_log("switch", &format!(
                        "active_idx changed {} -> {} by status-interval hook",
                        _pre_status_idx, app.active_idx));
                }
                // Mark state dirty so the next loop iteration pushes a fresh
                // frame with re-expanded strftime codes (%H:%M:%S, %r, etc.)
                // in status-left / status-right.  Without this, the status
                // bar clock never updates for persistent (TUI) clients.
                state_dirty = true;
            }
        }
        // ── Subscription check: expand format strings and emit %subscription-changed ──
        // Zero cost when no clients have subscriptions.
        if !app.control_clients.is_empty() {
            let now_sub = std::time::Instant::now();
            // Phase 1: collect (client_id, sub_name, format) pairs that need checking
            let mut to_check: Vec<(u64, String, String)> = Vec::new();
            for client in app.control_clients.values_mut() {
                if client.subscriptions.is_empty() {
                    continue;
                }
                let sub_names: Vec<String> = client.subscriptions.keys().cloned().collect();
                for name in sub_names {
                    // Rate limit: at most once per second per subscription
                    if let Some(last) = client.subscription_last_check.get(&name) {
                        if now_sub.duration_since(*last).as_secs() < 1 {
                            continue;
                        }
                    }
                    client.subscription_last_check.insert(name.clone(), now_sub);
                    let format = client.subscriptions[&name].1.clone();
                    to_check.push((client.client_id, name, format));
                }
            }
            // Phase 2: expand formats with immutable borrow of app
            let mut sub_results: Vec<(u64, String, String)> = Vec::new();
            for (cid, name, format) in &to_check {
                let expanded = crate::format::expand_format(format, &app);
                sub_results.push((*cid, name.clone(), expanded));
            }
            // Phase 3: compare and emit notifications
            let active_win = &app.windows[app.active_idx];
            let win_id = active_win.id;
            let pane_id = get_active_pane_id(&active_win.root, &active_win.active_path).unwrap_or(0);
            let session_id = app.session_id;
            let win_idx = app.active_idx;
            let mut sub_notifs: Vec<(u64, crate::types::ControlNotification)> = Vec::new();
            for (cid, name, expanded) in sub_results {
                if let Some(cc) = app.control_clients.get(&cid) {
                    let changed = match cc.subscription_values.get(&name) {
                        Some(prev) => prev != &expanded,
                        None => true,
                    };
                    if changed {
                        sub_notifs.push((cid, crate::types::ControlNotification::SubscriptionChanged {
                            name: name.clone(),
                            session_id,
                            window_id: win_id,
                            window_index: win_idx,
                            pane_id,
                            value: expanded.clone(),
                        }));
                    }
                }
            }
            // Phase 4: update cached values and send notifications
            for (cid, ref notif) in &sub_notifs {
                if let Some(cc) = app.control_clients.get_mut(cid) {
                    if let crate::types::ControlNotification::SubscriptionChanged { name, value, .. } = notif {
                        cc.subscription_values.insert(name.clone(), value.clone());
                    }
                }
            }
            for (cid, notif) in sub_notifs {
                if let Some(cc) = app.control_clients.get(&cid) {
                    let _ = cc.notification_tx.try_send(notif);
                }
            }
        }
        // ── PaneChooser timeout ──
        // Auto-close display-panes overlay after display-panes-time (default 1000ms).
        if let Mode::PaneChooser { opened_at } = &app.mode {
            if opened_at.elapsed() > Duration::from_millis(app.display_panes_time_ms) {
                app.mode = Mode::Passthrough;
                state_dirty = true;
            }
        }
        // ── Popup child exit detection ──
        // Check if popup PTY's child process has exited; if so, auto-close.
        if let Mode::PopupMode { ref mut popup_pane, close_on_exit, .. } = app.mode {
            let should_close = if let Some(ref mut pane) = popup_pane {
                matches!(pane.child.try_wait(), Ok(Some(_)))
            } else { false };
            if should_close && close_on_exit {
                app.mode = Mode::Passthrough;
                state_dirty = true;
            }
        }
        // Check if all windows/panes have exited (throttled to every 250ms)
        if last_reap.elapsed() >= Duration::from_millis(100) {
            last_reap = Instant::now();
            let (all_empty, any_pruned, any_newly_dead) = tree::reap_children(&mut app)?;
            if any_pruned {
                // A pane was removed from the tree - resize remaining panes to fill the space
                resize_all_panes(&mut app);
            }
            if any_pruned || any_newly_dead {
                // A pane exited — fire hooks whether it was removed (remain-on-exit off)
                // or just marked dead (remain-on-exit on).  Fixes #227.
                state_dirty = true;
                meta_dirty = true;
                crate::commands::fire_hooks(&mut app, "pane-died");
                crate::commands::fire_hooks(&mut app, "pane-exited");
            }
            if app.exit_empty && all_empty {
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
                let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
                let _ = std::fs::remove_file(&regpath);
                let _ = std::fs::remove_file(&keypath);
                crate::types::shutdown_persistent_streams();
                // Kill warm pane's child (process::exit skips Drop)
                if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                std::thread::sleep(std::time::Duration::from_millis(10));
                std::process::exit(0);
            }
        }
        // recv_timeout already handles the wait; no additional sleep needed.
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
#[path = "../../tests-rs/test_server.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests-rs/test_issue169_manual_rename.rs"]
mod test_issue169;

#[cfg(test)]
#[path = "../../tests-rs/test_pane_title.rs"]
mod test_pane_title;

#[cfg(test)]
#[path = "../../tests-rs/test_issue202_switch_client.rs"]
mod test_issue202;

#[cfg(test)]
#[path = "../../tests-rs/test_new_session_env.rs"]
mod test_new_session_env;