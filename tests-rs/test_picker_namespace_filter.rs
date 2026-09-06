//! The session picker (prefix+s) and the tree chooser (prefix+w) list only the
//! sessions of the client's own `-L` namespace.
//!
//! tmux parity: a `-L` socket is a separate server, so a client on one socket
//! never sees the sessions of another. psmux stores every namespace in one
//! registry directory as `<ns>__<name>`, and `ls` already hides other
//! namespaces, but both interactive choosers enumerated every `.port` file.
//!
//! Measured in the full sweep of 2026-08-30 (run 2026-08-30_21-24-30): an
//! external daemon kept `amx-6c9b6ad63d__main` alive in the default registry
//! while tests/test_issue259_picker_hjkl.ps1 created `a_issue259` ..
//! `d_issue259`. Sorted by name, `amx…` landed between `a_issue259` and
//! `b_issue259` (`m` is 0x6D, `_` is 0x5F), so every `j`/`k`/`h`/`l` step
//! moved the client to the wrong session while `g`/`G` still hit the ends.
//! The suite had been 58/0 in the four sweeps before.

use super::*;

#[test]
fn namespace_of_a_bare_name_is_none() {
    assert_eq!(session_namespace("a_issue259").as_deref(), None);
    assert_eq!(session_namespace("main").as_deref(), None);
    assert_eq!(session_namespace("0").as_deref(), None);
}

#[test]
fn namespace_of_a_prefixed_name_is_the_prefix() {
    assert_eq!(session_namespace("amx-6c9b6ad63d__main").as_deref(), Some("amx-6c9b6ad63d"));
    assert_eq!(session_namespace("dev__work").as_deref(), Some("dev"));
    // Only the first separator counts: the session part may itself hold `__`.
    assert_eq!(session_namespace("ns__a__b").as_deref(), Some("ns"));
}

#[test]
fn warm_helpers_have_no_namespace() {
    assert_eq!(session_namespace("__warm__").as_deref(), None);
    assert_eq!(session_namespace("dev____warm__").as_deref(), Some("dev"));
}

#[test]
fn default_namespace_sees_only_bare_names() {
    assert!(session_visible_from("a_issue259", None));
    assert!(session_visible_from("b_issue259", None));
    assert!(!session_visible_from("amx-6c9b6ad63d__main", None), "another socket's session must be invisible");
    assert!(!session_visible_from("dev__work", None));
}

#[test]
fn a_namespace_sees_only_its_own_sessions() {
    assert!(session_visible_from("dev__work", Some("dev")));
    assert!(session_visible_from("dev__a__b", Some("dev")));
    assert!(!session_visible_from("work", Some("dev")), "the default namespace is another server");
    assert!(!session_visible_from("devel__work", Some("dev")), "prefix match must stop at the separator");
    assert!(!session_visible_from("amx-6c9b6ad63d__main", Some("dev")));
}

fn write_port(dir: &std::path::Path, base: &str, port: u16) {
    std::fs::write(dir.join(format!("{}.port", base)), port.to_string()).unwrap();
}

fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("psmux_picker_ns_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn tree_chooser_in_default_namespace_skips_other_sockets_and_warm() {
    let dir = fresh_dir("default");
    write_port(&dir, "a_issue259", 40001);
    write_port(&dir, "amx-6c9b6ad63d__main", 40002);
    write_port(&dir, "b_issue259", 40003);
    write_port(&dir, "__warm__", 40004);
    write_port(&dir, "c_issue259", 40005);
    write_port(&dir, "d_issue259", 40006);

    let names: Vec<String> = tree_chooser_sessions_in(&dir, "a_issue259")
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    assert_eq!(
        names,
        vec!["a_issue259", "b_issue259", "c_issue259", "d_issue259"],
        "the picker order is the sweep's a, b, c, d with nothing wedged between a and b"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tree_chooser_in_a_namespace_lists_that_namespace_only_and_keeps_full_names() {
    let dir = fresh_dir("ns");
    write_port(&dir, "main", 40001);
    write_port(&dir, "amx-6c9b6ad63d__main", 40002);
    write_port(&dir, "amx-6c9b6ad63d__work", 40003);
    write_port(&dir, "amx-6c9b6ad63d____warm__", 40004);
    write_port(&dir, "other__main", 40005);

    let names: Vec<String> = tree_chooser_sessions_in(&dir, "amx-6c9b6ad63d__main")
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    assert_eq!(names, vec!["amx-6c9b6ad63d__main", "amx-6c9b6ad63d__work"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tree_chooser_ignores_unreadable_port_files() {
    let dir = fresh_dir("bad");
    write_port(&dir, "good", 40001);
    std::fs::write(dir.join("bad.port"), "not a port").unwrap();
    let names: Vec<String> = tree_chooser_sessions_in(&dir, "good")
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    assert_eq!(names, vec!["good"]);
    let _ = std::fs::remove_dir_all(&dir);
}
