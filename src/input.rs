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
    scroll_copy_up, scroll_copy_down, paste_latest, yank_selection,
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
                    create_window(&*pty_system, app, None)?;
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
                KeyCode::Char(' ') => { cycle_top_layout(app); true }
                KeyCode::Char('[') => { enter_copy_mode(app); true }
                KeyCode::Char(']') => { paste_latest(app)?; app.mode = Mode::Passthrough; true }
                KeyCode::Char(':') => {
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
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Passthrough; }
                KeyCode::Enter => {
                    // Save to history before executing
                    if let Mode::CommandPrompt { input, .. } = &app.mode {
                        if !input.is_empty() {
                            let cmd = input.clone();
                            app.command_history.push(cmd);
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
        Mode::PopupMode { ref mut output, ref mut process, close_on_exit, ref mut popup_pty, .. } => {
            let mut should_close = false;
            let mut exit_status: Option<std::process::ExitStatus> = None;
            
            // If we have a PTY popup, forward keys to it
            if let Some(ref mut pty) = popup_pty {
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
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        if let Some(ref mut proc) = process {
                            let _ = proc.kill();
                        }
                        should_close = true;
                    }
                    KeyCode::Char(c) => {
                        output.push(c);
                    }
                    KeyCode::Enter => {
                        output.push('\n');
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
    // Try direct neighbour first, then wrap to opposite edge (tmux parity #61)
    let target = find_best_pane_in_direction(&rects, ai, arect, dir)
        .or_else(|| find_wrap_target(&rects, ai, arect, dir));
    if let Some(ni) = target {
        win.active_path = rects[ni].0.clone();
    }
}

/// Spatial pane navigation: find the best pane in the given direction.
/// Prefers panes that overlap on the perpendicular axis (visually adjacent),
/// then picks the closest by primary-axis gap, tie-broken by perpendicular
/// center-distance for intuitive navigation in asymmetric layouts.
pub fn find_best_pane_in_direction(
    rects: &[(Vec<usize>, Rect)],
    ai: usize,
    arect: &Rect,
    dir: FocusDir,
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

    // (index, primary_gap, perp_center_dist, has_perp_overlap)
    let mut best: Option<(usize, u32, i32, bool)> = None;

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

        let dominated = if let Some((_, bg, bd, bo)) = best {
            // Prefer: (1) perp-overlapping over non-overlapping,
            //         (2) smaller primary gap, (3) smaller perp distance
            if perp_overlap && !bo {
                false  // new candidate has overlap, current best doesn't → new wins
            } else if !perp_overlap && bo {
                true   // current best has overlap, new doesn't → new loses
            } else if primary_gap < bg {
                false  // closer on primary axis
            } else if primary_gap > bg {
                true   // farther on primary axis
            } else {
                perp_dist >= bd  // same gap → pick closer center
            }
        } else {
            false  // no best yet
        };

        if !dominated {
            best = Some((i, primary_gap, perp_dist, perp_overlap));
        }
    }

    best.map(|(idx, _, _, _)| idx)
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
) -> Option<usize> {
    let acx = arect.x as i32 * 2 + arect.width as i32;
    let acy = arect.y as i32 * 2 + arect.height as i32;

    let ranges_overlap = |a_start: u16, a_len: u16, b_start: u16, b_len: u16| -> bool {
        let a_end = a_start + a_len;
        let b_end = b_start + b_len;
        a_start < b_end && b_start < a_end
    };

    // (index, edge_score, perp_center_dist, has_perp_overlap)
    // edge_score: lower = better (closer to the target edge after wrapping)
    let mut best: Option<(usize, i32, i32, bool)> = None;

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

        let dominated = if let Some((_, be, bd, bo)) = best {
            if perp_overlap && !bo {
                false
            } else if !perp_overlap && bo {
                true
            } else if edge_score < be {
                false
            } else if edge_score > be {
                true
            } else {
                perp_dist >= bd
            }
        } else {
            false
        };

        if !dominated {
            best = Some((i, edge_score, perp_dist, perp_overlap));
        }
    }

    best.map(|(idx, _, _, _)| idx)
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
    let mut m: u8 = 1;
    loop {
        if rest.starts_with("C-") { m |= 4; rest = &rest[2..]; }
        else if rest.starts_with("M-") { m |= 2; rest = &rest[2..]; }
        else if rest.starts_with("S-") { m |= 1; rest = &rest[2..]; }
        else { break; }
    }
    if m <= 1 { return None; } // no modifiers found
    // Match the base key name
    match rest {
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
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
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
            } else {
                // Shell prompt — auto-enter copy mode and scroll (tmux parity)
                enter_copy_mode(app);
                scroll_copy_up(app, 3);
                return Ok(());
            }
        }
        MouseEventKind::ScrollDown => {
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
            }
        }
        _ => {}
    }
    Ok(())
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

    // On Windows, ConPTY strips bracketed paste VT sequences (\x1b[200~/201~)
    // from the input pipe.  To deliver them to the child, we must inject
    // directly into the child's console input buffer via WriteConsoleInputW.
    // This bypasses ConPTY's VT input parser entirely.
    #[cfg(windows)]
    {
        use crate::platform::mouse_inject;

        // Helper: resolve child_pid and inject paste via console API
        fn inject_paste_console(p: &mut crate::types::Pane, text: &str, bracket: bool) -> bool {
            if p.child_pid.is_none() {
                p.child_pid = mouse_inject::get_child_pid(&*p.child);
            }
            if let Some(pid) = p.child_pid {
                crate::debug_log::input_log("paste", &format!(
                    "console inject: pid={} bracket={} text_len={}", pid, bracket, text.len()));
                mouse_inject::send_bracketed_paste(pid, text, bracket)
            } else {
                crate::debug_log::input_log("paste", "console inject: no child_pid, falling back to PTY write");
                false
            }
        }

        if app.sync_input {
            let win = &mut app.windows[app.active_idx];
            fn inject_all(node: &mut crate::types::Node, text: &str, bracket: bool) {
                match node {
                    crate::types::Node::Leaf(p) => {
                        if !inject_paste_console(p, text, bracket) {
                            // Fallback: write to PTY pipe (brackets may be stripped)
                            if bracket { let _ = p.writer.write_all(b"\x1b[200~"); }
                            let _ = p.writer.write_all(text.as_bytes());
                            if bracket { let _ = p.writer.write_all(b"\x1b[201~"); }
                            let _ = p.writer.flush();
                        }
                    }
                    crate::types::Node::Split { children, .. } => {
                        for c in children { inject_all(c, text, bracket); }
                    }
                }
            }
            inject_all(&mut win.root, text, use_bracket);
        } else {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                if !inject_paste_console(p, text, use_bracket) {
                    // Fallback: write to PTY pipe
                    if use_bracket { let _ = p.writer.write_all(b"\x1b[200~"); }
                    let _ = p.writer.write_all(text.as_bytes());
                    if use_bracket { let _ = p.writer.write_all(b"\x1b[201~"); }
                    let _ = p.writer.flush();
                }
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
                        if bracket { let _ = p.writer.write_all(b"\x1b[200~"); }
                        let _ = p.writer.write_all(text);
                        if bracket { let _ = p.writer.write_all(b"\x1b[201~"); }
                        let _ = p.writer.flush();
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
                if use_bracket { let _ = p.writer.write_all(b"\x1b[200~"); }
                let _ = p.writer.write_all(text.as_bytes());
                if use_bracket { let _ = p.writer.write_all(b"\x1b[201~"); }
                let _ = p.writer.flush();
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
                let _ = write!(p.writer, "\x1b{}", c);
            }
            s if (s.starts_with("C-M-") || s.starts_with("c-m-")) && s.len() == 5 => {
                let c = s.chars().nth(4).unwrap_or('c');
                let ctrl_char = (c.to_ascii_lowercase() as u8) & 0x1F;
                let _ = p.writer.write_all(&[0x1b, ctrl_char]);
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

// ---------------------------------------------------------------------------
// Unit tests for key encoding — especially the AltGr fix (GitHub issue #15)
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    /// Helper: build a KeyEvent with the given code and modifiers.
    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // ── AltGr characters (Ctrl+Alt on Windows) should be forwarded verbatim ──

    #[test]
    fn altgr_backslash_german_layout() {
        // German: AltGr+ß → '\'   reported as Ctrl+Alt+'\'
        let ev = key(KeyCode::Char('\\'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"\\", "AltGr+backslash must produce literal backslash");
    }

    #[test]
    fn altgr_at_sign_german_layout() {
        // German: AltGr+Q → '@'   reported as Ctrl+Alt+'@'
        let ev = key(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"@", "AltGr+@ must produce literal @");
    }

    #[test]
    fn altgr_open_curly_brace() {
        // German: AltGr+7 → '{'   reported as Ctrl+Alt+'{'
        let ev = key(KeyCode::Char('{'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"{", "AltGr+{{ must produce literal {{");
    }

    #[test]
    fn altgr_close_curly_brace() {
        // German: AltGr+0 → '}'
        let ev = key(KeyCode::Char('}'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"}", "AltGr+}} must produce literal }}");
    }

    #[test]
    fn altgr_open_bracket() {
        // German: AltGr+8 → '['
        let ev = key(KeyCode::Char('['), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"[", "AltGr+[ must produce literal [");
    }

    #[test]
    fn altgr_close_bracket() {
        // German: AltGr+9 → ']'
        let ev = key(KeyCode::Char(']'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"]", "AltGr+] must produce literal ]");
    }

    #[test]
    fn altgr_pipe() {
        // German: AltGr+< → '|'
        let ev = key(KeyCode::Char('|'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"|", "AltGr+| must produce literal |");
    }

    #[test]
    fn altgr_tilde() {
        // German: AltGr++ → '~'
        let ev = key(KeyCode::Char('~'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"~", "AltGr+~ must produce literal ~");
    }

    #[test]
    fn altgr_euro_sign() {
        // German: AltGr+E → '€'   (multi-byte UTF-8)
        let ev = key(KeyCode::Char('€'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, "€".as_bytes(), "AltGr+euro must produce UTF-8 euro sign");
    }

    #[test]
    fn altgr_dollar_czech_layout() {
        // Czech: AltGr produces '$'
        let ev = key(KeyCode::Char('$'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"$", "AltGr+$ must produce literal $");
    }

    // ── Genuine Ctrl+Alt+letter must still produce ESC + ctrl-char ──

    #[test]
    fn ctrl_alt_a_is_esc_ctrl_a() {
        let ev = key(KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, vec![0x1b, 0x01], "Ctrl+Alt+a → ESC + ^A");
    }

    #[test]
    fn ctrl_alt_c_is_esc_ctrl_c() {
        let ev = key(KeyCode::Char('c'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, vec![0x1b, 0x03], "Ctrl+Alt+c → ESC + ^C");
    }

    #[test]
    fn ctrl_alt_z_is_esc_ctrl_z() {
        let ev = key(KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, vec![0x1b, 0x1a], "Ctrl+Alt+z → ESC + ^Z");
    }

    // ── Plain characters / other modifier combos (regression checks) ──

    #[test]
    fn plain_char_no_modifiers() {
        let ev = key(KeyCode::Char('a'), KeyModifiers::NONE);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"a");
    }

    #[test]
    fn alt_a_produces_esc_a() {
        let ev = key(KeyCode::Char('a'), KeyModifiers::ALT);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"\x1ba");
    }

    #[test]
    fn ctrl_a_produces_soh() {
        let ev = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, vec![0x01]); // ^A = SOH
    }

    #[test]
    fn plain_backslash_no_modifiers() {
        let ev = key(KeyCode::Char('\\'), KeyModifiers::NONE);
        let bytes = encode_key_event(&ev).unwrap();
        assert_eq!(bytes, b"\\");
    }
}