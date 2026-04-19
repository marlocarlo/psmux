use std::io::{self, Write, BufRead as _};
use std::time::Duration;
use std::env;

/// Handle default session creation (bare `psmux` with no command).
/// Creates a new session or claims a warm server, then sets env vars for TUI attach.
pub(crate) fn handle_default_session(l_socket_name: &Option<String>) -> io::Result<()> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let session_name = env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| {
        crate::session::next_session_name(l_socket_name.as_deref())
    });
    let port_file_base = if let Some(ref l) = l_socket_name {
        format!("{}__{}", l, session_name)
    } else {
        session_name.clone()
    };
    let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);

    // Try warm server claim first (fast path)
    // Skipped when PSMUX_NO_WARM=1 is set or config has 'set -g warm off'.
    let warm_disabled = std::env::var("PSMUX_NO_WARM").map(|v| v == "1" || v == "true").unwrap_or(false)
        || crate::config::is_warm_disabled_by_config();
    let warm_base = if let Some(ref l) = l_socket_name {
        format!("{}____warm__", l)
    } else {
        "__warm__".to_string()
    };
    let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
    let mut warm_claimed = false;
    if !warm_disabled && std::path::Path::new(&warm_port_path).exists() {
        let warm_key = crate::session::read_session_key(&warm_base).unwrap_or_default();
        if let Ok(port_str) = std::fs::read_to_string(&warm_port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                let addr = format!("127.0.0.1:{}", port);
                if let Ok(mut stream) = std::net::TcpStream::connect_timeout(
                    &addr.parse().unwrap(),
                    Duration::from_millis(500),
                ) {
                    let _ = stream.set_nodelay(true);
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(3000)));
                    let _ = write!(stream, "AUTH {}\n", warm_key);
                    let client_cwd = std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(|s| s.to_string()));
                    if let Some(ref cwd) = client_cwd {
                        let _ = write!(stream, "claim-session {} {}\n", crate::util::quote_arg(&session_name), crate::util::quote_arg(cwd));
                    } else {
                        let _ = write!(stream, "claim-session {}\n", crate::util::quote_arg(&session_name));
                    }
                    let _ = stream.flush();
                    if let Ok(reader_stream) = stream.try_clone() {
                        let mut br = std::io::BufReader::new(reader_stream);
                        let mut auth_line = String::new();
                        if std::io::BufRead::read_line(&mut br, &mut auth_line).unwrap_or(0) > 0
                            && auth_line.trim().starts_with("OK")
                        {
                            let mut claim_line = String::new();
                            if std::io::BufRead::read_line(&mut br, &mut claim_line).unwrap_or(0) > 0
                                && claim_line.contains("OK")
                            {
                                warm_claimed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    if !warm_claimed {
        // Cold path: spawn a new background server
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
        let server_args: Vec<String> = vec!["server".into(), "-s".into(), session_name.clone()];
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

        // Wait for server to start (fast polling — port file is written early)
        for _ in 0..500 {
            if std::path::Path::new(&port_path).exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // Now attach to the session
    env::set_var("PSMUX_SESSION_NAME", &port_file_base);
    env::set_var("PSMUX_REMOTE_ATTACH", "1");
    Ok(())
}

/// Run as a control mode client (psmux -C or psmux -CC).
/// Connects to the server via TCP, sends CONTROL/CONTROL_NOECHO,
/// reads commands from stdin and prints responses/notifications to stdout.
pub(crate) fn run_control_mode(mode: u8) -> io::Result<()> {
    use std::net::TcpStream;

    let session_name = env::var("PSMUX_SESSION_NAME")
        .unwrap_or_else(|_| "default".to_string());
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME"))
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    let psmux_dir = format!("{}\\.psmux", home);

    // Read port and key
    let port_path = format!("{}\\{}.port", psmux_dir, session_name);
    let key_path = format!("{}\\{}.key", psmux_dir, session_name);

    let port_str = std::fs::read_to_string(&port_path)
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("session '{}' not found (no port file)", session_name)))?;
    let port: u16 = port_str.trim().parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "corrupted port file"))?;
    let key = std::fs::read_to_string(&key_path)
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "session key file not found"))?
        .trim().to_string();

    // Connect
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, format!("cannot connect to session: {}", e)))?;
    let _ = stream.set_nodelay(true);

    // Auth
    write!(stream, "AUTH {}\n", key)?;
    stream.flush()?;

    // Read OK response
    let mut reader = io::BufReader::new(stream.try_clone()?);
    let mut ok_line = String::new();
    reader.read_line(&mut ok_line)?;
    if !ok_line.trim().starts_with("OK") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("auth failed: {}", ok_line.trim())));
    }

    // Send CONTROL or CONTROL_NOECHO
    let mode_str = if mode == 1 { "CONTROL" } else { "CONTROL_NOECHO" };
    let mut write_stream = reader.get_ref().try_clone()?;
    write!(write_stream, "{}\n", mode_str)?;
    write_stream.flush()?;

    // Spawn a thread to read server responses/notifications and print to stdout
    let reader_stream = reader.get_ref().try_clone()?;
    let reader_thread = std::thread::spawn(move || {
        let mut br = io::BufReader::new(reader_stream);
        let mut line = String::new();
        let stdout = io::stdout();
        loop {
            line.clear();
            match br.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let mut out = stdout.lock();
                    let _ = out.write_all(line.as_bytes());
                    let _ = out.flush();
                }
            }
        }
    });

    // Read commands from stdin and send to server
    let stdin = io::stdin();
    let mut stdin_line = String::new();
    loop {
        stdin_line.clear();
        match stdin.read_line(&mut stdin_line) {
            Ok(0) => break, // EOF
            Err(_) => break,
            Ok(_) => {
                if write!(write_stream, "{}", stdin_line).is_err() { break; }
                if write_stream.flush().is_err() { break; }
            }
        }
    }

    // After stdin EOF, shut down the write side only
    let _ = write_stream.shutdown(std::net::Shutdown::Write);

    // Wait briefly for the reader thread to drain remaining responses
    let handle = reader_thread;
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done2 = done.clone();
    std::thread::spawn(move || {
        let _ = handle.join();
        done2.store(true, std::sync::atomic::Ordering::Release);
    });
    // Drain for up to 2 seconds, then exit
    for _ in 0..40 {
        if done.load(std::sync::atomic::Ordering::Acquire) { break; }
        std::thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}
