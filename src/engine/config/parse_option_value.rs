#[allow(unused_imports)]
use std::env;
use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::types::{AppState, Action, Bind};
use crate::commands::parse_command_to_action;

// Track the current config file being parsed (for #{current_file}, #{d:current_file})
use super::*;

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
        "allow-set-title" => {
            app.allow_set_title = matches!(value, "on" | "true" | "1");
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

pub(crate) fn split_chained_commands(command: &str) -> Vec<String> {
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
