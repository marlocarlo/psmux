#[allow(unused_imports)]
use std::env;
use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::types::{AppState, Action, Bind};
use crate::commands::parse_command_to_action;

// Track the current config file being parsed (for #{current_file}, #{d:current_file})
use super::*;

pub fn parse_bind_key(app: &mut AppState, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 { return; }
    
    let mut i = 1;
    let mut _key_table = "prefix".to_string();
    let mut _repeatable = false;
    
    while i < parts.len() {
        let p = parts[i];
        // A flag must start with '-' AND be longer than 1 char (e.g. "-r", "-n", "-T").
        // A bare "-" is a valid key name, not a flag.
        if p.starts_with('-') && p.len() > 1 {
            if p.contains('r') { _repeatable = true; }
            if p.contains('n') { _key_table = "root".to_string(); }
            if p.contains('T') {
                i += 1;
                if i < parts.len() { _key_table = parts[i].to_string(); }
            }
            i += 1;
        } else {
            break;
        }
    }
    
    if i >= parts.len() { return; }
    let key_str = parts[i];
    i += 1;
    
    if i >= parts.len() { return; }
    let command = parts[i..].join(" ");
    
    // Split on `\;` or `;` to support command chaining (like tmux `bind x split-window \; select-pane -D`)
    let sub_commands: Vec<String> = split_chained_commands(&command);
    
    if let Some(key) = parse_key_name(key_str) {
        let key = normalize_key_for_binding(key);
        let action = if sub_commands.len() > 1 {
            // Multiple chained commands
            Action::CommandChain(sub_commands)
        } else if let Some(a) = parse_command_to_action(&command) {
            a
        } else {
            return;
        };
        let table = app.key_tables.entry(_key_table).or_default();
        table.retain(|b| b.key != key);
        table.push(Bind { key, action, repeat: _repeatable });
    }
}

pub fn parse_unbind_key(app: &mut AppState, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 { return; }
    
    let mut i = 1;
    let mut unbind_all = false;
    let mut table: Option<String> = None;
    
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('a') { unbind_all = true; }
            if p.contains('n') { table = Some("root".to_string()); }
            if p.contains('T') && i + 1 < parts.len() {
                i += 1;
                table = Some(parts[i].to_string());
            }
            i += 1;
        } else {
            break;
        }
    }
    
    if unbind_all {
        if let Some(t) = table {
            // -a -T <table>: only clear that table
            if let Some(binds) = app.key_tables.get_mut(&t) {
                binds.clear();
            }
        } else {
            // -a (no table): clear ALL tables + suppress defaults
            app.key_tables.clear();
            app.defaults_suppressed = true;
        }
        return;
    }
    
    if i < parts.len() {
        if let Some(key) = parse_key_name(parts[i]) {
            let key = normalize_key_for_binding(key);
            // Remove from the targeted table only (tmux behavior).
            // Default is "prefix" when no -n or -T is specified.
            let target = table.unwrap_or_else(|| "prefix".to_string());
            if let Some(binds) = app.key_tables.get_mut(&target) {
                binds.retain(|b| b.key != key);
            }
        }
    }
}

/// Map a multi-character key name (case-insensitive) to a KeyCode.
/// Returns None if the name is not recognized.
pub(crate) fn named_key(name: &str) -> Option<KeyCode> {
    match name.to_lowercase().as_str() {
        "space" => Some(KeyCode::Char(' ')),
        "enter" | "return" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "btab" | "backtab" => Some(KeyCode::BackTab),
        "escape" | "esc" => Some(KeyCode::Esc),
        "bspace" | "backspace" => Some(KeyCode::Backspace),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" | "ppage" | "pgup" => Some(KeyCode::PageUp),
        "pagedown" | "npage" | "pgdn" => Some(KeyCode::PageDown),
        "insert" | "ic" => Some(KeyCode::Insert),
        "delete" | "dc" => Some(KeyCode::Delete),
        "f1" => Some(KeyCode::F(1)),
        "f2" => Some(KeyCode::F(2)),
        "f3" => Some(KeyCode::F(3)),
        "f4" => Some(KeyCode::F(4)),
        "f5" => Some(KeyCode::F(5)),
        "f6" => Some(KeyCode::F(6)),
        "f7" => Some(KeyCode::F(7)),
        "f8" => Some(KeyCode::F(8)),
        "f9" => Some(KeyCode::F(9)),
        "f10" => Some(KeyCode::F(10)),
        "f11" => Some(KeyCode::F(11)),
        "f12" => Some(KeyCode::F(12)),
        _ => None,
    }
}

pub fn parse_key_name(name: &str) -> Option<(KeyCode, KeyModifiers)> {
    let name = name.trim();
    // Strip surrounding quotes (single or double) — plugins often quote special chars
    // e.g., bind-key '|' split-window -h
    let name = if (name.starts_with('\'') && name.ends_with('\'') && name.len() >= 2)
        || (name.starts_with('"') && name.ends_with('"') && name.len() >= 2) {
        &name[1..name.len()-1]
    } else {
        name
    };

    // ── Extract all modifier prefixes (C-, M-, S-) then resolve the base key ──
    // This supports arbitrary combinations: C-Tab, C-S-Tab, C-M-S-Up, etc.
    let mut rest = name;
    let mut mods = KeyModifiers::NONE;
    loop {
        if rest.starts_with("C-") { mods |= KeyModifiers::CONTROL; rest = &rest[2..]; }
        else if rest.starts_with("M-") { mods |= KeyModifiers::ALT; rest = &rest[2..]; }
        else if rest.starts_with("S-") { mods |= KeyModifiers::SHIFT; rest = &rest[2..]; }
        else if rest.starts_with("^") && rest.len() > 1 { mods |= KeyModifiers::CONTROL; rest = &rest[1..]; }
        else { break; }
    }

    if mods != KeyModifiers::NONE {
        // S-Tab (with or without other modifiers) → BackTab + remaining mods
        if rest.eq_ignore_ascii_case("Tab") && mods.contains(KeyModifiers::SHIFT) {
            return Some((KeyCode::BackTab, mods.difference(KeyModifiers::SHIFT)));
        }
        if let Some(kc) = named_key(rest) {
            return Some((kc, mods));
        }
        if rest.len() == 1 {
            if let Some(c) = rest.chars().next() {
                if mods.contains(KeyModifiers::SHIFT) {
                    return Some((KeyCode::Char(c.to_ascii_uppercase()), mods.difference(KeyModifiers::SHIFT)));
                }
                return Some((KeyCode::Char(c.to_ascii_lowercase()), mods));
            }
        }
        // Unrecognized key after modifiers — fall through
    }
    
    match name.to_uppercase().as_str() {
        "ENTER" => return Some((KeyCode::Enter, KeyModifiers::NONE)),
        "TAB" => return Some((KeyCode::Tab, KeyModifiers::NONE)),
        "BTAB" => return Some((KeyCode::BackTab, KeyModifiers::NONE)),
        "ESCAPE" | "ESC" => return Some((KeyCode::Esc, KeyModifiers::NONE)),
        "SPACE" => return Some((KeyCode::Char(' '), KeyModifiers::NONE)),
        "BSPACE" | "BACKSPACE" => return Some((KeyCode::Backspace, KeyModifiers::NONE)),
        "UP" => return Some((KeyCode::Up, KeyModifiers::NONE)),
        "DOWN" => return Some((KeyCode::Down, KeyModifiers::NONE)),
        "LEFT" => return Some((KeyCode::Left, KeyModifiers::NONE)),
        "RIGHT" => return Some((KeyCode::Right, KeyModifiers::NONE)),
        "HOME" => return Some((KeyCode::Home, KeyModifiers::NONE)),
        "END" => return Some((KeyCode::End, KeyModifiers::NONE)),
        "PAGEUP" | "PPAGE" | "PGUP" => return Some((KeyCode::PageUp, KeyModifiers::NONE)),
        "PAGEDOWN" | "NPAGE" | "PGDN" => return Some((KeyCode::PageDown, KeyModifiers::NONE)),
        "INSERT" | "IC" => return Some((KeyCode::Insert, KeyModifiers::NONE)),
        "DELETE" | "DC" => return Some((KeyCode::Delete, KeyModifiers::NONE)),
        "F1" => return Some((KeyCode::F(1), KeyModifiers::NONE)),
        "F2" => return Some((KeyCode::F(2), KeyModifiers::NONE)),
        "F3" => return Some((KeyCode::F(3), KeyModifiers::NONE)),
        "F4" => return Some((KeyCode::F(4), KeyModifiers::NONE)),
        "F5" => return Some((KeyCode::F(5), KeyModifiers::NONE)),
        "F6" => return Some((KeyCode::F(6), KeyModifiers::NONE)),
        "F7" => return Some((KeyCode::F(7), KeyModifiers::NONE)),
        "F8" => return Some((KeyCode::F(8), KeyModifiers::NONE)),
        "F9" => return Some((KeyCode::F(9), KeyModifiers::NONE)),
        "F10" => return Some((KeyCode::F(10), KeyModifiers::NONE)),
        "F11" => return Some((KeyCode::F(11), KeyModifiers::NONE)),
        "F12" => return Some((KeyCode::F(12), KeyModifiers::NONE)),
        _ => {}
    }
    
    if name.len() == 1 {
        if let Some(c) = name.chars().next() {
            return Some((KeyCode::Char(c), KeyModifiers::NONE));
        }
    }
    
    None
}

pub fn source_file(app: &mut AppState, path: &str) {
    let path = path.trim().trim_matches('"').trim_matches('\'');

    // Handle -F flag: expand format strings in the path
    let (path, format_expand) = if path.starts_with("-F ") || path.starts_with("-F\t") {
        (path[3..].trim().trim_matches('"').trim_matches('\''), true)
    } else {
        (path, false)
    };

    let expanded_path = if format_expand {
        crate::format::expand_format(path, app)
    } else {
        path.to_string()
    };

    let expanded_path = if expanded_path.starts_with('~') {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        expanded_path.replacen('~', &home, 1)
    } else {
        expanded_path
    };

    // Normalize path separators for Windows
    let expanded_path = expanded_path.replace('/', &std::path::MAIN_SEPARATOR.to_string());

    // Fallback: if path references ~/.psmux/ but doesn't exist and the
    // XDG equivalent (~/.config/psmux/) does, use that instead (issue #135).
    let expanded_path = if !std::path::Path::new(&expanded_path).exists() {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        let classic = format!("{}\\.psmux\\", home);
        if expanded_path.starts_with(&classic) {
            let xdg_base = env::var("XDG_CONFIG_HOME")
                .unwrap_or_else(|_| format!("{}\\.config", home));
            let xdg_alt = expanded_path.replacen(&classic, &format!("{}\\psmux\\", xdg_base), 1);
            if std::path::Path::new(&xdg_alt).exists() { xdg_alt } else { expanded_path }
        } else {
            expanded_path
        }
    } else {
        expanded_path
    };

    // Save and restore current_config_file around the nested parse
    let prev_file = current_config_file();
    set_current_config_file(&expanded_path);

    if let Ok(content) = std::fs::read_to_string(&expanded_path) {
        parse_config_content(app, &content);
    }

    set_current_config_file(&prev_file);
}

/// Parse a key string like "C-a", "M-x", "F1", "Space" into (KeyCode, KeyModifiers)
pub fn parse_key_string(key: &str) -> Option<(KeyCode, KeyModifiers)> {
    let key = key.trim();
    let mut mods = KeyModifiers::empty();
    let mut key_part = key;
    
    while key_part.len() > 2 {
        if key_part.starts_with("C-") || key_part.starts_with("c-") {
            mods |= KeyModifiers::CONTROL;
            key_part = &key_part[2..];
        } else if key_part.starts_with("M-") || key_part.starts_with("m-") {
            mods |= KeyModifiers::ALT;
            key_part = &key_part[2..];
        } else if key_part.starts_with("S-") || key_part.starts_with("s-") {
            mods |= KeyModifiers::SHIFT;
            key_part = &key_part[2..];
        } else {
            break;
        }
    }
    
    let keycode = match key_part.to_lowercase().as_str() {
        // Single character keys: preserve the ORIGINAL case from key_part, not the lowercased version.
        // This is critical for case-sensitive bind-key (issue #157): bind-key T != bind-key t.
        _ if key_part.len() == 1 => {
            KeyCode::Char(key_part.chars().next().unwrap())
        }
        "space" => KeyCode::Char(' '),
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "btab" | "backtab" => KeyCode::BackTab,
        "escape" | "esc" => KeyCode::Esc,
        "backspace" | "bspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "ppage" => KeyCode::PageUp,
        "pagedown" | "npage" => KeyCode::PageDown,
        "insert" | "ic" => KeyCode::Insert,
        "delete" | "dc" => KeyCode::Delete,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        "\"" => KeyCode::Char('"'),
        "%" => KeyCode::Char('%'),
        "," => KeyCode::Char(','),
        "." => KeyCode::Char('.'),
        ":" => KeyCode::Char(':'),
        ";" => KeyCode::Char(';'),
        "[" => KeyCode::Char('['),
        "]" => KeyCode::Char(']'),
        "{" => KeyCode::Char('{'),
        "}" => KeyCode::Char('}'),
        _ => {
            return None;
        }
    };
    
    Some((keycode, mods))
}

/// Format a key binding back to string representation
pub fn format_key_binding(key: &(KeyCode, KeyModifiers)) -> String {
    let (keycode, mods) = key;
    let mut result = String::new();
    
    if mods.contains(KeyModifiers::CONTROL) {
        result.push_str("C-");
    }
    if mods.contains(KeyModifiers::ALT) {
        result.push_str("M-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        result.push_str("S-");
    }
    
    let key_str = match keycode {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BTab".to_string(),
        KeyCode::Esc => "Escape".to_string(),
        KeyCode::Backspace => "BSpace".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PPage".to_string(),
        KeyCode::PageDown => "NPage".to_string(),
        KeyCode::Insert => "IC".to_string(),
        KeyCode::Delete => "DC".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => "?".to_string(),
    };
    
    result.push_str(&key_str);
    result
}
