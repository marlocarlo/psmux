//! Win32-input-mode encoding for ConPTY.
//!
//! Encodes key events as `ESC[Vk;Sc;Uc;Kd;Cs;Rc_` sequences for writing
//! to the child ConPTY input pipe.  ConPTY created with
//! PSEUDOCONSOLE_WIN32_INPUT_MODE (0x4) accepts these unconditionally and
//! reconstructs KEY_EVENT_RECORDs for the child process.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

// ── Win32 API imports ──────────────────────────────────────────────────

#[link(name = "user32")]
extern "system" {
    fn VkKeyScanW(ch: u16) -> i16;
    fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
}

const MAPVK_VK_TO_VSC: u32 = 0;

// ── dwControlKeyState flags ────────────────────────────────────────────

const SHIFT_PRESSED: u32     = 0x0010;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const LEFT_ALT_PRESSED: u32  = 0x0002;
const ENHANCED_KEY: u32      = 0x0100;

// ── Static mapping table ───────────────────────────────────────────────

struct VkEntry { vk: u16, scan: u16, unicode: u16, enhanced: bool }

/// Map a crossterm KeyCode to (VK, scan, unicode, enhanced) using the
/// static table.  Returns None for character keys (handled dynamically)
/// and unrecognized codes.
fn static_mapping(code: &KeyCode) -> Option<VkEntry> {
    Some(match code {
        KeyCode::Enter    => VkEntry { vk: 0x0D, scan: 0x1C, unicode: 0x0D, enhanced: false },
        KeyCode::Tab      => VkEntry { vk: 0x09, scan: 0x0F, unicode: 0x09, enhanced: false },
        KeyCode::BackTab  => VkEntry { vk: 0x09, scan: 0x0F, unicode: 0x00, enhanced: false },
        KeyCode::Backspace=> VkEntry { vk: 0x08, scan: 0x0E, unicode: 0x08, enhanced: false },
        KeyCode::Esc      => VkEntry { vk: 0x1B, scan: 0x01, unicode: 0x1B, enhanced: false },
        KeyCode::Left     => VkEntry { vk: 0x25, scan: 0x4B, unicode: 0x00, enhanced: true },
        KeyCode::Right    => VkEntry { vk: 0x27, scan: 0x4D, unicode: 0x00, enhanced: true },
        KeyCode::Up       => VkEntry { vk: 0x26, scan: 0x48, unicode: 0x00, enhanced: true },
        KeyCode::Down     => VkEntry { vk: 0x28, scan: 0x50, unicode: 0x00, enhanced: true },
        KeyCode::Home     => VkEntry { vk: 0x24, scan: 0x47, unicode: 0x00, enhanced: true },
        KeyCode::End      => VkEntry { vk: 0x23, scan: 0x4F, unicode: 0x00, enhanced: true },
        KeyCode::PageUp   => VkEntry { vk: 0x21, scan: 0x49, unicode: 0x00, enhanced: true },
        KeyCode::PageDown => VkEntry { vk: 0x22, scan: 0x51, unicode: 0x00, enhanced: true },
        KeyCode::Insert   => VkEntry { vk: 0x2D, scan: 0x52, unicode: 0x00, enhanced: true },
        KeyCode::Delete   => VkEntry { vk: 0x2E, scan: 0x53, unicode: 0x00, enhanced: true },
        KeyCode::Char(' ')=> VkEntry { vk: 0x20, scan: 0x39, unicode: 0x20, enhanced: false },
        KeyCode::F(n) => {
            let (vk, scan) = match n {
                1  => (0x70u16, 0x3Bu16), 2  => (0x71, 0x3C),
                3  => (0x72, 0x3D),       4  => (0x73, 0x3E),
                5  => (0x74, 0x3F),       6  => (0x75, 0x40),
                7  => (0x76, 0x41),       8  => (0x77, 0x42),
                9  => (0x78, 0x43),       10 => (0x79, 0x44),
                11 => (0x7A, 0x57),       12 => (0x7B, 0x58),
                _ => return None,
            };
            VkEntry { vk, scan, unicode: 0x00, enhanced: false }
        }
        _ => return None,
    })
}

// ── Encode crossterm KeyEvent ──────────────────────────────────────────

/// Encode a crossterm KeyEvent as a win32-input-mode key-down sequence.
///
/// Format: `ESC [ Vk ; Sc ; Uc ; 1 ; Cs ; 1 _`
///
/// The child ConPTY parser reconstructs a KEY_EVENT_RECORD from this and
/// places it in the console input buffer for the child process.
pub fn encode_key_win32(key: &KeyEvent) -> Option<Vec<u8>> {
    let (vk, scan, unicode, enhanced) = if let KeyCode::Char(c) = key.code {
        if c == ' ' {
            let e = static_mapping(&key.code).unwrap();
            (e.vk, e.scan, e.unicode, e.enhanced)
        } else {
            // Dynamic: VkKeyScanW for VK, MapVirtualKeyW for scan code
            let vk_result = unsafe { VkKeyScanW(c as u16) };
            let vk = if vk_result < 0 { 0u16 } else { (vk_result as u16) & 0xFF };
            let scan = if vk > 0 {
                unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 }
            } else {
                0u16
            };
            // Unicode depends on modifiers:
            // - AltGr (Ctrl+Alt + non-lowercase): actual character verbatim
            // - Ctrl only: control character (c & 0x1F)
            // - Otherwise: literal character
            let unicode = if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.modifiers.contains(KeyModifiers::ALT)
                && !c.is_ascii_lowercase()
            {
                c as u16
            } else if key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                (c.to_ascii_lowercase() as u16) & 0x1F
            } else {
                c as u16
            };
            (vk, scan, unicode, false)
        }
    } else if let Some(e) = static_mapping(&key.code) {
        let mut uni = e.unicode;
        // When CTRL is held, Windows modifies the unicode char for certain
        // keys.  libuv emits the unicode char directly, so we must match
        // what Windows would produce or the child sees the wrong byte.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            uni = match uni {
                0x08 => 0x17,               // BS → ETB/Ctrl+W (word-delete, matches VS Code)
                0x0D => 0x0A,               // CR → LF   (Ctrl+Enter)
                u if u >= 0x20 => u & 0x1F, // Space/printable → control char
                u => u,                     // Tab, Esc, etc. unchanged
            };
        }
        (e.vk, e.scan, uni, e.enhanced)
    } else {
        return None;
    };

    // Build dwControlKeyState flags
    let mut mods = key.modifiers;
    if matches!(key.code, KeyCode::BackTab) {
        mods |= KeyModifiers::SHIFT; // BackTab implies Shift
    }
    // libuv (Node.js) reads KEY_EVENT_RECORDs via ReadConsoleInputW but
    // ignores SHIFT_PRESSED for keys with non-zero UnicodeChar.  It DOES
    // check LEFT_ALT_PRESSED and prepends ESC (0x1B).  Map SHIFT→ALT
    // only for Enter — Shift+Enter becomes Alt+Enter, libuv emits \x1b\r,
    // and terminal apps (Claude Code) recognize it as modified Enter.
    // Other named keys (Space, Tab, Backspace, Esc) are NOT mapped because
    // Shift has no meaningful terminal behavior for them, and the mapping
    // would cause phantom ESC prefixes during fast typing when the physical
    // Shift key-up overlaps with the next key-down.
    if matches!(key.code, KeyCode::Enter)
        && mods.contains(KeyModifiers::SHIFT)
        && !mods.contains(KeyModifiers::CONTROL)
    {
        mods.remove(KeyModifiers::SHIFT);
        mods.insert(KeyModifiers::ALT);
    }
    let mut flags = 0u32;
    if mods.contains(KeyModifiers::SHIFT)   { flags |= SHIFT_PRESSED; }
    if mods.contains(KeyModifiers::CONTROL) { flags |= LEFT_CTRL_PRESSED; }
    if mods.contains(KeyModifiers::ALT)     { flags |= LEFT_ALT_PRESSED; }
    if enhanced                              { flags |= ENHANCED_KEY; }

    Some(format!("\x1b[{};{};{};1;{};1_", vk, scan, unicode, flags).into_bytes())
}

// ── Encode string key name ─────────────────────────────────────────────

/// Encode a string key name (e.g. "S-Enter", "C-a", "f5") as a
/// win32-input-mode sequence.  Used by `send_key_to_active` which
/// receives string names from the client.
pub fn encode_key_name_win32(name: &str) -> Option<Vec<u8>> {
    let upper = name.to_uppercase();
    let mut rest = upper.as_str();
    let mut modifiers = KeyModifiers::empty();

    // Strip modifier prefixes
    loop {
        if rest.starts_with("C-") { modifiers |= KeyModifiers::CONTROL; rest = &rest[2..]; }
        else if rest.starts_with("M-") { modifiers |= KeyModifiers::ALT; rest = &rest[2..]; }
        else if rest.starts_with("S-") { modifiers |= KeyModifiers::SHIFT; rest = &rest[2..]; }
        else { break; }
    }

    // Map base key name to KeyCode
    let code = match rest {
        "ENTER" | "RETURN" | "CR" => KeyCode::Enter,
        "TAB" => KeyCode::Tab,
        "BTAB" | "BACKTAB" => KeyCode::BackTab,
        "BACKSPACE" | "BSPACE" => KeyCode::Backspace,
        "ESC" | "ESCAPE" => KeyCode::Esc,
        "LEFT" => KeyCode::Left,
        "RIGHT" => KeyCode::Right,
        "UP" => KeyCode::Up,
        "DOWN" => KeyCode::Down,
        "HOME" => KeyCode::Home,
        "END" => KeyCode::End,
        "PAGEUP" | "PPAGE" => KeyCode::PageUp,
        "PAGEDOWN" | "NPAGE" => KeyCode::PageDown,
        "INSERT" | "IC" => KeyCode::Insert,
        "DELETE" | "DC" => KeyCode::Delete,
        "SPACE" => KeyCode::Char(' '),
        s if s.starts_with('F') && s.len() >= 2 => {
            if let Ok(n) = s[1..].parse::<u8>() { KeyCode::F(n) } else { return None; }
        }
        s if s.len() == 1 => {
            KeyCode::Char(s.chars().next()?.to_ascii_lowercase())
        }
        _ => return None,
    };

    let event = KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    encode_key_win32(&event)
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn plain_enter() {
        let seq = encode_key_win32(&key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[13;28;13;1;0;1_");
    }

    #[test]
    fn shift_enter_maps_to_alt() {
        // SHIFT→ALT mapping for Enter: libuv ignores SHIFT but prepends ESC for ALT,
        // producing \x1b\r which terminal apps recognize as modified Enter.
        let seq = encode_key_win32(&key(KeyCode::Enter, KeyModifiers::SHIFT)).unwrap();
        // flags = LEFT_ALT_PRESSED (2), not SHIFT_PRESSED (16)
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[13;28;13;1;2;1_");
    }

    #[test]
    fn ctrl_backspace() {
        let seq = encode_key_win32(&key(KeyCode::Backspace, KeyModifiers::CONTROL)).unwrap();
        // unicode = 0x17 (ETB/Ctrl+W = word-delete, matches VS Code); flags = CTRL (8)
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[8;14;23;1;8;1_");
    }

    #[test]
    fn left_arrow_has_enhanced_flag() {
        let seq = encode_key_win32(&key(KeyCode::Left, KeyModifiers::NONE)).unwrap();
        // flags = ENHANCED_KEY (256)
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[37;75;0;1;256;1_");
    }

    #[test]
    fn ctrl_shift_left() {
        let seq = encode_key_win32(&key(KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::SHIFT)).unwrap();
        // flags = SHIFT(16) + CTRL(8) + ENHANCED(256) = 280
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[37;75;0;1;280;1_");
    }

    #[test]
    fn escape_key() {
        let seq = encode_key_win32(&key(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[27;1;27;1;0;1_");
    }

    #[test]
    fn backtab_has_shift() {
        let seq = encode_key_win32(&key(KeyCode::BackTab, KeyModifiers::NONE)).unwrap();
        // BackTab implies SHIFT (16), unicode = 0
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[9;15;0;1;16;1_");
    }

    #[test]
    fn f5_key() {
        let seq = encode_key_win32(&key(KeyCode::F(5), KeyModifiers::NONE)).unwrap();
        // VK_F5 = 116, scan = 63
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[116;63;0;1;0;1_");
    }

    #[test]
    fn space_key() {
        let seq = encode_key_win32(&key(KeyCode::Char(' '), KeyModifiers::NONE)).unwrap();
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[32;57;32;1;0;1_");
    }

    #[test]
    fn delete_key() {
        let seq = encode_key_win32(&key(KeyCode::Delete, KeyModifiers::NONE)).unwrap();
        // VK_DELETE=46, scan=83, enhanced
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[46;83;0;1;256;1_");
    }

    // ── encode_key_name_win32 tests ────────────────────────────────────

    #[test]
    fn name_shift_enter_maps_to_alt() {
        // S-Enter → ALT flag (libuv SHIFT→ALT mapping for Enter)
        let seq = encode_key_name_win32("S-Enter").unwrap();
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[13;28;13;1;2;1_");
    }

    #[test]
    fn name_plain_enter() {
        let seq = encode_key_name_win32("enter").unwrap();
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[13;28;13;1;0;1_");
    }

    #[test]
    fn name_ctrl_left() {
        let seq = encode_key_name_win32("C-Left").unwrap();
        // flags = CTRL(8) + ENHANCED(256) = 264
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[37;75;0;1;264;1_");
    }

    #[test]
    fn name_f1() {
        let seq = encode_key_name_win32("f1").unwrap();
        // VK_F1 = 112, scan = 59
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[112;59;0;1;0;1_");
    }

    #[test]
    fn name_ctrl_a() {
        let seq = encode_key_name_win32("C-a").unwrap();
        // VK_A = 65, scan = 30, unicode = 1 (Ctrl+A), flags = CTRL(8)
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[65;30;1;1;8;1_");
    }

    #[test]
    fn name_alt_a() {
        let seq = encode_key_name_win32("M-a").unwrap();
        // VK_A = 65, scan = 30, unicode = 97 ('a'), flags = ALT(2)
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[65;30;97;1;2;1_");
    }

    #[test]
    fn name_backtab() {
        let seq = encode_key_name_win32("btab").unwrap();
        assert_eq!(String::from_utf8_lossy(&seq), "\x1b[9;15;0;1;16;1_");
    }

    #[test]
    fn name_unknown_returns_none() {
        assert!(encode_key_name_win32("foobar").is_none());
    }
}
