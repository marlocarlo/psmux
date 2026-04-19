#[allow(unused_imports)]

use super::*;

impl VtParser {
    /// Dispatch CSI `~` (tilde) sequences: `\x1b[N~` or `\x1b[N;mod~`.
    pub(crate) fn dispatch_tilde<F: FnMut(Event)>(&self, mods: KeyModifiers, emit: &mut F) {
        let n = self.params[0];
        let code = match n {
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            11 => KeyCode::F(1),
            12 => KeyCode::F(2),
            13 => KeyCode::F(3),
            14 => KeyCode::F(4),
            15 => KeyCode::F(5),
            17 => KeyCode::F(6),
            18 => KeyCode::F(7),
            19 => KeyCode::F(8),
            20 => KeyCode::F(9),
            21 => KeyCode::F(10),
            23 => KeyCode::F(11),
            24 => KeyCode::F(12),
            _ => return,
        };
        emit(make_key(code, mods));
    }

    // ── SGR mouse ────────────────────────────────────────────────────────

    /// Decode SGR mouse: `\x1b[<Pb;Px;PyM` (press/drag) or `…m` (release).
    pub(crate) fn dispatch_sgr_mouse<F: FnMut(Event)>(&self, final_ch: char, emit: &mut F) {
        if self.pidx < 3 {
            return;
        }
        let pb = self.params[0];
        let px = self.params[1].saturating_sub(1); // → 0-based column
        let py = self.params[2].saturating_sub(1); // → 0-based row
        let is_release = final_ch == 'm';

        let btn_id    = pb & 0x03;
        let is_shift  = pb & 0x04 != 0;
        let is_alt    = pb & 0x08 != 0;
        let is_ctrl   = pb & 0x10 != 0;
        let is_motion = pb & 0x20 != 0;
        let is_scroll = pb & 0x40 != 0;

        let mut modifiers = KeyModifiers::empty();
        if is_shift { modifiers |= KeyModifiers::SHIFT; }
        if is_alt   { modifiers |= KeyModifiers::ALT; }
        if is_ctrl  { modifiers |= KeyModifiers::CONTROL; }

        let kind = if is_scroll {
            if btn_id == 0 {
                MouseEventKind::ScrollUp
            } else {
                MouseEventKind::ScrollDown
            }
        } else if is_release {
            let button = match btn_id {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::Left,
            };
            MouseEventKind::Up(button)
        } else if is_motion {
            if btn_id == 3 {
                MouseEventKind::Moved
            } else {
                let button = match btn_id {
                    0 => MouseButton::Left,
                    1 => MouseButton::Middle,
                    2 => MouseButton::Right,
                    _ => MouseButton::Left,
                };
                MouseEventKind::Drag(button)
            }
        } else {
            let button = match btn_id {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::Left,
            };
            MouseEventKind::Down(button)
        };

        emit(Event::Mouse(MouseEvent {
            kind,
            column: px,
            row: py,
            modifiers,
        }));
    }

    // ── X10 mouse ────────────────────────────────────────────────────────

    pub(crate) fn on_x10<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        let byte = (ch as u32).min(255) as u8;
        self.x10_buf[self.x10_n as usize] = byte;
        self.x10_n += 1;
        if self.x10_n < 3 {
            return;
        }
        // Got all 3 bytes: button, column+33, row+33.
        self.state = PS::Ground;
        let raw_btn = self.x10_buf[0].wrapping_sub(32);
        let col = self.x10_buf[1].wrapping_sub(33) as u16;
        let row = self.x10_buf[2].wrapping_sub(33) as u16;

        let btn_id    = raw_btn & 0x03;
        let is_motion = raw_btn & 0x20 != 0;
        let is_scroll = raw_btn & 0x40 != 0;

        let mut modifiers = KeyModifiers::empty();
        if raw_btn & 0x04 != 0 { modifiers |= KeyModifiers::SHIFT; }
        if raw_btn & 0x08 != 0 { modifiers |= KeyModifiers::ALT; }
        if raw_btn & 0x10 != 0 { modifiers |= KeyModifiers::CONTROL; }

        let kind = if is_scroll {
            if btn_id == 0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown }
        } else if is_motion {
            match btn_id {
                0 => MouseEventKind::Drag(MouseButton::Left),
                1 => MouseEventKind::Drag(MouseButton::Middle),
                2 => MouseEventKind::Drag(MouseButton::Right),
                _ => MouseEventKind::Moved,
            }
        } else if btn_id == 3 {
            // X10 "release" encoding.
            MouseEventKind::Up(MouseButton::Left)
        } else {
            let button = match btn_id {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::Left,
            };
            MouseEventKind::Down(button)
        };

        emit(Event::Mouse(MouseEvent { kind, column: col, row: row, modifiers }));
    }

    // ── SS3 (\x1bO) ─────────────────────────────────────────────────────

    pub(crate) fn on_ss3<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        self.state = PS::Ground;
        match ch {
            'A' => emit(make_key(KeyCode::Up, KeyModifiers::empty())),
            'B' => emit(make_key(KeyCode::Down, KeyModifiers::empty())),
            'C' => emit(make_key(KeyCode::Right, KeyModifiers::empty())),
            'D' => emit(make_key(KeyCode::Left, KeyModifiers::empty())),
            'H' => emit(make_key(KeyCode::Home, KeyModifiers::empty())),
            'F' => emit(make_key(KeyCode::End, KeyModifiers::empty())),
            'P' => emit(make_key(KeyCode::F(1), KeyModifiers::empty())),
            'Q' => emit(make_key(KeyCode::F(2), KeyModifiers::empty())),
            'R' => emit(make_key(KeyCode::F(3), KeyModifiers::empty())),
            'S' => emit(make_key(KeyCode::F(4), KeyModifiers::empty())),
            _ => {
                // Unknown SS3 — emit Alt+char as fallback.
                emit(make_key(KeyCode::Char(ch), KeyModifiers::ALT));
            }
        }
    }

    // ── Bracketed paste (\x1b[200~ … \x1b[201~) ─────────────────────────

    pub(crate) fn on_paste<F: FnMut(Event)>(&mut self, ch: char, _emit: &mut F) {
        if ch == '\x1b' {
            self.state = PS::PasteEsc;
        } else if self.paste.len() < Self::PASTE_MAX_BYTES {
            self.paste.push(ch);
        }
    }

    pub(crate) fn on_paste_esc<F: FnMut(Event)>(&mut self, ch: char, _emit: &mut F) {
        if ch == '[' {
            self.state = PS::PasteBrk;
        } else {
            self.paste.push('\x1b');
            self.paste.push(ch);
            self.state = PS::Paste;
        }
    }

    pub(crate) fn on_paste_brk<F: FnMut(Event)>(&mut self, ch: char, _emit: &mut F) {
        if ch.is_ascii_digit() {
            self.cur = (ch as u16) - (b'0' as u16);
            self.state = PS::PasteNum;
        } else {
            self.paste.push('\x1b');
            self.paste.push('[');
            self.paste.push(ch);
            self.state = PS::Paste;
        }
    }

    pub(crate) fn on_paste_num<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        if ch.is_ascii_digit() {
            self.cur = self.cur.saturating_mul(10).saturating_add((ch as u16) - (b'0' as u16));
        } else if ch == '~' && self.cur == 201 {
            // \x1b[201~ — paste end.
            let text = std::mem::take(&mut self.paste);
            self.paste_start = None;
            emit(Event::Paste(text));
            self.state = PS::Ground;
        } else {
            // Not the end marker — push partial escape into paste buffer.
            self.paste.push('\x1b');
            self.paste.push('[');
            let s = self.cur.to_string();
            self.paste.push_str(&s);
            self.paste.push(ch);
            self.cur = 0;
            self.state = PS::Paste;
        }
    }

    /// Post-paste-flush drain: absorbs residual close-sequence characters
    /// (`~`, `[`, digits, ESC) that may arrive after a paste timeout flush.
    /// ConPTY can strip the CSI prefix of `\x1b[201~` and leak only the
    /// final `~`, which would otherwise appear as a visible character.
    pub(crate) fn on_paste_drain<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '~' | '[' | '0'..='9' => {
                // Likely residue from a stripped close sequence — absorb.
                ssh_debug_log(&format!("PasteDrain: absorbing residue char {:?}", ch));
            }
            '\x1b' => {
                // ESC could start a new close sequence that ConPTY partially
                // passed through.  Transition to Escape to let the CSI
                // parser handle it (dispatch_tilde ignores param 201).
                self.paste_start = None;
                self.state = PS::Escape;
            }
            _ => {
                // Non-residue character: drain is done, process normally.
                self.paste_start = None;
                self.state = PS::Ground;
                self.on_ground(ch, emit);
            }
        }
    }

    // ── OSC (Operating System Command) ───────────────────────────────────
    //
    // Accumulates \x1b] ... ST where ST is \x07 (BEL) or \x1b\\.
    // Used to parse OSC 52 clipboard responses from the client terminal.

    pub(crate) fn on_osc<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '\x07' => {
                // ST (BEL) — dispatch OSC
                self.dispatch_osc(emit);
                self.state = PS::Ground;
            }
            '\x1b' => {
                // Possible start of ST (\x1b\\)
                self.state = PS::OscEsc;
            }
            c => {
                // Safety limit: 128 KB
                if self.osc.len() < 131072 {
                    self.osc.push(c);
                }
            }
        }
    }

    pub(crate) fn on_osc_esc<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        if ch == '\\' {
            // ST (\x1b\\) — dispatch OSC
            self.dispatch_osc(emit);
            self.state = PS::Ground;
        } else {
            // Not ST — abort OSC, re-process as new escape sequence
            self.osc.clear();
            self.state = PS::Escape;
            self.on_escape(ch, emit);
        }
    }

    pub(crate) fn dispatch_osc<F: FnMut(Event)>(&self, emit: &mut F) {
        // OSC 52 clipboard response: "52;<selection>;<base64data>"
        if let Some(rest) = self.osc.strip_prefix("52;") {
            if let Some(sc_idx) = rest.find(';') {
                let data = &rest[sc_idx + 1..];
                // Ignore queries ("?") and empty responses
                if data != "?" && !data.is_empty() {
                    if let Some(text) = crate::util::base64_decode(data) {
                        if !text.is_empty() {
                            emit(Event::Paste(text));
                        }
                    }
                }
            }
        }
        // All other OSC sequences silently discarded
    }
}
