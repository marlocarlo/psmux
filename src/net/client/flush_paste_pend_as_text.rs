#[allow(unused_imports)]
use std::io::{self, Write, BufRead, BufReader};
use std::time::{Duration, Instant};
use std::env;

use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyModifiers, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::layout::LayoutJson;
use crate::help;
use crate::util::{WinTree, base64_encode, quote_arg};
use crate::session::read_session_key;
use crate::rendering::{dim_predictions_enabled, map_color, dim_color, centered_rect, fix_border_intersections};
use crate::style::parse_tmux_style_components;
use crate::config::{parse_key_string, normalize_key_for_binding};
use crate::copy_mode::{copy_to_system_clipboard, read_from_system_clipboard};
use crate::debug_log::{client_log, client_log_enabled, input_log, input_log_enabled};
use crate::layout::RowRunsJson;
use crate::tree::split_with_gaps;

/// Build a send-key name with modifier prefix (e.g. "C-Left", "S-Right", "C-S-Up").
use super::*;

/// Flush the paste-pending buffer as individual send-text / send-key commands.
/// Called when a non-bufferable key (Backspace, Delete, Esc, BackTab) interrupts
/// a potential paste burst, so we emit whatever we had as normal keystrokes.
#[cfg(windows)]
pub(crate) fn flush_paste_pend_as_text(
    paste_pend: &mut String,
    paste_pend_start: &mut Option<Instant>,
    paste_stage2: &mut bool,
    cmd_batch: &mut Vec<String>,
) {
    if paste_pend.is_empty() {
        return;
    }
    // If we accumulated enough ASCII chars that stage2 was entered, this
    // is almost certainly pasted content — send as send-paste so the server
    // wraps it in bracketed paste sequences (fixes nvim autoindent).
    // Non-ASCII buffers (IME input) are always flushed as normal text to
    // avoid the 300ms delay (fixes #91).
    let has_non_ascii = paste_pend.chars().any(|c| !c.is_ascii());
    if (*paste_stage2 || paste_pend.len() >= 3) && !has_non_ascii {
        let encoded = crate::util::base64_encode(paste_pend);
        cmd_batch.push(format!("send-paste {}\n", encoded));
    } else {
        for c in paste_pend.chars() {
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
    paste_pend.clear();
    *paste_pend_start = None;
    *paste_stage2 = false;
}

/// Returns true if the buffer contains any non-ASCII characters (IME / CJK input).
/// Used by the paste detection heuristic to skip Stage 2 for IME input (fixes #91).
#[cfg(windows)]
pub(crate) fn paste_buffer_has_non_ascii(buf: &str) -> bool {
    buf.chars().any(|c| !c.is_ascii())
}

#[cfg(test)]
#[path = "../../../tests-rs/test_client.rs"]
mod tests;
