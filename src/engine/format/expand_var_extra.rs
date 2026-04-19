#[allow(unused_imports)]
use super::*;

/// Expand format variables: cursor, mouse, copy mode, buffer, client, server,
/// options, and misc.  Called from `expand_var` for variables not handled there.
pub(crate) fn expand_var_extra(var: &str, app: &AppState, win_idx: usize, fmt_pane_pos: usize, fmt_pane_is_active: bool) -> String {
    let win = &app.windows[win_idx]; // caller validated
    let target_pane = || -> Option<&Pane> {
        crate::tree::get_nth_pane(&win.root, fmt_pane_pos)
    };
    match var {
        // ── Cursor ──
        "cursor_x" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (_, c) = parser.screen().cursor_position();
                    return c.to_string();
                }
            }
            "0".into()
        }
        "cursor_y" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, _) = parser.screen().cursor_position();
                    return r.to_string();
                }
            }
            "0".into()
        }
        "cursor_character" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, c) = parser.screen().cursor_position();
                    if let Some(cell) = parser.screen().cell(r, c) {
                        return cell.contents().to_string();
                    }
                }
            }
            String::new()
        }
        "cursor_flag" => "0".into(),

        // ── Mouse ──
        "mouse_x" => app.last_mouse_x.to_string(),
        "mouse_y" => app.last_mouse_y.to_string(),
        "mouse_line" => {
            if let Some(w) = app.windows.get(win_idx) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        // Convert screen-absolute mouse_y to pane-relative row
                        let mut rects = Vec::new();
                        crate::tree::compute_rects(&w.root, app.last_window_area, &mut rects);
                        let pane_y_offset = rects.iter()
                            .find(|(path, _)| crate::tree::get_active_pane_id_at_path(&w.root, path) == Some(p.id))
                            .map(|(_, rect)| rect.y)
                            .unwrap_or(0);
                        let row = app.last_mouse_y.saturating_sub(pane_y_offset);
                        let mut row_text = String::with_capacity(cols as usize);
                        for col in 0..cols {
                            if let Some(cell) = screen.cell(row, col) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        return row_text.trim_end().to_string();
                    }
                }
            }
            String::new()
        }
        "mouse_word" => {
            if let Some(w) = app.windows.get(win_idx) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        let mut rects = Vec::new();
                        crate::tree::compute_rects(&w.root, app.last_window_area, &mut rects);
                        let (pane_x_offset, pane_y_offset) = rects.iter()
                            .find(|(path, _)| crate::tree::get_active_pane_id_at_path(&w.root, path) == Some(p.id))
                            .map(|(_, rect)| (rect.x, rect.y))
                            .unwrap_or((0, 0));
                        let row = app.last_mouse_y.saturating_sub(pane_y_offset);
                        let col = app.last_mouse_x.saturating_sub(pane_x_offset);
                        let mut row_text = String::with_capacity(cols as usize);
                        for c in 0..cols {
                            if let Some(cell) = screen.cell(row, c) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        let chars: Vec<char> = row_text.chars().collect();
                        let ci = col as usize;
                        if ci < chars.len() && !chars[ci].is_whitespace() {
                            let seps = &app.word_separators;
                            let cls = |ch: &char| -> u8 {
                                if ch.is_whitespace() { 0 }
                                else if seps.contains(*ch) { 1 }
                                else { 2 }
                            };
                            let target = cls(&chars[ci]);
                            let mut start = ci;
                            while start > 0 && cls(&chars[start - 1]) == target { start -= 1; }
                            let mut end = ci;
                            while end + 1 < chars.len() && cls(&chars[end + 1]) == target { end += 1; }
                            return chars[start..=end].iter().collect();
                        }
                    }
                }
            }
            String::new()
        }

        // ── Copy mode ──
        "copy_cursor_x" => app.copy_pos.map(|(_, c)| c.to_string()).unwrap_or("0".into()),
        "copy_cursor_y" => app.copy_pos.map(|(r, _)| r.to_string()).unwrap_or("0".into()),
        "copy_cursor_word" => {
            // Return the word under the copy cursor
            if let (Some((r, c)), Some(w)) = (app.copy_pos, app.windows.get(win_idx)) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        let mut row_text = String::with_capacity(cols as usize);
                        for col in 0..cols {
                            if let Some(cell) = screen.cell(r, col) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        let chars: Vec<char> = row_text.chars().collect();
                        let ci = c as usize;
                        if ci < chars.len() && !chars[ci].is_whitespace() {
                            let seps = &app.word_separators;
                            let cls = |ch: &char| -> u8 {
                                if ch.is_whitespace() { 0 }
                                else if seps.contains(*ch) { 1 }
                                else { 2 }
                            };
                            let target = cls(&chars[ci]);
                            let mut start = ci;
                            while start > 0 && cls(&chars[start - 1]) == target { start -= 1; }
                            let mut end = ci;
                            while end + 1 < chars.len() && cls(&chars[end + 1]) == target { end += 1; }
                            return chars[start..=end].iter().collect();
                        }
                    }
                }
            }
            String::new()
        }
        "copy_cursor_line" => {
            // Return the line under the copy cursor
            if let (Some((r, _)), Some(w)) = (app.copy_pos, app.windows.get(win_idx)) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        let mut row_text = String::with_capacity(cols as usize);
                        for col in 0..cols {
                            if let Some(cell) = screen.cell(r, col) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        return row_text.trim_end().to_string();
                    }
                }
            }
            String::new()
        }
        "selection_present" | "selection_active" => if app.copy_anchor.is_some() { "1".into() } else { "0".into() },
        "selection_start_x" => app.copy_anchor.map(|(_, c)| c.to_string()).unwrap_or("0".into()),
        "selection_start_y" => app.copy_anchor.map(|(r, _)| r.to_string()).unwrap_or("0".into()),
        "selection_end_x" => app.copy_pos.map(|(_, c)| c.to_string()).unwrap_or("0".into()),
        "selection_end_y" => app.copy_pos.map(|(r, _)| r.to_string()).unwrap_or("0".into()),
        "search_present" => if !app.copy_search_query.is_empty() { "1".into() } else { "0".into() },
        "search_match" => {
            if !app.copy_search_matches.is_empty() {
                app.copy_search_matches.get(app.copy_search_idx)
                    .map(|_| app.copy_search_query.clone())
                    .unwrap_or_default()
            } else { String::new() }
        }
        "scroll_position" => app.copy_scroll_offset.to_string(),
        "scroll_region_upper" => "0".into(),
        "scroll_region_lower" => {
            if let Some(p) = active_pane(&win.root, &win.active_path) {
                return p.last_rows.saturating_sub(1).to_string();
            }
            "0".into()
        }

        // ── Buffer ──
        "buffer_size" => {
            let idx = BUFFER_IDX_OVERRIDE.get().unwrap_or(0);
            app.paste_buffers.get(idx).map(|b| b.len().to_string()).unwrap_or("0".into())
        }
        "buffer_sample" => {
            let idx = BUFFER_IDX_OVERRIDE.get().unwrap_or(0);
            app.paste_buffers.get(idx).map(|b| b.chars().take(50).collect::<String>()).unwrap_or_default()
        }
        "buffer_name" => {
            let idx = BUFFER_IDX_OVERRIDE.get().unwrap_or(0);
            if idx < app.paste_buffers.len() { format!("buffer{:04}", idx) } else { String::new() }
        }
        "buffer_created" => app.created_at.timestamp().to_string(),

        // ── Client ──
        "client_width" => app.last_window_area.width.to_string(),
        "client_height" => (app.last_window_area.height + if app.status_visible { 1 } else { 0 }).to_string(),
        "client_session" | "client_last_session" => app.session_name.clone(),
        "client_name" | "client_tty" => "client0".into(),
        "client_pid" => std::process::id().to_string(),
        "client_prefix" => if app.client_prefix_active || matches!(app.mode, Mode::Prefix { .. }) { "1".into() } else { "0".into() },
        "client_activity" | "client_created" => app.created_at.timestamp().to_string(),
        "client_activity_string" | "client_created_string" => app.created_at.format("%a %b %e %H:%M:%S %Y").to_string(),
        "client_control_mode" => "0".into(),
        "client_flags" => "focused".into(),
        "client_key_table" => if app.client_prefix_active || matches!(app.mode, Mode::Prefix { .. }) {
            "prefix".into()
        } else {
            match app.mode {
                Mode::CopyMode => "copy-mode-vi".into(),
                _ => "root".into(),
            }
        },
        "client_termname" | "client_termtype" => env::var("TERM").unwrap_or_else(|_| "xterm-256color".into()),
        "client_termfeatures" => "256,RGB,title".into(),
        "client_utf8" => "1".into(),
        "client_cell_width" => "8".into(),
        "client_cell_height" => "16".into(),
        "client_written" | "client_discarded" => "0".into(),

        // ── Server ──
        "host" | "hostname" => hostname_cached(),
        "host_short" => { let h = hostname_cached(); h.split('.').next().unwrap_or(&h).to_string() }
        "user" | "username" => env::var("USERNAME").or_else(|_| env::var("USER")).unwrap_or_else(|_| "unknown".into()),
        "pid" | "server_pid" => std::process::id().to_string(),
        "version" => VERSION.to_string(),
        "start_time" => app.created_at.timestamp().to_string(),
        "socket_path" => {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            format!("{}/.psmux/default", home)
        }

        // ── Options as format variables ──
        "mouse" => if app.mouse_enabled { "on".into() } else { "off".into() },
        "scroll-enter-copy-mode" => if app.scroll_enter_copy_mode { "on".into() } else { "off".into() },
        "prefix" => format_key_binding(&app.prefix_key),
        "prefix2" => app.prefix2_key.as_ref().map(|k| format_key_binding(k)).unwrap_or_else(|| "none".to_string()),
        "status" => if app.status_visible { "on".into() } else { "off".into() },
        "mode_keys" => app.mode_keys.clone(),
        "history_limit" => app.history_limit.to_string(),
        "history_size" => app.history_limit.to_string(),
        "alternate_on" => {
            if let Some(p) = active_pane(&win.root, &win.active_path) {
                if let Ok(parser) = p.term.lock() {
                    if parser.screen().alternate_screen() { return "1".into(); }
                }
            }
            "0".into()
        }
        "alternate_saved_x" | "alternate_saved_y" => "0".into(),

        // ── Misc ──
        "origin_flag" | "insert_flag" | "keypad_cursor_flag" | "keypad_flag" => "0".into(),
        "wrap_flag" => "1".into(),
        "line" | "command" | "command_list_name" | "command_list_alias" | "command_list_usage" | "config_files" => String::new(),
        "current_file" => crate::config::current_config_file(),

        // Anything else: try as option, then env
        _ => {
            if let Some(val) = lookup_option(var, app) { val }
            else { String::new() }
        }
    }
}
