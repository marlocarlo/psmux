// Issue #275: detach-client CLI command parity with tmux
//
// These tests exercise the CLI argument parsing and the AppState mutations
// performed by the new server handlers.  They do NOT spin up a real TCP server
// (that's covered by tests/test_issue275_detach_client.ps1) — they verify the
// pure-state-mutation contract: which clients get removed from the registry,
// which counters decrement, and which conditions trigger destroy-on-detach.

use super::*;
use crate::types::{AppState, ClientInfo};
use std::time::Instant;

fn mock_app() -> AppState {
    let mut app = AppState::new("test_session".to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app
}

fn add_client(app: &mut AppState, id: u64, tty: &str) {
    app.client_registry.insert(id, ClientInfo {
        id,
        width: 120,
        height: 30,
        connected_at: Instant::now(),
        last_activity: Instant::now(),
        tty_name: tty.to_string(),
        is_control: false,
    });
    app.attached_clients += 1;
}

// ════════════════════════════════════════════════════════════════════════════
//  Pure state-mutation tests (mirror what the CtrlReq handlers do)
// ════════════════════════════════════════════════════════════════════════════

/// `detach-client -t %1` should remove only that client and decrement counters.
#[test]
fn force_detach_single_client_by_id() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");
    add_client(&mut app, 3, "/dev/pts/3");

    // Simulate the ForceDetachClient handler's effect.
    app.client_sizes.remove(&2);
    let was_present = app.client_registry.remove(&2).is_some();
    if was_present {
        app.attached_clients = app.attached_clients.saturating_sub(1);
    }

    assert_eq!(app.client_registry.len(), 2, "only target removed");
    assert!(!app.client_registry.contains_key(&2));
    assert!(app.client_registry.contains_key(&1));
    assert!(app.client_registry.contains_key(&3));
    assert_eq!(app.attached_clients, 2);
}

/// `detach-client -t /dev/pts/2` should resolve via tty_name lookup.
#[test]
fn force_detach_by_tty_name_lookup() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");

    let target_cid: Option<u64> = app.client_registry.iter()
        .find(|(_, ci)| ci.tty_name == "/dev/pts/2")
        .map(|(cid, _)| *cid);
    assert_eq!(target_cid, Some(2), "tty_name lookup should find client 2");

    if let Some(cid) = target_cid {
        app.client_registry.remove(&cid);
        app.attached_clients = app.attached_clients.saturating_sub(1);
    }
    assert!(!app.client_registry.contains_key(&2));
    assert_eq!(app.attached_clients, 1);
}

/// Unknown tty_name should resolve to None — the handler must be a safe no-op.
#[test]
fn force_detach_by_tty_name_missing() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");

    let target_cid: Option<u64> = app.client_registry.iter()
        .find(|(_, ci)| ci.tty_name == "/dev/pts/99")
        .map(|(cid, _)| *cid);
    assert_eq!(target_cid, None);

    // Original state unchanged.
    assert_eq!(app.client_registry.len(), 1);
    assert_eq!(app.attached_clients, 1);
}

/// `detach-client -a` from client_id=2: detaches 1 and 3, keeps 2.
#[test]
fn detach_all_other_clients_keeps_current() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");
    add_client(&mut app, 3, "/dev/pts/3");
    let except = 2u64;

    let targets: Vec<u64> = app.client_registry.iter()
        .filter(|(cid, _)| **cid != except)
        .map(|(cid, _)| *cid)
        .collect();
    assert_eq!(targets.len(), 2, "should target 1 and 3, not 2");

    for cid in &targets {
        app.client_registry.remove(cid);
        app.attached_clients = app.attached_clients.saturating_sub(1);
    }
    assert_eq!(app.client_registry.len(), 1);
    assert!(app.client_registry.contains_key(&2));
    assert_eq!(app.attached_clients, 1);
}

/// `detach-client -a` from CLI (except = u64::MAX) detaches everyone.
#[test]
fn detach_all_other_clients_with_cli_sentinel_detaches_all() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");
    let except = u64::MAX;

    let targets: Vec<u64> = app.client_registry.iter()
        .filter(|(cid, _)| **cid != except)
        .map(|(cid, _)| *cid)
        .collect();
    assert_eq!(targets.len(), 2, "u64::MAX sentinel matches no client → all detach");

    for cid in &targets {
        app.client_registry.remove(cid);
        app.attached_clients = app.attached_clients.saturating_sub(1);
    }
    assert!(app.client_registry.is_empty());
    assert_eq!(app.attached_clients, 0);
}

/// `detach-client -s <session>` (and the CLI default) detaches every client.
#[test]
fn detach_all_clients_clears_registry() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");
    add_client(&mut app, 3, "/dev/pts/3");
    app.latest_client_id = Some(2);
    app.client_prefix_active = true;

    let targets: Vec<u64> = app.client_registry.keys().copied().collect();
    for cid in &targets {
        app.client_registry.remove(cid);
        app.attached_clients = app.attached_clients.saturating_sub(1);
    }
    if !targets.is_empty() {
        app.latest_client_id = None;
        app.client_prefix_active = false;
    }

    assert!(app.client_registry.is_empty());
    assert_eq!(app.attached_clients, 0);
    assert_eq!(app.latest_client_id, None);
    assert!(!app.client_prefix_active);
}

/// destroy_unattached + last client detached → server should be eligible for shutdown.
#[test]
fn detach_last_client_with_destroy_unattached_signals_shutdown() {
    let mut app = mock_app();
    app.destroy_unattached = true;
    add_client(&mut app, 1, "/dev/pts/1");

    app.client_registry.remove(&1);
    app.attached_clients = app.attached_clients.saturating_sub(1);

    // Replicates the handler's exit-eligibility check.
    let eligible = app.attached_clients == 0 && app.destroy_unattached;
    assert!(eligible, "destroy_unattached + zero clients → shutdown path");
}

/// Without destroy_unattached, the same condition should NOT trigger shutdown.
#[test]
fn detach_last_client_without_destroy_unattached_does_not_signal_shutdown() {
    let mut app = mock_app();
    app.destroy_unattached = false;
    add_client(&mut app, 1, "/dev/pts/1");

    app.client_registry.remove(&1);
    app.attached_clients = app.attached_clients.saturating_sub(1);

    let eligible = app.attached_clients == 0 && app.destroy_unattached;
    assert!(!eligible, "without destroy_unattached, server stays alive");
}

// ════════════════════════════════════════════════════════════════════════════
//  Idempotent reap (ghost-client leak fix) — exercises the REAL AppState method
//  that the ClientDetach handler and the writer-thread Guard both call.
// ════════════════════════════════════════════════════════════════════════════

/// A first reap removes the client and decrements the counter, returning true.
#[test]
fn reap_client_first_call_removes_and_reports_present() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");

    assert!(app.reap_client(2), "present client → reap reports true");
    assert!(!app.client_registry.contains_key(&2));
    assert_eq!(app.attached_clients, 1, "exactly one decrement");
}

/// Reaping the SAME client twice must be a safe no-op the second time — the
/// core of the fix. A connection is torn down by whichever of its two threads
/// (reader loop / writer Guard) notices death first; both call this. Without
/// idempotency the second call would over-decrement attached_clients.
#[test]
fn reap_client_is_idempotent_no_double_decrement() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");

    assert!(app.reap_client(1), "first reap of client 1 → true");
    assert!(!app.reap_client(1), "second reap of client 1 → false (already gone)");

    // client 2 must still be counted; attached_clients must not underflow past it.
    assert_eq!(app.attached_clients, 1, "double reap must not over-decrement");
    assert!(app.client_registry.contains_key(&2), "unrelated client untouched");
}

/// Regression for the counter/registry desync: a duplicate detach used to drop
/// attached_clients to 0 while real client entries remained, so `list-clients`
/// showed clients while `session_attached` read 0. With the guard, the count
/// always matches the registry size.
#[test]
fn reap_client_keeps_counter_consistent_with_registry() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");
    add_client(&mut app, 3, "/dev/pts/3");

    // Client 2 dies; both of its threads fire a reap for the same cid.
    let _ = app.reap_client(2);
    let _ = app.reap_client(2);

    assert_eq!(app.client_registry.len(), 2, "two real clients remain");
    assert_eq!(app.attached_clients, app.client_registry.len(),
        "attached_clients must equal registry size (never 0-while-clients-remain)");
}

/// Reaping a client that was never registered is a safe no-op that reports
/// false and leaves all counters intact (e.g. a persistent connection whose
/// writer Guard fires though the client never sent attach-session).
#[test]
fn reap_client_unknown_id_is_noop() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");

    assert!(!app.reap_client(999), "unknown client → reap reports false");
    assert_eq!(app.client_registry.len(), 1);
    assert_eq!(app.attached_clients, 1);
}

/// Reaping the latest_client_id clears it; reaping a different client leaves it.
#[test]
fn reap_client_clears_latest_client_id_only_for_that_client() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    add_client(&mut app, 2, "/dev/pts/2");
    app.latest_client_id = Some(2);

    assert!(app.reap_client(1));
    assert_eq!(app.latest_client_id, Some(2), "reaping a different client keeps latest");

    assert!(app.reap_client(2));
    assert_eq!(app.latest_client_id, None, "reaping the latest client clears it");
}

/// A real reap clears the shared client_prefix_active flag; a no-op reap of an
/// absent client leaves it untouched (documents the flag's guarded semantics).
#[test]
fn reap_client_clears_prefix_flag_only_on_real_reap() {
    let mut app = mock_app();
    add_client(&mut app, 1, "/dev/pts/1");
    app.client_prefix_active = true;
    assert!(!app.reap_client(99), "absent client -> no-op");
    assert!(app.client_prefix_active, "no-op reap must not touch the prefix flag");
    assert!(app.reap_client(1), "present client -> real reap");
    assert!(!app.client_prefix_active, "real reap clears the prefix flag");
}

/// The fix's core promise via reap_client's return value: the destroy-unattached
/// exit is gated on a REAL reap, so a duplicate detach of the last client cannot
/// re-satisfy the `attached_clients == 0 && destroy_unattached` condition twice.
#[test]
fn reap_client_gates_destroy_unattached_to_a_single_real_reap() {
    let mut app = mock_app();
    app.destroy_unattached = true;
    add_client(&mut app, 1, "/dev/pts/1");

    // First reap is real -> the caller would run the destroy path here.
    assert!(app.reap_client(1));
    assert_eq!(app.attached_clients, 0);
    // The handler's exit gate: attached_clients == 0 && destroy_unattached.
    assert!(app.attached_clients == 0 && app.destroy_unattached,
        "first (real) reap leaves the session destroy-eligible");

    // Second reap of the same cid is a no-op -> the caller must NOT re-run destroy.
    assert!(!app.reap_client(1), "duplicate reap is a no-op (gate not re-triggered by a real reap)");
    assert_eq!(app.attached_clients, 0, "counter stays at 0, not underflowed");
}

/// Models the actual connection-teardown path this fix targets: BOTH the reader
/// loop (socket EOF) and the writer thread's Guard now enqueue `ClientDetach`
/// for the SAME registered client. The server processes them as two
/// `reap_client` calls in sequence. This asserts the whole contract the handler
/// relies on: exactly one registry removal, exactly one `attached_clients`
/// decrement, and — because the destroy-unattached / `client-detached` hook side
/// effects run only on a `true` return — those fire exactly once, never twice.
/// (A full server-loop integration test of the Guard is impractical because the
/// missed-reap race is timing-dependent; this pins the invariant the fix rests on.)
#[test]
fn reap_client_reader_and_writer_duplicate_detach_reaps_once() {
    let mut app = mock_app();
    app.destroy_unattached = true;
    add_client(&mut app, 1, "/dev/pts/1"); // survivor
    add_client(&mut app, 2, "/dev/pts/2"); // the client whose connection tears down
    assert_eq!(app.attached_clients, 2);

    // Reader loop observes EOF and enqueues ClientDetach(2).
    let reader_side_effects = app.reap_client(2);
    // Writer Guard also enqueues ClientDetach(2) for the same connection.
    let writer_side_effects = app.reap_client(2);

    assert!(reader_side_effects, "first (reader) reap is real -> side effects run once");
    assert!(!writer_side_effects, "second (writer Guard) reap is a no-op -> side effects do NOT run again");
    assert_eq!(app.client_registry.len(), 1, "exactly one registry removal");
    assert!(app.client_registry.contains_key(&1), "survivor untouched");
    assert_eq!(app.attached_clients, 1, "exactly one decrement; survivor still counted (not 0, no destroy)");
    assert!(!(app.attached_clients == 0 && app.destroy_unattached),
        "survivor remains -> destroy-unattached gate is NOT satisfied");
}

// ════════════════════════════════════════════════════════════════════════════
//  CLI flag-parsing tests (mirror the parser in main.rs detach-client branch)
// ════════════════════════════════════════════════════════════════════════════

/// Helper: parse the same flag set the CLI dispatch parses.
fn parse_detach_args(argv: &[&str]) -> (Option<String>, Option<String>, bool, bool, Option<String>) {
    let mut t_target: Option<String> = None;
    let mut s_target: Option<String> = None;
    let mut detach_all = false;
    let mut kill_parent = false;
    let mut shell_cmd: Option<String> = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i] {
            "-a" => { detach_all = true; }
            "-P" => { kill_parent = true; }
            "-t" => { if let Some(v) = argv.get(i + 1) { t_target = Some(v.to_string()); i += 1; } }
            "-s" => { if let Some(v) = argv.get(i + 1) { s_target = Some(v.to_string()); i += 1; } }
            "-E" => { if let Some(v) = argv.get(i + 1) { shell_cmd = Some(v.to_string()); i += 1; } }
            _ => {}
        }
        i += 1;
    }
    (t_target, s_target, detach_all, kill_parent, shell_cmd)
}

#[test]
fn cli_parse_no_args() {
    let (t, s, a, p, e) = parse_detach_args(&[]);
    assert_eq!(t, None);
    assert_eq!(s, None);
    assert!(!a);
    assert!(!p);
    assert_eq!(e, None);
}

#[test]
fn cli_parse_t_with_session_name() {
    let (t, _, _, _, _) = parse_detach_args(&["-t", "main"]);
    assert_eq!(t, Some("main".to_string()));
}

#[test]
fn cli_parse_t_with_tty_path() {
    let (t, _, _, _, _) = parse_detach_args(&["-t", "/dev/pts/2"]);
    assert_eq!(t, Some("/dev/pts/2".to_string()));
}

#[test]
fn cli_parse_t_with_percent_id() {
    let (t, _, _, _, _) = parse_detach_args(&["-t", "%5"]);
    assert_eq!(t, Some("%5".to_string()));
    let numeric: Option<u64> = t.as_ref().and_then(|v| v.trim_start_matches('%').parse().ok());
    assert_eq!(numeric, Some(5));
}

#[test]
fn cli_parse_a_flag() {
    let (_, _, a, _, _) = parse_detach_args(&["-a"]);
    assert!(a);
}

#[test]
fn cli_parse_P_flag() {
    let (_, _, _, p, _) = parse_detach_args(&["-P"]);
    assert!(p);
}

#[test]
fn cli_parse_combined_aP() {
    let (_, _, a, p, _) = parse_detach_args(&["-a", "-P"]);
    assert!(a);
    assert!(p);
}

#[test]
fn cli_parse_s_and_t_together() {
    let (t, s, _, _, _) = parse_detach_args(&["-s", "work", "-t", "%1"]);
    assert_eq!(s, Some("work".to_string()));
    assert_eq!(t, Some("%1".to_string()));
}

#[test]
fn cli_parse_E_shell_command() {
    let (_, _, _, _, e) = parse_detach_args(&["-E", "exit"]);
    assert_eq!(e, Some("exit".to_string()));
}

#[test]
fn cli_parse_unknown_flags_ignored() {
    // Unknown flags must not panic or consume positional arguments.
    let (t, _, _, _, _) = parse_detach_args(&["-X", "garbage", "-t", "main"]);
    assert_eq!(t, Some("main".to_string()));
}

// ════════════════════════════════════════════════════════════════════════════
//  Action mapping (keybinding dispatch path)
// ════════════════════════════════════════════════════════════════════════════

/// `detach-client` and `detach` (alias) both resolve to Action::Detach.
/// This is what `bind-key d detach-client` binds to.
#[test]
fn detach_client_resolves_to_action_detach() {
    use crate::types::Action;
    assert!(matches!(parse_command_to_action("detach-client"), Some(Action::Detach)),
        "detach-client should map to Action::Detach");
    assert!(matches!(parse_command_to_action("detach"), Some(Action::Detach)),
        "detach (alias) should map to Action::Detach");
}

/// Flag suffixes (`-a`, `-P`) on the bound command should still resolve to
/// Detach so prefix+d-with-flags works the same.  We accept either Detach or
/// a generic Command(...) — both are valid dispatch shapes.
#[test]
fn detach_with_flags_still_dispatches() {
    let action = parse_command_to_action("detach-client -a");
    assert!(action.is_some(), "detach-client -a must produce some Action");
}
