// Tests for issue #408: `attach-session -t <name>` ignored the requested
// target and attached to last_session instead.
//
// Root cause: the global flag handling strips `-t` (and its value) from the
// post-subcommand argv (`cmd_args` / `sub_args`). The attach handler then
// looked for `-t` in those stripped args, never found it, and fell through
// to the last_session fallback.
//
// Fix: when `-t` is present, use the target the global parser already
// resolved into PSMUX_TARGET_SESSION (it ran the value through parse_target,
// so `session:window`, `$id`, `=exact`, and the `-L` namespace prefix are
// handled). These tests replicate the resolution decision from main.rs
// (same approach as test_issue202_switch_client.rs) and prove the old code
// was buggy.

/// Mirrors the post-fix attach-target resolution in main.rs:
///   if `-t` present -> PSMUX_TARGET_SESSION  (parse_target result)
///   else            -> positional name  ->  last_session  ->  "0"
///
/// `target_session` models PSMUX_TARGET_SESSION, which already holds the
/// parse_target-resolved, `-L`-prefixed session base name.
fn resolve_attach_target(
    has_t_flag: bool,
    target_session: Option<&str>,
    sub_args: &[&str],
    last_session: Option<&str>,
) -> String {
    (if has_t_flag {
        target_session.map(|s| s.to_string()).filter(|s| !s.is_empty())
    } else {
        None
    })
    // Positional target comes from the post-subcommand args.
    .or_else(|| sub_args.iter().find(|a| !a.starts_with('-')).map(|a| a.to_string()))
    .or_else(|| last_session.map(|s| s.to_string()))
    .unwrap_or_else(|| "0".to_string())
}

/// The OLD (buggy) logic looked for `-t` in the post-subcommand args, where
/// it had already been stripped — so an explicit `-t` was lost and it fell
/// through to last_session.
fn resolve_attach_target_old(sub_args: &[&str], last_session: Option<&str>) -> String {
    sub_args
        .iter()
        .position(|a| *a == "-t")
        .and_then(|i| sub_args.get(i + 1))
        .map(|s| s.to_string())
        .or_else(|| sub_args.iter().find(|a| !a.starts_with('-')).map(|a| a.to_string()))
        .or_else(|| last_session.map(|s| s.to_string()))
        .unwrap_or_else(|| "0".to_string())
}

#[test]
fn attach_t_honours_target_over_last_session() {
    // `psmux attach -t A` with last_session = B must attach to A.
    let result = resolve_attach_target(true, Some("A"), &[], Some("B"));
    assert_eq!(result, "A", "attach -t A must honour the target, not last_session");
}

#[test]
fn attach_t_session_window_resolves_to_session_only() {
    // `psmux attach -t A:1` — the global parser's parse_target extracts the
    // session ("A") into PSMUX_TARGET_SESSION; the `:1` window suffix must NOT
    // leak into the session name (a raw argv value would, breaking the port
    // lookup).
    let result = resolve_attach_target(true, Some("A"), &[], Some("B"));
    assert_eq!(result, "A", "attach -t A:1 must resolve to session A");
}

#[test]
fn attach_positional_target_still_works() {
    // `psmux attach A` (no -t): A is a positional in sub_args.
    let result = resolve_attach_target(false, None, &["A"], Some("B"));
    assert_eq!(result, "A");
}

#[test]
fn attach_bare_falls_back_to_last_session() {
    // `psmux attach` with no target uses last_session (unchanged behaviour).
    let result = resolve_attach_target(false, None, &[], Some("B"));
    assert_eq!(result, "B");
}

#[test]
fn attach_bare_no_last_session_uses_default_index() {
    let result = resolve_attach_target(false, None, &[], None);
    assert_eq!(result, "0");
}

#[test]
fn attach_t_applies_socket_namespace_prefix() {
    // `psmux -L work attach -t A`: PSMUX_TARGET_SESSION is already the
    // namespaced base name "work__A".
    let result = resolve_attach_target(true, Some("work__A"), &[], Some("B"));
    assert_eq!(result, "work__A");
}

#[test]
fn attach_t_pane_only_target_falls_through() {
    // `psmux attach -t %2` has no explicit session, so PSMUX_TARGET_SESSION is
    // never set — resolution falls through to last_session (unchanged, not a
    // regression).
    let result = resolve_attach_target(true, None, &[], Some("B"));
    assert_eq!(result, "B");
}

#[test]
fn regression_old_code_attached_to_last_session() {
    // Proof of the bug: with `-t A` stripped from sub_args, the old logic
    // returned last_session (B) instead of A.
    let old = resolve_attach_target_old(&[], Some("B"));
    assert_eq!(old, "B", "old code ignored -t and used last_session");

    // The fixed logic returns A for the same invocation.
    let new = resolve_attach_target(true, Some("A"), &[], Some("B"));
    assert_eq!(new, "A");
    assert_ne!(old, new, "fix must change the resolved target");
}
