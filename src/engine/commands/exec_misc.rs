use super::*;

pub(crate) fn handle_misc(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "rename-window" | "renamew" => {
            if let Some(name) = parts.get(1) {
                if app.active_idx < app.windows.len() {
                    let win = &mut app.windows[app.active_idx];
                    win.name = name.to_string();
                    win.manual_rename = true;
                }
                // Forward to server so external queries (display-message, list-windows) see the new name
                if let Some(port) = app.control_port {
                    let _ = send_control_to_port(port, &format!("rename-window {}\n", crate::util::quote_arg(name)), &app.session_key);
                }
            }
        }
        "toggle-sync" => {
            app.sync_input = !app.sync_input;
        }
        "pipe-pane" | "pipep" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "clock-mode" => {
            app.mode = Mode::ClockMode;
        }
        "show-messages" | "showmsgs" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                show_output_popup(app, "show-messages", "(no messages)\n".to_string());
            }
        }
        "set-environment" | "setenv" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let has_u = parts.iter().any(|p| *p == "-u");
                let non_flag: Vec<&str> = parts[1..].iter().filter(|p| !p.starts_with('-')).copied().collect();
                if has_u {
                    if let Some(key) = non_flag.first() {
                        app.environment.remove(*key);
                        std::env::remove_var(key);
                    }
                } else if non_flag.len() >= 2 {
                    app.environment.insert(non_flag[0].to_string(), non_flag[1].to_string());
                    std::env::set_var(non_flag[0], non_flag[1]);
                } else if non_flag.len() == 1 {
                    app.environment.insert(non_flag[0].to_string(), String::new());
                    std::env::set_var(non_flag[0], "");
                }
            }
        }
        "show-environment" | "showenv" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let mut output = String::new();
                for (key, value) in &app.environment {
                    output.push_str(&format!("{}={}\n", key, value));
                }
                if output.is_empty() { output.push_str("(no environment variables)\n"); }
                show_output_popup(app, "show-environment", output);
            }
        }
        "set-hook" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let has_unset = parts.iter().any(|p| *p == "-u" || *p == "-gu" || *p == "-ug");
                let has_append = parts.iter().any(|p| *p == "-a" || *p == "-ga" || *p == "-ag");
                let non_flag: Vec<&str> = parts[1..].iter().filter(|p| !p.starts_with('-')).copied().collect();
                if has_unset {
                    if let Some(name) = non_flag.first() {
                        app.hooks.remove(*name);
                    }
                } else if non_flag.len() >= 2 {
                    // Extract hook command from the raw cmd string to preserve quoting.
                    // non_flag[0] is the hook name; everything after it in the raw
                    // string is the command (may contain quoted paths with spaces).
                    let hook_name = non_flag[0];
                    let hook_cmd = if let Some(pos) = cmd.find(hook_name) {
                        let after_name = pos + hook_name.len();
                        cmd[after_name..].trim().to_string()
                    } else {
                        non_flag[1..].join(" ")
                    };
                    if has_append {
                        app.hooks.entry(hook_name.to_string()).or_default().push(hook_cmd);
                    } else {
                        app.hooks.insert(hook_name.to_string(), vec![hook_cmd]);
                    }
                }
            }
        }
        "if-shell" | "if" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                // Re-parse with quote-aware tokenizer so quoted args are handled
                let parsed = parse_command_line(cmd);
                let format_mode = parsed.iter().any(|p| p == "-F" || p == "-bF" || p == "-Fb");
                let positional: Vec<&str> = parsed[1..].iter()
                    .filter(|p| !p.starts_with('-'))
                    .map(|s| s.as_str())
                    .collect();
                if positional.len() >= 2 {
                    let condition = positional[0];
                    let true_cmd = positional[1];
                    let false_cmd = positional.get(2).copied();
                    let success = if format_mode {
                        let expanded = crate::format::expand_format(condition, app);
                        !expanded.is_empty() && expanded != "0"
                    } else if condition == "true" || condition == "1" {
                        true
                    } else if condition == "false" || condition == "0" {
                        false
                    } else {
                        {
                            let (shell_prog, mut shell_args) = resolve_run_shell();
                            shell_args.push(condition.to_string());
                            let mut cmd = std::process::Command::new(&shell_prog);
                            cmd.args(shell_args)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null());
                            #[cfg(windows)]
                            { use crate::platform::HideWindowCommandExt; cmd.hide_window(); }
                            cmd.status()
                            .map(|s| s.success()).unwrap_or(false)
                        }
                    };
                    if let Some(chosen) = if success { Some(true_cmd) } else { false_cmd } {
                        if let Err(e) = execute_command_string(app, chosen) {
                            return Some(Err(e));
                        }
                    }
                }
            }
        }
        "wait-for" | "wait" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
            // Local wait-for is a no-op (requires server coordination)
        }
        "find-window" | "findw" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let pattern = parts[1..].iter().find(|p| !p.starts_with('-')).unwrap_or(&"");
                let mut output = String::new();
                for (i, win) in app.windows.iter().enumerate() {
                    if win.name.contains(pattern) {
                        output.push_str(&format!("{}: {}\n", i + app.window_base_index, win.name));
                    }
                }
                if output.is_empty() { output.push_str(&format!("(no windows matching '{}')\n", pattern)); }
                show_output_popup(app, "find-window", output);
            }
        }
        "move-window" | "movew" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let target = parts[1..].iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok());
                if let Some(t) = target {
                    let t: usize = t;
                    if t < app.windows.len() && app.active_idx != t {
                        let win = app.windows.remove(app.active_idx);
                        let insert_idx = if t > app.active_idx { t - 1 } else { t };
                        app.windows.insert(insert_idx.min(app.windows.len()), win);
                        app.active_idx = insert_idx.min(app.windows.len() - 1);
                    }
                }
            }
        }
        "swap-window" | "swapw" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                if let Some(target) = parts[1..].iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse::<usize>().ok()) {
                    if target < app.windows.len() && app.active_idx != target {
                        app.windows.swap(app.active_idx, target);
                    }
                }
            }
        }
        "link-window" | "linkw" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                // Intra-session link-window: parse -s and -t flags
                let src_idx = parts.windows(2).find(|w| w[0] == "-s")
                    .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
                let dst_idx = parts.windows(2).find(|w| w[0] == "-t")
                    .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
                let src = src_idx.unwrap_or(app.active_idx);
                if src < app.windows.len() {
                    let src_id = app.windows[src].id;
                    let src_name = app.windows[src].name.clone();
                    let pty_system = portable_pty::native_pty_system();
                    if let Ok(()) = crate::pane::create_window(&*pty_system, app, None, None) {
                        let new_idx = app.windows.len() - 1;
                        app.windows[new_idx].linked_from = Some(src_id);
                        app.windows[new_idx].name = src_name;
                        if let Some(dst) = dst_idx {
                            if dst < new_idx {
                                let win = app.windows.remove(new_idx);
                                app.windows.insert(dst, win);
                            }
                        }
                        fire_hooks(app, "window-linked");
                    }
                } else {
                    app.status_message = Some(("link-window: source window not found".to_string(), Instant::now(), None));
                }
            }
        }
        "unlink-window" | "unlinkw" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else if app.windows.len() > 1 {
                let mut win = app.windows.remove(app.active_idx);
                kill_all_children(&mut win.root);
                if app.active_idx >= app.windows.len() {
                    app.active_idx = app.windows.len() - 1;
                }
                fire_hooks(app, "window-unlinked");
            }
        }
        "respawn-window" | "respawnw" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
            // respawn-window requires PTY system from server context
        }
        "run-shell" | "run" => {
            // Parse with quote-aware parser to handle nested quotes properly
            let args = parse_command_line(cmd);
            let mut cmd_parts: Vec<&str> = Vec::new();
            let mut background = false;
            for arg in &args[1..] {
                if arg == "-b" { background = true; }
                else { cmd_parts.push(arg); }
            }
            let shell_cmd = cmd_parts.join(" ");
            if shell_cmd.is_empty() {
                // No command given: show usage (tmux parity)
                app.status_message = Some((
                    "usage: run-shell [-b] shell-command".to_string(),
                    Instant::now(),
                    None,
                ));
            } else {
                // Expand ~ to home directory + XDG fallback for plugin paths
                let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd);
                // Set PSMUX_TARGET_SESSION so child scripts connect to the correct server
                let target_session = app.port_file_base();

                if background {
                    // -b flag: fire and forget, no output capture
                    let mut c = build_run_shell_command(&shell_cmd);
                    if !target_session.is_empty() {
                        c.env("PSMUX_TARGET_SESSION", &target_session);
                    }
                    let _ = c.spawn();
                } else {
                    // No -b: spawn async to avoid blocking the UI thread.
                    // Interactive commands (htop, vim, etc.) would freeze psmux
                    // if we used synchronous .output() on the main thread.
                    // Lazily create the channel pair on first use.
                    if app.run_shell_tx.is_none() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        app.run_shell_tx = Some(tx);
                        app.run_shell_rx = Some(rx);
                    }
                    let tx = app.run_shell_tx.as_ref().unwrap().clone();
                    let shell_cmd = shell_cmd.clone();
                    let shell_cmd_display = shell_cmd.clone();
                    let target_session = target_session.clone();
                    std::thread::spawn(move || {
                        let mut c = build_run_shell_command(&shell_cmd);
                        if !target_session.is_empty() {
                            c.env("PSMUX_TARGET_SESSION", &target_session);
                        }
                        // Detach stdin so interactive programs exit immediately
                        c.stdin(std::process::Stdio::null());
                        match c.output() {
                            Ok(output) => {
                                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                if !stderr.is_empty() {
                                    if !text.is_empty() && !text.ends_with('\n') {
                                        text.push('\n');
                                    }
                                    text.push_str(&stderr);
                                }
                                // Send result back; empty output is also sent so
                                // the status message "running..." can be cleared.
                                let _ = tx.send(("run-shell".to_string(), text));
                            }
                            Err(e) => {
                                let _ = tx.send(("run-shell".to_string(), format!("run-shell: {}", e)));
                            }
                        }
                    });
                    app.status_message = Some((
                        format!("running: {}", shell_cmd_display),
                        Instant::now(),
                        None,
                    ));
                }
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
