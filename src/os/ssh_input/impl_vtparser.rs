#[allow(unused_imports)]
use std::io;
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
/// Explicitly (re-)send the VT mouse-enable escape sequences to stdout.
///
/// Over SSH, ConPTY may consume DECSET 1000/1002/1003/1006 from the output
/// stream and NOT forward them to sshd.  This tries several approaches:
///  1. `WriteFile` on the raw console output handle (may bypass ConPTY VT
///     processing in some Windows builds).
///  2. A regular `write_all` to stdout (belt-and-suspenders).
///
/// Call this **after** crossterm's `EnableMouseCapture` and `InputSource::new`.
#[cfg(windows)]
use super::*;

impl VtParser {
    pub(crate) fn new() -> Self {
        Self {
            state: PS::Ground,
            params: [0; 8],
            pidx: 0,
            cur: 0,
            has_digit: false,
            priv_ch: 0,
            x10_n: 0,
            x10_buf: [0; 3],
            paste: String::new(),
            paste_start: None,
            needs_vti_recheck: false,
            osc: String::new(),
            hi_sur: None,
        }
    }

    #[inline(always)]
    pub(crate) fn reset_csi(&mut self) {
        self.params = [0; 8];
        self.pidx = 0;
        self.cur = 0;
        self.has_digit = false;
        self.priv_ch = 0;
    }

    /// Feed one Unicode character into the parser, emitting events via `emit`.
    #[inline]
    pub(crate) fn feed<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match self.state {
            PS::Ground   => self.on_ground(ch, emit),
            PS::Escape   => self.on_escape(ch, emit),
            PS::CsiEntry => self.on_csi_entry(ch, emit),
            PS::CsiParam => self.on_csi_param(ch, emit),
            PS::X10Mouse => self.on_x10(ch, emit),
            PS::Ss3      => self.on_ss3(ch, emit),
            PS::Paste    => self.on_paste(ch, emit),
            PS::PasteEsc => self.on_paste_esc(ch, emit),
            PS::PasteBrk => self.on_paste_brk(ch, emit),
            PS::PasteNum => self.on_paste_num(ch, emit),
            PS::PasteDrain => self.on_paste_drain(ch, emit),
            PS::Osc      => self.on_osc(ch, emit),
            PS::OscEsc   => self.on_osc_esc(ch, emit),
        }
    }

    /// True when the parser holds a pending `\x1b` that might be a standalone
    /// Escape key or the start of a longer sequence.
    #[inline(always)]
    pub(crate) fn has_pending_escape(&self) -> bool {
        self.state == PS::Escape
    }

    /// Emit a standalone Escape key if the timeout expired mid-sequence.
    pub(crate) fn flush_escape<F: FnMut(Event)>(&mut self, emit: &mut F) {
        if self.state == PS::Escape {
            emit(make_key(KeyCode::Esc, KeyModifiers::empty()));
            self.state = PS::Ground;
        }
        // PasteDrain expires after a generous window (2 seconds) to absorb
        // any residual close-sequence characters that arrive late due to
        // SSH/ConPTY latency.  `paste_start` is reused as the drain
        // deadline timestamp.
        if self.state == PS::PasteDrain {
            let expired = match self.paste_start {
                Some(start) => start.elapsed().as_millis() >= 2000,
                None => true,
            };
            if expired {
                self.state = PS::Ground;
                self.paste_start = None;
            }
        }
    }

    /// Cancel a pending escape without emitting it.  Used when ConPTY has
    /// already consumed the ESC as part of a recognised VT sequence and
    /// delivered a VK event instead — the ESC in the parser is stale.
    pub(crate) fn cancel_escape(&mut self) {
        if self.state == PS::Escape {
            self.state = PS::Ground;
        }
    }

    /// True when the parser is inside a bracketed-paste sequence.
    #[inline(always)]
    pub(crate) fn is_in_paste(&self) -> bool {
        matches!(self.state, PS::Paste | PS::PasteEsc | PS::PasteBrk | PS::PasteNum)
    }

    /// Maximum paste buffer size (1 MB).  Prevents unbounded memory growth
    /// if the close sequence is never received.
    pub(crate) const PASTE_MAX_BYTES: usize = 1_048_576;

    /// Maximum time (in seconds) to stay in Paste state before force-flushing.
    /// If the `\x1b[201~` terminator is lost (e.g. ConPTY strips it, or sshd
    /// transforms it), this prevents the parser from being stuck forever,
    /// which would make the terminal completely unresponsive (issue #197).
    const PASTE_TIMEOUT_SECS: u64 = 2;

    /// Force-flush a stale paste if we have been in Paste state for too long
    /// or the buffer has exceeded the size limit.  Called on every timeout
    /// tick from the reader thread.
    pub(crate) fn flush_stale_paste<F: FnMut(Event)>(&mut self, emit: &mut F) {
        if !self.is_in_paste() { return; }

        let should_flush = if let Some(start) = self.paste_start {
            start.elapsed().as_secs() >= Self::PASTE_TIMEOUT_SECS
                || self.paste.len() >= Self::PASTE_MAX_BYTES
        } else {
            false
        };

        if should_flush {
            ssh_debug_log(&format!(
                "flush_stale_paste: forcing flush after {}ms, {} chars (state={:?})",
                self.paste_start.map(|s| s.elapsed().as_millis()).unwrap_or(0),
                self.paste.len(),
                self.state,
            ));
            // Save current state before flushing to determine the correct
            // transition for absorbing residual close-sequence characters.
            let pre_flush_state = self.state;
            let text = std::mem::take(&mut self.paste);
            if !text.is_empty() {
                emit(Event::Paste(text));
            }
            self.paste_start = None;
            // Transition to the appropriate state to absorb any remaining
            // characters of the close sequence (\x1b[201~) that may still
            // be in-flight.  Going directly to Ground would cause residual
            // characters (especially the trailing '~') to leak as visible
            // input (issue #197).
            match pre_flush_state {
                PS::Paste => {
                    // Close sequence hasn't started arriving through the
                    // VT parser.  However, ConPTY may have stripped the
                    // CSI prefix (\x1b[201) and only leaked the final `~`.
                    // Transition to PasteDrain to absorb that residue.
                    // Reuse paste_start as the drain deadline (500 ms window).
                    self.cur = 0;
                    self.paste_start = Some(std::time::Instant::now());
                    self.state = PS::PasteDrain;
                    ssh_debug_log(&format!(
                        "flush_stale_paste: transitioning to PasteDrain (pre={:?} post={:?})",
                        pre_flush_state, self.state,
                    ));
                }
                PS::PasteEsc => {
                    // Already consumed \x1b.  Transition to Escape so the
                    // remaining [201~ is processed as a normal CSI (which
                    // dispatch_tilde discards for param 201).
                    self.cur = 0;
                    self.state = PS::Escape;
                }
                PS::PasteBrk => {
                    // Consumed \x1b[.  Transition to CsiEntry.
                    self.reset_csi();
                    self.state = PS::CsiEntry;
                }
                PS::PasteNum => {
                    // Consumed \x1b[ plus digits (cur holds accumulated
                    // value).  Transition to CsiParam so the final ~
                    // dispatches via dispatch_tilde (which ignores 201).
                    let saved_cur = self.cur;
                    self.reset_csi();
                    self.cur = saved_cur;
                    self.has_digit = true;
                    self.state = PS::CsiParam;
                }
                _ => {
                    self.cur = 0;
                    self.state = PS::Ground;
                }
            }
        }
    }

    // ── Ground ───────────────────────────────────────────────────────────

    #[inline]
    pub(crate) fn on_ground<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '\x1b' => {
                self.state = PS::Escape;
            }
            '\r' | '\n' => emit(make_key(KeyCode::Enter, KeyModifiers::empty())),
            '\t' => emit(make_key(KeyCode::Tab, KeyModifiers::empty())),
            '\x7f' => emit(make_key(KeyCode::Backspace, KeyModifiers::empty())),
            '\x08' => emit(make_key(KeyCode::Backspace, KeyModifiers::empty())),
            '\0' => emit(make_key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            c if c as u32 >= 1 && (c as u32) <= 26 => {
                // Ctrl+A … Ctrl+Z
                let letter = (b'a' + (c as u8) - 1) as char;
                emit(make_key(KeyCode::Char(letter), KeyModifiers::CONTROL));
            }
            c if c as u32 == 28 => emit(make_key(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
            c if c as u32 == 29 => emit(make_key(KeyCode::Char(']'), KeyModifiers::CONTROL)),
            c if c as u32 == 30 => emit(make_key(KeyCode::Char('^'), KeyModifiers::CONTROL)),
            c if c as u32 == 31 => emit(make_key(KeyCode::Char('_'), KeyModifiers::CONTROL)),
            c => emit(make_key(KeyCode::Char(c), KeyModifiers::empty())),
        }
    }

    // ── Escape ───────────────────────────────────────────────────────────

    pub(crate) fn on_escape<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '[' => {
                self.reset_csi();
                self.state = PS::CsiEntry;
            }
            'O' => {
                self.state = PS::Ss3;
            }
            '\x1b' => {
                // Double-Esc → emit one Escape, stay in Escape state.
                emit(make_key(KeyCode::Esc, KeyModifiers::empty()));
            }
            ']' => {
                // OSC sequence start (\x1b])
                self.osc.clear();
                self.state = PS::Osc;
            }
            c if c >= ' ' && c <= '~' => {
                // Alt + printable character.
                emit(make_key(KeyCode::Char(c), KeyModifiers::ALT));
                self.state = PS::Ground;
            }
            c => {
                // Unknown after Esc — emit Esc then re-process char.
                emit(make_key(KeyCode::Esc, KeyModifiers::empty()));
                self.state = PS::Ground;
                self.on_ground(c, emit);
            }
        }
    }

    // ── CSI entry (\x1b[ received) ───────────────────────────────────────

    pub(crate) fn on_csi_entry<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '<' => {
                self.priv_ch = b'<';
                self.state = PS::CsiParam;
            }
            '?' => {
                self.priv_ch = b'?';
                self.state = PS::CsiParam;
            }
            '0'..='9' => {
                self.cur = (ch as u16) - (b'0' as u16);
                self.has_digit = true;
                self.state = PS::CsiParam;
            }
            ';' => {
                // Empty first param (implicitly 0).
                self.finish_param();
                self.state = PS::CsiParam;
            }
            'M' => {
                // X10 mouse: \x1b[M followed by 3 raw bytes.
                self.x10_n = 0;
                self.state = PS::X10Mouse;
            }
            // CSI with immediate final character (no params).
            c @ ('A'..='Z' | 'a'..='z' | '~') => {
                self.finish_param();
                self.dispatch_csi(c, emit);
                // dispatch_csi sets state (Ground or Paste).
            }
            '\x1b' => {
                // Abort — new escape sequence starting.
                self.state = PS::Escape;
            }
            _ => {
                // Unknown — discard and return to ground.
                self.state = PS::Ground;
            }
        }
    }

    // ── CSI parameter accumulation ───────────────────────────────────────

    pub(crate) fn on_csi_param<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        match ch {
            '0'..='9' => {
                self.cur = self.cur.saturating_mul(10).saturating_add((ch as u16) - (b'0' as u16));
                self.has_digit = true;
            }
            ';' => {
                self.finish_param();
            }
            ':' => {
                // Sub-parameter separator (kitty protocol, etc.) — accumulate
                // like ';' for simplicity; sufficient for SGR mouse.
                self.finish_param();
            }
            c @ ('A'..='Z' | 'a'..='z' | '~') => {
                self.finish_param();
                self.dispatch_csi(c, emit);
                // dispatch_csi sets state (Ground or Paste).
            }
            '\x1b' => {
                self.state = PS::Escape;
            }
            _ => {
                // Unexpected intermediate byte — discard whole sequence.
                self.state = PS::Ground;
            }
        }
    }

    /// Push the current accumulator into the param array and reset.
    #[inline]
    pub(crate) fn finish_param(&mut self) {
        if (self.pidx as usize) < self.params.len() {
            self.params[self.pidx as usize] = self.cur;
            self.pidx += 1;
        }
        self.cur = 0;
        self.has_digit = false;
    }

    // ── CSI dispatch ─────────────────────────────────────────────────────

    /// Dispatch a complete CSI sequence.  Sets `self.state` to Ground (or
    /// Paste for `\x1b[200~`).
    pub(crate) fn dispatch_csi<F: FnMut(Event)>(&mut self, ch: char, emit: &mut F) {
        // SGR mouse: \x1b[<Pb;Px;PyM/m
        if self.priv_ch == b'<' {
            self.dispatch_sgr_mouse(ch, emit);
            self.state = PS::Ground;
            return;
        }

        // DEC private-mode sequences (\x1b[?…) — ignore silently.
        if self.priv_ch == b'?' {
            self.state = PS::Ground;
            return;
        }

        // Bracketed paste start: \x1b[200~
        if ch == '~' && self.pidx >= 1 && self.params[0] == 200 {
            self.paste.clear();
            self.paste_start = Some(std::time::Instant::now());
            self.needs_vti_recheck = true;
            self.state = PS::Paste;
            return;
        }

        // Modifier — second param when present (e.g. \x1b[1;5A = Ctrl+Up).
        let mods = if self.pidx >= 2 {
            decode_modifiers(self.params[1])
        } else {
            KeyModifiers::empty()
        };

        match ch {
            'A' => emit(make_key(KeyCode::Up, mods)),
            'B' => emit(make_key(KeyCode::Down, mods)),
            'C' => emit(make_key(KeyCode::Right, mods)),
            'D' => emit(make_key(KeyCode::Left, mods)),
            'H' => emit(make_key(KeyCode::Home, mods)),
            'F' => emit(make_key(KeyCode::End, mods)),
            'P' => emit(make_key(KeyCode::F(1), mods)),
            'Q' => emit(make_key(KeyCode::F(2), mods)),
            'R' => emit(make_key(KeyCode::F(3), mods)),
            'S' => emit(make_key(KeyCode::F(4), mods)),
            'Z' => emit(make_key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            'I' if self.pidx <= 1 && self.params[0] == 0 => emit(Event::FocusGained),
            'O' if self.pidx <= 1 && self.params[0] == 0 => emit(Event::FocusLost),
            '~' => self.dispatch_tilde(mods, emit),
            _ => {} // Unknown — silently discard.
        }
        self.state = PS::Ground;
    }
}
