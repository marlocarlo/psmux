use super::*;
use super::srv_loop_ctx::LoopCtx;

// ── Capture / Buffer / Display helpers ──

pub(crate) fn handle_capture_pane(app: &mut AppState, resp: std::sync::mpsc::Sender<String>) -> io::Result<()> {
    if is_active_pane_squelched(app) {
        let _ = resp.send(String::new());
    } else if let Some(text) = capture_active_pane_text(app)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
    Ok(())
}

pub(crate) fn handle_capture_pane_styled(app: &mut AppState, resp: std::sync::mpsc::Sender<String>, s: Option<i32>, e: Option<i32>) -> io::Result<()> {
    if is_active_pane_squelched(app) {
        let _ = resp.send(String::new());
    } else if let Some(text) = capture_active_pane_styled(app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
    Ok(())
}

pub(crate) fn handle_capture_pane_range(app: &mut AppState, resp: std::sync::mpsc::Sender<String>, s: Option<i32>, e: Option<i32>) -> io::Result<()> {
    if is_active_pane_squelched(app) {
        let _ = resp.send(String::new());
    } else if let Some(text) = capture_active_pane_range(app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
    Ok(())
}

pub(crate) fn handle_list_panes(app: &mut AppState, resp: std::sync::mpsc::Sender<String>) {
    super::helpers::propagate_osc_titles(app);
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
                    Err(_) => (vt100::MouseProtocolMode::None, vt100::MouseProtocolEncoding::Default, false),
                };
                panes.push((p.id, p.last_cols, p.last_rows, mode, enc, alt));
            }
            Node::Split { children, .. } => { for c in children { collect_panes(c, panes); } }
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

pub(crate) fn handle_list_all_panes(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
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

pub(crate) fn handle_display_message(app: &mut AppState, resp: std::sync::mpsc::Sender<String>, fmt: String, target_pane_idx: Option<usize>, set_status_bar: bool, duration_ms: Option<u64>) {
    super::helpers::propagate_osc_titles(app);
    let result = if let Some(pane_idx) = target_pane_idx {
        crate::format::expand_format_for_pane(&fmt, app, app.active_idx, pane_idx)
    } else {
        expand_format(&fmt, app)
    };
    if set_status_bar {
        app.status_message = Some((result.clone(), Instant::now(), duration_ms));
    }
    let _ = resp.send(result);
}

// ── Buffer ops ──

pub(crate) fn handle_list_buffers(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
    let mut output = String::new();
    for (i, buf) in app.paste_buffers.iter().enumerate() {
        let preview: String = buf.chars().take(50).collect();
        output.push_str(&format!("buffer{}: {} bytes: \"{}\"\n", i, buf.len(), preview));
    }
    let _ = resp.send(output);
}

pub(crate) fn handle_list_buffers_format(app: &AppState, resp: std::sync::mpsc::Sender<String>, fmt: String) {
    let mut output = Vec::new();
    for (i, _buf) in app.paste_buffers.iter().enumerate() {
        set_buffer_idx_override(Some(i));
        output.push(expand_format(&fmt, app));
        set_buffer_idx_override(None);
    }
    let _ = resp.send(output.join("\n"));
}

// ── Environment ops ──

pub(crate) fn handle_set_environment(app: &mut AppState, ctx: &mut LoopCtx, key: String, value: String) {
    app.environment.insert(key.clone(), value.clone());
    env::set_var(&key, &value);
    respawn_warm_pane_if_needed(app, ctx);
}

pub(crate) fn handle_unset_environment(app: &mut AppState, ctx: &mut LoopCtx, key: String) {
    app.environment.remove(&key);
    env::remove_var(&key);
    respawn_warm_pane_if_needed(app, ctx);
}

fn respawn_warm_pane_if_needed(app: &mut AppState, ctx: &mut LoopCtx) {
    if app.warm_pane.is_some() {
        if let Some(mut old_wp) = app.warm_pane.take() {
            old_wp.child.kill().ok();
        }
        match spawn_warm_pane(&*ctx.pty_system, app) {
            Ok(new_wp) => { app.warm_pane = Some(new_wp); }
            Err(e) => { eprintln!("psmux: warm pane respawn failed: {e}"); }
        }
    }
}

pub(crate) fn handle_show_environment(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
    let mut output = String::new();
    for (key, value) in &app.environment {
        output.push_str(&format!("{}={}\n", key, value));
    }
    for (key, value) in env::vars() {
        if (key.starts_with("PSMUX") || key.starts_with("TMUX")) && !app.environment.contains_key(&key) {
            output.push_str(&format!("{}={}\n", key, value));
        }
    }
    let _ = resp.send(output);
}

// ── Hook ops ──

pub(crate) fn handle_set_hook(app: &mut AppState, hook: String, cmd: String) {
    app.hooks.insert(hook, vec![cmd]);
}

pub(crate) fn handle_append_hook(app: &mut AppState, hook: String, cmd: String) {
    app.hooks.entry(hook).or_insert_with(Vec::new).push(cmd);
}

pub(crate) fn handle_show_hooks(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
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
    if output.is_empty() { output.push_str("(no hooks)\n"); }
    let _ = resp.send(output);
}

// ── WaitFor ──

pub(crate) fn handle_wait_for(app: &mut AppState, channel: String, op: WaitForOp) {
    match op {
        WaitForOp::Lock => {
            let entry = app.wait_channels.entry(channel).or_insert_with(|| WaitChannel { locked: false, waiters: Vec::new() });
            entry.locked = true;
        }
        WaitForOp::Unlock => {
            if let Some(ch) = app.wait_channels.get_mut(&channel) {
                ch.locked = false;
                for waiter in ch.waiters.drain(..) { let _ = waiter.send(()); }
            }
        }
        WaitForOp::Signal => {
            if let Some(ch) = app.wait_channels.get_mut(&channel) {
                for waiter in ch.waiters.drain(..) { let _ = waiter.send(()); }
            }
        }
        WaitForOp::Wait => {
            app.wait_channels.entry(channel).or_insert_with(|| WaitChannel { locked: false, waiters: Vec::new() });
        }
    }
}

// ── Popup / Menu / Confirm ──

pub(crate) fn handle_display_popup(app: &mut AppState, command: String, width_spec: String, height_spec: String, close_on_exit: bool, start_dir: Option<String>) {
    let term_w = app.last_window_area.width;
    let term_h = app.last_window_area.height;
    let width = parse_popup_dim(&width_spec, term_w, 80);
    let height = parse_popup_dim(&height_spec, term_h, 24);
    let start_dir = start_dir.map(|d| expand_format(&d, app)).filter(|d| !d.is_empty());
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(dir) = &start_dir { let _ = env::set_current_dir(dir); }
    if !command.is_empty() {
        let inner_h = height.saturating_sub(2);
        let inner_w = width.saturating_sub(2);
        let pane_result = crate::popup::create_popup_pane(
            &command, start_dir.as_deref(), inner_h, inner_w,
            app.next_pane_id, &app.session_name, &app.environment,
        );
        if let Some(prev) = saved_dir { let _ = env::set_current_dir(prev); }
        app.mode = Mode::PopupMode {
            command, output: String::new(), process: None,
            width, height, close_on_exit, popup_pane: pane_result, scroll_offset: 0,
        };
    } else {
        if let Some(prev) = saved_dir { let _ = env::set_current_dir(prev); }
        app.mode = Mode::PopupMode {
            command: String::new(), output: "Press 'q' or Escape to close\n".to_string(),
            process: None, width, height, close_on_exit: true, popup_pane: None, scroll_offset: 0,
        };
    }
}

pub(crate) fn handle_confirm_before(app: &mut AppState, prompt: String, cmd: String) {
    let prompt_text = if prompt.is_empty() {
        format!("Confirm: {}? (y/n)", cmd)
    } else if prompt.contains("(y/n)") {
        prompt
    } else {
        let base = prompt.trim_end_matches('?');
        format!("{}? (y/n)", base)
    };
    app.mode = Mode::ConfirmMode { prompt: prompt_text, command: cmd, input: String::new() };
}

pub(crate) fn handle_display_menu(app: &mut AppState, menu_def: String, x: Option<i16>, y: Option<i16>) {
    let menu = parse_menu_definition(&menu_def, x, y);
    if !menu.items.is_empty() {
        app.mode = Mode::MenuMode { menu };
    }
}

// ── PipePane ──

pub(crate) fn handle_pipe_pane(app: &mut AppState, cmd: String, stdin: bool, stdout: bool, toggle: bool) {
    let win = &app.windows[app.active_idx];
    let pane_id = get_active_pane_id(&win.root, &win.active_path).unwrap_or(0);
    let has_existing = app.pipe_panes.iter().any(|p| p.pane_id == pane_id);

    if cmd.is_empty() {
        close_pipe(app, pane_id);
    } else if toggle && has_existing {
        close_pipe(app, pane_id);
    } else {
        close_pipe(app, pane_id);
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
        app.pipe_panes.push(PipePaneState { pane_id, process, stdin, stdout });
    }
}

fn close_pipe(app: &mut AppState, pane_id: usize) {
    if let Some(idx) = app.pipe_panes.iter().position(|p| p.pane_id == pane_id) {
        if let Some(ref mut proc) = app.pipe_panes[idx].process { let _ = proc.kill(); }
        app.pipe_panes.remove(idx);
    }
}

// ── Control mode registration ──

pub(crate) fn handle_control_register(app: &mut AppState, client_id: u64, echo: bool, notif_tx: mpsc::SyncSender<crate::types::ControlNotification>) {
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

// ── Customize mode ──

pub(crate) fn handle_customize_mode(app: &mut AppState) {
    let options = crate::server::option_catalog::build_option_list(app);
    app.mode = Mode::CustomizeMode {
        options, selected: 0, scroll_offset: 0,
        editing: false, edit_buffer: String::new(), edit_cursor: 0, filter: String::new(),
    };
}

// ── Focus events ──

pub(crate) fn handle_focus_in(app: &mut AppState) {
    if app.focus_events {
        let win = &mut app.windows[app.active_idx];
        fn send_focus_seq(node: &mut Node, seq: &[u8]) {
            match node {
                Node::Leaf(p) => { let _ = p.writer.write_all(seq); let _ = p.writer.flush(); }
                Node::Split { children, .. } => { for c in children { send_focus_seq(c, seq); } }
            }
        }
        send_focus_seq(&mut win.root, b"\x1b[I");
    }
}

pub(crate) fn handle_focus_out(app: &mut AppState) {
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
}

// ── SendPrefix ──

pub(crate) fn handle_send_prefix(app: &mut AppState) {
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
