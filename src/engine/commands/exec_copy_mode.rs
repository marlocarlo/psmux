use super::*;

pub(crate) fn handle_copy_mode(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "copy-mode" => {
            enter_copy_mode(app);
        }
        "send-keys" | "send" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                // Local: write key text directly to active pane
                let literal = parts.iter().any(|p| *p == "-l");
                let key_parts: Vec<&str> = parts[1..].iter().filter(|p| !p.starts_with('-')).copied().collect();
                if !key_parts.is_empty() {
                    if literal {
                        let text = key_parts.join(" ");
                        if let Some(win) = app.windows.get_mut(app.active_idx) {
                            if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                                let _ = p.writer.write_all(text.as_bytes());
                                let _ = p.writer.flush();
                            }
                        }
                    } else {
                        for key in &key_parts {
                            let key_upper = key.to_uppercase();
                            let expanded = match key_upper.as_str() {
                                "ENTER" => "\r".to_string(),
                                "TAB" => "\t".to_string(),
                                "BTAB" | "BACKTAB" => "\x1b[Z".to_string(),
                                "ESCAPE" | "ESC" => "\x1b".to_string(),
                                "SPACE" => " ".to_string(),
                                "BSPACE" | "BACKSPACE" => "\x7f".to_string(),
                                "UP" => "\x1b[A".to_string(),
                                "DOWN" => "\x1b[B".to_string(),
                                "RIGHT" => "\x1b[C".to_string(),
                                "LEFT" => "\x1b[D".to_string(),
                                "HOME" => "\x1b[H".to_string(),
                                "END" => "\x1b[F".to_string(),
                                "PAGEUP" | "PPAGE" => "\x1b[5~".to_string(),
                                "PAGEDOWN" | "NPAGE" => "\x1b[6~".to_string(),
                                "DELETE" | "DC" => "\x1b[3~".to_string(),
                                "INSERT" | "IC" => "\x1b[2~".to_string(),
                                "F1" => "\x1bOP".to_string(),
                                "F2" => "\x1bOQ".to_string(),
                                "F3" => "\x1bOR".to_string(),
                                "F4" => "\x1bOS".to_string(),
                                "F5" => "\x1b[15~".to_string(),
                                "F6" => "\x1b[17~".to_string(),
                                "F7" => "\x1b[18~".to_string(),
                                "F8" => "\x1b[19~".to_string(),
                                "F9" => "\x1b[20~".to_string(),
                                "F10" => "\x1b[21~".to_string(),
                                "F11" => "\x1b[23~".to_string(),
                                "F12" => "\x1b[24~".to_string(),
                                s if crate::input::parse_modified_special_key(s).is_some() => {
                                    crate::input::parse_modified_special_key(s).unwrap()
                                }
                                s if s.starts_with("C-M-") || s.starts_with("C-m-") => {
                                    if let Some(c) = key.chars().nth(4) {
                                        let ctrl = (c.to_ascii_lowercase() as u8) & 0x1F;
                                        format!("\x1b{}", ctrl as char)
                                    } else {
                                        key.to_string()
                                    }
                                }
                                s if s.starts_with("C-") => {
                                    if let Some(c) = s.chars().nth(2) {
                                        let ctrl = (c.to_ascii_lowercase() as u8) & 0x1F;
                                        #[cfg(windows)]
                                        if ctrl == 0x03 {
                                            if let Some(win) = app.windows.get_mut(app.active_idx) {
                                                if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                                                    if p.child_pid.is_none() {
                                                        p.child_pid = crate::platform::mouse_inject::get_child_pid(&*p.child);
                                                    }
                                                    if let Some(pid) = p.child_pid {
                                                        crate::platform::send_ctrl_c_event(pid, false);
                                                    }
                                                }
                                            }
                                        }
                                        String::from(ctrl as char)
                                    } else {
                                        key.to_string()
                                    }
                                }
                                s if s.starts_with("M-") => {
                                    if let Some(c) = key.chars().nth(2) {
                                        format!("\x1b{}", c)
                                    } else {
                                        key.to_string()
                                    }
                                }
                                _ => key.to_string(),
                            };
                            if let Some(win) = app.windows.get_mut(app.active_idx) {
                                if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                                    let _ = p.writer.write_all(expanded.as_bytes());
                                    let _ = p.writer.flush();
                                }
                            }
                        }
                    }
                }
            }
        }
        "send-prefix" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "send-prefix\n", &app.session_key);
            } else {
                // Send the prefix key to the active pane as if typed
                let prefix = app.prefix_key;
                let encoded: Vec<u8> = match prefix.0 {
                    crossterm::event::KeyCode::Char(c) if prefix.1.contains(crossterm::event::KeyModifiers::CONTROL) => {
                        vec![(c.to_ascii_lowercase() as u8) & 0x1F]
                    }
                    crossterm::event::KeyCode::Char(c) => format!("{}", c).into_bytes(),
                    _ => vec![],
                };
                if !encoded.is_empty() {
                    if let Some(win) = app.windows.get_mut(app.active_idx) {
                        if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                            let _ = p.writer.write_all(&encoded);
                            let _ = p.writer.flush();
                        }
                    }
                }
            }
        }
        "paste-buffer" | "pasteb" => {
            if let Err(e) = paste_latest(app) {
                return Some(Err(e));
            }
        }
        "set-buffer" | "setb" => {
            if let Some(text) = parts.get(1) {
                app.paste_buffers.insert(0, text.to_string());
                if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
            }
        }
        "delete-buffer" | "deleteb" => {
            if !app.paste_buffers.is_empty() { app.paste_buffers.remove(0); }
        }
        "list-buffers" | "lsb" => {
            let mut output = String::new();
            for (i, buf) in app.paste_buffers.iter().enumerate() {
                output.push_str(&format!("buffer{}: {} bytes: \"{}\"\n", i,
                    buf.len(), &buf.chars().take(50).collect::<String>()));
            }
            if output.is_empty() { output.push_str("(no buffers)\n"); }
            show_output_popup(app, "list-buffers", output);
        }
        "show-buffer" | "showb" => {
            if let Some(buf) = app.paste_buffers.first() {
                show_output_popup(app, "show-buffer", buf.clone());
            }
        }
        "choose-buffer" | "chooseb" => {
            // Enter buffer chooser mode
            app.mode = Mode::BufferChooser { selected: 0 };
        }
        "clear-history" | "clearhist" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "clear-history\n", &app.session_key);
            } else {
                let win = &mut app.windows[app.active_idx];
                if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                    if let Ok(mut parser) = p.term.lock() {
                        *parser = vt100::Parser::new(p.last_rows, p.last_cols, app.history_limit);
                    }
                }
            }
        }
        "capture-pane" | "capturep" => {
            if let Err(e) = capture_active_pane(app) {
                return Some(Err(e));
            }
        }
        "save-buffer" | "saveb" => {
            if let Some(file) = parts.get(1) {
                if let Err(e) = save_latest_buffer(app, file) {
                    return Some(Err(e));
                }
            }
        }
        "load-buffer" | "loadb" => {
            if let Some(path) = parts.get(1) {
                if let Ok(data) = std::fs::read_to_string(path) {
                    app.paste_buffers.insert(0, data);
                    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
                }
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
