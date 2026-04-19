use super::*;

pub(crate) fn handle_key_prefix(app: &mut AppState, key: KeyEvent, armed_at: Instant) -> io::Result<bool> {
    let elapsed = armed_at.elapsed().as_millis() as u64;

    // If we're in repeat mode and the repeat window has expired,
    // exit prefix and forward the key to the active pane (tmux parity).
    if app.prefix_repeating && elapsed >= app.repeat_time_ms {
        app.mode = Mode::Passthrough;
        app.prefix_repeating = false;
        forward_key_to_active(app, key)?;
        return Ok(false);
    }
    
    let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
    if let Some(bind) = app.key_tables.get("prefix").and_then(|t| t.iter().find(|b| b.key == key_tuple)).cloned() {
        if bind.repeat {
            // Stay in prefix mode for repeat-time window
            app.mode = Mode::Prefix { armed_at: Instant::now() };
            app.prefix_repeating = true;
        } else {
            app.mode = Mode::Passthrough;
            app.prefix_repeating = false;
        }
        return execute_action(app, &bind.action);
    }
    
    let handled = match key.code {
        // Alt+Arrow: resize pane by 5 (must be before plain arrows)
        KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
            crate::window_ops::resize_pane_vertical(app, -5); true
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
            crate::window_ops::resize_pane_vertical(app, 5); true
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
            crate::window_ops::resize_pane_horizontal(app, -5); true
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
            crate::window_ops::resize_pane_horizontal(app, 5); true
        }
        // Ctrl+Arrow: resize pane by 1 (must be before plain arrows)
        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
            crate::window_ops::resize_pane_vertical(app, -1); true
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
            crate::window_ops::resize_pane_vertical(app, 1); true
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            crate::window_ops::resize_pane_horizontal(app, -1); true
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            crate::window_ops::resize_pane_horizontal(app, 1); true
        }
        KeyCode::Left => { switch_with_copy_save(app, |app| move_focus(app, FocusDir::Left)); true }
        KeyCode::Right => { switch_with_copy_save(app, |app| move_focus(app, FocusDir::Right)); true }
        KeyCode::Up => { switch_with_copy_save(app, |app| move_focus(app, FocusDir::Up)); true }
        KeyCode::Down => { switch_with_copy_save(app, |app| move_focus(app, FocusDir::Down)); true }
        KeyCode::Char(d) if d.is_ascii_digit() => {
            let idx = d.to_digit(10).unwrap() as usize;
            if idx >= app.window_base_index {
                let internal_idx = idx - app.window_base_index;
                if internal_idx < app.windows.len() {
                    switch_with_copy_save(app, |app| {
                        app.last_window_idx = app.active_idx;
                        app.active_idx = internal_idx;
                    });
                }
            }
            true
        }
        KeyCode::Char('c') => {
            let pty_system = native_pty_system();
            create_window(&*pty_system, app, None, None)?;
            true
        }
        KeyCode::Char('n') => {
            if !app.windows.is_empty() {
                switch_with_copy_save(app, |app| {
                    app.last_window_idx = app.active_idx;
                    app.active_idx = (app.active_idx + 1) % app.windows.len();
                });
            }
            true
        }
        KeyCode::Char('p') => {
            if !app.windows.is_empty() {
                switch_with_copy_save(app, |app| {
                    app.last_window_idx = app.active_idx;
                    app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len();
                });
            }
            true
        }
        KeyCode::Char('%') => {
            split_active(app, LayoutKind::Horizontal)?;
            true
        }
        KeyCode::Char('"') => {
            split_active(app, LayoutKind::Vertical)?;
            true
        }
        KeyCode::Char('x') => {
            app.mode = Mode::ConfirmMode {
                prompt: "kill-pane? (y/n)".into(),
                command: "kill-pane".into(),
                input: String::new(),
            };
            true
        }
        KeyCode::Char('d') => {
            return Ok(true);
        }
        KeyCode::Char('w') => {
            let tree = crate::commands::build_choose_tree(app);
            let selected = tree.iter().position(|e| e.is_current_session && e.is_active_window && !e.is_session_header).unwrap_or(0);
            app.mode = Mode::WindowChooser { selected, tree };
            true
        }
        KeyCode::Char(',') => { app.mode = Mode::RenamePrompt { input: String::new() }; true }
        KeyCode::Char('\'') => { app.mode = Mode::WindowIndexPrompt { input: String::new() }; true }
        KeyCode::Char(' ') => { cycle_top_layout(app); true }
        KeyCode::Char('[') => { enter_copy_mode(app); true }
        KeyCode::Char(']') => { paste_latest(app)?; app.mode = Mode::Passthrough; true }
        KeyCode::Char(':') => {
            app.command_vi_normal = false;
            app.mode = Mode::CommandPrompt { input: String::new(), cursor: 0 };
            true
        }
        KeyCode::Char('q') => {
            let win = &app.windows[app.active_idx];
            let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
            compute_rects(&win.root, app.last_window_area, &mut rects);
            app.display_map.clear();
            for (i, (path, _)) in rects.into_iter().enumerate() {
                if i >= 10 { break; }
                let digit = (i + app.pane_base_index) % 10;
                app.display_map.push((digit, path));
            }
            app.mode = Mode::PaneChooser { opened_at: Instant::now() };
            true
        }
        // --- zoom pane (z) ---
        KeyCode::Char('z') => { toggle_zoom(app); true }
        // --- next pane (o) ---
        KeyCode::Char('o') => {
            switch_with_copy_save(app, |app| {
                let win = &app.windows[app.active_idx];
                let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
                compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some(cur) = rects.iter().position(|r| r.0 == win.active_path) {
                    let next = (cur + 1) % rects.len();
                    let new_path = rects[next].0.clone();
                    let win = &mut app.windows[app.active_idx];
                    app.last_pane_path = win.active_path.clone();
                    win.active_path = new_path;
                    // Update MRU
                    if let Some(pid) = crate::tree::get_active_pane_id(&win.root, &win.active_path) {
                        crate::tree::touch_mru(&mut win.pane_mru, pid);
                    }
                }
            });
            true
        }
        // --- last pane (;) ---
        KeyCode::Char(';') => {
            switch_with_copy_save(app, |app| {
                let win = &mut app.windows[app.active_idx];
                if !app.last_pane_path.is_empty() && path_exists(&win.root, &app.last_pane_path) {
                    let tmp = win.active_path.clone();
                    win.active_path = app.last_pane_path.clone();
                    app.last_pane_path = tmp;
                    // Update MRU
                    if let Some(pid) = crate::tree::get_active_pane_id(&win.root, &win.active_path) {
                        crate::tree::touch_mru(&mut win.pane_mru, pid);
                    }
                }
            });
            true
        }
        // --- last window (l) ---
        KeyCode::Char('l') => {
            if app.last_window_idx < app.windows.len() {
                switch_with_copy_save(app, |app| {
                    let tmp = app.active_idx;
                    app.active_idx = app.last_window_idx;
                    app.last_window_idx = tmp;
                });
            }
            true
        }
        // --- swap pane up/left ({) ---
        KeyCode::Char('{') => { swap_pane(app, FocusDir::Up); true }
        // --- swap pane down/right (}) ---
        KeyCode::Char('}') => { swap_pane(app, FocusDir::Down); true }
        // --- break pane to new window (!) ---
        KeyCode::Char('!') => { break_pane_to_window(app); true }
        // --- kill window (&) with confirmation ---
        KeyCode::Char('&') => {
            app.mode = Mode::ConfirmMode {
                prompt: "kill-window? (y/n)".into(),
                command: "kill-window".into(),
                input: String::new(),
            };
            true
        }
        // --- rename session ($) ---
        KeyCode::Char('$') => {
            app.mode = Mode::RenameSessionPrompt { input: String::new() };
            true
        }
        // --- Meta+1..5 preset layouts (like tmux) ---
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => {
            apply_layout(app, "even-horizontal"); true
        }
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => {
            apply_layout(app, "even-vertical"); true
        }
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => {
            apply_layout(app, "main-horizontal"); true
        }
        KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::ALT) => {
            apply_layout(app, "main-vertical"); true
        }
        KeyCode::Char('5') if key.modifiers.contains(KeyModifiers::ALT) => {
            apply_layout(app, "tiled"); true
        }
        // --- display pane info (i) ---
        KeyCode::Char('i') => {
            // Display window/pane info in status bar (tmux prefix+i)
            let win = &app.windows[app.active_idx];
            let pane_count = crate::tree::count_panes(&win.root);
            app.status_right = format!(
                "#{} ({}) [{}x{}] panes:{}", 
                app.active_idx, win.name,
                app.last_window_area.width, app.last_window_area.height,
                pane_count
            );
            true
        }
        // --- clock mode (t) ---
        KeyCode::Char('t') => {
            app.mode = Mode::ClockMode;
            true
        }
        // --- buffer chooser (=) ---
        KeyCode::Char('=') => {
            app.mode = Mode::BufferChooser { selected: 0 };
            true
        }
        _ => false,
    };

    if matches!(app.mode, Mode::Prefix { .. }) {
        // Arrow keys are repeatable by default (tmux binds them with -r)
        let is_repeatable = matches!(key.code,
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
        );
        if handled && is_repeatable {
            // Stay in prefix mode for repeat-time window
            app.mode = Mode::Prefix { armed_at: Instant::now() };
            app.prefix_repeating = true;
        } else if !handled && elapsed < app.escape_time_ms {
            return Ok(false);
        } else {
            app.mode = Mode::Passthrough;
            app.prefix_repeating = false;
        }
    }
    Ok(false)
}
