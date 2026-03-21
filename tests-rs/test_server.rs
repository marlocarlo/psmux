use super::should_spawn_warm_server;
use crate::types::AppState;

// ── Hook set/replace/unset tests (issue #133) ───────────────────

#[test]
fn set_hook_replaces_existing_hook() {
    let mut app = AppState::new("test".to_string());
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'display-message first'");
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'display-message second'");
    let cmds = app.hooks.get("client-attached").unwrap();
    assert_eq!(cmds.len(), 1, "hook should be replaced, not appended");
    assert_eq!(cmds[0], "display-message second");
}

#[test]
fn set_hook_unset_removes_hook() {
    let mut app = AppState::new("test".to_string());
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'display-message hello'");
    assert!(app.hooks.contains_key("client-attached"));
    crate::config::parse_config_line(&mut app, "set-hook -gu client-attached");
    assert!(!app.hooks.contains_key("client-attached"), "hook should be removed by -gu");
}

#[test]
fn set_hook_different_hooks_coexist() {
    let mut app = AppState::new("test".to_string());
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'display-message a'");
    crate::config::parse_config_line(&mut app, "set-hook -g after-new-window 'display-message b'");
    assert_eq!(app.hooks.len(), 2);
    assert_eq!(app.hooks["client-attached"][0], "display-message a");
    assert_eq!(app.hooks["after-new-window"][0], "display-message b");
}

#[test]
fn set_hook_replace_preserves_other_hooks() {
    let mut app = AppState::new("test".to_string());
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'cmd-a'");
    crate::config::parse_config_line(&mut app, "set-hook -g after-new-window 'cmd-b'");
    // Replace client-attached — after-new-window should be untouched
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'cmd-c'");
    assert_eq!(app.hooks["client-attached"], vec!["cmd-c"]);
    assert_eq!(app.hooks["after-new-window"], vec!["cmd-b"]);
}

#[test]
fn set_hook_unset_with_u_flag() {
    let mut app = AppState::new("test".to_string());
    crate::config::parse_config_line(&mut app, "set-hook -g client-attached 'hello'");
    crate::config::parse_config_line(&mut app, "set-hook -u client-attached");
    assert!(!app.hooks.contains_key("client-attached"), "hook should be removed by -u");
}

#[test]
fn warm_server_is_disabled_for_destroy_unattached_sessions() {
    let mut app = AppState::new("demo".to_string());
    app.destroy_unattached = true;
    assert!(!should_spawn_warm_server(&app));
}

#[test]
fn warm_server_is_disabled_for_warm_session_itself() {
    let app = AppState::new("__warm__".to_string());
    assert!(!should_spawn_warm_server(&app));
}

#[test]
fn warm_server_is_allowed_for_normal_sessions() {
    let app = AppState::new("demo".to_string());
    assert!(should_spawn_warm_server(&app));
}

// ── Options get/set tests ───────────────────────────────────────

#[test]
fn get_option_allow_rename() {
    let app = AppState::new("test".to_string());
    let val = super::options::get_option_value(&app, "allow-rename");
    assert_eq!(val, "on");
}

#[test]
fn get_option_bell_action() {
    let app = AppState::new("test".to_string());
    let val = super::options::get_option_value(&app, "bell-action");
    assert_eq!(val, "any");
}

#[test]
fn get_option_activity_action() {
    let app = AppState::new("test".to_string());
    let val = super::options::get_option_value(&app, "activity-action");
    assert_eq!(val, "other");
}

#[test]
fn get_option_silence_action() {
    let app = AppState::new("test".to_string());
    let val = super::options::get_option_value(&app, "silence-action");
    assert_eq!(val, "other");
}

#[test]
fn get_option_update_environment() {
    let app = AppState::new("test".to_string());
    let val = super::options::get_option_value(&app, "update-environment");
    assert!(val.contains("DISPLAY"));
    assert!(val.contains("SSH_AUTH_SOCK"));
}

#[test]
fn set_option_allow_rename_off() {
    let mut app = AppState::new("test".to_string());
    super::options::apply_set_option(&mut app, "allow-rename", "off", false);
    assert!(!app.allow_rename);
}

#[test]
fn set_option_activity_action() {
    let mut app = AppState::new("test".to_string());
    super::options::apply_set_option(&mut app, "activity-action", "any", false);
    assert_eq!(app.activity_action, "any");
}

#[test]
fn set_option_silence_action() {
    let mut app = AppState::new("test".to_string());
    super::options::apply_set_option(&mut app, "silence-action", "none", false);
    assert_eq!(app.silence_action, "none");
}

// ── Root table binding tests (discussion #130: vim-style C-hjkl nav) ────

use crossterm::event::{KeyCode, KeyModifiers};
use crate::config::{parse_key_string, normalize_key_for_binding, parse_bind_key};
use crate::types::{Action, FocusDir};

#[test]
fn bind_key_n_creates_root_binding() {
    let mut app = AppState::new("test".to_string());
    parse_bind_key(&mut app, "bind-key -n C-h select-pane -L");
    let root = app.key_tables.get("root").expect("root table should exist");
    assert_eq!(root.len(), 1, "root table should have one binding");
    let bind = &root[0];
    assert!(matches!(bind.action, Action::MoveFocus(FocusDir::Left)),
        "C-h should be bound to select-pane -L");
}

#[test]
fn bind_key_n_all_vim_directions() {
    let mut app = AppState::new("test".to_string());
    parse_bind_key(&mut app, "bind-key -n C-h select-pane -L");
    parse_bind_key(&mut app, "bind-key -n C-j select-pane -D");
    parse_bind_key(&mut app, "bind-key -n C-k select-pane -U");
    parse_bind_key(&mut app, "bind-key -n C-l select-pane -R");
    let root = app.key_tables.get("root").expect("root table should exist");
    assert_eq!(root.len(), 4, "root table should have four bindings");

    let expected = [
        ('h', FocusDir::Left),
        ('j', FocusDir::Down),
        ('k', FocusDir::Up),
        ('l', FocusDir::Right),
    ];
    for (ch, dir) in expected {
        let key = normalize_key_for_binding((KeyCode::Char(ch), KeyModifiers::CONTROL));
        let bind = root.iter().find(|b| b.key == key)
            .unwrap_or_else(|| panic!("binding for C-{} should exist", ch));
        assert!(matches!(&bind.action, Action::MoveFocus(d) if *d == dir),
            "C-{} should be bound to {:?}", ch, dir);
    }
}

#[test]
fn ctrl_h_binding_matches_windows_key_event() {
    // On Windows, Ctrl+H is reported as Char('h') + CONTROL by crossterm
    let mut app = AppState::new("test".to_string());
    parse_bind_key(&mut app, "bind-key -n C-h select-pane -L");
    let root = app.key_tables.get("root").unwrap();

    let win_key = normalize_key_for_binding((KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert!(root.iter().any(|b| b.key == win_key),
        "C-h binding must match Char('h')+CONTROL key event");
}

#[test]
fn backspace_and_ctrl_h_are_distinct_on_windows() {
    // On Windows, Backspace and Ctrl+H are distinct keys — they must NOT alias
    let backspace = normalize_key_for_binding((KeyCode::Backspace, KeyModifiers::empty()));
    let ctrl_h = normalize_key_for_binding((KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_ne!(backspace, ctrl_h,
        "Backspace and C-h must be distinct on Windows (no Unix aliasing)");
}

#[test]
fn tab_and_ctrl_i_are_distinct_on_windows() {
    let tab = normalize_key_for_binding((KeyCode::Tab, KeyModifiers::empty()));
    let ctrl_i = normalize_key_for_binding((KeyCode::Char('i'), KeyModifiers::CONTROL));
    assert_ne!(tab, ctrl_i,
        "Tab and C-i must be distinct on Windows");
}

#[test]
fn enter_and_ctrl_m_are_distinct_on_windows() {
    let enter = normalize_key_for_binding((KeyCode::Enter, KeyModifiers::empty()));
    let ctrl_m = normalize_key_for_binding((KeyCode::Char('m'), KeyModifiers::CONTROL));
    assert_ne!(enter, ctrl_m,
        "Enter and C-m must be distinct on Windows");
}

#[test]
fn normalize_only_strips_shift_from_char() {
    // Regular keys: SHIFT stripped from Char events
    let shifted = normalize_key_for_binding((KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert_eq!(shifted, (KeyCode::Char('A'), KeyModifiers::empty()));

    // Non-Char keys: modifiers preserved
    let ctrl_l = normalize_key_for_binding((KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert_eq!(ctrl_l, (KeyCode::Char('l'), KeyModifiers::CONTROL));

    let shift_bs = normalize_key_for_binding((KeyCode::Backspace, KeyModifiers::SHIFT));
    assert_eq!(shift_bs, (KeyCode::Backspace, KeyModifiers::SHIFT));
}
