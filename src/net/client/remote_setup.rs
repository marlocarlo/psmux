use super::*;

pub(crate) type FrameRx = std::sync::mpsc::Receiver<String>;

pub(crate) struct RemoteConnection {
    pub frame_rx: FrameRx,
    pub writer: std::net::TcpStream,
    pub name: String,
    pub home: String,
    pub is_ssh_mode: bool,
}

/// Establish the TCP connection to the psmux server, authenticate, enter
/// persistent mode, attach, and spawn the reader thread.
pub(crate) fn setup_connection() -> io::Result<RemoteConnection> {
    let name = env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| "default".to_string());
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let path = format!("{}\\.psmux\\{}.port", home, name);
    let port = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, format!("can't find session '{}' (no server running)", name)))?;
    let addr = format!("127.0.0.1:{}", port);
    let session_key = read_session_key(&name).unwrap_or_default();
    let last_path = format!("{}\\.psmux\\last_session", home);
    if !crate::session::is_warm_session(&name) {
        let _ = std::fs::write(&last_path, &name);
    }

    // ── Open persistent TCP connection ───────────────────────────────────
    let stream = std::net::TcpStream::connect(&addr)?;
    stream.set_nodelay(true)?;
    let mut writer = stream.try_clone()?;
    writer.set_nodelay(true)?;
    let mut reader = BufReader::new(stream);

    // AUTH handshake
    let _ = writer.write_all(format!("AUTH {}\n", session_key).as_bytes());
    let _ = writer.flush();
    let mut auth_line = String::new();
    reader.read_line(&mut auth_line)?;
    if !auth_line.trim().starts_with("OK") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "auth failed"));
    }

    // Enter persistent mode + attach
    let _ = writer.write_all(b"PERSISTENT\n");
    let _ = writer.write_all(b"client-attach\n");
    let _ = writer.flush();

    // Spawn a dedicated reader thread so the event loop never blocks on I/O.
    // Use a 2-second read timeout so the thread unblocks periodically.
    let _ = reader.get_ref().set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = String::with_capacity(64 * 1024);
        loop {
            buf.clear();
            loop {
                match reader.read_line(&mut buf) {
                    Ok(0) => return, // EOF
                    Ok(_) => break,
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        continue;
                    }
                    Err(_) => return,
                }
            }
            let line = std::mem::take(&mut buf);
            buf = String::with_capacity(64 * 1024);
            if frame_tx.send(line).is_err() { return; }
        }
    });

    let is_ssh_mode = crate::ssh_input::needs_vt_input();

    Ok(RemoteConnection { frame_rx, writer, name, home, is_ssh_mode })
}
