//! Utility functions for the server main loop: PTY output dispatch,
//! control notifications, status interval, subscriptions, req_tag.

#[allow(unused_imports)]
use super::*;
use super::srv_loop_ctx::LoopCtx;

/// Dispatch PTY output ring buffers to control mode clients.
pub(super) fn dispatch_pty_output_to_control_clients(app: &mut AppState) {
    if app.control_clients.is_empty() { return; }
    let mut pane_outputs: Vec<(usize, String)> = Vec::new();
    for win in &app.windows {
        crate::tree::for_each_pane(&win.root, &mut |pane: &crate::types::Pane| {
            if let Ok(mut ring) = pane.output_ring.lock() {
                if !ring.is_empty() {
                    let bytes: Vec<u8> = ring.drain(..).collect();
                    pane_outputs.push((pane.id, String::from_utf8_lossy(&bytes).to_string()));
                }
            }
        });
    }
    let now = Instant::now();
    for (pane_id, data) in &pane_outputs {
        for client in app.control_clients.values_mut() {
            if client.paused_panes.contains(pane_id) || client.output_paused_panes.contains(pane_id) { continue; }
            if let Some(pause_secs) = client.pause_after_secs {
                let last = client.pane_last_output.entry(*pane_id).or_insert(now);
                let age = now.duration_since(*last);
                *last = now;
                if age.as_secs() >= pause_secs {
                    client.output_paused_panes.insert(*pane_id);
                    let _ = client.notification_tx.try_send(crate::types::ControlNotification::Pause { pane_id: *pane_id });
                    continue;
                }
                let _ = client.notification_tx.try_send(crate::types::ControlNotification::ExtendedOutput { pane_id: *pane_id, age_ms: age.as_millis() as u64, data: data.clone() });
            } else {
                let _ = client.notification_tx.try_send(crate::types::ControlNotification::Output { pane_id: *pane_id, data: data.clone() });
            }
        }
    }
}

/// Emit control mode notifications for hook events.
pub(super) fn emit_control_notifications(app: &AppState, event: &str) {
    if app.control_clients.is_empty() { return; }
    let active_win = &app.windows[app.active_idx];
    let win_id = active_win.id;
    let active_pane_id = get_active_pane_id(&active_win.root, &active_win.active_path).unwrap_or(0);
    match event {
        "after-new-window" | "window-linked" => control::emit_notification(app, crate::types::ControlNotification::WindowAdd { window_id: win_id }),
        "after-kill-pane" | "window-closed" | "window-unlinked" => control::emit_notification(app, crate::types::ControlNotification::WindowClose { window_id: win_id }),
        "after-rename-window" => control::emit_notification(app, crate::types::ControlNotification::WindowRenamed { window_id: win_id, name: active_win.name.clone() }),
        "after-select-window" => control::emit_notification(app, crate::types::ControlNotification::SessionWindowChanged { session_id: app.session_id, window_id: win_id }),
        "after-select-pane" => control::emit_notification(app, crate::types::ControlNotification::WindowPaneChanged { window_id: win_id, pane_id: active_pane_id }),
        "after-rename-session" => control::emit_notification(app, crate::types::ControlNotification::SessionRenamed { name: app.session_name.clone() }),
        "client-attached" => control::emit_notification(app, crate::types::ControlNotification::SessionChanged { session_id: app.session_id, name: app.session_name.clone() }),
        "client-detached" => control::emit_notification(app, crate::types::ControlNotification::ClientDetached { client: "client".to_string() }),
        "after-split-window" | "after-resize-pane" | "after-break-pane" | "after-join-pane" | "after-rotate-window" | "after-swap-pane" | "client-resized" => {
            control::emit_notification(app, crate::types::ControlNotification::LayoutChange { window_id: win_id, layout: format!("{}x{}", app.last_window_area.width, app.last_window_area.height) });
        }
        _ => {}
    }
}

/// Fire status-interval hooks periodically.
pub(super) fn fire_status_interval(app: &mut AppState, ctx: &mut LoopCtx) {
    if app.status_interval > 0 {
        let elapsed = app.last_status_interval_fire.elapsed().as_secs();
        if elapsed >= app.status_interval {
            app.last_status_interval_fire = Instant::now();
            let cmds: Vec<String> = app.hooks.get("status-interval").cloned().unwrap_or_default();
            for cmd in cmds {
                let bg_cmd = crate::commands::ensure_background(&cmd);
                let _ = execute_command_string(app, &bg_cmd);
            }
            ctx.state_dirty = true;
        }
    }
}

/// Check subscriptions and emit %subscription-changed notifications.
pub(super) fn check_subscriptions(app: &mut AppState) {
    if app.control_clients.is_empty() { return; }
    let now_sub = Instant::now();
    let mut to_check: Vec<(u64, String, String)> = Vec::new();
    for client in app.control_clients.values_mut() {
        if client.subscriptions.is_empty() { continue; }
        let sub_names: Vec<String> = client.subscriptions.keys().cloned().collect();
        for name in sub_names {
            if let Some(last) = client.subscription_last_check.get(&name) {
                if now_sub.duration_since(*last).as_secs() < 1 { continue; }
            }
            client.subscription_last_check.insert(name.clone(), now_sub);
            let format = client.subscriptions[&name].1.clone();
            to_check.push((client.client_id, name, format));
        }
    }
    let mut sub_results: Vec<(u64, String, String)> = Vec::new();
    for (cid, name, format) in &to_check {
        let expanded = expand_format(format, app);
        sub_results.push((*cid, name.clone(), expanded));
    }
    let active_win = &app.windows[app.active_idx];
    let win_id = active_win.id;
    let pane_id = get_active_pane_id(&active_win.root, &active_win.active_path).unwrap_or(0);
    let session_id = app.session_id;
    let win_idx = app.active_idx;
    let mut sub_notifs: Vec<(u64, crate::types::ControlNotification)> = Vec::new();
    for (cid, name, expanded) in sub_results {
        if let Some(cc) = app.control_clients.get(&cid) {
            let changed = match cc.subscription_values.get(&name) { Some(prev) => prev != &expanded, None => true };
            if changed {
                sub_notifs.push((cid, crate::types::ControlNotification::SubscriptionChanged {
                    name: name.clone(), session_id, window_id: win_id, window_index: win_idx, pane_id, value: expanded.clone(),
                }));
            }
        }
    }
    for (cid, ref notif) in &sub_notifs {
        if let Some(cc) = app.control_clients.get_mut(cid) {
            if let crate::types::ControlNotification::SubscriptionChanged { name, value, .. } = notif {
                cc.subscription_values.insert(name.clone(), value.clone());
            }
        }
    }
    for (cid, notif) in sub_notifs {
        if let Some(cc) = app.control_clients.get(&cid) { let _ = cc.notification_tx.try_send(notif); }
    }
}

/// Return a short tag for a CtrlReq variant (for debug logging).
pub(super) fn req_tag(req: &CtrlReq) -> &'static str {
    match req {
        CtrlReq::NextWindow => "NextWindow", CtrlReq::PrevWindow => "PrevWindow",
        CtrlReq::SelectWindow(_) => "SelectWindow", CtrlReq::FocusWindow(_) => "FocusWindow",
        CtrlReq::FocusWindowByName(_) => "FocusWindowByName", CtrlReq::FocusWindowTemp(_) => "FocusWindowTemp",
        CtrlReq::FocusWindowByNameTemp(_) => "FocusWindowByNameTemp", CtrlReq::FocusWindowCmd(_) => "FocusWindowCmd",
        CtrlReq::LastWindow => "LastWindow",
        CtrlReq::MouseDown(..) => "MouseDown", CtrlReq::MouseDownRight(..) => "MouseDownRight",
        CtrlReq::MouseDownMiddle(..) => "MouseDownMiddle",
        CtrlReq::FocusPane(_) => "FocusPane", CtrlReq::FocusPaneTemp(_) => "FocusPaneTemp",
        CtrlReq::NewWindow(..) => "NewWindow", CtrlReq::KillWindow => "KillWindow",
        CtrlReq::KillPane => "KillPane", CtrlReq::KillPaneById(_) => "KillPaneById",
        CtrlReq::BreakPane => "BreakPane", CtrlReq::JoinPane { .. } => "JoinPane",
        CtrlReq::MovePane { .. } => "MovePane", CtrlReq::PaneForwardExtract(..) => "PaneForwardExtract",
        CtrlReq::PaneForwardInject { .. } => "PaneForwardInject",
        CtrlReq::PaneForwardResize(..) => "PaneForwardResize",
        CtrlReq::PaneForwardStatus(..) => "PaneForwardStatus",
        CtrlReq::PaneForwardKill(..) => "PaneForwardKill",
        CtrlReq::MoveWindow(..) => "MoveWindow", CtrlReq::SwapWindow(_) => "SwapWindow",
        _ => "",
    }
}
