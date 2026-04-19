use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::BindingEntry;

/// Result of prefix/binding dispatch.
pub(crate) struct PrefixDispatchResult {
    /// Whether to quit (detach-client).
    pub(crate) quit: bool,
    /// Whether choose-tree should be populated after dispatch.
    pub(crate) do_choose_tree: bool,
    /// Whether choose-session should be populated after dispatch.
    pub(crate) do_choose_session: bool,
    /// Whether choose-buffer should be populated after dispatch.
    pub(crate) do_choose_buffer: bool,
    /// Session navigation: Some(true) = next, Some(false) = prev.
    pub(crate) do_session_nav: Option<bool>,
    /// The matched user binding (if any), used for repeat flag check.
    pub(crate) user_binding: Option<BindingEntry>,
}

/// Handle prefix key dispatch: check synced bindings from the server,
/// then fall back to pre-sync hardcoded defaults.
///
/// Returns a `PrefixDispatchResult` describing what happened.
pub(crate) fn handle_prefix_and_bindings(
    state: &mut RunRemoteState,
    key: &crossterm::event::KeyEvent,
    cmd_batch: &mut Vec<String>,
    _home: &str,
    _current_session: &str,
) -> PrefixDispatchResult {
    let mut result = PrefixDispatchResult {
        quit: false,
        do_choose_tree: false,
        do_choose_session: false,
        do_choose_buffer: false,
        do_session_nav: None,
        user_binding: None,
    };

    // Check synced bindings from server (includes defaults from PREFIX_DEFAULTS)
    let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
    let user_binding = state.synced_bindings.iter().find(|b| {
        b.t == "prefix" && parse_key_string(&b.k).map_or(false, |k| normalize_key_for_binding(k) == key_tuple)
    }).cloned();

    if let Some(ref entry) = user_binding {
        // Dispatch binding (handles both defaults and user overrides).
        let cmd = &entry.c;
        if cmd == "detach-client" || cmd == "detach" {
            result.quit = true;
        } else if cmd == "kill-pane" || cmd == "kill-window" {
            state.confirm_cmd = Some(cmd.clone());
        } else if cmd.starts_with("confirm-before") {
            state.confirm_cmd = Some(cmd.clone());
        } else if cmd == "rename-window" {
            state.renaming = true; state.rename_buf.clear();
        } else if cmd == "rename-session" {
            state.renaming = true; state.rename_buf.clear(); state.session_renaming = true;
        } else if cmd == "command-prompt" {
            state.command_input = true; state.command_buf.clear(); state.command_cursor = 0; state.command_history_idx = state.command_history.len();
        } else if cmd == "list-keys" {
            state.keys_viewer_scroll = 0;
            let user_binds: Vec<(bool, String, String, String)> = state.synced_bindings
                .iter()
                .map(|b| (b.r, b.t.clone(), b.k.clone(), b.c.clone()))
                .collect();
            state.keys_viewer_lines = help::build_overlay_lines(&user_binds, state.defaults_suppressed);
            state.keys_viewer = true;
        } else if cmd == "select-window-index" {
            state.window_idx_input = true; state.window_idx_buf.clear();
        } else if cmd == "choose-tree" || cmd == "choose-window" {
            result.do_choose_tree = true;
        } else if cmd == "choose-buffer" || cmd == "chooseb" {
            result.do_choose_buffer = true;
        } else if cmd == "choose-session" {
            result.do_choose_session = true;
        } else if cmd.starts_with("switch-client") {
            result.do_session_nav = Some(cmd.contains("-n"));
        } else {
            // Generic: split on \; for command chaining (issue #192)
            let sub_cmds = crate::config::split_chained_commands_pub(&entry.c);
            for sub in &sub_cmds {
                cmd_batch.push(format!("{}\n", sub));
            }
        }
    } else if state.synced_bindings.is_empty() {
        // Pre-sync hardcoded fallback (only used before first server state sync)
        match key.code {
            KeyCode::Char('c') => { cmd_batch.push("new-window\n".into()); }
            KeyCode::Char('%') => { cmd_batch.push("split-window -h\n".into()); }
            KeyCode::Char('"') => { cmd_batch.push("split-window -v\n".into()); }
            KeyCode::Char('x') => { state.confirm_cmd = Some("kill-pane".into()); }
            KeyCode::Char('&') => { state.confirm_cmd = Some("kill-window".into()); }
            KeyCode::Char('z') => { cmd_batch.push("zoom-pane\n".into()); }
            KeyCode::Char('[') => { cmd_batch.push("copy-enter\n".into()); }
            KeyCode::Char(']') => { cmd_batch.push("paste-buffer\n".into()); }
            KeyCode::Char('{') => { cmd_batch.push("swap-pane -U\n".into()); }
            KeyCode::Char('}') => { cmd_batch.push("swap-pane -D\n".into()); }
            KeyCode::Char('n') => { cmd_batch.push("next-window\n".into()); }
            KeyCode::Char('p') => { cmd_batch.push("previous-window\n".into()); }
            KeyCode::Char('l') => { cmd_batch.push("last-window\n".into()); }
            KeyCode::Char(';') => { cmd_batch.push("last-pane\n".into()); }
            KeyCode::Char(' ') => { cmd_batch.push("next-layout\n".into()); }
            KeyCode::Char('!') => { cmd_batch.push("break-pane\n".into()); }
            KeyCode::Char(d) if d.is_ascii_digit() => {
                let idx = d.to_digit(10).unwrap() as usize;
                cmd_batch.push(format!("select-window {}\n", idx));
            }
            KeyCode::Char('o') => { cmd_batch.push("select-pane -t :.+\n".into()); }
            // Alt+Arrow: resize pane by 5 (must be before plain Arrow)
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -U 5\n".into()); }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -D 5\n".into()); }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -L 5\n".into()); }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -R 5\n".into()); }
            // Ctrl+Arrow: resize pane by 1
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -U 1\n".into()); }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -D 1\n".into()); }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -L 1\n".into()); }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -R 1\n".into()); }
            // Plain Arrow: select pane
            KeyCode::Up => { cmd_batch.push("select-pane -U\n".into()); }
            KeyCode::Down => { cmd_batch.push("select-pane -D\n".into()); }
            KeyCode::Left => { cmd_batch.push("select-pane -L\n".into()); }
            KeyCode::Right => { cmd_batch.push("select-pane -R\n".into()); }
            KeyCode::Char('d') => { result.quit = true; }
            KeyCode::Char(',') => { state.renaming = true; state.rename_buf.clear(); }
            KeyCode::Char('$') => {
                state.renaming = true;
                state.rename_buf.clear();
                state.session_renaming = true;
            }
            KeyCode::Char('?') => {
                state.keys_viewer_scroll = 0;
                let user_binds: Vec<(bool, String, String, String)> = state.synced_bindings
                    .iter()
                    .map(|b| (b.r, b.t.clone(), b.k.clone(), b.c.clone()))
                    .collect();
                state.keys_viewer_lines = help::build_overlay_lines(&user_binds, state.defaults_suppressed);
                state.keys_viewer = true;
            }
            KeyCode::Char('t') => { cmd_batch.push("clock-mode\n".into()); }
            KeyCode::Char('=') => { result.do_choose_buffer = true; }
            KeyCode::Char('#') => { cmd_batch.push("list-buffers\n".into()); }
            KeyCode::Char(':') => { state.command_input = true; state.command_buf.clear(); state.command_cursor = 0; state.command_history_idx = state.command_history.len(); }
            KeyCode::Char('\'') => { state.window_idx_input = true; state.window_idx_buf.clear(); }
            KeyCode::Char('w') => { result.do_choose_tree = true; }
            KeyCode::Char('s') => { result.do_choose_session = true; }
            KeyCode::Char('q') => { cmd_batch.push("display-panes\n".into()); }
            KeyCode::Char('v') => { cmd_batch.push("rectangle-toggle\n".into()); }
            KeyCode::Char('y') => { cmd_batch.push("copy-yank\n".into()); }
            // Session navigation (like tmux prefix+( and prefix+))
            KeyCode::Char('(') | KeyCode::Char(')') => {
                result.do_session_nav = Some(key.code == KeyCode::Char(')'));
            }
            // Meta+1..5 preset layouts (like tmux)
            KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout even-horizontal\n".into()); }
            KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout even-vertical\n".into()); }
            KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout main-horizontal\n".into()); }
            KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout main-vertical\n".into()); }
            KeyCode::Char('5') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout tiled\n".into()); }
            // Display pane info
            KeyCode::Char('i') => { cmd_batch.push("display-message\n".into()); }
            _ => {
                // No default binding for this key
            }
        }
    }

    result.user_binding = user_binding;
    result
}
