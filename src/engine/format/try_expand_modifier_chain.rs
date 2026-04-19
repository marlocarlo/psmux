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

/// Try to parse and apply modifier chain(s). Returns None if expr is a plain variable.
pub(crate) fn try_expand_modifier_chain(expr: &str, app: &AppState, win_idx: usize) -> Option<String> {
    let bytes = expr.as_bytes();
    let first = bytes[0];

    // Quick check: does this look like a modifier?
    let is_modifier_start = matches!(first,
        b't' | b'b' | b'd' | b'l' | b'E' | b'T' | b'q' | b's' | b'm' | b'C' |
        b'e' | b'p' | b'=' | b'N' | b'w'
    );

    if !is_modifier_start {
        return None;
    }

    // Special: 'l' modifier with colon — #{l:string} returns literal string
    if first == b'l' {
        if let Some(colon_pos) = find_modifier_colon(expr) {
            let literal_val = &expr[colon_pos + 1..];
            return Some(literal_val.to_string());
        }
    }

    // Find the colon separating modifier spec from the variable/format
    if let Some(colon_pos) = find_modifier_colon(expr) {
        let mod_spec = &expr[..colon_pos];
        let target = &expr[colon_pos + 1..];

        // Parse modifier chain (separated by ';')
        let modifiers = parse_modifier_chain(mod_spec);
        if modifiers.is_empty() {
            return None;
        }

        // First, check if the first modifier is one that takes the target as a
        // format to expand (e.g. comparisons, match, math — where the target is
        // "arg1,arg2" not a variable).
        let needs_raw_target = modifiers.iter().any(|m| matches!(m,
            Modifier::MathExpr { .. } | Modifier::Match { .. }
        ));

        let mut value = if needs_raw_target {
            // Expand each comma-separated part individually
            let parts = split_at_depth0(target, b',');
            parts.iter()
                .map(|p| expand_var_or_format(p, app, win_idx))
                .collect::<Vec<_>>()
                .join(",")
        } else {
            expand_var_or_format(target, app, win_idx)
        };

        // Apply modifiers in order
        for m in &modifiers {
            value = apply_modifier(m, &value, app, win_idx);
        }

        Some(value)
    } else {
        // No colon found — treat as plain variable
        None
    }
}

/// Find the colon that separates modifiers from the target, at brace depth 0.
pub(crate) fn find_modifier_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut depth = 0usize;

    while i < len {
        let b = bytes[i];
        if b == b'#' && i + 1 < len && bytes[i + 1] == b'{' {
            depth += 1;
            i += 2;
            continue;
        }
        if b == b'}' && depth > 0 {
            depth -= 1;
            i += 1;
            continue;
        }
        if b == b':' && depth == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parsed modifier representation.
#[derive(Debug, Clone)]
pub(crate) enum Modifier {
    Time,
    Basename,
    Dirname,
    Expand,
    ExpandTime,
    Quote,
    Substitute { pattern: String, replacement: String, case_insensitive: bool },
    Trim(i32),
    TrimWithMarker(i32, String),
    Pad(i32),
    MathExpr { op: char, floating: bool, decimals: u32 },
    Match { regex: bool, case_insensitive: bool },
    SearchContent { _regex: bool, _case_insensitive: bool },
    Width,
}

/// Parse one modifier segment.
pub(crate) fn parse_single_modifier(spec: &str) -> Option<Modifier> {
    if spec.is_empty() { return None; }
    let first = spec.as_bytes()[0] as char;
    let rest = &spec[1..];

    match first {
        't' => Some(Modifier::Time),
        'b' => Some(Modifier::Basename),
        'd' => Some(Modifier::Dirname),
        'E' => Some(Modifier::Expand),
        'T' => Some(Modifier::ExpandTime),
        'q' => Some(Modifier::Quote),
        'w' => Some(Modifier::Width),
        '=' => {
            if rest.is_empty() { return Some(Modifier::Trim(0)); }
            let sep = rest.as_bytes()[0];
            if sep == b'/' || sep == b'|' {
                let sep_ch = sep as char;
                let inner = &rest[1..];
                let parts: Vec<&str> = inner.splitn(2, sep_ch).collect();
                let n: i32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let marker = parts.get(1).unwrap_or(&"").to_string();
                Some(Modifier::TrimWithMarker(n, marker))
            } else {
                let n: i32 = rest.parse().unwrap_or(0);
                Some(Modifier::Trim(n))
            }
        }
        'p' => {
            let n: i32 = rest.parse().unwrap_or(0);
            Some(Modifier::Pad(n))
        }
        's' => {
            if rest.is_empty() { return None; }
            let sep = rest.as_bytes()[0] as char;
            let inner = &rest[1..];
            let parts: Vec<&str> = inner.splitn(3, sep).collect();
            let pattern = parts.first().unwrap_or(&"").to_string();
            let replacement = parts.get(1).unwrap_or(&"").to_string();
            let flags = parts.get(2).unwrap_or(&"");
            Some(Modifier::Substitute {
                pattern,
                replacement,
                case_insensitive: flags.contains('i'),
            })
        }
        'e' => {
            if rest.is_empty() { return None; }
            let sep = rest.as_bytes()[0] as char;
            let inner = &rest[1..];
            let parts: Vec<&str> = inner.splitn(3, sep).collect();
            let op = parts.first().and_then(|s| s.chars().next()).unwrap_or('+');
            let flags = parts.get(1).unwrap_or(&"");
            let floating = flags.contains('f');
            let decimals: u32 = parts.get(2).and_then(|s| s.parse().ok())
                .unwrap_or(if floating { 2 } else { 0 });
            Some(Modifier::MathExpr { op, floating, decimals })
        }
        'm' => {
            let regex = rest.contains('r');
            let ci = rest.contains('i');
            Some(Modifier::Match { regex, case_insensitive: ci })
        }
        'C' => {
            let regex = rest.contains('r');
            let ci = rest.contains('i');
            Some(Modifier::SearchContent { _regex: regex, _case_insensitive: ci })
        }
        _ => None,
    }
}

/// Apply a modifier to a value.
pub(crate) fn apply_modifier(m: &Modifier, value: &str, app: &AppState, win_idx: usize) -> String {
    match m {
        Modifier::Time => {
            if let Ok(ts) = value.parse::<i64>() {
                if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
                    let local: chrono::DateTime<chrono::Local> = dt.into();
                    return local.format("%a %b %e %H:%M:%S %Y").to_string();
                }
            }
            value.to_string()
        }
        Modifier::Basename => {
            std::path::Path::new(value)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(value)
                .to_string()
        }
        Modifier::Dirname => {
            std::path::Path::new(value)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string()
        }
        Modifier::Expand => {
            expand_format_for_window(value, app, win_idx)
        }
        Modifier::ExpandTime => {
            let expanded = expand_format_for_window(value, app, win_idx);
            if expanded.contains('%') {
                use std::fmt::Write;
                let formatted = chrono::Local::now().format(&expanded);
                let mut buf = String::with_capacity(expanded.len() + 32);
                if write!(buf, "{}", formatted).is_ok() { buf } else { expanded }
            } else {
                expanded
            }
        }
        Modifier::Quote => {
            let mut out = String::with_capacity(value.len() * 2);
            for ch in value.chars() {
                match ch {
                    '(' | ')' | '[' | ']' | '{' | '}' | '$' | '\\' | '\'' | '"'
                    | '`' | '!' | '#' | '&' | '|' | ';' | '<' | '>' | ' ' | '\t' | '\n' => {
                        out.push('\\');
                        out.push(ch);
                    }
                    _ => out.push(ch),
                }
            }
            out
        }
        Modifier::Trim(n) => {
            let n = *n;
            if n == 0 { return value.to_string(); }
            let chars: Vec<char> = value.chars().collect();
            if n > 0 {
                let len = n as usize;
                if chars.len() > len { chars[..len].iter().collect() }
                else { value.to_string() }
            } else {
                let len = (-n) as usize;
                if chars.len() > len { chars[chars.len() - len..].iter().collect() }
                else { value.to_string() }
            }
        }
        Modifier::TrimWithMarker(n, marker) => {
            let n = *n;
            if n == 0 { return value.to_string(); }
            let chars: Vec<char> = value.chars().collect();
            if n > 0 {
                let len = n as usize;
                if chars.len() > len {
                    let mut trimmed: String = chars[..len].iter().collect();
                    trimmed.push_str(marker);
                    trimmed
                } else { value.to_string() }
            } else {
                let len = (-n) as usize;
                if chars.len() > len {
                    let mut trimmed = marker.clone();
                    trimmed.extend(chars[chars.len() - len..].iter());
                    trimmed
                } else { value.to_string() }
            }
        }
        Modifier::Pad(n) => {
            let n = *n;
            let abs_n = n.unsigned_abs() as usize;
            let chars_len = value.chars().count();
            if chars_len >= abs_n { return value.to_string(); }
            let pad = abs_n - chars_len;
            let spaces: String = " ".repeat(pad);
            if n > 0 { format!("{}{}", value, spaces) }
            else { format!("{}{}", spaces, value) }
        }
        Modifier::Substitute { pattern, replacement, case_insensitive } => {
            let re_pattern = if *case_insensitive {
                format!("(?i){}", pattern)
            } else {
                pattern.clone()
            };
            match regex::Regex::new(&re_pattern) {
                Ok(re) => re.replace(value, replacement.as_str()).to_string(),
                Err(_) => value.to_string(),
            }
        }
        Modifier::MathExpr { op, floating, decimals } => {
            let parts = split_at_depth0(value, b',');
            if parts.len() < 2 { return "0".into(); }
            if *floating {
                let a: f64 = parts[0].parse().unwrap_or(0.0);
                let b: f64 = parts[1].parse().unwrap_or(0.0);
                let r = match op {
                    '+' => a + b, '-' => a - b, '*' => a * b,
                    '/' => if b != 0.0 { a / b } else { 0.0 },
                    'm' => if b != 0.0 { a % b } else { 0.0 },
                    _ => 0.0,
                };
                format!("{:.prec$}", r, prec = *decimals as usize)
            } else {
                let a: i64 = parts[0].parse().unwrap_or(0);
                let b: i64 = parts[1].parse().unwrap_or(0);
                let r = match op {
                    '+' => a + b, '-' => a - b, '*' => a * b,
                    '/' => if b != 0 { a / b } else { 0 },
                    'm' => if b != 0 { a % b } else { 0 },
                    _ => 0,
                };
                if *decimals > 0 {
                    format!("{:.prec$}", r as f64, prec = *decimals as usize)
                } else { r.to_string() }
            }
        }
        Modifier::Match { regex, case_insensitive } => {
            let parts = split_at_depth0(value, b',');
            if parts.len() < 2 { return "0".into(); }
            let pattern = &parts[0];
            let subject = &parts[1];
            if *regex {
                let re_pat = if *case_insensitive { format!("(?i){}", pattern) }
                    else { pattern.to_string() };
                match regex::Regex::new(&re_pat) {
                    Ok(re) => if re.is_match(subject) { "1".into() } else { "0".into() },
                    Err(_) => "0".into(),
                }
            } else {
                if glob_match(pattern, subject, *case_insensitive) { "1".into() }
                else { "0".into() }
            }
        }
        Modifier::SearchContent { _regex, _case_insensitive } => {
            // #{C:pattern} — Search for pattern in pane content, return line number or empty
            let pattern = value;
            if pattern.is_empty() { return String::new(); }
            if let Some(w) = app.windows.get(win_idx) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let re_result = if *_regex {
                            let pat = if *_case_insensitive { format!("(?i){}", pattern) } else { pattern.to_string() };
                            regex::Regex::new(&pat).ok()
                        } else {
                            let escaped = regex::escape(pattern);
                            let pat = if *_case_insensitive { format!("(?i){}", escaped) } else { escaped };
                            regex::Regex::new(&pat).ok()
                        };
                        if let Some(re) = re_result {
                            for r in 0..p.last_rows {
                                let mut row_text = String::with_capacity(p.last_cols as usize);
                                for c in 0..p.last_cols {
                                    if let Some(cell) = screen.cell(r, c) {
                                        let t = cell.contents();
                                        if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                                    } else { row_text.push(' '); }
                                }
                                if re.is_match(&row_text) {
                                    return r.to_string();
                                }
                            }
                        }
                    }
                }
            }
            String::new()
        }
        Modifier::Width => {
            value.chars().count().to_string()
        }
    }
}
