use super::*;
use super::srv_loop_ctx::LoopCtx;

/// Handle CtrlReq::SessionInfo.
pub(crate) fn handle_session_info(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
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

/// Handle CtrlReq::ClientAttach.
pub(crate) fn handle_client_attach(app: &mut AppState, cid: u64) -> Option<&'static str> {
    app.attached_clients = app.attached_clients.saturating_add(1);
    app.latest_client_id = Some(cid);
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
    // update-environment
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
    Some("client-attached")
}

/// Handle CtrlReq::ClientDetach.
pub(crate) fn handle_client_detach(app: &mut AppState, cid: u64) -> Option<&'static str> {
    app.attached_clients = app.attached_clients.saturating_sub(1);
    app.client_sizes.remove(&cid);
    app.client_registry.remove(&cid);
    app.client_prefix_active = false;
    if app.latest_client_id == Some(cid) {
        app.latest_client_id = None;
    }
    if let Some((w, h)) = compute_effective_client_size(app) {
        app.last_window_area = Rect { x: 0, y: 0, width: w, height: h };
        resize_all_panes(app);
    }
    if app.attached_clients == 0 && app.destroy_unattached {
        cleanup_and_exit(app);
    }
    Some("client-detached")
}

/// Handle CtrlReq::KillSession.
pub(crate) fn handle_kill_session(app: &mut AppState) {
    if let Some(cmds) = app.hooks.get("session-closed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    cleanup_and_exit(app);
}

/// Handle CtrlReq::RenameSession.
pub(crate) fn handle_rename_session(app: &mut AppState, name: String) -> Option<&'static str> {
    if let Some(cmds) = app.hooks.get("before-rename-session") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let old_path = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
    let old_keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
    let new_base = if let Some(ref sn) = app.socket_name {
        format!("{}__{}", sn, name)
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
    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());
    Some("after-rename-session")
}

/// Handle CtrlReq::ForceDetachClient.
pub(crate) fn handle_force_detach_client(app: &mut AppState, target_cid: u64) -> Option<&'static str> {
    app.client_sizes.remove(&target_cid);
    let was_present = app.client_registry.remove(&target_cid).is_some();
    if was_present {
        app.attached_clients = app.attached_clients.saturating_sub(1);
    }
    if app.latest_client_id == Some(target_cid) {
        app.latest_client_id = app.client_registry.keys().max().copied();
    }
    crate::types::shutdown_client_stream(target_cid);
    if let Some((w, h)) = compute_effective_client_size(app) {
        app.last_window_area = Rect { x: 0, y: 0, width: w, height: h };
        resize_all_panes(app);
    }
    control::emit_notification(app, crate::types::ControlNotification::ClientDetached {
        client: format!("/dev/pts/{}", target_cid),
    });
    if app.attached_clients == 0 && app.destroy_unattached {
        cleanup_and_exit(app);
    }
    Some("client-detached")
}

/// Handle CtrlReq::SwitchClient.
pub(crate) fn handle_switch_client(app: &mut AppState, target: String, flag: char) {
    let current = app.port_file_base();
    let all_sessions = crate::session::list_session_names();
    let resolved = match flag {
        't' => {
            if target.is_empty() { None }
            else if all_sessions.contains(&target) { Some(target.clone()) }
            else { all_sessions.iter().find(|s| s.starts_with(&target)).cloned() }
        }
        'n' => {
            let pos = all_sessions.iter().position(|s| s == &current);
            match pos {
                Some(i) if i + 1 < all_sessions.len() => Some(all_sessions[i + 1].clone()),
                Some(_) => all_sessions.first().cloned(),
                None => all_sessions.first().cloned(),
            }
        }
        'p' => {
            let pos = all_sessions.iter().position(|s| s == &current);
            match pos {
                Some(0) => all_sessions.last().cloned(),
                Some(i) => Some(all_sessions[i - 1].clone()),
                None => all_sessions.last().cloned(),
            }
        }
        'l' => {
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
            if let Some(cid) = app.latest_client_id {
                crate::types::send_directive_to_client(cid, &format!("SWITCH {}", sess));
            } else {
                crate::types::send_directive_to_all_clients(&format!("SWITCH {}", sess));
            }
        }
        Some(_) => {
            app.status_message = Some(("switch-client: already on that session".to_string(), std::time::Instant::now(), None));
        }
        None => {
            let msg = if flag == 't' && !target.is_empty() {
                format!("switch-client: session not found: {}", target)
            } else if flag == 'l' {
                "switch-client: no last session".to_string()
            } else if all_sessions.len() <= 1 {
                "switch-client: only one session available".to_string()
            } else {
                "switch-client: no target session".to_string()
            };
            app.status_message = Some((msg, std::time::Instant::now(), None));
        }
    }
}

/// Handle CtrlReq::ListClients.
pub(crate) fn handle_list_clients(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
    let mut output = String::new();
    if app.client_registry.is_empty() {
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
                ci.tty_name, app.session_name, app.windows[app.active_idx].name,
                ci.width, ci.height, kind, activity_secs,
            ));
        }
    }
    let _ = resp.send(output);
}

/// Handle CtrlReq::ListClientsFormat.
pub(crate) fn handle_list_clients_format(app: &AppState, resp: std::sync::mpsc::Sender<String>, fmt: String) {
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

/// Handle CtrlReq::KillServer.
pub(crate) fn handle_kill_server(app: &mut AppState) {
    cleanup_and_exit(app);
}

/// Handle CtrlReq::ServerInfo.
pub(crate) fn handle_server_info(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
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

/// Shared cleanup: remove port/key files, kill all children, exit.
fn cleanup_and_exit(app: &mut AppState) -> ! {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let regpath = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
    let keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
    let _ = std::fs::remove_file(&regpath);
    let _ = std::fs::remove_file(&keypath);
    crate::types::shutdown_persistent_streams();
    tree::kill_all_children_batch(&mut app.windows);
    if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::process::exit(0);
}
