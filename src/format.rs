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
thread_local! {
    static PANE_POS_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    static BUFFER_IDX_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Set the buffer index for per-buffer format expansion in list-buffers -F.
pub fn set_buffer_idx_override(idx: Option<usize>) {
    BUFFER_IDX_OVERRIDE.set(idx);
}

// ─────────────────── tmux window_layout generation ────────────────────

/// Generate a tmux-compatible window_layout string from the pane tree.
/// Format: `<checksum>,<layout_body>`
/// Body examples:
///   Single pane:  `80x24,0,0,0`
///   Horiz split:  `80x24,0,0{40x24,0,0,0,39x24,41,0,1}`
///   Vert split:   `80x24,0,0[80x12,0,0,0,80x11,0,13,1]`
pub fn generate_window_layout(node: &Node, area: ratatui::prelude::Rect) -> String {
    let body = layout_node(node, area);
    let checksum = tmux_layout_checksum(&body);
    format!("{:04x},{}", checksum, body)
}

fn layout_node(node: &Node, area: ratatui::prelude::Rect) -> String {
    match node {
        Node::Leaf(pane) => {
            // WxH,X,Y,pane_id
            format!("{}x{},{},{},{}", area.width, area.height, area.x, area.y, pane.id)
        }
        Node::Split { kind, sizes, children } => {
            let is_horizontal = matches!(*kind, LayoutKind::Horizontal);
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![(100 / children.len().max(1)) as u16; children.len()]
            };
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
            
            let (open, close) = if is_horizontal { ('{', '}') } else { ('[', ']') };
            
            let mut inner = String::new();
            for (i, child) in children.iter().enumerate() {
                if i > 0 { inner.push(','); }
                if i < rects.len() {
                    inner.push_str(&layout_node(child, rects[i]));
                }
            }
            
            format!("{}x{},{},{}{}{}{}", area.width, area.height, area.x, area.y, open, inner, close)
        }
    }
}

/// Compute tmux layout checksum (16-bit CSUM as used by tmux src/layout-custom.c).
fn tmux_layout_checksum(layout: &str) -> u16 {
    let mut csum: u16 = 0;
    for &b in layout.as_bytes() {
        csum = (csum >> 1) | ((csum & 1) << 15); // rotate right 1 bit
        csum = csum.wrapping_add(b as u16);
    }
    csum
}

// ─────────────────────────── public API ───────────────────────────

/// Expand tmux format strings for the active window.
#[inline]
pub fn expand_format(fmt: &str, app: &AppState) -> String {
    expand_format_for_window(fmt, app, app.active_idx)
}

/// Expand tmux format strings for a specific window index.
pub fn expand_format_for_window(fmt: &str, app: &AppState, win_idx: usize) -> String {
    let mut result = String::with_capacity(fmt.len() * 2);
    let bytes = fmt.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // Whether the original format contains strftime %-sequences.
    // If so, we need to escape '%' in expanded variable content so chrono
    // only interprets the real strftime codes from the original format.
    let has_strftime = fmt.contains('%');

    while i < len {
        if bytes[i] == b'#' && i + 1 < len {
            if bytes[i + 1] == b'{' {
                // #{...} expression
                if let Some(close) = find_matching_brace(fmt, i + 2) {
                    let inner = &fmt[i + 2..close];
                    let expanded = expand_expression(inner, app, win_idx);
                    if has_strftime {
                        result.push_str(&escape_strftime_percent(&expanded));
                    } else {
                        result.push_str(&expanded);
                    }
                    i = close + 1;
                    continue;
                }
            }
            if bytes[i + 1] == b'(' {
                // #(command) — shell command execution (tmux compat)
                if let Some(end) = fmt[i + 2..].find(')') {
                    let cmd = &fmt[i + 2..i + 2 + end];
                    let output = run_shell_command(cmd);
                    if has_strftime {
                        result.push_str(&escape_strftime_percent(&output));
                    } else {
                        result.push_str(&output);
                    }
                    i = i + 2 + end + 1;
                    continue;
                }
            }
            if bytes[i + 1] == b',' {
                // Escaped comma inside conditional branches
                result.push(',');
                i += 2;
                continue;
            }
            // Shorthand #X
            match bytes[i + 1] {
                b'S' => {
                    if has_strftime {
                        result.push_str(&escape_strftime_percent(&app.session_name));
                    } else {
                        result.push_str(&app.session_name);
                    }
                    i += 2; continue;
                }
                b'I' => {
                    let n = if win_idx < app.windows.len() { win_idx + app.window_base_index } else { 0 };
                    result.push_str(&n.to_string());
                    i += 2; continue;
                }
                b'W' => {
                    if let Some(w) = app.windows.get(win_idx) {
                        if has_strftime {
                            result.push_str(&escape_strftime_percent(&w.name));
                        } else {
                            result.push_str(&w.name);
                        }
                    }
                    i += 2; continue;
                }
                b'T' => {
                    if let Some(w) = app.windows.get(win_idx) {
                        let title = active_pane(&w.root, &w.active_path)
                            .map(|p| &p.title[..])
                            .filter(|t| !t.is_empty())
                            .unwrap_or("");
                        let title = if title.is_empty() { hostname_cached() } else { title.to_string() };
                        if has_strftime {
                            result.push_str(&escape_strftime_percent(&title));
                        } else {
                            result.push_str(&title);
                        }
                    }
                    i += 2; continue;
                }
                b'P' => {
                    if let Some(w) = app.windows.get(win_idx) {
                        let active_id = get_active_pane_id(&w.root, &w.active_path).unwrap_or(0);
                        let pos = crate::tree::get_pane_position_in_window(&w.root, active_id).unwrap_or(0);
                        result.push_str(&(pos + app.pane_base_index).to_string());
                    }
                    i += 2; continue;
                }
                b'F' => {
                    if win_idx == app.active_idx { result.push('*'); }
                    else if win_idx == app.last_window_idx { result.push('-'); }
                    i += 2; continue;
                }
                b'H' | b'h' => {
                    if has_strftime {
                        result.push_str(&escape_strftime_percent(&hostname_cached()));
                    } else {
                        result.push_str(&hostname_cached());
                    }
                    i += 2; continue;
                }
                b'D' => {
                    // tmux: #D = unique pane id (like %0, %1)
                    if let Some(w) = app.windows.get(win_idx) {
                        let active_id = get_active_pane_id(&w.root, &w.active_path).unwrap_or(0);
                        if has_strftime {
                            // Escape the '%' so chrono doesn't misinterpret %0, %1, etc.
                            result.push_str(&format!("%%{}", active_id));
                        } else {
                            result.push_str(&format!("%{}", active_id));
                        }
                    }
                    i += 2; continue;
                }
                b'#' => { result.push('#'); i += 2; continue; }
                _ => {}
            }
        }
        // Advance by full UTF-8 character (not single byte) to preserve
        // multi-byte chars like ▶ (U+25B6, 3 bytes) and ◀ (U+25C0).
        if let Some(ch) = fmt[i..].chars().next() {
            result.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    // Expand strftime %-sequences only if the ORIGINAL format contained '%'
    if has_strftime && result.contains('%') {
        // Use write! to catch chrono format errors instead of panicking.
        // Expanded variable content has '%' escaped to '%%' above, so chrono
        // will only interpret the real strftime codes from the original format.
        use std::fmt::Write;
        let formatted = chrono::Local::now().format(&result);
        let mut buf = String::with_capacity(result.len() + 32);
        if write!(buf, "{}", formatted).is_ok() {
            result = buf;
        }
        // On error, keep the pre-strftime result as-is
    }
    result
}

/// Execute a shell command and return its stdout (trimmed).
/// Used for `#(command)` expansion (tmux compatibility).
/// Caches results for the lifetime of a single format expansion cycle to
/// avoid repeated subprocess spawning on every refresh.
fn run_shell_command(cmd: &str) -> String {
    use std::process::Command;
    use crate::platform::HideWindowCommandExt;
    let output = if cfg!(windows) {
        Command::new("cmd").args(["/C", cmd]).hide_window().output()
    } else {
        Command::new("sh").args(["-c", cmd]).output()
    };
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// Escape '%' to '%%' in expanded variable content so chrono's strftime
/// doesn't misinterpret user content (pane titles, pane IDs, etc.) as
/// format specifiers.
#[inline]
fn escape_strftime_percent(s: &str) -> String {
    if s.contains('%') {
        s.replace('%', "%%")
    } else {
        s.to_string()
    }
}

/// Expand format for a specific pane (used by list-panes -F, loops, etc).
pub fn expand_format_for_pane(
    fmt: &str,
    app: &AppState,
    win_idx: usize,
    pane_pos: usize,
) -> String {
    PANE_POS_OVERRIDE.set(Some(pane_pos));
    let result = expand_format_for_window(fmt, app, win_idx);
    PANE_POS_OVERRIDE.set(None);
    result
}

// ─────────────────── expression dispatcher ───────────────────────

/// Expand a `#{...}` expression (the content between `#{` and `}`).
fn expand_expression(expr: &str, app: &AppState, win_idx: usize) -> String {
    if expr.is_empty() {
        return String::new();
    }

    let first = expr.as_bytes()[0];

    // Conditional: #{?cond,true,false}
    if first == b'?' {
        return expand_conditional(&expr[1..], app, win_idx);
    }

    // Comparison operators at top level: #{==:fmt,fmt}, #{!=:...}, #{<:...}, etc.
    if let Some(val) = try_comparison_op(expr, app, win_idx) {
        return val;
    }

    // Boolean: #{||:a,b} and #{&&:a,b}
    if let Some(rest) = expr.strip_prefix("||:") {
        return expand_boolean_or(rest, app, win_idx);
    }
    if let Some(rest) = expr.strip_prefix("&&:") {
        return expand_boolean_and(rest, app, win_idx);
    }

    // Loop expansion: #{W:format} = iterate windows, #{P:format} = iterate panes, #{S:format} = iterate sessions
    if expr.len() >= 3 && expr.as_bytes()[1] == b':' {
        match first {
            b'W' => {
                // #{W:fmt} — expand fmt once per window, join with spaces
                // #{W:fmt,current_fmt} — use fmt for non-active, current_fmt for active window
                let inner_fmt = &expr[2..];
                let args = split_at_depth0(inner_fmt, b',');
                let (normal_fmt, current_fmt) = if args.len() >= 2 {
                    (args[0].as_str(), args[1].as_str())
                } else {
                    (inner_fmt, inner_fmt)
                };
                let two_arg = args.len() >= 2;
                let mut parts = Vec::new();
                for wi in 0..app.windows.len() {
                    let fmt = if wi == app.active_idx { current_fmt } else { normal_fmt };
                    parts.push(expand_format_for_window(fmt, app, wi));
                }
                // Two-argument form joins without separator (user controls layout),
                // single-argument form joins with spaces (backward compatible).
                let sep = if two_arg { "" } else { " " };
                return parts.join(sep);
            }
            b'P' => {
                // #{P:fmt} — expand fmt once per pane in the current window
                let inner_fmt = &expr[2..];
                let mut parts = Vec::new();
                if let Some(win) = app.windows.get(win_idx) {
                    let mut pane_ids = Vec::new();
                    collect_pane_ids(&win.root, &mut pane_ids);
                    for (pos, _pid) in pane_ids.iter().enumerate() {
                        PANE_POS_OVERRIDE.set(Some(pos));
                        parts.push(expand_format_for_window(inner_fmt, app, win_idx));
                        PANE_POS_OVERRIDE.set(None);
                    }
                }
                return parts.join(" ");
            }
            b'S' => {
                // #{S:fmt} — expand fmt once per session (single session in psmux)
                let inner_fmt = &expr[2..];
                return expand_format_for_window(inner_fmt, app, win_idx);
            }
            _ => {}
        }
    }

    // Modifier chain: check if there's a modifier prefix
    if let Some(result) = try_expand_modifier_chain(expr, app, win_idx) {
        return result;
    }

    // Plain variable or option name
    expand_var(expr, app, win_idx)
}

// ─────────────────── modifier chain parsing ──────────────────────

/// Try to parse and apply modifier chain(s). Returns None if expr is a plain variable.
fn try_expand_modifier_chain(expr: &str, app: &AppState, win_idx: usize) -> Option<String> {
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
fn find_modifier_colon(s: &str) -> Option<usize> {
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
enum Modifier {
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

/// Parse a modifier chain string (e.g. "s|foo|bar|;=5" ) into modifiers.
fn parse_modifier_chain(spec: &str) -> Vec<Modifier> {
    let mut modifiers = Vec::new();
    let parts = split_at_depth0(spec, b';');
    for part in &parts {
        if let Some(m) = parse_single_modifier(part) {
            modifiers.push(m);
        }
    }
    modifiers
}

/// Parse one modifier segment.
fn parse_single_modifier(spec: &str) -> Option<Modifier> {
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
fn apply_modifier(m: &Modifier, value: &str, app: &AppState, win_idx: usize) -> String {
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

/// Expand something that could be a variable name or a format string.
fn expand_var_or_format(target: &str, app: &AppState, win_idx: usize) -> String {
    if target.contains("#{") {
        expand_format_for_window(target, app, win_idx)
    } else {
        // If it looks like a plain number or is empty, return as literal
        if target.is_empty() || target.parse::<f64>().is_ok() {
            return target.to_string();
        }
        let val = expand_var(target, app, win_idx);
        if val.is_empty() && !target.is_empty() {
            // Try as option
            if let Some(opt_val) = lookup_option(target, app) {
                return opt_val;
            }
            // Not a known variable — return as literal
            return target.to_string();
        }
        val
    }
}

/// Look up a tmux option by name.
/// Public wrapper for lookup_option so config.rs can use it for -o flag check.
pub fn lookup_option_pub(name: &str, app: &AppState) -> Option<String> {
    lookup_option(name, app)
}

fn lookup_option(name: &str, app: &AppState) -> Option<String> {
    if name.starts_with('@') {
        return app.user_options.get(name).cloned();
    }
    match name {
        "status-left" => Some(app.status_left.clone()),
        "status-right" => Some(app.status_right.clone()),
        "status" => Some(if app.status_visible { "on".into() } else { "off".into() }),
        "status-position" => Some(app.status_position.clone()),
        "status-style" => Some(app.status_style.clone()),
        "prefix" => Some(format_key_binding(&app.prefix_key)),
        "prefix2" => Some(app.prefix2_key.as_ref().map(|k| format_key_binding(k)).unwrap_or_else(|| "none".to_string())),
        "base-index" => Some(app.window_base_index.to_string()),
        "pane-base-index" => Some(app.pane_base_index.to_string()),
        "escape-time" => Some(app.escape_time_ms.to_string()),
        "history-limit" => Some(app.history_limit.to_string()),
        "mouse" => Some(if app.mouse_enabled { "on".into() } else { "off".into() }),
        "scroll-enter-copy-mode" => Some(if app.scroll_enter_copy_mode { "on".into() } else { "off".into() }),
        "mode-keys" => Some(app.mode_keys.clone()),
        "default-command" | "default-shell" => Some(if app.default_shell.is_empty() {
            crate::pane::cached_shell().unwrap_or("pwsh.exe").to_string()
        } else {
            app.default_shell.clone()
        }),
        "word-separators" => Some(app.word_separators.clone()),
        "renumber-windows" => Some(if app.renumber_windows { "on".into() } else { "off".into() }),
        "automatic-rename" => Some(if app.automatic_rename { "on".into() } else { "off".into() }),
        "monitor-activity" => Some(if app.monitor_activity { "on".into() } else { "off".into() }),
        "remain-on-exit" => Some(if app.remain_on_exit { "on".into() } else { "off".into() }),
        "destroy-unattached" => Some(if app.destroy_unattached { "on".into() } else { "off".into() }),
        "exit-empty" => Some(if app.exit_empty { "on".into() } else { "off".into() }),
        "set-titles" => Some(if app.set_titles { "on".into() } else { "off".into() }),
        "set-titles-string" => Some(app.set_titles_string.clone()),
        "pane-border-style" => Some(app.pane_border_style.clone()),
        "pane-active-border-style" => Some(app.pane_active_border_style.clone()),
        "pane-border-hover-style" => Some(app.pane_border_hover_style.clone()),
        "window-status-format" => Some(app.window_status_format.clone()),
        "window-status-current-format" => Some(app.window_status_current_format.clone()),
        "window-status-separator" => Some(app.window_status_separator.clone()),
        "window-status-style" => Some(app.window_status_style.clone()),
        "window-status-current-style" => Some(app.window_status_current_style.clone()),
        "window-status-activity-style" => Some(app.window_status_activity_style.clone()),
        "window-status-bell-style" => Some(app.window_status_bell_style.clone()),
        "window-status-last-style" => Some(app.window_status_last_style.clone()),
        "message-style" => Some(app.message_style.clone()),
        "message-command-style" => Some(app.message_command_style.clone()),
        "mode-style" => Some(app.mode_style.clone()),
        "status-left-style" => Some(app.status_left_style.clone()),
        "status-right-style" => Some(app.status_right_style.clone()),
        "status-interval" => Some(app.status_interval.to_string()),
        "status-justify" => Some(app.status_justify.clone()),
        "display-time" => Some(app.display_time_ms.to_string()),
        "display-panes-time" => Some(app.display_panes_time_ms.to_string()),
        "focus-events" => Some(if app.focus_events { "on".into() } else { "off".into() }),
        "aggressive-resize" => Some(if app.aggressive_resize { "on".into() } else { "off".into() }),
        "synchronize-panes" => Some(if app.sync_input { "on".into() } else { "off".into() }),
        "monitor-silence" => Some(app.monitor_silence.to_string()),
        "bell-action" => Some(app.bell_action.clone()),
        "visual-bell" => Some(if app.visual_bell { "on".into() } else { "off".into() }),
        "claude-code-fix-tty" => Some(if app.claude_code_fix_tty { "on".into() } else { "off".into() }),
        "claude-code-force-interactive" => Some(if app.claude_code_force_interactive { "on".into() } else { "off".into() }),
        _ => {
            // Try user_options first (plugins store @cpu_percentage etc.),
            // then environment, then @name fallback for plugin compat
            // (format strings use #{cpu_percentage} without the @ prefix).
            app.user_options.get(name).cloned()
                .or_else(|| app.environment.get(name).cloned())
                .or_else(|| {
                    if !name.starts_with('@') {
                        app.user_options.get(&format!("@{}", name)).cloned()
                    } else {
                        None
                    }
                })
        }
    }
}

// ─────────────────── comparison operators ─────────────────────────

/// Try to match a comparison operator at the start of expr.
fn try_comparison_op(expr: &str, app: &AppState, win_idx: usize) -> Option<String> {
    let ops: &[(&str, fn(&str, &str) -> bool)] = &[
        ("<=:", |a, b| a <= b),
        (">=:", |a, b| a >= b),
        ("==:", |a, b| a == b),
        ("!=:", |a, b| a != b),
        ("<:", |a, b| a < b),
        (">:", |a, b| a > b),
    ];

    for &(prefix, cmp_fn) in ops {
        if let Some(rest) = expr.strip_prefix(prefix) {
            let parts = split_at_depth0(rest, b',');
            if parts.len() < 2 { return Some("0".into()); }
            let lhs = expand_var_or_format(&parts[0], app, win_idx);
            let rhs = expand_var_or_format(&parts[1], app, win_idx);
            return Some(if cmp_fn(&lhs, &rhs) { "1".into() } else { "0".into() });
        }
    }
    None
}

fn expand_boolean_or(body: &str, app: &AppState, win_idx: usize) -> String {
    let parts = split_at_depth0(body, b',');
    for part in &parts {
        let val = expand_var_or_format(part, app, win_idx);
        if is_truthy(&val) { return "1".into(); }
    }
    "0".into()
}

fn expand_boolean_and(body: &str, app: &AppState, win_idx: usize) -> String {
    let parts = split_at_depth0(body, b',');
    for part in &parts {
        let val = expand_var_or_format(part, app, win_idx);
        if !is_truthy(&val) { return "0".into(); }
    }
    "1".into()
}

#[inline]
fn is_truthy(s: &str) -> bool {
    !s.is_empty() && s != "0" && s != "off" && s != "no"
}

// ─────────────────── conditional ─────────────────────────────────

fn expand_conditional(body: &str, app: &AppState, win_idx: usize) -> String {
    let (cond, true_branch, false_branch) = split_conditional(body);

    let is_true = if let Some((lhs_str, op, rhs_str)) = find_comparison_in_cond(&cond) {
        // Expand sides as format strings (plain text passes through, #{var} expands)
        let lhs = expand_format_for_window(lhs_str, app, win_idx);
        let rhs = expand_format_for_window(rhs_str, app, win_idx);
        match op {
            "==" => lhs == rhs,
            "!=" => lhs != rhs,
            "<" => lhs < rhs,
            ">" => lhs > rhs,
            "<=" => lhs <= rhs,
            ">=" => lhs >= rhs,
            _ => false,
        }
    } else {
        // If cond already contains format markers (#), expand it directly.
        // Otherwise wrap as #{variable_name} to resolve the variable.
        let cond_val = if cond.contains('#') {
            expand_format_for_window(&cond, app, win_idx)
        } else {
            expand_format_for_window(&format!("#{{{}}}", cond), app, win_idx)
        };
        is_truthy(&cond_val)
    };

    if is_true {
        expand_format_for_window(&true_branch, app, win_idx)
    } else {
        expand_format_for_window(&false_branch, app, win_idx)
    }
}

fn find_comparison_in_cond(cond: &str) -> Option<(&str, &str, &str)> {
    let ops = ["<=", ">=", "==", "!=", "<", ">"];
    for op in ops {
        // Scan for op outside of nested #{...} blocks
        let bytes = cond.as_bytes();
        let op_bytes = op.as_bytes();
        let mut i = 0;
        let mut depth = 0usize;
        while i + op_bytes.len() <= bytes.len() {
            if i + 1 < bytes.len() && bytes[i] == b'#' && bytes[i + 1] == b'{' {
                depth += 1;
                i += 2;
                continue;
            }
            if bytes[i] == b'}' && depth > 0 {
                depth -= 1;
                i += 1;
                continue;
            }
            if depth == 0 && &bytes[i..i + op_bytes.len()] == op_bytes {
                let lhs = &cond[..i];
                let rhs = &cond[i + op.len()..];
                if !lhs.is_empty() || !rhs.is_empty() {
                    return Some((lhs, op, rhs));
                }
            }
            i += 1;
        }
    }
    None
}

// ─────────────────── variable expansion ──────────────────────────

/// Expand a named variable.
pub fn expand_var(var: &str, app: &AppState, win_idx: usize) -> String {
    let win = match app.windows.get(win_idx) {
        Some(w) => w,
        None => {
            // Even without a window, some variables still resolve
            return match var {
                "session_name" => app.session_name.clone(),
                "session_windows" => app.windows.len().to_string(),
                "session_id" => format!("${}", app.session_id),
                "pid" | "server_pid" => std::process::id().to_string(),
                "version" => VERSION.to_string(),
                "host" | "hostname" => hostname_cached(),
                "host_short" => { let h = hostname_cached(); h.split('.').next().unwrap_or(&h).to_string() }
                _ => {
                    if let Some(v) = lookup_option(var, app) { v } else { String::new() }
                }
            };
        }
    };
    // Resolve the target pane for format expansion. When PANE_POS_OVERRIDE is set
    // (during list-panes iteration), use that positional pane instead of the active pane.
    let (fmt_pane_pos, fmt_pane_is_active) = {
        let override_pos = PANE_POS_OVERRIDE.get();
        if let Some(pos) = override_pos {
            let active_id = get_active_pane_id(&win.root, &win.active_path);
            let is_active = crate::tree::get_nth_pane(&win.root, pos)
                .map(|p| Some(p.id) == active_id).unwrap_or(false);
            (pos, is_active)
        } else {
            let active_id = get_active_pane_id(&win.root, &win.active_path).unwrap_or(0);
            let pos = crate::tree::get_pane_position_in_window(&win.root, active_id).unwrap_or(0);
            (pos, true)
        }
    };
    // Helper closure to get the target pane reference
    let target_pane = || -> Option<&Pane> {
        crate::tree::get_nth_pane(&win.root, fmt_pane_pos)
    };
    match var {
        // ── Session ──
        "session_name" => app.session_name.clone(),
        "session_attached" => if app.attached_clients > 0 { "1".into() } else { "0".into() },
        "session_windows" => app.windows.len().to_string(),
        "session_id" => format!("${}", app.session_id),
        "session_created" => app.created_at.timestamp().to_string(),
        "session_created_string" => app.created_at.format("%a %b %e %H:%M:%S %Y").to_string(),
        "session_activity" | "session_last_attached" => app.created_at.timestamp().to_string(),
        "session_activity_string" => app.created_at.format("%a %b %e %H:%M:%S %Y").to_string(),
        "session_group" | "session_group_list" => app.session_group.clone().unwrap_or_default(),
        "session_alerts" | "session_stack" => String::new(),
        "session_group_attached" => {
            if app.session_group.is_some() && app.attached_clients > 0 { "1".into() } else { "0".into() }
        }
        "session_group_size" => {
            if app.session_group.is_some() { "1".into() } else { "0".into() }
        }
        "session_grouped" => if app.session_group.is_some() { "1".into() } else { "0".into() },
        "session_format" | "session_many_attached" => if app.attached_clients > 1 { "1".into() } else { "0".into() },
        "session_path" => env::var("HOME").or_else(|_| env::var("USERPROFILE")).unwrap_or_default(),

        // ── Window ──
        "window_index" => (win_idx + app.window_base_index).to_string(),
        "window_name" => win.name.clone(),
        "window_active" => if win_idx == app.active_idx { "1".into() } else { "0".into() },
        "window_panes" => count_panes(&win.root).to_string(),
        "window_flags" | "window_raw_flags" => {
            let mut f = String::new();
            if win_idx == app.active_idx { f.push('*'); }
            else if win_idx == app.last_window_idx { f.push('-'); }
            if win.zoom_saved.is_some() { f.push('Z'); }
            if win.activity_flag { f.push('#'); }
            if win.bell_flag { f.push('!'); }
            if win.silence_flag { f.push('~'); }
            f
        }
        "window_id" => format!("@{}", win.id),
        "window_activity_flag" => if win.activity_flag { "1".into() } else { "0".into() },
        "window_zoomed_flag" => if win.zoom_saved.is_some() { "1".into() } else { "0".into() },
        "window_layout" | "window_visible_layout" => generate_window_layout(&win.root, app.last_window_area),
        "window_width" => app.last_window_area.width.to_string(),
        "window_height" => app.last_window_area.height.to_string(),
        "window_format" => "1".into(),
        "window_activity" => app.created_at.timestamp().to_string(),
        "window_silence_flag" => if win.silence_flag { "1".into() } else { "0".into() },
        "window_bell_flag" => if win.bell_flag { "1".into() } else { "0".into() },
        "window_linked" => if win.linked_from.is_some() { "1".into() } else { "0".into() },
        "window_linked_sessions" => if win.linked_from.is_some() { "1".into() } else { "0".into() },
        "window_linked_sessions_list" => String::new(),
        "window_last_flag" => if win_idx == app.last_window_idx { "1".into() } else { "0".into() },
        "window_start_flag" => if win_idx == 0 { "1".into() } else { "0".into() },
        "window_end_flag" => if win_idx == app.windows.len().saturating_sub(1) { "1".into() } else { "0".into() },
        "window_bigger" => "0".into(),
        "window_cell_width" => "8".into(),
        "window_cell_height" => "16".into(),
        "window_offset_x" | "window_offset_y" | "window_stack_index" => "0".into(),

        // ── Pane ──
        "pane_index" => {
            (fmt_pane_pos + app.pane_base_index).to_string()
        }
        "pane_id" => {
            if let Some(p) = target_pane() { format!("%{}", p.id) } else { "%0".into() }
        }
        "pane_title" => {
            if let Some(p) = target_pane() {
                if !p.title.is_empty() { p.title.clone() } else { hostname_cached() }
            } else { hostname_cached() }
        }
        "pane_width" => {
            if let Some(p) = target_pane() { p.last_cols.to_string() } else { "80".into() }
        }
        "pane_height" => {
            if let Some(p) = target_pane() { p.last_rows.to_string() } else { "24".into() }
        }
        "pane_active" => if fmt_pane_is_active { "1".into() } else { "0".into() },
        "pane_current_command" => {
            if let Some(p) = target_pane() {
                if let Some(pid) = p.child_pid {
                    crate::platform::process_info::get_foreground_process_name(pid)
                        .unwrap_or_else(|| "shell".into())
                } else if !p.title.is_empty() {
                    p.title.clone()
                } else {
                    "shell".into()
                }
            } else { String::new() }
        }
        "pane_current_path" => {
            if let Some(p) = target_pane() {
                // Layer 1: PEB walk (authoritative for local processes)
                if let Some(pid) = p.child_pid {
                    if let Some(cwd) = crate::platform::process_info::get_foreground_cwd(pid) {
                        return cwd;
                    }
                }
                // Layer 2: OSC 7 path (works over SSH/WSL where PEB fails)
                if let Ok(parser) = p.term.lock() {
                    if let Some(osc_path) = parser.screen().path() {
                        return osc_path.to_string();
                    }
                }
                // Layer 3: fallback to server CWD
                std::env::current_dir()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else { String::new() }
        }
        "pane_path" => {
            // Pure OSC 7 value (tmux-compatible: only what the shell announced)
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    parser.screen().path().unwrap_or_default().to_string()
                } else { String::new() }
            } else { String::new() }
        }
        "pane_pid" => {
            if let Some(p) = target_pane() {
                p.child_pid.map(|pid| pid.to_string()).unwrap_or_default()
            } else { String::new() }
        }
        "pane_tty" => {
            if let Some(p) = target_pane() { format!("/dev/pty{}", p.id) }
            else { String::new() }
        }
        "pane_in_mode" => match app.mode {
            Mode::CopyMode | Mode::CopySearch { .. } | Mode::ClockMode => "1".into(),
            _ => "0".into(),
        },
        "pane_mode" => match app.mode {
            Mode::CopyMode | Mode::CopySearch { .. } => "copy-mode".into(),
            Mode::ClockMode => "clock-mode".into(),
            _ => String::new(),
        },
        "pane_synchronized" => if app.sync_input { "1".into() } else { "0".into() },
        "pane_dead" => {
            if let Some(p) = target_pane() {
                if p.dead { "1".into() } else { "0".into() }
            } else { "0".into() }
        }
        "pane_dead_signal" | "pane_dead_status" | "pane_dead_time" => "0".into(),
        "pane_format" => "1".into(),
        "pane_input_off"
        | "pane_pipe" | "pane_unseen_changes" => "0".into(),
        "pane_last" => {
            if let Some(p) = target_pane() {
                if !app.last_pane_path.is_empty() {
                    if let Some(last_p) = active_pane(&win.root, &app.last_pane_path) {
                        if last_p.id == p.id { return "1".into(); }
                    }
                }
            }
            "0".into()
        }
        "pane_marked" => {
            if let Some(p) = target_pane() {
                if let Some((mw, mp)) = app.marked_pane {
                    if mw == win_idx && mp == p.id { "1".into() } else { "0".into() }
                } else { "0".into() }
            } else { "0".into() }
        }
        "pane_marked_set" => {
            if app.marked_pane.is_some() { "1".into() } else { "0".into() }
        }
        "pane_left" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { rect.x.to_string() } else { "0".into() }
            } else { "0".into() }
        }
        "pane_top" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { rect.y.to_string() } else { "0".into() }
            } else { "0".into() }
        }
        "pane_right" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { (rect.x + rect.width).saturating_sub(1).to_string() } else { "79".into() }
            } else { "79".into() }
        }
        "pane_bottom" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { (rect.y + rect.height).saturating_sub(1).to_string() } else { "23".into() }
            } else { "23".into() }
        }
        "pane_at_top" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    if rect.y == app.last_window_area.y { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_at_bottom" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    let bottom = rect.y + rect.height;
                    let win_bottom = app.last_window_area.y + app.last_window_area.height;
                    if bottom >= win_bottom { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_at_left" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    if rect.x == app.last_window_area.x { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_at_right" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    let right = rect.x + rect.width;
                    let win_right = app.last_window_area.x + app.last_window_area.width;
                    if right >= win_right { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_search_string" => app.copy_search_query.clone(),
        "pane_start_command" => app.default_shell.clone(),
        "pane_start_path" | "pane_tabs" => String::new(),
        "pane_fg" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, c) = parser.screen().cursor_position();
                    if let Some(cell) = parser.screen().cell(r, c) {
                        return format_vt100_color(cell.fgcolor());
                    }
                }
            }
            "default".into()
        }
        "pane_bg" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, c) = parser.screen().cursor_position();
                    if let Some(cell) = parser.screen().cell(r, c) {
                        return format_vt100_color(cell.bgcolor());
                    }
                }
            }
            "default".into()
        }

        // ── Cursor ──
        "cursor_x" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (_, c) = parser.screen().cursor_position();
                    return c.to_string();
                }
            }
            "0".into()
        }
        "cursor_y" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, _) = parser.screen().cursor_position();
                    return r.to_string();
                }
            }
            "0".into()
        }
        "cursor_character" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, c) = parser.screen().cursor_position();
                    if let Some(cell) = parser.screen().cell(r, c) {
                        return cell.contents().to_string();
                    }
                }
            }
            String::new()
        }
        "cursor_flag" => "0".into(),

        // ── Mouse ──
        "mouse_x" => app.last_mouse_x.to_string(),
        "mouse_y" => app.last_mouse_y.to_string(),
        "mouse_line" => {
            if let Some(w) = app.windows.get(win_idx) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        // Convert screen-absolute mouse_y to pane-relative row
                        let mut rects = Vec::new();
                        crate::tree::compute_rects(&w.root, app.last_window_area, &mut rects);
                        let pane_y_offset = rects.iter()
                            .find(|(path, _)| crate::tree::get_active_pane_id_at_path(&w.root, path) == Some(p.id))
                            .map(|(_, rect)| rect.y)
                            .unwrap_or(0);
                        let row = app.last_mouse_y.saturating_sub(pane_y_offset);
                        let mut row_text = String::with_capacity(cols as usize);
                        for col in 0..cols {
                            if let Some(cell) = screen.cell(row, col) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        return row_text.trim_end().to_string();
                    }
                }
            }
            String::new()
        }
        "mouse_word" => {
            if let Some(w) = app.windows.get(win_idx) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        let mut rects = Vec::new();
                        crate::tree::compute_rects(&w.root, app.last_window_area, &mut rects);
                        let (pane_x_offset, pane_y_offset) = rects.iter()
                            .find(|(path, _)| crate::tree::get_active_pane_id_at_path(&w.root, path) == Some(p.id))
                            .map(|(_, rect)| (rect.x, rect.y))
                            .unwrap_or((0, 0));
                        let row = app.last_mouse_y.saturating_sub(pane_y_offset);
                        let col = app.last_mouse_x.saturating_sub(pane_x_offset);
                        let mut row_text = String::with_capacity(cols as usize);
                        for c in 0..cols {
                            if let Some(cell) = screen.cell(row, c) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        let chars: Vec<char> = row_text.chars().collect();
                        let ci = col as usize;
                        if ci < chars.len() && !chars[ci].is_whitespace() {
                            let seps = &app.word_separators;
                            let cls = |ch: &char| -> u8 {
                                if ch.is_whitespace() { 0 }
                                else if seps.contains(*ch) { 1 }
                                else { 2 }
                            };
                            let target = cls(&chars[ci]);
                            let mut start = ci;
                            while start > 0 && cls(&chars[start - 1]) == target { start -= 1; }
                            let mut end = ci;
                            while end + 1 < chars.len() && cls(&chars[end + 1]) == target { end += 1; }
                            return chars[start..=end].iter().collect();
                        }
                    }
                }
            }
            String::new()
        }

        // ── Copy mode ──
        "copy_cursor_x" => app.copy_pos.map(|(_, c)| c.to_string()).unwrap_or("0".into()),
        "copy_cursor_y" => app.copy_pos.map(|(r, _)| r.to_string()).unwrap_or("0".into()),
        "copy_cursor_word" => {
            // Return the word under the copy cursor
            if let (Some((r, c)), Some(w)) = (app.copy_pos, app.windows.get(win_idx)) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        let mut row_text = String::with_capacity(cols as usize);
                        for col in 0..cols {
                            if let Some(cell) = screen.cell(r, col) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        let chars: Vec<char> = row_text.chars().collect();
                        let ci = c as usize;
                        if ci < chars.len() && !chars[ci].is_whitespace() {
                            let seps = &app.word_separators;
                            let cls = |ch: &char| -> u8 {
                                if ch.is_whitespace() { 0 }
                                else if seps.contains(*ch) { 1 }
                                else { 2 }
                            };
                            let target = cls(&chars[ci]);
                            let mut start = ci;
                            while start > 0 && cls(&chars[start - 1]) == target { start -= 1; }
                            let mut end = ci;
                            while end + 1 < chars.len() && cls(&chars[end + 1]) == target { end += 1; }
                            return chars[start..=end].iter().collect();
                        }
                    }
                }
            }
            String::new()
        }
        "copy_cursor_line" => {
            // Return the line under the copy cursor
            if let (Some((r, _)), Some(w)) = (app.copy_pos, app.windows.get(win_idx)) {
                if let Some(p) = active_pane(&w.root, &w.active_path) {
                    if let Ok(parser) = p.term.lock() {
                        let screen = parser.screen();
                        let cols = p.last_cols;
                        let mut row_text = String::with_capacity(cols as usize);
                        for col in 0..cols {
                            if let Some(cell) = screen.cell(r, col) {
                                let t = cell.contents();
                                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
                            } else { row_text.push(' '); }
                        }
                        return row_text.trim_end().to_string();
                    }
                }
            }
            String::new()
        }
        "selection_present" | "selection_active" => if app.copy_anchor.is_some() { "1".into() } else { "0".into() },
        "selection_start_x" => app.copy_anchor.map(|(_, c)| c.to_string()).unwrap_or("0".into()),
        "selection_start_y" => app.copy_anchor.map(|(r, _)| r.to_string()).unwrap_or("0".into()),
        "selection_end_x" => app.copy_pos.map(|(_, c)| c.to_string()).unwrap_or("0".into()),
        "selection_end_y" => app.copy_pos.map(|(r, _)| r.to_string()).unwrap_or("0".into()),
        "search_present" => if !app.copy_search_query.is_empty() { "1".into() } else { "0".into() },
        "search_match" => {
            if !app.copy_search_matches.is_empty() {
                app.copy_search_matches.get(app.copy_search_idx)
                    .map(|_| app.copy_search_query.clone())
                    .unwrap_or_default()
            } else { String::new() }
        }
        "scroll_position" => app.copy_scroll_offset.to_string(),
        "scroll_region_upper" => "0".into(),
        "scroll_region_lower" => {
            if let Some(p) = active_pane(&win.root, &win.active_path) {
                return p.last_rows.saturating_sub(1).to_string();
            }
            "0".into()
        }

        // ── Buffer ──
        "buffer_size" => {
            let idx = BUFFER_IDX_OVERRIDE.get().unwrap_or(0);
            app.paste_buffers.get(idx).map(|b| b.len().to_string()).unwrap_or("0".into())
        }
        "buffer_sample" => {
            let idx = BUFFER_IDX_OVERRIDE.get().unwrap_or(0);
            app.paste_buffers.get(idx).map(|b| b.chars().take(50).collect::<String>()).unwrap_or_default()
        }
        "buffer_name" => {
            let idx = BUFFER_IDX_OVERRIDE.get().unwrap_or(0);
            if idx < app.paste_buffers.len() { format!("buffer{:04}", idx) } else { String::new() }
        }
        "buffer_created" => app.created_at.timestamp().to_string(),

        // ── Client ──
        "client_width" => app.last_window_area.width.to_string(),
        "client_height" => (app.last_window_area.height + if app.status_visible { 1 } else { 0 }).to_string(),
        "client_session" | "client_last_session" => app.session_name.clone(),
        "client_name" | "client_tty" => "client0".into(),
        "client_pid" => std::process::id().to_string(),
        "client_prefix" => if app.client_prefix_active || matches!(app.mode, Mode::Prefix { .. }) { "1".into() } else { "0".into() },
        "client_activity" | "client_created" => app.created_at.timestamp().to_string(),
        "client_activity_string" | "client_created_string" => app.created_at.format("%a %b %e %H:%M:%S %Y").to_string(),
        "client_control_mode" => "0".into(),
        "client_flags" => "focused".into(),
        "client_key_table" => if app.client_prefix_active || matches!(app.mode, Mode::Prefix { .. }) {
            "prefix".into()
        } else {
            match app.mode {
                Mode::CopyMode => "copy-mode-vi".into(),
                _ => "root".into(),
            }
        },
        "client_termname" | "client_termtype" => env::var("TERM").unwrap_or_else(|_| "xterm-256color".into()),
        "client_termfeatures" => "256,RGB,title".into(),
        "client_utf8" => "1".into(),
        "client_cell_width" => "8".into(),
        "client_cell_height" => "16".into(),
        "client_written" | "client_discarded" => "0".into(),

        // ── Server ──
        "host" | "hostname" => hostname_cached(),
        "host_short" => { let h = hostname_cached(); h.split('.').next().unwrap_or(&h).to_string() }
        "user" | "username" => env::var("USERNAME").or_else(|_| env::var("USER")).unwrap_or_else(|_| "unknown".into()),
        "pid" | "server_pid" => std::process::id().to_string(),
        "version" => VERSION.to_string(),
        "start_time" => app.created_at.timestamp().to_string(),
        "socket_path" => {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            format!("{}/.psmux/default", home)
        }

        // ── Options as format variables ──
        "mouse" => if app.mouse_enabled { "on".into() } else { "off".into() },
        "scroll-enter-copy-mode" => if app.scroll_enter_copy_mode { "on".into() } else { "off".into() },
        "prefix" => format_key_binding(&app.prefix_key),
        "prefix2" => app.prefix2_key.as_ref().map(|k| format_key_binding(k)).unwrap_or_else(|| "none".to_string()),
        "status" => if app.status_visible { "on".into() } else { "off".into() },
        "mode_keys" => app.mode_keys.clone(),
        "history_limit" => app.history_limit.to_string(),
        "history_size" => app.history_limit.to_string(),
        "alternate_on" => {
            if let Some(p) = active_pane(&win.root, &win.active_path) {
                if let Ok(parser) = p.term.lock() {
                    if parser.screen().alternate_screen() { return "1".into(); }
                }
            }
            "0".into()
        }
        "alternate_saved_x" | "alternate_saved_y" => "0".into(),

        // ── Misc ──
        "origin_flag" | "insert_flag" | "keypad_cursor_flag" | "keypad_flag" => "0".into(),
        "wrap_flag" => "1".into(),
        "line" | "command" | "command_list_name" | "command_list_alias" | "command_list_usage" | "config_files" => String::new(),
        "current_file" => crate::config::current_config_file(),

        // Anything else: try as option, then env
        _ => {
            if let Some(val) = lookup_option(var, app) { val }
            else { String::new() }
        }
    }
}

// ─────────────────── helper utilities ────────────────────────────

fn format_vt100_color(color: vt100::Color) -> String {
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

fn find_matching_brace(s: &str, start: usize) -> Option<usize> {
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

fn split_at_depth0(s: &str, delim: u8) -> Vec<String> {
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

fn split_conditional(s: &str) -> (String, String, String) {
    let parts = split_at_depth0(s, b',');
    match parts.len() {
        0 => (String::new(), String::new(), String::new()),
        1 => (parts[0].clone(), String::new(), String::new()),
        2 => (parts[0].clone(), parts[1].clone(), String::new()),
        _ => (parts[0].clone(), parts[1].clone(), parts[2..].join(",")),
    }
}

fn glob_match(pattern: &str, text: &str, case_insensitive: bool) -> bool {
    let p = if case_insensitive { pattern.to_lowercase() } else { pattern.to_string() };
    let t = if case_insensitive { text.to_lowercase() } else { text.to_string() };
    glob_match_impl(p.as_bytes(), t.as_bytes())
}

fn glob_match_impl(pattern: &[u8], text: &[u8]) -> bool {
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

fn collect_pane_ids(node: &Node, ids: &mut Vec<usize>) {
    match node {
        Node::Leaf(p) => ids.push(p.id),
        Node::Split { children, .. } => {
            for child in children { collect_pane_ids(child, ids); }
        }
    }
}

#[cfg(test)]
#[path = "../tests-rs/test_format.rs"]
mod tests;
