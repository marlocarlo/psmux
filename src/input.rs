use std::io::{self, Write};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use portable_pty::native_pty_system;
use ratatui::prelude::*;

use crate::types::{AppState, Mode, FocusDir, LayoutKind, DragState, Node, Pane};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, path_exists, resize_all_panes};
use crate::pane::{create_window, split_active};
use crate::commands::{execute_action, execute_command_prompt, execute_command_string};
use crate::config::normalize_key_for_binding;
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, switch_with_copy_save, move_copy_cursor,
    scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, paste_latest, yank_selection,
    search_copy_mode, search_next, search_prev, scroll_to_top, scroll_to_bottom};
use crate::layout::{cycle_top_layout, apply_layout};
use crate::window_ops::{toggle_zoom, swap_pane, break_pane_to_window};

/// Write a mouse event to the child PTY using the encoding the child requested.
fn write_mouse_event(master: &mut dyn std::io::Write, button: u8, col: u16, row: u16, press: bool, enc: vt100::MouseProtocolEncoding) {
    match enc {
        vt100::MouseProtocolEncoding::Sgr => {
            let ch = if press { 'M' } else { 'm' };
            let _ = write!(master, "\x1b[<{};{};{}{}", button, col, row, ch);
            let _ = master.flush();
        }
        _ => {
            // Default / Utf8 X10-style encoding: \x1b[M Cb Cx Cy (all + 32)
            if press {
                let cb = (button + 32) as u8;
                let cx = ((col as u8).min(223)) + 32;
                let cy = ((row as u8).min(223)) + 32;
                let _ = master.write_all(&[0x1b, b'[', b'M', cb, cx, cy]);
                let _ = master.flush();
            }
            // X10-style has no release encoding for individual buttons
        }
    }
}

pub fn handle_key(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match app.mode {
        Mode::Passthrough => {
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
        Mode::Prefix { armed_at } => {
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
        Mode::CommandPrompt { .. } => {
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
        Mode::WindowChooser { selected, ref tree } => {
            let tree_len = tree.len();
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
                    if let Some(idx) = tree.iter().position(|e| !e.is_session_header && e.window_index == Some(n) && e.is_current_session) {
                        if let Mode::WindowChooser { selected: s, .. } = &mut app.mode { *s = idx; }
                    }
                }
                _ => {}
            }
            Ok(false)
        }
        Mode::WindowIndexPrompt { .. } => {
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
        Mode::RenamePrompt { .. } => {
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Passthrough; }
                KeyCode::Enter => { if let Mode::RenamePrompt { input } = &mut app.mode { app.windows[app.active_idx].name = input.clone(); app.mode = Mode::Passthrough; } }
                KeyCode::Backspace => { if let Mode::RenamePrompt { input } = &mut app.mode { let _ = input.pop(); } }
                KeyCode::Char(c) => { if let Mode::RenamePrompt { input } = &mut app.mode { input.push(c); } }
                _ => {}
            }
            Ok(false)
        }
        Mode::RenameSessionPrompt { .. } => {
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Passthrough; }
                KeyCode::Enter => {
                    if let Mode::RenameSessionPrompt { input } = &mut app.mode {
                        app.session_name = input.clone();
                        app.mode = Mode::Passthrough;
                    }
                }
                KeyCode::Backspace => { if let Mode::RenameSessionPrompt { input } = &mut app.mode { let _ = input.pop(); } }
                KeyCode::Char(c) => { if let Mode::RenameSessionPrompt { input } = &mut app.mode { input.push(c); } }
                _ => {}
            }
            Ok(false)
        }
        Mode::CopyMode => {
            // Check copy-mode key table for user bindings first (used by plugins like tmux-yank)
            let table_name = if app.mode_keys == "vi" { "copy-mode-vi" } else { "copy-mode" };
            let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
            if let Some(bind) = app.key_tables.get(table_name)
                .and_then(|t| t.iter().find(|b| b.key == key_tuple))
                .cloned()
            {
                return execute_action(app, &bind.action);
            }
            // Handle register pending state (waiting for a-z after ")
            if app.copy_register_pending {
                app.copy_register_pending = false;
                if let KeyCode::Char(ch) = key.code {
                    if ch.is_ascii_lowercase() {
                        app.copy_register = Some(ch);
                    }
                }
                return Ok(false);
            }
            // Handle text-object pending state (waiting for w/W after a/i)
            if let Some(prefix) = app.copy_text_object_pending.take() {
                if let KeyCode::Char(ch) = key.code {
                    match (prefix, ch) {
                        (0, 'w') => { crate::copy_mode::select_a_word(app); }
                        (1, 'w') => { crate::copy_mode::select_inner_word(app); }
                        (0, 'W') => { crate::copy_mode::select_a_word_big(app); }
                        (1, 'W') => { crate::copy_mode::select_inner_word_big(app); }
                        _ => {}
                    }
                }
                return Ok(false);
            }
            // Handle find-char pending state (waiting for char after f/F/t/T)
            if let Some(pending) = app.copy_find_char_pending.take() {
                let n = app.copy_count.take().unwrap_or(1);
                if let KeyCode::Char(ch) = key.code {
                    match pending {
                        0 => { for _ in 0..n { crate::copy_mode::find_char_forward(app, ch); } }
                        1 => { for _ in 0..n { crate::copy_mode::find_char_backward(app, ch); } }
                        2 => { for _ in 0..n { crate::copy_mode::find_char_to_forward(app, ch); } }
                        3 => { for _ in 0..n { crate::copy_mode::find_char_to_backward(app, ch); } }
                        _ => {}
                    }
                }
                return Ok(false);
            }
            // Handle numeric prefix accumulation for copy-mode motions (vi-style)
            if let KeyCode::Char(d) = key.code {
                if d.is_ascii_digit() && !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
                    let digit = d.to_digit(10).unwrap() as usize;
                    if let Some(count) = app.copy_count {
                        // Accumulate: multiply by 10 and add digit (cap at 9999)
                        app.copy_count = Some((count * 10 + digit).min(9999));
                        return Ok(false);
                    } else if digit >= 1 {
                        // Start new count with 1-9
                        app.copy_count = Some(digit);
                        return Ok(false);
                    }
                    // digit == 0 with no existing count → fall through to line-start handler
                }
            }
            let copy_repeat = app.copy_count.take().unwrap_or(1);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(']') => { 
                    exit_copy_mode(app);
                }
                // Ctrl+C exits copy mode (tmux parity, fixes #25)
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    exit_copy_mode(app);
                }
                KeyCode::Left | KeyCode::Char('h') => { for _ in 0..copy_repeat { move_copy_cursor(app, -1, 0); } }
                KeyCode::Right | KeyCode::Char('l') => { for _ in 0..copy_repeat { move_copy_cursor(app, 1, 0); } }
                KeyCode::Up | KeyCode::Char('k') => { for _ in 0..copy_repeat { move_copy_cursor(app, 0, -1); } }
                KeyCode::Down | KeyCode::Char('j') => { for _ in 0..copy_repeat { move_copy_cursor(app, 0, 1); } }
                // Page scroll: C-b / PageUp = page up, C-f / PageDown = page down
                KeyCode::PageUp => { scroll_copy_up(app, 10); }
                KeyCode::PageDown => { scroll_copy_down(app, 10); }
                KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.mode_keys == "emacs" { move_copy_cursor(app, -1, 0); }
                    else { scroll_copy_up(app, 10); }
                }
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.mode_keys == "emacs" { move_copy_cursor(app, 1, 0); }
                    else { scroll_copy_down(app, 10); }
                }
                // Half-page scroll: C-u / C-d
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let half = app.windows.get(app.active_idx)
                        .and_then(|w| active_pane(&w.root, &w.active_path))
                        .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                    scroll_copy_up(app, half);
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let half = app.windows.get(app.active_idx)
                        .and_then(|w| active_pane(&w.root, &w.active_path))
                        .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                    scroll_copy_down(app, half);
                }
                // Emacs copy-mode keys (must be before unqualified char matches)
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => { scroll_copy_down(app, 1); }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => { scroll_copy_up(app, 1); }
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => { crate::copy_mode::move_to_line_start(app); }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => { crate::copy_mode::move_to_line_end(app); }
                KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::ALT) => { scroll_copy_up(app, 10); }
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => { crate::copy_mode::move_word_forward(app); }
                KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => { crate::copy_mode::move_word_backward(app); }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::ALT) => { yank_selection(app)?; exit_copy_mode(app); }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.mode = Mode::CopySearch { input: String::new(), forward: true };
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.mode = Mode::CopySearch { input: String::new(), forward: false };
                }
                KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    exit_copy_mode(app);
                }
                KeyCode::Char('g') => { scroll_to_top(app); }
                KeyCode::Char('G') => { scroll_to_bottom(app); }
                // Word motions: w = next word, b = prev word, e = end of word
                KeyCode::Char('w') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_forward(app); } }
                KeyCode::Char('b') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_backward(app); } }
                KeyCode::Char('e') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_end(app); } }
                // WORD motions: W = next WORD, B = prev WORD, E = end WORD
                KeyCode::Char('W') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_forward_big(app); } }
                KeyCode::Char('B') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_backward_big(app); } }
                KeyCode::Char('E') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_end_big(app); } }
                // Screen position: H = top, M = middle, L = bottom
                KeyCode::Char('H') => { crate::copy_mode::move_to_screen_top(app); }
                KeyCode::Char('M') => { crate::copy_mode::move_to_screen_middle(app); }
                KeyCode::Char('L') => { crate::copy_mode::move_to_screen_bottom(app); }
                // Find char: f/F/t/T — sets pending state for next char
                KeyCode::Char('f') => { app.copy_find_char_pending = Some(0); app.copy_count = Some(copy_repeat); }
                KeyCode::Char('F') => { app.copy_find_char_pending = Some(1); app.copy_count = Some(copy_repeat); }
                KeyCode::Char('t') => { app.copy_find_char_pending = Some(2); app.copy_count = Some(copy_repeat); }
                KeyCode::Char('T') => { app.copy_find_char_pending = Some(3); app.copy_count = Some(copy_repeat); }
                // D = copy from cursor to end of line
                KeyCode::Char('D') => { crate::copy_mode::copy_end_of_line(app)?; exit_copy_mode(app); }
                // Bracket matching: % = jump to matching bracket/paren/brace
                KeyCode::Char('%') => { crate::copy_mode::move_matching_bracket(app); }
                // Paragraph jump: { = previous paragraph, } = next paragraph
                KeyCode::Char('{') => { for _ in 0..copy_repeat { crate::copy_mode::move_prev_paragraph(app); } }
                KeyCode::Char('}') => { for _ in 0..copy_repeat { crate::copy_mode::move_next_paragraph(app); } }
                // Line motions: 0 = start, $ = end, ^ = first non-blank
                KeyCode::Char('0') => { crate::copy_mode::move_to_line_start(app); }
                KeyCode::Char('$') => { crate::copy_mode::move_to_line_end(app); }
                KeyCode::Char('^') => { crate::copy_mode::move_to_first_nonblank(app); }
                KeyCode::Home => { crate::copy_mode::move_to_line_start(app); }
                KeyCode::End => { crate::copy_mode::move_to_line_end(app); }
                KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // vi: toggle rectangle selection, emacs: page down
                    if app.mode_keys == "emacs" {
                        scroll_copy_down(app, 10);
                    } else {
                        app.copy_selection_mode = crate::types::SelectionMode::Rect;
                    }
                }
                KeyCode::Char('v') => {
                    // tmux parity #62: rectangle-toggle (not begin-selection)
                    app.copy_selection_mode = match app.copy_selection_mode {
                        crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                        _ => crate::types::SelectionMode::Rect,
                    };
                }
                KeyCode::Char('V') => {
                    // Start line-wise selection (vi visual-line mode)
                    if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                        app.copy_anchor = Some((r,c));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_pos = Some((r,c));
                        app.copy_selection_mode = crate::types::SelectionMode::Line;
                    }
                }
                KeyCode::Char('o') => {
                    // Swap cursor and anchor
                    if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                        app.copy_anchor = Some(p);
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_pos = Some(a);
                    }
                }
                KeyCode::Char('A') => {
                    // Append to buffer (yank + append to buffer 0)
                    if let (Some(_), Some(_)) = (app.copy_anchor, app.copy_pos) {
                        // Save current buffer 0
                        let prev = app.paste_buffers.first().cloned().unwrap_or_default();
                        yank_selection(app)?;
                        // buffer 0 is now the new yank; prepend old text
                        if let Some(buf) = app.paste_buffers.first_mut() {
                            let new_text = buf.clone();
                            *buf = format!("{}{}", prev, new_text);
                        }
                        exit_copy_mode(app);
                    }
                }
                // Space = begin selection (vi mode), Enter = copy-selection-and-cancel
                KeyCode::Char(' ') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                        app.copy_anchor = Some((r,c));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_pos = Some((r,c));
                        app.copy_selection_mode = crate::types::SelectionMode::Char;
                    }
                }
                KeyCode::Enter => {
                    // Copy selection and exit copy mode (vi Enter)
                    if app.copy_anchor.is_some() {
                        yank_selection(app)?;
                    }
                    exit_copy_mode(app);
                }
                KeyCode::Char('y') => { yank_selection(app)?; exit_copy_mode(app); }
                // --- copy-mode search ---
                KeyCode::Char('/') => {
                    app.mode = Mode::CopySearch { input: String::new(), forward: true };
                }
                KeyCode::Char('?') => {
                    app.mode = Mode::CopySearch { input: String::new(), forward: false };
                }
                KeyCode::Char('n') => { search_next(app); }
                KeyCode::Char('N') => { search_prev(app); }
                KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Set mark (anchor)
                    if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                        app.copy_anchor = Some((r, c));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_pos = Some((r, c));
                    }
                }
                // Named register prefix: " then a-z
                KeyCode::Char('"') => { app.copy_register_pending = true; }
                // Text-object prefixes: a/i then w/W
                KeyCode::Char('a') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                    app.copy_text_object_pending = Some(0);
                }
                KeyCode::Char('i') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                    app.copy_text_object_pending = Some(1);
                }
                _ => {}
            }
            Ok(false)
        }
        Mode::CopySearch { .. } => {
            match key.code {
                KeyCode::Esc => {
                    // Cancel search, return to copy mode
                    app.mode = Mode::CopyMode;
                }
                KeyCode::Enter => {
                    // Execute search
                    if let Mode::CopySearch { ref input, forward } = app.mode {
                        let query = input.clone();
                        let fwd = forward;
                        app.copy_search_query = query.clone();
                        app.copy_search_forward = fwd;
                        search_copy_mode(app, &query, fwd);
                        // Jump to first match
                        if !app.copy_search_matches.is_empty() {
                            let (r, c, _) = app.copy_search_matches[0];
                            app.copy_pos = Some((r, c));
                        }
                    }
                    app.mode = Mode::CopyMode;
                }
                KeyCode::Backspace => {
                    if let Mode::CopySearch { ref mut input, .. } = app.mode { let _ = input.pop(); }
                }
                KeyCode::Char(c) => {
                    if let Mode::CopySearch { ref mut input, .. } = app.mode { input.push(c); }
                }
                _ => {}
            }
            Ok(false)
        }
        Mode::PaneChooser { .. } => {
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
        Mode::MenuMode { ref mut menu } => {
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
            Ok(false)
        }
        Mode::PopupMode { ref mut output, ref mut process, close_on_exit, ref mut popup_pane, ref mut scroll_offset, .. } => {
            let mut should_close = false;
            let mut exit_status: Option<std::process::ExitStatus> = None;
            
            // If we have a PTY popup, forward keys to it
            if let Some(ref mut pty) = popup_pane {
                match key.code {
                    KeyCode::Esc => {
                        // Check if the child has exited
                        if let Ok(Some(_)) = pty.child.try_wait() {
                            should_close = true;
                        } else {
                            // Forward Escape to the PTY
                            let _ = pty.writer.write_all(b"\x1b");
                        }
                    }
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                            let ctrl = (c as u8) & 0x1F;
                            let _ = pty.writer.write_all(&[ctrl]);
                        } else {
                            let mut buf = [0u8; 4];
                            let s = c.encode_utf8(&mut buf);
                            let _ = pty.writer.write_all(s.as_bytes());
                        }
                    }
                    KeyCode::Enter => { let _ = pty.writer.write_all(b"\r"); }
                    KeyCode::Backspace => { let _ = pty.writer.write_all(b"\x7f"); }
                    KeyCode::Tab => { let _ = pty.writer.write_all(b"\t"); }
                    KeyCode::BackTab => { let _ = pty.writer.write_all(b"\x1b[Z"); }
                    KeyCode::Up => { let _ = pty.writer.write_all(b"\x1b[A"); }
                    KeyCode::Down => { let _ = pty.writer.write_all(b"\x1b[B"); }
                    KeyCode::Right => { let _ = pty.writer.write_all(b"\x1b[C"); }
                    KeyCode::Left => { let _ = pty.writer.write_all(b"\x1b[D"); }
                    KeyCode::Home => { let _ = pty.writer.write_all(b"\x1b[H"); }
                    KeyCode::End => { let _ = pty.writer.write_all(b"\x1b[F"); }
                    KeyCode::PageUp => { let _ = pty.writer.write_all(b"\x1b[5~"); }
                    KeyCode::PageDown => { let _ = pty.writer.write_all(b"\x1b[6~"); }
                    KeyCode::Delete => { let _ = pty.writer.write_all(b"\x1b[3~"); }
                    _ => {}
                }
                // Check if child exited
                if let Ok(Some(_status)) = pty.child.try_wait() {
                    if close_on_exit {
                        should_close = true;
                    }
                }
            } else {
                // Non-PTY popup (static output)
                let total_lines = output.lines().count() as u16;
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        if let Some(ref mut proc) = process {
                            let _ = proc.kill();
                        }
                        should_close = true;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        *scroll_offset = scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if *scroll_offset < total_lines.saturating_sub(1) {
                            *scroll_offset += 1;
                        }
                    }
                    KeyCode::PageUp => {
                        *scroll_offset = scroll_offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        *scroll_offset = (*scroll_offset + 10).min(total_lines.saturating_sub(1));
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        *scroll_offset = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        *scroll_offset = total_lines.saturating_sub(1);
                    }
                    _ => {}
                }
                
                if let Some(ref mut proc) = process {
                    if let Ok(Some(status)) = proc.try_wait() {
                        exit_status = Some(status);
                        if close_on_exit {
                            should_close = true;
                        }
                    }
                }
                
                if let Some(status) = exit_status {
                    if !close_on_exit {
                        output.push_str(&format!("\n[Process exited with status: {}]", status));
                    }
                }
            }
            
            if should_close {
                app.mode = Mode::Passthrough;
            }
            
            Ok(false)
        }
        Mode::ConfirmMode { prompt: _, ref command, ref mut input } => {
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
            Ok(false)
        }
        Mode::ClockMode => {
            // Any key exits clock mode
            app.mode = Mode::Passthrough;
            Ok(false)
        }
        Mode::CustomizeMode { ref options, selected: _, ref filter, editing, .. } => {
            if editing {
                match key.code {
                    KeyCode::Esc => {
                        if let Mode::CustomizeMode { editing: ref mut e, edit_buffer: ref mut eb, .. } = app.mode {
                            *e = false;
                            *eb = String::new();
                        }
                    }
                    KeyCode::Enter => {
                        if let Mode::CustomizeMode { ref mut editing, ref options, selected, ref edit_buffer, .. } = app.mode {
                            let name = options[selected].0.clone();
                            let value = edit_buffer.clone();
                            *editing = false;
                            crate::server::options::apply_set_option(app, &name, &value, true);
                            if let Mode::CustomizeMode { ref mut options, selected, .. } = app.mode {
                                options[selected].1 = value;
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if let Mode::CustomizeMode { ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                            if *edit_cursor > 0 {
                                edit_buffer.remove(*edit_cursor - 1);
                                *edit_cursor -= 1;
                            }
                        }
                    }
                    KeyCode::Left => {
                        if let Mode::CustomizeMode { ref mut edit_cursor, .. } = app.mode {
                            *edit_cursor = edit_cursor.saturating_sub(1);
                        }
                    }
                    KeyCode::Right => {
                        if let Mode::CustomizeMode { ref mut edit_cursor, ref edit_buffer, .. } = app.mode {
                            if *edit_cursor < edit_buffer.len() { *edit_cursor += 1; }
                        }
                    }
                    KeyCode::Char(c) => {
                        if let Mode::CustomizeMode { ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                            edit_buffer.insert(*edit_cursor, c);
                            *edit_cursor += 1;
                        }
                    }
                    _ => {}
                }
            } else {
                let _visible_count = options.iter()
                    .filter(|(name, _, _)| filter.is_empty() || name.contains(filter.as_str()))
                    .count();
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Passthrough; }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, .. } = app.mode {
                            let visible: Vec<usize> = options.iter().enumerate()
                                .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i)
                                .collect();
                            if let Some(cur_pos) = visible.iter().position(|&i| i == *selected) {
                                if cur_pos > 0 {
                                    *selected = visible[cur_pos - 1];
                                    if cur_pos - 1 < *scroll_offset {
                                        *scroll_offset = cur_pos - 1;
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, .. } = app.mode {
                            let visible: Vec<usize> = options.iter().enumerate()
                                .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i)
                                .collect();
                            if let Some(cur_pos) = visible.iter().position(|&i| i == *selected) {
                                if cur_pos + 1 < visible.len() {
                                    *selected = visible[cur_pos + 1];
                                    if cur_pos + 1 >= *scroll_offset + 20 {
                                        *scroll_offset = (cur_pos + 1).saturating_sub(19);
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Enter => {
                        if let Mode::CustomizeMode { ref options, selected, ref mut editing, ref mut edit_buffer, ref mut edit_cursor, .. } = app.mode {
                            if let Some((_, value, _)) = options.get(selected) {
                                *edit_buffer = value.clone();
                                *edit_cursor = edit_buffer.len();
                                *editing = true;
                            }
                        }
                    }
                    KeyCode::Char('d') => {
                        if let Mode::CustomizeMode { ref mut options, selected, .. } = app.mode {
                            if let Some(def) = crate::server::option_catalog::default_for(&options[selected].0) {
                                let name = options[selected].0.clone();
                                let value = def.to_string();
                                options[selected].1 = value.clone();
                                crate::server::options::apply_set_option(app, &name, &value, true);
                            }
                        }
                    }
                    KeyCode::Char('/') => {
                        // Enter filter mode via command prompt (simplified: clear filter or apply)
                        if let Mode::CustomizeMode { ref mut filter, ref mut scroll_offset, ref mut selected, .. } = app.mode {
                            if !filter.is_empty() {
                                // Toggle filter off
                                *filter = String::new();
                                *scroll_offset = 0;
                                *selected = 0;
                            }
                            // If filter is empty, we would need a mini prompt; for now users
                            // use the server path for full filter support
                        }
                    }
                    KeyCode::PageUp => {
                        if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, .. } = app.mode {
                            let visible: Vec<usize> = options.iter().enumerate()
                                .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i).collect();
                            if let Some(cur_pos) = visible.iter().position(|&i| i == *selected) {
                                let new_pos = cur_pos.saturating_sub(20);
                                *selected = visible[new_pos];
                                *scroll_offset = new_pos;
                            }
                        }
                    }
                    KeyCode::PageDown => {
                        if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, .. } = app.mode {
                            let visible: Vec<usize> = options.iter().enumerate()
                                .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i).collect();
                            if let Some(cur_pos) = visible.iter().position(|&i| i == *selected) {
                                let new_pos = (cur_pos + 20).min(visible.len().saturating_sub(1));
                                *selected = visible[new_pos];
                                if new_pos >= *scroll_offset + 20 {
                                    *scroll_offset = new_pos.saturating_sub(19);
                                }
                            }
                        }
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, .. } = app.mode {
                            let first = options.iter().enumerate()
                                .find(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i);
                            if let Some(idx) = first { *selected = idx; *scroll_offset = 0; }
                        }
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        if let Mode::CustomizeMode { ref options, ref mut selected, ref filter, ref mut scroll_offset, .. } = app.mode {
                            let last = options.iter().enumerate()
                                .filter(|(_, (name, _, _))| filter.is_empty() || name.contains(filter.as_str()))
                                .map(|(i, _)| i).last();
                            if let Some(idx) = last {
                                *selected = idx;
                                let visible_len = options.iter()
                                    .filter(|(name, _, _)| filter.is_empty() || name.contains(filter.as_str()))
                                    .count();
                                *scroll_offset = visible_len.saturating_sub(20);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(false)
        }
        Mode::BufferChooser { selected } => {
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
    }
}

pub fn move_focus(app: &mut AppState, dir: FocusDir) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    let mut active_idx = None;
    for (i, (path, _)) in rects.iter().enumerate() { if *path == win.active_path { active_idx = Some(i); break; } }
    let Some(ai) = active_idx else { return; };
    let (_, arect) = &rects[ai];
    // Collect pane IDs for MRU-based tie-breaking (issue #70)
    let pane_ids: Vec<usize> = rects.iter().map(|(path, _)| {
        crate::tree::get_active_pane_id(&win.root, path).unwrap_or(usize::MAX)
    }).collect();
    // Try direct neighbour first, then wrap to opposite edge (tmux parity #61)
    let target = find_best_pane_in_direction(&rects, ai, arect, dir, &pane_ids, &win.pane_mru)
        .or_else(|| find_wrap_target(&rects, ai, arect, dir, &pane_ids, &win.pane_mru));
    if let Some(ni) = target {
        // Update MRU: push the newly focused pane to front
        if let Some(new_pane_id) = pane_ids.get(ni) {
            crate::tree::touch_mru(&mut win.pane_mru, *new_pane_id);
        }
        win.active_path = rects[ni].0.clone();
    }
}

/// Spatial pane navigation: find the best pane in the given direction.
/// Prefers panes that overlap on the perpendicular axis (visually adjacent),
/// then picks the closest by primary-axis gap, tie-broken by MRU recency
/// when multiple candidates have the same geometry (tmux parity #70).
pub fn find_best_pane_in_direction(
    rects: &[(Vec<usize>, Rect)],
    ai: usize,
    arect: &Rect,
    dir: FocusDir,
    pane_ids: &[usize],
    pane_mru: &[usize],
) -> Option<usize> {
    // Center of the active pane (scaled by 2 to avoid fractional math)
    let acx = arect.x as i32 * 2 + arect.width as i32;
    let acy = arect.y as i32 * 2 + arect.height as i32;

    // Check whether two 1-D ranges [a_start, a_start+a_len) and [b_start, b_start+b_len) overlap
    let ranges_overlap = |a_start: u16, a_len: u16, b_start: u16, b_len: u16| -> bool {
        let a_end = a_start + a_len;
        let b_end = b_start + b_len;
        a_start < b_end && b_start < a_end
    };

    // (index, primary_gap, perp_center_dist, has_perp_overlap, mru_rank)
    let mut best: Option<(usize, u32, i32, bool, usize)> = None;

    for (i, (_, r)) in rects.iter().enumerate() {
        if i == ai { continue; }
        // Primary-axis gap: the pane must be in the correct direction
        let (primary_gap, perp_overlap) = match dir {
            FocusDir::Left => {
                if r.x + r.width > arect.x { continue; }
                let gap = (arect.x - (r.x + r.width)) as u32;
                let overlap = ranges_overlap(r.y, r.height, arect.y, arect.height);
                (gap, overlap)
            }
            FocusDir::Right => {
                if r.x < arect.x + arect.width { continue; }
                let gap = (r.x - (arect.x + arect.width)) as u32;
                let overlap = ranges_overlap(r.y, r.height, arect.y, arect.height);
                (gap, overlap)
            }
            FocusDir::Up => {
                if r.y + r.height > arect.y { continue; }
                let gap = (arect.y - (r.y + r.height)) as u32;
                let overlap = ranges_overlap(r.x, r.width, arect.x, arect.width);
                (gap, overlap)
            }
            FocusDir::Down => {
                if r.y < arect.y + arect.height { continue; }
                let gap = (r.y - (arect.y + arect.height)) as u32;
                let overlap = ranges_overlap(r.x, r.width, arect.x, arect.width);
                (gap, overlap)
            }
        };

        // Perpendicular center distance (how far off-center the candidate is)
        let rcx = r.x as i32 * 2 + r.width as i32;
        let rcy = r.y as i32 * 2 + r.height as i32;
        let perp_dist = match dir {
            FocusDir::Left | FocusDir::Right => (rcy - acy).abs(),
            FocusDir::Up | FocusDir::Down => (rcx - acx).abs(),
        };

        // MRU rank: lower = more recently used (tmux parity #70)
        let rank = pane_ids.get(i)
            .map(|id| crate::tree::mru_rank(pane_mru, *id))
            .unwrap_or(usize::MAX);

        let dominated = if let Some((_, bg, bd, bo, br)) = best {
            // Prefer: (1) perp-overlapping over non-overlapping,
            //         (2) smaller primary gap,
            //         (3) among overlapping candidates with same gap → MRU (tmux parity #70),
            //         (4) among non-overlapping candidates → perpendicular center distance,
            //         (5) final fallback → MRU rank
            if perp_overlap && !bo {
                false  // new candidate has overlap, current best doesn't → new wins
            } else if !perp_overlap && bo {
                true   // current best has overlap, new doesn't → new loses
            } else if primary_gap < bg {
                false  // closer on primary axis
            } else if primary_gap > bg {
                true   // farther on primary axis
            } else if perp_overlap && bo {
                // Both candidates overlap the active pane's perpendicular
                // range with the same primary gap — use MRU directly.
                // tmux does NOT compare center distance for overlapping
                // candidates; it picks the most recently focused one.
                rank >= br
            } else if perp_dist < bd {
                false  // neither overlaps → closer perpendicular center
            } else if perp_dist > bd {
                true   // farther perpendicular center
            } else {
                rank >= br  // same geometry → MRU tie-break
            }
        } else {
            false  // no best yet
        };

        if !dominated {
            best = Some((i, primary_gap, perp_dist, perp_overlap, rank));
        }
    }

    best.map(|(idx, _, _, _, _)| idx)
}

/// Wrap-around pane navigation (tmux parity #61): when no pane exists in the
/// requested direction, wrap to the pane on the opposite edge.
/// For Right → leftmost pane, Left → rightmost, Down → topmost, Up → bottommost.
/// Prefers panes with perpendicular overlap, then closest to center.
pub fn find_wrap_target(
    rects: &[(Vec<usize>, Rect)],
    ai: usize,
    arect: &Rect,
    dir: FocusDir,
    pane_ids: &[usize],
    pane_mru: &[usize],
) -> Option<usize> {
    let acx = arect.x as i32 * 2 + arect.width as i32;
    let acy = arect.y as i32 * 2 + arect.height as i32;

    let ranges_overlap = |a_start: u16, a_len: u16, b_start: u16, b_len: u16| -> bool {
        let a_end = a_start + a_len;
        let b_end = b_start + b_len;
        a_start < b_end && b_start < a_end
    };

    // (index, edge_score, perp_center_dist, has_perp_overlap, mru_rank)
    // edge_score: lower = better (closer to the target edge after wrapping)
    let mut best: Option<(usize, i32, i32, bool, usize)> = None;

    for (i, (_, r)) in rects.iter().enumerate() {
        if i == ai { continue; }

        let (edge_score, perp_overlap) = match dir {
            // Going right, wrap to leftmost → prefer smallest x
            FocusDir::Right => {
                (r.x as i32, ranges_overlap(r.y, r.height, arect.y, arect.height))
            }
            // Going left, wrap to rightmost → prefer largest x+width (negate)
            FocusDir::Left => {
                (-((r.x + r.width) as i32), ranges_overlap(r.y, r.height, arect.y, arect.height))
            }
            // Going down, wrap to topmost → prefer smallest y
            FocusDir::Down => {
                (r.y as i32, ranges_overlap(r.x, r.width, arect.x, arect.width))
            }
            // Going up, wrap to bottommost → prefer largest y+height (negate)
            FocusDir::Up => {
                (-((r.y + r.height) as i32), ranges_overlap(r.x, r.width, arect.x, arect.width))
            }
        };

        let rcx = r.x as i32 * 2 + r.width as i32;
        let rcy = r.y as i32 * 2 + r.height as i32;
        let perp_dist = match dir {
            FocusDir::Left | FocusDir::Right => (rcy - acy).abs(),
            FocusDir::Up | FocusDir::Down => (rcx - acx).abs(),
        };

        let rank = pane_ids.get(i)
            .map(|id| crate::tree::mru_rank(pane_mru, *id))
            .unwrap_or(usize::MAX);

        let dominated = if let Some((_, be, bd, bo, br)) = best {
            if perp_overlap && !bo {
                false
            } else if !perp_overlap && bo {
                true
            } else if edge_score < be {
                false
            } else if edge_score > be {
                true
            } else if perp_overlap && bo {
                // Both overlap with same edge score → MRU (tmux parity #70)
                rank >= br
            } else if perp_dist < bd {
                false
            } else if perp_dist > bd {
                true
            } else {
                rank >= br  // same geometry → MRU tie-break
            }
        } else {
            false
        };

        if !dominated {
            best = Some((i, edge_score, perp_dist, perp_overlap, rank));
        }
    }

    // Tmux parity (#141): wrapped navigation must stay within the same
    // column (U/D) or row (L/R). If no candidate overlaps on the
    // perpendicular axis, the pane is alone in its row/column and
    // navigation should stay put (no-op) rather than jump sideways.
    best.filter(|(_, _, _, has_overlap, _)| *has_overlap)
        .map(|(idx, _, _, _, _)| idx)
}

/// Encode a crossterm `KeyEvent` into the byte sequence that should be
/// written to the child PTY.  Extracted as a standalone function so it can
/// be unit-tested without needing a full `AppState`.
///
/// Returns `None` for key codes we don't handle (F-keys, etc.).
/// Compute xterm modifier parameter: 1 + Shift*1 + Alt*2 + Ctrl*4.
/// Returns 1 when no modifiers are held (callers use >1 to decide whether to
/// emit the extended `;mod` form).
fn modifier_param(mods: KeyModifiers) -> u8 {
    let mut m: u8 = 1;
    if mods.contains(KeyModifiers::SHIFT) { m += 1; }
    if mods.contains(KeyModifiers::ALT) { m += 2; }
    if mods.contains(KeyModifiers::CONTROL) { m += 4; }
    m
}

/// Parse modifier+special key names like "C-Left", "S-Right", "C-S-Up",
/// "C-M-Home", etc. and return the xterm escape sequence.
/// Returns None if the string isn't a recognized modified special key.
pub fn parse_modified_special_key(s: &str) -> Option<String> {
    let upper = s.to_uppercase();
    // Extract modifier prefixes and base key name
    let mut rest = upper.as_str();
    let mut bits: u8 = 0;
    loop {
        if rest.starts_with("C-") { bits |= 4; rest = &rest[2..]; }
        else if rest.starts_with("M-") { bits |= 2; rest = &rest[2..]; }
        else if rest.starts_with("S-") { bits |= 1; rest = &rest[2..]; }
        else { break; }
    }
    if bits == 0 { return None; } // no modifiers found
    let m = bits + 1; // xterm modifier param = 1 + modifier bits
    // Match the base key name
    match rest {
        "ENTER" | "RETURN" | "CR" => Some(format!("\x1b[13;{}~", m)),
        "TAB" => Some(format!("\x1b[9;{}~", m)),
        "BTAB" | "BACKTAB" => {
            // Shift is implicit in BackTab; ensure Shift bit is set in the bitmask
            let sm = (bits | 1) + 1;
            Some(format!("\x1b[9;{}~", sm))
        }
        "LEFT" => Some(format!("\x1b[1;{}D", m)),
        "RIGHT" => Some(format!("\x1b[1;{}C", m)),
        "UP" => Some(format!("\x1b[1;{}A", m)),
        "DOWN" => Some(format!("\x1b[1;{}B", m)),
        "HOME" => Some(format!("\x1b[1;{}H", m)),
        "END" => Some(format!("\x1b[1;{}F", m)),
        "INSERT" | "IC" => Some(format!("\x1b[2;{}~", m)),
        "DELETE" | "DC" => Some(format!("\x1b[3;{}~", m)),
        "PAGEUP" | "PPAGE" => Some(format!("\x1b[5;{}~", m)),
        "PAGEDOWN" | "NPAGE" => Some(format!("\x1b[6;{}~", m)),
        s if s.starts_with('F') && s.len() >= 2 => {
            if let Ok(n) = s[1..].parse::<u8>() {
                let seq = encode_fkey(n, m);
                if seq.is_empty() { None } else { Some(String::from_utf8_lossy(&seq).into_owned()) }
            } else { None }
        }
        _ => None,
    }
}

/// Encode an F-key with optional xterm modifier parameter.
fn encode_fkey(n: u8, m: u8) -> Vec<u8> {
    // F1-F4 use SS3 when unmodified, CSI with modifier when modified.
    let (prefix, num) = match n {
        1 => if m > 1 { ("", Some((11, 'P'))) } else { return b"\x1bOP".to_vec() },
        2 => if m > 1 { ("", Some((12, 'Q'))) } else { return b"\x1bOQ".to_vec() },
        3 => if m > 1 { ("", Some((13, 'R'))) } else { return b"\x1bOR".to_vec() },
        4 => if m > 1 { ("", Some((14, 'S'))) } else { return b"\x1bOS".to_vec() },
        5 => ("", Some((15, '~'))),
        6 => ("", Some((17, '~'))),
        7 => ("", Some((18, '~'))),
        8 => ("", Some((19, '~'))),
        9 => ("", Some((20, '~'))),
        10 => ("", Some((21, '~'))),
        11 => ("", Some((23, '~'))),
        12 => ("", Some((24, '~'))),
        _ => return Vec::new(),
    };
    let _ = prefix;
    if let Some((code, suffix)) = num {
        if suffix == '~' {
            if m > 1 { format!("\x1b[{};{}~", code, m).into_bytes() }
            else { format!("\x1b[{}~", code).into_bytes() }
        } else {
            // F1-F4 modified: \x1b[1;{mod}P/Q/R/S
            format!("\x1b[1;{}{}", m, suffix).into_bytes()
        }
    } else {
        Vec::new()
    }
}

pub fn encode_key_event(key: &KeyEvent) -> Option<Vec<u8>> {
    let encoded: Vec<u8> = match key.code {
        // AltGr detection: On Windows, AltGr is reported as Ctrl+Alt by the
        // console subsystem / crossterm.  International keyboards (German,
        // Czech, Polish, …) use AltGr to produce characters like \ @ { } [ ]
        // | ~ €.  crossterm delivers these as KeyCode::Char(produced_char)
        // with CONTROL|ALT modifiers.  The produced character is NOT an ASCII
        // letter (a-z), so we can distinguish AltGr from genuine Ctrl+Alt
        // combos and forward the character as-is.
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::ALT)
            && !c.is_ascii_lowercase() => {
            // AltGr-produced character — forward it verbatim (UTF-8).
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf);
            buf[..c.len_utf8()].to_vec()
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::ALT) => {
            // Genuine Ctrl+Alt+letter — encode as ESC + ctrl-char.
            let ctrl_char = (c as u8) & 0x1F;
            vec![0x1b, ctrl_char]
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
            format!("\x1b{}", c).into_bytes()
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let ctrl_char = (c as u8) & 0x1F;
            vec![ctrl_char]
        }
        KeyCode::Char(c) if (c as u32) >= 0x01 && (c as u32) <= 0x1A => {
            vec![c as u8]
        }
        KeyCode::Char(c) => {
            format!("{}", c).into_bytes()
        }
        KeyCode::Enter => {
            let m = modifier_param(key.modifiers);
            if m > 1 {
                // On Windows, CSI 13;mod~ is non-standard and dropped by ConPTY.
                // Send ESC+CR (\x1b\r) for Shift/Alt+Enter — the same bytes VS Code's
                // xterm.js sends.  libuv preserves ESC as Alt prefix, so Node.js apps
                // (Claude Code) receive \x1b\r and interpret it as Shift+Enter.
                // Ctrl+Enter and Ctrl+Shift+Enter still use CSI encoding (those are
                // less common and consumed by other layers).
                #[cfg(windows)]
                {
                    let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    if !has_ctrl {
                        return Some(b"\x1b\r".to_vec());
                    }
                }
                // Non-Windows or Ctrl combos: xterm modified-Enter: CSI 13 ; mod ~
                format!("\x1b[13;{}~", m).into_bytes()
            } else {
                b"\r".to_vec()
            }
        }
        KeyCode::Tab => {
            let m = modifier_param(key.modifiers);
            if m > 1 {
                // xterm modified-Tab: CSI 9 ; mod ~
                format!("\x1b[9;{}~", m).into_bytes()
            } else {
                b"\t".to_vec()
            }
        }
        KeyCode::BackTab => {
            let m = modifier_param(key.modifiers);
            if m > 1 {
                // Shift is implicit in BackTab; add it back for the modifier param
                let sm = m | 1; // ensure Shift bit is set
                format!("\x1b[9;{}~", sm).into_bytes()
            } else {
                b"\x1b[Z".to_vec()
            }
        }
        KeyCode::Backspace => b"\x08".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        // Arrow keys and special keys with xterm modifier encoding.
        // Format: \x1b[1;{mod}{letter} where mod = 1 + Shift*1 + Alt*2 + Ctrl*4
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down |
        KeyCode::Home | KeyCode::End => {
            let letter = match key.code {
                KeyCode::Up => 'A', KeyCode::Down => 'B',
                KeyCode::Right => 'C', KeyCode::Left => 'D',
                KeyCode::Home => 'H', KeyCode::End => 'F',
                _ => unreachable!(),
            };
            let m = modifier_param(key.modifiers);
            if m > 1 {
                format!("\x1b[1;{}{}", m, letter).into_bytes()
            } else {
                format!("\x1b[{}", letter).into_bytes()
            }
        }
        // Tilde-style keys: \x1b[{N};{mod}~ when modifiers present
        KeyCode::Insert | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match key.code {
                KeyCode::Insert => 2, KeyCode::Delete => 3,
                KeyCode::PageUp => 5, KeyCode::PageDown => 6,
                _ => unreachable!(),
            };
            let m = modifier_param(key.modifiers);
            if m > 1 {
                format!("\x1b[{};{}~", n, m).into_bytes()
            } else {
                format!("\x1b[{}~", n).into_bytes()
            }
        }
        KeyCode::F(n) => {
            let m = modifier_param(key.modifiers);
            encode_fkey(n, m)
        }
        _ => return None,
    };
    Some(encoded)
}

pub fn forward_key_to_active(app: &mut AppState, key: KeyEvent) -> io::Result<()> {
    // On Windows, modified Enter delivery depends on the modifier:
    //
    // Shift/Alt+Enter (no Ctrl): Use VT encoding ONLY (\x1b\r).  Native
    //   WriteConsoleInputW injection would cause ConPTY to translate the
    //   KEY_EVENT back to plain \r, so VT-native apps (Claude Code) see a
    //   double Enter.
    //
    // Ctrl+Enter / Ctrl+Shift+Enter: Use native injection ONLY.  ConPTY
    //   cannot encode Ctrl+Enter in VT, so injection is the only reliable
    //   path for console apps (PSReadLine).  Falls back to xterm CSI
    //   encoding (\x1b[13;N~) if injection fails (for non-console apps).
    #[cfg(windows)]
    {
        if matches!(key.code, KeyCode::Enter) && !key.modifiers.is_empty() {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);

            // Only use native injection when Ctrl is involved.
            if ctrl {
                let try_inject = |pane: &mut Pane| -> bool {
                    if let Some(pid) = pane.child_pid {
                        crate::platform::mouse_inject::send_modified_enter_event(pid, ctrl, alt, shift)
                    } else {
                        false
                    }
                };

                if app.sync_input {
                    let win = &mut app.windows[app.active_idx];
                    fn inject_all(node: &mut Node, ctrl: bool, alt: bool, shift: bool) {
                        match node {
                            Node::Leaf(p) if !p.dead => {
                                if let Some(pid) = p.child_pid {
                                    if !crate::platform::mouse_inject::send_modified_enter_event(pid, ctrl, alt, shift) {
                                        // Fallback: xterm CSI encoding for non-console apps
                                        let m: u8 = 1 + (shift as u8) + (alt as u8) * 2 + (ctrl as u8) * 4;
                                        let bytes = if m > 1 { format!("\x1b[13;{}~", m).into_bytes() } else { b"\r".to_vec() };
                                        let _ = p.writer.write_all(&bytes);
                                        let _ = p.writer.flush();
                                    }
                                }
                            }
                            Node::Leaf(_) => {}
                            Node::Split { children, .. } => {
                                for c in children { inject_all(c, ctrl, alt, shift); }
                            }
                        }
                    }
                    inject_all(&mut win.root, ctrl, alt, shift);
                    return Ok(());
                } else {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if !active.dead {
                            if try_inject(active) {
                                return Ok(());
                            }
                            // Fallback: VT encoding (falls through below)
                        }
                    }
                }
            }
            // Shift/Alt+Enter (no Ctrl): fall through to VT encoding below.
        }
    }

    let encoded = match encode_key_event(&key) {
        Some(bytes) => bytes,
        None => return Ok(()),
    };

    if app.sync_input {
        // Fan out to ALL panes in the current window
        let win = &mut app.windows[app.active_idx];
        fn write_all_panes(node: &mut Node, data: &[u8]) {
            match node {
                Node::Leaf(p) if !p.dead => { let _ = p.writer.write_all(data); let _ = p.writer.flush(); }
                Node::Leaf(_) => {}
                Node::Split { children, .. } => { for c in children { write_all_panes(c, data); } }
            }
        }
        write_all_panes(&mut win.root, &encoded);

    } else {
        let win = &mut app.windows[app.active_idx];
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if !active.dead {
                let _ = active.writer.write_all(&encoded);
                let _ = active.writer.flush();

            }
        }
    }
    Ok(())
}

fn wheel_cell_for_area(area: Rect, x: u16, y: u16) -> (u16, u16) {
    // Convert global terminal coordinates to 1-based pane-local coordinates.
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_w = area.width.saturating_sub(2).max(1);
    let inner_h = area.height.saturating_sub(2).max(1);

    let col = x
        .saturating_sub(inner_x)
        .min(inner_w.saturating_sub(1))
        .saturating_add(1);
    let row = y
        .saturating_sub(inner_y)
        .min(inner_h.saturating_sub(1))
        .saturating_add(1);
    (col, row)
}

/// Paste the system clipboard content into the active pane.
/// This is the Windows Terminal right-click-to-paste behavior.
fn paste_clipboard_to_active(app: &mut AppState) -> io::Result<()> {
    if let Some(text) = crate::copy_mode::read_from_system_clipboard() {
        if !text.is_empty() {
            send_paste_to_active(app, &text)?;
        }
    }
    Ok(())
}

/// Forward a mouse event to the child pane.
///
/// If the child has mouse protocol enabled (TUI app running), write VT mouse
/// sequences directly to the ConPTY input pipe (pane.writer).  Modern TUI
/// apps (crossterm, etc.) use VT input mode (ReadFile + ENABLE_VIRTUAL_TERMINAL_INPUT)
/// and receive these directly through stdin.  If VT input mode is off, ConPTY
/// parses the VT and converts to MOUSE_EVENT records for ReadConsoleInputW apps.
///
/// When mouse protocol is NOT enabled (shell prompt), use Win32 MOUSE_EVENT
/// injection as a harmless fallback (most programs ignore it).
fn forward_mouse_to_pane(pane: &mut Pane, area: Rect, abs_x: u16, abs_y: u16, button_state: u32, event_flags: u32) {
    forward_mouse_to_pane_ex(pane, area, abs_x, abs_y, button_state, event_flags, 0xff, false);
}

/// Forward a mouse event to a child pane by writing SGR mouse sequences
/// to the ConPTY input pipe — the same mechanism Windows Terminal uses.
///
/// ConPTY/conhost automatically translates SGR mouse sequences into
/// MOUSE_EVENT records for crossterm/ratatui apps (ReadConsoleInputW),
/// and passes VT through for nvim/vim apps.  (fixes #60)
fn forward_mouse_to_pane_ex(pane: &mut Pane, area: Rect, abs_x: u16, abs_y: u16,
                             _button_state: u32, _event_flags: u32,
                             vt_button: u8, press: bool) {
    let col = abs_x as i16 - area.x as i16;
    let row = abs_y as i16 - area.y as i16;
    crate::window_ops::inject_mouse_combined(
        pane, col, row, vt_button, press, 0, 0, "client");
}

pub fn handle_mouse(app: &mut AppState, me: MouseEvent, window_area: Rect) -> io::Result<()> {
    use crossterm::event::{MouseEventKind, MouseButton};

    // Track last mouse position for #{mouse_x}, #{mouse_y} format variables
    app.last_mouse_x = me.column;
    app.last_mouse_y = me.row;

    // --- MenuMode: handle mouse clicks on menu items ---
    if let Mode::MenuMode { ref mut menu } = app.mode {
        if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) {
            // Recompute menu_area the same way as the renderer (app.rs).
            let full_area = Rect {
                x: 0, y: 0,
                width: window_area.width,
                height: window_area.height + app.status_lines as u16,
            };
            let item_count = menu.items.len();
            let height = (item_count as u16 + 2).min(20);
            let width = menu.items.iter().map(|i| i.name.len()).max().unwrap_or(10).max(menu.title.len()) as u16 + 8;
            let menu_area = if let (Some(x), Some(y)) = (menu.x, menu.y) {
                let x = if x < 0 { (full_area.width as i16 + x).max(0) as u16 } else { x as u16 };
                let y = if y < 0 { (full_area.height as i16 + y).max(0) as u16 } else { y as u16 };
                Rect { x: x.min(full_area.width.saturating_sub(width)), y: y.min(full_area.height.saturating_sub(height)), width, height }
            } else {
                crate::rendering::centered_rect((width * 100 / full_area.width.max(1)).max(30), height, full_area)
            };
            let pos = ratatui::layout::Position { x: me.column, y: me.row };
            if menu_area.contains(pos) {
                // Block border is 1 row top
                let inner_y = me.row.saturating_sub(menu_area.y + 1);
                let idx = inner_y as usize;
                if idx < menu.items.len() && !menu.items[idx].is_separator && !menu.items[idx].command.is_empty() {
                    let cmd = menu.items[idx].command.clone();
                    app.mode = Mode::Passthrough;
                    let _ = execute_command_string(app, &cmd);
                } else {
                    app.mode = Mode::Passthrough;
                }
            } else {
                app.mode = Mode::Passthrough;
            }
            return Ok(());
        }
        if matches!(me.kind, MouseEventKind::ScrollUp) {
            if menu.selected > 0 {
                menu.selected -= 1;
                while menu.selected > 0 && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                    menu.selected -= 1;
                }
            }
            return Ok(());
        }
        if matches!(me.kind, MouseEventKind::ScrollDown) {
            if menu.selected + 1 < menu.items.len() {
                menu.selected += 1;
                while menu.selected + 1 < menu.items.len() && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                    menu.selected += 1;
                }
            }
            return Ok(());
        }
        return Ok(());
    }

    // Customize mode: absorb all mouse events
    if matches!(app.mode, Mode::CustomizeMode { .. }) {
        return Ok(());
    }

    // --- Tab click: check if click is on the status bar row ---
    let status_row = window_area.y + window_area.height; // status bar is 1 row below window area
    if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) && me.row == status_row {
        for &(win_idx, x_start, x_end) in app.tab_positions.iter() {
            if me.column >= x_start && me.column < x_end {
                if win_idx < app.windows.len() {
                    switch_with_copy_save(app, |app| {
                        app.last_window_idx = app.active_idx;
                        app.active_idx = win_idx;
                    });
                }
                return Ok(());
            }
        }
        // Click was on status bar but not on a tab — ignore
        return Ok(());
    }

    // If a left-click lands on a different pane while in copy mode,
    // exit copy mode entirely and switch to the clicked pane (tmux parity #62).
    if matches!(me.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left))
        && matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. })
    {
        let win = &app.windows[app.active_idx];
        let mut rects_check: Vec<(Vec<usize>, Rect)> = Vec::new();
        compute_rects(&win.root, window_area, &mut rects_check);
        let mut clicked_new_path: Option<Vec<usize>> = None;
        for (path, area) in rects_check.iter() {
            if area.contains(ratatui::layout::Position { x: me.column, y: me.row }) {
                if *path != win.active_path {
                    clicked_new_path = Some(path.clone());
                }
                break;
            }
        }
        if let Some(np) = clicked_new_path {
            // Exit copy mode cleanly (resets scroll, clears selection)
            exit_copy_mode(app);
            // Switch active pane path
            {
                let win = &mut app.windows[app.active_idx];
                app.last_pane_path = win.active_path.clone();
                win.active_path = np;
            }
        }
    }

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, window_area, &mut rects);
    let mut borders: Vec<(Vec<usize>, LayoutKind, usize, u16, u16)> = Vec::new();
    compute_split_borders(&win.root, window_area, &mut borders);
    let mut active_area = rects
        .iter()
        .find(|(path, _)| *path == win.active_path)
        .map(|(_, area)| *area);

    // Helper: convert absolute screen coordinates to 0-based pane-local
    // (row, col) for copy-mode cursor positioning.  Mirrors
    // `copy_cell_for_area` in window_ops.rs.
    fn copy_cell(area: Rect, abs_x: u16, abs_y: u16) -> (u16, u16) {
        let col = abs_x.saturating_sub(area.x).min(area.width.saturating_sub(1));
        let row = abs_y.saturating_sub(area.y).min(area.height.saturating_sub(1));
        (row, col)
    }

    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });

    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // ── Copy-mode: left click positions cursor, clears selection ──
            // tmux parity: single click moves cursor without starting a selection.
            // Selection only starts when dragging (see Drag handler below).
            if in_copy {
                if let Some(area) = active_area {
                    let (row, col) = copy_cell(area, me.column, me.row);
                    app.copy_anchor = None;
                    app.copy_pos = Some((row, col));
                }
                return Ok(());
            }

            // Check if click is on a split border (for dragging)
            let mut on_border = false;
            let tol = 1u16;
            for (path, kind, idx, pos, total_px) in borders.iter() {
                match kind {
                    LayoutKind::Horizontal => {
                        if me.column >= pos.saturating_sub(tol) && me.column <= pos + tol {
                            if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) {
                                app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: *pos, start_y: me.row, left_initial: left, _right_initial: right, total_pixels: *total_px });
                            }
                            on_border = true;
                            break;
                        }
                    }
                    LayoutKind::Vertical => {
                        if me.row >= pos.saturating_sub(tol) && me.row <= pos + tol {
                            if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) {
                                app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: me.column, start_y: *pos, left_initial: left, _right_initial: right, total_pixels: *total_px });
                            }
                            on_border = true;
                            break;
                        }
                    }
                }
            }

            // Switch pane focus if clicking inside a pane
            for (path, area) in rects.iter() {
                if area.contains(ratatui::layout::Position { x: me.column, y: me.row }) {
                    win.active_path = path.clone();
                    // Update MRU for clicked pane
                    if let Some(pid) = crate::tree::get_active_pane_id(&win.root, path) {
                        crate::tree::touch_mru(&mut win.pane_mru, pid);
                    }
                    active_area = Some(*area);
                }
            }

            // Forward left-click only when active pane wants mouse input.
            if !on_border {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if crate::window_ops::pane_wants_mouse(active) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row,
                                crate::platform::mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED, 0,
                                0, true); // SGR button 0 = left, press
                        }
                    }
                }
            }

        }
        MouseEventKind::Down(MouseButton::Right) => {
            // Windows Terminal behaviour: right-click = paste clipboard.
            // When the child has mouse tracking enabled (TUI app), forward
            // the right-click to the app instead.
            if in_copy {
                // In copy mode: paste clipboard (like Windows Terminal)
                let _ = paste_clipboard_to_active(app);
                return Ok(());
            }
            // Forward right-click only when active pane wants mouse input.
            let wants_mouse = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| crate::window_ops::pane_wants_mouse(p));
            if wants_mouse {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if crate::window_ops::pane_wants_mouse(active) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row,
                                crate::platform::mouse_inject::RIGHTMOST_BUTTON_PRESSED, 0,
                                2, true); // SGR button 2 = right, press
                        }
                    }
                }
            } else {
                // Shell prompt — paste clipboard (Windows Terminal parity)
                let _ = paste_clipboard_to_active(app);
                return Ok(());
            }
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            // In copy mode, suppress — don't forward to child
            if in_copy { return Ok(()); }
            if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            crate::platform::mouse_inject::FROM_LEFT_2ND_BUTTON_PRESSED, 0,
                            1, true); // SGR button 1 = middle, press
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // ── Copy-mode: left release finalises position, auto-yank if selection ──
            if in_copy {
                if let Some(area) = active_area {
                    let (row, col) = copy_cell(area, me.column, me.row);
                    app.copy_pos = Some((row, col));
                }
                // Auto-yank if there is a selection (anchor != pos) — tmux parity
                if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                    if a != p {
                        let _ = yank_selection(app);
                        // tmux parity #62: auto-exit copy mode after mouse yank
                        exit_copy_mode(app);
                    }
                }
                return Ok(());
            }

            let was_dragging = app.drag.is_some();
            app.drag = None;
            if was_dragging {
                resize_all_panes(app);
            } else if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row, 0, 0,
                            0, false); // SGR button 0 = left, release
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Right) => {
            if in_copy { return Ok(()); }
            // Forward right-release only when active pane wants mouse input.
            let wants_mouse = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| crate::window_ops::pane_wants_mouse(p));
            if wants_mouse {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if crate::window_ops::pane_wants_mouse(active) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row, 0, 0,
                                2, false); // SGR button 2 = right, release
                        }
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Middle) => {
            if in_copy { return Ok(()); }
            if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row, 0, 0,
                            1, false); // SGR button 1 = middle, release
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // ── Copy-mode: drag extends the selection ──
            if in_copy {
                if let Some(area) = active_area {
                    let (row, col) = copy_cell(area, me.column, me.row);
                    if app.copy_anchor.is_none() {
                        app.copy_anchor = Some((row, col));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_selection_mode = crate::types::SelectionMode::Char;
                    }
                    app.copy_pos = Some((row, col));
                    // tmux parity #62: auto-scroll when dragging at pane edges
                    if me.row <= area.y {
                        scroll_copy_up(app, 1);
                    } else if me.row >= area.y + area.height.saturating_sub(1) {
                        scroll_copy_down(app, 1);
                    }
                }
                return Ok(());
            }

            if let Some(d) = &app.drag {
                adjust_split_sizes(&mut win.root, d, me.column, me.row);
            } else {
                // tmux parity #62: drag from normal mode enters copy mode
                // and starts selection (when child doesn't want mouse).
                let wants_mouse = {
                    let win2 = &app.windows[app.active_idx];
                    active_pane(&win2.root, &win2.active_path)
                        .map_or(false, |p| crate::window_ops::pane_wants_mouse(p))
                };
                if wants_mouse {
                    if let Some(area) = active_area {
                        let win2 = &mut app.windows[app.active_idx];
                        if let Some(active) = active_pane_mut(&mut win2.root, &win2.active_path) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row,
                                crate::platform::mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED,
                                crate::platform::mouse_inject::MOUSE_MOVED,
                                32, true); // SGR button 32 = left-drag
                        }
                    }
                } else {
                    // Shell prompt: enter copy mode, start selection
                    enter_copy_mode(app);
                    if let Some(area) = active_area {
                        let (row, col) = copy_cell(area, me.column, me.row);
                        app.copy_anchor = Some((row, col));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_selection_mode = crate::types::SelectionMode::Char;
                        app.copy_pos = Some((row, col));
                    }
                }
            }
        }
        MouseEventKind::Moved => {
            // Forward bare mouse motion (hover) only when active pane
            // explicitly wants mouse input. This avoids sending raw
            // SGR motion bytes (ESC[<35;...) to shell-like prompts.
            if app.last_hover_pos == Some((me.column, me.row)) {
                return Ok(());
            }
            app.last_hover_pos = Some((me.column, me.row));

            if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            0, crate::platform::mouse_inject::MOUSE_MOVED,
                            35, true);
                    }
                }
            }
        }
        MouseEventKind::ScrollUp => {
            // Ignore scroll in popup mode — don't enter copy-mode (#110)
            if matches!(app.mode, Mode::PopupMode { .. }) {
                return Ok(());
            }
            if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
                scroll_copy_up(app, 3);
                return Ok(());
            }
            if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x: me.column, y: me.row })) {
                win.active_path = path.clone();
                active_area = Some(*area);
            }
            // tmux parity: Only forward scroll to child if alternate screen
            // is active (real TUI app like nvim/htop).  If not (shell prompt),
            // auto-enter copy mode.
            //
            // We only check alternate_screen() — NOT is_fullscreen_tui() or
            // pane_wants_mouse() — because:
            //   - is_fullscreen_tui() false-positives when shell output (ls/dir)
            //     fills the screen, preventing scroll-to-copy-mode.
            //   - pane_wants_mouse() false-positives from PSReadLine's spurious
            //     mouse tracking on ConPTY.
            //   - alternate_screen() is reliable: all TUI apps report it correctly.
            let child_in_alt = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| {
                    if let Ok(parser) = p.term.lock() {
                        return parser.screen().alternate_screen();
                    }
                    false
                });
            if child_in_alt {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        let wheel_delta: i16 = 120;
                        let button_state = ((wheel_delta as i32) << 16) as u32;
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            button_state, crate::platform::mouse_inject::MOUSE_WHEELED,
                            64, true); // SGR button 64 = scroll-up
                    }
                }
            } else if app.scroll_enter_copy_mode {
                // Shell prompt — auto-enter copy mode and scroll (tmux parity)
                enter_copy_mode(app);
                scroll_copy_up(app, 3);
                return Ok(());
            } else {
                scroll_pane_scrollback(app, 3, true);
            }
        }
        MouseEventKind::ScrollDown => {
            // Ignore scroll in popup mode — don't enter copy-mode (#110)
            if matches!(app.mode, Mode::PopupMode { .. }) {
                return Ok(());
            }
            if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
                scroll_copy_down(app, 3);
                // Auto-exit copy mode when scrolled back to live output
                // (only when no active selection, to avoid losing a selection in progress)
                if app.copy_scroll_offset == 0 && app.copy_anchor.is_none() {
                    exit_copy_mode(app);
                }
                return Ok(());
            }
            if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x: me.column, y: me.row })) {
                win.active_path = path.clone();
                active_area = Some(*area);
            }
            // Forward scroll-down to child only if alternate screen is active
            // (real TUI app).  At a shell prompt, scroll-down without copy
            // mode is a no-op (can't scroll past live output).
            // Only check alternate_screen() — NOT is_fullscreen_tui() — to
            // avoid false-positives when shell output fills the screen.
            let child_in_alt = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| {
                    if let Ok(parser) = p.term.lock() {
                        return parser.screen().alternate_screen();
                    }
                    false
                });
            if child_in_alt {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        let wheel_delta: i16 = -120;
                        let button_state = ((wheel_delta as i32) << 16) as u32;
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            button_state, crate::platform::mouse_inject::MOUSE_WHEELED,
                            65, true); // SGR button 65 = scroll-down
                    }
                }
            } else if !app.scroll_enter_copy_mode {
                scroll_pane_scrollback(app, 3, false);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Chunked PTY write for paste delivery.  The PTY pipe can silently
/// drop bytes when a large payload (140+ lines) is written in a single
/// call because the OS pipe buffer fills up.  We split the text into
/// ~2 KiB chunks with small yields between them so the consumer
/// (shell / PSReadLine / nvim) has time to drain.  Bracket sequences
/// are tiny and always written in one shot.
fn write_paste_chunked(writer: &mut dyn std::io::Write, text: &[u8], bracket: bool) {
    const CHUNK: usize = 512;
    // Normalize line endings to CR for ConPTY.  Clipboard text may arrive
    // with LF (\n) or CRLF (\r\n), but ConPTY's input parser expects CR
    // (\r) for Enter.  Bare LF is misinterpreted by PSReadLine, causing
    // multi-line pastes to appear in reverse order.
    let text = {
        let mut out = Vec::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            if text[i] == b'\r' && i + 1 < text.len() && text[i + 1] == b'\n' {
                out.push(b'\r');
                i += 2; // CRLF → CR
            } else if text[i] == b'\n' {
                out.push(b'\r');
                i += 1; // LF → CR
            } else {
                out.push(text[i]);
                i += 1;
            }
        }
        out
    };
    let text = &text[..];
    if bracket { let _ = writer.write_all(b"\x1b[200~"); }
    let mut offset: usize = 0;
    while offset < text.len() {
        let remaining = (text.len() - offset).min(CHUNK);
        let chunk = &text[offset..offset + remaining];
        match writer.write(chunk) {
            Ok(0) => {
                // Zero bytes written — yield and retry once
                std::thread::sleep(std::time::Duration::from_millis(10));
                match writer.write(chunk) {
                    Ok(n) if n > 0 => { offset += n; }
                    _ => break, // give up on persistent failure
                }
            }
            Ok(n) => { offset += n; }
            Err(_) => break,
        }
        // Yield between chunks to let the consumer drain the buffer
        if offset < text.len() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    if bracket { let _ = writer.write_all(b"\x1b[201~"); }
    let _ = writer.flush();
}

/// Send pasted text to the active pane, wrapping in bracketed-paste
/// sequences (\x1b[200~ … \x1b[201~) when the child has enabled that mode.
/// This is the correct handler for `Event::Paste` (crossterm) and
/// drag-and-drop file paths, ensuring applications like Claude CLI can
/// distinguish paste/drop from typed input.
pub fn send_paste_to_active(app: &mut AppState, text: &str) -> io::Result<()> {
    // In clock mode, any input exits back to passthrough
    if matches!(app.mode, Mode::ClockMode) {
        app.mode = Mode::Passthrough;
        return Ok(());
    }
    // In copy / copy-search modes, treat like regular text
    if matches!(app.mode, Mode::CopyMode) {
        return send_text_to_active(app, text);
    }
    if matches!(app.mode, Mode::CopySearch { .. }) {
        return send_text_to_active(app, text);
    }

    // Check if the child requested bracketed paste mode
    let use_bracket = {
        let win = &app.windows[app.active_idx];
        if let Some(p) = crate::tree::active_pane(&win.root, &win.active_path) {
            if let Ok(parser) = p.term.lock() {
                let bp = parser.screen().bracketed_paste();
                crate::debug_log::input_log("paste", &format!("child bracketed_paste()={}", bp));
                bp
            } else {
                crate::debug_log::input_log("paste", "term lock failed");
                false
            }
        } else {
            crate::debug_log::input_log("paste", "no active pane");
            false
        }
    };
    crate::debug_log::input_log("paste", &format!("use_bracket={} text_len={} text_preview={:?}", use_bracket, text.len(), &text.chars().take(100).collect::<String>()));

    // On Windows, bracketed paste delivery is tricky:
    //
    // - ConPTY may strip \x1b[200~/201~ from the PTY input pipe (older Windows).
    // - WriteConsoleInputW can bypass ConPTY, but it sends each byte of the
    //   bracket sequence as a separate KEY_EVENT record.  Apps that read via
    //   ReadConsoleInputW (crossterm-based apps like Helix) cannot reassemble
    //   VT sequences from individual key events, so \x1b[200~ appears as the
    //   literal characters Esc [ 2 0 0 ~ in the editor (issue #98).
    // - Apps that read raw bytes via ReadFile (nvim via libuv) CAN parse the
    //   bracket sequences from console-injected KEY_EVENTs.
    //
    // Strategy: try the PTY pipe first with bracket markers.  This works on
    // newer Windows where ConPTY passes VT input through, and also works for
    // byte-stream readers (nvim).  If the child uses ReadConsoleInputW
    // (crossterm), ConPTY converts the VT bytes to KEY_EVENTs anyway, so the
    // brackets may still not be parsed -- but at least the text content
    // arrives correctly without stray visible bracket characters.
    //
    // For apps where PTY-pipe brackets get stripped by ConPTY, fall back to
    // console injection for the TEXT ONLY (no bracket markers) so the content
    // still arrives reliably.
    #[cfg(windows)]
    {
        if app.sync_input {
            let win = &mut app.windows[app.active_idx];
            fn write_all_panes(node: &mut crate::types::Node, text: &[u8], bracket: bool) {
                match node {
                    crate::types::Node::Leaf(p) => {
                        write_paste_chunked(&mut p.writer, text, bracket);
                    }
                    crate::types::Node::Split { children, .. } => {
                        for c in children { write_all_panes(c, text, bracket); }
                    }
                }
            }
            write_all_panes(&mut win.root, text.as_bytes(), use_bracket);
        } else {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                write_paste_chunked(&mut p.writer, text.as_bytes(), use_bracket);
            }
        }
    }

    // On non-Windows, use standard PTY pipe write with bracket sequences
    #[cfg(not(windows))]
    {
        if app.sync_input {
            let win = &mut app.windows[app.active_idx];
            fn write_paste_all_panes(node: &mut Node, text: &[u8], bracket: bool) {
                match node {
                    Node::Leaf(p) => {
                        write_paste_chunked(&mut p.writer, text, bracket);
                    }
                    Node::Split { children, .. } => {
                        for c in children { write_paste_all_panes(c, text, bracket); }
                    }
                }
            }
            write_paste_all_panes(&mut win.root, text.as_bytes(), use_bracket);
        } else {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                write_paste_chunked(&mut p.writer, text.as_bytes(), use_bracket);
            }
        }
    }
    Ok(())
}

pub fn send_text_to_active(app: &mut AppState, text: &str) -> io::Result<()> {
    // In clock mode, any input exits back to passthrough
    if matches!(app.mode, Mode::ClockMode) {
        app.mode = Mode::Passthrough;
        return Ok(());
    }
    // Route input to active overlay (so CLI send-keys can interact with overlays)
    if matches!(app.mode, Mode::PopupMode { .. }) {
        // Escape (\x1b alone) closes popup; other text goes to popup PTY
        if text == "\x1b" {
            app.mode = Mode::Passthrough;
            return Ok(());
        }
        if let Mode::PopupMode { ref mut popup_pane, .. } = app.mode {
            if let Some(ref mut pty) = popup_pane {
                let _ = pty.writer.write_all(text.as_bytes());
                let _ = pty.writer.flush();
            }
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::ConfirmMode { .. }) {
        for c in text.chars() {
            match c {
                'y' | 'Y' => {
                    if let Mode::ConfirmMode { ref command, .. } = app.mode {
                        let cmd = command.clone();
                        app.mode = Mode::Passthrough;
                        crate::config::parse_config_line(app, &cmd);
                    }
                    return Ok(());
                }
                'n' | 'N' => {
                    app.mode = Mode::Passthrough;
                    return Ok(());
                }
                _ => {} // Ignore other chars during confirm
            }
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::MenuMode { .. }) {
        // Escape closes menu; other text is ignored (menu is navigated via send-key)
        if text == "\x1b" {
            app.mode = Mode::Passthrough;
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::PaneChooser { .. }) {
        // Escape closes display-panes
        if text == "\x1b" {
            app.mode = Mode::Passthrough;
            return Ok(());
        }
        // In display-panes mode, handle digit selection
        for c in text.chars() {
            if c.is_ascii_digit() {
                let idx = c.to_digit(10).unwrap() as usize;
                if let Some((_, path)) = app.display_map.iter().find(|(d, _)| *d == idx) {
                    let path = path.clone();
                    if let Some(win) = app.windows.get_mut(app.active_idx) {
                        win.active_path = path;
                    }
                }
                app.mode = Mode::Passthrough;
                return Ok(());
            }
        }
        return Ok(());
    }
    // In copy mode, interpret characters as copy-mode actions (never send to PTY)
    if matches!(app.mode, Mode::CopyMode) {
        for c in text.chars() {
            handle_copy_mode_char(app, c)?;
        }
        return Ok(());
    }
    // In copy-search mode, append characters to the search input
    if matches!(app.mode, Mode::CopySearch { .. }) {
        if let Mode::CopySearch { ref mut input, .. } = app.mode {
            for c in text.chars() {
                input.push(c);
            }
        }
        return Ok(());
    }

    if app.sync_input {
        // Fan out to ALL panes in the current window
        let win = &mut app.windows[app.active_idx];
        fn write_all_panes(node: &mut Node, text: &[u8]) {
            match node {
                Node::Leaf(p) => { let _ = p.writer.write_all(text); let _ = p.writer.flush(); }
                Node::Split { children, .. } => { for c in children { write_all_panes(c, text); } }
            }
        }
        write_all_panes(&mut win.root, text.as_bytes());
    } else {
        let win = &mut app.windows[app.active_idx];
        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
            let _ = p.writer.write_all(text.as_bytes());
            let _ = p.writer.flush();
        }
    }
    Ok(())
}

/// Dispatch a single character as a copy-mode action.
fn handle_copy_mode_char(app: &mut AppState, c: char) -> io::Result<()> {
    // Handle text-object pending state (waiting for w/W after a/i)
    if let Some(prefix) = app.copy_text_object_pending.take() {
        match (prefix, c) {
            (0, 'w') => { crate::copy_mode::select_a_word(app); }
            (1, 'w') => { crate::copy_mode::select_inner_word(app); }
            (0, 'W') => { crate::copy_mode::select_a_word_big(app); }
            (1, 'W') => { crate::copy_mode::select_inner_word_big(app); }
            _ => {}
        }
        return Ok(());
    }
    // Handle find-char pending state (waiting for char after f/F/t/T)
    if let Some(pending) = app.copy_find_char_pending.take() {
        match pending {
            0 => crate::copy_mode::find_char_forward(app, c),
            1 => crate::copy_mode::find_char_backward(app, c),
            2 => crate::copy_mode::find_char_to_forward(app, c),
            3 => crate::copy_mode::find_char_to_backward(app, c),
            _ => {}
        }
        return Ok(());
    }
    match c {
        'q' | ']' | '\x1b' => {
            exit_copy_mode(app);
        }
        'h' => { move_copy_cursor(app, -1, 0); }
        'l' => { move_copy_cursor(app, 1, 0); }
        'k' => { move_copy_cursor(app, 0, -1); }
        'j' => { move_copy_cursor(app, 0, 1); }
        'g' => { scroll_to_top(app); }
        'G' => { scroll_to_bottom(app); }
        'w' => { crate::copy_mode::move_word_forward(app); }
        'b' => { crate::copy_mode::move_word_backward(app); }
        'e' => { crate::copy_mode::move_word_end(app); }
        'W' => { crate::copy_mode::move_word_forward_big(app); }
        'B' => { crate::copy_mode::move_word_backward_big(app); }
        'E' => { crate::copy_mode::move_word_end_big(app); }
        'H' => { crate::copy_mode::move_to_screen_top(app); }
        'M' => { crate::copy_mode::move_to_screen_middle(app); }
        'L' => { crate::copy_mode::move_to_screen_bottom(app); }
        'f' => { app.copy_find_char_pending = Some(0); }
        'F' => { app.copy_find_char_pending = Some(1); }
        't' => { app.copy_find_char_pending = Some(2); }
        'T' => { app.copy_find_char_pending = Some(3); }
        'D' => { crate::copy_mode::copy_end_of_line(app)?; exit_copy_mode(app); }
        '0' => { crate::copy_mode::move_to_line_start(app); }
        '$' => { crate::copy_mode::move_to_line_end(app); }
        '^' => { crate::copy_mode::move_to_first_nonblank(app); }
        ' ' => {
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
        }
        'v' => {
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
        }
        'V' => {
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
                app.copy_selection_mode = crate::types::SelectionMode::Line;
            }
        }
        'o' => {
            if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                app.copy_anchor = Some(p);
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some(a);
            }
        }
        'A' => {
            if let (Some(_), Some(_)) = (app.copy_anchor, app.copy_pos) {
                let prev = app.paste_buffers.first().cloned().unwrap_or_default();
                yank_selection(app)?;
                if let Some(buf) = app.paste_buffers.first_mut() {
                    let new_text = buf.clone();
                    *buf = format!("{}{}", prev, new_text);
                }
                exit_copy_mode(app);
            }
        }
        'y' => { yank_selection(app)?; exit_copy_mode(app); }
        '/' => { app.mode = Mode::CopySearch { input: String::new(), forward: true }; }
        '?' => { app.mode = Mode::CopySearch { input: String::new(), forward: false }; }
        'n' => { search_next(app); }
        'N' => { search_prev(app); }
        'i' => { app.copy_text_object_pending = Some(1); }  // inner text object
        'a' => { app.copy_text_object_pending = Some(0); }  // a text object
        _ => {} // Swallow unrecognized characters in copy mode
    }
    Ok(())
}

pub fn send_key_to_active(app: &mut AppState, k: &str) -> io::Result<()> {
    // In clock mode, any key exits back to passthrough
    if matches!(app.mode, Mode::ClockMode) {
        app.mode = Mode::Passthrough;
        return Ok(());
    }
    // Route named keys to active overlay (so CLI send-keys can interact with overlays)
    if matches!(app.mode, Mode::PopupMode { .. }) {
        // Map named keys to VT sequences for the popup PTY
        let seq = match k {
            "enter" => Some("\r"),
            "esc" | "escape" => {
                app.mode = Mode::Passthrough;
                return Ok(());
            }
            "tab" => Some("\t"),
            "backspace" | "bspace" => Some("\x7f"),
            "up" => Some("\x1b[A"),
            "down" => Some("\x1b[B"),
            "right" => Some("\x1b[C"),
            "left" => Some("\x1b[D"),
            "home" => Some("\x1b[H"),
            "end" => Some("\x1b[F"),
            "pageup" | "ppage" => Some("\x1b[5~"),
            "pagedown" | "npage" => Some("\x1b[6~"),
            "delete" | "dc" => Some("\x1b[3~"),
            "space" => Some(" "),
            _ => None,
        };
        if let Some(seq) = seq {
            if let Mode::PopupMode { ref mut popup_pane, .. } = app.mode {
                if let Some(ref mut pty) = popup_pane {
                    let _ = pty.writer.write_all(seq.as_bytes());
                    let _ = pty.writer.flush();
                }
            }
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::ConfirmMode { .. }) {
        match k {
            "esc" | "escape" => {
                app.mode = Mode::Passthrough;
            }
            _ => {} // y/n handled via send_text_to_active
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::MenuMode { .. }) {
        match k {
            "up" => {
                if let Mode::MenuMode { ref mut menu } = app.mode {
                    if menu.selected > 0 { menu.selected -= 1; }
                }
            }
            "down" => {
                if let Mode::MenuMode { ref mut menu } = app.mode {
                    let len = menu.items.len();
                    if menu.selected + 1 < len { menu.selected += 1; }
                }
            }
            "enter" => {
                if let Mode::MenuMode { ref menu } = app.mode {
                    if let Some(item) = menu.items.get(menu.selected) {
                        if !item.is_separator && !item.command.is_empty() {
                            let cmd = item.command.clone();
                            app.mode = Mode::Passthrough;
                            crate::config::parse_config_line(app, &cmd);
                            return Ok(());
                        }
                    }
                }
                app.mode = Mode::Passthrough;
            }
            "esc" | "escape" | "q" => {
                app.mode = Mode::Passthrough;
            }
            _ => {}
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::PaneChooser { .. }) {
        match k {
            "esc" | "escape" => {
                app.mode = Mode::Passthrough;
            }
            _ => {}
        }
        return Ok(());
    }
    // --- Copy-search mode: handle esc/enter/backspace ---
    if matches!(app.mode, Mode::CopySearch { .. }) {
        match k {
            "esc" => { app.mode = Mode::CopyMode; }
            "enter" => {
                if let Mode::CopySearch { ref input, forward } = app.mode {
                    let query = input.clone();
                    let fwd = forward;
                    app.copy_search_query = query.clone();
                    app.copy_search_forward = fwd;
                    search_copy_mode(app, &query, fwd);
                    if !app.copy_search_matches.is_empty() {
                        let (r, c, _) = app.copy_search_matches[0];
                        app.copy_pos = Some((r, c));
                    }
                }
                app.mode = Mode::CopyMode;
            }
            "backspace" => {
                if let Mode::CopySearch { ref mut input, .. } = app.mode { input.pop(); }
            }
            _ => {}
        }
        return Ok(());
    }

    // --- Copy mode: full vi-style key table ---
    if matches!(app.mode, Mode::CopyMode) {
        match k {
            "esc" | "q" => {
                exit_copy_mode(app);
            }
            "enter" => {
                // Copy selection and exit copy mode (vi Enter)
                if app.copy_anchor.is_some() {
                    yank_selection(app)?;
                }
                exit_copy_mode(app);
            }
            "space" => {
                // Begin selection (like v in vi mode)
                if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                    app.copy_anchor = Some((r, c));
                    app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                    app.copy_pos = Some((r, c));
                    app.copy_selection_mode = crate::types::SelectionMode::Char;
                }
            }
            "up" => { move_copy_cursor(app, 0, -1); }
            "down" => { move_copy_cursor(app, 0, 1); }
            "pageup" => { scroll_copy_up(app, 10); }
            "pagedown" => { scroll_copy_down(app, 10); }
            "left" => { move_copy_cursor(app, -1, 0); }
            "right" => { move_copy_cursor(app, 1, 0); }
            "home" => { crate::copy_mode::move_to_line_start(app); }
            "end" => { crate::copy_mode::move_to_line_end(app); }
            "C-b" | "c-b" => {
                if app.mode_keys == "emacs" { move_copy_cursor(app, -1, 0); }
                else { scroll_copy_up(app, 10); }
            }
            "C-f" | "c-f" => {
                if app.mode_keys == "emacs" { move_copy_cursor(app, 1, 0); }
                else { scroll_copy_down(app, 10); }
            }
            "C-n" | "c-n" => { move_copy_cursor(app, 0, 1); }
            "C-p" | "c-p" => { move_copy_cursor(app, 0, -1); }
            "C-a" | "c-a" => { crate::copy_mode::move_to_line_start(app); }
            "C-e" | "c-e" => { crate::copy_mode::move_to_line_end(app); }
            "C-v" | "c-v" => { scroll_copy_down(app, 10); }
            "M-v" | "m-v" => { scroll_copy_up(app, 10); }
            "M-f" | "m-f" => { crate::copy_mode::move_word_forward(app); }
            "M-b" | "m-b" => { crate::copy_mode::move_word_backward(app); }
            "M-w" | "m-w" => { yank_selection(app)?; exit_copy_mode(app); }
            "C-s" | "c-s" => { app.mode = Mode::CopySearch { input: String::new(), forward: true }; }
            "C-r" | "c-r" => { app.mode = Mode::CopySearch { input: String::new(), forward: false }; }
            "C-c" | "c-c" => {
                exit_copy_mode(app);
            }
            "C-g" | "c-g" => {
                exit_copy_mode(app);
            }
            "c-space" | "C-space" => {
                // Set mark (anchor) at current position
                if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                    app.copy_anchor = Some((r, c));
                    app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                    app.copy_pos = Some((r, c));
                }
            }
            "C-u" | "c-u" => {
                let half = app.windows.get(app.active_idx)
                    .and_then(|w| active_pane(&w.root, &w.active_path))
                    .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                scroll_copy_up(app, half);
            }
            "C-d" | "c-d" => {
                let half = app.windows.get(app.active_idx)
                    .and_then(|w| active_pane(&w.root, &w.active_path))
                    .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                scroll_copy_down(app, half);
            }
            _ => {}
        }
        return Ok(());
    }
    
    let win = &mut app.windows[app.active_idx];
    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
        match k {
            "enter" => { let _ = write!(p.writer, "\r"); }
            "tab" => { let _ = write!(p.writer, "\t"); }
            "btab" | "backtab" => { let _ = write!(p.writer, "\x1b[Z"); }
            "backspace" => { let _ = p.writer.write_all(&[0x7F]); }
            "delete" => { let _ = write!(p.writer, "\x1b[3~"); }
            "esc" => { let _ = write!(p.writer, "\x1b"); }
            "left" => { let _ = write!(p.writer, "\x1b[D"); }
            "right" => { let _ = write!(p.writer, "\x1b[C"); }
            "up" => { let _ = write!(p.writer, "\x1b[A"); }
            "down" => { let _ = write!(p.writer, "\x1b[B"); }
            "pageup" => { let _ = write!(p.writer, "\x1b[5~"); }
            "pagedown" => { let _ = write!(p.writer, "\x1b[6~"); }
            "home" => { let _ = write!(p.writer, "\x1b[H"); }
            "end" => { let _ = write!(p.writer, "\x1b[F"); }
            "insert" => { let _ = write!(p.writer, "\x1b[2~"); }
            "space" => { let _ = write!(p.writer, " "); }
            s if s.starts_with("f") && s.len() >= 2 && s.len() <= 3 => {
                if let Ok(n) = s[1..].parse::<u8>() {
                    let seq = match n {
                        1 => "\x1bOP",
                        2 => "\x1bOQ",
                        3 => "\x1bOR",
                        4 => "\x1bOS",
                        5 => "\x1b[15~",
                        6 => "\x1b[17~",
                        7 => "\x1b[18~",
                        8 => "\x1b[19~",
                        9 => "\x1b[20~",
                        10 => "\x1b[21~",
                        11 => "\x1b[23~",
                        12 => "\x1b[24~",
                        _ => "",
                    };
                    if !seq.is_empty() { let _ = write!(p.writer, "{}", seq); }
                }
            }
            s if s.starts_with("C-") && s.len() == 3 => {
                let c = s.chars().nth(2).unwrap_or('c');
                let ctrl_char = (c.to_ascii_lowercase() as u8) & 0x1F;
                let _ = p.writer.write_all(&[ctrl_char]);

            }
            s if (s.starts_with("M-") || s.starts_with("m-")) && s.len() == 3 => {
                let c = s.chars().nth(2).unwrap_or('a');
                // Try native console injection (WriteConsoleInputW with LEFT_ALT_PRESSED)
                // first.  ConPTY does NOT reassemble ESC+char into Alt+key events, so
                // PSReadLine Alt+f/Alt+b/etc. won't work via the VT path.
                let injected = if let Some(pid) = p.child_pid {
                    crate::platform::mouse_inject::send_alt_key_event(pid, c)
                } else {
                    false
                };
                if !injected {
                    // Fallback: VT encoding (ESC + char) — works for VT-native apps
                    let _ = write!(p.writer, "\x1b{}", c);
                }
            }
            s if (s.starts_with("C-M-") || s.starts_with("c-m-")) && s.len() == 5 => {
                let c = s.chars().nth(4).unwrap_or('c');
                // Try native console injection (WriteConsoleInputW with
                // LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED).  ConPTY does NOT
                // reassemble ESC + ctrl-char into Ctrl+Alt+key.
                let injected = if let Some(pid) = p.child_pid {
                    crate::platform::mouse_inject::send_modified_key_event(pid, c, true, true, false)
                } else {
                    false
                };
                if !injected {
                    let ctrl_char = (c.to_ascii_lowercase() as u8) & 0x1F;
                    let _ = p.writer.write_all(&[0x1b, ctrl_char]);
                }
            }
            // Modified Enter: for Ctrl combos, try native console injection
            // (WriteConsoleInputW) so PSReadLine sees the correct modifier flags.
            // For Shift/Alt-only combos, use VT encoding to avoid ConPTY
            // translating the injected KEY_EVENT back to plain \r (double Enter).
            #[cfg(windows)]
            s if {
                let u = s.to_uppercase();
                let r = u.trim_start_matches("C-").trim_start_matches("M-").trim_start_matches("S-");
                r == "ENTER" || r == "RETURN" || r == "CR"
            } => {
                let upper = s.to_uppercase();
                let has_shift = upper.contains("S-");
                let has_ctrl = upper.contains("C-");
                let has_alt = upper.contains("M-");
                let injected = if has_ctrl {
                    // Only use native injection for Ctrl combos.
                    if let Some(pid) = p.child_pid {
                        crate::platform::mouse_inject::send_modified_enter_event(pid, has_ctrl, has_alt, has_shift)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !injected {
                    if (has_shift || has_alt) && !has_ctrl {
                        // Fallback: ESC + CR for VT-native apps (Claude Code, etc.)
                        let _ = p.writer.write_all(b"\x1b\r");
                    } else {
                        // Ctrl+Enter and other combos: CSI encoding
                        if let Some(seq) = parse_modified_special_key(s) {
                            let _ = p.writer.write_all(seq.as_bytes());
                        }
                    }
                }
            }
            // Modifier + special key combos: C-Left, S-Right, C-S-Up, C-M-Home, etc.
            s if parse_modified_special_key(s).is_some() => {
                let seq = parse_modified_special_key(s).unwrap();
                let _ = p.writer.write_all(seq.as_bytes());
            }
            _ => {}
        }
        let _ = p.writer.flush();
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests-rs/test_input.rs"]
mod tests;
