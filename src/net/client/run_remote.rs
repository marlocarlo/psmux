use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::DumpState;
use super::remote_overlay_keys::handle_server_overlay_key;
use super::remote_prefix_dispatch::handle_prefix_and_bindings;
use super::remote_chooser::{populate_choose_tree, populate_choose_session, populate_choose_buffer, navigate_session};
use super::remote_key_dispatch::handle_nonprefix_keys;
use super::remote_mouse::handle_mouse_event;
use super::remote_frame_parse::{parse_and_update_state, extract_post_draw_cursor, receive_frames};
use super::remote_render_panes::{render_json, recolor_borders, render_hover_border};
use super::remote_render_overlays::{render_selection_overlay, render_chooser_overlays, render_input_overlays};
use super::remote_render_status::render_status_bar;
use super::remote_render_server_overlays::{render_server_overlays, post_draw_cursor};
use super::remote_setup::setup_connection;
use super::remote_command_send::send_commands_and_request_dump;
use super::remote_post_draw::{handle_post_draw, handle_frame_end};

pub fn run_remote(terminal: &mut Terminal<CrosstermBackend<crate::platform::PsmuxWriter>>, input: &crate::ssh_input::InputSource) -> io::Result<()> {
    let conn = setup_connection()?;
    let frame_rx = conn.frame_rx;
    let mut writer = conn.writer;
    let name = conn.name;
    let home = conn.home;
    let is_ssh_mode = conn.is_ssh_mode;
    let current_session = name.clone();

    // ── Initialize state ───────────────────────────────────────────────
    let latency_log_enabled = env::var("PSMUX_LATENCY_LOG").unwrap_or_default() == "1";
    let latency_log: Option<std::fs::File> = if latency_log_enabled {
        let path = format!("{}\\.psmux\\latency.log", home);
        std::fs::File::create(&path).ok()
    } else { None };
    let mut s = RunRemoteState::new(latency_log);

    loop {
        // Expire stale key_send_instant after 30ms
        if let Some(ks) = s.key_send_instant {
            if ks.elapsed().as_millis() > 30 { s.key_send_instant = None; }
        }
        // Safety valve: release stuck dump_in_flight after 500ms
        if s.dump_in_flight && s.dump_flight_start.elapsed().as_millis() > 500 {
            s.dump_in_flight = false;
        }

        // ── STEP 0: Receive latest frame from reader thread ─────────────
        let got_frame = receive_frames(&mut s, &frame_rx, &mut writer);
        if s.quit && !got_frame { break; }

        // ── STEP 1: Adaptive poll timing ─────────────────────────────────
        let since_dump = s.last_dump_time.elapsed().as_millis() as u64;
        if let Some(kt) = s.last_key_send_time {
            if kt.elapsed().as_millis() > 100 { s.last_key_send_time = None; }
        }
        let typing_active = s.last_key_send_time.is_some();
        #[cfg(windows)]
        let paste_pend_active = !s.paste_pend.is_empty();
        #[cfg(not(windows))]
        let paste_pend_active = false;

        let poll_ms = if paste_pend_active { 1 }
            else if got_frame { 0 }
            else if s.dump_in_flight { 5 }
            else if s.force_dump { 0 }
            else if typing_active { 10u64.saturating_sub(since_dump) }
            else { 16 };

        s.cmd_batch.clear();

        // ── Windows paste buffer management ──────────────────────────────
        #[cfg(windows)]
        {
            let mut batch = Vec::new();
            super::remote_paste::handle_paste_stage(&mut s, &mut batch);
            s.cmd_batch.extend(batch);
        }

        // ── Event loop ───────────────────────────────────────────────────
        {
            let mut _pending_evt = input.read_timeout(Duration::from_millis(poll_ms))?;
            while let Some(_cur_evt) = _pending_evt {
                if input_log_enabled() {
                    match &_cur_evt {
                        Event::Key(key) => {
                            input_log("event", &format!(
                                "Key code={:?} mods={:?} kind={:?} state={:?}",
                                key.code, key.modifiers, key.kind, key.state
                            ));
                        }
                        Event::Mouse(me) => { input_log("event", &format!("Mouse {:?}", me.kind)); }
                        Event::Resize(w, h) => { input_log("event", &format!("Resize {}x{}", w, h)); }
                        Event::Paste(d) => { input_log("event", &format!("Paste ({} bytes)", d.len())); }
                        other => { input_log("event", &format!("Other {:?}", other)); }
                    }
                }
                match _cur_evt {
                    // ── Windows modified-Enter Release suppression ────────
                    #[cfg(windows)]
                    Event::Key(key) if key.kind == KeyEventKind::Release
                        && matches!(key.code, KeyCode::Enter)
                        && s.modified_enter_press_handled =>
                    {
                        // drop phantom Release
                    }
                    // ── Windows Ctrl+V paste interception ─────────────────
                    #[cfg(windows)]
                    Event::Key(key) if key.kind == KeyEventKind::Release
                        && matches!(key.code, KeyCode::Char('v'))
                        && key.modifiers == KeyModifiers::CONTROL =>
                    {
                        if input_log_enabled() {
                            input_log("paste", &format!("Ctrl+V Release detected, paste_pend len={}", s.paste_pend.len()));
                        }
                        s.paste_confirmed = true;
                    }
                    // ── WezTerm: Shift+Enter Release-only ─────────────────
                    #[cfg(windows)]
                    Event::Key(mut key) if key.kind == KeyEventKind::Release
                        && matches!(key.code, KeyCode::Enter)
                        && !key.modifiers.is_empty() =>
                    {
                        key.kind = KeyEventKind::Press;
                        crate::platform::augment_enter_shift(&mut key);
                        s.modified_enter_press_handled = true;
                        let is_prefix = (key.code, key.modifiers) == s.prefix_key
                            || s.prefix_raw_char.map_or(false, |c| matches!(key.code, KeyCode::Char(ch) if ch == c))
                            || s.prefix2_key.map_or(false, |p2| (key.code, key.modifiers) == p2)
                            || s.prefix2_raw_char.map_or(false, |c| matches!(key.code, KeyCode::Char(ch) if ch == c));
                        if !is_prefix {
                            if let Some(encoded) = crate::input::encode_key_event(&key) {
                                s.cmd_batch.push(format!("send-key-raw {}\n",
                                    encoded.iter().map(|b| format!("{:02x}", b)).collect::<String>()));
                            }
                        }
                    }
                    // ── Key Press/Repeat ──────────────────────────────────
                    Event::Key(mut key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
                        #[cfg(windows)]
                        crate::platform::augment_enter_shift(&mut key);
                        #[cfg(windows)]
                        {
                            if matches!(key.code, KeyCode::Enter) && !key.modifiers.is_empty() {
                                s.modified_enter_press_handled = true;
                            } else {
                                s.modified_enter_press_handled = false;
                            }
                        }
                        // Flush paste buffer for non-bufferable keys
                        #[cfg(windows)]
                        {
                            if !s.paste_pend.is_empty() {
                                let is_bufferable = match key.code {
                                    KeyCode::Char(' ') => true,
                                    KeyCode::Char(c) => {
                                        let is_altgr = key.modifiers.contains(KeyModifiers::CONTROL)
                                            && key.modifiers.contains(KeyModifiers::ALT)
                                            && !c.is_ascii_lowercase();
                                        is_altgr || (!key.modifiers.contains(KeyModifiers::CONTROL)
                                                  && !key.modifiers.contains(KeyModifiers::ALT))
                                    }
                                    KeyCode::Enter | KeyCode::Tab => true,
                                    _ => false,
                                };
                                if !is_bufferable {
                                    flush_paste_pend_as_text(&mut s.paste_pend, &mut s.paste_pend_start, &mut s.paste_stage2, &mut s.cmd_batch);
                                }
                            }
                        }
                        // Dynamic prefix key check
                        let is_prefix = (key.code, key.modifiers) == s.prefix_key
                            || s.prefix_raw_char.map_or(false, |c| matches!(key.code, KeyCode::Char(ch) if ch == c))
                            || s.prefix2_key.map_or(false, |p2| (key.code, key.modifiers) == p2)
                            || s.prefix2_raw_char.map_or(false, |c| matches!(key.code, KeyCode::Char(ch) if ch == c));

                        // Expire repeat-mode prefix
                        if s.prefix_armed && s.prefix_repeating
                            && s.prefix_armed_at.elapsed().as_millis() >= s.repeat_time_ms as u128
                        {
                            s.prefix_armed = false;
                            s.prefix_repeating = false;
                            s.cmd_batch.push("prefix-end\n".into());
                        }

                        // Server overlay keys (consume if overlay active)
                        let mut overlay_batch = Vec::new();
                        if handle_server_overlay_key(&mut s, &key, &mut overlay_batch) {
                            s.cmd_batch.extend(overlay_batch);
                        } else if key.code == KeyCode::Esc {
                            // Esc closes client overlays
                            if s.command_input { s.command_input = false; }
                            else if s.renaming { s.renaming = false; }
                            else if s.pane_renaming { s.pane_renaming = false; }
                            else if s.tree_chooser { s.tree_chooser = false; }
                            else if s.buffer_chooser { s.buffer_chooser = false; }
                            else if s.session_chooser { s.session_chooser = false; }
                            else if s.keys_viewer { s.keys_viewer = false; }
                            else if s.confirm_cmd.is_some() { s.confirm_cmd = None; }
                            else if s.window_idx_input { s.window_idx_input = false; }
                            else if s.rsel_start.is_some() {
                                s.rsel_start = None;
                                s.rsel_end = None;
                                s.rsel_pane_rect = None;
                                s.rsel_block = false;
                                s.rsel_dragged = false;
                                s.selection_changed = true;
                            } else if is_prefix && s.prefix_armed {
                                s.prefix_armed = false;
                                s.prefix_repeating = false;
                                if let Some(c) = s.prefix_raw_char {
                                    let escaped = match c { '"' => "\\\"".to_string(), '\\' => "\\\\".to_string(), _ => c.to_string() };
                                    s.cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                                }
                            } else {
                                s.cmd_batch.push("send-key escape\n".into());
                            }
                        } else if is_prefix && !s.prefix_armed {
                            s.prefix_armed = true;
                            s.prefix_armed_at = Instant::now();
                            s.prefix_repeating = false;
                            s.cmd_batch.push("prefix-begin\n".into());
                        } else if s.prefix_armed {
                            let mut batch = Vec::new();
                            let result = handle_prefix_and_bindings(&mut s, &key, &mut batch, &home, &current_session);
                            s.cmd_batch.extend(batch);
                            if result.quit { s.quit = true; }
                            if result.do_choose_tree { populate_choose_tree(&mut s, &home, &current_session); }
                            if result.do_choose_session { populate_choose_session(&mut s, &home, &current_session); }
                            if result.do_choose_buffer { populate_choose_buffer(&mut s, &home, &current_session); }
                            if let Some(next) = result.do_session_nav {
                                let mut nav_batch = Vec::new();
                                if navigate_session(&mut s, &mut nav_batch, &home, &current_session, next) {
                                    s.quit = true;
                                }
                                s.cmd_batch.extend(nav_batch);
                            }
                            if let Some(ref binding) = result.user_binding {
                                if binding.r {
                                    s.prefix_armed = true;
                                    s.prefix_repeating = true;
                                    s.prefix_armed_at = Instant::now();
                                } else {
                                    s.prefix_armed = false;
                                    s.prefix_repeating = false;
                                }
                            } else {
                                s.prefix_armed = false;
                                s.prefix_repeating = false;
                            }
                        } else {
                            let mut batch = Vec::new();
                            if handle_nonprefix_keys(&mut s, &key, &mut batch, &home, &current_session) {
                                s.quit = true;
                            }
                            s.cmd_batch.extend(batch);
                        }
                    }
                    Event::Paste(data) => {
                        let encoded = base64_encode(&data);
                        s.cmd_batch.push(format!("send-paste {}\n", encoded));
                    }
                    Event::Mouse(me) => {
                        let mut batch = Vec::new();
                        handle_mouse_event(&mut s, &me, &mut batch);
                        s.cmd_batch.extend(batch);
                    }
                    Event::FocusGained => { s.cmd_batch.push("focus-in\n".into()); }
                    Event::FocusLost => { s.cmd_batch.push("focus-out\n".into()); }
                    _ => {}
                }
                if s.quit { break; }
                _pending_evt = input.try_read()?;
            }
        }
        if s.quit { break; }

        // ── Post-event paste flush (Windows) ─────────────────────────────
        #[cfg(windows)]
        {
            let mut batch = Vec::new();
            super::remote_paste::handle_post_event_flush(&mut s, &mut batch);
            s.cmd_batch.extend(batch);
            let mut batch2 = Vec::new();
            super::remote_paste::handle_zero_latency_paste_flush(&mut s, &mut batch2);
            s.cmd_batch.extend(batch2);
        }

        // ── STEP 2: Send commands + request screen update ────────────────
        let mut size_changed = false;
        let ts = terminal.size()?;
        if send_commands_and_request_dump(&mut s, &mut writer, (ts.width, ts.height),
            is_ssh_mode, typing_active, since_dump, &mut size_changed) { break; }

        // ── STEP 3: Render ───────────────────────────────────────────────
        let overlays_active = s.command_input || s.renaming || s.pane_renaming || s.tree_chooser
            || s.buffer_chooser || s.session_chooser || s.keys_viewer || s.confirm_cmd.is_some()
            || s.srv_popup_active || s.srv_confirm_active || s.srv_menu_active
            || s.srv_display_panes || s.clock_active;
        if !got_frame && !s.selection_changed && !overlays_active { continue; }
        if s.dump_buf == s.prev_dump_buf && !s.selection_changed && !overlays_active {
            s.last_dump_time = Instant::now();
            continue;
        }

        let frame_to_parse = if got_frame && s.dump_buf != s.prev_dump_buf {
            s.dump_buf.clone()
        } else {
            s.prev_dump_buf.clone()
        };
        let _t_parse = Instant::now();

        let parsed = match parse_and_update_state(&mut s, &frame_to_parse) {
            Some(p) => p,
            None => { s.selection_changed = false; continue; }
        };
        let (ds, state_cursor_style_code) = parsed;
        let _parse_us = _t_parse.elapsed().as_micros();

        let root = ds.layout;
        let windows = ds.windows;
        let base_index = ds.base_index;
        let dim_preds = ds.prediction_dimming;
        let zoomed = ds.zoomed;
        let status_lines = if ds.status_visible { ds.status_lines } else { 0 };
        let status_format = ds.status_format;
        let status_message = ds.status_message;
        let border_status = ds.pane_border_status.unwrap_or_default();
        let border_format = ds.pane_border_format.unwrap_or_default();
        let post_draw_cursor_pos = if !s.clock_active {
            extract_post_draw_cursor(&root)
        } else { None };

        let sel_s = s.rsel_start;
        let sel_e = s.rsel_end;
        let sel_rect = s.rsel_pane_rect;
        let sel_pwsh = s.client_pwsh_selection;
        let sel_block = s.rsel_block;
        let status_at_top = s.status_position_str == "top";
        let border_fg = s.pane_border_fg;
        let active_border_fg = s.pane_active_border_fg;
        let hover_fg = s.pane_border_hover_fg;
        let clock_colour = s.clock_colour_str.as_deref()
            .map(|c| map_color(c))
            .unwrap_or(Color::Cyan);
        let mode_style_str = s.mode_style_str.clone();
        let hovered_border = s.hovered_border.clone();

        terminal.draw(|f| {
            let area = f.area();
            let constraints = if status_at_top {
                vec![Constraint::Length(status_lines as u16), Constraint::Min(1)]
            } else {
                vec![Constraint::Min(1), Constraint::Length(status_lines as u16)]
            };
            let chunks = Layout::default().direction(Direction::Vertical)
                .constraints(constraints).split(area);
            let (content_chunk, status_chunk) = if status_at_top {
                (chunks[1], chunks[0])
            } else {
                (chunks[0], chunks[1])
            };

            s.client_content_area = content_chunk;
            s.client_pane_rects.clear();
            collect_pane_rects(&root, content_chunk, &mut s.client_pane_rects);
            s.client_borders.clear();
            let mut border_path = Vec::new();
            collect_layout_borders(&root, content_chunk, &mut border_path, &mut s.client_borders);

            let active_rect = compute_active_rect_json(&root, content_chunk);

            render_json(f, &root, content_chunk, dim_preds, border_fg, active_border_fg,
                s.clock_active, clock_colour, active_rect, &mode_style_str,
                zoomed, &border_status, &border_format);

            fix_border_intersections(f.buffer_mut());
            recolor_borders(f.buffer_mut(), active_rect, border_fg, active_border_fg);
            render_hover_border(f.buffer_mut(), &hovered_border, hover_fg);

            render_selection_overlay(f, &root, area, sel_s, sel_e, sel_rect, sel_pwsh, sel_block);
            render_chooser_overlays(f, &mut s, content_chunk, &current_session);
            render_status_bar(f, &mut s, status_chunk, &windows, base_index, &name,
                status_lines, &status_format, &status_message);
            render_input_overlays(f, &s, content_chunk);
            render_server_overlays(f, &s, content_chunk, &root);
        })?;

        // ── Post-draw ────────────────────────────────────────────────────
        handle_post_draw(&mut s, is_ssh_mode);
        post_draw_cursor(&root, post_draw_cursor_pos, terminal, status_at_top,
            status_lines, state_cursor_style_code, &mut s.last_cursor_style, is_ssh_mode);

        let _render_us = _t_parse.elapsed().as_micros().saturating_sub(_parse_us as u128);
        handle_frame_end(&mut s, got_frame, _parse_us as u128, _render_us, since_dump);
    }

    // Clean disconnect
    let _ = writer.write_all(b"client-detach\n");
    let _ = writer.flush();
    Ok(())
}
