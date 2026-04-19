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

pub fn encode_key_event(key: &KeyEvent) -> Option<Vec<u8>> {
    let encoded: Vec<u8> = match key.code {
        // AltGr detection: On Windows, AltGr is reported as Ctrl+Alt by the
        // console subsystem / crossterm.  International keyboards (German,
        // Czech, Polish, …) use AltGr to produce characters like \ @ { } [ ]
        // | ~ €.  crossterm delivers these as KeyCode::Char(produced_char)
        // with CONTROL|ALT modifiers.  The produced character is NOT an ASCII
        // letter (a-z), so we can distinguish AltGr from genuine Ctrl+Alt
        // combos and forward the character as-is.
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::ALT)
            && !c.is_ascii_lowercase() => {
            // AltGr-produced character — forward it verbatim (UTF-8).
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf);
            buf[..c.len_utf8()].to_vec()
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::ALT) => {
            // Genuine Ctrl+Alt+letter — encode as ESC + ctrl-char.
            let ctrl_char = (c as u8) & 0x1F;
            vec![0x1b, ctrl_char]
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
            format!("\x1b{}", c).into_bytes()
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let ctrl_char = (c as u8) & 0x1F;
            vec![ctrl_char]
        }
        KeyCode::Char(c) if (c as u32) >= 0x01 && (c as u32) <= 0x1A => {
            vec![c as u8]
        }
        KeyCode::Char(c) => {
            format!("{}", c).into_bytes()
        }
        KeyCode::Enter => {
            let m = modifier_param(key.modifiers);
            if m > 1 {
                // On Windows, CSI 13;mod~ is non-standard and dropped by ConPTY.
                // Send ESC+CR (\x1b\r) for Shift/Alt+Enter — the same bytes VS Code's
                // xterm.js sends.  libuv preserves ESC as Alt prefix, so Node.js apps
                // (Claude Code) receive \x1b\r and interpret it as Shift+Enter.
                // Ctrl+Enter and Ctrl+Shift+Enter still use CSI encoding (those are
                // less common and consumed by other layers).
                #[cfg(windows)]
                {
                    let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    if !has_ctrl {
                        return Some(b"\x1b\r".to_vec());
                    }
                }
                // Non-Windows or Ctrl combos: xterm modified-Enter: CSI 13 ; mod ~
                format!("\x1b[13;{}~", m).into_bytes()
            } else {
                b"\r".to_vec()
            }
        }
        KeyCode::Tab => {
            let m = modifier_param(key.modifiers);
            if m > 1 {
                // xterm modified-Tab: CSI 9 ; mod ~
                format!("\x1b[9;{}~", m).into_bytes()
            } else {
                b"\t".to_vec()
            }
        }
        KeyCode::BackTab => {
            let m = modifier_param(key.modifiers);
            if m > 1 {
                // Shift is implicit in BackTab; add it back for the modifier param
                let sm = m | 1; // ensure Shift bit is set
                format!("\x1b[9;{}~", sm).into_bytes()
            } else {
                b"\x1b[Z".to_vec()
            }
        }
        KeyCode::Backspace => b"\x08".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        // Arrow keys and special keys with xterm modifier encoding.
        // Format: \x1b[1;{mod}{letter} where mod = 1 + Shift*1 + Alt*2 + Ctrl*4
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down |
        KeyCode::Home | KeyCode::End => {
            let letter = match key.code {
                KeyCode::Up => 'A', KeyCode::Down => 'B',
                KeyCode::Right => 'C', KeyCode::Left => 'D',
                KeyCode::Home => 'H', KeyCode::End => 'F',
                _ => unreachable!(),
            };
            let m = modifier_param(key.modifiers);
            if m > 1 {
                format!("\x1b[1;{}{}", m, letter).into_bytes()
            } else {
                format!("\x1b[{}", letter).into_bytes()
            }
        }
        // Tilde-style keys: \x1b[{N};{mod}~ when modifiers present
        KeyCode::Insert | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match key.code {
                KeyCode::Insert => 2, KeyCode::Delete => 3,
                KeyCode::PageUp => 5, KeyCode::PageDown => 6,
                _ => unreachable!(),
            };
            let m = modifier_param(key.modifiers);
            if m > 1 {
                format!("\x1b[{};{}~", n, m).into_bytes()
            } else {
                format!("\x1b[{}~", n).into_bytes()
            }
        }
        KeyCode::F(n) => {
            let m = modifier_param(key.modifiers);
            encode_fkey(n, m)
        }
        _ => return None,
    };
    Some(encoded)
}

pub fn forward_key_to_active(app: &mut AppState, key: KeyEvent) -> io::Result<()> {
    // On Windows, modified Enter delivery depends on the modifier:
    //
    // Shift/Alt+Enter (no Ctrl): Use VT encoding ONLY (\x1b\r).  Native
    //   WriteConsoleInputW injection would cause ConPTY to translate the
    //   KEY_EVENT back to plain \r, so VT-native apps (Claude Code) see a
    //   double Enter.
    //
    // Ctrl+Enter / Ctrl+Shift+Enter: Use native injection ONLY.  ConPTY
    //   cannot encode Ctrl+Enter in VT, so injection is the only reliable
    //   path for console apps (PSReadLine).  Falls back to xterm CSI
    //   encoding (\x1b[13;N~) if injection fails (for non-console apps).
    #[cfg(windows)]
    {
        if matches!(key.code, KeyCode::Enter) && !key.modifiers.is_empty() {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);

            // Only use native injection when Ctrl is involved.
            if ctrl {
                let try_inject = |pane: &mut Pane| -> bool {
                    if let Some(pid) = pane.child_pid {
                        crate::platform::send_modified_enter_event(pid, ctrl, alt, shift)
                    } else {
                        false
                    }
                };

                if app.sync_input {
                    let win = &mut app.windows[app.active_idx];
                    fn inject_all(node: &mut Node, ctrl: bool, alt: bool, shift: bool) {
                        match node {
                            Node::Leaf(p) if !p.dead => {
                                if let Some(pid) = p.child_pid {
                                    if !crate::platform::send_modified_enter_event(pid, ctrl, alt, shift) {
                                        // Fallback: xterm CSI encoding for non-console apps
                                        let m: u8 = 1 + (shift as u8) + (alt as u8) * 2 + (ctrl as u8) * 4;
                                        let bytes = if m > 1 { format!("\x1b[13;{}~", m).into_bytes() } else { b"\r".to_vec() };
                                        let _ = p.writer.write_all(&bytes);
                                        let _ = p.writer.flush();
                                    }
                                }
                            }
                            Node::Leaf(_) => {}
                            Node::Split { children, .. } => {
                                for c in children { inject_all(c, ctrl, alt, shift); }
                            }
                        }
                    }
                    inject_all(&mut win.root, ctrl, alt, shift);
                    return Ok(());
                } else {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if !active.dead {
                            if try_inject(active) {
                                return Ok(());
                            }
                            // Fallback: VT encoding (falls through below)
                        }
                    }
                }
            }
            // Shift/Alt+Enter (no Ctrl): fall through to VT encoding below.
        }
    }

    let encoded = match encode_key_event(&key) {
        Some(bytes) => bytes,
        None => return Ok(()),
    };

    if app.sync_input {
        // Fan out to ALL panes in the current window
        let win = &mut app.windows[app.active_idx];
        fn write_all_panes(node: &mut Node, data: &[u8]) {
            match node {
                Node::Leaf(p) if !p.dead => { let _ = p.writer.write_all(data); let _ = p.writer.flush(); }
                Node::Leaf(_) => {}
                Node::Split { children, .. } => { for c in children { write_all_panes(c, data); } }
            }
        }
        write_all_panes(&mut win.root, &encoded);

    } else {
        let win = &mut app.windows[app.active_idx];
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if !active.dead {
                let _ = active.writer.write_all(&encoded);
                let _ = active.writer.flush();

            }
        }
    }
    Ok(())
}

pub(crate) fn wheel_cell_for_area(area: Rect, x: u16, y: u16) -> (u16, u16) {
    // Convert global terminal coordinates to 1-based pane-local coordinates.
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_w = area.width.saturating_sub(2).max(1);
    let inner_h = area.height.saturating_sub(2).max(1);

    let col = x
        .saturating_sub(inner_x)
        .min(inner_w.saturating_sub(1))
        .saturating_add(1);
    let row = y
        .saturating_sub(inner_y)
        .min(inner_h.saturating_sub(1))
        .saturating_add(1);
    (col, row)
}

/// Paste the system clipboard content into the active pane.
/// This is the Windows Terminal right-click-to-paste behavior.
pub(crate) fn paste_clipboard_to_active(app: &mut AppState) -> io::Result<()> {
    if let Some(text) = crate::copy_mode::read_from_system_clipboard() {
        if !text.is_empty() {
            send_paste_to_active(app, &text)?;
        }
    }
    Ok(())
}

/// Forward a mouse event to the child pane.
///
/// If the child has mouse protocol enabled (TUI app running), write VT mouse
/// sequences directly to the ConPTY input pipe (pane.writer).  Modern TUI
/// apps (crossterm, etc.) use VT input mode (ReadFile + ENABLE_VIRTUAL_TERMINAL_INPUT)
/// and receive these directly through stdin.  If VT input mode is off, ConPTY
/// parses the VT and converts to MOUSE_EVENT records for ReadConsoleInputW apps.
///
/// When mouse protocol is NOT enabled (shell prompt), use Win32 MOUSE_EVENT
/// injection as a harmless fallback (most programs ignore it).
pub(crate) fn forward_mouse_to_pane(pane: &mut Pane, area: Rect, abs_x: u16, abs_y: u16, button_state: u32, event_flags: u32) {
    forward_mouse_to_pane_ex(pane, area, abs_x, abs_y, button_state, event_flags, 0xff, false);
}

/// Forward a mouse event to a child pane by writing SGR mouse sequences
/// to the ConPTY input pipe — the same mechanism Windows Terminal uses.
///
/// ConPTY/conhost automatically translates SGR mouse sequences into
/// MOUSE_EVENT records for crossterm/ratatui apps (ReadConsoleInputW),
/// and passes VT through for nvim/vim apps.  (fixes #60)
pub(crate) fn forward_mouse_to_pane_ex(pane: &mut Pane, area: Rect, abs_x: u16, abs_y: u16,
                             _button_state: u32, _event_flags: u32,
                             vt_button: u8, press: bool) {
    let col = abs_x as i16 - area.x as i16;
    let row = abs_y as i16 - area.y as i16;
    crate::window_ops::inject_mouse_combined(
        pane, col, row, vt_button, press, 0, 0, "client");
}
