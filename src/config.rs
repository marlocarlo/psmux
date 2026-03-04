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
        // Parse set-hook: set-hook [-g] hook-name command
        let parts: Vec<&str> = l.split_whitespace().collect();
        let mut i = 1;
        while i < parts.len() && parts[i].starts_with('-') { i += 1; }
        if i + 1 < parts.len() {
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
            app.hooks.entry(hook).or_insert_with(Vec::new).push(cmd);
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
        parts[1].trim().trim_matches('"').trim_matches('\'')
    } else { 
        "" 
    };
    
    match key {
        "status-left" => app.status_left = value.to_string(),
        "status-right" => app.status_right = value.to_string(),
        "mouse" => app.mouse_enabled = matches!(value, "on" | "true" | "1"),
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
        "cursor-blink" => env::set_var("PSMUX_CURSOR_BLINK", if matches!(value, "on"|"true"|"1") { "1" } else { "0" }),
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
        "status-keys" => { app.environment.insert(key.to_string(), value.to_string()); }
        "pane-border-style" => { app.pane_border_style = value.to_string(); }
        "pane-active-border-style" => { app.pane_active_border_style = value.to_string(); }
        "window-status-format" => { app.window_status_format = value.to_string(); }
        "window-status-current-format" => { app.window_status_current_format = value.to_string(); }
        "window-status-separator" => { app.window_status_separator = value.to_string(); }
        "automatic-rename" => {
            app.automatic_rename = matches!(value, "on" | "true" | "1");
        }
        "synchronize-panes" => {
            app.sync_input = matches!(value, "on" | "true" | "1");
        }
        "allow-rename" => { app.environment.insert(key.to_string(), value.to_string()); }
        "terminal-overrides" => { app.environment.insert(key.to_string(), value.to_string()); }
        "default-terminal" => { app.environment.insert(key.to_string(), value.to_string()); }
        "update-environment" => { app.environment.insert(key.to_string(), value.to_string()); }
        "bell-action" => { app.bell_action = value.to_string(); }
        "visual-bell" => { app.visual_bell = matches!(value, "on" | "true" | "1"); }
        "activity-action" => { app.environment.insert(key.to_string(), value.to_string()); }
        "silence-action" => { app.environment.insert(key.to_string(), value.to_string()); }
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
        "clock-mode-colour" | "clock-mode-style" => { app.environment.insert(key.to_string(), value.to_string()); }
        "pane-border-format" | "pane-border-status" => { app.environment.insert(key.to_string(), value.to_string()); }
        "popup-style" | "popup-border-style" | "popup-border-lines" => { app.environment.insert(key.to_string(), value.to_string()); }
        "window-style" | "window-active-style" => { app.environment.insert(key.to_string(), value.to_string()); }
        "wrap-search" => { app.environment.insert(key.to_string(), value.to_string()); }
        "lock-after-time" | "lock-command" => { app.environment.insert(key.to_string(), value.to_string()); }
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
            // Store any unknown option in the environment map for plugin compat
            app.environment.insert(key.to_string(), value.to_string());

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
                    let candidates = [
                        format!("{}\\.psmux\\plugins\\{}\\plugin.conf", home, value.replace('/', "\\")),
                        format!("{}\\.psmux\\plugins\\{}\\plugin.conf", home, plugin_name),
                    ];
                    for conf in &candidates {
                        if std::path::Path::new(conf).exists() {
                            let prev_file = current_config_file();
                            set_current_config_file(conf);
                            if let Ok(content) = std::fs::read_to_string(conf) {
                                parse_config_content(app, &content);
                            }
                            set_current_config_file(&prev_file);
                            break;
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
    
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('a') { unbind_all = true; }
            if p.contains('T') { i += 1; }
            i += 1;
        } else {
            break;
        }
    }
    
    if unbind_all {
        app.key_tables.clear();
        return;
    }
    
    if i < parts.len() {
        if let Some(key) = parse_key_name(parts[i]) {
            let key = normalize_key_for_binding(key);
            // Remove from all tables
            for table in app.key_tables.values_mut() {
                table.retain(|b| b.key != key);
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
    
    if name.starts_with("C-") || name.starts_with("^") {
        let ch = if name.starts_with("C-") {
            name.chars().nth(2)
        } else {
            name.chars().nth(1)
        };
        if let Some(c) = ch {
            return Some((KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL));
        }
    }
    
    if name.starts_with("M-") {
        if let Some(c) = name.chars().nth(2) {
            return Some((KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::ALT));
        }
    }
    
    if name.starts_with("S-") {
        let rest = &name[2..];
        if rest.eq_ignore_ascii_case("Tab") {
            return Some((KeyCode::BackTab, KeyModifiers::NONE));
        }
        // Handle S-Left, S-Right, S-Up, S-Down (Shift+Arrow)
        match rest.to_lowercase().as_str() {
            "left" => return Some((KeyCode::Left, KeyModifiers::SHIFT)),
            "right" => return Some((KeyCode::Right, KeyModifiers::SHIFT)),
            "up" => return Some((KeyCode::Up, KeyModifiers::SHIFT)),
            "down" => return Some((KeyCode::Down, KeyModifiers::SHIFT)),
            "home" => return Some((KeyCode::Home, KeyModifiers::SHIFT)),
            "end" => return Some((KeyCode::End, KeyModifiers::SHIFT)),
            "pageup" | "ppage" => return Some((KeyCode::PageUp, KeyModifiers::SHIFT)),
            "pagedown" | "npage" => return Some((KeyCode::PageDown, KeyModifiers::SHIFT)),
            _ => {}
        }
        if let Some(c) = rest.chars().next() {
            if rest.len() == 1 {
                return Some((KeyCode::Char(c.to_ascii_uppercase()), KeyModifiers::SHIFT));
            }
        }
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
        "a" => KeyCode::Char('a'),
        "b" => KeyCode::Char('b'),
        "c" => KeyCode::Char('c'),
        "d" => KeyCode::Char('d'),
        "e" => KeyCode::Char('e'),
        "f" => KeyCode::Char('f'),
        "g" => KeyCode::Char('g'),
        "h" => KeyCode::Char('h'),
        "i" => KeyCode::Char('i'),
        "j" => KeyCode::Char('j'),
        "k" => KeyCode::Char('k'),
        "l" => KeyCode::Char('l'),
        "m" => KeyCode::Char('m'),
        "n" => KeyCode::Char('n'),
        "o" => KeyCode::Char('o'),
        "p" => KeyCode::Char('p'),
        "q" => KeyCode::Char('q'),
        "r" => KeyCode::Char('r'),
        "s" => KeyCode::Char('s'),
        "t" => KeyCode::Char('t'),
        "u" => KeyCode::Char('u'),
        "v" => KeyCode::Char('v'),
        "w" => KeyCode::Char('w'),
        "x" => KeyCode::Char('x'),
        "y" => KeyCode::Char('y'),
        "z" => KeyCode::Char('z'),
        "0" => KeyCode::Char('0'),
        "1" => KeyCode::Char('1'),
        "2" => KeyCode::Char('2'),
        "3" => KeyCode::Char('3'),
        "4" => KeyCode::Char('4'),
        "5" => KeyCode::Char('5'),
        "6" => KeyCode::Char('6'),
        "7" => KeyCode::Char('7'),
        "8" => KeyCode::Char('8'),
        "9" => KeyCode::Char('9'),
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
            if key_part.len() == 1 {
                KeyCode::Char(key_part.chars().next().unwrap())
            } else {
                return None;
            }
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

    // Expand ~ to home directory in the command
    let shell_cmd = if shell_cmd.contains('~') {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        shell_cmd.replace("~/", &format!("{}/", home)).replace("~\\", &format!("{}\\", home))
    } else {
        shell_cmd
    };

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
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("pwsh");
        cmd.args(["-NoProfile", "-Command", &shell_cmd]);
        if !target_session.is_empty() {
            cmd.env("PSMUX_TARGET_SESSION", &target_session);
        }
        let _ = cmd.spawn();
    }
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", &shell_cmd]);
        if !target_session.is_empty() {
            cmd.env("PSMUX_TARGET_SESSION", &target_session);
        }
        let _ = cmd.spawn();
    }
}

/// Parse a `.tmux` entry script (bash) and extract tmux commands from it.
///
/// .tmux files are the standard entry point for tmux plugins. They are bash
/// scripts that typically call `tmux source <file>`, `tmux set -g ...`, etc.
/// On Windows we can't run bash, so we parse the script and translate the
/// tmux CLI calls into psmux config lines.
///
/// Supported patterns:
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
        !condition.is_empty() && condition != "0"
    } else {
        #[cfg(windows)]
        { std::process::Command::new("pwsh").args(["-NoProfile", "-Command", condition]).status().map(|s| s.success()).unwrap_or(false) }
        #[cfg(not(windows))]
        { std::process::Command::new("sh").args(["-c", condition]).status().map(|s| s.success()).unwrap_or(false) }
    };

    let cmd_to_run = if success { Some(true_cmd) } else { false_cmd };
    if let Some(cmd) = cmd_to_run {
        // Execute the branch as a config line (recursive — supports set, bind, source, etc.)
        parse_config_line(app, cmd);
    }
}
