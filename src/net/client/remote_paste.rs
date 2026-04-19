use super::*;
use super::run_remote_state::RunRemoteState;

/// Windows paste detection state machine: check if the paste-pending buffer
/// has matured (confirmed via Ctrl+V Release, stage2 timeout, or 20ms expiry)
/// and flush accordingly.
#[cfg(windows)]
pub(crate) fn handle_paste_stage(
    state: &mut RunRemoteState,
    cmd_batch: &mut Vec<String>,
) {
    if state.paste_pend.is_empty() || state.paste_pend_start.is_none() {
        return;
    }

    let start = state.paste_pend_start.unwrap();
    let elapsed = start.elapsed();

    if state.paste_confirmed {
        // Ctrl+V Release already seen -> send as paste now
        if !state.paste_pend.is_empty() {
            if input_log_enabled() {
                input_log("paste", &format!("paste CONFIRMED, sending {} chars as send-paste: {:?}",
                    state.paste_pend.len(), &state.paste_pend.chars().take(200).collect::<String>()));
            }
            let encoded = base64_encode(&state.paste_pend);
            cmd_batch.push(format!("send-paste {}\n", encoded));
            state.paste_suppress_until = Some(Instant::now() + Duration::from_millis(200));
        }
        state.paste_pend.clear();
        state.paste_pend_start = None;
        state.paste_stage2 = false;
        state.paste_confirmed = false;
    } else if !state.paste_stage2 && elapsed > Duration::from_millis(20) {
        // 20ms window expired
        let has_non_ascii = state.paste_pend.chars().any(|c| !c.is_ascii());
        if state.paste_pend.len() >= 3 && !has_non_ascii {
            // >=3 ASCII chars in 20ms -> likely paste, enter stage 2.
            state.paste_stage2 = true;
            state.paste_stage2_last_len = state.paste_pend.len();
            if input_log_enabled() {
                input_log("paste", &format!("stage2: {} chars in 20ms, waiting for Ctrl+V Release", state.paste_pend.len()));
            }
        } else if state.paste_pend.len() >= 20 && has_non_ascii {
            // >=20 non-ASCII chars in 20ms -> almost certainly a paste
            state.paste_stage2 = true;
            state.paste_stage2_last_len = state.paste_pend.len();
            if input_log_enabled() {
                input_log("paste", &format!("stage2 (large non-ASCII): {} chars in 20ms", state.paste_pend.len()));
            }
        } else if state.paste_pend.len() >= 3 && has_non_ascii {
            // >=3 chars but contains non-ASCII (IME input) -> flush
            // immediately as normal text to avoid 300ms delay.
            if input_log_enabled() {
                input_log("paste", &format!("flush {} chars as normal (non-ASCII / IME detected)", state.paste_pend.len()));
            }
            flush_chars_as_text(&state.paste_pend, cmd_batch);
            state.paste_pend.clear();
            state.paste_pend_start = None;
        } else {
            // <3 chars -> normal typing, flush as send-text
            if input_log_enabled() {
                input_log("paste", &format!("flush {} chars as normal (< 3 in 20ms)", state.paste_pend.len()));
            }
            flush_chars_as_text(&state.paste_pend, cmd_batch);
            state.paste_pend.clear();
            state.paste_pend_start = None;
        }
    } else if state.paste_stage2 && elapsed > Duration::from_millis(300) {
        // Stage 2 timeout -> no Ctrl+V Release arrived.
        // Growth detection: if the buffer grew since last check, ConPTY
        // is still injecting characters (large paste). Extend the window.
        if state.paste_pend.len() > state.paste_stage2_last_len {
            state.paste_stage2_last_len = state.paste_pend.len();
            state.paste_pend_start = Some(Instant::now() - Duration::from_millis(280));
        } else {
            // Buffer stopped growing -> send as paste
            if input_log_enabled() {
                input_log("paste", &format!("stage2 timeout, sending {} chars as send-paste", state.paste_pend.len()));
            }
            let encoded = base64_encode(&state.paste_pend);
            cmd_batch.push(format!("send-paste {}\n", encoded));
            state.paste_pend.clear();
            state.paste_pend_start = None;
            state.paste_stage2 = false;
            state.paste_stage2_last_len = 0;
            state.paste_suppress_until = Some(Instant::now() + Duration::from_millis(200));
        }
    }
}

/// Handle the post-event paste flush: after processing all events in a poll
/// cycle on Windows, handle Event::Paste and clear stale paste state.
#[cfg(windows)]
pub(crate) fn handle_post_event_flush(
    state: &mut RunRemoteState,
    cmd_batch: &mut Vec<String>,
) {
    // If a bracketed paste event came through, suppress duplicate key events
    // that ConPTY might inject.
    if !state.paste_pend.is_empty() && state.paste_confirmed {
        if input_log_enabled() {
            input_log("paste", &format!("post-event flush: confirmed {} chars", state.paste_pend.len()));
        }
        let encoded = base64_encode(&state.paste_pend);
        cmd_batch.push(format!("send-paste {}\n", encoded));
        state.paste_suppress_until = Some(Instant::now() + Duration::from_millis(200));
        state.paste_pend.clear();
        state.paste_pend_start = None;
        state.paste_stage2 = false;
        state.paste_confirmed = false;
    }
}

/// Flush a string of characters as individual send-text / send-key commands.
#[cfg(windows)]
fn flush_chars_as_text(chars: &str, cmd_batch: &mut Vec<String>) {
    for c in chars.chars() {
        match c {
            '\n' => { cmd_batch.push("send-key enter\n".into()); }
            '\t' => { cmd_batch.push("send-key tab\n".into()); }
            ' '  => { cmd_batch.push("send-key space\n".into()); }
            _ => {
                let escaped = match c {
                    '"' => "\\\"".to_string(),
                    '\\' => "\\\\".to_string(),
                    _ => c.to_string(),
                };
                cmd_batch.push(format!("send-text \"{}\"\n", escaped));
            }
        }
    }
}

/// Handle zero-latency typing flush and Ctrl+V paste confirmation after the event loop.
#[cfg(windows)]
pub(crate) fn handle_zero_latency_paste_flush(
    state: &mut RunRemoteState,
    cmd_batch: &mut Vec<String>,
) {
    // Zero-latency typing flush: 1-2 chars with no paste in progress
    if !state.paste_confirmed && !state.paste_stage2
        && state.paste_pend.len() >= 1 && state.paste_pend.len() <= 2
    {
        if input_log_enabled() {
            input_log("paste", &format!("zero-latency flush {} char(s)", state.paste_pend.len()));
        }
        for c in state.paste_pend.chars() {
            match c {
                '\n' => { cmd_batch.push("send-key enter\n".into()); }
                '\t' => { cmd_batch.push("send-key tab\n".into()); }
                ' '  => { cmd_batch.push("send-key space\n".into()); }
                _ => {
                    let escaped = match c {
                        '"' => "\\\"".to_string(),
                        '\\' => "\\\\".to_string(),
                        _ => c.to_string(),
                    };
                    cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                }
            }
        }
        state.paste_pend.clear();
        state.paste_pend_start = None;
    }
    // Ctrl+V Release with pending buffer
    if state.paste_confirmed && !state.paste_pend.is_empty() {
        if input_log_enabled() {
            input_log("paste", &format!("paste CONFIRMED (post-event), {} chars", state.paste_pend.len()));
        }
        let encoded = base64_encode(&state.paste_pend);
        cmd_batch.push(format!("send-paste {}\n", encoded));
        state.paste_pend.clear();
        state.paste_pend_start = None;
        state.paste_stage2 = false;
        state.paste_confirmed = false;
        state.paste_suppress_until = Some(Instant::now() + Duration::from_millis(200));
    } else if state.paste_confirmed && state.paste_pend.is_empty() {
        let suppressed = state.paste_suppress_until.map_or(false, |t| Instant::now() < t);
        if !suppressed {
            if let Some(text) = read_from_system_clipboard() {
                if !text.is_empty() {
                    let encoded = base64_encode(&text);
                    cmd_batch.push(format!("send-paste {}\n", encoded));
                    state.paste_suppress_until = Some(Instant::now() + Duration::from_millis(200));
                }
            }
        }
        state.paste_confirmed = false;
    }
}
