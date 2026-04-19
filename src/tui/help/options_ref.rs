#[allow(unused_imports)]
// ── src/help.rs ───────────────────────────────────────────────────────
// Comprehensive help / reference data for the C-b ? overlay and
// `list-keys` CLI command.  Kept as a standalone module so it does not
// bloat existing source files.
// ─────────────────────────────────────────────────────────────────────

/// Default prefix-table keybindings.
/// Each entry is `(key_string, command_string)`.
/// The overlay and `list-keys` both use this as the canonical source
/// of truth, so there is exactly *one* place to update.
use super::*;

pub(crate) const OPTIONS_REF: &[(&str, &str)] = &[
    // Key
    ("prefix",                     "C-b"),
    ("prefix2",                    "none"),
    // Behaviour
    ("escape-time",                "500"),
    ("base-index",                 "0"),
    ("pane-base-index",            "0"),
    ("history-limit",              "2000"),
    ("mouse",                      "on"),
    ("mode-keys",                  "emacs"),
    ("focus-events",               "off"),
    ("remain-on-exit",             "off"),
    ("renumber-windows",           "off"),
    ("aggressive-resize",          "off"),
    ("automatic-rename",           "on"),
    ("synchronize-panes",          "off"),
    ("set-titles",                 "off"),
    ("allow-passthrough",          "off"),
    ("default-command",            "(system shell)"),
    ("word-separators",            "\" -_@\""),
    // Display timing
    ("display-time",               "750"),
    ("display-panes-time",         "1000"),
    ("status-interval",            "15"),
    // Status bar
    ("status",                     "on"),
    ("status-position",            "bottom"),
    ("status-justify",             "left"),
    ("status-left",                "\"[#S] \""),
    ("status-right",               "\"#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y\""),
    ("status-left-length",         "10"),
    ("status-right-length",        "40"),
    ("status-style",               "bg=green,fg=black"),
    ("status-left-style",          "\"\""),
    ("status-right-style",         "\"\""),
    // Window status
    ("window-status-format",       "#I:#W#{...}"),
    ("window-status-current-format", "#I:#W#{...}"),
    ("window-status-separator",    "\" \""),
    ("window-status-style",        "\"\""),
    ("window-status-current-style","\"\""),
    ("window-status-activity-style","reverse"),
    ("window-status-bell-style",   "reverse"),
    ("window-status-last-style",   "\"\""),
    // Pane borders
    ("pane-border-style",          "\"\""),
    ("pane-active-border-style",   "fg=green"),
    ("pane-border-hover-style",     "fg=yellow"),
    // Messages / Modes
    ("message-style",              "bg=yellow,fg=black"),
    ("message-command-style",      "bg=black,fg=yellow"),
    ("mode-style",                 "bg=yellow,fg=black"),
    // Monitoring
    ("monitor-activity",           "off"),
    ("monitor-silence",            "0"),
    ("visual-activity",            "off"),
    ("visual-bell",                "off"),
    ("bell-action",                "any"),
    // Layout
    ("main-pane-width",            "0 (60% heuristic)"),
    ("main-pane-height",           "0 (60% heuristic)"),
    // Copy / Clipboard
    ("copy-command",               "\"\""),
    ("set-clipboard",              "on"),
    ("set-titles-string",          "\"\""),
    // psmux extensions
    ("cursor-style",               "\"\""),
    ("cursor-blink",               "off"),
    ("prediction-dimming",         "off"),
    ("allow-predictions",          "off"),
    ("env-shim",                   "on"),
    ("claude-code-fix-tty",        "on"),
    ("claude-code-force-interactive", "on"),
];

/// Section: format variables quick-reference.
pub fn format_vars_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── format variables (#{...}) ───────────────────────────────".into());
    for (group, vars) in FORMAT_GROUPS {
        v.push(format!("  {}:", group));
        v.push(format!("    {}", vars));
    }
    v.push(String::new());
    v.push("  Modifiers: #{=N:var} truncate, #{T:var} strftime,".into());
    v.push("    #{?test,true,false} conditional, #{==:a,b} compare,".into());
    v.push("    #{e:var} shell escape, #{b:var} basename,".into());
    v.push("    #{d:var} dirname, #{m:pat,str} match, #{s/p/r/:var} sub".into());
    v
}

pub(crate) const FORMAT_GROUPS: &[(&str, &str)] = &[
    ("Session", "session_name session_id session_windows session_attached session_created session_path ..."),
    ("Window",  "window_index window_name window_active window_panes window_flags window_id window_layout window_zoomed_flag ..."),
    ("Pane",    "pane_index pane_id pane_title pane_width pane_height pane_active pane_current_command pane_current_path pane_pid pane_dead ..."),
    ("Cursor",  "cursor_x cursor_y cursor_character cursor_flag"),
    ("Copy",    "copy_cursor_x copy_cursor_y copy_cursor_word copy_cursor_line selection_present search_present scroll_position"),
    ("Buffer",  "buffer_name buffer_size buffer_sample buffer_created"),
    ("Client",  "client_width client_height client_name client_session client_prefix client_pid client_termname ..."),
    ("Server",  "pid version host hostname host_short"),
    ("Misc",    "history_limit history_size alternate_on pane_mode pane_in_mode"),
];

/// Section: hooks reference.
pub fn hooks_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── hooks (set-hook) ───────────────────────────────────────".into());
    v.push("  after-new-session     after-new-window      after-kill-pane".into());
    v.push("  after-split-window    after-select-window    after-select-pane".into());
    v.push("  after-resize-pane     after-rename-window    after-rename-session".into());
    v.push("  after-select-layout   after-copy-mode        after-set-option".into());
    v.push("  after-bind-key        after-unbind-key       after-source".into());
    v.push("  after-swap-pane       after-swap-window      client-attached".into());
    v.push("  client-detached".into());
    v
}

/// Section: mouse bindings.
pub fn mouse_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── mouse bindings (when mouse is on) ──────────────────────".into());
    v.push("  Left click status tab    switch to clicked window".into());
    v.push("  Left click pane          focus pane (+ forward to child)".into());
    v.push("  Left click border        begin drag-resize".into());
    v.push("  Left drag border         resize split interactively".into());
    v.push("  Scroll up/down           forward wheel to child (or copy mode scroll)".into());
    v
}

/// Build the full ordered list of lines for the C-b ? overlay.
///
/// `user_bindings` — `Vec<(repeat, table, key, command)>` from the
/// synced binding list.  Defaults that have been overridden by a user
/// binding in the prefix table are automatically excluded.
pub fn build_overlay_lines(
    user_bindings: &[(bool, String, String, String)],
    _defaults_suppressed: bool,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    // Since defaults are now populated in key_tables and synced as bindings,
    // all prefix bindings (defaults + user) come through user_bindings.
    // No need to separately iterate PREFIX_DEFAULTS.

    // ── 1. Prefix bindings ──
    lines.push("── prefix table (C-b + key) ───────────────────────────────".into());
    let prefix_bindings: Vec<_> = user_bindings.iter()
        .filter(|(_, t, _, _)| t == "prefix")
        .collect();
    for (repeat, table, key, cmd) in &prefix_bindings {
        let r = if *repeat { " -r" } else { "" };
        lines.push(format!("bind-key{} -T {} {} {}", r, table, key, cmd));
    }

    // ── 2. Non-prefix user bindings ──
    let non_prefix: Vec<_> = user_bindings.iter()
        .filter(|(_, t, _, _)| t != "prefix")
        .collect();
    if !non_prefix.is_empty() {
        lines.push(String::new());
        lines.push("── other table bindings ───────────────────────────────────".into());
        for (repeat, table, key, cmd) in &non_prefix {
            let r = if *repeat { " -r" } else { "" };
            lines.push(format!("bind-key{} -T {} {} {}", r, table, key, cmd));
        }
    }

    // ── 3-8. Reference sections ──
    lines.extend(copy_mode_vi_lines());
    lines.extend(copy_search_lines());
    lines.extend(command_prompt_lines());
    lines.extend(mouse_lines());
    lines.extend(cli_command_lines());
    lines.extend(options_lines());
    lines.extend(format_vars_lines());
    lines.extend(hooks_lines());

    lines
}

/// Build the output for the CLI `list-keys` command (server-side).
///
/// `user_tables` — iterator of `(table_name, key_str, action_str, repeat)`.
/// `defaults_suppressed` — when true, skip PREFIX_DEFAULTS (set by unbind-key -a).
pub fn build_list_keys_output<'a>(
    user_tables: impl Iterator<Item = (&'a str, String, String, bool)>,
    _defaults_suppressed: bool,
) -> String {
    let mut output = String::new();

    // Since defaults are now populated in key_tables (via populate_default_bindings),
    // all bindings (defaults + user overrides) come through user_tables.
    // No need to separately prepend PREFIX_DEFAULTS.
    let user_entries: Vec<(&str, String, String, bool)> = user_tables.collect();

    for (table, key, action, repeat) in &user_entries {
        let r = if *repeat { " -r" } else { "" };
        output.push_str(&format!("bind-key{} -T {} {} {}\n", r, table, key, action));
    }

    output
}
