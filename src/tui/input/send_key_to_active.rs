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

pub fn send_key_to_active(app: &mut AppState, k: &str) -> io::Result<()> {
    // In clock mode, any key exits back to passthrough
    if matches!(app.mode, Mode::ClockMode) {
        app.mode = Mode::Passthrough;
        return Ok(());
    }
    // Route named keys to active overlay (so CLI send-keys can interact with overlays)
    if matches!(app.mode, Mode::PopupMode { .. }) {
        // Map named keys to VT sequences for the popup PTY
        let seq = match k {
            "enter" => Some("\r"),
            "esc" | "escape" => {
                app.mode = Mode::Passthrough;
                return Ok(());
            }
            "tab" => Some("\t"),
            "backspace" | "bspace" => Some("\x7f"),
            "up" => Some("\x1b[A"),
            "down" => Some("\x1b[B"),
            "right" => Some("\x1b[C"),
            "left" => Some("\x1b[D"),
            "home" => Some("\x1b[H"),
            "end" => Some("\x1b[F"),
            "pageup" | "ppage" => Some("\x1b[5~"),
            "pagedown" | "npage" => Some("\x1b[6~"),
            "delete" | "dc" => Some("\x1b[3~"),
            "space" => Some(" "),
            _ => None,
        };
        if let Some(seq) = seq {
            if let Mode::PopupMode { ref mut popup_pane, .. } = app.mode {
                if let Some(ref mut pty) = popup_pane {
                    let _ = pty.writer.write_all(seq.as_bytes());
                    let _ = pty.writer.flush();
                }
            }
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::ConfirmMode { .. }) {
        match k {
            "esc" | "escape" => {
                app.mode = Mode::Passthrough;
            }
            _ => {} // y/n handled via send_text_to_active
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::MenuMode { .. }) {
        match k {
            "up" => {
                if let Mode::MenuMode { ref mut menu } = app.mode {
                    if menu.selected > 0 { menu.selected -= 1; }
                }
            }
            "down" => {
                if let Mode::MenuMode { ref mut menu } = app.mode {
                    let len = menu.items.len();
                    if menu.selected + 1 < len { menu.selected += 1; }
                }
            }
            "enter" => {
                if let Mode::MenuMode { ref menu } = app.mode {
                    if let Some(item) = menu.items.get(menu.selected) {
                        if !item.is_separator && !item.command.is_empty() {
                            let cmd = item.command.clone();
                            app.mode = Mode::Passthrough;
                            crate::config::parse_config_line(app, &cmd);
                            return Ok(());
                        }
                    }
                }
                app.mode = Mode::Passthrough;
            }
            "esc" | "escape" | "q" => {
                app.mode = Mode::Passthrough;
            }
            _ => {}
        }
        return Ok(());
    }
    if matches!(app.mode, Mode::PaneChooser { .. }) {
        match k {
            "esc" | "escape" => {
                app.mode = Mode::Passthrough;
            }
            _ => {}
        }
        return Ok(());
    }
    // --- Copy-search mode: handle esc/enter/backspace ---
    if matches!(app.mode, Mode::CopySearch { .. }) {
        match k {
            "esc" => { app.mode = Mode::CopyMode; }
            "enter" => {
                if let Mode::CopySearch { ref input, forward } = app.mode {
                    let query = input.clone();
                    let fwd = forward;
                    app.copy_search_query = query.clone();
                    app.copy_search_forward = fwd;
                    search_copy_mode(app, &query, fwd);
                    if !app.copy_search_matches.is_empty() {
                        let (r, c, _) = app.copy_search_matches[0];
                        app.copy_pos = Some((r, c));
                    }
                }
                app.mode = Mode::CopyMode;
            }
            "backspace" => {
                if let Mode::CopySearch { ref mut input, .. } = app.mode { input.pop(); }
            }
            _ => {}
        }
        return Ok(());
    }

    // --- Copy mode: full vi-style key table ---
    if matches!(app.mode, Mode::CopyMode) {
        match k {
            "esc" | "q" => {
                exit_copy_mode(app);
            }
            "enter" => {
                // Copy selection and exit copy mode (vi Enter)
                if app.copy_anchor.is_some() {
                    yank_selection(app)?;
                }
                exit_copy_mode(app);
            }
            "space" => {
                // Begin selection (like v in vi mode)
                if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                    app.copy_anchor = Some((r, c));
                    app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                    app.copy_pos = Some((r, c));
                    app.copy_selection_mode = crate::types::SelectionMode::Char;
                }
            }
            "up" => { move_copy_cursor(app, 0, -1); }
            "down" => { move_copy_cursor(app, 0, 1); }
            "pageup" => { scroll_copy_up(app, 10); }
            "pagedown" => { scroll_copy_down(app, 10); }
            "left" => { move_copy_cursor(app, -1, 0); }
            "right" => { move_copy_cursor(app, 1, 0); }
            "home" => { crate::copy_mode::move_to_line_start(app); }
            "end" => { crate::copy_mode::move_to_line_end(app); }
            "C-b" | "c-b" => {
                if app.mode_keys == "emacs" { move_copy_cursor(app, -1, 0); }
                else { scroll_copy_up(app, 10); }
            }
            "C-f" | "c-f" => {
                if app.mode_keys == "emacs" { move_copy_cursor(app, 1, 0); }
                else { scroll_copy_down(app, 10); }
            }
            "C-n" | "c-n" => { move_copy_cursor(app, 0, 1); }
            "C-p" | "c-p" => { move_copy_cursor(app, 0, -1); }
            "C-a" | "c-a" => { crate::copy_mode::move_to_line_start(app); }
            "C-e" | "c-e" => { crate::copy_mode::move_to_line_end(app); }
            "C-v" | "c-v" => { scroll_copy_down(app, 10); }
            "M-v" | "m-v" => { scroll_copy_up(app, 10); }
            "M-f" | "m-f" => { crate::copy_mode::move_word_forward(app); }
            "M-b" | "m-b" => { crate::copy_mode::move_word_backward(app); }
            "M-w" | "m-w" => { yank_selection(app)?; exit_copy_mode(app); }
            "C-s" | "c-s" => { app.mode = Mode::CopySearch { input: String::new(), forward: true }; }
            "C-r" | "c-r" => { app.mode = Mode::CopySearch { input: String::new(), forward: false }; }
            "C-c" | "c-c" => {
                exit_copy_mode(app);
            }
            "C-g" | "c-g" => {
                exit_copy_mode(app);
            }
            "c-space" | "C-space" => {
                // Set mark (anchor) at current position
                if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                    app.copy_anchor = Some((r, c));
                    app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                    app.copy_pos = Some((r, c));
                }
            }
            "C-u" | "c-u" => {
                let half = app.windows.get(app.active_idx)
                    .and_then(|w| active_pane(&w.root, &w.active_path))
                    .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                scroll_copy_up(app, half);
            }
            "C-d" | "c-d" => {
                let half = app.windows.get(app.active_idx)
                    .and_then(|w| active_pane(&w.root, &w.active_path))
                    .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
                scroll_copy_down(app, half);
            }
            _ => {}
        }
        return Ok(());
    }
    
    let win = &mut app.windows[app.active_idx];
    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
        match k {
            "enter" => { let _ = write!(p.writer, "\r"); }
            "tab" => { let _ = write!(p.writer, "\t"); }
            "btab" | "backtab" => { let _ = write!(p.writer, "\x1b[Z"); }
            "backspace" => { let _ = p.writer.write_all(&[0x7F]); }
            "delete" => { let _ = write!(p.writer, "\x1b[3~"); }
            "esc" => { let _ = write!(p.writer, "\x1b"); }
            "left" => { let _ = write!(p.writer, "\x1b[D"); }
            "right" => { let _ = write!(p.writer, "\x1b[C"); }
            "up" => { let _ = write!(p.writer, "\x1b[A"); }
            "down" => { let _ = write!(p.writer, "\x1b[B"); }
            "pageup" => { let _ = write!(p.writer, "\x1b[5~"); }
            "pagedown" => { let _ = write!(p.writer, "\x1b[6~"); }
            "home" => { let _ = write!(p.writer, "\x1b[H"); }
            "end" => { let _ = write!(p.writer, "\x1b[F"); }
            "insert" => { let _ = write!(p.writer, "\x1b[2~"); }
            "space" => { let _ = write!(p.writer, " "); }
            s if s.starts_with("f") && s.len() >= 2 && s.len() <= 3 => {
                if let Ok(n) = s[1..].parse::<u8>() {
                    let seq = match n {
                        1 => "\x1bOP",
                        2 => "\x1bOQ",
                        3 => "\x1bOR",
                        4 => "\x1bOS",
                        5 => "\x1b[15~",
                        6 => "\x1b[17~",
                        7 => "\x1b[18~",
                        8 => "\x1b[19~",
                        9 => "\x1b[20~",
                        10 => "\x1b[21~",
                        11 => "\x1b[23~",
                        12 => "\x1b[24~",
                        _ => "",
                    };
                    if !seq.is_empty() { let _ = write!(p.writer, "{}", seq); }
                }
            }
            s if s.starts_with("C-") && s.len() == 3 => {
                let c = s.chars().nth(2).unwrap_or('c');
                let ctrl_char = (c.to_ascii_lowercase() as u8) & 0x1F;
                let _ = p.writer.write_all(&[ctrl_char]);

            }
            s if (s.starts_with("M-") || s.starts_with("m-")) && s.len() == 3 => {
                let c = s.chars().nth(2).unwrap_or('a');
                // Try native console injection (WriteConsoleInputW with LEFT_ALT_PRESSED)
                // first.  ConPTY does NOT reassemble ESC+char into Alt+key events, so
                // PSReadLine Alt+f/Alt+b/etc. won't work via the VT path.
                let injected = if let Some(pid) = p.child_pid {
                    crate::platform::mouse_inject::send_alt_key_event(pid, c)
                } else {
                    false
                };
                if !injected {
                    // Fallback: VT encoding (ESC + char) — works for VT-native apps
                    let _ = write!(p.writer, "\x1b{}", c);
                }
            }
            s if (s.starts_with("C-M-") || s.starts_with("c-m-")) && s.len() == 5 => {
                let c = s.chars().nth(4).unwrap_or('c');
                // Try native console injection (WriteConsoleInputW with
                // LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED).  ConPTY does NOT
                // reassemble ESC + ctrl-char into Ctrl+Alt+key.
                let injected = if let Some(pid) = p.child_pid {
                    crate::platform::send_modified_key_event(pid, c, true, true, false)
                } else {
                    false
                };
                if !injected {
                    let ctrl_char = (c.to_ascii_lowercase() as u8) & 0x1F;
                    let _ = p.writer.write_all(&[0x1b, ctrl_char]);
                }
            }
            // Modified Enter: for Ctrl combos, try native console injection
            // (WriteConsoleInputW) so PSReadLine sees the correct modifier flags.
            // For Shift/Alt-only combos, use VT encoding to avoid ConPTY
            // translating the injected KEY_EVENT back to plain \r (double Enter).
            #[cfg(windows)]
            s if {
                let u = s.to_uppercase();
                let r = u.trim_start_matches("C-").trim_start_matches("M-").trim_start_matches("S-");
                r == "ENTER" || r == "RETURN" || r == "CR"
            } => {
                let upper = s.to_uppercase();
                let has_shift = upper.contains("S-");
                let has_ctrl = upper.contains("C-");
                let has_alt = upper.contains("M-");
                let injected = if has_ctrl {
                    // Only use native injection for Ctrl combos.
                    if let Some(pid) = p.child_pid {
                        crate::platform::send_modified_enter_event(pid, has_ctrl, has_alt, has_shift)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !injected {
                    if (has_shift || has_alt) && !has_ctrl {
                        // Fallback: ESC + CR for VT-native apps (Claude Code, etc.)
                        let _ = p.writer.write_all(b"\x1b\r");
                    } else {
                        // Ctrl+Enter and other combos: CSI encoding
                        if let Some(seq) = parse_modified_special_key(s) {
                            let _ = p.writer.write_all(seq.as_bytes());
                        }
                    }
                }
            }
            // Modifier + special key combos: C-Left, S-Right, C-S-Up, C-M-Home, etc.
            s if parse_modified_special_key(s).is_some() => {
                let seq = parse_modified_special_key(s).unwrap();
                let _ = p.writer.write_all(seq.as_bytes());
            }
            _ => {}
        }
        let _ = p.writer.flush();
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests-rs/test_input.rs"]
mod tests;
