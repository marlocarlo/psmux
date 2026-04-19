use super::*;

pub(crate) fn handle_key_command_prompt(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    let vi_mode = app.user_options.get("status-keys").map(|v| v.as_str()) == Some("vi");

    // Vi normal mode handling
    if vi_mode && app.command_vi_normal {
        match key.code {
            KeyCode::Esc => { app.command_vi_normal = false; app.mode = Mode::Passthrough; }
            KeyCode::Enter => {
                if let Mode::CommandPrompt { input, .. } = &app.mode {
                    if !input.is_empty() {
                        let cmd = input.clone();
                        app.command_history.push(cmd);
                        if app.command_history.len() > 100 { app.command_history.remove(0); }
                        app.command_history_idx = app.command_history.len();
                    }
                }
                app.command_vi_normal = false;
                execute_command_prompt(app)?;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Mode::CommandPrompt { cursor, .. } = &mut app.mode {
                    if *cursor > 0 { *cursor -= 1; }
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    if *cursor < input.len() { *cursor += 1; }
                }
            }
            KeyCode::Char('0') | KeyCode::Home => {
                if let Mode::CommandPrompt { cursor, .. } = &mut app.mode {
                    *cursor = 0;
                }
            }
            KeyCode::Char('$') | KeyCode::End => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    *cursor = input.len();
                }
            }
            KeyCode::Char('b') => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    let mut pos = *cursor;
                    while pos > 0 && input.as_bytes().get(pos - 1) == Some(&b' ') { pos -= 1; }
                    while pos > 0 && input.as_bytes().get(pos - 1) != Some(&b' ') { pos -= 1; }
                    *cursor = pos;
                }
            }
            KeyCode::Char('w') => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    let len = input.len();
                    let mut pos = *cursor;
                    while pos < len && input.as_bytes().get(pos) != Some(&b' ') { pos += 1; }
                    while pos < len && input.as_bytes().get(pos) == Some(&b' ') { pos += 1; }
                    *cursor = pos;
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    if *cursor < input.len() { input.remove(*cursor); }
                }
            }
            KeyCode::Char('i') => { app.command_vi_normal = false; }
            KeyCode::Char('a') => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    if *cursor < input.len() { *cursor += 1; }
                }
                app.command_vi_normal = false;
            }
            KeyCode::Char('A') => {
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    *cursor = input.len();
                }
                app.command_vi_normal = false;
            }
            KeyCode::Char('I') => {
                if let Mode::CommandPrompt { cursor, .. } = &mut app.mode {
                    *cursor = 0;
                }
                app.command_vi_normal = false;
            }
            KeyCode::Up => {
                if app.command_history_idx > 0 {
                    app.command_history_idx -= 1;
                    let cmd = app.command_history[app.command_history_idx].clone();
                    let len = cmd.len();
                    if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                        *input = cmd;
                        *cursor = len;
                    }
                }
            }
            KeyCode::Down => {
                if app.command_history_idx < app.command_history.len() {
                    app.command_history_idx += 1;
                    let cmd = if app.command_history_idx < app.command_history.len() {
                        app.command_history[app.command_history_idx].clone()
                    } else {
                        String::new()
                    };
                    let len = cmd.len();
                    if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                        *input = cmd;
                        *cursor = len;
                    }
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    // Emacs mode / vi insert mode
    match key.code {
        KeyCode::Esc => {
            if vi_mode {
                app.command_vi_normal = true;
            } else {
                app.mode = Mode::Passthrough;
            }
        }
        KeyCode::Enter => {
            // Save to history before executing
            if let Mode::CommandPrompt { input, .. } = &app.mode {
                if !input.is_empty() {
                    let cmd = input.clone();
                    app.command_history.push(cmd);
                    if app.command_history.len() > 100 { app.command_history.remove(0); }
                    app.command_history_idx = app.command_history.len();
                }
            }
            execute_command_prompt(app)?;
        }
        KeyCode::Backspace => {
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                if *cursor > 0 {
                    input.remove(*cursor - 1);
                    *cursor -= 1;
                }
            }
        }
        KeyCode::Delete => {
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                if *cursor < input.len() {
                    input.remove(*cursor);
                }
            }
        }
        KeyCode::Left => {
            if let Mode::CommandPrompt { cursor, .. } = &mut app.mode {
                if *cursor > 0 { *cursor -= 1; }
            }
        }
        KeyCode::Right => {
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                if *cursor < input.len() { *cursor += 1; }
            }
        }
        KeyCode::Home => {
            if let Mode::CommandPrompt { cursor, .. } = &mut app.mode {
                *cursor = 0;
            }
        }
        KeyCode::End => {
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                *cursor = input.len();
            }
        }
        KeyCode::Up => {
            // Cycle through command history (older)
            if app.command_history_idx > 0 {
                app.command_history_idx -= 1;
                let cmd = app.command_history[app.command_history_idx].clone();
                let len = cmd.len();
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    *input = cmd;
                    *cursor = len;
                }
            }
        }
        KeyCode::Down => {
            // Cycle through command history (newer)
            if app.command_history_idx < app.command_history.len() {
                app.command_history_idx += 1;
                let cmd = if app.command_history_idx < app.command_history.len() {
                    app.command_history[app.command_history_idx].clone()
                } else {
                    String::new()
                };
                let len = cmd.len();
                if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                    *input = cmd;
                    *cursor = len;
                }
            }
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+A: move to beginning
            if let Mode::CommandPrompt { cursor, .. } = &mut app.mode {
                *cursor = 0;
            }
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+E: move to end
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                *cursor = input.len();
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+U: kill line (clear from cursor to start)
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                input.drain(..*cursor);
                *cursor = 0;
            }
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+K: kill to end of line
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                input.truncate(*cursor);
            }
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+W: delete word backwards
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                let mut pos = *cursor;
                while pos > 0 && input.as_bytes().get(pos - 1) == Some(&b' ') { pos -= 1; }
                while pos > 0 && input.as_bytes().get(pos - 1) != Some(&b' ') { pos -= 1; }
                input.drain(pos..*cursor);
                *cursor = pos;
            }
        }
        KeyCode::Char(c) => {
            if let Mode::CommandPrompt { input, cursor } = &mut app.mode {
                input.insert(*cursor, c);
                *cursor += 1;
            }
        }
        _ => {}
    }
    Ok(false)
}
