//! Thin skeleton for the server main loop.
//! All heavy logic is delegated to helper modules under src/server/.

#[allow(unused_imports)]
use super::*;
use super::srv_loop_ctx::LoopCtx;
use super::server_init::initialize_server;
use super::srv_dump_state::*;
use super::srv_dispatch_a::dispatch_req;
use super::srv_utilities::*;

pub fn run_server(
    session_name: String, socket_name: Option<String>,
    initial_command: Option<String>, raw_command: Option<Vec<String>>,
    start_dir: Option<String>, window_name: Option<String>,
    init_size: Option<(u16, u16)>, group_target: Option<String>,
    env_vars: Vec<(String, String)>,
) -> io::Result<()> {
    let (mut app, mut ctx) = initialize_server(
        session_name, socket_name, initial_command, raw_command,
        start_dir, window_name, init_size, group_target, env_vars,
    )?;

    let mut last_client_activity = Instant::now();
    let mut last_reap = Instant::now();

    loop {
        // ── PTY data readiness ──
        let data_ready = crate::types::PTY_DATA_READY.swap(false, std::sync::atomic::Ordering::AcqRel);
        if data_ready {
            ctx.state_dirty = true;
            dispatch_pty_output_to_control_clients(&mut app);
        }
        if matches!(app.mode, Mode::PopupMode { .. }) {
            ctx.state_dirty = true;
        }

        // ── Adaptive timeout ──
        let echo_active = ctx.echo_pending_until.map_or(false, |t| t.elapsed().as_millis() < 50);
        let idle_secs = last_client_activity.elapsed().as_secs();
        let timeout_ms: u64 = if echo_active || data_ready { 1 }
            else if idle_secs < 2 { 5 }
            else if crate::types::has_frame_receivers() { 16 }
            else { 50 };

        // ── Receive and dispatch CtrlReq messages ──
        if let Some(rx) = app.control_rx.as_ref() {
            if let Ok(req) = rx.recv_timeout(Duration::from_millis(timeout_ms)) {
                last_client_activity = Instant::now();
                let mut pending = vec![req];
                while let Ok(r) = rx.try_recv() { pending.push(r); }
                if crate::types::PTY_DATA_READY.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    ctx.state_dirty = true;
                }
                pending.sort_by_key(|r| match r {
                    CtrlReq::DumpState(..) | CtrlReq::DumpLayout(_) => 1,
                    _ => 0,
                });

                for req in pending {
                    let mutates_state = !matches!(&req,
                        CtrlReq::DumpState(..) | CtrlReq::SendText(_)
                        | CtrlReq::SendKey(_) | CtrlReq::SendPaste(_)
                    );
                    let is_temp_focus = matches!(&req,
                        CtrlReq::FocusWindowTemp(_) | CtrlReq::FocusWindowByNameTemp(_)
                        | CtrlReq::FocusPaneTemp(_) | CtrlReq::FocusPaneByIndexTemp(_));
                    let _prev_active_idx = app.active_idx;
                    let _req_tag = req_tag(&req);

                    let hook_event = dispatch_req(&mut app, &mut ctx, req)?;

                    if app.active_idx != _prev_active_idx && crate::debug_log::server_log_enabled() {
                        crate::debug_log::server_log("switch", &format!(
                            "active_idx changed {} -> {} by req={} hook={:?}",
                            _prev_active_idx, app.active_idx, _req_tag, hook_event));
                    }
                    if let Some(event) = hook_event {
                        let _pre_hook_idx = app.active_idx;
                        let cmds: Vec<String> = app.hooks.get(event).cloned().unwrap_or_default();
                        for cmd in cmds { let _ = execute_command_string(&mut app, &cmd); }
                        emit_control_notifications(&app, event);
                        if app.active_idx != _pre_hook_idx && crate::debug_log::server_log_enabled() {
                            crate::debug_log::server_log("switch", &format!(
                                "active_idx changed {} -> {} by HOOK event={}",
                                _pre_hook_idx, app.active_idx, event));
                        }
                    }
                    if !is_temp_focus {
                        if let Some((restore_idx, restore_pane_id)) = ctx.temp_focus_restore.take() {
                            if restore_idx < app.windows.len() {
                                app.active_idx = restore_idx;
                                let win = &mut app.windows[restore_idx];
                                if let Some(path) = crate::tree::find_path_by_id(&win.root, restore_pane_id) {
                                    win.active_path = path;
                                }
                            }
                        }
                    }
                    if mutates_state { ctx.state_dirty = true; }
                }
            }
        }

        // ── Drain async run-shell results ──
        if let Some(rx) = app.run_shell_rx.as_ref() {
            while let Ok((title, text)) = rx.try_recv() {
                if !text.is_empty() {
                    let lines: Vec<&str> = text.lines().collect();
                    let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
                    let height = (lines.len() as u16 + 2).max(5);
                    app.mode = Mode::PopupMode {
                        command: title, output: text, process: None,
                        width: width.min(120), height, close_on_exit: false,
                        popup_pane: None, scroll_offset: 0,
                    };
                    ctx.state_dirty = true;
                }
            }
        }

        // ── Server-push to persistent clients ──
        push_frame_if_dirty(&mut app, &mut ctx)?;

        // ── Status-interval timer ──
        fire_status_interval(&mut app, &mut ctx);

        // ── Subscription check ──
        check_subscriptions(&mut app);

        // ── PaneChooser timeout ──
        if let Mode::PaneChooser { opened_at } = &app.mode {
            if opened_at.elapsed() > Duration::from_millis(app.display_panes_time_ms) {
                app.mode = Mode::Passthrough;
                ctx.state_dirty = true;
            }
        }

        // ── Popup child exit detection ──
        if let Mode::PopupMode { ref mut popup_pane, close_on_exit, .. } = app.mode {
            let should_close = if let Some(ref mut pane) = popup_pane {
                matches!(pane.child.try_wait(), Ok(Some(_)))
            } else { false };
            if should_close && close_on_exit {
                app.mode = Mode::Passthrough;
                ctx.state_dirty = true;
            }
        }

        // ── Reap exited children (throttled) ──
        if last_reap.elapsed() >= Duration::from_millis(100) {
            last_reap = Instant::now();
            let (all_empty, any_pruned, any_newly_dead) = tree::reap_children(&mut app)?;
            if any_pruned { resize_all_panes(&mut app); }
            if any_pruned || any_newly_dead {
                ctx.state_dirty = true;
                ctx.meta_dirty = true;
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
                if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
                std::thread::sleep(Duration::from_millis(10));
                std::process::exit(0);
            }
        }
    }
    #[allow(unreachable_code)]
    Ok(())
}
