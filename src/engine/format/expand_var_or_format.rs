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

/// Expand something that could be a variable name or a format string.
pub(crate) fn expand_var_or_format(target: &str, app: &AppState, win_idx: usize) -> String {
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

pub(crate) fn lookup_option(name: &str, app: &AppState) -> Option<String> {
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
pub(crate) fn try_comparison_op(expr: &str, app: &AppState, win_idx: usize) -> Option<String> {
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

pub(crate) fn expand_boolean_or(body: &str, app: &AppState, win_idx: usize) -> String {
    let parts = split_at_depth0(body, b',');
    for part in &parts {
        let val = expand_var_or_format(part, app, win_idx);
        if is_truthy(&val) { return "1".into(); }
    }
    "0".into()
}

pub(crate) fn expand_boolean_and(body: &str, app: &AppState, win_idx: usize) -> String {
    let parts = split_at_depth0(body, b',');
    for part in &parts {
        let val = expand_var_or_format(part, app, win_idx);
        if !is_truthy(&val) { return "0".into(); }
    }
    "1".into()
}

#[inline]
pub(crate) fn is_truthy(s: &str) -> bool {
    !s.is_empty() && s != "0" && s != "off" && s != "no"
}

// ─────────────────── conditional ─────────────────────────────────

pub(crate) fn expand_conditional(body: &str, app: &AppState, win_idx: usize) -> String {
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

pub(crate) fn find_comparison_in_cond(cond: &str) -> Option<(&str, &str, &str)> {
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
