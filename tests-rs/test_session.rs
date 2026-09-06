// Tests for crate::session::fetch_session_info, covering the AUTH+session-info
// framing race that motivated issue #250.
//
// Each test spins up a minimal in-process TCP listener on 127.0.0.1:0 that
// acts as a fake psmux session server, then calls the real production
// function — no re-implementation of the parser in the test.

use super::*;

use std::fs;
use std::io::{Read, Write as IoWrite};
use std::path::PathBuf;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Read the `AUTH <key>\n` + `session-info\n` lines the client sends so the
/// fake server's subsequent writes land against the expected client state.
fn drain_client_request(stream: &mut TcpStream) {
    // AUTH line + session-info line — two LFs total.
    let mut seen_lf = 0u8;
    let mut buf = [0u8; 1];
    while seen_lf < 2 {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(_) => {
                if buf[0] == b'\n' {
                    seen_lf += 1;
                }
            }
            Err(_) => return,
        }
    }
}

/// Spawns a listener bound to an ephemeral port, hands the accepted stream
/// to `respond`, and returns `127.0.0.1:<port>` for the client to dial.
///
/// Returns the address plus a channel the caller can block on to ensure the
/// server thread finished before the test exits.
fn spawn_fake_server<F>(respond: F) -> (String, mpsc::Receiver<()>)
where
    F: FnOnce(TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap().to_string();
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            respond(stream);
        }
        let _ = done_tx.send(());
    });
    (addr, done_rx)
}

fn temp_psmux_dir(test_name: &str) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join(format!("psmux_{test_name}_{}_{}", std::process::id(), n))
        .join(".psmux");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_registry_files(
    dir: &std::path::Path,
    session: &str,
    port: &str,
) -> (PathBuf, PathBuf, PathBuf) {
    let port_path = dir.join(format!("{session}.port"));
    let key_path = dir.join(format!("{session}.key"));
    let sid_path = dir.join(format!("{session}.sid"));
    fs::write(&port_path, port).unwrap();
    fs::write(&key_path, "test-key").unwrap();
    fs::write(&sid_path, "7").unwrap();
    (port_path, key_path, sid_path)
}

#[test]
fn happy_path_returns_info_line() {
    let (addr, done) = spawn_fake_server(|mut s| {
        drain_client_request(&mut s);
        let _ = s.write_all(b"OK\n");
        let _ = s.write_all(b"call-controller: 2 windows (created Mon Apr 20 11:10:58 2026)\n");
        let _ = s.flush();
    });

    let info = fetch_session_info(
        &addr,
        "key",
        Duration::from_millis(200),
        Duration::from_millis(500),
    );

    assert_eq!(
        info.as_deref(),
        Some("call-controller: 2 windows (created Mon Apr 20 11:10:58 2026)")
    );
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn issue_250_late_auth_ack_is_not_reported_as_session_info() {
    // Reproduces the #250 race: AUTH `OK\n` is delayed until after the client's
    // first read_line would have timed out. In the old code the late "OK"
    // landed in the second read and was rendered as the session name. The
    // production function must either return the real info or `None` — never
    // `Some("OK")`.
    let (addr, done) = spawn_fake_server(|mut s| {
        drain_client_request(&mut s);
        // Hold the "OK" ack longer than the client's per-read timeout so the
        // first read_line is forced to return (on the old code, empty) and
        // the ack arrives during what was previously the "info" read.
        thread::sleep(Duration::from_millis(120));
        let _ = s.write_all(b"OK\n");
        let _ = s.flush();
        // Then send the real info line comfortably within the second read.
        thread::sleep(Duration::from_millis(20));
        let _ = s.write_all(b"convserv: 3 windows (created Mon Apr 20 11:11:06 2026)\n");
        let _ = s.flush();
    });

    let info = fetch_session_info(
        &addr,
        "key",
        Duration::from_millis(200),
        Duration::from_millis(80),  // shorter than the 120ms server delay
    );

    // The critical assertion: even under the race, we never mis-report "OK"
    // as the info line. Either the real line makes it (if the read timeout
    // is generous) or we get None — but never Some("OK").
    assert_ne!(info.as_deref(), Some("OK"), "late AUTH ack leaked as session info");
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn only_ok_ack_received_returns_none() {
    // Server replies with just the AUTH ack and never sends session-info
    // (the worst-case of #250: second read's timeout leaves nothing).
    let (addr, done) = spawn_fake_server(|mut s| {
        drain_client_request(&mut s);
        let _ = s.write_all(b"OK\n");
        let _ = s.flush();
        // Keep the connection open briefly so the client isn't racing EOF
        // against its own read_timeout.
        thread::sleep(Duration::from_millis(200));
    });

    let info = fetch_session_info(
        &addr,
        "key",
        Duration::from_millis(200),
        Duration::from_millis(80),
    );

    assert_eq!(info, None, "sole OK ack must not be reported as info");
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn connect_refused_returns_none() {
    // Bind then drop the listener so the port is (briefly) closed — on
    // loopback this produces a fast refusal. The socket might race to be
    // reused, but `fetch_session_info` must never panic and must return
    // None on connect failure.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    let info = fetch_session_info(
        &addr,
        "key",
        Duration::from_millis(50),
        Duration::from_millis(50),
    );

    assert_eq!(info, None);
}

#[test]
fn auth_rejected_returns_none() {
    // Server responds to AUTH with an error instead of OK — must not be
    // rendered as the session info line.
    let (addr, done) = spawn_fake_server(|mut s| {
        drain_client_request(&mut s);
        let _ = s.write_all(b"ERROR: Invalid session key\n");
        let _ = s.flush();
    });

    let info = fetch_session_info(
        &addr,
        "wrong-key",
        Duration::from_millis(200),
        Duration::from_millis(200),
    );

    // The picker should fall back to the generic "(not responding)"
    // label rather than rendering the raw ERROR line as the session info.
    assert_eq!(info, None, "auth error leaked as session info: {:?}", info);
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn stale_cleanup_preserves_invalid_registry_without_ownership() {
    let dir = temp_psmux_dir("stale_cleanup_invalid");
    let (port_path, key_path, sid_path) = write_registry_files(&dir, "bad", "not-a-port");

    cleanup_stale_port_files_in(&dir);

    assert!(port_path.exists(), "unowned cleanup must preserve registry metadata");
    assert!(key_path.exists(), "unowned cleanup must preserve registry metadata");
    assert!(sid_path.exists(), "unowned cleanup must preserve registry metadata");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn stale_cleanup_preserves_registry_even_if_network_probe_says_stale() {
    let dir = temp_psmux_dir("stale_cleanup_confirmed");
    let (port_path, key_path, sid_path) = write_registry_files(&dir, "dead", "54321");

    cleanup_stale_port_files_in_with(&dir, |_, _| PortProbeResult::Stale);

    assert!(port_path.exists(), "unowned cleanup must preserve registry metadata");
    assert!(key_path.exists(), "unowned cleanup must preserve registry metadata");
    assert!(sid_path.exists(), "unowned cleanup must preserve registry metadata");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn stale_cleanup_preserves_registry_when_probe_is_inconclusive() {
    let dir = temp_psmux_dir("stale_cleanup_inconclusive");
    let (port_path, key_path, sid_path) =
        write_registry_files(&dir, "maybe-live", "54322");

    cleanup_stale_port_files_in_with(&dir, |_, _| PortProbeResult::Inconclusive);

    assert!(port_path.exists(), "inconclusive probe must not remove .port");
    assert!(key_path.exists(), "inconclusive probe must not remove .key");
    assert!(sid_path.exists(), "inconclusive probe must not remove .sid");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn stale_cleanup_preserves_registry_for_live_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind live listener");
    let port = listener.local_addr().unwrap().port().to_string();
    let dir = temp_psmux_dir("stale_cleanup_live");
    let (port_path, key_path, sid_path) = write_registry_files(&dir, "live", &port);

    cleanup_stale_port_files_in(&dir);

    assert!(port_path.exists(), "live listener .port should be preserved");
    assert!(key_path.exists(), "live listener .key should be preserved");
    assert!(sid_path.exists(), "live listener .sid should be preserved");
    drop(listener);
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

/// Read a single `\n`-terminated line from the stream (the probe's AUTH line),
/// so the fake server's reply lands against the client's read.
fn read_one_line(stream: &mut TcpStream) {
    let mut buf = [0u8; 1];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(_) if buf[0] == b'\n' => return,
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

#[test]
fn stale_cleanup_preserves_session_when_credentials_are_rejected() {
    // After a crash/reboot the old port can be grabbed by a *different* live
    // psmux server, which rejects our key. A bare TCP connect would call this
    // "alive" and leave the dead session as a "(not responding)" zombie; the
    // identity probe must classify the key rejection as Stale and reap it.
    let (addr, done) = spawn_fake_server(|mut s| {
        read_one_line(&mut s);
        let _ = s.write_all(b"ERROR: Invalid session key\n");
        let _ = s.flush();
    });
    let port = addr.rsplit(':').next().unwrap().to_string();
    let dir = temp_psmux_dir("stale_cleanup_reused_port");
    let (port_path, key_path, sid_path) = write_registry_files(&dir, "ghost", &port);

    cleanup_stale_port_files_in(&dir);

    assert!(port_path.exists(), "unowned cleanup must preserve registry metadata");
    assert!(key_path.exists(), "unowned cleanup must preserve registry metadata");
    assert!(sid_path.exists(), "unowned cleanup must preserve registry metadata");
    let _ = done.recv_timeout(Duration::from_secs(2));
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn stale_cleanup_preserves_session_for_authenticated_server() {
    // Our own live server accepts the key — it must never be reaped.
    let (addr, done) = spawn_fake_server(|mut s| {
        read_one_line(&mut s);
        let _ = s.write_all(b"OK\n");
        let _ = s.flush();
        thread::sleep(Duration::from_millis(100));
    });
    let port = addr.rsplit(':').next().unwrap().to_string();
    let dir = temp_psmux_dir("stale_cleanup_authed");
    let (port_path, key_path, sid_path) = write_registry_files(&dir, "mine", &port);

    cleanup_stale_port_files_in(&dir);

    assert!(port_path.exists(), "authenticated .port must be preserved");
    assert!(key_path.exists(), "authenticated .key must be preserved");
    assert!(sid_path.exists(), "authenticated .sid must be preserved");
    let _ = done.recv_timeout(Duration::from_secs(2));
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn pre_boot_registry_is_reaped_regardless_of_port() {
    use std::time::SystemTime;
    let boot = SystemTime::now();
    let margin = Duration::from_secs(10);

    // Written well before boot (previous boot) -> reap.
    let old = boot - Duration::from_secs(3600);
    assert!(is_pre_boot(old, boot, margin), "pre-boot file must be reaped");

    // Written within the boot grace window -> keep (could be a server that
    // came up moments after boot).
    let recent = boot - Duration::from_secs(2);
    assert!(!is_pre_boot(recent, boot, margin), "just-after-boot file must be kept");

    // Written after boot -> keep.
    let fresh = boot + Duration::from_secs(30);
    assert!(!is_pre_boot(fresh, boot, margin), "post-boot file must be kept");
}

#[test]
fn liveness_authenticated_server_is_alive() {
    let (addr, done) = spawn_fake_server(|mut s| {
        drain_client_request(&mut s); // AUTH + session-info (two lines)
        let _ = s.write_all(b"OK\n");
        let _ = s.write_all(b"mysession: 2 windows (created Mon Apr 20 11:10:58 2026)\n");
        let _ = s.flush();
        thread::sleep(Duration::from_millis(50));
    });

    let v = probe_session_liveness(
        &addr,
        "key",
        Duration::from_millis(300),
        Duration::from_millis(400),
    );

    match v {
        SessionLiveness::Alive(info) => assert!(info.contains("mysession"), "info: {info}"),
        other => panic!("expected Alive, got {other:?}"),
    }
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn liveness_auth_rejection_is_unresponsive() {
    // The reboot/reused-port case: a different server rejects our key.
    let (addr, done) = spawn_fake_server(|mut s| {
        drain_client_request(&mut s);
        let _ = s.write_all(b"ERROR: Invalid session key\n");
        let _ = s.flush();
    });

    let v = probe_session_liveness(
        &addr,
        "stale-key",
        Duration::from_millis(300),
        Duration::from_millis(400),
    );

    assert_eq!(v, SessionLiveness::Unresponsive, "auth rejection is not evidence the process exited");
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn liveness_connection_refused_is_unresponsive() {
    // Bind then drop so the port is guaranteed free -> connect refused.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to grab a port");
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    let v = probe_session_liveness(
        &addr,
        "key",
        Duration::from_millis(300),
        Duration::from_millis(200),
    );

    assert_eq!(v, SessionLiveness::Unresponsive, "refused connection is not a process death certificate");
}

#[test]
fn liveness_connected_but_silent_is_unresponsive() {
    // A listener that accepts (via backlog) but never speaks our protocol.
    // Bounded: we wait one read timeout, then declare it Dead (honors
    // "no response within the timeout -> kill"); a real server self-heals.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind silent listener");
    let addr = listener.local_addr().unwrap().to_string();

    let start = std::time::Instant::now();
    let v = probe_session_liveness(
        &addr,
        "key",
        Duration::from_millis(300),
        Duration::from_millis(150),
    );

    assert_eq!(v, SessionLiveness::Unresponsive, "silent peer must remain registered after timeout");
    assert!(start.elapsed() < Duration::from_secs(2), "probe must stay bounded, not hang");
    drop(listener);
}

// --- .pid body parsing: one parser shared by every reader --------------------
// `.pid` is written as `pid:creation_filetime`, but a bare `pid` (the #448 anchor
// as first written, or an older server mid-upgrade) must still parse so the
// orphan reaper never loses track of a live server.

#[test]
fn parse_pid_file_contents_reads_both_forms() {
    assert_eq!(parse_pid_file_contents("1234:567890"), Some((1234, Some(567890))));
    assert_eq!(parse_pid_file_contents("1234"), Some((1234, None)));
    assert_eq!(parse_pid_file_contents("  1234:567890 \n"), Some((1234, Some(567890))));
    // Unparseable pid -> not a record at all.
    assert_eq!(parse_pid_file_contents("notanumber"), None);
    // Valid pid, unparseable creation time -> pid is kept, creation dropped.
    assert_eq!(parse_pid_file_contents("12:notatime"), Some((12, None)));
}

#[test]
fn format_pid_file_contents_round_trips() {
    let s = format_pid_file_contents(4321, 987654);
    assert_eq!(s, "4321:987654");
    assert_eq!(parse_pid_file_contents(&s), Some((4321, Some(987654))));
}

// --- force_kill_targets: the data-dir-scoped force-kill selector -------------
// kill-server's force-kill fallback must target only PIDs recorded in *this*
// data dir's registry, never a machine-wide scan by image name.

#[test]
fn force_kill_targets_reads_pid_files_in_its_dir() {
    let dir = temp_psmux_dir("fkt_basic");
    fs::write(dir.join("ns__a.pid"), "1234:567890").unwrap();

    let targets = force_kill_targets(&dir, None);

    assert_eq!(
        targets,
        vec![PidTarget { pid: 1234, creation_time: 567890 }],
        "the .pid file's pid and creation time must be parsed and returned"
    );
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn force_kill_targets_is_scoped_to_its_dir() {
    // The whole point of the fix: a kill-server in dir A must never reach a
    // server registered under dir B.
    let dir_a = temp_psmux_dir("fkt_a");
    let dir_b = temp_psmux_dir("fkt_b");
    fs::write(dir_a.join("a.pid"), "111:1").unwrap();
    fs::write(dir_b.join("b.pid"), "999:2").unwrap();

    let targets = force_kill_targets(&dir_a, None);

    assert_eq!(targets, vec![PidTarget { pid: 111, creation_time: 1 }]);
    assert!(
        !targets.iter().any(|t| t.pid == 999),
        "dir A's selection must not include dir B's pid"
    );
    let _ = fs::remove_dir_all(dir_a.parent().unwrap());
    let _ = fs::remove_dir_all(dir_b.parent().unwrap());
}

#[test]
fn force_kill_targets_skips_bare_and_malformed_pid_files() {
    let dir = temp_psmux_dir("fkt_malformed");
    fs::write(dir.join("good.pid"), "5:6").unwrap();
    fs::write(dir.join("bare.pid"), "12345").unwrap();          // no creation time -> no gate
    fs::write(dir.join("bad_pid.pid"), "notanumber:6").unwrap();
    fs::write(dir.join("bad_time.pid"), "7:notatime").unwrap();

    let targets = force_kill_targets(&dir, None);

    assert_eq!(
        targets,
        vec![PidTarget { pid: 5, creation_time: 6 }],
        "only well-formed pid:creation files are force-kill candidates"
    );
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn force_kill_targets_honors_ns_prefix() {
    // Many namespaces share one data dir (-L only changes the filename prefix).
    // A namespaced kill-server must force-kill only its own namespace's wedged
    // servers, never another namespace's, even though all .pid files sit side by
    // side in the same dir.
    let dir = temp_psmux_dir("fkt_ns");
    fs::write(dir.join("ns1__a.pid"), "11:1").unwrap();
    fs::write(dir.join("ns1__b.pid"), "12:2").unwrap();
    fs::write(dir.join("ns2__c.pid"), "21:3").unwrap();
    fs::write(dir.join("plain.pid"), "30:4").unwrap();

    let ns1 = force_kill_targets(&dir, Some("ns1__"));

    assert!(
        ns1.iter().all(|t| t.pid == 11 || t.pid == 12),
        "ns1 selection must contain only ns1's pids, got {ns1:?}"
    );
    assert!(
        !ns1.iter().any(|t| t.pid == 21 || t.pid == 30),
        "ns1 selection must not reach ns2's or the default namespace's pids"
    );
    assert_eq!(ns1.len(), 2, "ns1 has exactly two sessions");
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}

// --- confirms_identity: the exact-match gate that defeats pid reuse ----------

#[test]
fn confirms_identity_matches_exact_creation_time() {
    assert!(confirms_identity(Some(567890), 567890));
}

#[test]
fn confirms_identity_rejects_recycled_pid() {
    // A different creation time at the same pid means the pid was reused by an
    // unrelated process. It must never be killed.
    assert!(!confirms_identity(Some(567891), 567890));
}

#[test]
fn confirms_identity_rejects_unqueryable_process() {
    // Process gone, or OpenProcess/GetProcessTimes failed: fail safe, no kill.
    assert!(!confirms_identity(None, 567890));
}
