use super::*;

pub(crate) fn handle_key_passthrough(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    // Check switch-client -T key table first
    if let Some(table_name) = app.current_key_table.take() {
        let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
        if let Some(bind) = app.key_tables.get(&table_name)
            .and_then(|t| t.iter().find(|b| b.key == key_tuple))
            .cloned()
        {
            return execute_action(app, &bind.action);
        }
        // Key not found in table — fall through to normal dispatch
    }
    let is_prefix = (key.code, key.modifiers) == app.prefix_key
        || matches!(key.code, KeyCode::Char(c) if c == '\u{0002}')
        || app.prefix2_key.map_or(false, |p2| (key.code, key.modifiers) == p2);
    if is_prefix {
        app.mode = Mode::Prefix { armed_at: Instant::now() };
        app.prefix_repeating = false;
        return Ok(false);
    }
    // Check root key table for bindings (bind-key -n / bind-key -T root)
    let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
    if let Some(bind) = app.key_tables.get("root").and_then(|t| t.iter().find(|b| b.key == key_tuple)).cloned() {
        return execute_action(app, &bind.action);
    }
    forward_key_to_active(app, key)?;
    Ok(false)
}

pub(crate) fn handle_key_window_chooser(app: &mut AppState, key: KeyEvent, selected: usize) -> io::Result<bool> {
    let tree_len = if let Mode::WindowChooser { ref tree, .. } = app.mode { tree.len() } else { 0 };
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Passthrough; }
        KeyCode::Up | KeyCode::Char('k') => {
            if selected > 0 { if let Mode::WindowChooser { selected: s, .. } = &mut app.mode { *s -= 1; } }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if selected + 1 < tree_len { if let Mode::WindowChooser { selected: s, .. } = &mut app.mode { *s += 1; } }
        }
        KeyCode::Enter => {
            if let Mode::WindowChooser { selected: s, ref tree } = &app.mode {
                let entry = &tree[*s];
                if entry.is_current_session {
                    // Same session: switch window directly
                    if let Some(wi) = entry.window_index {
                        app.last_window_idx = app.active_idx;
                        app.active_idx = wi;
                    }
                } else {
                    // Different session: set env and trigger switch
                    std::env::set_var("PSMUX_SWITCH_TO", &entry.session_name);
                }
            }
            app.mode = Mode::Passthrough;
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // Quick-select by window number
            let n = c.to_digit(10).unwrap_or(0) as usize;
            if let Mode::WindowChooser { ref tree, .. } = app.mode {
                if let Some(idx) = tree.iter().position(|e| !e.is_session_header && e.window_index == Some(n) && e.is_current_session) {
                    if let Mode::WindowChooser { selected: s, .. } = &mut app.mode { *s = idx; }
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

pub(crate) fn handle_key_window_index_prompt(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => { app.mode = Mode::Passthrough; }
        KeyCode::Enter => {
            if let Mode::WindowIndexPrompt { input } = &app.mode {
                if let Ok(idx) = input.parse::<usize>() {
                    if idx >= app.window_base_index {
                        let internal_idx = idx - app.window_base_index;
                        if internal_idx < app.windows.len() {
                            switch_with_copy_save(app, |app| {
                                app.last_window_idx = app.active_idx;
                                app.active_idx = internal_idx;
                            });
                        }
                    }
                }
            }
            app.mode = Mode::Passthrough;
        }
        KeyCode::Backspace => { if let Mode::WindowIndexPrompt { input } = &mut app.mode { let _ = input.pop(); } }
        KeyCode::Char(c) if c.is_ascii_digit() => { if let Mode::WindowIndexPrompt { input } = &mut app.mode { input.push(c); } }
        _ => {}
    }
    Ok(false)
}

pub(crate) fn handle_key_rename_prompt(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => { app.mode = Mode::Passthrough; }
        KeyCode::Enter => {
            if let Mode::RenamePrompt { input } = &mut app.mode {
                let name = input.clone();
                app.mode = Mode::Passthrough;
                // Update local state with bounds check
                if app.active_idx < app.windows.len() {
                    app.windows[app.active_idx].name = name.clone();
                    app.windows[app.active_idx].manual_rename = true;
                }
                // Forward to server so external queries see the new name
                if let Some(port) = app.control_port {
                    let _ = crate::session::send_control_to_port(port, &format!("rename-window {}\n", crate::util::quote_arg(&name)), &app.session_key);
                }
            }
        }
        KeyCode::Backspace => { if let Mode::RenamePrompt { input } = &mut app.mode { let _ = input.pop(); } }
        KeyCode::Char(c) => { if let Mode::RenamePrompt { input } = &mut app.mode { input.push(c); } }
        _ => {}
    }
    Ok(false)
}

pub(crate) fn handle_key_rename_session_prompt(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => { app.mode = Mode::Passthrough; }
        KeyCode::Enter => {
            if let Mode::RenameSessionPrompt { input } = &mut app.mode {
                let name = input.clone();
                app.mode = Mode::Passthrough;
                // Update local state
                app.session_name = name.clone();
                // Forward to server so external queries see the new name
                if let Some(port) = app.control_port {
                    let _ = crate::session::send_control_to_port(port, &format!("rename-session {}\n", crate::util::quote_arg(&name)), &app.session_key);
                }
            }
        }
        KeyCode::Backspace => { if let Mode::RenameSessionPrompt { input } = &mut app.mode { let _ = input.pop(); } }
        KeyCode::Char(c) => { if let Mode::RenameSessionPrompt { input } = &mut app.mode { input.push(c); } }
        _ => {}
    }
    Ok(false)
}

pub(crate) fn handle_key_pane_chooser(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Passthrough; }
        KeyCode::Char(d) if d.is_ascii_digit() => {
            let choice = d.to_digit(10).unwrap() as usize;
            if let Some((_, path)) = app.display_map.iter().find(|(n, _)| *n == choice) {
                let win = &mut app.windows[app.active_idx];
                win.active_path = path.clone();
            }
            app.mode = Mode::Passthrough;
        }
        _ => { app.mode = Mode::Passthrough; }
    }
    Ok(false)
}

pub(crate) fn handle_key_menu_mode(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    if let Mode::MenuMode { ref mut menu } = app.mode {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => { 
                app.mode = Mode::Passthrough; 
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if menu.selected > 0 {
                    menu.selected -= 1;
                    while menu.selected > 0 && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                        menu.selected -= 1;
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if menu.selected + 1 < menu.items.len() {
                    menu.selected += 1;
                    while menu.selected + 1 < menu.items.len() && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                        menu.selected += 1;
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(item) = menu.items.get(menu.selected) {
                    if !item.is_separator && !item.command.is_empty() {
                        let cmd = item.command.clone();
                        app.mode = Mode::Passthrough;
                        let _ = execute_command_string(app, &cmd);
                    } else {
                        app.mode = Mode::Passthrough;
                    }
                } else {
                    app.mode = Mode::Passthrough;
                }
            }
            KeyCode::Char(c) => {
                if let Some((_idx, item)) = menu.items.iter().enumerate().find(|(_, i)| i.key == Some(c)) {
                    if !item.is_separator && !item.command.is_empty() {
                        let cmd = item.command.clone();
                        app.mode = Mode::Passthrough;
                        let _ = execute_command_string(app, &cmd);
                    } else {
                        app.mode = Mode::Passthrough;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

pub(crate) fn handle_key_confirm_mode(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    if let Mode::ConfirmMode { prompt: _, ref command, ref mut input } = app.mode {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                app.mode = Mode::Passthrough;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let cmd = command.clone();
                app.mode = Mode::Passthrough;
                let _ = execute_command_string(app, &cmd);
            }
            KeyCode::Char(c) => {
                input.push(c);
            }
            KeyCode::Backspace => {
                input.pop();
            }
            _ => {}
        }
    }
    Ok(false)
}

pub(crate) fn handle_key_clock_mode(app: &mut AppState) -> io::Result<bool> {
    // Any key exits clock mode
    app.mode = Mode::Passthrough;
    Ok(false)
}

pub(crate) fn handle_key_buffer_chooser(app: &mut AppState, key: KeyEvent, selected: usize) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Passthrough; }
        KeyCode::Up | KeyCode::Char('k') => {
            if selected > 0 {
                if let Mode::BufferChooser { selected: s } = &mut app.mode { *s -= 1; }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.paste_buffers.len().saturating_sub(1);
            if selected < max {
                if let Mode::BufferChooser { selected: s } = &mut app.mode { *s += 1; }
            }
        }
        KeyCode::Enter => {
            // Paste selected buffer
            if selected < app.paste_buffers.len() {
                let text = app.paste_buffers[selected].clone();
                app.mode = Mode::Passthrough;
                let win = &mut app.windows[app.active_idx];
                if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                    let _ = write!(p.writer, "{}", text);
                }
            } else {
                app.mode = Mode::Passthrough;
            }
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            // Delete selected buffer
            if selected < app.paste_buffers.len() {
                app.paste_buffers.remove(selected);
                if let Mode::BufferChooser { selected: s } = &mut app.mode {
                    if *s >= app.paste_buffers.len() && *s > 0 { *s -= 1; }
                }
                if app.paste_buffers.is_empty() { app.mode = Mode::Passthrough; }
            }
        }
        _ => {}
    }
    Ok(false)
}
