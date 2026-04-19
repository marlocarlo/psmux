#[allow(unused_imports)]
use std::io::{self, Write};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL};
#[cfg(windows)]
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
#[cfg(windows)]
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

use crate::types::{AppState, Mode, CopyModeState};
use crate::tree::{active_pane, active_pane_mut};

/// Emit an OSC 52 escape sequence to set the terminal clipboard.
/// This works over SSH because the sequence travels through the SSH pipe
/// to the local terminal emulator (e.g. Windows Terminal, iTerm2).
/// The `writer` should be the client's stdout (not the server's).
use super::*;

/// Select "inner WORD" (iW) — whitespace-delimited token without surrounding whitespace.
pub fn select_inner_word_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, _cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let col = c as usize;
    if col >= bytes.len() { return; }
    if bytes[col].is_whitespace() {
        // Cursor on whitespace — select contiguous whitespace
        let mut start = col;
        while start > 0 && bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    } else {
        // Cursor on non-whitespace — select contiguous non-whitespace
        let mut start = col;
        while start > 0 && !bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && !bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    }
    app.copy_selection_mode = crate::types::SelectionMode::Char;
}

/// Select "a WORD" (aW) — whitespace-delimited token plus trailing whitespace.
pub fn select_a_word_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, _cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let col = c as usize;
    if col >= bytes.len() { return; }
    if bytes[col].is_whitespace() {
        // Cursor on whitespace — select contiguous whitespace
        let mut start = col;
        while start > 0 && bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    } else {
        // Cursor on non-whitespace — select contiguous non-whitespace
        let mut start = col;
        while start > 0 && !bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && !bytes[end + 1].is_whitespace() { end += 1; }
        // Include trailing whitespace
        while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    }
    app.copy_selection_mode = crate::types::SelectionMode::Char;
}
