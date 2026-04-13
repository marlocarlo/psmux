use std::env;
use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::types::{AppState, Action, Bind};
use crate::commands::parse_command_to_action;

// Track the current config file being parsed (for #{current_file}, #{d:current_file})
thread_local! {
    static CURRENT_CONFIG_FILE: RefCell<String> = RefCell::new(String::new());
}

/// Get the current config file path being parsed.
pub fn current_config_file() -> String {
    CURRENT_CONFIG_FILE.with(|f| f.borrow().clone())
}

/// Set the current config file path.
fn set_current_config_file(path: &str) {
    CURRENT_CONFIG_FILE.with(|f| *f.borrow_mut() = path.to_string());
}

/// Quick scan of the config file to check if `set -g warm off` is present.
/// Used by the client side before attempting warm server claim.
pub fn is_warm_disabled_by_config() -> bool {
    let content = if let Ok(config_file) = env::var("PSMUX_CONFIG_FILE") {
        let expanded = if config_file.starts_with('~') {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            config_file.replacen('~', &home, 1)
        } else {
            config_file
        };
        std::fs::read_to_string(expanded).ok()
    } else {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        let paths = [
            format!("{}/.psmux.conf", home),
            format!("{}/.psmuxrc", home),
            format!("{}/.tmux.conf", home),
            format!("{}/.config/psmux/psmux.conf", home),
        ];
        paths.iter().find_map(|p| std::fs::read_to_string(p).ok())
    };
    if let Some(content) = content {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') { continue; }
            // Match: set -g warm off, set warm off, set-option -g warm off, etc.
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 3 {
                let cmd = parts[0];
                if cmd == "set" || cmd == "set-option" {
                    // Find the option name and value, skipping flags like -g, -s, -q
                    let mut i = 1;
                    while i < parts.len() && parts[i].starts_with('-') { i += 1; }
                    if i + 1 < parts.len() && parts[i] == "warm" {
                        let val = parts[i + 1].trim_matches('"').trim_matches('\'');
                        return val == "off" || val == "false" || val == "0";
                    }
                }
            }
        }
    }
    false
}

/// Populate key_tables with PREFIX_DEFAULTS from help.rs.
/// This ensures default bindings live in key_tables (like tmux)
/// so that unbind-key <key> can actually remove them.
/// Must be called BEFORE load_config / source_file.
pub fn populate_default_bindings(app: &mut AppState) {
    let defaults = crate::help::PREFIX_DEFAULTS;
    let table = app.key_tables.entry("prefix".to_string()).or_default();
    for (key_str, cmd_str) in defaults {
        if let Some(key) = parse_key_name(key_str) {
            let key = normalize_key_for_binding(key);
            if let Some(action) = parse_command_to_action(cmd_str) {
                // Only add if not already present (user config may have overridden)
                if !table.iter().any(|b| b.key == key) {
                    table.push(Bind { key, action, repeat: false });
                }
            }
        }
    }
}

pub fn load_config(app: &mut AppState) {
    // If -f flag was used, load that specific config file instead of default search
    if let Ok(config_file) = env::var("PSMUX_CONFIG_FILE") {
        let expanded = if config_file.starts_with('~') {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            config_file.replacen('~', &home, 1)
        } else {
            config_file
        };
        set_current_config_file(&expanded);
        if let Ok(content) = std::fs::read_to_string(&expanded) {
            parse_config_content(app, &content);
        }
        set_current_config_file("");
        return;
    }

    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let paths = vec![
        format!("{}\\.psmux.conf", home),
        format!("{}\\.psmuxrc", home),
        format!("{}\\.tmux.conf", home),
        format!("{}\\.config\\psmux\\psmux.conf", home),
    ];
    for path in paths {
        if let Ok(content) = std::fs::read_to_string(&path) {
            set_current_config_file(&path);
            parse_config_content(app, &content);
            set_current_config_file("");
            break;
        }
    }
}

pub fn parse_config_content(app: &mut AppState, content: &str) {
    // Strip UTF-8 BOM if present (common on Windows when files are saved
    // with Notepad or other editors that prepend EF BB BF).
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);

    // Process %if / %elif / %else / %endif conditional blocks.
    // These are tmux config-level directives that control which lines are parsed.
    //
    // %if "#{==:#{@option},value}"   — evaluate format condition
    // %elif "#{condition}"           — else-if branch
    // %else                          — else branch
    // %endif                         — end conditional block
    // %hidden NAME=value             — define a hidden variable (stored but not shown)
    //
    // Blocks can nest. We track a stack of (active, satisfied) states.
    // - active: whether the current block should execute lines
    // - satisfied: whether any branch of the current if/elif/else has matched
    struct IfState {
        active: bool,    // are we executing lines in this block?
        satisfied: bool, // has any branch of this if/elif/else already matched?
        parent_active: bool, // was the parent context active?
    }

    let mut if_stack: Vec<IfState> = Vec::new();

    // Join continuation lines (ending with \)
    let mut lines: Vec<String> = Vec::new();
    let mut continuation = String::new();
    for line in content.lines() {
        let trimmed = line.trim_end();
        if trimmed.ends_with('\\') {
            continuation.push_str(trimmed.trim_end_matches('\\'));
            continuation.push(' ');
        } else {
            if !continuation.is_empty() {
                continuation.push_str(trimmed);
                lines.push(continuation.clone());
                continuation.clear();
            } else {
                lines.push(trimmed.to_string());
            }
        }
    }
    if !continuation.is_empty() {
        lines.push(continuation);
    }

    for line in &lines {
        let l = line.trim();

        // Skip empty lines and comments (but comments start with # not %)
        if l.is_empty() { continue; }

        // Handle %-directives before checking for # comments
        if l.starts_with('%') {
            if l.starts_with("%if ") || l.starts_with("%if\t") {
                let condition = l[3..].trim().trim_matches('"').trim_matches('\'');

                // Evaluate the condition using format expansion
                let parent_active = if_stack.last().map(|s| s.active).unwrap_or(true);
                let result = if parent_active {
                    let expanded = crate::format::expand_format(condition, app);
                    is_truthy_config(&expanded)
                } else {
                    false
                };

                if_stack.push(IfState {
                    active: parent_active && result,
                    satisfied: result,
                    parent_active,
                });
                continue;
            }

            if l.starts_with("%elif ") || l.starts_with("%elif\t") {
                if let Some(state) = if_stack.last_mut() {
                    let condition = l[5..].trim().trim_matches('"').trim_matches('\'');
                    if state.parent_active && !state.satisfied {
                        let expanded = crate::format::expand_format(condition, app);
                        let result = is_truthy_config(&expanded);
                        state.active = result;
                        if result { state.satisfied = true; }
                    } else {
                        state.active = false;
                    }
                }
                continue;
            }

            if l == "%else" {
                if let Some(state) = if_stack.last_mut() {
                    state.active = state.parent_active && !state.satisfied;
                    state.satisfied = true; // prevent further elif from matching
                }
                continue;
            }

            if l == "%endif" {
                if_stack.pop();
                continue;
            }

            if l.starts_with("%hidden ") {
                // %hidden NAME=VALUE — define a hidden config variable
                let rest = l[8..].trim();
                if let Some(eq_pos) = rest.find('=') {
                    let name = rest[..eq_pos].trim();
                    let value = rest[eq_pos + 1..].trim().trim_matches('"').trim_matches('\'');
                    // Only process if active
                    let active = if_stack.last().map(|s| s.active).unwrap_or(true);
                    if active {
                        app.environment.insert(name.to_string(), value.to_string());
                    }
                }
                continue;
            }

            // Unknown %-directive — skip
            continue;
        }

        // Regular line — only process if all enclosing %if blocks are active
        let active = if_stack.last().map(|s| s.active).unwrap_or(true);
        if !active { continue; }

        // Expand $NAME / ${NAME} references from %hidden variables.
        // tmux's %hidden directive defines server-level variables that are
        // expanded with $ syntax in subsequent config lines.
        let l = if l.contains('$') {
            expand_hidden_vars(l, &app.environment)
        } else {
            l.to_string()
        };

        parse_config_line(app, &l);
    }
}

/// Expand `$NAME` and `${NAME}` references to %hidden variable values.
/// Only expand if the variable exists in the environment map (which stores
/// both %hidden variables and @user-options without the @ prefix).
fn expand_hidden_vars(line: &str, env: &std::collections::HashMap<String, String>) -> String {
    let mut result = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'$' {
            // Check for ${NAME} syntax
            if i + 1 < len && bytes[i + 1] == b'{' {
                if let Some(close) = line[i + 2..].find('}') {
                    let name = &line[i + 2..i + 2 + close];
                    if let Some(val) = env.get(name) {
                        result.push_str(val);
                    } else {
                        // Not found — keep as literal
                        result.push_str(&line[i..i + 2 + close + 1]);
                    }
                    i = i + 2 + close + 1;
                    continue;
                }
            }
            // Check for $NAME syntax (NAME = [A-Z_][A-Z0-9_]*)
            let start = i + 1;
            let mut end = start;
            while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start {
                let name = &line[start..end];
                if let Some(val) = env.get(name) {
                    result.push_str(val);
                    i = end;
                    continue;
                }
            }
            // Not a recognized variable — keep literal $
            result.push('$');
            i += 1;
        } else {
            // Advance by full UTF-8 character (not single byte) to preserve
            // multi-byte chars like ▶ (U+25B6, 3 bytes) and ◀ (U+25C0).
            if let Some(ch) = line[i..].chars().next() {
                result.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        }
    }
    result
}

/// Check if a config-level condition result is truthy
fn is_truthy_config(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s != "0"
}

pub fn parse_config_line(app: &mut AppState, line: &str) {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') { return; }
    
    let l = if l.ends_with('\\') {
        l.trim_end_matches('\\').trim()
    } else {
        l
    };
    
    if l.starts_with("set-option ") || l.starts_with("set ") {
        parse_set_option(app, l);
    }
    else if l.starts_with("setw ") || l.starts_with("set-window-option ") {
        // setw maps to the same option parser (tmux window options overlap)
        parse_set_option(app, l);
    }
    else if l.starts_with("bind-key ") || l.starts_with("bind ") {
        parse_bind_key(app, l);
    }
    else if l.starts_with("unbind-key ") || l.starts_with("unbind ") {
        parse_unbind_key(app, l);
    }
    else if l.starts_with("source-file ") || l.starts_with("source ") {
        let parts: Vec<&str> = l.splitn(2, ' ').collect();
        if parts.len() > 1 {
            source_file(app, parts[1].trim());
        }
    }
    else if l.starts_with("run-shell ") || l.starts_with("run ") {
        parse_run_shell(app, l);
    }
    else if l.starts_with("if-shell ") || l.starts_with("if ") {
        parse_if_shell(app, l);
    }
    else if l.starts_with("set-hook ") {
        // Parse set-hook: set-hook [-g] [-a] [-u] hook-name [command]
        let parts: Vec<&str> = l.split_whitespace().collect();
        let mut i = 1;
        let mut unset = false;
        let mut append = false;
        while i < parts.len() && parts[i].starts_with('-') {
            if parts[i].contains('u') { unset = true; }
            if parts[i].contains('a') { append = true; }
            i += 1;
        }
        if unset {
            // set-hook -gu <hook-name>  →  remove the hook
            if i < parts.len() {
                app.hooks.remove(parts[i]);
            }
        } else if i + 1 < parts.len() {
            let hook = parts[i].to_string();
            let cmd = parts[i+1..].join(" ");
            // Strip matching outer quotes (single or double) that wrap the command
            let cmd = {
                let trimmed = cmd.trim();
                let bytes = trimmed.as_bytes();
                if bytes.len() >= 2 {
                    let first = bytes[0];
                    let last = bytes[bytes.len() - 1];
                    if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
                        trimmed[1..trimmed.len()-1].to_string()
                    } else {
                        cmd
                    }
                } else {
                    cmd
                }
            };
            if append {
                // -a/-ga: append to existing hook list (tmux multi-handler)
                app.hooks.entry(hook).or_insert_with(Vec::new).push(cmd);
            } else {
                // Replace (not append) to match tmux – prevents duplicates on
                // config reload (issue #133).
                app.hooks.insert(hook, vec![cmd]);
            }
        }
    }
    else if l.starts_with("set-environment ") || l.starts_with("setenv ") {
        let parts: Vec<&str> = l.split_whitespace().collect();
        let mut i = 1;
        while i < parts.len() && parts[i].starts_with('-') { i += 1; }
        if i + 1 < parts.len() {
            let val = parts[i+1..].join(" ");
            app.environment.insert(parts[i].to_string(), val.clone());
            // Also set on the server process so child panes inherit via env block
            std::env::set_var(parts[i], &val);
        }
    }
}

fn parse_set_option(app: &mut AppState, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 { return; }
    
    let mut i = 1;
    let mut is_global = false;
    let mut format_expand = false;  // -F: expand format strings in value
    let mut only_if_unset = false;  // -o: only set if not already set
    let mut append_mode = false;    // -a: append to current value
    let mut unset_mode = false;     // -u: unset (reset to default)
    
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('g') { is_global = true; }
            if p.contains('F') { format_expand = true; }
            if p.contains('o') { only_if_unset = true; }
            if p.contains('a') { append_mode = true; }
            if p.contains('u') { unset_mode = true; }
            // -q (quiet): no-op — we don't produce errors for unknown options
            // -w: window option — treat same as global for our single-server model
            i += 1;
            if p.contains('t') && i < parts.len() { i += 1; }
        } else {
            break;
        }
    }
    
    if i >= parts.len() { return; }

    // Extract key and value
    let key = parts[i];
    let raw_value = if i + 1 < parts.len() {
        parts[i + 1..].join(" ")
    } else {
        String::new()
    };

    // Handle -u (unset): reset option to empty
    if unset_mode {
        parse_option_value(app, &format!("{} ", key), is_global);
        return;
    }

    // Handle -o (only set if not currently set)
    if only_if_unset {
        let current = crate::format::lookup_option_pub(key, app);
        if let Some(ref v) = current {
            if !v.is_empty() { return; }
        }
    }

    // Expand format strings in the value if -F flag is set
    let value = if format_expand && !raw_value.is_empty() {
        let stripped = raw_value.trim_matches('"').trim_matches('\'');
        let expanded = crate::format::expand_format(stripped, app);
        expanded
    } else {
        raw_value
    };

    // Handle -a (append to current value)
    let final_value = if append_mode {
        let current = crate::format::lookup_option_pub(key, app).unwrap_or_default();
        format!("{}{}", current, value.trim_matches('"').trim_matches('\''))
    } else {
        value
    };

    let rest = format!("{} {}", key, final_value);
    parse_option_value(app, &rest, is_global);
}

pub fn parse_option_value(app: &mut AppState, rest: &str, _is_global: bool) {
    let parts: Vec<&str> = rest.splitn(2, ' ').collect();
    if parts.is_empty() { return; }
    
    let key = parts[0].trim();
    let value = if parts.len() > 1 {
        let v = parts[1].trim();
        // Only strip quotes when the entire value is wrapped in matching
        // quotes.  Preserves values like `"path with spaces" --login`.
        if (v.starts_with('"') && v.ends_with('"'))
            || (v.starts_with('\'') && v.ends_with('\''))
        {
            &v[1..v.len() - 1]
        } else {
            v
        }
    } else {
        ""
    };
    
    match key {
        "status-left" => app.status_left = value.to_string(),
        "status-right" => app.status_right = value.to_string(),
        "mouse" => app.mouse_enabled = matches!(value, "on" | "true" | "1"),
        "scroll-enter-copy-mode" => app.scroll_enter_copy_mode = matches!(value, "on" | "true" | "1"),
        "pwsh-mouse-selection" => app.pwsh_mouse_selection = matches!(value, "on" | "true" | "1"),
        "prefix" => {
            if let Some(key) = parse_key_name(value) {
                app.prefix_key = key;
            }
        }
        "prefix2" => {
            if value == "none" || value.is_empty() {
                app.prefix2_key = None;
            } else if let Some(key) = parse_key_name(value) {
                app.prefix2_key = Some(key);
            }
        }
        "escape-time" => {
            if let Ok(ms) = value.parse::<u64>() {
                app.escape_time_ms = ms;
            }
        }
        "prediction-dimming" | "dim-predictions" => {
            app.prediction_dimming = !matches!(value, "off" | "false" | "0");
        }
        "cursor-style" => env::set_var("PSMUX_CURSOR_STYLE", value),
        "cursor-blink" => {
            let on = matches!(value, "on"|"true"|"1");
            env::set_var("PSMUX_CURSOR_BLINK", if on { "1" } else { "0" });
            let _ = std::io::Write::write_all(&mut std::io::stdout(), if on { b"\x1b[?12h" } else { b"\x1b[?12l" });
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
        "status" => {
            if let Ok(n) = value.parse::<usize>() {
                if n >= 2 {
                    app.status_visible = true;
                    app.status_lines = n;
                } else if n == 1 {
                    app.status_visible = true;
                    app.status_lines = 1;
                } else {
                    app.status_visible = false;
                    app.status_lines = 1;
                }
            } else {
                app.status_visible = matches!(value, "on" | "true");
            }
        }
        "status-style" => {
            app.status_style = value.to_string();
        }
        "status-position" => {
            app.status_position = value.to_string();
        }
        "status-interval" => {
            if let Ok(n) = value.parse::<u64>() { app.status_interval = n; }
        }
        "status-justify" => { app.status_justify = value.to_string(); }
        "base-index" => {
            if let Ok(idx) = value.parse::<usize>() {
                app.window_base_index = idx;
            }
        }
        "pane-base-index" => {
            if let Ok(idx) = value.parse::<usize>() {
                app.pane_base_index = idx;
            }
        }
        "history-limit" => {
            if let Ok(limit) = value.parse::<usize>() {
                app.history_limit = limit;
            }
        }
        "display-time" => {
            if let Ok(ms) = value.parse::<u64>() {
                app.display_time_ms = ms;
            }
        }
        "display-panes-time" => {
            if let Ok(ms) = value.parse::<u64>() {
                app.display_panes_time_ms = ms;
            }
        }
        "default-command" | "default-shell" => {
            app.default_shell = value.to_string();
        }
        "word-separators" => {
            app.word_separators = value.to_string();
        }
        "renumber-windows" => {
            app.renumber_windows = matches!(value, "on" | "true" | "1");
        }
        "mode-keys" => {
            app.mode_keys = value.to_string();
        }
        "focus-events" => {
            app.focus_events = matches!(value, "on" | "true" | "1");
        }
        "monitor-activity" => {
            app.monitor_activity = matches!(value, "on" | "true" | "1");
        }
        "visual-activity" => {
            app.visual_activity = matches!(value, "on" | "true" | "1");
        }
        "remain-on-exit" => {
            app.remain_on_exit = matches!(value, "on" | "true" | "1");
        }
        "destroy-unattached" => {
            app.destroy_unattached = matches!(value, "on" | "true" | "1");
        }
        "exit-empty" => {
            app.exit_empty = matches!(value, "on" | "true" | "1");
        }
        "aggressive-resize" => {
            app.aggressive_resize = matches!(value, "on" | "true" | "1");
        }
        "set-titles" => {
            app.set_titles = matches!(value, "on" | "true" | "1");
        }
        "set-titles-string" => {
            app.set_titles_string = value.to_string();
        }
        "status-keys" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "pane-border-style" => { app.pane_border_style = value.to_string(); }
        "pane-active-border-style" => { app.pane_active_border_style = value.to_string(); }
        "pane-border-hover-style" => { app.pane_border_hover_style = value.to_string(); }
        "window-status-format" => { app.window_status_format = value.to_string(); }
        "window-status-current-format" => { app.window_status_current_format = value.to_string(); }
        "window-status-separator" => { app.window_status_separator = value.to_string(); }
        "automatic-rename" => {
            app.automatic_rename = matches!(value, "on" | "true" | "1");
        }
        "synchronize-panes" => {
            app.sync_input = matches!(value, "on" | "true" | "1");
        }
        "allow-rename" => {
            app.allow_rename = matches!(value, "on" | "true" | "1");
        }
        "terminal-overrides" => { /* tmux terminfo override — accepted for compatibility, no-op on Windows */ }
        "default-terminal" => {
            // tmux sets the TERM env var from this option (#137)
            app.environment.insert("TERM".to_string(), value.to_string());
        }
        "update-environment" => {
            // tmux: space-separated list of env var names to update from client on attach
            app.update_environment = value.split_whitespace().map(|s| s.to_string()).collect();
        }
        "bell-action" => { app.bell_action = value.to_string(); }
        "visual-bell" => { app.visual_bell = matches!(value, "on" | "true" | "1"); }
        "activity-action" => {
            app.activity_action = value.to_string();
        }
        "silence-action" => {
            app.silence_action = value.to_string();
        }
        "monitor-silence" => {
            if let Ok(n) = value.parse::<u64>() { app.monitor_silence = n; }
        }
        "message-style" => { app.message_style = value.to_string(); }
        "message-command-style" => { app.message_command_style = value.to_string(); }
        "mode-style" => { app.mode_style = value.to_string(); }
        "window-status-style" => { app.window_status_style = value.to_string(); }
        "window-status-current-style" => { app.window_status_current_style = value.to_string(); }
        "window-status-activity-style" => { app.window_status_activity_style = value.to_string(); }
        "window-status-bell-style" => { app.window_status_bell_style = value.to_string(); }
        "window-status-last-style" => { app.window_status_last_style = value.to_string(); }
        "status-left-style" => { app.status_left_style = value.to_string(); }
        "status-right-style" => { app.status_right_style = value.to_string(); }
        "clock-mode-colour" | "clock-mode-style" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "pane-border-format" | "pane-border-status" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "popup-style" | "popup-border-style" | "popup-border-lines" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "window-style" | "window-active-style" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "wrap-search" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "lock-after-time" | "lock-command" => { app.user_options.insert(key.to_string(), value.to_string()); }
        "main-pane-width" => {
            if let Ok(n) = value.parse::<u16>() { app.main_pane_width = n; }
        }
        "main-pane-height" => {
            if let Ok(n) = value.parse::<u16>() { app.main_pane_height = n; }
        }
        "status-left-length" => {
            if let Ok(n) = value.parse::<usize>() { app.status_left_length = n; }
        }
        "status-right-length" => {
            if let Ok(n) = value.parse::<usize>() { app.status_right_length = n; }
        }
        "window-size" => { app.window_size = value.to_string(); }
        "allow-passthrough" => { app.allow_passthrough = value.to_string(); }
        "copy-command" => { app.copy_command = value.to_string(); }
        "set-clipboard" => { app.set_clipboard = value.to_string(); }
        "env-shim" => {
            app.env_shim = matches!(value, "on" | "true" | "1");
        }
        "allow-predictions" => {
            app.allow_predictions = matches!(value, "on" | "true" | "1");
        }
        "claude-code-fix-tty" => {
            app.claude_code_fix_tty = matches!(value, "on" | "true" | "1");
        }
        "claude-code-force-interactive" => {
            app.claude_code_force_interactive = matches!(value, "on" | "true" | "1");
        }
        "warm" => {
            app.warm_enabled = matches!(value, "on" | "true" | "1");
            if !app.warm_enabled {
                if let Some(mut wp) = app.warm_pane.take() {
                    wp.child.kill().ok();
                }
            }
        }
        "command-alias" => {
            if let Some(pos) = value.find('=') {
                let alias = value[..pos].trim().to_string();
                let expansion = value[pos+1..].trim().to_string();
                app.command_aliases.insert(alias, expansion);
            }
        }
        _ => {
            // Handle status-format[N] patterns
            if key.starts_with("status-format[") && key.ends_with(']') {
                if let Ok(idx) = key["status-format[".len()..key.len()-1].parse::<usize>() {
                    while app.status_format.len() <= idx {
                        app.status_format.push(String::new());
                    }
                    app.status_format[idx] = value.to_string();
                    return;
                }
            }
            // Store @-prefixed user/plugin options separately from environment
            // so they don't leak into child shells (#105).
            if key.starts_with('@') {
                app.user_options.insert(key.to_string(), value.to_string());
            } else if key.contains('-') {
                // Options with hyphens are tmux config options, NOT environment
                // variables.  Storing them in environment causes PowerShell
                // ParserErrors when injected via $env:NAME syntax (#137).
                app.user_options.insert(key.to_string(), value.to_string());
            } else {
                app.environment.insert(key.to_string(), value.to_string());
            }

            // Auto-source plugin conf files when @plugin is declared.
            // This makes theme/settings load synchronously during config
            // parsing instead of waiting for PPM's async run-shell to
            // source them later (which causes a visible flash).
            //
            // Format: set -g @plugin 'org/plugin-name' or 'plugin-name'
            // Tries:  ~/.psmux/plugins/<full-value>/plugin.conf
            //   then: ~/.psmux/plugins/<last-component>/plugin.conf
            if key == "@plugin" && !value.is_empty() {
                let plugin_name = value.rsplit('/').next().unwrap_or(value);
                if plugin_name != "ppm" {
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let xdg_config = env::var("XDG_CONFIG_HOME")
                        .unwrap_or_else(|_| format!("{}\\.config", home));
                    let candidates = [
                        // Classic paths: ~/.psmux/plugins/
                        format!("{}\\.psmux\\plugins\\{}\\plugin.conf", home, value.replace('/', "\\")),
                        format!("{}\\.psmux\\plugins\\{}\\plugin.conf", home, plugin_name),
                        format!("{}\\.psmux\\plugins\\psmux-plugins\\{}\\plugin.conf", home, plugin_name),
                        // XDG paths: ~/.config/psmux/plugins/
                        format!("{}\\psmux\\plugins\\{}\\plugin.conf", xdg_config, value.replace('/', "\\")),
                        format!("{}\\psmux\\plugins\\{}\\plugin.conf", xdg_config, plugin_name),
                        format!("{}\\psmux\\plugins\\psmux-plugins\\{}\\plugin.conf", xdg_config, plugin_name),
                    ];
                    let mut found = false;
                    for conf in &candidates {
                        if std::path::Path::new(conf).exists() {
                            let prev_file = current_config_file();
                            set_current_config_file(conf);
                            if let Ok(content) = std::fs::read_to_string(conf) {
                                parse_config_content(app, &content);
                            }
                            set_current_config_file(&prev_file);
                            found = true;
                            break;
                        }
                    }
                    // If no plugin.conf, try .ps1 entry scripts
                    if !found {
                        let ps1_candidates = [
                            // Classic paths
                            format!("{}\\.psmux\\plugins\\{}\\{}.ps1", home, value.replace('/', "\\"), plugin_name),
                            format!("{}\\.psmux\\plugins\\{}\\{}.ps1", home, plugin_name, plugin_name),
                            format!("{}\\.psmux\\plugins\\psmux-plugins\\{}\\{}.ps1", home, plugin_name, plugin_name),
                            // XDG paths
                            format!("{}\\psmux\\plugins\\{}\\{}.ps1", xdg_config, value.replace('/', "\\"), plugin_name),
                            format!("{}\\psmux\\plugins\\{}\\{}.ps1", xdg_config, plugin_name, plugin_name),
                            format!("{}\\psmux\\plugins\\psmux-plugins\\{}\\{}.ps1", xdg_config, plugin_name, plugin_name),
                        ];
                        for ps1 in &ps1_candidates {
                            if std::path::Path::new(ps1).exists() {
                                // First try static extraction of set/bind commands
                                if let Ok(content) = std::fs::read_to_string(ps1) {
                                    let prev_file = current_config_file();
                                    set_current_config_file(ps1);
                                    let applied = parse_ps1_plugin_script(app, &content);
                                    set_current_config_file(&prev_file);
                                    // If the script uses PS variables (theme plugins),
                                    // static extraction yields unresolved $vars.
                                    // Queue for post-startup execution when the
                                    // server is listening.
                                    if !applied {
                                        app.pending_plugin_scripts.push(ps1.clone());
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Split a bind-key command string on `\;` or bare `;` to produce sub-commands.
/// Handles: `split-window \; select-pane -D` → ["split-window", "select-pane -D"]
pub fn split_chained_commands_pub(command: &str) -> Vec<String> {
    split_chained_commands(command)
}
fn split_chained_commands(command: &str) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    let mut current = String::new();
    let tokens: Vec<&str> = command.split_whitespace().collect();
    
    for token in &tokens {
        if *token == "\\;" || *token == ";" {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                commands.push(trimmed);
            }
            current.clear();
        } else {
            if !current.is_empty() { current.push(' '); }
            current.push_str(token);
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        commands.push(trimmed);
    }
    commands
}

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

/// Normalize a key tuple for binding comparison.
/// Strips SHIFT from Char events since the character itself encodes shift information.
/// e.g., '|' already implies Shift was pressed, so (Char('|'), SHIFT) and (Char('|'), NONE) should match.
pub fn normalize_key_for_binding(key: (KeyCode, KeyModifiers)) -> (KeyCode, KeyModifiers) {
    match key.0 {
        KeyCode::Char(_) => (key.0, key.1.difference(KeyModifiers::SHIFT)),
        _ => key,
    }
}

/// Map a multi-character key name (case-insensitive) to a KeyCode.
/// Returns None if the name is not recognized.
fn named_key(name: &str) -> Option<KeyCode> {
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

/// Execute a run-shell / run command from config or hooks.
/// Syntax: run-shell [-b] <command>
/// Always spawns non-blocking to avoid deadlocks when hooks fire on the
/// server thread (scripts may call back to psmux via CLI).
fn parse_run_shell(app: &mut AppState, line: &str) {
    // Use quote-aware parser to properly handle nested quotes and escapes
    let args = crate::commands::parse_command_line(line);
    if args.len() < 2 { return; }
    let mut cmd_parts: Vec<&str> = Vec::new();
    for arg in &args[1..] {
        if arg == "-b" { /* background flag — always spawn anyway */ }
        else { cmd_parts.push(arg); }
    }
    let shell_cmd = cmd_parts.join(" ");
    if shell_cmd.is_empty() { return; }

    // Expand ~ to home directory + XDG fallback for plugin paths
    let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd);

    // ── Handle .tmux files natively ──────────────────────────────────
    // .tmux files are bash scripts used by tmux plugins. On Windows they
    // can't be executed by pwsh. Parse them for `tmux source`, `tmux set`,
    // etc. and apply the extracted commands as config lines.
    let trimmed_cmd = shell_cmd.trim().trim_matches('\'').trim_matches('"');
    if trimmed_cmd.ends_with(".tmux") {
        let tmux_path = std::path::Path::new(trimmed_cmd);
        if tmux_path.is_file() {
            parse_tmux_entry_script(app, tmux_path);
            return;
        }
    }
    // Also handle .ps1 files natively when possible: if the command is a
    // bare .ps1 path (no arguments), we can run it directly with pwsh -File
    // which is more reliable than -Command for script paths with spaces.

    // Always spawn non-blocking: run-shell commands from hooks may call back
    // to the psmux server (e.g., `psmux set -g @option value`), which would
    // deadlock if we blocked the server thread with .output().
    // Set PSMUX_TARGET_SESSION so child scripts connect to the correct server
    // (especially important when using -L socket namespaces like in tppanel preview).
    let target_session = app.port_file_base();
    let mut cmd = crate::commands::build_run_shell_command(&shell_cmd);
    if !target_session.is_empty() {
        cmd.env("PSMUX_TARGET_SESSION", &target_session);
    }
    let _ = cmd.spawn();
}

/// Parse a `.tmux` entry script (bash) and extract tmux commands from it.
///
/// .tmux files are the standard entry point for tmux plugins. They are bash
/// scripts that typically call `tmux source <file>`, `tmux set -g ...`, etc.
/// On Windows we can't run bash, so we parse the script and translate the
/// tmux CLI calls into psmux config lines.
///
/// Supported patterns:
/// Parse a .ps1 plugin entry script and extract psmux set/bind commands.
///
/// Plugin .ps1 scripts use patterns like:
///   & $PSMUX set -g key value 2>&1 | Out-Null
///   & $PSMUX bind-key ...
///
/// We extract the psmux command portion and apply it as config.
/// Returns true if all extracted values are literal (no unresolved PS variables).
/// Returns false if the script uses PowerShell variables that need runtime eval.
fn parse_ps1_plugin_script(app: &mut AppState, content: &str) -> bool {
    let mut has_ps_vars = false;
    let mut applied_any = false;

    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') { continue; }

        // Match patterns like: & $PSMUX set -g ... 2>&1 | Out-Null
        // Also: & $PSMUX bind-key ... 2>&1 | Out-Null
        let cmd_start = if let Some(pos) = l.find("$PSMUX ") {
            // Ensure it's preceded by "& " (PowerShell call operator)
            let prefix = &l[..pos];
            if prefix.trim_end().ends_with('&') {
                Some(pos + 7) // skip "$PSMUX "
            } else {
                None
            }
        } else {
            None
        };

        let cmd = match cmd_start {
            Some(start) => &l[start..],
            None => continue,
        };

        // Strip trailing PowerShell noise: 2>&1 | Out-Null, 2>$null
        let cmd = cmd.split(" 2>&1").next().unwrap_or(cmd);
        let cmd = cmd.split(" 2>$null").next().unwrap_or(cmd);
        let cmd = cmd.trim();

        // Check for unresolved PowerShell variables (e.g., $bg1, $fg)
        // but not $PSMUX or $TMUX which are expected patterns
        if cmd.contains('$') {
            // Check if it's a PS variable reference (not env var pattern)
            let has_var = cmd.split('$').skip(1).any(|part| {
                let first_word: String = part.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                !first_word.is_empty() && first_word != "PSMUX" && first_word != "TMUX"
                    && first_word != "env" && first_word != "null"
            });
            if has_var { has_ps_vars = true; }
        }

        if cmd.starts_with("set ") || cmd.starts_with("set-option ")
            || cmd.starts_with("bind-key ") || cmd.starts_with("bind ")
            || cmd.starts_with("setw ") || cmd.starts_with("set-window-option ") {
            if !has_ps_vars {
                parse_config_line(app, cmd);
                applied_any = true;
            }
        }
    }

    // Return true if we applied commands and they were all literal
    applied_any && !has_ps_vars
}

///   tmux source[-file] "path"       → source-file "path"
///   tmux set[-option] [-g] key val  → set [-g] key val
///   tmux setw key val               → setw key val
///   PLUGIN_DIR=...                  → track for variable expansion
fn parse_tmux_entry_script(app: &mut AppState, path: &std::path::Path) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Determine the directory of the .tmux file for $PLUGIN_DIR / ${PLUGIN_DIR}
    let plugin_dir = path.parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Also look for PLUGIN_DIR assignment in the script (may differ)
    let mut script_plugin_dir = plugin_dir.clone();
    // Common pattern:  PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    // We can't evaluate bash, so we just use the file's parent directory.

    for line in content.lines() {
        let l = line.trim();
        // Skip empty lines, comments, shebang
        if l.is_empty() || l.starts_with('#') { continue; }

        // Track explicit PLUGIN_DIR assignment (best-effort)
        if l.starts_with("PLUGIN_DIR=") || l.starts_with("export PLUGIN_DIR=") {
            // If it's a simple literal path, use it
            let val = l.splitn(2, '=').nth(1).unwrap_or("").trim_matches('"').trim_matches('\'');
            if !val.contains('$') && !val.contains('`') && !val.is_empty() {
                script_plugin_dir = val.to_string();
            }
            // Otherwise keep using the .tmux file's parent dir
            continue;
        }

        // Skip other bash-isms (variable assignments, if/fi, for, etc.)
        if l.contains("BASH_SOURCE") || l.starts_with("cd ") || l.starts_with("export ")
            || l.starts_with("if ") || l == "fi" || l.starts_with("for ")
            || l.starts_with("done") || l.starts_with("then") || l.starts_with("else")
            || l.starts_with("local ") || l.starts_with("readonly ") {
            continue;
        }

        // Extract tmux commands: look for lines starting with `tmux `
        let tmux_cmd = if l.starts_with("tmux ") {
            &l[5..]
        } else if l.starts_with("\"$TMUX_PROGRAM\" ") || l.starts_with("$TMUX_PROGRAM ") {
            // Some plugins use $TMUX_PROGRAM variable
            let start = l.find(' ').unwrap_or(l.len());
            l[start..].trim()
        } else {
            continue;
        };

        // Expand $PLUGIN_DIR, ${PLUGIN_DIR}, $CURRENT_DIR, ${CURRENT_DIR}
        let expanded = tmux_cmd
            .replace("${PLUGIN_DIR}", &script_plugin_dir)
            .replace("$PLUGIN_DIR", &script_plugin_dir)
            .replace("${CURRENT_DIR}", &script_plugin_dir)
            .replace("$CURRENT_DIR", &script_plugin_dir);

        // Now parse the tmux subcommand as a psmux config line
        let expanded = expanded.trim();
        if expanded.starts_with("source-file ") || expanded.starts_with("source ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("set-option ") || expanded.starts_with("set ")
            || expanded.starts_with("set -g ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("setw ") || expanded.starts_with("set-window-option ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("run-shell ") || expanded.starts_with("run ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("bind-key ") || expanded.starts_with("bind ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("if-shell ") || expanded.starts_with("if ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("set-hook ") {
            parse_config_line(app, expanded);
        } else {
            // Try to parse it anyway — it might be a valid config directive
            parse_config_line(app, expanded);
        }
    }

    // Fallback: if we didn't find any tmux commands in the script, try to
    // source .conf files from the same directory (many themes ship both
    // .tmux entry script and .conf files).
    // Check if we actually parsed anything by looking at common indicators
    // (status-left, status-right being changed from defaults).
    // For now, also auto-source any *_tmux.conf or *.conf files in the dir.
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if ext == "conf" {
                        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        // Source companion .conf files (but not the .tmux script itself)
                        // Prioritize files like plugin_name_options.conf, plugin_name.conf
                        if fname.ends_with("_tmux.conf") || fname.ends_with("_options_tmux.conf") {
                            source_file(app, &p.to_string_lossy());
                        }
                    }
                }
            }
        }
    }
}

/// Execute an if-shell / if command from config.
/// Syntax: if-shell [-bF] <condition> <true-cmd> [<false-cmd>]
/// Runs the condition command (or evaluates format with -F), then executes the
/// appropriate branch command as a config line.
fn parse_if_shell(app: &mut AppState, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 { return; }

    let mut format_mode = false;
    let mut _background = false;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1;
    while i < parts.len() {
        match parts[i] {
            "-b" => { _background = true; }
            "-F" => { format_mode = true; }
            "-bF" | "-Fb" => { _background = true; format_mode = true; }
            "-t" => { i += 1; } // skip target
            s => {
                // Handle quoted strings that might span multiple parts
                if s.starts_with('"') || s.starts_with('\'') {
                    let quote = s.chars().next().unwrap();
                    if s.ends_with(quote) && s.len() > 1 {
                        positional.push(s[1..s.len()-1].to_string());
                    } else {
                        let mut buf = s[1..].to_string();
                        i += 1;
                        while i < parts.len() {
                            buf.push(' ');
                            buf.push_str(parts[i]);
                            if parts[i].ends_with(quote) {
                                buf.truncate(buf.len() - 1);
                                break;
                            }
                            i += 1;
                        }
                        positional.push(buf);
                    }
                } else {
                    positional.push(s.to_string());
                }
            }
        }
        i += 1;
    }

    if positional.len() < 2 { return; }
    let condition = &positional[0];
    let true_cmd = &positional[1];
    let false_cmd = positional.get(2);

    let success = if format_mode {
        let expanded = crate::format::expand_format(condition, app);
        !expanded.is_empty() && expanded != "0"
    } else if condition == "true" || condition == "1" {
        true
    } else if condition == "false" || condition == "0" {
        false
    } else {
        let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
        let mut c = std::process::Command::new(&shell_prog);
        for a in &shell_args { c.arg(a); }
        c.arg(condition);
        c.status().map(|s| s.success()).unwrap_or(false)
    };

    let cmd_to_run = if success { Some(true_cmd) } else { false_cmd };
    if let Some(cmd) = cmd_to_run {
        // Execute the branch as a config line (recursive — supports set, bind, source, etc.)
        parse_config_line(app, cmd);
    }
}

#[cfg(test)]
#[path = "../tests-rs/test_config_plugin_paths.rs"]
mod tests_plugin_paths;

#[cfg(test)]
#[path = "../tests-rs/test_issue137_env_leak.rs"]
mod tests_issue137_env_leak;

#[cfg(test)]
#[path = "../tests-rs/test_issue157_bind_key_case.rs"]
mod tests_issue157_bind_key_case;

#[cfg(test)]
#[path = "../tests-rs/test_issue145_source_file.rs"]
mod tests_issue145_source_file;

#[cfg(test)]
#[path = "../tests-rs/test_issue193_scroll_enter_copy_mode.rs"]
mod tests_issue193_scroll_enter_copy_mode;

#[cfg(test)]
#[path = "../tests-rs/test_issue198_unbind_individual.rs"]
mod tests_issue198_unbind_individual;
