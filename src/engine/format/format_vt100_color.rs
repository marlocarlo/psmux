#[allow(unused_imports)]
// format.rs — tmux-compatible format expansion engine
//
// Supports: variables, #{?cond,t,f}, #{==:a,b}, #{!=:a,b}, #{<:a,b}, etc,
// #{s/pat/rep/flags:var}, #{b:var}, #{d:var}, #{t:var}, #{l:str},
// #{E:var}, #{T:var}, #{q:var}, #{e|op|flags:a,b}, #{m/flags:pat,str},
// #{=N:var}, #{=/N/marker:var}, #{pN:var}, #{||:a,b}, #{&&:a,b},
// #{C/flags:fmt}, chained modifiers with ';',
// -F custom format for list commands.

use std::env;
use std::cell::Cell;

use crate::types::{AppState, Node, LayoutKind, Pane, Mode, VERSION};
use crate::tree::{split_with_gaps, get_active_pane_id, active_pane, count_panes};
use crate::config::format_key_binding;

// Thread-local override for per-pane format expansion in list-panes.
// When set to Some(pos), pane_* variables resolve for the Nth pane (0-based)
// instead of the active pane.
use super::*;

pub(crate) fn format_vt100_color(color: vt100::Color) -> String {
    match color {
        vt100::Color::Default => "default".into(),
        vt100::Color::Idx(i) => match i {
            0 => "black".into(),
            1 => "red".into(),
            2 => "green".into(),
            3 => "yellow".into(),
            4 => "blue".into(),
            5 => "magenta".into(),
            6 => "cyan".into(),
            7 => "white".into(),
            8 => "bright black".into(),
            9 => "bright red".into(),
            10 => "bright green".into(),
            11 => "bright yellow".into(),
            12 => "bright blue".into(),
            13 => "bright magenta".into(),
            14 => "bright cyan".into(),
            15 => "bright white".into(),
            _ => format!("colour{}", i),
        },
        vt100::Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
    }
}

pub(crate) fn hostname_cached() -> String {
    use std::sync::OnceLock;
    static HOSTNAME: OnceLock<String> = OnceLock::new();
    HOSTNAME.get_or_init(|| {
        env::var("COMPUTERNAME")
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_default()
    }).clone()
}

pub(crate) fn find_matching_brace(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1usize;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 { return Some(i); }
        } else if i + 1 < bytes.len() && bytes[i] == b'#' && bytes[i + 1] == b'{' {
            depth += 1;
            i += 1;
        }
        i += 1;
    }
    None
}

pub(crate) fn split_at_depth0(s: &str, delim: u8) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;       // #{...} nesting depth
    let mut in_style = false;      // inside #[...] style directive
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            depth += 1;
            i += 2;
            continue;
        }
        if bytes[i] == b'}' && depth > 0 {
            depth -= 1;
            i += 1;
            continue;
        }
        // Track #[...] style directives — commas inside are NOT delimiters
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' && !in_style {
            in_style = true;
            i += 2;
            continue;
        }
        if bytes[i] == b']' && in_style {
            in_style = false;
            i += 1;
            continue;
        }
        // Handle #, (escaped delimiter) – skip over without splitting
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == delim && depth == 0 {
            i += 2;
            continue;
        }
        if bytes[i] == delim && depth == 0 && !in_style {
            parts.push(s[start..i].to_string());
            start = i + 1;
        }
        i += 1;
    }
    parts.push(s[start..].to_string());
    parts
}

pub(crate) fn split_conditional(s: &str) -> (String, String, String) {
    let parts = split_at_depth0(s, b',');
    match parts.len() {
        0 => (String::new(), String::new(), String::new()),
        1 => (parts[0].clone(), String::new(), String::new()),
        2 => (parts[0].clone(), parts[1].clone(), String::new()),
        _ => (parts[0].clone(), parts[1].clone(), parts[2..].join(",")),
    }
}

pub(crate) fn glob_match(pattern: &str, text: &str, case_insensitive: bool) -> bool {
    let p = if case_insensitive { pattern.to_lowercase() } else { pattern.to_string() };
    let t = if case_insensitive { text.to_lowercase() } else { text.to_string() };
    glob_match_impl(p.as_bytes(), t.as_bytes())
}

pub(crate) fn glob_match_impl(pattern: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;
    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == text[ti]) {
            pi += 1; ti += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star_pi = pi; star_ti = ti; pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1; star_ti += 1; ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' { pi += 1; }
    pi == pattern.len()
}

// ─────────────────── list-* format helpers ───────────────────────

/// Default format for list-windows (tmux-style one-per-line).
pub fn default_list_windows_format() -> &'static str {
    "#{window_index}: #{window_name}#{window_flags} (#{window_panes} panes) [#{window_width}x#{window_height}]"
}

/// Default format for list-panes.
pub fn default_list_panes_format() -> &'static str {
    "#{pane_index}: [#{pane_width}x#{pane_height}] [history #{history_limit}/#{history_limit}] #{pane_id} (active)"
}

/// Default format for list-sessions.
pub fn default_list_sessions_format() -> &'static str {
    "#{session_name}: #{session_windows} windows (created #{session_created_string})"
}

/// Default format for list-buffers.
pub fn default_list_buffers_format() -> &'static str {
    "#{buffer_name}: #{buffer_size} bytes: \"#{buffer_sample}\""
}

/// Format a list of windows using a format string.
pub fn format_list_windows(app: &AppState, fmt: &str) -> String {
    let mut lines = Vec::with_capacity(app.windows.len());
    for (i, _win) in app.windows.iter().enumerate() {
        lines.push(expand_format_for_window(fmt, app, i));
    }
    lines.join("\n")
}

/// Format a list of panes for the active window.
pub fn format_list_panes(app: &AppState, fmt: &str, win_idx: usize) -> String {
    let win = match app.windows.get(win_idx) {
        Some(w) => w,
        None => return String::new(),
    };
    let mut ids = Vec::new();
    collect_pane_ids(&win.root, &mut ids);
    ids.iter().enumerate().map(|(pos, _pid)| {
        PANE_POS_OVERRIDE.set(Some(pos));
        let line = expand_format_for_window(fmt, app, win_idx);
        PANE_POS_OVERRIDE.set(None);
        line
    }).collect::<Vec<_>>().join("\n")
}

pub(crate) fn collect_pane_ids(node: &Node, ids: &mut Vec<usize>) {
    match node {
        Node::Leaf(p) => ids.push(p.id),
        Node::Split { children, .. } => {
            for child in children { collect_pane_ids(child, ids); }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests-rs/test_format.rs"]
mod tests;
