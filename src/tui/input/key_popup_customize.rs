use super::*;

pub(crate) fn handle_key_popup_mode(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    if let Mode::PopupMode { ref mut output, ref mut process, close_on_exit, ref mut popup_pane, ref mut scroll_offset, .. } = app.mode {
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
    }
    
    Ok(false)
}

pub(crate) fn handle_key_customize_mode(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    if let Mode::CustomizeMode { ref options, selected: _, ref filter, editing, .. } = app.mode {
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
    }
    Ok(false)
}
