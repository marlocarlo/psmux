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

/// Move to bottom of visible screen — L key
pub fn move_to_screen_bottom(app: &mut AppState) {
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    app.copy_pos = Some((rows.saturating_sub(1), 0));
}

/// Find character forward on current line — f key
pub fn find_char_forward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (c as usize + 1)..bytes.len() {
            if bytes[i] == ch { app.copy_pos = Some((r, i as u16)); return; }
        }
    }
}

/// Find character backward on current line — F key
pub fn find_char_backward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (0..(c as usize)).rev() {
            if bytes[i] == ch { app.copy_pos = Some((r, i as u16)); return; }
        }
    }
}

/// Find char up to (not including) forward — t key
pub fn find_char_to_forward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (c as usize + 1)..bytes.len() {
            if bytes[i] == ch { app.copy_pos = Some((r, (i as u16).saturating_sub(1))); return; }
        }
    }
}

/// Find char up to (not including) backward — T key
pub fn find_char_to_backward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (0..(c as usize)).rev() {
            if bytes[i] == ch { app.copy_pos = Some((r, (i as u16) + 1)); return; }
        }
    }
}

/// Yank from cursor to end of line — D key
pub fn copy_end_of_line(app: &mut AppState) -> io::Result<()> {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return Ok(()) };
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(()) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(()) };
    let screen = parser.screen();
    let cols = p.last_cols;
    let mut text = String::new();
    for col in c..cols {
        if let Some(cell) = screen.cell(r, col) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); }
    }
    let text = text.trim_end().to_string();
    app.paste_buffers.insert(0, text.clone());
    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
    copy_to_system_clipboard(&text);
    Ok(())
}

/// Jump to the previous search match in copy mode.
pub fn search_prev(app: &mut AppState) {
    if app.copy_search_matches.is_empty() { return; }
    let wrap = app.user_options.get("wrap-search").map(|v| v.as_str()) != Some("off");
    if app.copy_search_idx == 0 {
        if !wrap { return; }
        app.copy_search_idx = app.copy_search_matches.len() - 1;
    } else {
        app.copy_search_idx -= 1;
    }
    let (r, c, _) = app.copy_search_matches[app.copy_search_idx];
    app.copy_pos = Some((r, c));
}

/// Compute the (start, end) row range for capture-pane given optional -S/-E
/// values and the last visible row index.
///
/// Tmux semantics (from cmd-capture-pane.c):
///   Negative -S means "N scrollback lines above visible". Since psmux only
///   exposes visible rows here, any negative start clamps to 0 (top of visible),
///   matching tmux behavior when no scrollback history is available.
///   Negative -E likewise clamps to 0.
pub fn compute_capture_range(s: Option<i32>, e: Option<i32>, last_row: u16) -> (u16, u16) {
    let start = match s {
        Some(v) if v < 0 => 0u16,
        Some(v) => (v as u16).min(last_row),
        None => 0,
    };
    let end = match e {
        Some(v) if v < 0 => 0u16,
        Some(v) => (v as u16).min(last_row),
        None => last_row,
    };
    (start, end)
}

pub fn capture_active_pane_range(app: &mut AppState, s: Option<i32>, e: Option<i32>) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let screen = parser.screen();
    let last_row = p.last_rows.saturating_sub(1);
    let (start, end) = compute_capture_range(s, e, last_row);
    let mut text = String::new();
    for r in start..=end {
        let mut row = String::new();
        for c in 0..p.last_cols { if let Some(cell) = screen.cell(r, c) { row.push_str(&cell.contents().to_string()); } else { row.push(' '); } }
        text.push_str(row.trim_end());
        text.push('\n');
    }
    Ok(Some(text))
}

/// Capture the active pane's screen content with ANSI escape sequences preserved.
/// This is the `-e` flag for capture-pane.  Supports optional start/end range.
pub fn capture_active_pane_styled(app: &mut AppState, s: Option<i32>, e: Option<i32>) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let screen = parser.screen();
    let last_row = p.last_rows.saturating_sub(1);
    let (start_row, end_row) = compute_capture_range(s, e, last_row);
    let mut text = String::new();
    let mut prev_fg: Option<vt100::Color> = None;
    let mut prev_bg: Option<vt100::Color> = None;
    let mut prev_bold = false;
    let mut prev_dim = false;
    let mut prev_italic = false;
    let mut prev_underline = false;
    let mut prev_blink = false;
    let mut prev_inverse = false;
    let mut prev_hidden = false;
    let mut prev_strikethrough = false;

    for r in start_row..=end_row {
        // Build the row content, then trim trailing whitespace
        let mut row_chars: Vec<String> = Vec::new();
        let mut row_sgr: Vec<Option<String>> = Vec::new();
        let mut any_style_active = false;
        for c in 0..p.last_cols {
            if let Some(cell) = screen.cell(r, c) {
                let fg = cell.fgcolor();
                let bg = cell.bgcolor();
                let bold = cell.bold();
                let dim = cell.dim();
                let italic = cell.italic();
                let underline = cell.underline();
                let blink = cell.blink();
                let inverse = cell.inverse();
                let hidden = cell.hidden();
                let strikethrough = cell.strikethrough();

                // Emit SGR if attributes changed
                let style_changed = Some(fg) != prev_fg || Some(bg) != prev_bg
                    || bold != prev_bold || dim != prev_dim
                    || italic != prev_italic
                    || underline != prev_underline || blink != prev_blink
                    || inverse != prev_inverse || hidden != prev_hidden
                    || strikethrough != prev_strikethrough;

                let sgr = if style_changed {
                    let mut params = Vec::new();
                    params.push("0".to_string()); // reset first
                    if bold { params.push("1".to_string()); }
                    if dim { params.push("2".to_string()); }
                    if italic { params.push("3".to_string()); }
                    if underline { params.push("4".to_string()); }
                    if blink { params.push("5".to_string()); }
                    if inverse { params.push("7".to_string()); }
                    if hidden { params.push("8".to_string()); }
                    if strikethrough { params.push("9".to_string()); }
                    // Foreground
                    match fg {
                        vt100::Color::Default => {}
                        vt100::Color::Idx(n) => {
                            if n < 8 { params.push(format!("{}", 30 + n)); }
                            else if n < 16 { params.push(format!("{}", 90 + n - 8)); }
                            else { params.push(format!("38;5;{}", n)); }
                        }
                        vt100::Color::Rgb(r, g, b) => { params.push(format!("38;2;{};{};{}", r, g, b)); }
                    }
                    // Background
                    match bg {
                        vt100::Color::Default => {}
                        vt100::Color::Idx(n) => {
                            if n < 8 { params.push(format!("{}", 40 + n)); }
                            else if n < 16 { params.push(format!("{}", 100 + n - 8)); }
                            else { params.push(format!("48;5;{}", n)); }
                        }
                        vt100::Color::Rgb(r, g, b) => { params.push(format!("48;2;{};{};{}", r, g, b)); }
                    }
                    prev_fg = Some(fg);
                    prev_bg = Some(bg);
                    prev_bold = bold;
                    prev_dim = dim;
                    prev_italic = italic;
                    prev_underline = underline;
                    prev_blink = blink;
                    prev_inverse = inverse;
                    prev_hidden = hidden;
                    prev_strikethrough = strikethrough;
                    any_style_active = true;
                    Some(format!("\x1b[{}m", params.join(";")))
                } else {
                    None
                };
                row_sgr.push(sgr);
                row_chars.push(cell.contents().to_string());
            } else {
                row_sgr.push(None);
                row_chars.push(" ".to_string());
            }
        }
        // Find last non-whitespace cell to trim trailing spaces
        let last_non_ws = row_chars.iter().rposition(|s| !s.is_empty() && s.trim() != "");
        let trim_end = match last_non_ws {
            Some(pos) => pos + 1,
            None => 0,  // entirely empty row
        };
        for c in 0..trim_end {
            if let Some(ref sgr) = row_sgr[c] { text.push_str(sgr); }
            text.push_str(&row_chars[c]);
        }
        if any_style_active {
            text.push_str("\x1b[0m");
            prev_fg = None;
            prev_bg = None;
            prev_bold = false;
            prev_dim = false;
            prev_italic = false;
            prev_underline = false;
            prev_blink = false;
            prev_inverse = false;
            prev_hidden = false;
        }
        text.push('\n');
    }
    Ok(Some(text))
}

/// Move to next empty line (paragraph boundary) — } key
pub fn move_next_paragraph(app: &mut AppState) {
    let (r, _) = match get_copy_pos(app) { Some(p) => p, None => return };
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    // Skip current non-blank lines, then find next blank line
    let mut row = r + 1;
    // Skip non-blank
    while row < rows {
        if let Some((text, _)) = read_row_text(app, row) {
            if text.trim().is_empty() { break; }
        } else { break; }
        row += 1;
    }
    // Skip blank lines to find start of next paragraph
    while row < rows {
        if let Some((text, _)) = read_row_text(app, row) {
            if !text.trim().is_empty() { break; }
        } else { break; }
        row += 1;
    }
    app.copy_pos = Some((row.min(rows.saturating_sub(1)), 0));
}

/// Move to previous empty line (paragraph boundary) — { key
pub fn move_prev_paragraph(app: &mut AppState) {
    let (r, _) = match get_copy_pos(app) { Some(p) => p, None => return };
    if r == 0 { return; }
    let mut row = r.saturating_sub(1);
    // Skip non-blank
    loop {
        if let Some((text, _)) = read_row_text(app, row) {
            if text.trim().is_empty() { break; }
        } else { break; }
        if row == 0 { app.copy_pos = Some((0, 0)); return; }
        row -= 1;
    }
    // Skip blank lines
    loop {
        if let Some((text, _)) = read_row_text(app, row) {
            if !text.trim().is_empty() { break; }
        } else { break; }
        if row == 0 { app.copy_pos = Some((0, 0)); return; }
        row -= 1;
    }
    app.copy_pos = Some((row, 0));
}

/// Move to matching bracket — % key
pub fn move_matching_bracket(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let win = match app.windows.get(app.active_idx) { Some(w) => w, None => return };
    let p = match active_pane(&win.root, &win.active_path) { Some(p) => p, None => return };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    let screen = parser.screen();
    
    // Get char at cursor
    let ch = screen.cell(r, c).map(|cell| {
        let t = cell.contents();
        t.chars().next().unwrap_or(' ')
    }).unwrap_or(' ');
    
    let (open, close, forward) = match ch {
        '(' => ('(', ')', true),
        ')' => ('(', ')', false),
        '[' => ('[', ']', true),
        ']' => ('[', ']', false),
        '{' => ('{', '}', true),
        '}' => ('{', '}', false),
        '<' => ('<', '>', true),
        '>' => ('<', '>', false),
        _ => return,
    };
    
    let rows = p.last_rows;
    let cols = p.last_cols;
    let mut depth = 1i32;
    let mut cr = r;
    let mut cc = c;
    
    loop {
        if forward {
            cc += 1;
            if cc >= cols { cc = 0; cr += 1; }
            if cr >= rows { return; }
        } else {
            if cc == 0 {
                if cr == 0 { return; }
                cr -= 1;
                cc = cols.saturating_sub(1);
            } else { cc -= 1; }
        }
        
        let cell_ch = screen.cell(cr, cc).map(|cell| {
            cell.contents().chars().next().unwrap_or(' ')
        }).unwrap_or(' ');
        
        if cell_ch == open { depth += if forward { 1 } else { -1 }; }
        if cell_ch == close { depth += if forward { -1 } else { 1 }; }
        if depth == 0 {
            app.copy_pos = Some((cr, cc));
            return;
        }
    }
}
