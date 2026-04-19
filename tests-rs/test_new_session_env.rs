//! new-session -e: session environment merges into app.environment and format expansion.

use crate::types::AppState;
use crate::config::parse_config_content;
use crate::format::expand_format;

#[test]
fn session_env_merge_after_config_visible_in_expand_format() {
    let mut app = AppState::new("sess_e".to_string());
    parse_config_content(&mut app, "");
    let env = vec![("PSMUX_NS_E_TEST".to_string(), "from_cli".to_string())];
    crate::util::merge_session_env_into_app(&mut app, &env);
    assert_eq!(
        app.environment.get("PSMUX_NS_E_TEST").map(|s| s.as_str()),
        Some("from_cli")
    );
    let out = expand_format("#{PSMUX_NS_E_TEST}", &app);
    assert_eq!(out, "from_cli");
}

/// tmux: repeated `-e` for the same variable, last wins (HashMap insert order).
#[test]
fn session_env_merge_last_wins_duplicate_variable() {
    let mut app = AppState::new("sess_e2".to_string());
    parse_config_content(&mut app, "");
    let env = vec![
        ("PSMUX_DUP_E".to_string(), "first".to_string()),
        ("PSMUX_DUP_E".to_string(), "last".to_string()),
    ];
    crate::util::merge_session_env_into_app(&mut app, &env);
    assert_eq!(app.environment.get("PSMUX_DUP_E").map(|s| s.as_str()), Some("last"));
    assert_eq!(expand_format("#{PSMUX_DUP_E}", &app), "last");
}
