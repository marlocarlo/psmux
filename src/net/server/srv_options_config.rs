use super::*;
use super::srv_loop_ctx::LoopCtx;

pub(crate) fn handle_set_option(app: &mut AppState, ctx: &mut LoopCtx, option: String, value: String) {
    apply_set_option(app, &option, &value, false);
    app.user_set_options.insert(option.clone());
    if option == "command-alias" {
        if let Ok(mut map) = ctx.shared_aliases.write() { *map = app.command_aliases.clone(); }
    }
    ctx.meta_dirty = true; ctx.state_dirty = true;
}

pub(crate) fn handle_set_option_quiet(app: &mut AppState, ctx: &mut LoopCtx, option: String, value: String, quiet: bool) {
    let old_shell = app.default_shell.clone();
    apply_set_option(app, &option, &value, quiet);
    app.user_set_options.insert(option.clone());
    if app.default_shell != old_shell {
        if let Some(mut wp) = app.warm_pane.take() { wp.child.kill().ok(); }
    }
    if option == "command-alias" {
        if let Ok(mut map) = ctx.shared_aliases.write() { *map = app.command_aliases.clone(); }
    }
    ctx.meta_dirty = true; ctx.state_dirty = true;
}

pub(crate) fn handle_set_option_unset(app: &mut AppState, option: &str) {
    if option.starts_with('@') {
        app.user_options.remove(option);
    } else {
        match option {
            "status-left" => { app.status_left = "psmux:#I".to_string(); }
            "status-right" => { app.status_right = "#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y".to_string(); }
            "mouse" => { app.mouse_enabled = true; }
            "scroll-enter-copy-mode" => { app.scroll_enter_copy_mode = true; }
            "pwsh-mouse-selection" => { app.pwsh_mouse_selection = false; }
            "escape-time" => { app.escape_time_ms = 500; }
            "history-limit" => { app.history_limit = 2000; }
            "display-time" => { app.display_time_ms = 750; }
            "mode-keys" => { app.mode_keys = "emacs".to_string(); }
            "status" => { app.status_visible = true; }
            "status-position" => { app.status_position = "bottom".to_string(); }
            "status-style" => { app.status_style = String::new(); }
            "renumber-windows" => { app.renumber_windows = false; }
            "remain-on-exit" => { app.remain_on_exit = false; }
            "destroy-unattached" => { app.destroy_unattached = false; }
            "exit-empty" => { app.exit_empty = true; }
            "automatic-rename" => { app.automatic_rename = true; }
            "pane-border-style" => { app.pane_border_style = String::new(); }
            "pane-active-border-style" => { app.pane_active_border_style = "fg=green".to_string(); }
            "pane-border-hover-style" => { app.pane_border_hover_style = "fg=yellow".to_string(); }
            "window-status-format" => { app.window_status_format = "#I:#W#{?window_flags,#{window_flags}, }".to_string(); }
            "window-status-current-format" => { app.window_status_current_format = "#I:#W#{?window_flags,#{window_flags}, }".to_string(); }
            "window-status-separator" => { app.window_status_separator = " ".to_string(); }
            "cursor-style" => { std::env::set_var("PSMUX_CURSOR_STYLE", "bar"); }
            "cursor-blink" => { std::env::set_var("PSMUX_CURSOR_BLINK", "1"); }
            _ => {}
        }
    }
}

pub(crate) fn handle_set_option_only_if_unset(app: &mut AppState, ctx: &mut LoopCtx, option: String, value: String) {
    let already_set = if option.starts_with('@') {
        app.user_options.contains_key(&option)
    } else {
        app.user_set_options.contains(&option)
    };
    if !already_set {
        apply_set_option(app, &option, &value, false);
        app.user_set_options.insert(option.clone());
        if option == "command-alias" {
            if let Ok(mut map) = ctx.shared_aliases.write() { *map = app.command_aliases.clone(); }
        }
        ctx.meta_dirty = true; ctx.state_dirty = true;
    }
}

pub(crate) fn handle_show_options(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
    let mut output = String::new();
    output.push_str(&format!("prefix {}\n", format_key_binding(&app.prefix_key)));
    if let Some(ref p2) = app.prefix2_key { output.push_str(&format!("prefix2 {}\n", format_key_binding(p2))); }
    output.push_str(&format!("base-index {}\n", app.window_base_index));
    output.push_str(&format!("pane-base-index {}\n", app.pane_base_index));
    output.push_str(&format!("escape-time {}\n", app.escape_time_ms));
    output.push_str(&format!("mouse {}\n", if app.mouse_enabled { "on" } else { "off" }));
    output.push_str(&format!("scroll-enter-copy-mode {}\n", if app.scroll_enter_copy_mode { "on" } else { "off" }));
    output.push_str(&format!("pwsh-mouse-selection {}\n", if app.pwsh_mouse_selection { "on" } else { "off" }));
    output.push_str(&format!("status {}\n", if app.status_visible { "on" } else { "off" }));
    output.push_str(&format!("status-position {}\n", app.status_position));
    output.push_str(&format!("status-left \"{}\"\n", app.status_left));
    output.push_str(&format!("status-right \"{}\"\n", app.status_right));
    output.push_str(&format!("history-limit {}\n", app.history_limit));
    output.push_str(&format!("display-time {}\n", app.display_time_ms));
    output.push_str(&format!("display-panes-time {}\n", app.display_panes_time_ms));
    output.push_str(&format!("mode-keys {}\n", app.mode_keys));
    output.push_str(&format!("focus-events {}\n", if app.focus_events { "on" } else { "off" }));
    output.push_str(&format!("renumber-windows {}\n", if app.renumber_windows { "on" } else { "off" }));
    output.push_str(&format!("automatic-rename {}\n", if app.automatic_rename { "on" } else { "off" }));
    output.push_str(&format!("monitor-activity {}\n", if app.monitor_activity { "on" } else { "off" }));
    output.push_str(&format!("synchronize-panes {}\n", if app.sync_input { "on" } else { "off" }));
    output.push_str(&format!("remain-on-exit {}\n", if app.remain_on_exit { "on" } else { "off" }));
    output.push_str(&format!("destroy-unattached {}\n", if app.destroy_unattached { "on" } else { "off" }));
    output.push_str(&format!("exit-empty {}\n", if app.exit_empty { "on" } else { "off" }));
    output.push_str(&format!("set-titles {}\n", if app.set_titles { "on" } else { "off" }));
    if !app.set_titles_string.is_empty() { output.push_str(&format!("set-titles-string \"{}\"\n", app.set_titles_string)); }
    output.push_str(&format!("prediction-dimming {}\n", if app.prediction_dimming { "on" } else { "off" }));
    output.push_str(&format!("allow-predictions {}\n", if app.allow_predictions { "on" } else { "off" }));
    output.push_str(&format!("cursor-style {}\n", std::env::var("PSMUX_CURSOR_STYLE").unwrap_or_else(|_| "bar".to_string())));
    output.push_str(&format!("cursor-blink {}\n", if std::env::var("PSMUX_CURSOR_BLINK").unwrap_or_else(|_| "1".to_string()) != "0" { "on" } else { "off" }));
    { let shell_val = if app.default_shell.is_empty() { crate::pane::cached_shell().unwrap_or("pwsh.exe").to_string() } else { app.default_shell.clone() }; output.push_str(&format!("default-shell {}\n", shell_val)); }
    output.push_str(&format!("word-separators \"{}\"\n", app.word_separators));
    if !app.pane_border_style.is_empty() { output.push_str(&format!("pane-border-style \"{}\"\n", app.pane_border_style)); }
    if !app.pane_active_border_style.is_empty() { output.push_str(&format!("pane-active-border-style \"{}\"\n", app.pane_active_border_style)); }
    if !app.pane_border_hover_style.is_empty() { output.push_str(&format!("pane-border-hover-style \"{}\"\n", app.pane_border_hover_style)); }
    if !app.status_style.is_empty() { output.push_str(&format!("status-style \"{}\"\n", app.status_style)); }
    if !app.status_left_style.is_empty() { output.push_str(&format!("status-left-style \"{}\"\n", app.status_left_style)); }
    if !app.status_right_style.is_empty() { output.push_str(&format!("status-right-style \"{}\"\n", app.status_right_style)); }
    output.push_str(&format!("status-interval {}\n", app.status_interval));
    output.push_str(&format!("status-justify {}\n", app.status_justify));
    output.push_str(&format!("window-status-format \"{}\"\n", app.window_status_format));
    output.push_str(&format!("window-status-current-format \"{}\"\n", app.window_status_current_format));
    if !app.window_status_style.is_empty() { output.push_str(&format!("window-status-style \"{}\"\n", app.window_status_style)); }
    if !app.window_status_current_style.is_empty() { output.push_str(&format!("window-status-current-style \"{}\"\n", app.window_status_current_style)); }
    if !app.window_status_activity_style.is_empty() { output.push_str(&format!("window-status-activity-style \"{}\"\n", app.window_status_activity_style)); }
    if !app.message_style.is_empty() { output.push_str(&format!("message-style \"{}\"\n", app.message_style)); }
    if !app.message_command_style.is_empty() { output.push_str(&format!("message-command-style \"{}\"\n", app.message_command_style)); }
    if !app.mode_style.is_empty() { output.push_str(&format!("mode-style \"{}\"\n", app.mode_style)); }
    for (key, val) in &app.user_options { output.push_str(&format!("{} \"{}\"\n", key, val)); }
    output.push_str(&format!("main-pane-width {}\n", app.main_pane_width));
    output.push_str(&format!("main-pane-height {}\n", app.main_pane_height));
    output.push_str(&format!("status-left-length {}\n", app.status_left_length));
    output.push_str(&format!("status-right-length {}\n", app.status_right_length));
    output.push_str(&format!("window-size {}\n", app.window_size));
    output.push_str(&format!("allow-passthrough {}\n", app.allow_passthrough));
    output.push_str(&format!("set-clipboard {}\n", app.set_clipboard));
    if !app.copy_command.is_empty() { output.push_str(&format!("copy-command \"{}\"\n", app.copy_command)); }
    output.push_str(&format!("allow-rename {}\n", if app.allow_rename { "on" } else { "off" }));
    output.push_str(&format!("allow-set-title {}\n", if app.allow_set_title { "on" } else { "off" }));
    output.push_str(&format!("bell-action {}\n", app.bell_action));
    output.push_str(&format!("activity-action {}\n", app.activity_action));
    output.push_str(&format!("silence-action {}\n", app.silence_action));
    output.push_str(&format!("update-environment \"{}\"\n", app.update_environment.join(" ")));
    if let Some(ref group) = app.session_group { output.push_str(&format!("session-group \"{}\"\n", group)); }
    for (alias, expansion) in &app.command_aliases { output.push_str(&format!("command-alias \"{}={}\"\n", alias, expansion)); }
    let _ = resp.send(output);
}

pub(crate) fn handle_source_file(app: &mut AppState, path: String) {
    app.defaults_suppressed = false;
    app.key_tables.clear();
    crate::config::populate_default_bindings(app);
    let is_format_expand = path.starts_with("-F ") || path.starts_with("-F\t");
    let path_for_glob = if is_format_expand { path[3..].trim() } else { &path };
    if !is_format_expand && (path_for_glob.contains('*') || path_for_glob.contains('?')) {
        let expanded = if path_for_glob.starts_with('~') {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            path_for_glob.replacen('~', &home, 1)
        } else { path_for_glob.to_string() };
        if let Ok(entries) = glob::glob(&expanded) {
            for entry in entries.flatten() {
                if let Ok(contents) = std::fs::read_to_string(&entry) { parse_config_content(app, &contents); }
            }
        }
    } else {
        crate::config::source_file(app, &path);
    }
}

pub(crate) fn handle_bind_key(app: &mut AppState, table_name: String, key: String, command: String, repeat: bool) {
    if let Some(kc) = parse_key_string(&key) {
        let kc = normalize_key_for_binding(kc);
        let sub_cmds = crate::config::split_chained_commands_pub(&command);
        let action = if sub_cmds.len() > 1 { Some(Action::CommandChain(sub_cmds)) } else { parse_command_to_action(&command) };
        if let Some(act) = action {
            let table = app.key_tables.entry(table_name).or_default();
            table.retain(|b| b.key != kc);
            table.push(Bind { key: kc, action: act, repeat });
        }
    }
}

pub(crate) fn handle_list_keys(app: &AppState, resp: std::sync::mpsc::Sender<String>) {
    let user_iter = app.key_tables.iter().flat_map(|(table_name, binds)| {
        binds.iter().map(move |bind| {
            let key_str = format_key_binding(&bind.key);
            let action_str = format_action(&bind.action);
            (table_name.as_str(), key_str, action_str, bind.repeat)
        })
    });
    let output = help::build_list_keys_output(user_iter, app.defaults_suppressed);
    let _ = resp.send(output);
}
