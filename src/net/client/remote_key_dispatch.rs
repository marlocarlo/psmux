use super::*;
use super::run_remote_state::RunRemoteState;

/// Handle non-prefix key dispatch: chooser navigation, rename/command input,
/// and normal key forwarding to the PTY.
///
/// Returns `true` if the key triggered quit (detach/kill-session).
pub(crate) fn handle_nonprefix_keys(
    state: &mut RunRemoteState,
    key: &crossterm::event::KeyEvent,
    cmd_batch: &mut Vec<String>,
    home: &str,
    current_session: &str,
) -> bool {
    let mut quit = false;

    match key.code {
        // ── Session chooser navigation ──────────────────────────────
        KeyCode::Up if state.session_chooser => { if state.session_selected > 0 { state.session_selected -= 1; } }
        KeyCode::Down if state.session_chooser => { if state.session_selected + 1 < state.session_entries.len() { state.session_selected += 1; } }
        KeyCode::PageUp if state.session_chooser => { state.session_selected = state.session_selected.saturating_sub(10); }
        KeyCode::PageDown if state.session_chooser => { state.session_selected = (state.session_selected + 10).min(state.session_entries.len().saturating_sub(1)); }
        KeyCode::Home if state.session_chooser => { state.session_selected = 0; }
        KeyCode::End if state.session_chooser => { state.session_selected = state.session_entries.len().saturating_sub(1); }
        KeyCode::Enter if state.session_chooser => {
            if let Some((sname, _)) = state.session_entries.get(state.session_selected) {
                if sname != current_session {
                    cmd_batch.push("client-detach\n".into());
                    env::set_var("PSMUX_SWITCH_TO", sname);
                    quit = true;
                }
                state.session_chooser = false;
            }
        }
        KeyCode::Esc if state.session_chooser => { state.session_chooser = false; }
        KeyCode::Char('x') if state.session_chooser => {
            // Kill the selected session (like tmux session chooser)
            if let Some((sname, _)) = state.session_entries.get(state.session_selected) {
                let sname = sname.clone();
                if sname == current_session {
                    cmd_batch.push("kill-session\n".into());
                    state.session_chooser = false;
                    quit = true;
                } else {
                    super::remote_key_helpers::kill_remote_session(&sname);
                    state.session_entries.remove(state.session_selected);
                    if state.session_selected >= state.session_entries.len() && state.session_selected > 0 {
                        state.session_selected -= 1;
                    }
                    if state.session_entries.is_empty() {
                        state.session_chooser = false;
                    }
                }
            }
        }

        // ── Tree chooser navigation ─────────────────────────────────
        KeyCode::Up if state.tree_chooser => { if state.tree_selected > 0 { state.tree_selected -= 1; } }
        KeyCode::Down if state.tree_chooser => { if state.tree_selected + 1 < state.tree_entries.len() { state.tree_selected += 1; } }
        KeyCode::Enter if state.tree_chooser => {
            if let Some((is_win, wid, pid, _label, sess_name)) = state.tree_entries.get(state.tree_selected) {
                if *wid == usize::MAX {
                    if *sess_name != current_session {
                        cmd_batch.push("client-detach\n".into());
                        env::set_var("PSMUX_SWITCH_TO", sess_name);
                        quit = true;
                    }
                    state.tree_chooser = false;
                } else if *sess_name != current_session {
                    cmd_batch.push("client-detach\n".into());
                    env::set_var("PSMUX_SWITCH_TO", sess_name);
                    quit = true;
                    state.tree_chooser = false;
                } else if *is_win {
                    cmd_batch.push(format!("focus-window {}\n", wid));
                    state.tree_chooser = false;
                } else {
                    cmd_batch.push(format!("focus-pane {}\n", pid));
                    state.tree_chooser = false;
                }
            }
        }
        KeyCode::Esc if state.tree_chooser => { state.tree_chooser = false; }

        // ── Buffer chooser navigation ───────────────────────────────
        KeyCode::Up | KeyCode::Char('k') if state.buffer_chooser => {
            if state.buffer_selected > 0 { state.buffer_selected -= 1; }
        }
        KeyCode::Down | KeyCode::Char('j') if state.buffer_chooser => {
            if state.buffer_selected + 1 < state.buffer_entries.len() { state.buffer_selected += 1; }
        }
        KeyCode::Enter if state.buffer_chooser => {
            if state.buffer_selected < state.buffer_entries.len() {
                let (idx, _, _) = &state.buffer_entries[state.buffer_selected];
                cmd_batch.push(format!("paste-buffer-at {}\n", idx));
            }
            state.buffer_chooser = false;
        }
        KeyCode::Char('d') | KeyCode::Delete if state.buffer_chooser => {
            if state.buffer_selected < state.buffer_entries.len() {
                let (idx, _, _) = &state.buffer_entries[state.buffer_selected];
                cmd_batch.push(format!("delete-buffer-at {}\n", idx));
                state.buffer_entries.remove(state.buffer_selected);
                for (i, entry) in state.buffer_entries.iter_mut().enumerate() {
                    entry.0 = i;
                }
                if state.buffer_selected >= state.buffer_entries.len() && state.buffer_selected > 0 {
                    state.buffer_selected -= 1;
                }
                if state.buffer_entries.is_empty() {
                    state.buffer_chooser = false;
                }
            }
        }
        KeyCode::Esc | KeyCode::Char('q') if state.buffer_chooser => { state.buffer_chooser = false; }

        // ── Keys viewer ─────────────────────────────────────────────
        KeyCode::Up if state.keys_viewer => { if state.keys_viewer_scroll > 0 { state.keys_viewer_scroll -= 1; } }
        KeyCode::Down if state.keys_viewer => { state.keys_viewer_scroll += 1; }
        KeyCode::PageUp if state.keys_viewer => { state.keys_viewer_scroll = state.keys_viewer_scroll.saturating_sub(20); }
        KeyCode::PageDown if state.keys_viewer => { state.keys_viewer_scroll += 20; }
        KeyCode::Home if state.keys_viewer => { state.keys_viewer_scroll = 0; }
        KeyCode::End if state.keys_viewer => { state.keys_viewer_scroll = state.keys_viewer_lines.len().saturating_sub(1); }
        KeyCode::Char('q') if state.keys_viewer => { state.keys_viewer = false; }
        KeyCode::Esc if state.keys_viewer => { state.keys_viewer = false; }
        KeyCode::Char('k') if state.keys_viewer => { if state.keys_viewer_scroll > 0 { state.keys_viewer_scroll -= 1; } }
        KeyCode::Char('j') if state.keys_viewer => { state.keys_viewer_scroll += 1; }

        // ── Kill confirmation ───────────────────────────────────────
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter if state.confirm_cmd.is_some() => {
            if let Some(cmd) = state.confirm_cmd.take() {
                cmd_batch.push(format!("{}\n", cmd));
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc if state.confirm_cmd.is_some() => {
            state.confirm_cmd = None;
        }

        // ── Rename input ────────────────────────────────────────────
        KeyCode::Char(c) if state.renaming && !key.modifiers.contains(KeyModifiers::CONTROL) => { state.rename_buf.push(c); }
        KeyCode::Char(c) if state.pane_renaming && !key.modifiers.contains(KeyModifiers::CONTROL) => { state.pane_title_buf.push(c); }
        KeyCode::Char(c) if state.window_idx_input && c.is_ascii_digit() => { state.window_idx_buf.push(c); }
        KeyCode::Char(c) if state.command_input && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.command_buf.insert(state.command_cursor, c); state.command_cursor += 1;
        }
        KeyCode::Backspace if state.renaming => { let _ = state.rename_buf.pop(); }
        KeyCode::Backspace if state.pane_renaming => { let _ = state.pane_title_buf.pop(); }
        KeyCode::Backspace if state.window_idx_input => { let _ = state.window_idx_buf.pop(); }
        KeyCode::Backspace if state.command_input => { if state.command_cursor > 0 { state.command_buf.remove(state.command_cursor - 1); state.command_cursor -= 1; } }
        KeyCode::Enter if state.renaming => {
            if state.session_renaming {
                cmd_batch.push(format!("rename-session {}\n", quote_arg(&state.rename_buf)));
                state.session_renaming = false;
            } else {
                cmd_batch.push(format!("rename-window {}\n", quote_arg(&state.rename_buf)));
            }
            state.renaming = false;
        }
        KeyCode::Enter if state.pane_renaming => { cmd_batch.push(format!("set-pane-title {}\n", quote_arg(&state.pane_title_buf))); state.pane_renaming = false; }
        KeyCode::Enter if state.window_idx_input => {
            if !state.window_idx_buf.is_empty() {
                cmd_batch.push(format!("select-window -t :{}\n", state.window_idx_buf));
            }
            state.window_idx_input = false;
        }
        KeyCode::Enter if state.command_input => {
            let trimmed = state.command_buf.trim().to_string();
            if !trimmed.is_empty() {
                state.command_history.push(trimmed.clone());
                state.command_history_idx = state.command_history.len();
                let first_word = trimmed.split_whitespace().next().unwrap_or("");
                if first_word == "choose-buffer" || first_word == "chooseb" {
                    super::remote_chooser::populate_choose_buffer(state, home, current_session);
                } else {
                    let sub_cmds = crate::config::split_chained_commands_pub(&trimmed);
                    for sub in &sub_cmds {
                        cmd_batch.push(format!("{}\n", sub));
                    }
                }
            }
            state.command_input = false;
            state.command_cursor = 0;
        }
        KeyCode::Esc if state.renaming => { state.renaming = false; state.session_renaming = false; }
        KeyCode::Esc if state.pane_renaming => { state.pane_renaming = false; }
        KeyCode::Esc if state.window_idx_input => { state.window_idx_input = false; }
        KeyCode::Esc if state.command_input => { state.command_input = false; state.command_cursor = 0; }

        // ── Command prompt editing keys ─────────────────────────────
        KeyCode::Left if state.command_input => { if state.command_cursor > 0 { state.command_cursor -= 1; } }
        KeyCode::Right if state.command_input => { if state.command_cursor < state.command_buf.len() { state.command_cursor += 1; } }
        KeyCode::Home if state.command_input => { state.command_cursor = 0; }
        KeyCode::End if state.command_input => { state.command_cursor = state.command_buf.len(); }
        KeyCode::Delete if state.command_input => { if state.command_cursor < state.command_buf.len() { state.command_buf.remove(state.command_cursor); } }
        KeyCode::Up if state.command_input => {
            if state.command_history_idx > 0 {
                state.command_history_idx -= 1;
                state.command_buf = state.command_history[state.command_history_idx].clone();
                state.command_cursor = state.command_buf.len();
            }
        }
        KeyCode::Down if state.command_input => {
            if state.command_history_idx < state.command_history.len() {
                state.command_history_idx += 1;
                state.command_buf = if state.command_history_idx < state.command_history.len() {
                    state.command_history[state.command_history_idx].clone()
                } else {
                    String::new()
                };
                state.command_cursor = state.command_buf.len();
            }
        }
        KeyCode::Char('a') if state.command_input && key.modifiers.contains(KeyModifiers::CONTROL) => { state.command_cursor = 0; }
        KeyCode::Char('e') if state.command_input && key.modifiers.contains(KeyModifiers::CONTROL) => { state.command_cursor = state.command_buf.len(); }
        KeyCode::Char('u') if state.command_input && key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.command_buf.drain(..state.command_cursor);
            state.command_cursor = 0;
        }
        KeyCode::Char('k') if state.command_input && key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.command_buf.truncate(state.command_cursor);
        }
        KeyCode::Char('w') if state.command_input && key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut pos = state.command_cursor;
            while pos > 0 && state.command_buf.as_bytes().get(pos - 1) == Some(&b' ') { pos -= 1; }
            while pos > 0 && state.command_buf.as_bytes().get(pos - 1) != Some(&b' ') { pos -= 1; }
            state.command_buf.drain(pos..state.command_cursor);
            state.command_cursor = pos;
        }

        // ── Normal key forwarding ───────────────────────────────────
        KeyCode::Char(' ') => {
            #[cfg(windows)]
            {
                state.paste_pend.push(' ');
                if state.paste_pend_start.is_none() {
                    state.paste_pend_start = Some(Instant::now());
                }
            }
            #[cfg(not(windows))]
            {
                cmd_batch.push("send-key space\n".into());
            }
        }
        // AltGr detection
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::ALT)
            && !c.is_ascii_lowercase() => {
            #[cfg(windows)]
            {
                state.paste_pend.push(c);
                if state.paste_pend_start.is_none() {
                    state.paste_pend_start = Some(Instant::now());
                }
            }
            #[cfg(not(windows))]
            {
                let escaped = match c {
                    '"' => "\\\"".to_string(),
                    '\\' => "\\\\".to_string(),
                    _ => c.to_string(),
                };
                cmd_batch.push(format!("send-text \"{}\"\n", escaped));
            }
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::ALT) => {
            cmd_batch.push(format!("send-key C-M-{}\n", c.to_ascii_lowercase()));
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
            cmd_batch.push(format!("send-key M-{}\n", c));
        }
        // pwsh-mouse-selection: Ctrl+Shift+C copy
        KeyCode::Char('C') if state.client_pwsh_selection
            && key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            super::remote_key_helpers::copy_and_clear_selection(state);
        }
        // pwsh-mouse-selection: Ctrl+Shift+V paste
        KeyCode::Char('V') if state.client_pwsh_selection
            && key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            if let Some(text) = read_from_system_clipboard() {
                if !text.is_empty() {
                    let encoded = base64_encode(&text);
                    cmd_batch.push(format!("send-paste {}\n", encoded));
                }
            }
        }
        // Ctrl+C smart: copy selection or SIGINT
        KeyCode::Char('c') if state.client_pwsh_selection
            && key.kind == KeyEventKind::Press
            && key.modifiers == KeyModifiers::CONTROL
            && state.rsel_dragged
            && state.rsel_start.is_some() =>
        {
            super::remote_key_helpers::copy_and_clear_selection(state);
        }
        // On Windows, suppress Ctrl+V Press
        #[cfg(windows)]
        KeyCode::Char('v') if key.modifiers == KeyModifiers::CONTROL => {}
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            cmd_batch.push(format!("send-key C-{}\n", c.to_ascii_lowercase()));
        }
        KeyCode::Char(c) if (c as u32) >= 0x01 && (c as u32) <= 0x1A => {
            let ctrl_letter = ((c as u8) + b'a' - 1) as char;
            cmd_batch.push(format!("send-key C-{}\n", ctrl_letter));
        }
        KeyCode::Char(c) => {
            #[cfg(windows)]
            {
                let suppressed = state.paste_suppress_until
                    .map_or(false, |t| Instant::now() < t);
                if suppressed {
                    if input_log_enabled() {
                        input_log("paste", &format!("suppressed char '{}' during paste suppress window", c));
                    }
                } else {
                    state.paste_suppress_until = None;
                    state.paste_pend.push(c);
                    if state.paste_pend_start.is_none() {
                        state.paste_pend_start = Some(Instant::now());
                    }
                }
            }
            #[cfg(not(windows))]
            {
                let escaped = match c {
                    '"' => "\\\"".to_string(),
                    '\\' => "\\\\".to_string(),
                    _ => c.to_string(),
                };
                cmd_batch.push(format!("send-text \"{}\"\n", escaped));
            }
        }
        KeyCode::Enter => {
            #[cfg(windows)]
            {
                if !state.paste_pend.is_empty() {
                    state.paste_pend.push('\n');
                } else {
                    cmd_batch.push(format!("send-key {}\n", modified_key_name("Enter", key.modifiers)));
                }
            }
            #[cfg(not(windows))]
            { cmd_batch.push(format!("send-key {}\n", modified_key_name("Enter", key.modifiers))); }
        }
        KeyCode::Tab => {
            #[cfg(windows)]
            {
                if !state.paste_pend.is_empty() {
                    state.paste_pend.push('\t');
                } else {
                    cmd_batch.push("send-key tab\n".into());
                }
            }
            #[cfg(not(windows))]
            { cmd_batch.push("send-key tab\n".into()); }
        }
        KeyCode::BackTab => { cmd_batch.push("send-key btab\n".into()); }
        KeyCode::Backspace => { cmd_batch.push("send-key backspace\n".into()); }
        KeyCode::Delete => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Delete", key.modifiers))); }
        KeyCode::Esc => { cmd_batch.push("send-key esc\n".into()); }
        KeyCode::Left => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Left", key.modifiers))); }
        KeyCode::Right => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Right", key.modifiers))); }
        KeyCode::Up => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Up", key.modifiers))); }
        KeyCode::Down => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Down", key.modifiers))); }
        KeyCode::PageUp => { cmd_batch.push(format!("send-key {}\n", modified_key_name("PageUp", key.modifiers))); }
        KeyCode::PageDown => { cmd_batch.push(format!("send-key {}\n", modified_key_name("PageDown", key.modifiers))); }
        KeyCode::Home => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Home", key.modifiers))); }
        KeyCode::End => { cmd_batch.push(format!("send-key {}\n", modified_key_name("End", key.modifiers))); }
        KeyCode::Insert => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Insert", key.modifiers))); }
        KeyCode::F(n) => { cmd_batch.push(format!("send-key {}\n", modified_key_name(&format!("F{}", n), key.modifiers))); }
        _ => {}
    }

    quit
}
