use super::*;
use super::run_remote_state::RunRemoteState;

/// Handle key events when a server-side overlay is active (popup, confirm,
/// menu, display-panes, customize-mode).
///
/// Returns `true` if the key was consumed by an overlay, `false` otherwise.
pub(crate) fn handle_server_overlay_key(
    state: &mut RunRemoteState,
    key: &crossterm::event::KeyEvent,
    cmd_batch: &mut Vec<String>,
) -> bool {
    if state.srv_popup_active {
        if state.srv_popup_has_pty {
            // PTY popup: forward all keys to server
            match key.code {
                KeyCode::Esc => { cmd_batch.push("overlay-close\n".into()); }
                KeyCode::Char(c) => {
                    let bytes = if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                        vec![(c as u8) & 0x1F]
                    } else {
                        let mut buf = [0u8; 4];
                        let s = c.encode_utf8(&mut buf);
                        s.as_bytes().to_vec()
                    };
                    let encoded = crate::util::base64_encode(std::str::from_utf8(&bytes).unwrap_or(""));
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Enter => {
                    let encoded = crate::util::base64_encode("\r");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Backspace => {
                    let encoded = crate::util::base64_encode("\x7f");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Tab => {
                    let encoded = crate::util::base64_encode("\t");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Up => {
                    let encoded = crate::util::base64_encode("\x1b[A");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Down => {
                    let encoded = crate::util::base64_encode("\x1b[B");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Right => {
                    let encoded = crate::util::base64_encode("\x1b[C");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Left => {
                    let encoded = crate::util::base64_encode("\x1b[D");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Home => {
                    let encoded = crate::util::base64_encode("\x1b[H");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::End => {
                    let encoded = crate::util::base64_encode("\x1b[F");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::PageUp => {
                    let encoded = crate::util::base64_encode("\x1b[5~");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::PageDown => {
                    let encoded = crate::util::base64_encode("\x1b[6~");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                KeyCode::Delete => {
                    let encoded = crate::util::base64_encode("\x1b[3~");
                    cmd_batch.push(format!("popup-input {}\n", encoded));
                }
                _ => {}
            }
        } else {
            // Static (non-PTY) popup: handle scroll locally, q/Esc close
            let total_lines = state.srv_popup_lines.len() as u16;
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    cmd_batch.push("overlay-close\n".into());
                    state.srv_popup_scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    state.srv_popup_scroll = state.srv_popup_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if state.srv_popup_scroll < total_lines.saturating_sub(1) {
                        state.srv_popup_scroll += 1;
                    }
                }
                KeyCode::PageUp => {
                    state.srv_popup_scroll = state.srv_popup_scroll.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    state.srv_popup_scroll = (state.srv_popup_scroll + 10).min(total_lines.saturating_sub(1));
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    state.srv_popup_scroll = 0;
                }
                KeyCode::End | KeyCode::Char('G') => {
                    state.srv_popup_scroll = total_lines.saturating_sub(1);
                }
                _ => {}
            }
        }
        return true;
    }

    if state.srv_confirm_active {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                cmd_batch.push("confirm-respond y\n".into());
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                cmd_batch.push("confirm-respond n\n".into());
            }
            _ => {} // Ignore other keys during confirm
        }
        return true;
    }

    if state.srv_menu_active {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => { cmd_batch.push("menu-navigate -1\n".into()); }
            KeyCode::Down | KeyCode::Char('j') => { cmd_batch.push("menu-navigate 1\n".into()); }
            KeyCode::Enter => {
                cmd_batch.push(format!("menu-select {}\n", state.srv_menu_selected));
            }
            KeyCode::Esc | KeyCode::Char('q') => { cmd_batch.push("overlay-close\n".into()); }
            KeyCode::Char(c) => {
                // Shortcut key: find menu item with matching key
                if let Some(idx) = state.srv_menu_items.iter().position(|item| {
                    item.key.as_ref().map(|k| k.len() == 1 && k.chars().next() == Some(c)).unwrap_or(false)
                }) {
                    cmd_batch.push(format!("menu-select {}\n", idx));
                }
            }
            _ => {}
        }
        return true;
    }

    if state.srv_display_panes {
        match key.code {
            KeyCode::Char(d) if d.is_ascii_digit() => {
                let digit = d.to_digit(10).unwrap() as usize;
                cmd_batch.push(format!("display-panes-select {}\n", digit));
            }
            _ => { cmd_batch.push("overlay-close\n".into()); }
        }
        return true;
    }

    if state.srv_customize_active {
        if state.srv_customize_editing {
            match key.code {
                KeyCode::Esc => { cmd_batch.push("customize-edit-cancel\n".into()); }
                KeyCode::Enter => { cmd_batch.push("customize-edit-confirm\n".into()); }
                KeyCode::Backspace => {
                    if state.srv_customize_cursor > 0 {
                        let mut buf = state.srv_customize_edit_buf.clone();
                        buf.remove(state.srv_customize_cursor - 1);
                        cmd_batch.push(format!("customize-edit-update {}\n", buf));
                    }
                }
                KeyCode::Char(c) => {
                    let mut buf = state.srv_customize_edit_buf.clone();
                    buf.insert(state.srv_customize_cursor, c);
                    cmd_batch.push(format!("customize-edit-update {}\n", buf));
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => { cmd_batch.push("overlay-close\n".into()); }
                KeyCode::Up | KeyCode::Char('k') => { cmd_batch.push("customize-navigate -1\n".into()); }
                KeyCode::Down | KeyCode::Char('j') => { cmd_batch.push("customize-navigate 1\n".into()); }
                KeyCode::PageUp => { cmd_batch.push("customize-navigate -20\n".into()); }
                KeyCode::PageDown => { cmd_batch.push("customize-navigate 20\n".into()); }
                KeyCode::Home | KeyCode::Char('g') => { cmd_batch.push("customize-navigate -9999\n".into()); }
                KeyCode::End | KeyCode::Char('G') => { cmd_batch.push("customize-navigate 9999\n".into()); }
                KeyCode::Enter => { cmd_batch.push("customize-edit\n".into()); }
                KeyCode::Char('d') => { cmd_batch.push("customize-reset-default\n".into()); }
                KeyCode::Char('/') => {
                    if !state.srv_customize_filter.is_empty() {
                        cmd_batch.push("customize-filter \n".into());
                    }
                }
                _ => {}
            }
        }
        return true;
    }

    false
}
