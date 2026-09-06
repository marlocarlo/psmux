// Root-cause regression tests for issue #250 (session picker AUTH ack race).
//
// Issue #250 was patched in PR #251 by adding a one-shot `fetch_session_info`
// helper that explicitly skips the AUTH `OK\n` ack. That fixed the symptom
// at one call site, but the underlying smell — every TCP picker fetch
// reimplementing AUTH+command framing by hand — remained. This file tests
// the deeper fix: a centralized `fetch_authed_response` /
// `fetch_authed_response_multi` helper that:
//
//   1. Validates the session key against CRLF/NUL injection (security).
//   2. Caps response payloads at MAX_AUTHED_RESPONSE_BYTES (DoS guard).
//   3. Handles every AUTH-ack timing race uniformly (the same correctness
//      property #251 added for `session-info`, but for ALL command sites).
//   4. Fans out picker fetches in parallel across N sessions with a wall
//      time bounded by a single read_timeout (performance).
//
// Tests use real TCP listeners on 127.0.0.1:0 and call the production
// helpers directly — no parser re-implementation.

use super::*;

use std::io::{Read, Write as IoWrite};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// ---- Shared helpers ----------------------------------------------------------

/// Drain the client's `AUTH key\n` + first command line so the fake server's
/// writes land against the expected state. Reads exactly two newlines.
fn drain_two_lines(stream: &mut TcpStream) {
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

/// Spawn a one-shot listener; hand the accepted stream to `respond`.
/// Returns the bind address and a channel that signals when the responder
/// thread finished (so tests can avoid leaking servers).
fn spawn_fake<F>(respond: F) -> (String, mpsc::Receiver<()>)
where
    F: FnOnce(TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            respond(stream);
        }
        let _ = tx.send(());
    });
    (addr, rx)
}

// ---- validate_auth_key (SECURITY: CRLF/NUL injection guard) -----------------

#[test]
fn validate_auth_key_accepts_normal_key() {
    assert_eq!(validate_auth_key("abc123XYZ_-"), Some("abc123XYZ_-"));
}

#[test]
fn validate_auth_key_rejects_empty() {
    assert_eq!(validate_auth_key(""), None);
    assert_eq!(validate_auth_key("\n"), None);
    assert_eq!(validate_auth_key("\r\n"), None);
}

#[test]
fn validate_auth_key_rejects_embedded_lf() {
    // SECURITY: an LF in the key would terminate the AUTH line early and let
    // anything after smuggle in as a second protocol frame.
    assert_eq!(validate_auth_key("realkey\nkill-server"), None);
}

#[test]
fn validate_auth_key_rejects_embedded_cr() {
    assert_eq!(validate_auth_key("realkey\rkill-server"), None);
}

#[test]
fn validate_auth_key_rejects_embedded_nul() {
    assert_eq!(validate_auth_key("real\0key"), None);
}

#[test]
fn validate_auth_key_strips_only_outer_crlf() {
    // A trailing newline (e.g. from read_to_string of a key file) is fine.
    // It is stripped, the remainder is returned.
    assert_eq!(validate_auth_key("realkey\n"), Some("realkey"));
    assert_eq!(validate_auth_key("\nrealkey\r\n"), Some("realkey"));
}

#[test]
fn fetch_authed_response_refuses_injected_key() {
    // SECURITY: even if a caller passed a malicious key, the helper must
    // not put it on the wire and must not connect at all. This is verified
    // by an unbound port — if the helper tried to dial a real server it
    // would either time out or refuse, but with a CRLF-tainted key it
    // should bail before opening a socket. We assert it returns None
    // immediately (well under the connect_timeout).
    let start = Instant::now();
    let info = fetch_authed_response(
        "127.0.0.1:1", // unbound, would otherwise fail with refused
        "good\nkill-server",
        b"session-info\n",
        Duration::from_millis(500),
        Duration::from_millis(500),
    );
    let elapsed = start.elapsed();
    assert_eq!(info, None);
    assert!(
        elapsed < Duration::from_millis(50),
        "key validation must short-circuit before any TCP work, took {:?}",
        elapsed
    );
}

// ---- fetch_authed_response (single-line, all races covered) -----------------

#[test]
fn fetch_authed_response_pipelined_ack_and_payload() {
    // Server replies with both the ack and the payload in a single write.
    // This is the common "happy path" on loopback.
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\nmy-session: 3 windows\n");
        let _ = s.flush();
    });
    let info = fetch_authed_response(
        &addr,
        "key",
        b"session-info\n",
        Duration::from_millis(200),
        Duration::from_millis(500),
    );
    assert_eq!(info.as_deref(), Some("my-session: 3 windows"));
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn fetch_authed_response_late_ack_does_not_leak_as_payload() {
    // The exact #250 race: the AUTH ack is delayed past the client's first
    // read. The centralized helper must NEVER report "OK" as the payload.
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        thread::sleep(Duration::from_millis(120));
        let _ = s.write_all(b"OK\n");
        let _ = s.flush();
        thread::sleep(Duration::from_millis(20));
        let _ = s.write_all(b"real-payload-line\n");
        let _ = s.flush();
    });
    let info = fetch_authed_response(
        &addr,
        "key",
        b"session-info\n",
        Duration::from_millis(200),
        Duration::from_millis(500), // generous so we catch the real line
    );
    assert_ne!(info.as_deref(), Some("OK"), "late ack leaked as payload");
    // With a generous read_timeout, the payload SHOULD make it through.
    assert_eq!(info.as_deref(), Some("real-payload-line"));
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn fetch_authed_response_only_ok_returns_none() {
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\n");
        let _ = s.flush();
        thread::sleep(Duration::from_millis(200));
    });
    let info = fetch_authed_response(
        &addr,
        "key",
        b"session-info\n",
        Duration::from_millis(200),
        Duration::from_millis(80),
    );
    assert_eq!(info, None);
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn fetch_authed_response_error_reply_returns_none() {
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"ERROR: Invalid session key\n");
        let _ = s.flush();
    });
    let info = fetch_authed_response(
        &addr,
        "wrong",
        b"session-info\n",
        Duration::from_millis(200),
        Duration::from_millis(200),
    );
    assert_eq!(info, None);
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn fetch_authed_response_appends_missing_newline() {
    // Helper accepts cmds without trailing newline and adds it. Confirms the
    // server still sees a valid command frame.
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\npayload\n");
        let _ = s.flush();
    });
    let info = fetch_authed_response(
        &addr,
        "key",
        b"session-info", // no trailing \n on purpose
        Duration::from_millis(200),
        Duration::from_millis(300),
    );
    assert_eq!(info.as_deref(), Some("payload"));
    let _ = done.recv_timeout(Duration::from_secs(2));
}

// ---- fetch_authed_response_multi (multi-line responses) ---------------------

#[test]
fn fetch_authed_response_multi_strips_leading_ok() {
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\n[{\"id\":1,\"name\":\"w0\"}]\n");
        let _ = s.flush();
    });
    let info = fetch_authed_response_multi(
        &addr,
        "key",
        b"list-tree\n",
        Duration::from_millis(200),
        Duration::from_millis(300),
    );
    assert_eq!(info.as_deref(), Some("[{\"id\":1,\"name\":\"w0\"}]"));
    let _ = done.recv_timeout(Duration::from_secs(2));
}

#[test]
fn fetch_authed_response_multi_handles_multiline_body() {
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\nbuffer0: 5 bytes: \"hello\"\nbuffer1: 3 bytes: \"hi!\"\n");
        let _ = s.flush();
    });
    let info = fetch_authed_response_multi(
        &addr,
        "key",
        b"choose-buffer\n",
        Duration::from_millis(200),
        Duration::from_millis(300),
    );
    let body = info.expect("payload");
    assert!(body.contains("buffer0:"));
    assert!(body.contains("buffer1:"));
    assert!(!body.starts_with("OK"));
    let _ = done.recv_timeout(Duration::from_secs(2));
}

// ---- DoS guard: response size cap -------------------------------------------

#[test]
fn fetch_authed_response_caps_runaway_response() {
    // SECURITY: a server that sends an unbounded line with no newline could
    // otherwise force the client to buffer until timeout. The cap must
    // bound BOTH wall time AND memory.
    let (addr, _done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\n");
        // Pump lots of bytes with no newline. We send well over the cap so
        // the client should hit the limit and stop.
        let chunk = vec![b'X'; 64 * 1024];
        for _ in 0..16 {
            // 1 MB total, no newline
            if s.write_all(&chunk).is_err() {
                return;
            }
        }
        thread::sleep(Duration::from_millis(50));
    });
    let start = Instant::now();
    let info = fetch_authed_response(
        &addr,
        "key",
        b"session-info\n",
        Duration::from_millis(500),
        Duration::from_millis(500),
    );
    let elapsed = start.elapsed();
    assert!(info.is_none(), "a truncated line at the byte cap must never count as a valid response");
    assert!(
        elapsed < Duration::from_millis(1500),
        "should finish within ~1.5x read_timeout, took {:?}",
        elapsed
    );
}

// ---- Parallel fetch (PERFORMANCE) -------------------------------------------

#[test]
fn parallel_fetch_runs_n_servers_within_one_read_timeout() {
    // PERFORMANCE: the picker used to call fetch_session_info sequentially,
    // so opening with N sessions took O(N * read_timeout) in the worst case.
    // The new parallel helper must complete in ~one read_timeout regardless
    // of N. We spin up 8 fake servers that each delay 120 ms before replying,
    // and assert the wall time is well under N * delay.
    const N: usize = 8;
    const DELAY_MS: u64 = 120;
    const READ_TIMEOUT_MS: u64 = 400;

    let mut inputs: Vec<(String, String, String)> = Vec::with_capacity(N);
    let mut dones: Vec<mpsc::Receiver<()>> = Vec::with_capacity(N);
    for i in 0..N {
        let (addr, done) = spawn_fake(move |mut s| {
            drain_two_lines(&mut s);
            thread::sleep(Duration::from_millis(DELAY_MS));
            let _ = s.write_all(format!("OK\nsess{}: 1 windows\n", i).as_bytes());
            let _ = s.flush();
        });
        inputs.push((format!("sess{}", i), addr, "key".to_string()));
        dones.push(done);
    }

    let start = Instant::now();
    let results = fetch_session_infos_parallel(
        inputs,
        Duration::from_millis(200),
        Duration::from_millis(READ_TIMEOUT_MS),
        |label| format!("{}: (not responding)", label),
    );
    let elapsed = start.elapsed();

    assert_eq!(results.len(), N);
    for (i, (label, info)) in results.iter().enumerate() {
        assert_eq!(label, &format!("sess{}", i));
        assert_eq!(info, &format!("sess{}: 1 windows", i));
    }
    // Sequential would be N * DELAY_MS = 960 ms. Parallel should be roughly
    // DELAY_MS plus thread spawn overhead. Allow plenty of slack but still
    // strictly less than half the sequential bound.
    let sequential_bound_ms = (N as u64) * DELAY_MS;
    assert!(
        elapsed.as_millis() < (sequential_bound_ms / 2) as u128,
        "parallel fetch took {:?}, expected < {}ms (sequential would be {}ms)",
        elapsed,
        sequential_bound_ms / 2,
        sequential_bound_ms
    );

    for d in dones {
        let _ = d.recv_timeout(Duration::from_secs(2));
    }
}

#[test]
fn parallel_fetch_handles_mixed_success_and_failure() {
    // Two responsive sessions, one connect-refused (port immediately closed),
    // one that returns only OK (no payload). All four must be present in the
    // output, in input order, with the unhappy two replaced by the fallback.
    let (good1_addr, d1) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\nalpha: 1 windows\n");
        let _ = s.flush();
    });
    let (good2_addr, d2) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\nbeta: 2 windows\n");
        let _ = s.flush();
    });
    let dead_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead_listener.local_addr().unwrap().to_string();
    drop(dead_listener);
    let (only_ok_addr, d4) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\n");
        let _ = s.flush();
        thread::sleep(Duration::from_millis(200));
    });

    let inputs = vec![
        ("alpha".to_string(), good1_addr, "k".to_string()),
        ("dead".to_string(), dead_addr, "k".to_string()),
        ("beta".to_string(), good2_addr, "k".to_string()),
        ("hush".to_string(), only_ok_addr, "k".to_string()),
    ];

    let results = fetch_session_infos_parallel(
        inputs,
        Duration::from_millis(100),
        Duration::from_millis(150),
        |label| format!("{}: (not responding)", label),
    );

    assert_eq!(results.len(), 4);
    assert_eq!(results[0], ("alpha".into(), "alpha: 1 windows".into()));
    assert_eq!(results[1], ("dead".into(), "dead: (not responding)".into()));
    assert_eq!(results[2], ("beta".into(), "beta: 2 windows".into()));
    assert_eq!(results[3], ("hush".into(), "hush: (not responding)".into()));

    let _ = d1.recv_timeout(Duration::from_secs(2));
    let _ = d2.recv_timeout(Duration::from_secs(2));
    let _ = d4.recv_timeout(Duration::from_secs(2));
}

#[test]
fn parallel_fetch_empty_input_returns_empty() {
    let out = fetch_session_infos_parallel(
        Vec::new(),
        Duration::from_millis(50),
        Duration::from_millis(50),
        |_| "x".into(),
    );
    assert!(out.is_empty());
}

#[test]
fn parallel_fetch_single_input_skips_thread_spawn() {
    // Single-input path takes the fast non-scoped branch. Just verify it
    // produces correct output with the same semantics.
    let (addr, done) = spawn_fake(|mut s| {
        drain_two_lines(&mut s);
        let _ = s.write_all(b"OK\nlonely: 0 windows\n");
        let _ = s.flush();
    });
    let out = fetch_session_infos_parallel(
        vec![("lonely".into(), addr, "k".into())],
        Duration::from_millis(200),
        Duration::from_millis(300),
        |label| format!("{}: (not responding)", label),
    );
    assert_eq!(out, vec![("lonely".into(), "lonely: 0 windows".into())]);
    let _ = done.recv_timeout(Duration::from_secs(2));
}
