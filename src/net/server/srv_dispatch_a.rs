//! First half of dispatch_req: window/pane/focus/mouse/input/options/buffers.

#[allow(unused_imports)]
use super::*;
use super::srv_loop_ctx::LoopCtx;
use super::srv_window_ops::*;
use super::srv_navigation::*;
use super::srv_dump_state::*;
use super::srv_send_keys::*;
use super::srv_options_config::*;
use super::srv_client_session::*;
use super::srv_misc::*;

pub(super) fn dispatch_req(app: &mut AppState, ctx: &mut LoopCtx, req: CtrlReq) -> io::Result<Option<&'static str>> {
    match req {
        // ── Window creation ──
        CtrlReq::NewWindow(cmd, name, detached, start_dir) => {
            handle_new_window(app, ctx, cmd, name, detached, start_dir)?;
            ctx.meta_dirty = true; Ok(Some("after-new-window"))
        }
        CtrlReq::NewWindowPrint(cmd, name, detached, start_dir, format_str, resp) => {
            handle_new_window_print(app, ctx, cmd, name, detached, start_dir, format_str, resp)?;
            ctx.meta_dirty = true; Ok(Some("after-new-window"))
        }
        CtrlReq::SplitWindow(k, cmd, detached, start_dir, split_size, resp) => {
            handle_split_window(app, ctx, k, cmd, detached, start_dir, split_size, resp)?;
            ctx.meta_dirty = true; Ok(Some("after-split-window"))
        }
        CtrlReq::SplitWindowPrint(k, cmd, detached, start_dir, split_size, format_str, resp) => {
            handle_split_window_print(app, ctx, k, cmd, detached, start_dir, split_size, format_str, resp)?;
            ctx.meta_dirty = true; Ok(Some("after-split-window"))
        }

        // ── Pane kill ──
        CtrlReq::KillPane => {
            if let Some(cmds) = app.hooks.get("before-kill-pane") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
            unzoom_if_zoomed(app); let _ = kill_active_pane(app); resize_all_panes(app);
            ctx.meta_dirty = true; Ok(Some("after-kill-pane"))
        }
        CtrlReq::KillPaneById(pid) => {
            if let Some(cmds) = app.hooks.get("before-kill-pane") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
            unzoom_if_zoomed(app); let _ = crate::pane::kill_pane_by_id(app, pid); resize_all_panes(app);
            ctx.meta_dirty = true; Ok(Some("after-kill-pane"))
        }

        // ── Capture ──
        CtrlReq::CapturePane(resp) => { handle_capture_pane(app, resp)?; Ok(None) }
        CtrlReq::CapturePaneStyled(resp, s, e) => { handle_capture_pane_styled(app, resp, s, e)?; Ok(None) }
        CtrlReq::CapturePaneRange(resp, s, e) => { handle_capture_pane_range(app, resp, s, e)?; Ok(None) }

        // ── Focus window/pane ──
        CtrlReq::FocusWindow(wid) => { handle_focus_window(app, wid); ctx.meta_dirty = true; Ok(None) }
        CtrlReq::FocusWindowByName(ref name) => { handle_focus_window_by_name(app, name); ctx.meta_dirty = true; Ok(None) }
        CtrlReq::FocusPane(pid) => { handle_focus_pane(app, pid); ctx.meta_dirty = true; Ok(None) }
        CtrlReq::FocusPaneByIndex(idx) => { handle_focus_pane_by_index(app, idx); ctx.meta_dirty = true; Ok(None) }
        CtrlReq::FocusPaneCmd(pid) => {
            let old_path = app.windows[app.active_idx].active_path.clone();
            switch_with_copy_save(app, |app| { focus_pane_by_id(app, pid); });
            if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(app); }
            ctx.meta_dirty = true; Ok(None)
        }
        CtrlReq::FocusWindowCmd(wid) => {
            switch_with_copy_save(app, |app| { if let Some(idx) = find_window_index_by_id(app, wid) { app.active_idx = idx; } });
            resize_all_panes(app); ctx.meta_dirty = true; Ok(None)
        }

        // ── Temp focus (for -t targeting) ──
        CtrlReq::FocusWindowTemp(wid) => { handle_focus_window_temp(app, ctx, wid); Ok(None) }
        CtrlReq::FocusWindowByNameTemp(ref name) => { handle_focus_window_by_name_temp(app, ctx, name); Ok(None) }
        CtrlReq::FocusPaneTemp(pid) => { handle_focus_pane_temp(app, ctx, pid); Ok(None) }
        CtrlReq::FocusPaneByIndexTemp(idx) => { handle_focus_pane_by_index_temp(app, ctx, idx); Ok(None) }

        // ── Session/client ──
        CtrlReq::SessionInfo(resp) => { handle_session_info(app, resp); Ok(None) }
        CtrlReq::ClientAttach(cid) => Ok(handle_client_attach(app, cid)),
        CtrlReq::ClientDetach(cid) => Ok(handle_client_detach(app, cid)),
        CtrlReq::HasSession(resp) => { let _ = resp.send(true); Ok(None) }

        // ── Dump state/layout ──
        CtrlReq::DumpLayout(resp) => { let json = dump_layout_json(app)?; let _ = resp.send(json); Ok(None) }
        CtrlReq::DumpState(resp, allow_nc) => { handle_dump_state(app, ctx, resp, allow_nc)?; Ok(None) }

        // ── Input (text/keys) ──
        CtrlReq::SendText(s) => { app.status_message = None; send_text_to_active(app, &s)?; ctx.echo_pending_until = Some(Instant::now()); Ok(None) }
        CtrlReq::SendKey(k) => { app.status_message = None; send_key_to_active(app, &k)?; ctx.echo_pending_until = Some(Instant::now()); Ok(None) }
        CtrlReq::SendPaste(s) => { send_paste_to_active(app, &s)?; ctx.echo_pending_until = Some(Instant::now()); Ok(None) }
        CtrlReq::SendKeys(keys, literal) => { handle_send_keys(app, keys, literal)?; ctx.echo_pending_until = Some(Instant::now()); Ok(None) }
        CtrlReq::SendKeysX(cmd) => Ok(handle_send_keys_x(app, cmd)?),

        // ── Zoom / prefix / copy mode ──
        CtrlReq::ZoomPane => { toggle_zoom(app); ctx.state_dirty = true; ctx.meta_dirty = true; Ok(Some("after-resize-pane")) }
        CtrlReq::PrefixBegin => { app.client_prefix_active = true; ctx.state_dirty = true; Ok(None) }
        CtrlReq::PrefixEnd => { app.client_prefix_active = false; ctx.state_dirty = true; Ok(None) }
        CtrlReq::CopyEnter => { enter_copy_mode(app); Ok(Some("pane-mode-changed")) }
        CtrlReq::CopyEnterPageUp => {
            enter_copy_mode(app);
            let half = app.windows.get(app.active_idx)
                .and_then(|w| active_pane(&w.root, &w.active_path))
                .map(|p| p.last_rows as usize).unwrap_or(20);
            scroll_copy_up(app, half);
            Ok(Some("pane-mode-changed"))
        }
        CtrlReq::ClockMode => { app.mode = Mode::ClockMode; ctx.state_dirty = true; Ok(Some("pane-mode-changed")) }
        CtrlReq::CopyMove(dx, dy) => { move_copy_cursor(app, dx, dy); Ok(None) }
        CtrlReq::CopyAnchor => {
            if let Some((r,c)) = current_prompt_pos(app) {
                app.copy_anchor = Some((r,c)); app.copy_anchor_scroll_offset = app.copy_scroll_offset; app.copy_pos = Some((r,c));
            }
            Ok(None)
        }
        CtrlReq::CopyYank => {
            let _ = yank_selection(app);
            if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
            exit_copy_mode(app);
            Ok(Some("pane-mode-changed"))
        }
        CtrlReq::CopyRectToggle => {
            app.copy_selection_mode = match app.copy_selection_mode {
                crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                _ => crate::types::SelectionMode::Rect,
            };
            Ok(None)
        }

        // ── Client size ──
        CtrlReq::ClientSize(cid, w, h) => {
            app.client_sizes.insert(cid, (w, h));
            app.latest_client_id = Some(cid);
            if let Some(info) = app.client_registry.get_mut(&cid) {
                info.width = w; info.height = h; info.last_activity = Instant::now();
            }
            let (ew, eh) = compute_effective_client_size(app).unwrap_or((w, h));
            app.last_window_area = Rect { x: 0, y: 0, width: ew, height: eh };
            resize_all_panes(app);
            let need_respawn = app.warm_pane.as_ref().map_or(true, |wp| wp.rows != eh || wp.cols != ew);
            if need_respawn {
                if let Some(mut old) = app.warm_pane.take() { old.child.kill().ok(); }
                if let Ok(wp) = spawn_warm_pane(&*ctx.pty_system, app) { app.warm_pane = Some(wp); }
            }
            Ok(Some("client-resized"))
        }

        // ── Mouse events ──
        CtrlReq::MouseDown(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_down(app, x, y); ctx.state_dirty = true; ctx.meta_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseDownRight(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(app, x, y, 2, true); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseDownMiddle(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(app, x, y, 1, true); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseDrag(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_drag(app, x, y); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseUp(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_up(app, x, y); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseUpRight(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(app, x, y, 2, false); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseUpMiddle(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_button(app, x, y, 1, false); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::MouseMove(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_mouse_motion(app, x, y); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::ScrollUp(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_scroll_up(app, x, y); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::ScrollDown(cid,x,y) => { if app.mouse_enabled { app.latest_client_id = Some(cid); remote_scroll_down(app, x, y); ctx.state_dirty = true; ctx.echo_pending_until = Some(Instant::now()); } Ok(None) }
        CtrlReq::PaneMouse(cid, pane_id, button, col, row, press) => {
            if app.mouse_enabled { app.latest_client_id = Some(cid); handle_pane_mouse(app, pane_id, button, col, row, press); ctx.state_dirty = true; ctx.meta_dirty = true; ctx.echo_pending_until = Some(Instant::now()); }
            Ok(None)
        }
        CtrlReq::PaneScroll(cid, pane_id, up) => {
            if app.mouse_enabled { app.latest_client_id = Some(cid); handle_pane_scroll(app, pane_id, up); ctx.state_dirty = true; ctx.meta_dirty = true; ctx.echo_pending_until = Some(Instant::now()); }
            Ok(None)
        }
        CtrlReq::SplitSetSizes(cid, path, sizes) => {
            if app.mouse_enabled { app.latest_client_id = Some(cid); handle_split_set_sizes(app, &path, &sizes); ctx.state_dirty = true; ctx.meta_dirty = true; ctx.echo_pending_until = Some(Instant::now()); }
            Ok(None)
        }
        CtrlReq::SplitResizeDone(cid) => {
            if app.mouse_enabled { app.latest_client_id = Some(cid); handle_split_resize_done(app); ctx.state_dirty = true; ctx.meta_dirty = true; }
            Ok(None)
        }

        // ── Window navigation ──
        CtrlReq::NextWindow => {
            if let Some(cmds) = app.hooks.get("before-select-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
            if !app.windows.is_empty() { switch_with_copy_save(app, |app| { app.last_window_idx = app.active_idx; app.active_idx = (app.active_idx + 1) % app.windows.len(); }); resize_all_panes(app); }
            ctx.meta_dirty = true; Ok(Some("after-select-window"))
        }
        CtrlReq::PrevWindow => {
            if let Some(cmds) = app.hooks.get("before-select-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
            if !app.windows.is_empty() { switch_with_copy_save(app, |app| { app.last_window_idx = app.active_idx; app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); }); resize_all_panes(app); }
            ctx.meta_dirty = true; Ok(Some("after-select-window"))
        }
        CtrlReq::LastWindow => {
            if app.windows.len() > 1 && app.last_window_idx < app.windows.len() {
                switch_with_copy_save(app, |app| { let tmp = app.active_idx; app.active_idx = app.last_window_idx; app.last_window_idx = tmp; });
            }
            ctx.meta_dirty = true; Ok(Some("after-select-window"))
        }
        CtrlReq::SelectWindow(idx) => { handle_select_window(app, idx); ctx.meta_dirty = true; Ok(Some("after-select-window")) }
        CtrlReq::SelectPane(dir) => { handle_select_pane(app, dir); ctx.meta_dirty = true; Ok(Some("after-select-pane")) }

        // ── Rename ──
        CtrlReq::RenameWindow(name) => {
            if let Some(cmds) = app.hooks.get("before-rename-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
            let win = &mut app.windows[app.active_idx]; win.name = name; win.manual_rename = true;
            ctx.meta_dirty = true; Ok(Some("after-rename-window"))
        }
        CtrlReq::RenameSession(name) => { let h = handle_rename_session(app, name); ctx.meta_dirty = true; Ok(h) }

        // ── List queries ──
        CtrlReq::ListWindows(resp) => { propagate_osc_titles(app); let json = list_windows_json(app)?; let _ = resp.send(json); Ok(None) }
        CtrlReq::ListWindowsTmux(resp) => { propagate_osc_titles(app); let text = list_windows_tmux(app); let _ = resp.send(text); Ok(None) }
        CtrlReq::ListWindowsFormat(resp, fmt) => { propagate_osc_titles(app); let text = format_list_windows(app, &fmt); let _ = resp.send(text); Ok(None) }
        CtrlReq::ListTree(resp) => { let json = list_tree_json(app)?; let _ = resp.send(json); Ok(None) }
        CtrlReq::ListPanes(resp) => { handle_list_panes(app, resp); Ok(None) }
        CtrlReq::ListPanesFormat(resp, fmt) => { propagate_osc_titles(app); let text = format_list_panes(app, &fmt, app.active_idx); let _ = resp.send(text); Ok(None) }
        CtrlReq::ListAllPanes(resp) => { handle_list_all_panes(app, resp); Ok(None) }
        CtrlReq::ListAllPanesFormat(resp, fmt) => {
            let mut lines = Vec::new();
            for wi in 0..app.windows.len() { lines.push(format_list_panes(app, &fmt, wi)); }
            let _ = resp.send(lines.join("\n")); Ok(None)
        }
        CtrlReq::ListClients(resp) => { handle_list_clients(app, resp); Ok(None) }
        CtrlReq::ListClientsFormat(resp, fmt) => { handle_list_clients_format(app, resp, fmt); Ok(None) }
        CtrlReq::ListBuffers(resp) => { handle_list_buffers(app, resp); Ok(None) }
        CtrlReq::ListBuffersFormat(resp, fmt) => { handle_list_buffers_format(app, resp, fmt); Ok(None) }
        CtrlReq::ListKeys(resp) => { handle_list_keys(app, resp); Ok(None) }
        CtrlReq::ListCommands(resp) => { let cmds = TMUX_COMMANDS.join("\n"); let _ = resp.send(cmds); Ok(None) }

        // ── Sync / pane props ──
        CtrlReq::ToggleSync => { app.sync_input = !app.sync_input; Ok(None) }
        CtrlReq::SetPaneTitle(title) => {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                p.title_locked = !title.is_empty(); p.title = title;
            }
            ctx.meta_dirty = true; Ok(None)
        }
        CtrlReq::SetPaneStyle(style) => {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { p.pane_style = Some(style); }
            Ok(None)
        }

        // ── Kill window / session / server ──
        CtrlReq::KillWindow => {
            if app.windows.len() > 1 {
                let mut win = app.windows.remove(app.active_idx);
                kill_all_children(&mut win.root);
                if app.active_idx >= app.windows.len() { app.active_idx = app.windows.len() - 1; }
            } else {
                kill_all_children(&mut app.windows[0].root);
            }
            Ok(Some("window-closed"))
        }
        CtrlReq::KillSession => { handle_kill_session(app); Ok(None) }
        CtrlReq::KillServer => { handle_kill_server(app); Ok(None) }

        // ── Options ──
        CtrlReq::SetOption(option, value) => { handle_set_option(app, ctx, option, value); Ok(None) }
        CtrlReq::SetOptionQuiet(option, value, quiet) => { handle_set_option_quiet(app, ctx, option, value, quiet); Ok(None) }
        CtrlReq::SetOptionUnset(option) => { handle_set_option_unset(app, &option); Ok(None) }
        CtrlReq::SetOptionAppend(option, value) => {
            if option.starts_with('@') {
                let existing = app.user_options.get(&option).cloned().unwrap_or_default();
                app.user_options.insert(option, format!("{}{}", existing, value));
            } else {
                match option.as_str() {
                    "status-left" => app.status_left.push_str(&value),
                    "status-right" => app.status_right.push_str(&value),
                    "status-style" => app.status_style.push_str(&value),
                    "pane-border-style" => app.pane_border_style.push_str(&value),
                    "pane-active-border-style" => app.pane_active_border_style.push_str(&value),
                    "pane-border-hover-style" => app.pane_border_hover_style.push_str(&value),
                    "window-status-format" => app.window_status_format.push_str(&value),
                    "window-status-current-format" => app.window_status_current_format.push_str(&value),
                    _ => {}
                }
            }
            Ok(None)
        }
        CtrlReq::SetOptionOnlyIfUnset(option, value) => { handle_set_option_only_if_unset(app, ctx, option, value); Ok(None) }
        CtrlReq::ShowOptions(resp) => { handle_show_options(app, resp); Ok(None) }
        CtrlReq::ShowOptionValue(resp, name) => { let val = get_option_value(app, &name); let _ = resp.send(val); Ok(None) }
        CtrlReq::ShowWindowOptionValue(resp, name) => { let val = get_window_option_value(app, &name); let _ = resp.send(val); Ok(None) }
        CtrlReq::ShowWindowOptions(resp) => { let _ = resp.send(render_window_options(app)); Ok(None) }

        // ── Config / bindings ──
        CtrlReq::SourceFile(path) => { handle_source_file(app, path); ctx.state_dirty = true; ctx.meta_dirty = true; Ok(None) }
        CtrlReq::BindKey(table_name, key, command, repeat) => { handle_bind_key(app, table_name, key, command, repeat); ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None) }
        CtrlReq::UnbindKey(key, table) => {
            if let Some(kc) = parse_key_string(&key) {
                let kc = normalize_key_for_binding(kc);
                let target = table.unwrap_or_else(|| "prefix".to_string());
                if let Some(binds) = app.key_tables.get_mut(&target) { binds.retain(|b| b.key != kc); }
            }
            ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None)
        }
        CtrlReq::UnbindAll => { app.key_tables.clear(); app.defaults_suppressed = true; ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None) }
        CtrlReq::UnbindAllInTable(table) => { if let Some(binds) = app.key_tables.get_mut(&table) { binds.clear(); } ctx.meta_dirty = true; ctx.state_dirty = true; Ok(None) }

        // ── Buffers ──
        CtrlReq::SetBuffer(content) => { app.paste_buffers.insert(0, content); if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); } Ok(None) }
        CtrlReq::ShowBuffer(resp) => { let _ = resp.send(app.paste_buffers.first().cloned().unwrap_or_default()); Ok(None) }
        CtrlReq::ShowBufferAt(resp, idx) => { let _ = resp.send(app.paste_buffers.get(idx).cloned().unwrap_or_default()); Ok(None) }
        CtrlReq::DeleteBuffer => { if !app.paste_buffers.is_empty() { app.paste_buffers.remove(0); } Ok(None) }
        CtrlReq::DeleteBufferAt(idx) => { if idx < app.paste_buffers.len() { app.paste_buffers.remove(idx); } Ok(None) }
        CtrlReq::PasteBufferAt(idx) => {
            if idx < app.paste_buffers.len() {
                let text = app.paste_buffers[idx].clone();
                let win = &mut app.windows[app.active_idx];
                if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.writer, "{}", text); }
            }
            Ok(None)
        }
        CtrlReq::SaveBuffer(path) => { if let Some(c) = app.paste_buffers.first() { let _ = std::fs::write(&path, c); } Ok(None) }
        CtrlReq::LoadBuffer(path) => {
            if let Ok(content) = std::fs::read_to_string(&path) { app.paste_buffers.insert(0, content); if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); } }
            Ok(None)
        }
        CtrlReq::ChooseBuffer(resp) => {
            let mut output = String::new();
            for (i, buf) in app.paste_buffers.iter().enumerate() {
                let preview: String = buf.chars().take(50).collect();
                let preview = preview.replace('\n', "\\n").replace('\r', "");
                output.push_str(&format!("buffer{}: {} bytes: \"{}\"\n", i, buf.len(), preview));
            }
            let _ = resp.send(output); Ok(None)
        }

        // Remaining variants handled by dispatch_req_b
        other => super::srv_dispatch_b::dispatch_req_b(app, ctx, other),
    }
}
