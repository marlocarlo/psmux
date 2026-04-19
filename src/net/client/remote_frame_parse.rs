use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::DumpState;

/// Drain all pending frames from the reader thread channel.
/// Returns true if a new content frame was received.
pub(crate) fn receive_frames(
    state: &mut RunRemoteState,
    frame_rx: &std::sync::mpsc::Receiver<String>,
    writer: &mut impl Write,
) -> bool {
    let mut got_frame = false;
    loop {
        match frame_rx.try_recv() {
            Ok(line) => {
                if line.trim() == "NC" {
                    state.dump_in_flight = false;
                    state.last_dump_time = Instant::now();
                    if state.key_send_instant.is_some() { state.force_dump = true; }
                } else if line.trim().starts_with("SWITCH ") {
                    let target = line.trim().strip_prefix("SWITCH ").unwrap_or("").to_string();
                    if !target.is_empty() {
                        env::set_var("PSMUX_SWITCH_TO", &target);
                        let _ = writer.write_all(b"client-detach\n");
                        let _ = writer.flush();
                        state.quit = true;
                    }
                } else {
                    if client_log_enabled() {
                        client_log("frame", &format!("received {} bytes", line.len()));
                    }
                    state.dump_buf = line; got_frame = true; state.dump_in_flight = false;
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => { state.quit = true; break; }
        }
    }
    got_frame
}

/// Parse the JSON frame buffer into a DumpState and update the RunRemoteState
/// with all extracted values (overlay state, prefix keys, styles, bindings, etc.).
///
/// Returns `Some((DumpState, cursor_style_code))` on success, `None` on parse error.
pub(crate) fn parse_and_update_state(
    state: &mut RunRemoteState,
    frame_to_parse: &str,
) -> Option<(DumpState, Option<u8>)> {
    let parsed: DumpState = match serde_json::from_str(frame_to_parse) {
        Ok(s) => s,
        Err(_e) => {
            client_log("parse", &format!("JSON parse error: {} (len={})", _e, frame_to_parse.len()));
            state.force_dump = true;
            state.selection_changed = false;
            return None;
        }
    };

    // Update tree and base index
    state.last_tree = parsed.tree.clone();
    state.client_base_index = parsed.base_index;
    state.client_copy_mode = active_pane_in_copy_mode(&parsed.layout);
    state.client_pwsh_selection = parsed.pwsh_mouse_selection;
    state.client_zoomed = parsed.zoomed;
    state.clock_active = parsed.clock_mode;
    state.clock_colour_str = parsed.clock_colour.clone();
    let cursor_style_code = parsed.cursor_style_code;

    // Server-side overlay state
    state.srv_popup_active = parsed.popup_active;
    state.srv_popup_command = parsed.popup_command.clone().unwrap_or_default();
    state.srv_popup_width = parsed.popup_width.unwrap_or(80);
    state.srv_popup_height = parsed.popup_height.unwrap_or(24);
    state.srv_popup_lines = parsed.popup_lines.clone();
    state.srv_popup_rows = parsed.popup_rows.clone();
    let new_popup_has_pty = parsed.popup_has_pty;
    if !state.srv_popup_active || new_popup_has_pty != state.srv_popup_has_pty {
        state.srv_popup_scroll = 0;
    }
    state.srv_popup_has_pty = new_popup_has_pty;
    state.srv_confirm_active = parsed.confirm_active;
    state.srv_confirm_prompt = parsed.confirm_prompt.clone().unwrap_or_default();
    state.srv_menu_active = parsed.menu_active;
    state.srv_menu_title = parsed.menu_title.clone().unwrap_or_default();
    state.srv_menu_selected = parsed.menu_selected;
    state.srv_menu_items = parsed.menu_items.clone();
    state.srv_display_panes = parsed.display_panes;
    state.srv_pane_base_index = parsed.pane_base_index;
    state.srv_customize_active = parsed.customize_active;
    state.srv_customize_selected = parsed.customize_selected;
    state.srv_customize_scroll = parsed.customize_scroll;
    state.srv_customize_editing = parsed.customize_editing;
    state.srv_customize_cursor = parsed.customize_cursor;
    state.srv_customize_edit_buf = parsed.customize_edit_buf.clone().unwrap_or_default();
    state.srv_customize_filter = parsed.customize_filter.clone().unwrap_or_default();
    state.srv_customize_options = parsed.customize_options.clone();

    // OSC 52 clipboard
    if let Some(ref clip_b64) = parsed.clipboard_osc52 {
        if let Some(clip_text) = crate::util::base64_decode(clip_b64) {
            copy_to_system_clipboard(&clip_text);
            state.pending_osc52 = Some(clip_text);
        }
    }

    // Audible bell
    if parsed.bell {
        state.pending_bell = true;
    }

    // Update prefix key from server config
    if let Some(ref prefix_str) = parsed.prefix {
        if let Some((kc, km)) = parse_key_string(prefix_str) {
            if (kc, km) != state.prefix_key {
                state.prefix_key = (kc, km);
                state.prefix_raw_char = if km.contains(KeyModifiers::CONTROL) {
                    if let KeyCode::Char(c) = kc { Some((c as u8 & 0x1f) as char) } else { None }
                } else { None };
            }
        }
    }

    // Update prefix2 key from server config
    if let Some(ref prefix2_str) = parsed.prefix2 {
        if !prefix2_str.is_empty() {
            if let Some((kc, km)) = parse_key_string(prefix2_str) {
                state.prefix2_key = Some((kc, km));
                state.prefix2_raw_char = if km.contains(KeyModifiers::CONTROL) {
                    if let KeyCode::Char(c) = kc { Some((c as u8 & 0x1f) as char) } else { None }
                } else { None };
            }
        } else {
            state.prefix2_key = None;
            state.prefix2_raw_char = None;
        }
    }

    // Update status-style from server config
    if let Some(ref ss) = parsed.status_style {
        if !ss.is_empty() {
            let (fg, bg, bold) = parse_tmux_style_components(ss);
            state.status_fg = fg.unwrap_or(Color::Black);
            state.status_bg = bg.unwrap_or(Color::Green);
            state.status_bold = bold;
        }
    }

    // Sync key bindings
    if !parsed.bindings.is_empty() || !state.synced_bindings.is_empty() {
        state.synced_bindings = parsed.bindings.clone();
    }
    state.defaults_suppressed = parsed.defaults_suppressed;
    state.repeat_time_ms = parsed.repeat_time;

    // Update status-left / status-right
    if let Some(ref sl) = parsed.status_left {
        state.custom_status_left = if sl.is_empty() { None } else { Some(sl.clone()) };
    }
    if let Some(ref sr) = parsed.status_right {
        state.custom_status_right = if sr.is_empty() { None } else { Some(sr.clone()) };
    }

    // Status lines
    let status_lines = if parsed.status_visible { parsed.status_lines } else { 0 };
    let new_sl = (status_lines as u16).max(1);
    if new_sl != state.last_status_lines {
        state.last_status_lines = new_sl;
        state.last_sent_size = (0, 0); // force client-size re-send
    }

    // Update pane border styles
    if let Some(ref pbs) = parsed.pane_border_style {
        if !pbs.is_empty() {
            let (fg, _bg, _bold) = parse_tmux_style_components(pbs);
            if let Some(c) = fg { state.pane_border_fg = c; }
        }
    }
    if let Some(ref pabs) = parsed.pane_active_border_style {
        if !pabs.is_empty() {
            let (fg, _bg, _bold) = parse_tmux_style_components(pabs);
            if let Some(c) = fg { state.pane_active_border_fg = c; }
        }
    }
    if let Some(ref pbhs) = parsed.pane_border_hover_style {
        if !pbhs.is_empty() {
            let (fg, _bg, _bold) = parse_tmux_style_components(pbhs);
            if let Some(c) = fg { state.pane_border_hover_fg = c; }
        }
    }

    // Update window-status-format strings
    if let Some(ref f) = parsed.wsf { if !f.is_empty() { state.win_status_fmt = f.clone(); } }
    if let Some(ref f) = parsed.wscf { if !f.is_empty() { state.win_status_current_fmt = f.clone(); } }
    if let Some(ref s) = parsed.wss { state.win_status_sep = s.clone(); }

    // Update window-status styles
    if let Some(ref s) = parsed.ws_style {
        if !s.is_empty() { state.win_status_style = Some(parse_tmux_style_components(s)); }
    }
    if let Some(ref s) = parsed.wsc_style {
        if !s.is_empty() { state.win_status_current_style = Some(parse_tmux_style_components(s)); }
    }

    // Update mode-style, status-position, status-justify
    if let Some(ref ms) = parsed.mode_style {
        if !ms.is_empty() { state.mode_style_str = ms.clone(); }
    }
    if let Some(ref sp) = parsed.status_position {
        if !sp.is_empty() { state.status_position_str = sp.clone(); }
    }
    if let Some(ref sj) = parsed.status_justify {
        if !sj.is_empty() { state.status_justify_str = sj.clone(); }
    }

    Some((parsed, cursor_style_code))
}

/// Extract the active pane's cursor position (pane-local coords) for post-draw.
pub(crate) fn extract_post_draw_cursor(root: &LayoutJson) -> Option<(u16, u16)> {
    fn active_cursor_info(node: &LayoutJson) -> Option<(bool, u16, u16, bool)> {
        match node {
            LayoutJson::Leaf { active, hide_cursor, cursor_row, cursor_col, copy_mode, .. } => {
                if *active { Some((*hide_cursor, *cursor_row, *cursor_col, *copy_mode)) } else { None }
            }
            LayoutJson::Split { children, .. } => {
                children.iter().find_map(active_cursor_info)
            }
        }
    }
    if let Some((hide, cr, cc, copy)) = active_cursor_info(root) {
        if !hide && !copy {
            return Some((cc, cr));
        }
    }
    None
}
