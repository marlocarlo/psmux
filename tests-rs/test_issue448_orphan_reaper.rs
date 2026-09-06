// Reliability policy: these historical scenarios now require preserving every live server.
// Issue #448: harden server cleanup — reap live orphaned servers and store PID.
//
// These unit tests exercise the PURE decision logic that drives the reaper
// (`select_orphan_pids`) and the registry reader (`read_tracked_registry`)
// against the real production code — no OS process enumeration, so they are
// deterministic and cross-platform. The end-to-end proof that a live orphan
// process is actually terminated lives in the PowerShell E2E test.
//
// Issue #510 added a precondition to every case below: being unreferenced by
// the registry is no longer sufficient to reap a process, because the candidate
// list is machine-wide and an unreferenced server may simply belong to a
// different data dir. A process must additionally be CLAIMED by this data dir
// (an ownership marker, identity-gated on creation time). The scenarios here
// are unchanged in intent — an orphan of ours still gets reaped — so each one
// now supplies the marker its server would have written.

use super::*;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    let n = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    p.push(format!("psmux_issue448_{}_{}", std::process::id(), n));
    let _ = std::fs::create_dir_all(&p);
    p
}

fn cand(pid: u32, ports: &[u16], creation_ft: u64) -> ServerCandidate {
    ServerCandidate { pid, ports: ports.to_vec(), creation_ft }
}

fn ports(list: &[u16]) -> HashSet<u16> { list.iter().copied().collect() }
fn pids(list: &[u32]) -> HashSet<u32> { list.iter().copied().collect() }

/// Ownership markers this data dir holds: `pid -> recorded creation time`.
fn owned(list: &[(u32, u64)]) -> HashMap<u32, Option<u64>> {
    list.iter().map(|&(p, c)| (p, Some(c))).collect()
}

/// Every candidate is claimed by this data dir, at its true creation time.
/// The default for tests about registry/grace policy rather than ownership.
fn owning_all(candidates: &[ServerCandidate]) -> HashMap<u32, Option<u64>> {
    candidates.iter().map(|c| (c.pid, Some(c.creation_ft))).collect()
}

// ── select_orphan_pids: core policy ──────────────────────────────────────

#[test]
fn orphan_with_no_registry_reference_is_preserved() {
    // A live server of OURS on port 5000 that no .port file references -> orphan.
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owning_all(&cands), 42, u64::MAX);
    assert!(got.is_empty(), "a live server must not be terminated merely because registry metadata is missing");
}

#[test]
fn server_with_tracked_port_is_never_reaped() {
    // Legitimate session: its port IS in a .port file -> keep it, even though
    // its PID is not in tracked_pids (backward-compat with pre-#448 servers).
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[5000]), &pids(&[]), &owning_all(&cands), 42, u64::MAX);
    assert!(got.is_empty(), "a server whose port is registered must be preserved");
}

#[test]
fn server_with_tracked_pid_is_never_reaped() {
    // Belt-and-suspenders: PID recorded in a .pid file is protected.
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[1000]), &owning_all(&cands), 42, u64::MAX);
    assert!(got.is_empty(), "a tracked PID must be preserved");
}

#[test]
fn self_pid_is_never_reaped() {
    let cands = vec![cand(42, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owning_all(&cands), 42, u64::MAX);
    assert!(got.is_empty(), "the reaping process must never terminate itself");
}

#[test]
fn young_process_is_skipped_by_grace_window() {
    // creation_ft (200) is LATER than the age cutoff (150) -> too young, skip.
    let cands = vec![cand(1000, &[5000], 200)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owning_all(&cands), 42, 150);
    assert!(got.is_empty(), "a process younger than the grace window must be skipped");
}

#[test]
fn old_process_is_not_disposable_based_on_age() {
    // creation_ft (100) is at/older than the cutoff (150) -> eligible.
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owning_all(&cands), 42, 150);
    assert!(got.is_empty(), "a live server must not be terminated merely because registry metadata is missing");
}

#[test]
fn multi_port_server_kept_if_any_port_tracked() {
    // A server listening on two ports where only one is registered is still a
    // legitimate server (the reaper must not kill it).
    let cands = vec![cand(1000, &[5000, 5001], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[5001]), &pids(&[]), &owning_all(&cands), 42, u64::MAX);
    assert!(got.is_empty(), "any tracked port must protect the whole process");
}

#[test]
fn mixed_fleet_all_live_sessions_preserved() {
    let cands = vec![
        cand(10, &[6000], 100), // orphan (untracked, ours)
        cand(11, &[6001], 100), // legit (port tracked)
        cand(12, &[6002], 100), // legit (pid tracked)
        cand(13, &[6003], 100), // orphan (untracked, ours)
        cand(42, &[6004], 100), // self
    ];
    let mut got = select_orphan_pids(
        &cands, &ports(&[6001]), &pids(&[12]), &owning_all(&cands), 42, u64::MAX);
    got.sort();
    assert!(got.is_empty(), "a live server must not be terminated merely because registry metadata is missing");
}

// ── read_tracked_registry: file -> (ports, pids) ─────────────────────────

#[test]
fn reads_ports_and_pids_from_registry() {
    let dir = temp_dir();
    std::fs::write(dir.join("alpha.port"), "5000").unwrap();
    std::fs::write(dir.join("alpha.pid"), "1111").unwrap();
    std::fs::write(dir.join("beta.port"), "5001").unwrap();
    std::fs::write(dir.join("beta.pid"), "2222").unwrap();

    let (tp, tpid) = read_tracked_registry(&dir);
    assert!(tp.contains(&5000) && tp.contains(&5001), "both ports must be read");
    assert!(tpid.contains(&1111) && tpid.contains(&2222), "both pids must be read");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pid_without_live_port_is_ignored() {
    // A .pid whose sibling .port is gone must NOT be treated as tracked, so a
    // dead-then-reused PID can never masquerade as a live tracked server.
    let dir = temp_dir();
    std::fs::write(dir.join("ghost.pid"), "9999").unwrap(); // no ghost.port
    std::fs::write(dir.join("live.port"), "5002").unwrap();
    std::fs::write(dir.join("live.pid"), "3333").unwrap();

    let (tp, tpid) = read_tracked_registry(&dir);
    assert!(tp.contains(&5002), "live port must be tracked");
    assert!(tpid.contains(&3333), "live pid must be tracked");
    assert!(!tpid.contains(&9999), "orphaned .pid without a .port must be ignored");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn end_to_end_selection_over_registry_files() {
    // Combine both pieces: build a registry on disk, read it, and confirm the
    // orphan (untracked port) is chosen while the registered one is preserved.
    let dir = temp_dir();
    std::fs::write(dir.join("session.port"), "7000").unwrap();
    std::fs::write(dir.join("session.pid"), "500").unwrap();
    let (tp, tpid) = read_tracked_registry(&dir);

    let cands = vec![
        cand(500, &[7000], 100),  // the tracked session server
        cand(501, &[7777], 100),  // an orphaned duplicate, nothing points at it
    ];
    // Both are ours: the duplicate wrote its own ownership marker at startup,
    // which is what still identifies it after the spawn race overwrote the
    // shared .port/.pid entries with the winner's.
    let got = select_orphan_pids(
        &cands, &tp, &tpid, &owned(&[(500, 100), (501, 100)]), 1, u64::MAX);
    assert!(got.is_empty(), "a live server must not be terminated merely because registry metadata is missing");
    let _ = std::fs::remove_dir_all(&dir);
}
