use super::*;
use super::run_remote_state::RunRemoteState;

/// Send batched commands to the server and request a dump-state if rate limits allow.
/// Returns `true` if the writer encountered an error (caller should break).
pub(crate) fn send_commands_and_request_dump(
    state: &mut RunRemoteState,
    writer: &mut impl Write,
    terminal_size: (u16, u16),
    is_ssh_mode: bool,
    typing_active: bool,
    since_dump: u64,
    size_changed: &mut bool,
) -> bool {
    let new_size = (terminal_size.0, terminal_size.1.saturating_sub(state.last_status_lines));
    if new_size != state.last_sent_size {
        state.last_sent_size = new_size;
        *size_changed = true;
        if writer.write_all(format!("client-size {} {}\n", new_size.0, new_size.1).as_bytes()).is_err() {
            return true;
        }
        if is_ssh_mode {
            crate::ssh_input::send_mouse_enable();
            state.last_mouse_enable = Instant::now();
        }
    }

    let sent_keys = !state.cmd_batch.is_empty();
    if sent_keys {
        if input_log_enabled() {
            for cmd in &state.cmd_batch { input_log("send", &format!("-> {}", cmd.trim())); }
        }
        for cmd in &state.cmd_batch {
            if writer.write_all(cmd.as_bytes()).is_err() { return true; }
        }
        let _ = writer.flush();
        state.last_key_send_time = Some(Instant::now());
        state.key_send_instant = Some(Instant::now());
        state.force_dump = true;
    }

    // Rate-limited dump-state request
    let should_dump = if state.force_dump || *size_changed { true }
        else if typing_active { since_dump >= 10 }
        else { false };
    if should_dump && !state.dump_in_flight {
        if writer.write_all(b"dump-state\n").is_err() { return true; }
        if writer.flush().is_err() { return true; }
        state.dump_in_flight = true;
        state.dump_flight_start = Instant::now();
    }

    false
}
