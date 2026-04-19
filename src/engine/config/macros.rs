#[allow(unused_imports)]
use std::env;
use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::types::{AppState, Action, Bind};
use crate::commands::parse_command_to_action;

// Track the current config file being parsed (for #{current_file}, #{d:current_file})
use super::*;

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
pub(crate) fn expand_hidden_vars(line: &str, env: &std::collections::HashMap<String, String>) -> String {
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
