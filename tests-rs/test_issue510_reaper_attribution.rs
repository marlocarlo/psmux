// Reliability policy: these historical scenarios now require preserving every live server.
// Issue #510: the startup reaper must never terminate a server it cannot
// positively attribute to THIS data dir.
//
// `reap_orphaned_servers` enumerates candidates machine-wide (loopback
// listeners filtered by image name) but draws its authority from a single
// ~/.psmux resolved from USERPROFILE/HOME. Anything that registry did not
// account for was classified as an orphan and killed, so any invocation
// resolving a different home reaped every other instance's servers: a
// redirected-HOME test harness, an MSYS2/Git-Bash shell (#474's family), a
// second account or service context, or jefe running under changed identity
// material (vybestack/llxprt-jefe#547).
//
// The defect was the polarity. A server was reaped when we could not prove it
// was ours, rather than only when we could prove it was — absence of evidence
// treated as evidence of orphanhood. These tests pin the inverted rule:
// unknown means leave it alone.
//
// The ownership marker is what supplies the proof. A server writes it into its
// own data dir, so no inspection of another process is required.

use super::*;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    let n = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    p.push(format!("psmux_issue510_{}_{}", std::process::id(), n));
    let _ = std::fs::create_dir_all(&p);
    p
}

fn cand(pid: u32, ports: &[u16], creation_ft: u64) -> ServerCandidate {
    ServerCandidate { pid, ports: ports.to_vec(), creation_ft }
}

fn ports(list: &[u16]) -> HashSet<u16> { list.iter().copied().collect() }
fn pids(list: &[u32]) -> HashSet<u32> { list.iter().copied().collect() }

fn owned(list: &[(u32, u64)]) -> HashMap<u32, Option<u64>> {
    list.iter().map(|&(p, c)| (p, Some(c))).collect()
}

// ── The core rule: no claim, no kill ─────────────────────────────────────

#[test]
fn foreign_server_is_never_reaped() {
    // The incident in miniature: a live psmux server belonging to another data
    // dir, which our registry knows nothing about. The old rule read
    // "not tracked -> orphan -> kill" and destroyed it.
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(&cands, &ports(&[]), &pids(&[]), &owned(&[]), 42, u64::MAX);
    assert!(got.is_empty(), "a server this data dir cannot claim must be left alone, got {got:?}");
}

#[test]
fn foreign_servers_survive_a_populated_local_registry() {
    // The guard that would NOT have been enough. Refusing to reap only when the
    // registry is empty protects the first invocation under a fresh HOME, but
    // as soon as the harness creates its own session the registry is non-empty
    // again and every foreign server is back in scope. Attribution has to be
    // per candidate, not a global "does my view look usable" check.
    let cands = vec![
        cand(1000, &[5000], 100), // ours, tracked
        cand(2000, &[6000], 100), // another instance's server
        cand(2001, &[6001], 100), // another instance's server
    ];
    let got = select_orphan_pids(
        &cands, &ports(&[5000]), &pids(&[1000]), &owned(&[(1000, 100)]), 42, u64::MAX);
    assert!(got.is_empty(), "foreign servers must survive a non-empty local registry, got {got:?}");
}

#[test]
fn claimed_orphan_and_foreign_servers_are_preserved() {
    // Both halves of the contract at once: our own orphan still dies, and the
    // servers we cannot account for still live.
    let cands = vec![
        cand(1000, &[5000], 100), // ours, orphaned  -> reap
        cand(2000, &[6000], 100), // foreign         -> keep
        cand(2001, &[6001], 100), // foreign         -> keep
    ];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owned(&[(1000, 100)]), 42, u64::MAX);
    assert!(got.is_empty(), "a live server must not be terminated merely because registry metadata is missing");
}

// ── The claim is identity-gated ──────────────────────────────────────────

#[test]
fn claim_with_mismatched_creation_time_is_not_honoured() {
    // Our marker names PID 1000, but the live PID 1000 was created at a
    // different time: the process we claimed exited and an unrelated one
    // inherited its PID. A stale claim must not authorise a kill (#447).
    let cands = vec![cand(1000, &[5000], 999)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owned(&[(1000, 100)]), 42, u64::MAX);
    assert!(got.is_empty(), "a claim whose creation time disagrees must not reap, got {got:?}");
}

#[test]
fn claim_without_recorded_creation_time_is_not_honoured() {
    // A bare-pid marker cannot be identity-checked, so it is not a kill
    // authority — the same policy force_kill_targets applies to bare .pid files.
    let mut markers: HashMap<u32, Option<u64>> = HashMap::new();
    markers.insert(1000, None);
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(&cands, &ports(&[]), &pids(&[]), &markers, 42, u64::MAX);
    assert!(got.is_empty(), "an un-gated claim must not reap, got {got:?}");
}

// ── Existing protections still apply on top of ownership ─────────────────

#[test]
fn claimed_but_tracked_server_is_not_reaped() {
    // Owning a process is a licence to reap it only when nothing references it.
    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[5000]), &pids(&[]), &owned(&[(1000, 100)]), 42, u64::MAX);
    assert!(got.is_empty(), "a claimed server with a tracked port must be preserved");
}

#[test]
fn claimed_self_is_not_reaped() {
    let cands = vec![cand(42, &[5000], 100)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owned(&[(42, 100)]), 42, u64::MAX);
    assert!(got.is_empty(), "the reaping process must never terminate itself");
}

#[test]
fn claimed_but_young_server_is_not_reaped() {
    // Still inside the grace window: it may not have finished registering.
    let cands = vec![cand(1000, &[5000], 200)];
    let got = select_orphan_pids(
        &cands, &ports(&[]), &pids(&[]), &owned(&[(1000, 200)]), 42, 150);
    assert!(got.is_empty(), "a claimed server younger than the grace window must be skipped");
}

// ── read_owned_server_pids: markers on disk ──────────────────────────────

#[test]
fn reads_markers_written_by_this_data_dir() {
    let dir = temp_dir();
    let servers = dir.join("servers");
    std::fs::create_dir_all(&servers).unwrap();
    std::fs::write(servers.join("1000"), "1000:100").unwrap();
    std::fs::write(servers.join("1001"), "1001:200").unwrap();

    let got = read_owned_server_pids(&dir);
    assert_eq!(got.get(&1000), Some(&Some(100)), "marker 1000 must be read with its creation time");
    assert_eq!(got.get(&1001), Some(&Some(200)), "marker 1001 must be read with its creation time");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_marker_dir_claims_nothing() {
    // A data dir that has never run a server claims no process on the machine.
    // This is the fresh-HOME harness case: the reaper finds a full machine of
    // live servers and must conclude that none of them are its business.
    let dir = temp_dir();
    let got = read_owned_server_pids(&dir);
    assert!(got.is_empty(), "a data dir with no marker dir must claim nothing, got {got:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pid_is_taken_from_marker_body_not_filename() {
    // A truncated or hand-copied marker must not be able to assert a claim over
    // whatever PID its filename happens to spell.
    let dir = temp_dir();
    let servers = dir.join("servers");
    std::fs::create_dir_all(&servers).unwrap();
    std::fs::write(servers.join("7777"), "1000:100").unwrap();

    let got = read_owned_server_pids(&dir);
    assert_eq!(got.get(&1000), Some(&Some(100)), "the body's pid is authoritative");
    assert!(!got.contains_key(&7777), "the filename must not create a claim");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreadable_marker_is_skipped_not_fatal() {
    let dir = temp_dir();
    let servers = dir.join("servers");
    std::fs::create_dir_all(&servers).unwrap();
    std::fs::write(servers.join("1000"), "1000:100").unwrap();
    std::fs::write(servers.join("garbage"), "not-a-pid").unwrap();

    let got = read_owned_server_pids(&dir);
    assert_eq!(got.get(&1000), Some(&Some(100)), "valid markers must still be read");
    assert_eq!(got.len(), 1, "a malformed marker must be skipped, got {got:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── End to end over real files: the incident, reconstructed ──────────────

#[test]
fn harness_data_dir_does_not_reap_the_users_servers() {
    // A redirected-HOME test harness (New-PsmuxTestEnv) after it has created
    // its own session: its registry is populated and its marker dir claims only
    // its own server. The user's live sessions — the agent's own psmux among
    // them — are listening, untracked here, and must survive.
    let harness = temp_dir();
    std::fs::write(harness.join("i443__probe.port"), "5000").unwrap();
    std::fs::write(harness.join("i443__probe.pid"), "1000:100").unwrap();
    let servers = harness.join("servers");
    std::fs::create_dir_all(&servers).unwrap();
    std::fs::write(servers.join("1000"), "1000:100").unwrap();

    let (tp, tpid) = read_tracked_registry(&harness);
    let owned_pids = read_owned_server_pids(&harness);

    let cands = vec![
        cand(1000, &[5000], 100),  // the harness's own server
        cand(22316, &[7001], 100), // the agent's live session
        cand(5772, &[7002], 100),  // a second agent's session
        cand(22420, &[7003], 100), // the warm-pane server
    ];
    let got = select_orphan_pids(&cands, &tp, &tpid, &owned_pids, 1, u64::MAX);
    assert!(got.is_empty(), "a harness data dir must not reap the user's servers, got {got:?}");
    let _ = std::fs::remove_dir_all(&harness);
}

#[test]
fn wiped_registry_does_not_authorize_killing_our_server() {
    // run_all_tests.ps1 deletes ~/.psmux\*.port and *.key, which is one way a
    // live server of ours loses its registry entry. The marker dir is outside
    // that pattern, so the server is still identifiable as ours and #448's
    // cleanup still works — this is the case the ownership marker exists to
    // keep working after the polarity flip.
    let dir = temp_dir();
    let servers = dir.join("servers");
    std::fs::create_dir_all(&servers).unwrap();
    std::fs::write(servers.join("1000"), "1000:100").unwrap();

    let (tp, tpid) = read_tracked_registry(&dir);
    let owned_pids = read_owned_server_pids(&dir);
    assert!(tp.is_empty() && tpid.is_empty(), "registry was wiped");

    let cands = vec![cand(1000, &[5000], 100)];
    let got = select_orphan_pids(&cands, &tp, &tpid, &owned_pids, 1, u64::MAX);
    assert!(got.is_empty(), "a live server must not be terminated merely because registry metadata is missing");
    let _ = std::fs::remove_dir_all(&dir);
}
