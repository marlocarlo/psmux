// Regression tests for the stale-port startup tax and for the PID-anchor
// false-death fix.
//
// Tax root cause: every CLI invocation ran cleanup_stale_port_files(), which
// TCP-probed each .port file serially (3 attempts x 100ms connect timeout).
// On Windows hosts where a dead loopback port never sends RST (stealth
// firewall), each probe attempt burned its full connect timeout AND
// classified as Inconclusive, so the stale files were never reaped:
// ~350-400ms per stale file on EVERY psmux command, forever. Six stale files
// made `psmux new-session` take ~2.4s (and cold start ~4.9s, since the
// spawned server pays the tax again).
//
// The #448 fix consults the .pid sentinel first: a LIVE PID is a definitive,
// microsecond-cheap keep with no network round-trip, and that fast path is
// still in effect (see live_pid_anchor_skips_the_network_probe).
//
// False-death root cause: a DEAD anchor is only a process-table guess. A live
// server can read as "dead" when the client cannot open its process
// (elevated/service spawns), when its image was renamed, when its PID was
// recycled, or when the .pid file itself is stale (a hard-killed server's
// leftover anchor that the replacement server has not rewritten yet). The
// old code treated that guess as proof of death and deleted the whole
// registry of a LIVE server, producing intermittent "no server running on
// session X" failures until the server's 5s registry self-heal rewrote the
// files. The fix escalates dead-anchor verdicts to the AUTH network probe and
// reaps only when the probe confirms; a probe that authenticates the port
// repairs the stale .pid anchor in place instead.

use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_psmux_dir(test_name: &str) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join(format!("psmux_{test_name}_{}_{}", std::process::id(), n))
        .join(".psmux");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_registry(dir: &std::path::Path, session: &str, port: &str) -> (PathBuf, PathBuf, PathBuf) {
    let port_path = dir.join(format!("{session}.port"));
    let key_path = dir.join(format!("{session}.key"));
    let sid_path = dir.join(format!("{session}.sid"));
    fs::write(&port_path, port).unwrap();
    fs::write(&key_path, "test-key").unwrap();
    fs::write(&sid_path, "1").unwrap();
    (port_path, key_path, sid_path)
}

/// A dead-looking anchor: a PID body that no live psmux process can match
/// (the real pid_anchor_verdict is Windows-only, so tests inject the verdict
/// directly; the body just needs to parse like the real sentinel).
const STALE_ANCHOR: &str = "999999:134301758043996634";

/// Drive the stale-port sweep with fully injected oracles so every verdict is
/// deterministic on any platform.
fn sweep_with(
    dir: &std::path::Path,
    anchor: impl FnMut(&Path) -> Option<bool>,
    probe: impl FnMut(&str, u16) -> PortProbeResult,
    listener_pid: impl FnMut(u16) -> Option<u32>,
) {
    let mut anchor = anchor;
    let mut probe = probe;
    let mut listener_pid = listener_pid;
    cleanup_stale_port_files_in_with_full(&dir, &mut anchor, &mut probe, &mut listener_pid);
}

/// (a) Dead .pid + alive probe: the registry must NOT be reaped — the dead
/// anchor is a false negative, and the probe proves a live server owns the
/// port. The .pid anchor is repaired in place to name the live listener.
#[test]
fn dead_pid_anchor_with_alive_probe_keeps_and_repairs_registry() {
    let dir = temp_psmux_dir("anchor_dead_probe_alive");
    let (port_path, key_path, sid_path) = write_registry(&dir, "vibex__tmex", "54329");
    let pid_path = dir.join("vibex__tmex.pid");
    fs::write(&pid_path, STALE_ANCHOR).unwrap();

    let mut probes = 0;
    sweep_with(
        &dir,
        |_| Some(false),
        |_, _| {
            probes += 1;
            PortProbeResult::Alive
        },
        |port| {
            assert_eq!(port, 54329, "the probe port must be the one under scrutiny");
            Some(424242)
        },
    );

    assert_eq!(probes, 1, "a dead anchor must escalate to the network probe");
    assert!(port_path.exists(), "alive probe must preserve the .port of a live server");
    assert!(key_path.exists(), "alive probe must preserve the .key");
    assert!(sid_path.exists(), "alive probe must preserve the .sid");
    let repaired = fs::read_to_string(&pid_path).unwrap();
    let (pid, _) = parse_pid_file_contents(&repaired).expect("repaired .pid must parse");
    assert_eq!(pid, 424242, "the .pid anchor must be repaired to the live listener's PID");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// An alive probe with a resolvable listener PID is repaired even when the
/// sweep's listener oracle fails, the stale anchor must be left in place —
/// never reaped (the server's own 5s self-heal rewrites it).
#[test]
fn dead_pid_anchor_with_alive_probe_and_no_listener_pid_is_kept() {
    let dir = temp_psmux_dir("anchor_dead_probe_alive_no_listener");
    let (port_path, _, _) = write_registry(&dir, "vibex__warm", "54340");
    fs::write(dir.join("vibex__warm.pid"), STALE_ANCHOR).unwrap();

    sweep_with(&dir, |_| Some(false), |_, _| PortProbeResult::Alive, |_| None);

    assert!(port_path.exists(), "an unresolvable listener must still not cause a reap");
    assert!(dir.join("vibex__warm.pid").exists(), "stale anchor left for the server self-heal");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// (b) Dead .pid + refused probe: the probe confirms the server is really
/// gone, so the whole registry is reaped as before.
#[test]
fn dead_pid_anchor_with_stale_probe_reaps_registry() {
    let dir = temp_psmux_dir("anchor_dead_probe_stale");
    let (port_path, key_path, sid_path) = write_registry(&dir, "crashed", "54330");
    let pid_path = dir.join("crashed.pid");
    fs::write(&pid_path, STALE_ANCHOR).unwrap();

    let mut probes = 0;
    sweep_with(
        &dir,
        |_| Some(false),
        |_, _| {
            probes += 1;
            PortProbeResult::Stale
        },
        |_| None,
    );

    assert_eq!(probes, 1, "a dead anchor must escalate to the network probe");
    assert!(!port_path.exists(), "probe-confirmed stale .port must be reaped");
    assert!(!key_path.exists(), "matching .key must be reaped");
    assert!(!sid_path.exists(), "matching .sid must be reaped");
    assert!(!pid_path.exists(), "the dead .pid sentinel itself must be reaped");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// A dead anchor whose probe is ambiguous must be kept: cleanup may only reap
/// when liveness is disproven by the strongest available signal, never on a
/// guess.
#[test]
fn dead_pid_anchor_with_inconclusive_probe_is_kept() {
    let dir = temp_psmux_dir("anchor_dead_probe_inconclusive");
    let (port_path, key_path, sid_path) = write_registry(&dir, "busy", "54333");
    fs::write(dir.join("busy.pid"), STALE_ANCHOR).unwrap();

    let mut probes = 0;
    sweep_with(
        &dir,
        |_| Some(false),
        |_, _| {
            probes += 1;
            PortProbeResult::Inconclusive
        },
        |_| None,
    );

    assert_eq!(probes, 1);
    assert!(port_path.exists(), "inconclusive probe must not remove .port");
    assert!(key_path.exists(), "inconclusive probe must not remove .key");
    assert!(sid_path.exists(), "inconclusive probe must not remove .sid");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// The live-PID fast path stays probe-free: only dead-anchor verdicts are
/// escalated, so a healthy server's entry never pays a network round-trip.
#[test]
fn live_pid_anchor_skips_the_network_probe() {
    let dir = temp_psmux_dir("anchor_live");
    write_registry(&dir, "serving", "54331");
    fs::write(dir.join("serving.pid"), "4242:1").unwrap();

    let mut probes = 0;
    sweep_with(
        &dir,
        |_| Some(true),
        |_, _| {
            probes += 1;
            PortProbeResult::Stale
        },
        |_| None,
    );

    assert_eq!(probes, 0, "live-PID fast path must stay probe-free");
    assert!(dir.join("serving.port").exists(), "live entry must be kept");
    assert!(dir.join("serving.pid").exists());
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// (c) The namespace-token prune follows the same sweep: a namespace whose
/// registry survives (dead anchor, alive probe) keeps its identity token.
/// Deleting the token here would look like a server restart to a supervisor
/// watching #{server_instance}.
#[test]
fn namespace_token_survives_sweep_when_probe_shows_live_server() {
    let dir = temp_psmux_dir("token_survives");
    let t = crate::paths::namespace_instance_file(&dir, Some("vibex-dev"));
    fs::create_dir_all(t.parent().unwrap()).unwrap();
    fs::write(&t, "a1b2c3d4e5f60718").unwrap();
    write_registry(&dir, "vibex-dev__tmex", "54332");
    fs::write(dir.join("vibex-dev__tmex.pid"), STALE_ANCHOR).unwrap();

    // The full sweep as main.rs runs it: stale-port cleanup first, then the
    // namespace-token prune. A false-negative anchor must not cascade into
    // the namespace losing its identity.
    sweep_with(&dir, |_| Some(false), |_, _| PortProbeResult::Alive, |_| Some(424242));

    let pruned = prune_orphaned_instance_tokens_in_with(&dir, Duration::ZERO, usize::MAX);
    assert_eq!(pruned, 0, "the live namespace's token must not be pruned");
    assert!(t.exists(), "the namespace token must survive the sweep");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// Pre-#448 registries have no .pid file; behavior must be unchanged: the
/// probe runs, and Inconclusive keeps the files.
#[test]
fn missing_pid_anchor_falls_back_to_network_probe() {
    let dir = temp_psmux_dir("pid_anchor_missing");
    let (port_path, key_path, sid_path) = write_registry(&dir, "legacy", "54331");

    let mut probe_ran = false;
    sweep_with(
        &dir,
        |_| None,
        |_, _| {
            probe_ran = true;
            PortProbeResult::Inconclusive
        },
        |_| None,
    );

    assert!(probe_ran, "without a .pid anchor the network probe must still run");
    assert!(port_path.exists(), "inconclusive probe must keep .port");
    assert!(key_path.exists(), "inconclusive probe must keep .key");
    assert!(sid_path.exists(), "inconclusive probe must keep .sid");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// An alive probe on an anchorless registry keeps the entry (there is no
/// anchor to repair; the server writes one on its next registry tick).
#[test]
fn alive_probe_without_anchor_keeps_registry() {
    let dir = temp_psmux_dir("no_anchor_probe_alive");
    let (port_path, _, _) = write_registry(&dir, "legacy_alive", "54334");

    sweep_with(&dir, |_| None, |_, _| PortProbeResult::Alive, |_| None);

    assert!(port_path.exists(), "an authenticating port must keep the registry");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn unparseable_pid_anchor_falls_back_to_network_probe() {
    let dir = temp_psmux_dir("pid_anchor_garbage");
    let (port_path, _, _) = write_registry(&dir, "garbled", "54332");
    fs::write(dir.join("garbled.pid"), "not-a-pid").unwrap();

    let mut probe_ran = false;
    sweep_with(
        &dir,
        |_| None,
        |_, _| {
            probe_ran = true;
            PortProbeResult::Inconclusive
        },
        |_| None,
    );

    assert!(probe_ran, "garbage .pid must fall back to the network probe");
    assert!(port_path.exists(), "inconclusive fallback must keep files");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// The measured defect was 6 stale files costing ~2.4s (serial network
/// probes). With the #448 sentinel plus the probe escalation, dead anchors
/// with a fast-stale probe must still clean up in well under the cost of a
/// single 100ms probe attempt — the escalation is what makes a dead anchor
/// reapable, not an excuse to re-introduce the tax.
#[test]
fn cleanup_with_many_dead_anchor_registries_is_fast() {
    let dir = temp_psmux_dir("anchor_speed");
    for i in 0..6 {
        write_registry(&dir, &format!("dead{i}"), &format!("5430{i}"));
        fs::write(dir.join(format!("dead{i}.pid")), STALE_ANCHOR).unwrap();
    }

    let start = Instant::now();
    let mut probes = 0;
    sweep_with(
        &dir,
        |_| Some(false),
        |_, _| {
            probes += 1;
            PortProbeResult::Stale
        },
        |_| None,
    );
    let elapsed = start.elapsed();

    assert_eq!(probes, 6, "every dead anchor must be verified once");
    assert!(
        elapsed < Duration::from_millis(250),
        "cleanup of 6 dead registries took {:?}; the stale-port tax is back",
        elapsed
    );
    for i in 0..6 {
        assert!(!dir.join(format!("dead{i}.port")).exists(), "dead{i}.port must be reaped");
    }
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn filetime_conversion_is_monotonic_and_anchored() {
    // 1601->1970 offset must be present and ordering preserved.
    let now = std::time::SystemTime::now();
    let later = now + Duration::from_secs(10);
    let a = system_time_to_filetime_ticks(now).unwrap();
    let b = system_time_to_filetime_ticks(later).unwrap();
    assert!(b > a, "later SystemTime must map to larger FILETIME ticks");
    assert_eq!(b - a, 10 * 10_000_000, "10s must be exactly 10^8 ticks");
    assert!(a > 116_444_736_000_000_000, "ticks must include the 1601 epoch offset");
}
