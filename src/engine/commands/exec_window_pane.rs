use super::*;

pub(crate) fn handle_window_pane(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "new-window" | "neww" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "new-window\n", &app.session_key);
            }
        }
        "split-window" | "splitw" => {
            if let Some(port) = app.control_port {
                // Forward the full command string to preserve -c, -d, -p etc. flags
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "kill-pane" => {
            let _ = kill_active_pane(app);
        }
        "kill-window" | "killw" => {
            if app.windows.len() > 1 {
                let mut win = app.windows.remove(app.active_idx);
                kill_all_children(&mut win.root);
                if app.active_idx >= app.windows.len() {
                    app.active_idx = app.windows.len() - 1;
                }
            }
        }
        "break-pane" | "breakp" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "break-pane\n", &app.session_key);
            } else {
                crate::window_ops::break_pane_to_window(app);
            }
        }
        "respawn-pane" | "respawnp" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let kill = parts.iter().any(|p| *p == "-k");
                if let Err(e) = crate::window_ops::respawn_active_pane(app, None, None, kill) {
                    return Some(Err(e));
                }
            }
        }
        "move-pane" | "movep" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let horizontal = parts[1..].iter().any(|a| *a == "-h");
                let mut src_win: Option<usize> = None;
                let mut src_pane: Option<usize> = None;
                let mut tgt_win: Option<usize> = None;
                let mut tgt_pane: Option<usize> = None;
                let mut pi = 1;
                while pi < parts.len() {
                    match parts[pi] {
                        "-s" => {
                            if let Some(sv) = parts.get(pi + 1) {
                                let pt = crate::cli::parse_target(sv);
                                src_win = pt.window;
                                src_pane = pt.pane;
                            }
                            pi += 2; continue;
                        }
                        "-t" => {
                            if let Some(tv) = parts.get(pi + 1) {
                                let pt = crate::cli::parse_target(tv);
                                tgt_win = pt.window;
                                tgt_pane = pt.pane;
                            }
                            pi += 2; continue;
                        }
                        _ => {}
                    }
                    pi += 1;
                }
                // Legacy: bare integer as target window
                if tgt_win.is_none() {
                    tgt_win = parts[1..].iter()
                        .filter(|a| !a.starts_with('-'))
                        .find(|a| a.parse::<usize>().is_ok())
                        .and_then(|s| s.parse::<usize>().ok());
                }
                join_pane_local(app, src_win, src_pane, tgt_win, tgt_pane, horizontal);
            }
        }
        "join-pane" | "joinp" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let horizontal = parts[1..].iter().any(|a| *a == "-h");
                let mut src_win: Option<usize> = None;
                let mut src_pane: Option<usize> = None;
                let mut tgt_win: Option<usize> = None;
                let mut tgt_pane: Option<usize> = None;
                let mut pi = 1;
                while pi < parts.len() {
                    match parts[pi] {
                        "-s" => {
                            if let Some(sv) = parts.get(pi + 1) {
                                let pt = crate::cli::parse_target(sv);
                                src_win = pt.window;
                                src_pane = pt.pane;
                            }
                            pi += 2; continue;
                        }
                        "-t" => {
                            if let Some(tv) = parts.get(pi + 1) {
                                let pt = crate::cli::parse_target(tv);
                                tgt_win = pt.window;
                                tgt_pane = pt.pane;
                            }
                            pi += 2; continue;
                        }
                        _ => {}
                    }
                    pi += 1;
                }
                // Legacy: bare integer as target window
                if tgt_win.is_none() {
                    tgt_win = parts[1..].iter()
                        .filter(|a| !a.starts_with('-'))
                        .find(|a| a.parse::<usize>().is_ok())
                        .and_then(|s| s.parse::<usize>().ok());
                }
                join_pane_local(app, src_win, src_pane, tgt_win, tgt_pane, horizontal);
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
