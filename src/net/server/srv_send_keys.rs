use super::*;

/// Handle CtrlReq::SendKeys (non-literal and literal key sending).
pub(crate) fn handle_send_keys(app: &mut AppState, keys: String, literal: bool) -> io::Result<()> {
    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
    if in_copy {
        if literal {
            send_text_to_active(app, &keys)?;
        } else {
            let parts: Vec<&str> = keys.split_whitespace().collect();
            for key in parts.iter() {
                let key_upper = key.to_uppercase();
                let normalized = match key_upper.as_str() {
                    "ENTER" => "enter", "TAB" => "tab", "BTAB" | "BACKTAB" => "btab",
                    "ESCAPE" | "ESC" => "esc", "SPACE" => "space", "BSPACE" | "BACKSPACE" => "backspace",
                    "UP" => "up", "DOWN" => "down", "RIGHT" => "right", "LEFT" => "left",
                    "HOME" => "home", "END" => "end", "PAGEUP" | "PPAGE" => "pageup",
                    "PAGEDOWN" | "NPAGE" => "pagedown", "DELETE" | "DC" => "delete",
                    "INSERT" | "IC" => "insert", _ => "",
                };
                if !normalized.is_empty() {
                    send_key_to_active(app, normalized)?;
                } else if key_upper.starts_with("C-") || key_upper.starts_with("M-") || (key_upper.starts_with("F") && key_upper.len() >= 2 && key_upper[1..].chars().all(|c| c.is_ascii_digit())) {
                    send_key_to_active(app, &key.to_lowercase())?;
                } else {
                    send_text_to_active(app, key)?;
                }
            }
        }
    } else if literal {
        send_text_to_active(app, &keys)?;
    } else {
        let parts: Vec<&str> = keys.split_whitespace().collect();
        for (i, key) in parts.iter().enumerate() {
            let key_upper = key.to_uppercase();
            match key_upper.as_str() {
                "ENTER" => send_text_to_active(app, "\r")?,
                "TAB" => send_text_to_active(app, "\t")?,
                "BTAB" | "BACKTAB" => send_text_to_active(app, "\x1b[Z")?,
                "ESCAPE" | "ESC" => send_text_to_active(app, "\x1b")?,
                "SPACE" => send_text_to_active(app, " ")?,
                "BSPACE" | "BACKSPACE" => send_text_to_active(app, "\x7f")?,
                "UP" => send_text_to_active(app, "\x1b[A")?,
                "DOWN" => send_text_to_active(app, "\x1b[B")?,
                "RIGHT" => send_text_to_active(app, "\x1b[C")?,
                "LEFT" => send_text_to_active(app, "\x1b[D")?,
                "HOME" => send_text_to_active(app, "\x1b[H")?,
                "END" => send_text_to_active(app, "\x1b[F")?,
                "PAGEUP" | "PPAGE" => send_text_to_active(app, "\x1b[5~")?,
                "PAGEDOWN" | "NPAGE" => send_text_to_active(app, "\x1b[6~")?,
                "DELETE" | "DC" => send_text_to_active(app, "\x1b[3~")?,
                "INSERT" | "IC" => send_text_to_active(app, "\x1b[2~")?,
                "F1" => send_text_to_active(app, "\x1bOP")?,
                "F2" => send_text_to_active(app, "\x1bOQ")?,
                "F3" => send_text_to_active(app, "\x1bOR")?,
                "F4" => send_text_to_active(app, "\x1bOS")?,
                "F5" => send_text_to_active(app, "\x1b[15~")?,
                "F6" => send_text_to_active(app, "\x1b[17~")?,
                "F7" => send_text_to_active(app, "\x1b[18~")?,
                "F8" => send_text_to_active(app, "\x1b[19~")?,
                "F9" => send_text_to_active(app, "\x1b[20~")?,
                "F10" => send_text_to_active(app, "\x1b[21~")?,
                "F11" => send_text_to_active(app, "\x1b[23~")?,
                "F12" => send_text_to_active(app, "\x1b[24~")?,
                s if crate::input::parse_modified_special_key(s).is_some() => {
                    let seq = crate::input::parse_modified_special_key(s).unwrap();
                    send_text_to_active(app, &seq)?;
                }
                s if s.starts_with("C-M-") || s.starts_with("C-m-") => {
                    if let Some(c) = key.chars().nth(4) {
                        let ctrl = (c.to_ascii_lowercase() as u8) & 0x1F;
                        send_text_to_active(app, &format!("\x1b{}", ctrl as char))?;
                    }
                }
                s if s.starts_with("C-") => {
                    if let Some(c) = s.chars().nth(2) {
                        let ctrl = (c.to_ascii_lowercase() as u8) & 0x1F;
                        send_text_to_active(app, &String::from(ctrl as char))?;
                        #[cfg(windows)]
                        if ctrl == 0x03 {
                            if let Some(win) = app.windows.get_mut(app.active_idx) {
                                if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                                    if p.child_pid.is_none() { p.child_pid = crate::platform::mouse_inject::get_child_pid(&*p.child); }
                                    if let Some(pid) = p.child_pid { crate::platform::send_ctrl_c_event(pid, false); }
                                }
                            }
                        }
                    }
                }
                s if s.starts_with("M-") => {
                    if let Some(c) = key.chars().nth(2) {
                        send_text_to_active(app, &format!("\x1b{}", c))?;
                    }
                }
                _ => {
                    send_text_to_active(app, key)?;
                    if i + 1 < parts.len() {
                        let next_upper = parts[i + 1].to_uppercase();
                        let next_is_special = matches!(next_upper.as_str(),
                            "ENTER" | "TAB" | "BTAB" | "BACKTAB" | "ESCAPE" | "ESC" | "SPACE" | "BSPACE" | "BACKSPACE" |
                            "UP" | "DOWN" | "RIGHT" | "LEFT" | "HOME" | "END" |
                            "PAGEUP" | "PPAGE" | "PAGEDOWN" | "NPAGE" | "DELETE" | "DC" | "INSERT" | "IC" |
                            "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
                        ) || next_upper.starts_with("C-") || next_upper.starts_with("M-") || next_upper.starts_with("S-");
                        if !next_is_special { send_text_to_active(app, " ")?; }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Handle CtrlReq::SendKeysX (copy-mode X commands).
pub(crate) fn handle_send_keys_x(app: &mut AppState, cmd: String) -> io::Result<Option<&'static str>> {
    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
    if !in_copy { enter_copy_mode(app); }
    match cmd.as_str() {
        "cancel" => {
            app.mode = Mode::Passthrough;
            app.copy_anchor = None; app.copy_pos = None; app.copy_scroll_offset = 0;
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                if let Ok(mut parser) = p.term.lock() { parser.screen_mut().set_scrollback(0); }
            }
            fire_mode_changed_hooks(app);
        }
        "begin-selection" => {
            if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r,c)); app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r,c)); app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
        }
        "select-line" => {
            if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r,c)); app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r,c)); app.copy_selection_mode = crate::types::SelectionMode::Line;
            }
        }
        "rectangle-toggle" => {
            app.copy_selection_mode = match app.copy_selection_mode {
                crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                _ => crate::types::SelectionMode::Rect,
            };
        }
        "copy-selection" => { yank_and_fire_clipboard(app); }
        "copy-selection-and-cancel" => {
            yank_and_fire_clipboard(app);
            app.mode = Mode::Passthrough; app.copy_scroll_offset = 0; app.copy_pos = None;
            fire_mode_changed_hooks(app);
        }
        "copy-selection-no-clear" => { yank_and_fire_clipboard(app); }
        s if s.starts_with("copy-pipe-and-cancel") || s.starts_with("copy-pipe") => {
            yank_and_fire_clipboard(app);
            let cancel = s.contains("cancel");
            let pipe_cmd = cmd.strip_prefix("copy-pipe-and-cancel").or_else(|| cmd.strip_prefix("copy-pipe")).unwrap_or("").trim();
            if !pipe_cmd.is_empty() {
                if let Some(text) = app.paste_buffers.first().cloned() {
                    let mut copy_pipe_cmd = std::process::Command::new(if cfg!(windows) { "pwsh" } else { "sh" });
                    copy_pipe_cmd.args(if cfg!(windows) { vec!["-NoProfile", "-Command", pipe_cmd] } else { vec!["-c", pipe_cmd] })
                        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                    { use crate::platform::HideWindowCommandExt; copy_pipe_cmd.hide_window(); }
                    if let Ok(mut child) = copy_pipe_cmd.spawn() {
                        if let Some(mut stdin) = child.stdin.take() { use std::io::Write; let _ = stdin.write_all(text.as_bytes()); }
                        let _ = child.wait();
                    }
                }
            }
            if cancel {
                app.mode = Mode::Passthrough; app.copy_scroll_offset = 0; app.copy_pos = None;
                fire_mode_changed_hooks(app);
            }
        }
        "cursor-up" => { move_copy_cursor(app, 0, -1); }
        "cursor-down" => { move_copy_cursor(app, 0, 1); }
        "cursor-left" => { move_copy_cursor(app, -1, 0); }
        "cursor-right" => { move_copy_cursor(app, 1, 0); }
        "start-of-line" => { crate::copy_mode::move_to_line_start(app); }
        "end-of-line" => { crate::copy_mode::move_to_line_end(app); }
        "back-to-indentation" => { crate::copy_mode::move_to_first_nonblank(app); }
        "next-word" => { crate::copy_mode::move_word_forward(app); }
        "previous-word" => { crate::copy_mode::move_word_backward(app); }
        "next-word-end" => { crate::copy_mode::move_word_end(app); }
        "next-space" => { crate::copy_mode::move_word_forward_big(app); }
        "previous-space" => { crate::copy_mode::move_word_backward_big(app); }
        "next-space-end" => { crate::copy_mode::move_word_end_big(app); }
        "top-line" => { crate::copy_mode::move_to_screen_top(app); }
        "middle-line" => { crate::copy_mode::move_to_screen_middle(app); }
        "bottom-line" => { crate::copy_mode::move_to_screen_bottom(app); }
        "history-top" => { crate::copy_mode::scroll_to_top(app); }
        "history-bottom" => { crate::copy_mode::scroll_to_bottom(app); }
        "halfpage-up" => {
            let half = app.windows.get(app.active_idx).and_then(|w| active_pane(&w.root, &w.active_path)).map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
            scroll_copy_up(app, half);
        }
        "halfpage-down" => {
            let half = app.windows.get(app.active_idx).and_then(|w| active_pane(&w.root, &w.active_path)).map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
            scroll_copy_down(app, half);
        }
        "page-up" => { scroll_copy_up(app, 20); }
        "page-down" => { scroll_copy_down(app, 20); }
        "scroll-up" => { scroll_copy_up(app, 1); }
        "scroll-down" => { scroll_copy_down(app, 1); }
        "search-forward" | "search-forward-incremental" => { app.mode = Mode::CopySearch { input: String::new(), forward: true }; }
        "search-backward" | "search-backward-incremental" => { app.mode = Mode::CopySearch { input: String::new(), forward: false }; }
        "search-again" => { crate::copy_mode::search_next(app); }
        "search-reverse" => { crate::copy_mode::search_prev(app); }
        "copy-end-of-line" => { let _ = crate::copy_mode::copy_end_of_line(app); app.mode = Mode::Passthrough; app.copy_scroll_offset = 0; app.copy_pos = None; }
        "select-word" => {
            crate::copy_mode::move_word_backward(app);
            if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r,c)); app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
            crate::copy_mode::move_word_end(app);
        }
        "other-end" => {
            if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                app.copy_anchor = Some(p); app.copy_anchor_scroll_offset = app.copy_scroll_offset; app.copy_pos = Some(a);
            }
        }
        "clear-selection" => { app.copy_anchor = None; app.copy_selection_mode = crate::types::SelectionMode::Char; }
        "append-selection" => {
            yank_and_fire_clipboard(app);
            if app.paste_buffers.len() >= 2 { let appended = format!("{}{}", app.paste_buffers[1], app.paste_buffers[0]); app.paste_buffers[0] = appended; }
        }
        "append-selection-and-cancel" => {
            yank_and_fire_clipboard(app);
            if app.paste_buffers.len() >= 2 { let appended = format!("{}{}", app.paste_buffers[1], app.paste_buffers[0]); app.paste_buffers[0] = appended; }
            app.mode = Mode::Passthrough; app.copy_scroll_offset = 0; app.copy_pos = None;
            fire_mode_changed_hooks(app);
        }
        "copy-line" => {
            if let Some((r, _)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, 0)); app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_selection_mode = crate::types::SelectionMode::Line;
                let cols = app.windows.get(app.active_idx).and_then(|w| active_pane(&w.root, &w.active_path)).map(|p| p.last_cols).unwrap_or(80);
                app.copy_pos = Some((r, cols.saturating_sub(1)));
                yank_and_fire_clipboard(app);
            }
            app.mode = Mode::Passthrough; app.copy_scroll_offset = 0; app.copy_pos = None;
            fire_mode_changed_hooks(app);
        }
        s if s.starts_with("goto-line") => {
            let n = s.strip_prefix("goto-line").unwrap_or("").trim().parse::<u16>().unwrap_or(0);
            app.copy_pos = Some((n, 0));
        }
        "jump-forward" => { app.copy_find_char_pending = Some(0); }
        "jump-backward" => { app.copy_find_char_pending = Some(1); }
        "jump-to-forward" => { app.copy_find_char_pending = Some(2); }
        "jump-to-backward" => { app.copy_find_char_pending = Some(3); }
        "jump-again" => {}
        "jump-reverse" => {}
        "next-paragraph" => { crate::copy_mode::move_next_paragraph(app); }
        "previous-paragraph" => { crate::copy_mode::move_prev_paragraph(app); }
        "next-matching-bracket" => { crate::copy_mode::move_matching_bracket(app); }
        "stop-selection" => { app.copy_anchor = None; }
        _ => {}
    }
    Ok(None)
}

fn yank_and_fire_clipboard(app: &mut AppState) {
    let _ = yank_selection(app);
    if let Some(cmds) = app.hooks.get("pane-set-clipboard") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
}

fn fire_mode_changed_hooks(app: &mut AppState) {
    if let Some(cmds) = app.hooks.get("pane-mode-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
}
