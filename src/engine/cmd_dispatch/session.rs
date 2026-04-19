use std::io::{self, Write, BufRead as _};
use std::time::Duration;
use std::env;

use crate::session::{read_session_key, send_control,
    kill_remaining_server_processes};

/// Handle `server` internal command. Returns directly (does not fall through).
pub(crate) fn handle_server_cmd(args: &[String]) -> io::Result<()> {
    let name = args.iter().position(|a| a == "-s").and_then(|i| args.get(i+1)).map(|s| s.clone()).unwrap_or_else(|| "default".to_string());
    let server_socket_name = args.iter().position(|a| a == "-L").and_then(|i| args.get(i+1)).map(|s| s.clone());
    let initial_cmd = args.iter().position(|a| a == "-c").and_then(|i| args.get(i+1)).map(|s| s.clone());
    let srv_start_dir = args.iter().position(|a| a == "-d").and_then(|i| args.get(i+1)).map(|s| s.clone());
    let srv_window_name = args.iter().position(|a| a == "-n").and_then(|i| args.get(i+1)).map(|s| s.clone());
    let srv_init_width = args.iter().position(|a| a == "-x").and_then(|i| args.get(i+1)).and_then(|s| s.parse::<u16>().ok());
    let srv_init_height = args.iter().position(|a| a == "-y").and_then(|i| args.get(i+1)).and_then(|s| s.parse::<u16>().ok());
    let srv_init_size = match (srv_init_width, srv_init_height) {
        (Some(w), Some(h)) => Some((w, h)),
        (Some(w), None) => Some((w, 24)),
        (None, Some(h)) => Some((80, h)),
        _ => None,
    };
    let srv_group_target = args.iter().position(|a| a == "-g").and_then(|i| args.get(i+1)).map(|s| s.clone());
    let srv_env_vars = crate::util::collect_server_session_env_args(args).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, e)
    })?;
    let raw_cmd: Option<Vec<String>> = args.iter().position(|a| a == "--").map(|pos| {
        args.iter().skip(pos + 1).cloned().collect()
    }).filter(|v: &Vec<String>| !v.is_empty());
    crate::server::run_server(name, server_socket_name, initial_cmd, raw_cmd, srv_start_dir, srv_window_name, srv_init_size, srv_group_target, srv_env_vars)
}

pub(crate) fn handle_kill_server(l_socket_name: &Option<String>) -> io::Result<()> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let psmux_dir = format!("{}\\.psmux", home);
    // Compute namespace prefix for -L filtering (matches list-sessions behavior)
    let ns_prefix = l_socket_name.as_ref().map(|l| format!("{l}__"));
    let mut targets: Vec<(std::path::PathBuf, u16, String)> = Vec::new();
    let mut stale_ports: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "port").unwrap_or(false) {
                if let Some(session_name) = path.file_stem().and_then(|s| s.to_str()) {
                    // Apply -L namespace filtering:
                    // With -L: only kill sessions under that namespace
                    // Without -L: kill ALL sessions (tmux behavior)
                    if let Some(ref pfx) = ns_prefix {
                        if !session_name.starts_with(pfx.as_str()) { continue; }
                    }
                    if let Ok(port_str) = std::fs::read_to_string(&path) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let sess_key = read_session_key(session_name).unwrap_or_default();
                            targets.push((path.clone(), port, sess_key));
                        }
                    } else {
                        stale_ports.push(path.clone());
                    }
                }
            }
        }
    }
    // Send kill-server to all sessions in parallel via threads
    let handles: Vec<std::thread::JoinHandle<()>> = targets.into_iter().map(|(path, port, sess_key)| {
        std::thread::spawn(move || {
            let addr = format!("127.0.0.1:{}", port);
            if let Ok(mut stream) = std::net::TcpStream::connect_timeout(
                &addr.parse().unwrap(),
                Duration::from_millis(500),
            ) {
                let _ = stream.set_nodelay(true);
                let _ = write!(stream, "AUTH {}\n", sess_key);
                let _ = stream.flush();
                let _ = std::io::Write::write_all(&mut stream, b"kill-server\n");
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                // Wait for server to exit (EOF = done)
                let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
                let mut buf = [0u8; 64];
                loop {
                    match std::io::Read::read(&mut stream, &mut buf) {
                        Ok(0) => break,
                        Err(_) => break,
                        Ok(_) => continue,
                    }
                }
            }
            // Remove port/key files regardless
            let _ = std::fs::remove_file(&path);
            let key_path = path.with_extension("key");
            let _ = std::fs::remove_file(&key_path);
        })
    }).collect();
    // Wait for all threads to complete
    for h in handles { let _ = h.join(); }
    // Clean up stale port/key files
    for path in &stale_ports {
        let _ = std::fs::remove_file(path);
        let key_path = path.with_extension("key");
        let _ = std::fs::remove_file(&key_path);
    }
    // Brief wait then verify no processes remain; if any do, force-kill them.
    // Only do the nuclear fallback when not using -L namespace filtering.
    std::thread::sleep(Duration::from_millis(50));
    if ns_prefix.is_none() {
        kill_remaining_server_processes();
    }
    Ok(())
}

pub(crate) fn handle_list_sessions(cmd_args: &[&String], l_socket_name: &Option<String>) -> io::Result<()> {
    // Parse -F (format) and -f (filter) flags
    let mut format_str: Option<String> = None;
    let mut filter_str: Option<String> = None;
    {
        let mut i = 1;
        while i < cmd_args.len() {
            match cmd_args[i].as_str() {
                "-F" => {
                    if let Some(f) = cmd_args.get(i + 1) {
                        format_str = Some(f.to_string());
                        i += 1;
                    }
                }
                "-f" => {
                    if let Some(f) = cmd_args.get(i + 1) {
                        filter_str = Some(f.to_string());
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let dir = format!("{}\\.psmux", home);
    // Compute namespace prefix for -L filtering
    let ns_prefix = l_socket_name.as_ref().map(|l| format!("{l}__"));
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some((base, ext)) = name.rsplit_once('.') {
                    if ext == "port" {
                        // Skip warm (standby) sessions — internal-only
                        if crate::session::is_warm_session(base) { continue; }
                        // Filter by -L namespace: when -L is given, only show
                        // sessions with that prefix; when no -L, only show
                        // sessions without any namespace prefix
                        if let Some(ref pfx) = ns_prefix {
                            if !base.starts_with(pfx.as_str()) { continue; }
                        } else {
                            if base.contains("__") { continue; }
                        }
                        if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                            if let Ok(_p) = port_str.trim().parse::<u16>() {
                                let addr = format!("127.0.0.1:{}", port_str.trim());
                                if let Ok(mut s) = std::net::TcpStream::connect_timeout(
                                    &addr.parse().unwrap(),
                                    Duration::from_millis(50)
                                ) {
                                    let _ = s.set_read_timeout(Some(Duration::from_millis(50)));
                                    // Read session key and authenticate
                                    let key_path = format!("{}\\.psmux\\{}.key", home, base);
                                    if let Ok(key) = std::fs::read_to_string(&key_path) {
                                        let _ = std::io::Write::write_all(&mut s, format!("AUTH {}\n", key.trim()).as_bytes());
                                    }
                                    // Use -F format if provided, otherwise session-info
                                    let query = if let Some(ref fmt) = format_str {
                                        format!("list-sessions -F \"{}\"\n", fmt.replace('"', "\\\""))
                                    } else {
                                        "session-info\n".to_string()
                                    };
                                    let _ = std::io::Write::write_all(&mut s, query.as_bytes());
                                    let mut br = std::io::BufReader::new(s);
                                    let mut line = String::new();
                                    // Skip "OK" response from AUTH
                                    let _ = br.read_line(&mut line);
                                    if line.trim() == "OK" {
                                        line.clear();
                                        let _ = br.read_line(&mut line);
                                    }
                                    if line.trim() == "ERROR: Authentication required" {
                                        // Auth failed, skip this session
                                        continue;
                                    }
                                    // When -F format is provided, the server already
                                    // expanded it; use the result even if empty (tmux
                                    // prints an empty line for unknown format vars).
                                    // Only fall back to display_name when no -F was given.
                                    if format_str.is_some() || !line.trim().is_empty() {
                                        let output = line.trim_end().to_string();
                                        // Apply -f filter if provided.
                                        // tmux -f accepts format expressions; support
                                        // the common #{==:#{session_name},NAME} pattern
                                        // as well as a plain substring fallback.
                                        if let Some(ref flt) = filter_str {
                                            let passes = if let Some(target) = flt
                                                .strip_prefix("#{==:#{session_name},")
                                                .and_then(|s| s.strip_suffix('}'))
                                            {
                                                // Compare port-file display name against literal
                                                let display_name = if let Some(ref pfx) = ns_prefix {
                                                    base.strip_prefix(pfx.as_str()).unwrap_or(base)
                                                } else {
                                                    base
                                                };
                                                display_name == target
                                            } else {
                                                // Fallback: plain substring match
                                                output.contains(flt.as_str())
                                            };
                                            if !passes { continue; }
                                        }
                                        println!("{}", output);
                                    } else {
                                        // Strip namespace prefix for display (e.g. "foo__dev" -> "dev")
                                        let display_name = if let Some(ref pfx) = ns_prefix {
                                            base.strip_prefix(pfx.as_str()).unwrap_or(base)
                                        } else {
                                            base
                                        };
                                        if let Some(ref flt) = filter_str {
                                            let passes = if let Some(target) = flt
                                                .strip_prefix("#{==:#{session_name},")
                                                .and_then(|s| s.strip_suffix('}'))
                                            {
                                                display_name == target
                                            } else {
                                                display_name.contains(flt.as_str())
                                            };
                                            if !passes { continue; }
                                        }
                                        println!("{}", display_name); 
                                    }
                                } else {
                                    // stale port file - remove it along with matching key
                                    let _ = std::fs::remove_file(e.path());
                                    let key_path = e.path().with_extension("key");
                                    let _ = std::fs::remove_file(&key_path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_kill_session(cmd_args: &[&String], l_socket_name: &Option<String>) -> io::Result<()> {
    let mut target: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    // Apply -L namespace prefix for port file lookup
                    let namespaced = if let Some(ref l) = l_socket_name {
                        format!("{}__{}", l, t)
                    } else {
                        t.to_string()
                    };
                    target = Some(namespaced);
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let session_name = target.clone().unwrap_or_else(|| {
        env::var("PSMUX_TARGET_SESSION").unwrap_or_else(|_| {
            // Apply -L namespace prefix to default
            if let Some(ref l) = l_socket_name {
                format!("{}__{}", l, "default")
            } else {
                "default".to_string()
            }
        })
    });
    if let Some(ref t) = target {
        env::set_var("PSMUX_TARGET_SESSION", t);
    }
    // Try to send kill command to server
    if send_control("kill-session\n".to_string()).is_err() {
        // Server not responding - clean up stale port file
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        let port_path = format!("{}\\.psmux\\{}.port", home, session_name);
        let _ = std::fs::remove_file(&port_path);
    }
    Ok(())
}

pub(crate) fn handle_has_session(cmd_args: &[&String], l_socket_name: &Option<String>) -> io::Result<()> {
    // Get target from env (set from -t flag) or from remaining args
    let target = env::var("PSMUX_TARGET_SESSION").unwrap_or_else(|_| {
        // Try to get session name from cmd_args
        let mut t = "default".to_string();
        let mut i = 1;
        while i < cmd_args.len() {
            if cmd_args[i].as_str() == "-t" {
                if let Some(v) = cmd_args.get(i + 1) { t = v.to_string(); }
                i += 1;
            } else if !cmd_args[i].starts_with('-') {
                t = cmd_args[i].to_string();
                break;
            }
            i += 1;
        }
        // Apply -L namespace prefix for port file lookup
        if let Some(ref l) = l_socket_name {
            format!("{}__{}", l, t)
        } else {
            t
        }
    });
    // Warm (standby) sessions are internal-only — treat as non-existent
    if crate::session::is_warm_session(&target) {
        std::process::exit(1);
    }
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let path = format!("{}\\.psmux\\{}.port", home, target);
    if let Ok(port_str) = std::fs::read_to_string(&path) {
        if let Ok(port) = port_str.trim().parse::<u16>() {
            let addr = format!("127.0.0.1:{}", port);
            // Actually authenticate and query the server to ensure it's healthy
            let session_key = read_session_key(&target).unwrap_or_default();
            if let Ok(mut s) = std::net::TcpStream::connect_timeout(
                &addr.parse().unwrap(),
                Duration::from_millis(500)
            ) {
                let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = write!(s, "AUTH {}\n", session_key);
                let _ = write!(s, "session-info\n");
                let _ = s.flush();
                let mut buf = [0u8; 256];
                if let Ok(n) = std::io::Read::read(&mut s, &mut buf) {
                    if n > 0 {
                        let resp = String::from_utf8_lossy(&buf[..n]);
                        if resp.contains("OK") {
                            std::process::exit(0);
                        }
                    }
                }
                // Fallback: connection succeeded so session likely exists
                std::process::exit(0);
            } else {
                // Stale port file - clean it up
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    std::process::exit(1);
}

pub(crate) fn handle_rename_session(cmd_args: &[&String]) -> io::Result<()> {
    let mut new_name: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        if !cmd_args[i].starts_with('-') {
            new_name = Some(cmd_args[i].to_string());
            break;
        }
        i += 1;
    }
    if let Some(name) = new_name {
        send_control(format!("rename-session {}\n", crate::util::quote_arg(&name)))?;
    }
    Ok(())
}
