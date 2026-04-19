use std::io;
use std::time::Duration;
use std::env;

use crate::session::{send_control_with_response,
    resolve_default_session_name};

/// Resolve the session name for `attach-session`. Returns the port file base name.
pub(crate) fn resolve_attach_name(args: &[String], l_socket_name: &Option<String>) -> String {
    args
        .iter()
        .position(|a| a == "-t")
        .and_then(|i| args.get(i + 1))
        .map(|s| {
            // Apply -L namespace prefix when -t is specified
            if let Some(ref l) = l_socket_name {
                format!("{}__{}", l, s)
            } else {
                s.clone()
            }
        })
        .or_else(resolve_default_session_name)
        .or_else(|| crate::session::resolve_last_session_name_ns(l_socket_name.as_deref()))
        .unwrap_or_else(|| {
            if let Some(ref l) = l_socket_name {
                format!("{}__0", l)
            } else {
                "0".to_string()
            }
        })
}

/// Handle `new-session` / `new` command.
/// Returns `true` if the caller should continue to TUI attach (env vars already set).
/// Returns `false` if the command is done (detached mode or early exit).
pub(crate) fn handle_new_session(cmd_args: &[&String], l_socket_name: &Option<String>, f_config_file: &Option<String>) -> io::Result<bool> {
    // Prevent nesting: block new-session inside an existing psmux session
    if env::var("PSMUX_ALLOW_NESTING").ok().as_deref() != Some("1") {
        if env::var("PSMUX_ACTIVE").ok().as_deref() == Some("1")
            || env::var("PSMUX_SESSION").ok().filter(|v| !v.is_empty()).is_some()
        {
            eprintln!("psmux: sessions should be nested with care, unset PSMUX_SESSION to force");
            return Ok(false);
        }
    }
    // Strict getopt-style parsing for new-session flags.
    // tmux template: "Ac:dDe:EF:f:n:Ps:t:x:Xy:"
    let mut session_name: Option<String> = None;
    let mut detached = false;
    let mut print_info = false;
    let mut format_str: Option<String> = None;
    let mut window_name: Option<String> = None;
    let mut start_dir: Option<String> = None;
    let mut attach_if_exists = false;
    let mut init_width: Option<u16> = None;
    let mut init_height: Option<u16> = None;
    let mut group_target: Option<String> = None;
    let mut env_vars: Vec<(String, String)> = Vec::new();
    let mut positional_args: Vec<String> = Vec::new();
    let mut raw_cmd_after_dd: Option<Vec<String>> = None;

    {
        let mut i = 1; // skip command name (cmd_args[0])
        while i < cmd_args.len() {
            let a = cmd_args[i].as_str();
            if a == "--" {
                // Everything after -- is raw command
                raw_cmd_after_dd = Some(cmd_args[i+1..].iter().map(|s| s.to_string()).collect());
                break;
            }
            if !a.starts_with('-') {
                // Positional argument — collect it and everything after
                positional_args.extend(cmd_args[i..].iter().map(|s| s.to_string()));
                break;
            }

            let chars: Vec<char> = if a.len() > 2 && !a.starts_with("--") {
                a[1..].chars().collect()
            } else if a.len() == 2 {
                vec![a.chars().nth(1).unwrap()]
            } else {
                // Unknown long flag, skip
                i += 1; continue;
            };

            let mut k = 0;
            while k < chars.len() {
                let c = chars[k];
                match c {
                's' => { i += 1; if i < cmd_args.len() { session_name = Some(cmd_args[i].to_string()); } break; }
                'n' => { i += 1; if i < cmd_args.len() { window_name = Some(cmd_args[i].to_string()); } break; }
                'F' => { i += 1; if i < cmd_args.len() { format_str = Some(cmd_args[i].trim_matches('"').to_string()); } break; }
                'c' => { i += 1; if i < cmd_args.len() { start_dir = Some(cmd_args[i].trim_matches('"').to_string()); } break; }
                'x' => { i += 1; if i < cmd_args.len() { init_width = cmd_args[i].parse::<u16>().ok(); } break; }
                'y' => { i += 1; if i < cmd_args.len() { init_height = cmd_args[i].parse::<u16>().ok(); } break; }
                'e' => {
                    i += 1;
                    match crate::util::parse_new_session_e_value_token(
                        cmd_args.get(i).map(|s| s.as_str()),
                    ) {
                        Ok(pair) => env_vars.push(pair),
                        Err(msg) => {
                            return Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
                        }
                    }
                    break;
                }
                'f' => { i += 1; break; /* skip value */ }
                't' => { i += 1; if i < cmd_args.len() { group_target = Some(cmd_args[i].to_string()); } break; }
                // Boolean flags
                'd' => { detached = true; }
                'P' => { print_info = true; }
                'A' => { attach_if_exists = true; }
                'D' | 'E' | 'X' => { /* ignored for compatibility */ }
                _ => { /* unknown flag, skip */ }
                }
                k += 1;
            }
            i += 1;
        }
    }

    let name = session_name.unwrap_or_else(|| {
        // tmux-compatible: auto-generate numeric name (0, 1, 2, ...)
        crate::session::next_session_name(l_socket_name.as_deref())
    });
    // Compute port file base name: with -L namespace prefix if specified
    let port_file_base = if let Some(ref l) = l_socket_name {
        format!("{}__{}", l, name)
    } else {
        name.clone()
    };
    // Check for -- separator: everything after it is a raw command (direct execution)
    let raw_cmd_args: Option<Vec<String>> = raw_cmd_after_dd.filter(|v| !v.is_empty());
    // Parse initial command from positional args (legacy mode, no --)
    let initial_cmd: Option<String> = if raw_cmd_args.is_some() || positional_args.is_empty() {
        None
    } else {
        Some(positional_args.join(" "))
    };

    // Check if session already exists AND is actually running
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);
    if std::path::Path::new(&port_path).exists() {
        // Verify server is actually running
        let server_alive = if let Ok(port_str) = std::fs::read_to_string(&port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                let addr = format!("127.0.0.1:{}", port);
                std::net::TcpStream::connect_timeout(
                    &addr.parse().unwrap(),
                    Duration::from_millis(100)
                ).is_ok()
            } else { false }
        } else { false };

        if server_alive {
            if attach_if_exists {
                // -A flag: attach to existing session instead of erroring
                env::set_var("PSMUX_SESSION_NAME", &port_file_base);
                env::set_var("PSMUX_REMOTE_ATTACH", "1");
                // Skip server creation, jump straight to attach
            } else {
                eprintln!("duplicate session: {}", name);
                std::process::exit(1);
            }
        } else {
            // Stale port file - remove it and continue
            let _ = std::fs::remove_file(&port_path);
        }
    }

    // If -A attached to an existing session, skip server creation
    if env::var("PSMUX_REMOTE_ATTACH").ok().as_deref() == Some("1") {
        // Already set up for attach — skip server spawn
    } else {
    // Fast path: try to claim a pre-spawned warm server.
    let warm_disabled = std::env::var("PSMUX_NO_WARM").map(|v| v == "1" || v == "true").unwrap_or(false)
        || crate::config::is_warm_disabled_by_config();
    let has_custom_config = f_config_file.is_some() || std::env::var("PSMUX_CONFIG_FILE").is_ok();
    let claimed_warm = if !warm_disabled && !has_custom_config && initial_cmd.is_none() && raw_cmd_args.is_none() && start_dir.is_none() && env_vars.is_empty() {
        let warm_base = if let Some(ref l) = l_socket_name {
            format!("{}____warm__", l)
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
                        Duration::from_millis(100),
                    ).is_ok() {
                        let warm_key = crate::session::read_session_key(&warm_base).unwrap_or_default();
                        if !warm_key.is_empty() {
                            let client_cwd = std::env::current_dir()
                                .ok()
                                .and_then(|p| p.to_str().map(|s| s.to_string()));
                            let claim_cmd = if let Some(ref cwd) = client_cwd {
                                format!("claim-session {} {}\n", crate::util::quote_arg(&name), crate::util::quote_arg(cwd))
                            } else {
                                format!("claim-session {}\n", crate::util::quote_arg(&name))
                            };
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
                    } else {
                        let _ = std::fs::remove_file(&warm_port_path);
                        false
                    }
                } else { false }
            } else { false }
        } else { false }
    } else { false };

    if !claimed_warm {
    // Cold path: spawn a background server from scratch
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), name.clone()];
    // Pass -L socket name to server for namespace isolation
    if let Some(ref l) = l_socket_name {
        server_args.push("-L".into());
        server_args.push(l.clone());
    }
    // Pass initial command if provided
    if let Some(ref init_cmd) = initial_cmd {
        server_args.push("-c".into());
        server_args.push(init_cmd.clone());
    }
    // Pass start directory to server
    if let Some(ref dir) = start_dir {
        server_args.push("-d".into());
        server_args.push(dir.clone());
    }
    // Pass window name to server
    if let Some(ref wn) = window_name {
        server_args.push("-n".into());
        server_args.push(wn.clone());
    }
    // Pass initial dimensions to server
    if let Some(w) = init_width {
        server_args.push("-x".into());
        server_args.push(w.to_string());
    }
    if let Some(h) = init_height {
        server_args.push("-y".into());
        server_args.push(h.to_string());
    }
    // Pass session group target to server
    if let Some(ref gt) = group_target {
        server_args.push("-g".into());
        server_args.push(gt.clone());
    }
    // Pass -e environment variables to server
    for (k, v) in &env_vars {
        server_args.push("-e".into());
        server_args.push(format!("{}={}", k, v));
    }
    // Pass raw command args (direct execution) if -- was used
    if let Some(ref raw_args) = raw_cmd_args {
        server_args.push("--".into());
        for a in raw_args {
            server_args.push(a.clone());
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
            fn SetHandleInformation(hObject: *mut std::ffi::c_void, dwMask: u32, dwFlags: u32) -> i32;
        }
        const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5u32; // -11i32 as u32
        const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4u32;  // -12i32 as u32
        const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
        unsafe {
            let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
            let stderr = GetStdHandle(STD_ERROR_HANDLE);
            SetHandleInformation(stdout, HANDLE_FLAG_INHERIT, 0);
            SetHandleInformation(stderr, HANDLE_FLAG_INHERIT, 0);
        }
    }
    #[cfg(windows)]
    crate::platform::spawn_server_hidden(&exe, &server_args)?;
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new(&exe);
        for a in &server_args { cmd.arg(a); }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn server: {e}")))?;
    }
    } // end if !claimed_warm (cold path)
    } // end else (not PSMUX_REMOTE_ATTACH)

    // Wait for server to create port file (up to 5 seconds)
    for _ in 0..500 {
        if std::path::Path::new(&port_path).exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Verify the server is actually alive
    if !std::path::Path::new(&port_path).exists() {
        eprintln!("psmux: failed to create session '{}'", name);
        std::process::exit(1);
    }
    {
        let server_alive = if let Ok(port_str) = std::fs::read_to_string(&port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                let addr = format!("127.0.0.1:{}", port);
                std::net::TcpStream::connect_timeout(
                    &addr.parse().unwrap(),
                    Duration::from_millis(100)
                ).is_ok()
            } else { false }
        } else { false };
        if !server_alive {
            let _ = std::fs::remove_file(&port_path);
            eprintln!("psmux: session '{}' exited immediately (check shell command)", name);
            std::process::exit(1);
        }
    }

    if detached {
        // If -P flag, print pane info before returning
        if print_info {
            // Set target session so send_control_with_response connects to the right server
            env::set_var("PSMUX_TARGET_SESSION", &port_file_base);
            // Give server a moment to initialize
            std::thread::sleep(Duration::from_millis(200));
            // Query the server for pane info using display-message
            let fmt = if let Some(ref f) = format_str {
                f.clone()
            } else {
                // tmux default: new-session -P prints "session_name:"
                "#{session_name}:".to_string()
            };
            match send_control_with_response(format!("display-message -p {}\n", fmt)) {
                Ok(resp) => { let trimmed = resp.trim(); if !trimmed.is_empty() { println!("{}", trimmed); } }
                Err(_) => {}
            }
        }
        return Ok(false); // done, caller should return Ok(())
    } else {
        // User wants attached session - set env vars to attach
        env::set_var("PSMUX_SESSION_NAME", &port_file_base);
        env::set_var("PSMUX_REMOTE_ATTACH", "1");
        return Ok(true); // caller should continue to TUI attach
    }
}
