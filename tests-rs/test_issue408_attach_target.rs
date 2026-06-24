// Tests for issue #408: `attach-session -t <name>` ignored the requested
// target and attached to last_session instead.
//
// Root cause: the global flag handling strips `-t` (and its value) from the
// post-subcommand argv (`cmd_args` / `sub_args`). The attach handler then
// looked for `-t` in those stripped args, never found it, and fell through
// to the last_session fallback.
//
// Fix: resolve `-t` from the original argv. These tests replicate the
// attach-target resolution logic from main.rs (same approach as
// test_issue202_switch_client.rs) and prove the old code was buggy.

/// Mirrors the post-fix attach-target resolution in main.rs:
///   `-t` from the full argv  ->  positional name  ->  last_session  ->  "0"
fn resolve_attach_target(
    full_args: &[&str],
    sub_args: &[&str],
    l_socket: Option<&str>,
    last_session: Option<&str>,
) -> String {
    let pref = |s: &str| match l_socket {
        Some(l) => format!("{}__{}", l, s),
        None => s.to_string(),
    };
    full_args
        .iter()
        .position(|a| *a == "-t")
        .and_then(|i| full_args.get(i + 1))
        .map(|s| pref(s))
        // Positional target comes from the post-subcommand args.
        .or_else(|| sub_args.iter().find(|a| !a.starts_with('-')).map(|a| pref(a)))
        .or_else(|| last_session.map(|s| s.to_string()))
        .unwrap_or_else(|| "0".to_string())
}

/// The OLD (buggy) logic looked for `-t` in the post-subcommand args, where
/// it had already been stripped — so an explicit `-t` was lost.
fn resolve_attach_target_old(
    sub_args: &[&str],
    last_session: Option<&str>,
) -> String {
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
    // The global handler strips `-t A`, so sub_args is empty here.
    let result = resolve_attach_target(&["psmux", "attach", "-t", "A"], &[], None, Some("B"));
    assert_eq!(result, "A", "attach -t A must honour the target, not last_session");
}

#[test]
fn attach_positional_target_still_works() {
    // `psmux attach A` (no -t): A is a positional in sub_args.
    let result = resolve_attach_target(&["psmux", "attach", "A"], &["A"], None, Some("B"));
    assert_eq!(result, "A");
}

#[test]
fn attach_bare_falls_back_to_last_session() {
    // `psmux attach` with no target uses last_session (unchanged behaviour).
    let result = resolve_attach_target(&["psmux", "attach"], &[], None, Some("B"));
    assert_eq!(result, "B");
}

#[test]
fn attach_bare_no_last_session_uses_default_index() {
    let result = resolve_attach_target(&["psmux", "attach"], &[], None, None);
    assert_eq!(result, "0");
}

#[test]
fn attach_t_applies_socket_namespace_prefix() {
    // `psmux -L work attach -t A` resolves to the namespaced base name.
    let result = resolve_attach_target(
        &["psmux", "-L", "work", "attach", "-t", "A"],
        &[],
        Some("work"),
        Some("B"),
    );
    assert_eq!(result, "work__A");
}

#[test]
fn regression_old_code_attached_to_last_session() {
    // Proof of the bug: with `-t A` stripped from sub_args, the old logic
    // returned last_session (B) instead of A.
    let old = resolve_attach_target_old(&[], Some("B"));
    assert_eq!(old, "B", "old code ignored -t and used last_session");

    // The fixed logic returns A for the same invocation.
    let new = resolve_attach_target(&["psmux", "attach", "-t", "A"], &[], None, Some("B"));
    assert_eq!(new, "A");
    assert_ne!(old, new, "fix must change the resolved target");
}
