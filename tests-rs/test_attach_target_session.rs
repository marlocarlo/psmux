// Regression: `psmux attach -t SESSION1` attached to the most-recently-used
// session instead of SESSION1.
//
// Root cause: the argv pre-filter in `main` strips `-t <value>` for every
// subcommand, but the attach handler re-derived its target by scanning that
// stripped arg list. It therefore never saw `-t`, fell through to the
// "most recent session" fallback, and attached to the wrong session. The
// positional form (`psmux attach SESSION1`) was unaffected because positional
// args are not stripped — which is the tell that only `-t` was broken.
//
// `attach_target_session` now reads `-t` from the RAW argv, so these tests pin
// that behavior (including the `-L` socket-namespace prefix).

use super::*;

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn attach_dash_t_returns_requested_session() {
    // The core bug: -t must win, regardless of what other sessions exist.
    let a = args(&["psmux", "attach", "-t", "SESSION1"]);
    assert_eq!(attach_target_session(&a, None), Some("SESSION1".to_string()));
}

#[test]
fn attach_dash_t_alias_forms() {
    for sub in ["a", "at", "attach", "attach-session"] {
        let a = args(&["psmux", sub, "-t", "work"]);
        assert_eq!(attach_target_session(&a, None), Some("work".to_string()));
    }
}

#[test]
fn attach_dash_t_equals_form() {
    // normalize_flag_equals splits `-t=NAME` into `-t NAME` before this runs.
    let a = normalize_flag_equals(args(&["psmux", "attach", "-t=SESSION1"]));
    assert_eq!(attach_target_session(&a, None), Some("SESSION1".to_string()));
}

#[test]
fn attach_without_dash_t_is_none() {
    // No -t: caller falls back to positional / default / last-session.
    let a = args(&["psmux", "attach", "SESSION1"]);
    assert_eq!(attach_target_session(&a, None), None);
}

#[test]
fn attach_dash_t_applies_socket_namespace_prefix() {
    let a = args(&["psmux", "-L", "mysock", "attach", "-t", "SESSION1"]);
    assert_eq!(
        attach_target_session(&a, Some("mysock")),
        Some("mysock__SESSION1".to_string())
    );
}

#[test]
fn attach_dash_t_target_with_window_pane() {
    // Full target specs are passed through verbatim here; the server splits them.
    let a = args(&["psmux", "attach", "-t", "dev:0.1"]);
    assert_eq!(attach_target_session(&a, None), Some("dev:0.1".to_string()));
}

#[test]
fn attach_dash_t_missing_value_is_none() {
    // Trailing `-t` with no value must not panic and must not fabricate a target.
    let a = args(&["psmux", "attach", "-t"]);
    assert_eq!(attach_target_session(&a, None), None);
}
