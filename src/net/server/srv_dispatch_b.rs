//! Second half of dispatch_req: display/swap/layout/session/control/customize.

#[allow(unused_imports)]
use super::*;
use super::srv_loop_ctx::LoopCtx;
use super::srv_window_ops::*;
use super::srv_navigation::*;
use super::srv_options_config::*;
use super::srv_client_session::*;
use super::srv_misc::*;

pub(super) fn dispatch_req_b(app: &mut AppState, ctx: &mut LoopCtx, req: CtrlReq) -> io::Result<Option<&'static str>> {
    match req {
        // ── Display ──
        CtrlReq::DisplayMessage(resp, fmt, target_pane_idx, set_status_bar, duration_ms) => {
            handle_display_message(app, resp, fmt, target_pane_idx, set_status_bar, duration_ms);
            if set_status_bar { ctx.state_dirty = true; }
            Ok(None)
        }
        CtrlReq::DisplayPanes => { app.mode = Mode::PaneChooser { opened_at: Instant::now() }; ctx.state_dirty = true; Ok(None) }
        CtrlReq::DisplayPaneSelect(digit) => {
            let win = &app.windows[app.active_idx];
            let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
            crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
            for (i, (path, _)) in rects.iter().enumerate() {
                if i >= 10 { break; }
                if (i + app.pane_base_index) % 10 == digit {
                    let new_path = path.clone();
                    let old_path = app.windows[app.active_idx].active_path.clone();
                    app.windows[app.active_idx].active_path = new_path;
                    if app.windows[app.active_idx].active_path != old_path { app.last_pane_path = old_path; }
                    break;
                }
            }
            app.mode = Mode::Passthrough; ctx.state_dirty = true; ctx.meta_dirty = true; Ok(None)
        }
        CtrlReq::DisplayPopup(command, width_spec, height_spec, close_on_exit, start_dir) => {
            handle_display_popup(app, command, width_spec, height_spec, close_on_exit, start_dir);
            ctx.state_dirty = true; Ok(None)
        }
        CtrlReq::DisplayMenu(menu_def, x, y) => { handle_display_menu(app, menu_def, x, y); ctx.state_dirty = true; Ok(None) }
        CtrlReq::DisplayMenuDirect(menu) => {
            if !menu.items.is_empty() { app.mode = Mode::MenuMode { menu }; ctx.state_dirty = true; }
            Ok(None)
        }
        CtrlReq::ConfirmBefore(prompt, cmd) => { handle_confirm_before(app, prompt, cmd); ctx.state_dirty = true; Ok(None) }
        CtrlReq::ShowTextPopup(title, content) => {
            let lines: Vec<&str> = content.lines().collect();
            let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
            let height = (lines.len() as u16 + 2).max(5);
            app.mode = Mode::PopupMode {
                command: title, output: content, process: None,
                width: width.min(120), height, close_on_exit: false, popup_pane: None, scroll_offset: 0,
            };
            ctx.state_dirty = true; Ok(None)
        }
        CtrlReq::StatusMessage(msg) => { app.status_message = Some((msg, Instant::now(), None)); ctx.state_dirty = true; Ok(None) }

        // ── Swap / resize / rotate / break / join ──
        CtrlReq::SwapPane(dir) => {
            unzoom_if_zoomed(app);
            match dir.as_str() { "U" => swap_pane(app, FocusDir::Up), _ => swap_pane(app, FocusDir::Down) }
            Ok(Some("after-swap-pane"))
        }
        CtrlReq::ResizePane(dir, amount) => {
            unzoom_if_zoomed(app);
            match dir.as_str() {
                "U" | "D" => resize_pane_vertical(app, if dir == "U" { -(amount as i16) } else { amount as i16 }),
                "L" | "R" => resize_pane_horizontal(app, if dir == "L" { -(amount as i16) } else { amount as i16 }),
                _ => {}
            }
            resize_all_panes(app); ctx.meta_dirty = true; Ok(Some("after-resize-pane"))
        }
        CtrlReq::ResizePaneAbsolute(axis, size) => { unzoom_if_zoomed(app); resize_pane_absolute(app, &axis, size); resize_all_panes(app); Ok(Some("after-resize-pane")) }
        CtrlReq::ResizePanePercent(axis, pct) => {
            unzoom_if_zoomed(app);
            let area = app.last_window_area;
            let total = if axis == "x" { area.width } else { area.height };
            let abs_size = ((total as u32) * (pct as u32) / 100).max(1) as u16;
            resize_pane_absolute(app, &axis, abs_size); resize_all_panes(app);
            Ok(Some("after-resize-pane"))
        }
        CtrlReq::RotateWindow(reverse) => { rotate_panes(app, reverse); Ok(Some("after-rotate-window")) }
        CtrlReq::BreakPane => { unzoom_if_zoomed(app); break_pane_to_window(app); ctx.meta_dirty = true; Ok(Some("after-break-pane")) }
        CtrlReq::JoinPane { src_win, src_pane, target_win, target_pane, horizontal }
        | CtrlReq::MovePane { src_win, src_pane, target_win, target_pane, horizontal } => {
            let h = handle_join_pane(app, src_win, src_pane, target_win, target_pane, horizontal)?;
            ctx.meta_dirty = true; Ok(h)
        }
        CtrlReq::RespawnPane(workdir, kill) => { respawn_active_pane(app, Some(&*ctx.pty_system), workdir.as_deref(), kill)?; Ok(Some("after-respawn-pane")) }
        CtrlReq::RespawnWindow => { respawn_active_pane(app, Some(&*ctx.pty_system), None, true)?; ctx.state_dirty = true; Ok(None) }
        CtrlReq::LastPane => {
            switch_with_copy_save(app, |app| {
                let win = &mut app.windows[app.active_idx];
                if !app.last_pane_path.is_empty() && path_exists(&win.root, &app.last_pane_path) {
                    let tmp = win.active_path.clone(); win.active_path = app.last_pane_path.clone(); app.last_pane_path = tmp;
                } else if !win.active_path.is_empty() {
                    if let Some(idx) = win.active_path.last_mut() { *idx = (*idx + 1) % 2; }
                }
            });
            ctx.meta_dirty = true; Ok(None)
        }

        // ── Layout ──
        CtrlReq::SelectLayout(layout) => { unzoom_if_zoomed(app); apply_layout(app, &layout); resize_all_panes(app); ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None) }
        CtrlReq::NextLayout => { unzoom_if_zoomed(app); cycle_layout(app); resize_all_panes(app); ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None) }
        CtrlReq::PrevLayout => { unzoom_if_zoomed(app); cycle_layout_reverse(app); resize_all_panes(app); ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None) }

        // ── Move / swap / link window ──
        CtrlReq::MoveWindow(target) => {
            if let Some(t) = target {
                if t < app.windows.len() && app.active_idx != t {
                    let win = app.windows.remove(app.active_idx);
                    let insert_idx = if t > app.active_idx { t - 1 } else { t };
                    app.windows.insert(insert_idx.min(app.windows.len()), win);
                    app.active_idx = insert_idx.min(app.windows.len() - 1);
                }
            }
            Ok(None)
        }
        CtrlReq::SwapWindow(target) => { if target < app.windows.len() && app.active_idx != target { app.windows.swap(app.active_idx, target); } Ok(None) }
        CtrlReq::LinkWindow(src, dst) => { let h = handle_link_window(app, ctx, src, dst)?; ctx.state_dirty = true; Ok(h) }
        CtrlReq::UnlinkWindow => {
            if app.windows.len() > 1 {
                let mut win = app.windows.remove(app.active_idx);
                kill_all_children(&mut win.root);
                if app.active_idx >= app.windows.len() { app.active_idx = app.windows.len() - 1; }
                resize_all_panes(app); ctx.meta_dirty = true;
            }
            Ok(Some("window-unlinked"))
        }

        // ── Session ──
        CtrlReq::ClaimSession(name, client_cwd, resp) => { handle_claim_session(app, ctx, name, client_cwd, resp)?; Ok(Some("after-rename-session")) }
        CtrlReq::SetSessionGroup(group_name) => { app.session_group = Some(group_name); ctx.state_dirty = true; Ok(None) }
        CtrlReq::SwitchClient(target, flag) => { handle_switch_client(app, target, flag); Ok(None) }
        CtrlReq::SwitchClientTable(table) => { app.current_key_table = Some(table); ctx.state_dirty = true; Ok(None) }
        CtrlReq::ForceDetachClient(target_cid) => Ok(handle_force_detach_client(app, target_cid)),
        CtrlReq::ServerInfo(resp) => { handle_server_info(app, resp); Ok(None) }
        CtrlReq::FindWindow(resp, pattern) => {
            let mut output = String::new();
            for (i, win) in app.windows.iter().enumerate() {
                if win.name.contains(&pattern) { output.push_str(&format!("{}: {} []\n", i + app.window_base_index, win.name)); }
            }
            let _ = resp.send(output); Ok(None)
        }

        // ── Environment ──
        CtrlReq::SetEnvironment(key, value) => { handle_set_environment(app, ctx, key, value); Ok(None) }
        CtrlReq::UnsetEnvironment(key) => { handle_unset_environment(app, ctx, key); Ok(None) }
        CtrlReq::ShowEnvironment(resp) => { handle_show_environment(app, resp); Ok(None) }

        // ── Hooks ──
        CtrlReq::SetHook(hook, cmd) => { handle_set_hook(app, hook, cmd); Ok(None) }
        CtrlReq::AppendHook(hook, cmd) => { handle_append_hook(app, hook, cmd); Ok(None) }
        CtrlReq::ShowHooks(resp) => { handle_show_hooks(app, resp); Ok(None) }
        CtrlReq::RemoveHook(hook) => { app.hooks.remove(&hook); Ok(None) }

        // ── WaitFor ──
        CtrlReq::WaitFor(channel, op) => { handle_wait_for(app, channel, op); Ok(None) }

        // ── Pipe pane ──
        CtrlReq::PipePane(cmd, stdin, stdout, toggle) => { handle_pipe_pane(app, cmd, stdin, stdout, toggle); Ok(None) }

        // ── Misc ──
        CtrlReq::LockClient => { app.status_message = Some(("lock: not available on Windows".to_string(), Instant::now(), None)); ctx.state_dirty = true; Ok(None) }
        CtrlReq::RefreshClient => { ctx.state_dirty = true; ctx.meta_dirty = true; Ok(None) }
        CtrlReq::SuspendClient => { app.status_message = Some(("suspend: not available on Windows".to_string(), Instant::now(), None)); ctx.state_dirty = true; Ok(None) }
        CtrlReq::CopyModePageUp => { enter_copy_mode(app); move_copy_cursor(app, 0, -20); Ok(None) }
        CtrlReq::ClearHistory => {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                if let Ok(mut parser) = p.term.lock() { *parser = vt100::Parser::new(p.last_rows, p.last_cols, app.history_limit); }
            }
            Ok(None)
        }
        CtrlReq::SendPrefix => { handle_send_prefix(app); Ok(None) }
        CtrlReq::FocusIn => { handle_focus_in(app); Ok(Some("pane-focus-in")) }
        CtrlReq::FocusOut => { handle_focus_out(app); Ok(Some("pane-focus-out")) }
        CtrlReq::CommandPrompt(initial) => { app.mode = Mode::CommandPrompt { input: initial.clone(), cursor: initial.len() }; ctx.state_dirty = true; Ok(None) }
        CtrlReq::ShowMessages(resp) => { let _ = resp.send(String::new()); Ok(None) }
        CtrlReq::ResizeWindow(_dim, _size) => Ok(None),
        CtrlReq::ClearPromptHistory => { app.command_history.clear(); app.command_history_idx = 0; Ok(None) }
        CtrlReq::ShowPromptHistory(persistent) => {
            if persistent {
                let content = if app.command_history.is_empty() { "(no prompt history)\n".to_string() }
                else { app.command_history.iter().enumerate().map(|(i, cmd)| format!("{}: {}", i, cmd)).collect::<Vec<_>>().join("\n") };
                let lines: Vec<&str> = content.lines().collect();
                let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
                let height = (lines.len() as u16 + 2).max(5);
                app.mode = Mode::PopupMode { command: "show-prompt-history".to_string(), output: content, process: None, width: width.min(120), height: height.min(40), close_on_exit: false, popup_pane: None, scroll_offset: 0 };
                ctx.state_dirty = true;
            }
            Ok(None)
        }

        // ── Popup / overlay interaction ──
        CtrlReq::PopupInput(data) => {
            if let Mode::PopupMode { ref mut popup_pane, .. } = app.mode {
                if let Some(ref mut pty) = popup_pane {
                    let child_exited = matches!(pty.child.try_wait(), Ok(Some(_)));
                    if child_exited && data == b"q" { app.mode = Mode::Passthrough; }
                    else if !child_exited { let _ = pty.writer.write_all(&data); let _ = pty.writer.flush(); }
                } else if data == b"q" { app.mode = Mode::Passthrough; }
            }
            ctx.state_dirty = true; Ok(None)
        }
        CtrlReq::OverlayClose => {
            match app.mode {
                Mode::PopupMode { .. } | Mode::MenuMode { .. } | Mode::ConfirmMode { .. }
                | Mode::PaneChooser { .. } | Mode::ClockMode | Mode::CustomizeMode { .. } => {
                    app.mode = Mode::Passthrough; ctx.state_dirty = true;
                }
                _ => {}
            }
            Ok(None)
        }
        CtrlReq::ConfirmRespond(yes) => {
            if let Mode::ConfirmMode { ref command, .. } = app.mode {
                let cmd = command.clone(); app.mode = Mode::Passthrough;
                if yes { let _ = execute_command_string(app, &cmd); }
                ctx.state_dirty = true;
            }
            Ok(None)
        }
        CtrlReq::MenuSelect(idx) => {
            if let Mode::MenuMode { ref menu } = app.mode {
                if let Some(item) = menu.items.get(idx) {
                    if !item.is_separator && !item.command.is_empty() {
                        let cmd = item.command.clone(); app.mode = Mode::Passthrough;
                        let _ = execute_command_string(app, &cmd); ctx.state_dirty = true;
                    }
                }
            }
            Ok(None)
        }
        CtrlReq::MenuNavigate(delta) => {
            if let Mode::MenuMode { ref mut menu } = app.mode {
                let len = menu.items.len();
                if len > 0 {
                    let mut next = if delta > 0 { (menu.selected + 1) % len }
                        else { if menu.selected == 0 { len - 1 } else { menu.selected - 1 } };
                    let start = next;
                    while menu.items[next].is_separator {
                        next = if delta > 0 { (next + 1) % len } else { if next == 0 { len - 1 } else { next - 1 } };
                        if next == start { break; }
                    }
                    menu.selected = next;
                    ctx.state_dirty = true;
                }
            }
            Ok(None)
        }

        // ── Cross-session pane forwarding ──
        CtrlReq::PaneForwardExtract(win_idx, pane_idx, resp) => {
            crate::cross_session_server::handle_pane_forward_extract(app, win_idx, pane_idx, resp);
            resize_all_panes(app); ctx.meta_dirty = true; Ok(None)
        }
        CtrlReq::PaneForwardInject { source_session, source_addr, source_key, forward_id, fwd_port, pid, title, rows, cols, screen_b64, target_win, target_pane, horizontal } => {
            crate::cross_session_server::handle_pane_forward_inject(app, source_session, source_addr, source_key, forward_id, fwd_port, pid, title, rows, cols, screen_b64, target_win, target_pane, horizontal);
            resize_all_panes(app); ctx.meta_dirty = true; Ok(Some("after-join-pane"))
        }
        CtrlReq::PaneForwardResize(fwd_id, fwd_rows, fwd_cols) => {
            if let Some(fp) = app.forwarded_panes.get(&fwd_id) {
                let _ = fp.master.resize(portable_pty::PtySize { rows: fwd_rows, cols: fwd_cols, pixel_width: 0, pixel_height: 0 });
            }
            Ok(None)
        }
        CtrlReq::PaneForwardStatus(fwd_id, resp) => {
            let status = if let Some(fp) = app.forwarded_panes.get_mut(&fwd_id) {
                match fp.child.try_wait() { Ok(Some(_)) => "exited", Ok(None) => "running", Err(_) => "exited" }
            } else { "exited" };
            let _ = resp.send(status.to_string()); Ok(None)
        }
        CtrlReq::PaneForwardKill(fwd_id) => {
            if let Some(mut fp) = app.forwarded_panes.remove(&fwd_id) {
                fp.shutdown.store(true, std::sync::atomic::Ordering::Relaxed); let _ = fp.child.kill();
            }
            Ok(None)
        }

        // ── Control mode ──
        CtrlReq::ControlRegister { client_id, echo, notif_tx } => { handle_control_register(app, client_id, echo, notif_tx); Ok(None) }
        CtrlReq::ControlSubscribe { client_id, name, target, format } => {
            if let Some(cc) = app.control_clients.get_mut(&client_id) {
                cc.subscriptions.insert(name.clone(), (target, format));
                cc.subscription_values.remove(&name); cc.subscription_last_check.remove(&name);
            }
            Ok(None)
        }
        CtrlReq::ControlUnsubscribe { client_id, name } => {
            if let Some(cc) = app.control_clients.get_mut(&client_id) {
                cc.subscriptions.remove(&name); cc.subscription_values.remove(&name); cc.subscription_last_check.remove(&name);
            }
            Ok(None)
        }
        CtrlReq::ControlSetPauseAfter { client_id, pause_after_secs } => {
            if let Some(cc) = app.control_clients.get_mut(&client_id) {
                cc.pause_after_secs = pause_after_secs;
                if pause_after_secs.is_none() { cc.output_paused_panes.clear(); cc.pane_last_output.clear(); }
            }
            Ok(None)
        }
        CtrlReq::ControlContinuePane { client_id, pane_id } => {
            if let Some(cc) = app.control_clients.get_mut(&client_id) {
                if cc.output_paused_panes.remove(&pane_id) {
                    let _ = cc.notification_tx.try_send(crate::types::ControlNotification::Continue { pane_id });
                }
            }
            Ok(None)
        }
        CtrlReq::ControlDeregister { client_id } => {
            app.control_clients.remove(&client_id); app.client_registry.remove(&client_id);
            app.attached_clients = app.attached_clients.saturating_sub(1); Ok(None)
        }

        // ── Customize mode ──
        CtrlReq::CustomizeMode => { handle_customize_mode(app); ctx.state_dirty = true; Ok(None) }
        CtrlReq::CustomizeNavigate(delta) => {
            if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, editing, .. } = app.mode {
                if !editing {
                    let visible: Vec<usize> = options.iter().enumerate()
                        .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str())).map(|(i, _)| i).collect();
                    if !visible.is_empty() {
                        let cur_pos = visible.iter().position(|&i| i == *selected).unwrap_or(0);
                        let new_pos = if delta > 0 { (cur_pos + delta as usize).min(visible.len() - 1) } else { cur_pos.saturating_sub((-delta) as usize) };
                        *selected = visible[new_pos];
                        if new_pos < *scroll_offset { *scroll_offset = new_pos; } else if new_pos >= *scroll_offset + 20 { *scroll_offset = new_pos.saturating_sub(19); }
                    }
                    ctx.state_dirty = true;
                }
            }
            Ok(None)
        }
        CtrlReq::CustomizeEdit => {
            if let Mode::CustomizeMode { ref options, selected, ref mut editing, ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                if !*editing { if let Some((_, value, _)) = options.get(selected) { *edit_buffer = value.clone(); *edit_cursor = edit_buffer.len(); *editing = true; ctx.state_dirty = true; } }
            }
            Ok(None)
        }
        CtrlReq::CustomizeEditUpdate(text) => {
            if let Mode::CustomizeMode { editing, ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                if editing { *edit_buffer = text; *edit_cursor = edit_buffer.len(); ctx.state_dirty = true; }
            }
            Ok(None)
        }
        CtrlReq::CustomizeEditConfirm => {
            if let Mode::CustomizeMode { ref mut options, selected, ref mut editing, ref edit_buffer, .. } = app.mode {
                if *editing {
                    let name = options[selected].0.clone(); let value = edit_buffer.clone();
                    options[selected].1 = value.clone(); *editing = false;
                    apply_set_option(app, &name, &value, true); ctx.state_dirty = true;
                }
            }
            Ok(None)
        }
        CtrlReq::CustomizeEditCancel => {
            if let Mode::CustomizeMode { ref mut editing, ref mut edit_buffer, .. } = app.mode {
                if *editing { *editing = false; *edit_buffer = String::new(); ctx.state_dirty = true; }
            }
            Ok(None)
        }
        CtrlReq::CustomizeResetDefault => {
            if let Mode::CustomizeMode { ref mut options, selected, editing, .. } = app.mode {
                if !editing {
                    if let Some(def) = super::option_catalog::default_for(&options[selected].0) {
                        let name = options[selected].0.clone(); let value = def.to_string();
                        options[selected].1 = value.clone();
                        apply_set_option(app, &name, &value, true); ctx.state_dirty = true;
                    }
                }
            }
            Ok(None)
        }
        CtrlReq::CustomizeFilter(text) => {
            if let Mode::CustomizeMode { ref mut filter, ref mut selected, ref mut scroll_offset, ref options, .. } = app.mode {
                *filter = text;
                let first_match = options.iter().enumerate().find(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str())).map(|(i, _)| i);
                if let Some(idx) = first_match { *selected = idx; }
                *scroll_offset = 0; ctx.state_dirty = true;
            }
            Ok(None)
        }

        // ── Run command ──
        CtrlReq::RunCommand(cmd, resp) => {
            match execute_command_string(app, &cmd) {
                Ok(()) => { let _ = resp.send("OK".to_string()); }
                Err(e) => { let _ = resp.send(format!("error: {}", e)); }
            }
            Ok(None)
        }

        _ => Ok(None),
    }
}
