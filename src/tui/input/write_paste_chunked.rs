#[allow(unused_imports)]
use std::io::{self, Write};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use portable_pty::native_pty_system;
use ratatui::prelude::*;

use crate::types::{AppState, Mode, FocusDir, LayoutKind, DragState, Node, Pane};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, path_exists, resize_all_panes};
use crate::pane::{create_window, split_active};
use crate::commands::{execute_action, execute_command_prompt, execute_command_string};
use crate::config::normalize_key_for_binding;
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, switch_with_copy_save, move_copy_cursor,
    scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, paste_latest, yank_selection,
    search_copy_mode, search_next, search_prev, scroll_to_top, scroll_to_bottom};
use crate::layout::{cycle_top_layout, apply_layout};
use crate::window_ops::{toggle_zoom, swap_pane, break_pane_to_window};

/// Write a mouse event to the child PTY using the encoding the child requested.
use super::*;

/// Chunked PTY write for paste delivery.  The PTY pipe can silently
/// drop bytes when a large payload (140+ lines) is written in a single
/// call because the OS pipe buffer fills up.  We split the text into
/// ~2 KiB chunks with small yields between them so the consumer
/// (shell / PSReadLine / nvim) has time to drain.  Bracket sequences
/// are tiny and always written in one shot.
pub(crate) fn write_paste_chunked(writer: &mut dyn std::io::Write, text: &[u8], bracket: bool) {
    const CHUNK: usize = 512;
    // Normalize line endings to CR for ConPTY.  Clipboard text may arrive
    // with LF (\n) or CRLF (\r\n), but ConPTY's input parser expects CR
    // (\r) for Enter.  Bare LF is misinterpreted by PSReadLine, causing
    // multi-line pastes to appear in reverse order.
    let text = {
        let mut out = Vec::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            if text[i] == b'\r' && i + 1 < text.len() && text[i + 1] == b'\n' {
                out.push(b'\r');
                i += 2; // CRLF → CR
            } else if text[i] == b'\n' {
                out.push(b'\r');
                i += 1; // LF → CR
            } else {
                out.push(text[i]);
                i += 1;
            }
        }
        out
    };
    let text = &text[..];
    if bracket { let _ = writer.write_all(b"\x1b[200~"); }
    let mut offset: usize = 0;
    while offset < text.len() {
        let remaining = (text.len() - offset).min(CHUNK);
        let chunk = &text[offset..offset + remaining];
        match writer.write(chunk) {
            Ok(0) => {
                // Zero bytes written — yield and retry once
                std::thread::sleep(std::time::Duration::from_millis(10));
                match writer.write(chunk) {
                    Ok(n) if n > 0 => { offset += n; }
                    _ => break, // give up on persistent failure
                }
            }
            Ok(n) => { offset += n; }
            Err(_) => break,
        }
        // Yield between chunks to let the consumer drain the buffer
        if offset < text.len() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    if bracket { let _ = writer.write_all(b"\x1b[201~"); }
    let _ = writer.flush();
}

/// Send pasted text to the active pane, wrapping in bracketed-paste
/// sequences (\x1b[200~ … \x1b[201~) when the child has enabled that mode.
/// This is the correct handler for `Event::Paste` (crossterm) and
/// drag-and-drop file paths, ensuring applications like Claude CLI can
/// distinguish paste/drop from typed input.
pub fn send_paste_to_active(app: &mut AppState, text: &str) -> io::Result<()> {
    // In clock mode, any input exits back to passthrough
    if matches!(app.mode, Mode::ClockMode) {
        app.mode = Mode::Passthrough;
        return Ok(());
    }
    // In copy / copy-search modes, treat like regular text
    if matches!(app.mode, Mode::CopyMode) {
        return send_text_to_active(app, text);
    }
    if matches!(app.mode, Mode::CopySearch { .. }) {
        return send_text_to_active(app, text);
    }

    // Check if the child requested bracketed paste mode
    let use_bracket = {
        let win = &app.windows[app.active_idx];
        if let Some(p) = crate::tree::active_pane(&win.root, &win.active_path) {
            if let Ok(parser) = p.term.lock() {
                let bp = parser.screen().bracketed_paste();
                crate::debug_log::input_log("paste", &format!("child bracketed_paste()={}", bp));
                bp
            } else {
                crate::debug_log::input_log("paste", "term lock failed");
                false
            }
        } else {
            crate::debug_log::input_log("paste", "no active pane");
            false
        }
    };
    crate::debug_log::input_log("paste", &format!("use_bracket={} text_len={} text_preview={:?}", use_bracket, text.len(), &text.chars().take(100).collect::<String>()));

    // On Windows, bracketed paste delivery is tricky:
    //
    // - ConPTY may strip \x1b[200~/201~ from the PTY input pipe (older Windows).
    // - WriteConsoleInputW can bypass ConPTY, but it sends each byte of the
    //   bracket sequence as a separate KEY_EVENT record.  Apps that read via
    //   ReadConsoleInputW (crossterm-based apps like Helix) cannot reassemble
    //   VT sequences from individual key events, so \x1b[200~ appears as the
    //   literal characters Esc [ 2 0 0 ~ in the editor (issue #98).
    // - Apps that read raw bytes via ReadFile (nvim via libuv) CAN parse the
    //   bracket sequences from console-injected KEY_EVENTs.
    //
    // Strategy: try the PTY pipe first with bracket markers.  This works on
    // newer Windows where ConPTY passes VT input through, and also works for
    // byte-stream readers (nvim).  If the child uses ReadConsoleInputW
    // (crossterm), ConPTY converts the VT bytes to KEY_EVENTs anyway, so the
    // brackets may still not be parsed -- but at least the text content
    // arrives correctly without stray visible bracket characters.
    //
    // For apps where PTY-pipe brackets get stripped by ConPTY, fall back to
    // console injection for the TEXT ONLY (no bracket markers) so the content
    // still arrives reliably.
    #[cfg(windows)]
    {
        if app.sync_input {
            let win = &mut app.windows[app.active_idx];
            fn write_all_panes(node: &mut crate::types::Node, text: &[u8], bracket: bool) {
                match node {
                    crate::types::Node::Leaf(p) => {
                        write_paste_chunked(&mut p.writer, text, bracket);
                    }
                    crate::types::Node::Split { children, .. } => {
                        for c in children { write_all_panes(c, text, bracket); }
                    }
                }
            }
            write_all_panes(&mut win.root, text.as_bytes(), use_bracket);
        } else {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                write_paste_chunked(&mut p.writer, text.as_bytes(), use_bracket);
            }
        }
    }

    // On non-Windows, use standard PTY pipe write with bracket sequences
    #[cfg(not(windows))]
    {
        if app.sync_input {
            let win = &mut app.windows[app.active_idx];
            fn write_paste_all_panes(node: &mut Node, text: &[u8], bracket: bool) {
                match node {
                    Node::Leaf(p) => {
                        write_paste_chunked(&mut p.writer, text, bracket);
                    }
                    Node::Split { children, .. } => {
                        for c in children { write_paste_all_panes(c, text, bracket); }
                    }
                }
            }
            write_paste_all_panes(&mut win.root, text.as_bytes(), use_bracket);
        } else {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                write_paste_chunked(&mut p.writer, text.as_bytes(), use_bracket);
            }
        }
    }
    Ok(())
}

pub fn send_text_to_active(app: &mut AppState, text: &str) -> io::Result<()> {
    // In clock mode, any input exits back to passthrough
    if matches!(app.mode, Mode::ClockMode) {
        app.mode = Mode::Passthrough;
        return Ok(());
    }
    // Route input to active overlay (so CLI send-keys can interact with overlays)
    if matches!(app.mode, Mode::PopupMode { .. }) {
        // Escape (\x1b alone) closes popup; other text goes to popup PTY
        if text == "\x1b" {
            app.mode = Mode::Passthrough;
            return Ok(());
        }
        if let Mode::PopupMode { ref mut popup_pane, .. } = app.mode {
            if let Some(ref mut pty) = popup_pane {
                let _ = pty.writer.write_all(text.as_bytes());
                let _ = pty.writer.flush();
            }
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::ConfirmMode { .. }) {
        for c in text.chars() {
            match c {
                'y' | 'Y' => {
                    if let Mode::ConfirmMode { ref command, .. } = app.mode {
                        let cmd = command.clone();
                        app.mode = Mode::Passthrough;
                        crate::config::parse_config_line(app, &cmd);
                    }
                    return Ok(());
                }
                'n' | 'N' => {
                    app.mode = Mode::Passthrough;
                    return Ok(());
                }
                _ => {} // Ignore other chars during confirm
            }
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::MenuMode { .. }) {
        // Escape closes menu; other text is ignored (menu is navigated via send-key)
        if text == "\x1b" {
            app.mode = Mode::Passthrough;
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::PaneChooser { .. }) {
        // Escape closes display-panes
        if text == "\x1b" {
            app.mode = Mode::Passthrough;
            return Ok(());
        }
        // In display-panes mode, handle digit selection
        for c in text.chars() {
            if c.is_ascii_digit() {
                let idx = c.to_digit(10).unwrap() as usize;
                if let Some((_, path)) = app.display_map.iter().find(|(d, _)| *d == idx) {
                    let path = path.clone();
                    if let Some(win) = app.windows.get_mut(app.active_idx) {
                        win.active_path = path;
                    }
                }
                app.mode = Mode::Passthrough;
                return Ok(());
            }
        }
        return Ok(());
    }
    // In copy mode, interpret characters as copy-mode actions (never send to PTY)
    if matches!(app.mode, Mode::CopyMode) {
        for c in text.chars() {
            handle_copy_mode_char(app, c)?;
        }
        return Ok(());
    }
    // In copy-search mode, append characters to the search input
    if matches!(app.mode, Mode::CopySearch { .. }) {
        if let Mode::CopySearch { ref mut input, .. } = app.mode {
            for c in text.chars() {
                input.push(c);
            }
        }
        return Ok(());
    }

    if app.sync_input {
        // Fan out to ALL panes in the current window
        let win = &mut app.windows[app.active_idx];
        fn write_all_panes(node: &mut Node, text: &[u8]) {
            match node {
                Node::Leaf(p) => { let _ = p.writer.write_all(text); let _ = p.writer.flush(); }
                Node::Split { children, .. } => { for c in children { write_all_panes(c, text); } }
            }
        }
        write_all_panes(&mut win.root, text.as_bytes());
    } else {
        let win = &mut app.windows[app.active_idx];
        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
            let _ = p.writer.write_all(text.as_bytes());
            let _ = p.writer.flush();
        }
    }
    Ok(())
}

/// Dispatch a single character as a copy-mode action.
pub(crate) fn handle_copy_mode_char(app: &mut AppState, c: char) -> io::Result<()> {
    // Handle text-object pending state (waiting for w/W after a/i)
    if let Some(prefix) = app.copy_text_object_pending.take() {
        match (prefix, c) {
            (0, 'w') => { crate::copy_mode::select_a_word(app); }
            (1, 'w') => { crate::copy_mode::select_inner_word(app); }
            (0, 'W') => { crate::copy_mode::select_a_word_big(app); }
            (1, 'W') => { crate::copy_mode::select_inner_word_big(app); }
            _ => {}
        }
        return Ok(());
    }
    // Handle find-char pending state (waiting for char after f/F/t/T)
    if let Some(pending) = app.copy_find_char_pending.take() {
        match pending {
            0 => crate::copy_mode::find_char_forward(app, c),
            1 => crate::copy_mode::find_char_backward(app, c),
            2 => crate::copy_mode::find_char_to_forward(app, c),
            3 => crate::copy_mode::find_char_to_backward(app, c),
            _ => {}
        }
        return Ok(());
    }
    match c {
        'q' | ']' | '\x1b' => {
            exit_copy_mode(app);
        }
        'h' => { move_copy_cursor(app, -1, 0); }
        'l' => { move_copy_cursor(app, 1, 0); }
        'k' => { move_copy_cursor(app, 0, -1); }
        'j' => { move_copy_cursor(app, 0, 1); }
        'g' => { scroll_to_top(app); }
        'G' => { scroll_to_bottom(app); }
        'w' => { crate::copy_mode::move_word_forward(app); }
        'b' => { crate::copy_mode::move_word_backward(app); }
        'e' => { crate::copy_mode::move_word_end(app); }
        'W' => { crate::copy_mode::move_word_forward_big(app); }
        'B' => { crate::copy_mode::move_word_backward_big(app); }
        'E' => { crate::copy_mode::move_word_end_big(app); }
        'H' => { crate::copy_mode::move_to_screen_top(app); }
        'M' => { crate::copy_mode::move_to_screen_middle(app); }
        'L' => { crate::copy_mode::move_to_screen_bottom(app); }
        'f' => { app.copy_find_char_pending = Some(0); }
        'F' => { app.copy_find_char_pending = Some(1); }
        't' => { app.copy_find_char_pending = Some(2); }
        'T' => { app.copy_find_char_pending = Some(3); }
        'D' => { crate::copy_mode::copy_end_of_line(app)?; exit_copy_mode(app); }
        '0' => { crate::copy_mode::move_to_line_start(app); }
        '$' => { crate::copy_mode::move_to_line_end(app); }
        '^' => { crate::copy_mode::move_to_first_nonblank(app); }
        ' ' => {
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
        }
        'v' => {
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
        }
        'V' => {
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
                app.copy_selection_mode = crate::types::SelectionMode::Line;
            }
        }
        'o' => {
            if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                app.copy_anchor = Some(p);
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some(a);
            }
        }
        'A' => {
            if let (Some(_), Some(_)) = (app.copy_anchor, app.copy_pos) {
                let prev = app.paste_buffers.first().cloned().unwrap_or_default();
                yank_selection(app)?;
                if let Some(buf) = app.paste_buffers.first_mut() {
                    let new_text = buf.clone();
                    *buf = format!("{}{}", prev, new_text);
                }
                exit_copy_mode(app);
            }
        }
        'y' => { yank_selection(app)?; exit_copy_mode(app); }
        '/' => { app.mode = Mode::CopySearch { input: String::new(), forward: true }; }
        '?' => { app.mode = Mode::CopySearch { input: String::new(), forward: false }; }
        'n' => { search_next(app); }
        'N' => { search_prev(app); }
        'i' => { app.copy_text_object_pending = Some(1); }  // inner text object
        'a' => { app.copy_text_object_pending = Some(0); }  // a text object
        _ => {} // Swallow unrecognized characters in copy mode
    }
    Ok(())
}
