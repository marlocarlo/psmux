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

thread_local! {
    pub(crate) static PANE_POS_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    pub(crate) static BUFFER_IDX_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
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

pub(crate) fn layout_node(node: &Node, area: ratatui::prelude::Rect) -> String {
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
pub(crate) fn tmux_layout_checksum(layout: &str) -> u16 {
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
pub(crate) fn run_shell_command(cmd: &str) -> String {
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
pub(crate) fn escape_strftime_percent(s: &str) -> String {
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
pub(crate) fn expand_expression(expr: &str, app: &AppState, win_idx: usize) -> String {
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
