use super::*;
use super::run_remote_state::RunRemoteState;

/// Handle post-draw operations: OSC 52 clipboard, bell, SSH mouse refresh.
pub(crate) fn handle_post_draw(state: &mut RunRemoteState, is_ssh_mode: bool) {
    // OSC 52 clipboard emit
    if let Some(ref text) = state.pending_osc52 {
        let b64 = base64_encode(text);
        let osc = format!("\x1b]52;c;{}\x07", b64);
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(osc.as_bytes());
        let _ = out.flush();
        state.pending_osc52 = None;
    }
    // Bell emit
    if state.pending_bell {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(b"\x07");
        let _ = out.flush();
        state.pending_bell = false;
    }
    // SSH mouse-enable refresh
    if is_ssh_mode && state.last_mouse_enable.elapsed().as_secs() >= 5 {
        crate::ssh_input::send_mouse_enable();
        state.last_mouse_enable = Instant::now();
    }
}

/// Handle latency logging and frame buffer management at end of render cycle.
pub(crate) fn handle_frame_end(
    state: &mut RunRemoteState,
    got_frame: bool,
    parse_us: u128,
    render_us: u128,
    since_dump: u64,
) {
    state.last_dump_time = Instant::now();
    if let (Some(ref mut log), Some(ks)) = (&mut state.latency_log, state.key_send_instant) {
        let elapsed_ms = ks.elapsed().as_millis();
        state.loop_count += 1;
        use std::io::Write;
        let _ = writeln!(log, "L{}: key->render {}ms  parse={}us  render={}us  json_len={}  since_dump={}",
            state.loop_count, elapsed_ms, parse_us, render_us, state.dump_buf.len(), since_dump);
        if got_frame && state.dump_buf != state.prev_dump_buf {
            let _ = writeln!(log, "L{}: ECHO VISIBLE after {}ms", state.loop_count, elapsed_ms);
            state.key_send_instant = None;
        }
    }
    state.selection_changed = false;
    if got_frame && state.dump_buf != state.prev_dump_buf {
        std::mem::swap(&mut state.prev_dump_buf, &mut state.dump_buf);
    }
    if got_frame && state.dump_buf != state.prev_dump_buf {
        state.key_send_instant = None;
    }
    state.force_dump = false;
}
