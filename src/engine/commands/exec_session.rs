use super::*;

pub(crate) fn handle_session(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "detach-client" | "detach" => {
            // handled by caller to set quit flag
        }
        "rename-session" => {
            if let Some(name) = parts.get(1) {
                app.session_name = name.to_string();
                // Forward to server so external queries see the new session name
                if let Some(port) = app.control_port {
                    let _ = send_control_to_port(port, &format!("rename-session {}\n", crate::util::quote_arg(name)), &app.session_key);
                }
            }
        }
        "kill-session" | "kill-ses" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "kill-session\n", &app.session_key);
            }
        }
        "kill-server" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "kill-server\n", &app.session_key);
            }
        }
        "has-session" | "has" => {
            // In embedded mode we ARE the session; always succeeds
        }
        "attach-session" | "attach" | "a" | "at" => {
            // Already attached in a running session; no-op
        }
        "start-server" | "start" => {
            // Already running
        }
        "lock-client" | "lockc" | "lock-server" | "lock" | "lock-session" | "locks" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "lock-server\n", &app.session_key);
            }
            app.status_message = Some(("lock: not available on Windows".to_string(), Instant::now(), None));
        }
        "refresh-client" | "refresh" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "refresh-client\n", &app.session_key);
            }
            // Trigger redraw in all modes
            app.status_message = Some(("client refreshed".to_string(), Instant::now(), None));
        }
        "suspend-client" | "suspendc" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "suspend-client\n", &app.session_key);
            }
            app.status_message = Some(("suspend: not available on Windows".to_string(), Instant::now(), None));
        }
        "choose-client" => {
            app.status_message = Some(("choose-client: single-client model (you are the only client)".to_string(), Instant::now(), None));
        }
        "new-session" | "new" => {
            // Issue #200: create a new session from inside a running session.
            // Parse flags: -s name, -d (detached), -n windowname, -c startdir, -e env
            let mut session_name: Option<String> = None;
            let mut detached = false;
            let mut window_name: Option<String> = None;
            let mut start_dir: Option<String> = None;
            let mut env_vars: Vec<(String, String)> = Vec::new();
            let mut initial_command: Option<String> = None;
            {
                let mut i = 1;
                while i < parts.len() {
                    match parts[i] {
                        "-s" => { i += 1; if i < parts.len() { session_name = Some(parts[i].trim_matches('"').to_string()); } }
                        "-n" => { i += 1; if i < parts.len() { window_name = Some(parts[i].trim_matches('"').to_string()); } }
                        "-c" => { i += 1; if i < parts.len() { start_dir = Some(parts[i].trim_matches('"').to_string()); } }
                        "-e" => {
                            i += 1;
                            match crate::util::parse_new_session_e_value_token(parts.get(i).copied()) {
                                Ok(p) => env_vars.push(p),
                                Err(e) => {
                                    app.status_message = Some((format!("psmux: {}", e), Instant::now(), None));
                                    return Some(Ok(()));
                                }
                            }
                        }
                        "-d" => { detached = true; }
                        "-A" | "-D" | "-E" | "-P" | "-X" => { /* compatibility flags, ignored */ }
                        "-F" | "-f" | "-t" | "-x" | "-y" => { i += 1; /* skip value */ }
                        other => {
                            // Positional arg: initial shell command (issue #229)
                            if !other.starts_with('-') {
                                initial_command = Some(parts[i..].iter().map(|s| s.trim_matches('"').to_string()).collect::<Vec<_>>().join(" "));
                                break;
                            }
                        }
                    }
                    i += 1;
                }
            }

            // Generate session name if not provided
            let ns_prefix = app.socket_name.as_deref();
            let name = session_name.unwrap_or_else(|| crate::session::next_session_name(ns_prefix));

            // Build port file base (with namespace prefix if applicable)
            let port_file_base = if let Some(ref sn) = app.socket_name {
                format!("{}__{}", sn, name)
            } else {
                name.clone()
            };

            // Check if session already exists
            let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
            let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);
            if std::path::Path::new(&port_path).exists() {
                if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                    if let Ok(port) = port_str.trim().parse::<u16>() {
                        let addr = format!("127.0.0.1:{}", port);
                        if std::net::TcpStream::connect_timeout(
                            &addr.parse().unwrap(),
                            std::time::Duration::from_millis(100),
                        ).is_ok() {
                            app.status_message = Some((format!("session '{}' already exists", name), Instant::now(), None));
                            return Some(Ok(()));
                        }
                    }
                }
                // Stale port file, remove it
                let _ = std::fs::remove_file(&port_path);
            }

            // Try to claim a warm server first (fast path)
            let warm_disabled = std::env::var("PSMUX_NO_WARM").map(|v| v == "1" || v == "true").unwrap_or(false)
                || crate::config::is_warm_disabled_by_config();
            let claimed_warm = if !warm_disabled && initial_command.is_none() && start_dir.is_none() && env_vars.is_empty() {
                let warm_base = if let Some(ref sn) = app.socket_name {
                    format!("{}____warm__", sn)
                } else {
                    "__warm__".to_string()
                };
                let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
                if std::path::Path::new(&warm_port_path).exists() {
                    if let Ok(warm_port_str) = std::fs::read_to_string(&warm_port_path) {
                        if let Ok(warm_port) = warm_port_str.trim().parse::<u16>() {
                            let warm_addr = format!("127.0.0.1:{}", warm_port);
                            if std::net::TcpStream::connect_timeout(
                                &warm_addr.parse().unwrap(),
                                std::time::Duration::from_millis(100),
                            ).is_ok() {
                                let warm_key = crate::session::read_session_key(&warm_base).unwrap_or_default();
                                if !warm_key.is_empty() {
                                    let claim_cmd = format!("claim-session {}\n", crate::util::quote_arg(&name));
                                    match crate::session::send_auth_cmd_response(
                                        &warm_addr, &warm_key,
                                        claim_cmd.as_bytes(),
                                    ) {
                                        Ok(resp) if resp.contains("OK") => {
                                            if let Some(ref wn) = window_name {
                                                let new_key = crate::session::read_session_key(&port_file_base).unwrap_or_default();
                                                let _ = crate::session::send_auth_cmd(
                                                    &warm_addr, &new_key,
                                                    format!("rename-window {}\n", crate::util::quote_arg(wn)).as_bytes(),
                                                );
                                            }
                                            // Apply -e environment variables to the claimed warm session
                                            if !env_vars.is_empty() {
                                                let new_key = crate::session::read_session_key(&port_file_base).unwrap_or_default();
                                                for (k, v) in &env_vars {
                                                    let _ = crate::session::send_auth_cmd(
                                                        &warm_addr, &new_key,
                                                        format!("set-environment {} {}\n", crate::util::quote_arg(k), crate::util::quote_arg(v)).as_bytes(),
                                                    );
                                                }
                                            }
                                            true
                                        }
                                        _ => false,
                                    }
                                } else { false }
                            } else { false }
                        } else { false }
                    } else { false }
                } else { false }
            } else { false };

            if !claimed_warm {
                // Cold path: spawn a background server process
                let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
                let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), name.clone()];
                if let Some(ref sn) = app.socket_name {
                    server_args.push("-L".into());
                    server_args.push(sn.clone());
                }
                if let Some(ref dir) = start_dir {
                    server_args.push("-d".into());
                    server_args.push(dir.clone());
                }
                if let Some(ref wn) = window_name {
                    server_args.push("-n".into());
                    server_args.push(wn.clone());
                }
                // Pass initial command to server (issue #229)
                if let Some(ref cmd) = initial_command {
                    server_args.push("-c".into());
                    server_args.push(cmd.clone());
                }
                // Pass current terminal dimensions
                let area = app.last_window_area;
                if area.width > 1 && area.height > 1 {
                    server_args.push("-x".into());
                    server_args.push(area.width.to_string());
                    server_args.push("-y".into());
                    server_args.push(area.height.to_string());
                }
                // Pass -e environment variables to server
                for (k, v) in &env_vars {
                    server_args.push("-e".into());
                    server_args.push(format!("{}={}", k, v));
                }
                #[cfg(windows)]
                { let _ = crate::platform::spawn_server_hidden(&exe, &server_args); }
                #[cfg(not(windows))]
                {
                    let mut cmd_proc = std::process::Command::new(&exe);
                    for a in &server_args { cmd_proc.arg(a); }
                    cmd_proc.stdin(std::process::Stdio::null());
                    cmd_proc.stdout(std::process::Stdio::null());
                    cmd_proc.stderr(std::process::Stdio::null());
                    let _ = cmd_proc.spawn();
                }
            }

            // Wait for port file to appear (up to 5 seconds)
            for _ in 0..500 {
                if std::path::Path::new(&port_path).exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            if std::path::Path::new(&port_path).exists() {
                if !detached {
                    // Switch to the new session
                    if let Some(port) = app.control_port {
                        let switch_cmd = format!("switch-client -t {}\n", crate::util::quote_arg(&name));
                        let _ = send_control_to_port(port, &switch_cmd, &app.session_key);
                    }
                }
                app.status_message = Some((format!("created session '{}'", name), Instant::now(), None));
            } else {
                app.status_message = Some((format!("failed to create session '{}'", name), Instant::now(), None));
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
