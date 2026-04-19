//! Cross-session pane transfer orchestration.
//!
//! Coordinates moving a pane between two session servers via a TCP I/O
//! tunnel.  The real ConPTY stays in the source process; the target gets
//! a proxy pane whose reads/writes are forwarded over the tunnel.
//!
//! Protocol flow (driven by the CLI process in main.rs):
//!
//!   1. CLI sends `pane-forward-extract <window>.<pane>` to **source** session
//!      Source replies: `FORWARD <forward_id> <listen_port> <pid> <title> <rows> <cols> <screen_b64_len>\n<screen_b64>`
//!
//!   2. CLI sends `pane-forward-inject <source_session> <source_addr> <source_key>
//!      <forward_id> <pid> <title> <rows> <cols> <screen_b64_len>\n<screen_b64>`
//!      to **target** session
//!      Target creates a ProxyMasterPty, connects the I/O tunnel, inserts pane.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Resolve a session name to (port, key).
pub fn resolve_session(session_name: &str) -> io::Result<(u16, String)> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let port_path = format!("{}\\.psmux\\{}.port", home, session_name);
    let port: u16 = std::fs::read_to_string(&port_path)
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound,
            format!("no server for session '{}'", session_name)))?
        .trim()
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad port file"))?;
    let key = crate::session::read_session_key(session_name).unwrap_or_default();
    Ok((port, key))
}

/// Send a command to a specific session and return the full response.
fn send_to_session(port: u16, key: &str, cmd: &str) -> io::Result<String> {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{}", e)))?,
        Duration::from_millis(2000),
    )?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(5000)));
    write!(stream, "AUTH {}\n{}\n", key, cmd)?;
    stream.flush()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 65536];
    const MAX_RESPONSE: usize = 4 * 1024 * 1024; // 4 MB cap
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > MAX_RESPONSE {
                    return Err(io::Error::new(io::ErrorKind::InvalidData,
                        "response exceeded 4 MB limit"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock
                   || e.kind() == io::ErrorKind::TimedOut => break,
            Err(_) => break,
        }
    }
    let r = String::from_utf8_lossy(&buf).to_string();
    Ok(if r.starts_with("OK\n") { r[3..].to_string() } else { r })
}

/// Orchestrate a cross-session pane transfer.
///
/// Called from main.rs when join-pane's `-s` session differs from `-t` session.
/// Returns Ok(()) on success or an error description.
pub fn orchestrate_cross_session_join(
    src_session: &str,
    src_window: usize,
    src_pane: usize,
    tgt_session: &str,
    tgt_window: Option<usize>,
    tgt_pane: Option<usize>,
    horizontal: bool,
) -> io::Result<()> {
    // 1. Resolve both sessions
    let (src_port, src_key) = resolve_session(src_session)?;
    let (tgt_port, tgt_key) = resolve_session(tgt_session)?;
    let src_addr = format!("127.0.0.1:{}", src_port);

    // 2. Tell source to extract the pane and start forwarding
    let extract_cmd = format!("pane-forward-extract {}.{}", src_window, src_pane);
    let extract_resp = send_to_session(src_port, &src_key, &extract_cmd)?;

    // Parse: FORWARD <forward_id> <listen_port> <pid> <title> <rows> <cols> <screen_b64_len>
    // followed by optional base64 screen data
    let extract_resp = extract_resp.trim();
    if !extract_resp.starts_with("FORWARD ") {
        return Err(io::Error::new(io::ErrorKind::Other,
            format!("extract failed: {}", extract_resp)));
    }

    let parts: Vec<&str> = extract_resp.splitn(8, ' ').collect();
    if parts.len() < 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad FORWARD response"));
    }
    let forward_id: u64 = parts[1].parse().unwrap_or(0);
    let fwd_port: u16 = parts[2].parse().unwrap_or(0);
    let pid: u32 = parts[3].parse().unwrap_or(0);
    let title = parts[4].replace('\x01', " "); // spaces encoded as \x01
    let rows: u16 = parts[5].parse().unwrap_or(24);
    let cols: u16 = parts[6].parse().unwrap_or(80);
    let screen_b64_len: usize = parts[7].parse().unwrap_or(0);

    // Read screen base64 data if present (may follow the FORWARD line)
    let screen_b64 = if screen_b64_len > 0 {
        // The screen data follows after the first newline in the response
        if let Some(nl_pos) = extract_resp.find('\n') {
            let data = &extract_resp[nl_pos + 1..];
            if data.len() >= screen_b64_len {
                Some(data[..screen_b64_len].to_string())
            } else {
                Some(data.to_string())
            }
        } else {
            None
        }
    } else {
        None
    };

    // 3. Build inject command for target
    let _tgt_spec = match (tgt_window, tgt_pane) {
        (Some(w), Some(p)) => format!("{}.{}", w, p),
        (Some(w), None) => format!("{}", w),
        _ => String::new(),
    };
    let h_flag = if horizontal { " -h" } else { "" };
    let screen_payload = screen_b64.as_deref().unwrap_or("");
    let inject_cmd = format!(
        "pane-forward-inject {} {} {} {} {} {} {} {} {} {}{}\n{}",
        src_session,
        src_addr,
        src_key,
        forward_id,
        fwd_port,
        pid,
        title.replace(' ', "\x01"),
        rows,
        cols,
        screen_payload.len(),
        h_flag,
        screen_payload,
    );

    // 4. Tell target to create proxy pane
    let inject_resp = send_to_session(tgt_port, &tgt_key, &inject_cmd)?;
    if inject_resp.trim().starts_with("ERR") {
        return Err(io::Error::new(io::ErrorKind::Other,
            format!("inject failed: {}", inject_resp.trim())));
    }

    Ok(())
}
