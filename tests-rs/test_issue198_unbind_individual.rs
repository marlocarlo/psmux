// Tests for issue #198: unbind-key for individual keys does not work.
//
// Root cause: Default prefix bindings are hardcoded in client.rs and
// PREFIX_DEFAULTS (help.rs), NOT in key_tables.  When unbind-key removes
// a key from key_tables, there is nothing to remove since defaults were
// never there.  The hardcoded dispatch still fires.
//
// Validates:
// 1. unbind-key <key> removes the key from the correct table
// 2. unbind-key -n <key> removes only from the root table
// 3. unbind-key -T <table> <key> removes only from that table
// 4. list-keys output reflects individually unbound keys
// 5. Defaults populated in key_tables are removable via unbind-key

use super::*;

fn mock_app() -> AppState {
    AppState::new("test_session".to_string())
}

// ═══════════════════════════════════════════════════════════════════
//  BUG PROOF: unbind-key <key> should remove default prefix bindings
// ═══════════════════════════════════════════════════════════════════

#[test]
fn unbind_key_d_removes_detach_from_defaults() {
    // The user wants to unbind 'd' (detach-client) from prefix table.
    // After unbind-key d, list-keys should NOT show prefix d detach-client.
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Verify 'd' is in the prefix table
    let prefix = app.key_tables.get("prefix").expect("prefix table should exist");
    let has_d = prefix.iter().any(|b| {
        matches!(b.key.0, KeyCode::Char('d'))
    });
    assert!(has_d, "'d' should be in prefix table after populating defaults");

    // Unbind 'd'
    parse_unbind_key(&mut app, "unbind-key d");

    // Verify 'd' is no longer in the prefix table
    let prefix = app.key_tables.get("prefix").expect("prefix table should still exist");
    let has_d_after = prefix.iter().any(|b| {
        matches!(b.key.0, KeyCode::Char('d'))
    });
    assert!(!has_d_after, "'d' should be removed from prefix table after unbind-key d");
}

#[test]
fn unbind_key_c_removes_new_window() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    let prefix = app.key_tables.get("prefix").unwrap();
    let has_c = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('c')));
    assert!(has_c, "'c' should be in prefix table");

    parse_unbind_key(&mut app, "unbind-key c");

    let prefix = app.key_tables.get("prefix").unwrap();
    let has_c = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('c')));
    assert!(!has_c, "'c' should be removed after unbind-key c");
}

// ═══════════════════════════════════════════════════════════════════
//  unbind-key with -n (root table)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn unbind_key_n_removes_from_root_only() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Add a root table binding for C-v
    parse_bind_key(&mut app, "bind-key -n C-v send-keys foo");

    // Also add a prefix table binding for C-v
    parse_bind_key(&mut app, "bind-key C-v send-keys bar");

    // Verify both exist
    let root = app.key_tables.get("root").expect("root table should exist");
    let has_cv_root = root.iter().any(|b| {
        matches!(b.key.0, KeyCode::Char('v')) && b.key.1.contains(KeyModifiers::CONTROL)
    });
    assert!(has_cv_root, "C-v should be in root table");

    let prefix = app.key_tables.get("prefix").unwrap();
    let has_cv_prefix = prefix.iter().any(|b| {
        matches!(b.key.0, KeyCode::Char('v')) && b.key.1.contains(KeyModifiers::CONTROL)
    });
    assert!(has_cv_prefix, "C-v should be in prefix table");

    // unbind-key -n C-v should only remove from root
    parse_unbind_key(&mut app, "unbind-key -n C-v");

    let root = app.key_tables.get("root").expect("root table should still exist");
    let has_cv_root_after = root.iter().any(|b| {
        matches!(b.key.0, KeyCode::Char('v')) && b.key.1.contains(KeyModifiers::CONTROL)
    });
    assert!(!has_cv_root_after, "C-v should be removed from root table after unbind-key -n C-v");

    // Prefix table should be UNTOUCHED
    let prefix = app.key_tables.get("prefix").unwrap();
    let has_cv_prefix_after = prefix.iter().any(|b| {
        matches!(b.key.0, KeyCode::Char('v')) && b.key.1.contains(KeyModifiers::CONTROL)
    });
    assert!(has_cv_prefix_after, "C-v should still exist in prefix table (only root was unbound)");
}

// ═══════════════════════════════════════════════════════════════════
//  unbind-key with -T <table>
// ═══════════════════════════════════════════════════════════════════

#[test]
fn unbind_key_t_removes_from_specific_table() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Add bindings to both root and prefix
    parse_bind_key(&mut app, "bind-key -n F5 send-keys test1");
    parse_bind_key(&mut app, "bind-key F5 send-keys test2");

    // Unbind from root only via -T
    parse_unbind_key(&mut app, "unbind-key -T root F5");

    let root = app.key_tables.get("root").expect("root table should exist");
    let has_f5_root = root.iter().any(|b| matches!(b.key.0, KeyCode::F(5)));
    assert!(!has_f5_root, "F5 should be removed from root table");

    let prefix = app.key_tables.get("prefix").unwrap();
    let has_f5_prefix = prefix.iter().any(|b| matches!(b.key.0, KeyCode::F(5)));
    assert!(has_f5_prefix, "F5 should still exist in prefix table");
}

#[test]
fn unbind_key_t_prefix_removes_from_prefix() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Unbind 'n' specifically from prefix table
    parse_unbind_key(&mut app, "unbind-key -T prefix n");

    let prefix = app.key_tables.get("prefix").unwrap();
    let has_n = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('n')));
    assert!(!has_n, "'n' (next-window) should be removed from prefix after unbind-key -T prefix n");
}

// ═══════════════════════════════════════════════════════════════════
//  unbind-key without flags defaults to prefix table (tmux behavior)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn unbind_key_no_flags_defaults_to_prefix() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Add 'x' to root table too
    parse_bind_key(&mut app, "bind-key -n x send-keys rootx");

    // unbind-key x (no flags) should remove from prefix only (tmux default)
    parse_unbind_key(&mut app, "unbind-key x");

    let prefix = app.key_tables.get("prefix").unwrap();
    let has_x_prefix = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('x')));
    assert!(!has_x_prefix, "'x' should be removed from prefix table");

    let root = app.key_tables.get("root").unwrap();
    let has_x_root = root.iter().any(|b| matches!(b.key.0, KeyCode::Char('x')));
    assert!(has_x_root, "'x' should STILL be in root table (unbind-key defaults to prefix)");
}

// ═══════════════════════════════════════════════════════════════════
//  list-keys reflects individually unbound keys
// ═══════════════════════════════════════════════════════════════════

#[test]
fn list_keys_does_not_show_unbound_individual_key() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Unbind 'd' (detach-client)
    parse_unbind_key(&mut app, "unbind-key d");

    // Build list-keys output
    let user_iter = app.key_tables.iter().flat_map(|(table_name, binds)| {
        binds.iter().map(move |bind| {
            let key_str = format_key_binding(&bind.key);
            let action_str = crate::commands::format_action(&bind.action);
            (table_name.as_str(), key_str, action_str, bind.repeat)
        })
    });
    let output = crate::help::build_list_keys_output(user_iter, app.defaults_suppressed);

    // 'd' should NOT appear in output
    let has_detach = output.lines().any(|l| {
        l.contains(" d ") && l.contains("detach")
    });
    assert!(!has_detach, "list-keys should not show 'd detach-client' after unbind-key d, got:\n{}", output);
}

#[test]
fn list_keys_shows_remaining_defaults_after_individual_unbind() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Unbind only 'd'
    parse_unbind_key(&mut app, "unbind-key d");

    let user_iter = app.key_tables.iter().flat_map(|(table_name, binds)| {
        binds.iter().map(move |bind| {
            let key_str = format_key_binding(&bind.key);
            let action_str = crate::commands::format_action(&bind.action);
            (table_name.as_str(), key_str, action_str, bind.repeat)
        })
    });
    let output = crate::help::build_list_keys_output(user_iter, app.defaults_suppressed);

    // Other defaults like 'c' (new-window) should still be present
    let has_new_window = output.lines().any(|l| {
        l.contains(" c ") && l.contains("new-window")
    });
    assert!(has_new_window, "list-keys should still show 'c new-window' after only unbinding 'd'");
}

// ═══════════════════════════════════════════════════════════════════
//  Config file with unbind-key works end to end
// ═══════════════════════════════════════════════════════════════════

#[test]
fn config_unbind_key_d_then_rebind_works() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Parse a config that unbinds 'd' and rebinds it to something else
    parse_config_content(&mut app, "unbind-key d\nbind-key d display-message \"custom\"");

    let prefix = app.key_tables.get("prefix").unwrap();
    let d_bind = prefix.iter().find(|b| matches!(b.key.0, KeyCode::Char('d')));
    assert!(d_bind.is_some(), "'d' should exist in prefix after rebind");

    // Verify it's the NEW binding, not detach-client
    let d = d_bind.unwrap();
    let action_str = crate::commands::format_action(&d.action);
    assert!(!action_str.contains("detach"), "'d' should no longer be detach-client");
}

#[test]
fn config_unbind_all_prefix_then_rebind_selective() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // This is the exact use case from issue #195 user
    let config = r#"
unbind-key -a
set -g prefix C-a
unbind-key C-b
bind-key C-a send-prefix
bind-key C-r source-file ~/.tmux.conf
"#;
    parse_config_content(&mut app, config);

    assert!(app.defaults_suppressed, "defaults_suppressed should be true after unbind-key -a");

    // Only user-defined bindings should exist
    let all_bindings: Vec<_> = app.key_tables.values().flat_map(|v| v.iter()).collect();
    assert_eq!(all_bindings.len(), 2, "Should have exactly 2 bindings (C-a and C-r), got {}", all_bindings.len());
}

// ═══════════════════════════════════════════════════════════════════
//  populate_default_bindings correctness
// ═══════════════════════════════════════════════════════════════════

#[test]
fn populate_default_bindings_adds_all_prefix_defaults() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    let prefix = app.key_tables.get("prefix").expect("prefix table should be created");
    assert!(prefix.len() >= 40, "Should have ~50 default prefix bindings, got {}", prefix.len());

    // Spot check some specific defaults
    let has_d = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('d')));
    let has_c = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('c')));
    let has_percent = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('%')));
    let has_question = prefix.iter().any(|b| matches!(b.key.0, KeyCode::Char('?')));

    assert!(has_d, "prefix table should have 'd' (detach-client)");
    assert!(has_c, "prefix table should have 'c' (new-window)");
    assert!(has_percent, "prefix table should have '%%' (split-window -h)");
    assert!(has_question, "prefix table should have '?' (list-keys)");
}

#[test]
fn populate_default_bindings_does_not_add_root_bindings() {
    let mut app = mock_app();
    populate_default_bindings(&mut app);

    // Root table should not exist or be empty (no default root bindings in tmux)
    let root = app.key_tables.get("root");
    assert!(root.is_none() || root.unwrap().is_empty(),
        "root table should not have default bindings");
}
