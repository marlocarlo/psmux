use super::*;
use super::run_remote_state::RunRemoteState;

/// Populate the choose-tree overlay entries by querying all live sessions.
pub(crate) fn populate_choose_tree(
    state: &mut RunRemoteState,
    home: &str,
    current_session: &str,
) {
    state.tree_chooser = true;
    state.tree_entries.clear();
    state.tree_selected = 0;
    state.tree_scroll = 0;
    // Query ALL sessions (like tmux choose-tree)
    let dir = format!("{}\\.psmux", home);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut sessions: Vec<(String, Vec<(usize, String, Vec<(usize, String)>)>)> = Vec::new();
        for e in entries.flatten() {
            if let Some(fname) = e.file_name().to_str().map(|s| s.to_string()) {
                if let Some((base, ext)) = fname.rsplit_once('.') {
                    if ext == "port" {
                        if crate::session::is_warm_session(base) { continue; }
                        if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                            if let Ok(p) = port_str.trim().parse::<u16>() {
                                let sess_addr = format!("127.0.0.1:{}", p);
                                let sess_key = read_session_key(base).unwrap_or_default();
                                if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                                    &sess_addr.parse().unwrap(), Duration::from_millis(50)
                                ) {
                                    let _ = ss.set_read_timeout(Some(Duration::from_millis(100)));
                                    let _ = write!(ss, "AUTH {}\n", sess_key);
                                    let _ = ss.write_all(b"list-tree\n");
                                    let _ = ss.flush();
                                    let mut br = BufReader::new(ss);
                                    let mut al = String::new();
                                    let _ = br.read_line(&mut al); // AUTH OK
                                    let mut tree_line = String::new();
                                    if br.read_line(&mut tree_line).is_ok() {
                                        if let Ok(wins) = serde_json::from_str::<Vec<WinTree>>(&tree_line.trim()) {
                                            let mut win_data = Vec::new();
                                            for w in &wins {
                                                let panes: Vec<(usize, String)> = w.panes.iter().map(|p| (p.id, p.title.clone())).collect();
                                                win_data.push((w.id, w.name.clone(), panes));
                                            }
                                            sessions.push((base.to_string(), win_data));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        sessions.sort_by(|a, b| {
            if a.0 == current_session { std::cmp::Ordering::Less }
            else if b.0 == current_session { std::cmp::Ordering::Greater }
            else { a.0.cmp(&b.0) }
        });
        for (sess_name, wins) in &sessions {
            let is_current = sess_name == current_session;
            let attached = if is_current { " (attached)" } else { "" };
            let nw = wins.len();
            state.tree_entries.push((true, usize::MAX, 0,
                format!("{}: {} windows{}", sess_name, nw, attached),
                sess_name.clone()));
            if is_current {
                for (wi, (wid, wname, panes)) in wins.iter().enumerate() {
                    let _flag = if panes.len() > 0 { "" } else { "" };
                    state.tree_entries.push((true, *wid, 0,
                        format!("  {}: {}{} ({} panes)", wi, wname, _flag, panes.len()),
                        sess_name.clone()));
                    for (pid, ptitle) in panes {
                        state.tree_entries.push((false, *wid, *pid,
                            format!("    {}", ptitle),
                            sess_name.clone()));
                    }
                }
            } else {
                for (wi, (wid, wname, panes)) in wins.iter().enumerate() {
                    state.tree_entries.push((true, *wid, 0,
                        format!("  {}: {} ({} panes)", wi, wname, panes.len()),
                        sess_name.clone()));
                }
            }
        }
    }
    if state.tree_entries.is_empty() {
        for wi in &state.last_tree {
            state.tree_entries.push((true, wi.id, 0, wi.name.clone(), current_session.to_string()));
            for pi in &wi.panes {
                state.tree_entries.push((false, wi.id, pi.id, pi.title.clone(), current_session.to_string()));
            }
        }
    }
}

/// Populate the choose-session overlay entries.
pub(crate) fn populate_choose_session(
    state: &mut RunRemoteState,
    _home: &str,
    current_session: &str,
) {
    state.session_chooser = true;
    state.session_entries.clear();
    state.session_selected = 0;
    state.session_scroll = 0;
    let dir = format!("{}\\.psmux", _home);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(fname) = e.file_name().to_str() {
                if let Some((base, ext)) = fname.rsplit_once('.') {
                    if ext == "port" {
                        if crate::session::is_warm_session(base) { continue; }
                        if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                            if let Ok(p) = port_str.trim().parse::<u16>() {
                                let sess_addr = format!("127.0.0.1:{}", p);
                                let sess_key = read_session_key(base).unwrap_or_default();
                                let info = if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                                    &sess_addr.parse().unwrap(), Duration::from_millis(25)
                                ) {
                                    let _ = ss.set_read_timeout(Some(Duration::from_millis(25)));
                                    let _ = write!(ss, "AUTH {}\n", sess_key);
                                    let _ = ss.write_all(b"session-info\n");
                                    let mut br = BufReader::new(ss);
                                    let mut al = String::new();
                                    let _ = br.read_line(&mut al);
                                    let mut line = String::new();
                                    if br.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                                        line.trim().to_string()
                                    } else {
                                        format!("{}: (no info)", base)
                                    }
                                } else {
                                    format!("{}: (not responding)", base)
                                };
                                state.session_entries.push((base.to_string(), info));
                            }
                        }
                    }
                }
            }
        }
    }
    if state.session_entries.is_empty() {
        state.session_entries.push((current_session.to_string(), format!("{}: (current)", current_session)));
    }
    for (i, (sname, _)) in state.session_entries.iter().enumerate() {
        if sname == current_session { state.session_selected = i; break; }
    }
}

/// Populate the choose-buffer overlay entries.
pub(crate) fn populate_choose_buffer(
    state: &mut RunRemoteState,
    home: &str,
    current_session: &str,
) {
    state.buffer_chooser = true;
    state.buffer_entries.clear();
    state.buffer_selected = 0;
    state.buffer_scroll = 0;
    // Fetch buffer list from server via TCP
    let port_file = format!("{}\\.psmux\\{}.port", home, current_session);
    if let Ok(port_str) = std::fs::read_to_string(&port_file) {
        if let Ok(p) = port_str.trim().parse::<u16>() {
            let sess_key = read_session_key(current_session).unwrap_or_default();
            let addr = format!("127.0.0.1:{}", p);
            if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                &addr.parse().unwrap(), Duration::from_millis(100)
            ) {
                let _ = ss.set_read_timeout(Some(Duration::from_millis(200)));
                let _ = write!(ss, "AUTH {}\n", sess_key);
                let _ = ss.write_all(b"choose-buffer\n");
                let _ = ss.flush();
                let mut br = BufReader::new(ss);
                let mut al = String::new();
                let _ = br.read_line(&mut al); // AUTH OK
                let mut buf_line = String::new();
                if br.read_line(&mut buf_line).is_ok() {
                    for line in buf_line.trim().split('\n') {
                        let line = line.trim();
                        if line.is_empty() { continue; }
                        if let Some(rest) = line.strip_prefix("buffer") {
                            if let Some(colon_pos) = rest.find(':') {
                                if let Ok(idx) = rest[..colon_pos].parse::<usize>() {
                                    let after_colon = &rest[colon_pos+1..].trim_start();
                                    let byte_len = after_colon.split_whitespace().next()
                                        .and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                                    let preview = if let Some(bp) = after_colon.find('"') {
                                        let p = &after_colon[bp+1..];
                                        p.trim_end_matches('"').to_string()
                                    } else {
                                        after_colon.to_string()
                                    };
                                    state.buffer_entries.push((idx, byte_len, preview));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if state.buffer_entries.is_empty() {
        // No buffers, don't show chooser
        state.buffer_chooser = false;
    }
}

/// Navigate sessions: cycle to next or previous session.
/// Returns `true` if a session switch was initiated (caller should set quit).
pub(crate) fn navigate_session(
    state: &mut RunRemoteState,
    cmd_batch: &mut Vec<String>,
    home: &str,
    current_session: &str,
    dir_next: bool,
) -> bool {
    let dir = format!("{}\\.psmux", home);
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(fname) = e.file_name().to_str() {
                if let Some((base, ext)) = fname.rsplit_once('.') {
                    if ext == "port" {
                        if crate::session::is_warm_session(base) { continue; }
                        if let Ok(ps) = std::fs::read_to_string(e.path()) {
                            if let Ok(p) = ps.trim().parse::<u16>() {
                                let a = format!("127.0.0.1:{}", p);
                                if std::net::TcpStream::connect_timeout(
                                    &a.parse().unwrap(), Duration::from_millis(25)
                                ).is_ok() {
                                    names.push(base.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    names.sort();
    if names.len() > 1 {
        if let Some(cur_pos) = names.iter().position(|n| *n == current_session) {
            let next_pos = if dir_next {
                (cur_pos + 1) % names.len()
            } else {
                (cur_pos + names.len() - 1) % names.len()
            };
            let next_name = names[next_pos].clone();
            cmd_batch.push("client-detach\n".into());
            env::set_var("PSMUX_SWITCH_TO", &next_name);
            let _ = state; // state not mutated here, but keep signature uniform
            return true;
        }
    }
    false
}
