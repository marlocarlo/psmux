use super::*;
use srv_loop_ctx::LoopCtx;

/// Initialize the server: create AppState, bind TCP listener, load config,
/// spawn warm pane, create initial window.  Returns (AppState, LoopCtx).
pub(crate) fn initialize_server(
    session_name: String,
    socket_name: Option<String>,
    initial_command: Option<String>,
    raw_command: Option<Vec<String>>,
    start_dir: Option<String>,
    window_name: Option<String>,
    init_size: Option<(u16, u16)>,
    group_target: Option<String>,
    env_vars: Vec<(String, String)>,
) -> io::Result<(AppState, LoopCtx)> {
    let panic_session_name = session_name.clone();
    let panic_socket_name = socket_name.clone();
    std::panic::set_hook(Box::new(move |info| {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let path = format!("{}\\.psmux\\crash.log", home);
        let bt = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::write(&path, format!("{info}\n\nBacktrace:\n{bt}"));
        let base = if let Some(ref sn) = panic_socket_name {
            format!("{}__{}", sn, panic_session_name)
        } else {
            panic_session_name.clone()
        };
        let _ = std::fs::remove_file(format!("{}\\.psmux\\{}.port", home, base));
        let _ = std::fs::remove_file(format!("{}\\.psmux\\{}.key", home, base));
    }));
    install_console_ctrl_handler();

    let pty_system = native_pty_system();

    let mut app = AppState::new(session_name);
    app.socket_name = socket_name;
    app.session_group = group_target;
    app.attached_clients = 0;

    let (tx, rx) = mpsc::channel::<CtrlReq>();
    app.control_rx = Some(rx);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    app.control_port = Some(port);

    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let dir = format!("{}\\.psmux", home);
    let _ = std::fs::create_dir_all(&dir);

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

    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());

    #[cfg(windows)]
    {
        let _ = std::fs::OpenOptions::new()
            .write(true).create(true).truncate(true)
            .open(&keypath)
            .map(|mut f| std::io::Write::write_all(&mut f, session_key.as_bytes()));
    }

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
                    super::connection::handle_connection(stream, tx, &session_key_clone, aliases);
                });
            }
        }
    });

    if let Some((w, h)) = init_size {
        app.last_window_area = ratatui::layout::Rect { x: 0, y: 0, width: w, height: h };
    }

    crate::util::merge_session_env_into_app(&mut app, &env_vars);

    let early_warm = if initial_command.is_none() && raw_command.is_none() && start_dir.is_none() {
        match spawn_warm_pane(&*pty_system, &mut app) {
            Ok(wp) => Some(wp),
            Err(_) => None,
        }
    } else { None };

    crate::config::populate_default_bindings(&mut app);
    load_config(&mut app);

    // Execute queued plugin scripts
    if !app.pending_plugin_scripts.is_empty() {
        let scripts: Vec<String> = app.pending_plugin_scripts.drain(..).collect();
        let target_session = app.port_file_base();
        let mut children: Vec<std::process::Child> = Vec::new();
        for ps1 in &scripts {
            let shell = if which::which("pwsh").is_ok() { "pwsh" } else { "powershell" };
            let mut cmd = std::process::Command::new(shell);
            cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1]);
            if !target_session.is_empty() { cmd.env("PSMUX_TARGET_SESSION", &target_session); }
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            { use crate::platform::HideWindowCommandExt; cmd.hide_window(); }
            if let Ok(child) = cmd.spawn() { children.push(child); }
        }
        if !children.is_empty() {
            let deadline = Instant::now() + Duration::from_secs(5);
            if let Some(rx) = app.control_rx.take() {
                loop {
                    let all_done = children.iter_mut().all(|c| matches!(c.try_wait(), Ok(Some(_))));
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if all_done || remaining.is_zero() {
                        while let Ok(req) = rx.try_recv() { drain_plugin_req(&mut app, req, &shared_aliases_main); }
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

    // Handle early warm pane
    if let Some(wp) = early_warm {
        if !app.warm_enabled {
            let mut wp = wp; wp.child.kill().ok();
        } else if app.default_shell.is_empty() {
            let needs_env = app.environment.iter().any(|(k, _)| !k.starts_with("PSMUX_TARGET_SESSION") && k != "TMUX" && k != "TMUX_PANE");
            let needs_predictions_fix = app.allow_predictions;
            if needs_env || needs_predictions_fix {
                let mut wp = wp; wp.child.kill().ok();
                match spawn_warm_pane(&*pty_system, &mut app) {
                    Ok(new_wp) => { app.warm_pane = Some(new_wp); }
                    Err(e) => { eprintln!("psmux: warm pane respawn failed: {e}"); }
                }
            } else {
                app.warm_pane = Some(wp);
            }
        } else {
            let mut wp = wp; wp.child.kill().ok();
        }
    }

    if let Ok(mut w) = shared_aliases_main.write() { *w = app.command_aliases.clone(); }

    // Create initial window
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(ref dir) = start_dir { env::set_current_dir(dir).ok(); }
    let create_result = if let Some(ref raw_args) = raw_command {
        create_window_raw(&*pty_system, &mut app, raw_args)
    } else {
        create_window(&*pty_system, &mut app, initial_command.as_deref(), None)
    };
    if let Err(e) = create_result {
        let _ = std::fs::remove_file(&regpath);
        let _ = std::fs::remove_file(&keypath);
        if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
        return Err(e);
    }
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    if let Some(n) = window_name { app.windows.last_mut().map(|w| w.name = n); }
    if app.warm_pane.is_none() {
        match spawn_warm_pane(&*pty_system, &mut app) {
            Ok(wp) => { app.warm_pane = Some(wp); }
            Err(e) => { eprintln!("psmux: warm pane pre-spawn failed: {e}"); }
        }
    }
    crate::commands::fire_hooks(&mut app, "client-attached");
    crate::commands::fire_hooks(&mut app, "session-created");
    if should_spawn_warm_server(&app) { spawn_warm_server(&app); }

    let ctx = LoopCtx::new(pty_system, shared_aliases_main);
    Ok((app, ctx))
}
