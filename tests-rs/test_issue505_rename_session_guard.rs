// Issue #505: "session rename results in failing new session creation".
//
// A server holds the single-server-per-name mutex (issue #2) for its whole life,
// keyed on the session's port-file base. That base changes on rename, but the
// guard never followed it, so the OLD name stayed locked forever: any later
// server spawned under it exited as a "duplicate" without writing a .port file
// and the caller reported `failed to create session '<old name>'`. Since the
// default session name is "0" (exactly what a bare `new-session` auto-picks),
// renaming the first session broke plain `new-session` outright. The flip side
// is just as bad: the NEW name was left unguarded, so a renamed session lost its
// duplicate-server protection entirely.
//
// These tests drive rekey_session_guard directly. Every ownership probe runs on
// a SPAWNED THREAD on purpose: a Windows mutex is recursive for its owning
// thread, so probing from the thread that holds it would always report "free"
// and could never catch the stale hold.

use super::*;

/// Try to take `name` from a fresh thread. `true` means no live owner.
/// Acquired guards are dropped inside that thread, leaving the name free again.
fn name_is_free(name: &str) -> bool {
    let owned = name.to_string();
    std::thread::spawn(move || crate::platform::acquire_session_mutex(&owned).is_some())
        .join()
        .expect("probe thread panicked")
}

/// Unique per-test key so a parallel test run never collides on a shared name.
fn key(suffix: &str) -> String {
    format!("psmux-t505-{}-{}", std::process::id(), suffix)
}

#[test]
#[cfg(windows)]
fn rekey_frees_the_old_name() {
    let old = key("old-a");
    let new = key("new-a");

    let mut guard = crate::platform::acquire_session_mutex(&old);
    assert!(guard.is_some(), "setup: should have acquired '{}'", old);
    assert!(!name_is_free(&old), "setup: '{}' should read as held", old);

    rekey_session_guard(&mut guard, &new);

    // This is the #505 regression guard: before the fix the old name stayed
    // locked for the process's whole life, so `new-session -s <old>` silently
    // died as a duplicate and never produced a .port file.
    assert!(
        name_is_free(&old),
        "BUG #505: old name '{}' is still locked after the rename",
        old
    );
    drop(guard);
}

#[test]
#[cfg(windows)]
fn rekey_guards_the_new_name() {
    let old = key("old-b");
    let new = key("new-b");

    let mut guard = crate::platform::acquire_session_mutex(&old);
    assert!(guard.is_some(), "setup: should have acquired '{}'", old);
    assert!(name_is_free(&new), "setup: '{}' should start free", new);

    rekey_session_guard(&mut guard, &new);

    assert!(guard.is_some(), "rekey should hold a guard on '{}'", new);
    // Without this the issue #2 duplicate-server protection quietly stops
    // applying to every renamed session.
    assert!(
        !name_is_free(&new),
        "new name '{}' must be guarded after the rename",
        new
    );
    drop(guard);
    assert!(name_is_free(&new), "dropping the guard must free '{}'", new);
}

#[test]
#[cfg(windows)]
fn chained_renames_free_every_intermediate_name() {
    let names: Vec<String> = (0..4).map(|i| key(&format!("chain-{}", i))).collect();

    let mut guard = crate::platform::acquire_session_mutex(&names[0]);
    assert!(guard.is_some(), "setup: should have acquired '{}'", names[0]);
    for n in &names[1..] {
        rekey_session_guard(&mut guard, n);
    }

    for stale in &names[..names.len() - 1] {
        assert!(
            name_is_free(stale),
            "intermediate name '{}' is still locked",
            stale
        );
    }
    assert!(
        !name_is_free(names.last().unwrap()),
        "final name '{}' must be guarded",
        names.last().unwrap()
    );
    drop(guard);
}

#[test]
#[cfg(windows)]
fn rename_back_to_the_original_name_reacquires_it() {
    let original = key("round-orig");
    let temp = key("round-temp");

    let mut guard = crate::platform::acquire_session_mutex(&original);
    assert!(guard.is_some(), "setup: should have acquired '{}'", original);

    rekey_session_guard(&mut guard, &temp);
    rekey_session_guard(&mut guard, &original);

    // Release-then-acquire ordering matters here: acquiring first would have the
    // process contending with a name it already owns.
    assert!(guard.is_some(), "guard should be back on '{}'", original);
    assert!(!name_is_free(&original), "'{}' must be guarded again", original);
    assert!(name_is_free(&temp), "intermediate '{}' must be free", temp);
    drop(guard);
}

#[test]
#[cfg(windows)]
fn rekey_onto_the_same_name_keeps_it_guarded() {
    let name = key("same");

    let mut guard = crate::platform::acquire_session_mutex(&name);
    assert!(guard.is_some(), "setup: should have acquired '{}'", name);

    rekey_session_guard(&mut guard, &name);

    assert!(guard.is_some(), "re-keying onto the same name must not lose it");
    assert!(!name_is_free(&name), "'{}' must still be guarded", name);
    drop(guard);
}

#[test]
#[cfg(windows)]
fn warm_name_is_guarded_like_any_other_name() {
    let old = key("warm-old");
    // Namespaced warm base, so this test never contends with the real
    // `__warm__` server that may be running on this machine.
    let warm = format!("{}____warm__", key("warm-ns"));

    let mut guard = crate::platform::acquire_session_mutex(&old);
    assert!(guard.is_some(), "setup: should have acquired '{}'", old);

    rekey_session_guard(&mut guard, &warm);

    // Changed by issue #459. The warm name used to be exempt here, on the theory
    // that "the pool runs several". There is no pool: `__warm__.port` is a single
    // file, so a namespace can only ever publish one warm server. Leaving the name
    // unguarded meant every warm that failed or was slow to register left another
    // live process behind, which is the unbounded-growth mechanism in #459.
    assert!(guard.is_some(), "warm name '{}' must be guarded", warm);
    assert!(!name_is_free(&warm), "'{}' must read as held", warm);
    assert!(name_is_free(&old), "old name '{}' must still be released", old);
    drop(guard);
}

#[test]
#[cfg(windows)]
fn claiming_a_warm_server_guards_the_claimed_name() {
    let claimed = key("claimed");

    // A warm server reaches the claim holding the `__warm__` name (issue #459);
    // the claim is what turns it into a real named session.
    let warm = format!("{}____warm__", key("claim-ns"));
    let mut guard = crate::platform::acquire_session_mutex(&warm);
    assert!(guard.is_some(), "setup: warm should hold '{}'", warm);
    rekey_session_guard(&mut guard, &claimed);

    // Releasing the warm name is what lets the replacement warm start.
    assert!(name_is_free(&warm), "claim must release '{}'", warm);

    assert!(guard.is_some(), "claim should acquire '{}'", claimed);
    assert!(!name_is_free(&claimed), "claimed name '{}' must be guarded", claimed);
    drop(guard);
}

#[test]
#[cfg(windows)]
fn rekey_collision_preserves_original_guard() {
    let old = key("busy-old");
    let busy = key("busy-target");

    // Hold `busy` from another thread, the way a different live server would.
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let busy_owned = busy.clone();
    let holder = std::thread::spawn(move || {
        let held = crate::platform::acquire_session_mutex(&busy_owned);
        assert!(held.is_some(), "holder thread should own the busy name");
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        drop(held);
    });
    started_rx.recv().unwrap();

    let mut guard = crate::platform::acquire_session_mutex(&old);
    assert!(guard.is_some(), "setup: should have acquired '{}'", old);

    assert!(rekey_session_guard(&mut guard, &busy).is_err());
    assert!(guard.is_some(), "collision must preserve the original guard");
    assert!(!name_is_free(&old), "old name must remain guarded after collision");
    assert!(!name_is_free(&busy), "destination must remain guarded by its owner");

    release_tx.send(()).unwrap();
    holder.join().unwrap();
}
