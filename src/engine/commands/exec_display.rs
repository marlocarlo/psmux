use super::*;

pub(crate) fn handle_display(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "display-panes" | "displayp" => {
            let win = &app.windows[app.active_idx];
            let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
            compute_rects(&win.root, app.last_window_area, &mut rects);
            app.display_map.clear();
            for (i, (path, _)) in rects.into_iter().enumerate() {
                if i >= 10 { break; }
                let digit = (i + app.pane_base_index) % 10;
                app.display_map.push((digit, path));
            }
            app.mode = Mode::PaneChooser { opened_at: Instant::now() };
        }
        "confirm-before" | "confirm" => {
            let rest = parts[1..].join(" ");
            app.mode = Mode::ConfirmMode {
                prompt: format!("Run '{}'?", rest),
                command: rest,
                input: String::new(),
            };
        }
        "display-menu" | "menu" => {
            let rest = parts[1..].join(" ");
            let menu = parse_menu_definition(&rest, None, None);
            if !menu.items.is_empty() {
                app.mode = Mode::MenuMode { menu };
            }
        }
        "display-popup" | "popup" => {
            // Parse -w width, -h height, -E close-on-exit, -d start-dir flags
            let mut width_spec = "80".to_string();
            let mut height_spec = "24".to_string();
            let mut start_dir: Option<String> = None;
            let close_on_exit = parts.iter().any(|p| *p == "-E");
            let mut skip_indices = std::collections::HashSet::new();
            skip_indices.insert(0); // skip the command name itself
            let mut i = 1;
            while i < parts.len() {
                match parts[i] {
                    "-w" => { if let Some(v) = parts.get(i + 1) { width_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i + 1); i += 1; } }
                    "-h" => { if let Some(v) = parts.get(i + 1) { height_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i + 1); i += 1; } }
                    "-d" | "-c" => { if let Some(v) = parts.get(i + 1) { start_dir = Some(v.to_string()); skip_indices.insert(i); skip_indices.insert(i + 1); i += 1; } }
                    "-E" | "-K" => { skip_indices.insert(i); }
                    _ => {}
                }
                i += 1;
            }
            // Resolve percentage dimensions against terminal size (#154)
            let (term_w, term_h) = crossterm::terminal::size().unwrap_or((120, 40));
            let width = parse_popup_dim_local(&width_spec, term_w, 80);
            let height = parse_popup_dim_local(&height_spec, term_h, 24);
            // Collect remaining args as the command
            let rest: String = parts.iter().enumerate()
                .filter(|(idx, _)| !skip_indices.contains(idx))
                .map(|(_, a)| *a)
                .collect::<Vec<&str>>()
                .join(" ");
            
            // Spawn popup as a real Pane via the popup module
            let pane_result = if !rest.is_empty() {
                crate::popup::create_popup_pane(
                    &rest,
                    start_dir.as_deref(),
                    height.saturating_sub(2),
                    width.saturating_sub(2),
                    app.next_pane_id,
                    "1", // session name not available in local mode
                    &app.environment,
                )
            } else { None };
            
            app.mode = Mode::PopupMode {
                command: rest,
                output: String::new(),
                process: None,
                width,
                height,
                close_on_exit,
                popup_pane: pane_result,
                scroll_offset: 0,
            };
        }
        "choose-tree" | "choose-window" | "choose-session" => {
            let tree = build_choose_tree(app);
            let selected = tree.iter().position(|e| e.is_current_session && e.is_active_window && !e.is_session_header).unwrap_or(0);
            app.mode = Mode::WindowChooser { selected, tree };
        }
        "command-prompt" => {
            // Support -I initial_text, -p prompt (ignored), -1 (ignored)
            let initial = parts.windows(2).find(|w| w[0] == "-I").map(|w| w[1].to_string()).unwrap_or_default();
            app.command_vi_normal = false;
            app.mode = Mode::CommandPrompt { input: initial.clone(), cursor: initial.len() };
        }
        "display-message" | "display" => {
            if let Some(port) = app.control_port {
                // Forward to server; use default format when no args given
                let effective_cmd = if parts.len() <= 1 {
                    format!("display-message \"{}\"", DISPLAY_MESSAGE_DEFAULT_FMT)
                } else {
                    cmd.to_string()
                };
                let _ = send_control_to_port(port, &format!("{}\n", effective_cmd), &app.session_key);
            } else {
                // Local: expand format string and show as status message
                // Parse flags from parts (same as CLI/server):
                //   -d <ms>  per-message display duration
                //   -I <val> consumed (not implemented locally)
                //   -t <val> target (ignored locally)
                //   -p       print to stdout (ignored locally, we show on status bar)
                let mut msg_parts: Vec<&str> = Vec::new();
                let mut duration_ms: Option<u64> = None;
                let mut idx = 1;
                while idx < parts.len() {
                    match parts[idx] {
                        "-d" => {
                            if idx + 1 < parts.len() {
                                duration_ms = parts[idx + 1].parse::<u64>().ok();
                            }
                            idx += 1;
                        }
                        "-I" | "-t" => { idx += 1; }
                        "-p" => {}
                        other => { msg_parts.push(other); }
                    }
                    idx += 1;
                }
                let raw = msg_parts.join(" ");
                let msg = if raw.is_empty() {
                    DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
                } else {
                    raw.trim_matches('"').trim_matches('\'').to_string()
                };
                let expanded = crate::format::expand_format(&msg, app);
                app.status_message = Some((expanded, Instant::now(), duration_ms));
            }
        }
        "customize-mode" => {
            // tmux 3.2+ customize-mode: interactive options editor
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "customize-mode\n", &app.session_key);
            } else {
                // In-process fallback: build option list directly
                let options = crate::server::option_catalog::build_option_list(app);
                app.mode = Mode::CustomizeMode {
                    options,
                    selected: 0,
                    scroll_offset: 0,
                    editing: false,
                    edit_buffer: String::new(),
                    edit_cursor: 0,
                    filter: String::new(),
                };
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
