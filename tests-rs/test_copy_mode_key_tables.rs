//! `bind -T copy-mode-vi` / `-T copy-mode` bindings must actually fire.
//!
//! Plain characters reach `handle_copy_mode_char`; special keys reach
//! `send_key_to_active`. Both live handlers must consult `app.key_tables`
//! before their built-in copy-mode behavior. The tests bind actions that differ
//! from the defaults so a hardcoded fallback cannot satisfy them.

use super::*;

fn app_vi() -> AppState {
    let mut a = AppState::new("copytbl".to_string());
    a.window_base_index = 0;
    a.mode_keys = "vi".to_string();
    a
}

fn bind_copy_key(app: &mut AppState, table: &str, key: &str, cmd: &str) {
    crate::config::parse_config_line(app, &format!("bind -T {} {} {}", table, key, cmd));
}

// ───────────────────────── table selection ─────────────────────────

#[test]
fn vi_mode_keys_selects_the_copy_mode_vi_table() {
    let mut a = app_vi();
    bind_copy_key(&mut a, "copy-mode-vi", "y", "send-keys -X copy-pipe-and-cancel \"clip.exe\"");

    let action = copy_mode_binding(&a, (KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(
        action.is_some(),
        "a copy-mode-vi binding must resolve when mode-keys is vi"
    );
}

#[test]
fn emacs_mode_keys_selects_the_copy_mode_table() {
    let mut a = app_vi();
    a.mode_keys = "emacs".to_string();
    bind_copy_key(&mut a, "copy-mode", "w", "send-keys -X copy-selection");

    assert!(
        copy_mode_binding(&a, (KeyCode::Char('w'), KeyModifiers::NONE)).is_some(),
        "a copy-mode binding must resolve when mode-keys is not vi"
    );
}

#[test]
fn the_wrong_table_is_not_consulted() {
    let mut a = app_vi(); // vi -> copy-mode-vi
    bind_copy_key(&mut a, "copy-mode", "w", "send-keys -X copy-selection");

    assert!(
        copy_mode_binding(&a, (KeyCode::Char('w'), KeyModifiers::NONE)).is_none(),
        "a copy-mode binding must NOT fire while mode-keys is vi"
    );
}

#[test]
fn an_unbound_key_resolves_to_nothing() {
    let a = app_vi();
    assert!(
        copy_mode_binding(&a, (KeyCode::Char('j'), KeyModifiers::NONE)).is_none(),
        "unbound keys must fall through to the built-in copy-mode handling"
    );
}

/// Shift is stripped when matching, so a binding on `V` is found by the
/// shifted keypress that produces it.
#[test]
fn shift_is_normalised_away_when_matching() {
    let mut a = app_vi();
    bind_copy_key(&mut a, "copy-mode-vi", "V", "send-keys -X select-line");

    assert!(
        copy_mode_binding(&a, (KeyCode::Char('V'), KeyModifiers::SHIFT)).is_some(),
        "a shifted character keypress must match its binding"
    );
}

#[test]
fn modified_keys_resolve() {
    let mut a = app_vi();
    bind_copy_key(&mut a, "copy-mode-vi", "C-v", "send-keys -X rectangle-toggle");

    assert!(
        copy_mode_binding(&a, (KeyCode::Char('v'), KeyModifiers::CONTROL)).is_some(),
        "C-v in a copy-mode table must resolve for a real Ctrl+v keypress"
    );
}

// ───────────────────────── dispatch ─────────────────────────

/// `send-keys -X` is queued back onto the server loop, where the copy-mode
/// command implementation lives.
#[test]
fn send_keys_x_bindings_are_queued_with_their_arguments_intact() {
    let mut a = app_vi();
    let (tx, rx) = crate::types::control_channel();
    a.control_tx = Some(tx);
    bind_copy_key(&mut a, "copy-mode-vi", "y", "send-keys -X copy-pipe-and-cancel \"clip.exe\"");

    let action = copy_mode_binding(&a, (KeyCode::Char('y'), KeyModifiers::NONE)).unwrap();
    assert!(run_copy_mode_binding(&mut a, &action), "binding should be handled");

    match rx.try_recv() {
        Ok(crate::types::CtrlReq::SendKeysX(cmd)) => {
            assert!(
                cmd.starts_with("copy-pipe-and-cancel"),
                "queued the wrong copy-mode command: {:?}",
                cmd
            );
            assert!(
                cmd.contains("clip.exe"),
                "the pipe target was dropped: {:?} — yanking would work but the \
                 user's clip.exe would never run, which is the original bug",
                cmd
            );
        }
        other => panic!("expected a queued SendKeysX, got {:?}", other.is_ok()),
    }
}

/// A quoted argument containing spaces must survive — the command is rebuilt
/// from the original string, not from a whitespace split.
#[test]
fn a_quoted_argument_with_spaces_survives() {
    let mut a = app_vi();
    let (tx, rx) = crate::types::control_channel();
    a.control_tx = Some(tx);
    bind_copy_key(
        &mut a,
        "copy-mode-vi",
        "y",
        "send-keys -X copy-pipe-and-cancel \"pwsh -NoProfile -Command Set-Clipboard\"",
    );

    let action = copy_mode_binding(&a, (KeyCode::Char('y'), KeyModifiers::NONE)).unwrap();
    run_copy_mode_binding(&mut a, &action);

    match rx.try_recv() {
        Ok(crate::types::CtrlReq::SendKeysX(cmd)) => {
            assert!(
                cmd.contains("-NoProfile") && cmd.contains("Set-Clipboard"),
                "multi-word pipe command was truncated: {:?}",
                cmd
            );
            // The SendKeysX arm hands the tail after the copy-mode command
            // name to `pwsh -Command` verbatim. A surviving quote character
            // turns the pipe command into a string literal that pwsh
            // evaluates and discards, so the queued command must carry the
            // words with their quote grouping stripped, exactly like the TCP
            // dispatcher's `has_x` arm.
            assert!(
                !cmd.contains('"'),
                "quote characters must not reach the SendKeysX arm: {:?}",
                cmd
            );
        }
        _ => panic!("expected a queued SendKeysX"),
    }
}

/// With no sender wired the binding must decline to handle the key, so the
/// caller falls through to the built-in behaviour instead of the key doing
/// nothing at all.
#[test]
fn without_a_control_sender_the_binding_declines_rather_than_swallowing() {
    let mut a = app_vi();
    a.control_tx = None;
    bind_copy_key(&mut a, "copy-mode-vi", "y", "send-keys -X copy-selection");

    let action = copy_mode_binding(&a, (KeyCode::Char('y'), KeyModifiers::NONE)).unwrap();
    assert!(
        !run_copy_mode_binding(&mut a, &action),
        "with no way to queue the command the key must fall through, not vanish"
    );
}

/// The four bindings from the reported config all resolve.
#[test]
fn the_reported_config_bindings_all_resolve() {
    let mut a = app_vi();
    bind_copy_key(&mut a, "copy-mode-vi", "v", "send-keys -X begin-selection");
    bind_copy_key(&mut a, "copy-mode-vi", "C-v", "send-keys -X rectangle-toggle");
    bind_copy_key(&mut a, "copy-mode-vi", "y", "send-keys -X copy-pipe-and-cancel \"clip.exe\"");
    bind_copy_key(&mut a, "copy-mode-vi", "Escape", "send-keys -X cancel");

    for (code, mods, label) in [
        (KeyCode::Char('v'), KeyModifiers::NONE, "v"),
        (KeyCode::Char('v'), KeyModifiers::CONTROL, "C-v"),
        (KeyCode::Char('y'), KeyModifiers::NONE, "y"),
        (KeyCode::Esc, KeyModifiers::NONE, "Escape"),
    ] {
        assert!(
            copy_mode_binding(&a, (code, mods)).is_some(),
            "copy-mode-vi binding for {} did not resolve",
            label
        );
    }
}
