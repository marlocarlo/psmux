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

/// Map a Windows virtual-key code to a crossterm `KeyCode`.
/// Returns `None` for modifier-only keys (Ctrl, Shift, Alt, CapsLock, etc.)
/// and other keys we don't need to handle.
#[cfg(windows)]
pub(crate) fn vk_to_keycode(vk: u16) -> Option<KeyCode> {
    match vk {
        0x08 => Some(KeyCode::Backspace),   // VK_BACK
        0x09 => Some(KeyCode::Tab),         // VK_TAB
        0x0D => Some(KeyCode::Enter),       // VK_RETURN
        0x1B => Some(KeyCode::Esc),         // VK_ESCAPE
        0x20 => Some(KeyCode::Char(' ')),   // VK_SPACE
        0x21 => Some(KeyCode::PageUp),      // VK_PRIOR
        0x22 => Some(KeyCode::PageDown),    // VK_NEXT
        0x23 => Some(KeyCode::End),         // VK_END
        0x24 => Some(KeyCode::Home),        // VK_HOME
        0x25 => Some(KeyCode::Left),        // VK_LEFT
        0x26 => Some(KeyCode::Up),          // VK_UP
        0x27 => Some(KeyCode::Right),       // VK_RIGHT
        0x28 => Some(KeyCode::Down),        // VK_DOWN
        0x2D => Some(KeyCode::Insert),      // VK_INSERT
        0x2E => Some(KeyCode::Delete),      // VK_DELETE
        0x70 => Some(KeyCode::F(1)),        // VK_F1
        0x71 => Some(KeyCode::F(2)),
        0x72 => Some(KeyCode::F(3)),
        0x73 => Some(KeyCode::F(4)),
        0x74 => Some(KeyCode::F(5)),
        0x75 => Some(KeyCode::F(6)),
        0x76 => Some(KeyCode::F(7)),
        0x77 => Some(KeyCode::F(8)),
        0x78 => Some(KeyCode::F(9)),
        0x79 => Some(KeyCode::F(10)),
        0x7A => Some(KeyCode::F(11)),
        0x7B => Some(KeyCode::F(12)),       // VK_F12
        _ => None,
    }
}

/// Extract crossterm `KeyModifiers` from Win32 `dwControlKeyState`.
#[cfg(windows)]
pub(crate) fn vk_modifiers(state: u32) -> KeyModifiers {
    let mut m = KeyModifiers::empty();
    if state & 0x0010 != 0 { m |= KeyModifiers::SHIFT; }      // SHIFT_PRESSED
    if state & (0x0001 | 0x0002) != 0 { m |= KeyModifiers::ALT; }     // LEFT/RIGHT_ALT
    if state & (0x0004 | 0x0008) != 0 { m |= KeyModifiers::CONTROL; } // LEFT/RIGHT_CTRL
    m
}

// ─── Debug logging ───────────────────────────────────────────────────────────

/// Global log file shared across all threads (main + reader).
#[cfg(windows)]
pub(crate) static SSH_LOG: std::sync::LazyLock<std::sync::Mutex<Option<std::fs::File>>> =
    std::sync::LazyLock::new(|| {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        let dir = format!("{}/.psmux", home);
        let _ = std::fs::create_dir_all(&dir);
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(format!("{}/ssh_input.log", dir))
            .ok();
        std::sync::Mutex::new(f)
    });

/// Write a line to `~/.psmux/ssh_input.log`.  Always active in SSH mode;
/// set `PSMUX_SSH_DEBUG=1` for verbose per-event logging.
#[cfg(windows)]
pub(crate) fn ssh_debug_log(msg: &str) {
    use std::io::Write;
    if let Ok(mut guard) = SSH_LOG.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{}", msg);
            let _ = f.flush();
        }
    }
}

/// True when verbose per-event logging is enabled.
#[cfg(windows)]
pub(crate) fn ssh_verbose() -> bool {
    std::env::var("PSMUX_SSH_DEBUG").ok().as_deref() == Some("1")
}

// ─── Windows: SSH reader thread + Win32 FFI ──────────────────────────────────
