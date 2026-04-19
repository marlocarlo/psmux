use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::DumpState;

/// Copy the current selection to clipboard and clear selection state.
pub(crate) fn copy_and_clear_selection(state: &mut RunRemoteState) {
    if let (Some(s), Some(e)) = (state.rsel_start, state.rsel_end) {
        if state.rsel_dragged {
            if let Ok(dump) = serde_json::from_str::<DumpState>(&state.prev_dump_buf) {
                let text = extract_selection_text(
                    &dump.layout,
                    state.last_sent_size.0,
                    state.last_sent_size.1,
                    s, e,
                    state.rsel_block,
                );
                if !text.is_empty() {
                    copy_to_system_clipboard(&text);
                    state.pending_osc52 = Some(text);
                }
            }
        }
    }
    state.rsel_start = None;
    state.rsel_end = None;
    state.rsel_pane_rect = None;
    state.rsel_block = false;
    state.rsel_dragged = false;
    state.selection_changed = true;
}

/// Kill a remote session by connecting to its TCP port and sending kill-session.
pub(crate) fn kill_remote_session(name: &str) {
    let h = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let port_path = format!("{}\\.psmux\\{}.port", h, name);
    let key_path = format!("{}\\.psmux\\{}.key", h, name);
    if let Ok(port_str) = std::fs::read_to_string(&port_path) {
        if let Ok(port) = port_str.trim().parse::<u16>() {
            let addr = format!("127.0.0.1:{}", port);
            let sess_key = std::fs::read_to_string(&key_path).unwrap_or_default();
            if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                &addr.parse().unwrap(), Duration::from_millis(100)
            ) {
                let _ = write!(ss, "AUTH {}\n", sess_key.trim());
                let _ = ss.write_all(b"kill-session\n");
            }
        }
    }
}
