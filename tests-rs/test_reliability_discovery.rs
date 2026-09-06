//! Isolated reliability regressions. Fake loopback peers and unique scratch
//! directories only; no psmux server processes or default registry writes.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

fn scratch() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!("psmux-reliability-discovery-{}-{}-{}",
        std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn peer(reply: impl FnOnce(TcpStream) + Send + 'static) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut request = Vec::new();
        // Production client half-closes its write side after the whole request.
        stream.read_to_end(&mut request).unwrap();
        reply(stream);
    });
    (address, handle)
}

fn line_reply(bytes: &'static [u8]) -> io::Result<String> {
    let (address, handle) = peer(move |mut stream| { let _ = stream.write_all(bytes); });
    let result = query_authed_line(&address, "key", b"list-sessions -F test\n",
        Duration::from_millis(200), Duration::from_millis(300));
    handle.join().unwrap();
    result
}

#[test]
fn exact_auth_and_complete_payload_are_required() {
    for reply in [&b"session: 1 windows\n"[..], &b"OKAY\nvalue\n"[..], &b"OK \nvalue\n"[..]] {
        assert_eq!(line_reply(reply).unwrap_err().kind(), ErrorKind::InvalidData);
    }
    for reply in [&b"OK\n"[..], &b"OK\npartial"[..], &b"OK"[..]] {
        assert_eq!(line_reply(reply).unwrap_err().kind(), ErrorKind::UnexpectedEof);
    }
    for reply in [&b"ERROR: Invalid session key\n"[..], &b"ERROR: Authentication required\n"[..]] {
        assert_eq!(line_reply(reply).unwrap_err().kind(), ErrorKind::PermissionDenied);
    }
}

#[test]
fn empty_formatted_payload_is_distinct_from_no_reply_and_preserves_spaces() {
    assert_eq!(line_reply(b"OK\n\n").unwrap(), "");
    assert_eq!(line_reply(b"OK\r\n  padded  \r\n").unwrap(), "  padded  ");
    assert_eq!(line_reply(b"OK\nOK\n").unwrap(), "OK");
}

#[test]
fn split_auth_and_payload_are_framed_independently() {
    let (address, handle) = peer(|mut stream| {
        for fragment in [b"O".as_slice(), b"K\nrow", b"\n"] {
            stream.write_all(fragment).unwrap();
            thread::sleep(Duration::from_millis(10));
        }
    });
    assert_eq!(query_authed_line(&address, "key", b"session-info\n",
        Duration::from_millis(200), Duration::from_millis(300)).unwrap(), "row");
    handle.join().unwrap();
}

#[test]
fn authenticated_slow_session_stays_unresponsive_and_registry_survives() {
    let dir = scratch();
    let base = "slow";
    for (ext, bytes) in [("port", "1234"), ("key", "key"), ("pid", "123:456"), ("sid", "2")] {
        std::fs::write(dir.join(format!("{}.{}", base, ext)), bytes).unwrap();
    }
    let before: Vec<_> = ["port", "key", "pid", "sid"].iter().map(|ext| std::fs::read(dir.join(format!("{}.{}", base, ext))).unwrap()).collect();
    let (address, handle) = peer(|mut stream| {
        stream.write_all(b"OK\n").unwrap();
        thread::sleep(Duration::from_millis(150));
        let _ = stream.write_all(b"slow: 1 windows (created now)\n");
    });
    assert_eq!(probe_session_liveness(&address, "key", Duration::from_millis(200), Duration::from_millis(50)), SessionLiveness::Unresponsive);
    handle.join().unwrap();
    cleanup_stale_port_files_in_with(&dir, |_, _| PortProbeResult::Inconclusive);
    for (ext, original) in ["port", "key", "pid", "sid"].iter().zip(before) {
        assert_eq!(std::fs::read(dir.join(format!("{}.{}", base, ext))).unwrap(), original);
    }
    let candidates = vec![ServerCandidate { pid: 123, creation_ft: 456, ports: vec![1234] }];
    assert!(select_orphan_pids(&candidates, &Default::default(), &Default::default(),
        &[(123, Some(456))].into_iter().collect(), 1, u64::MAX).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn response_limit_rejects_truncation_instead_of_accepting_take_eof() {
    for newline in [false, true] {
        let (address, handle) = peer(move |mut stream| {
            let _ = stream.write_all(b"OK\n");
            let mut data = vec![b'x'; MAX_AUTHED_RESPONSE_BYTES as usize];
            if newline { data.push(b'\n'); }
            let _ = stream.write_all(&data);
        });
        assert_eq!(query_authed_line(&address, "key", b"session-info\n",
            Duration::from_millis(200), Duration::from_secs(1)).unwrap_err().kind(), ErrorKind::InvalidData);
        handle.join().unwrap();
    }
}

#[test]
fn multiline_reply_requires_eof_and_obeys_total_size_limit() {
    let (address, handle) = peer(|mut stream| {
        let _ = stream.write_all(b"OK\nrow\n");
        thread::sleep(Duration::from_millis(150));
    });
    let error = query_authed_all(&address, "key", b"list-windows\n", Duration::from_millis(200), Duration::from_millis(50)).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock));
    handle.join().unwrap();
    let (address, handle) = peer(|mut stream| {
        let _ = stream.write_all(b"OK\n");
        let _ = stream.write_all(&vec![b'x'; MAX_AUTHED_RESPONSE_BYTES as usize]);
    });
    assert_eq!(query_authed_all(&address, "key", b"list-windows\n", Duration::from_millis(200), Duration::from_secs(1)).unwrap_err().kind(), ErrorKind::InvalidData);
    handle.join().unwrap();
}

#[test]
fn byte_dribble_cannot_extend_total_response_deadline() {
    let (address, handle) = peer(|mut stream| {
        for _ in 0..30 {
            if stream.write_all(b"O").is_err() { break; }
            thread::sleep(Duration::from_millis(20));
        }
    });
    let started = std::time::Instant::now();
    let error = query_authed_line(&address, "key", b"session-info\n", Duration::from_millis(200), Duration::from_millis(80)).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock));
    assert!(started.elapsed() < Duration::from_millis(500));
    handle.join().unwrap();
}

#[test]
fn effective_namespace_and_manifest_discovery_are_consistent() {
    let dir = scratch();
    let a = crate::paths::storage_base(Some("work"), "alpha");
    std::fs::write(dir.join(format!("{}.port", a)), "34567").unwrap();
    let ambiguous = crate::paths::storage_base(None, "build__dev");
    std::fs::write(dir.join(format!("{}.registry.json", ambiguous)), "{}").unwrap();
    let tmux = "ignored,34567,0";
    assert_eq!(effective_namespace(None, None, Some(tmux), &dir).as_deref(), Some("work"));
    assert_eq!(effective_namespace(Some("other"), Some("inherited"), Some(tmux), &dir).as_deref(), Some("other"));
    assert_eq!(effective_namespace(None, Some("inherited"), Some(tmux), &dir).as_deref(), Some("inherited"));
    assert_eq!(list_session_names_ns_in(&dir, Some("work")), vec![a]);
    assert_eq!(list_session_names_ns_in(&dir, None), vec![ambiguous]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn namespace_delimiters_cannot_collide_or_cross_boundaries() {
    let pairs = [(Some("a"), "b__c"), (Some("a__b"), "c"), (Some("a_"), "_b"), (Some("a"), "__b"), (None, "build__dev"), (Some("build"), "dev")];
    let bases: std::collections::BTreeSet<_> = pairs.iter().map(|(ns, name)| crate::paths::storage_base(*ns, name)).collect();
    assert_eq!(bases.len(), pairs.len());
    for (namespace, name) in pairs {
        let base = crate::paths::storage_base(namespace, name);
        assert_eq!(crate::paths::registry_namespace(&base).as_deref(), namespace);
        assert_eq!(crate::paths::registry_session_name(&base), name);
        assert!(session_visible_from(&base, namespace));
    }
    assert!(!session_visible_from(&crate::paths::storage_base(Some("a__b"), "c"), Some("a")));
}

#[cfg(windows)]
#[test]
fn exact_pid_anchor_accepts_renamed_test_executable_and_rejects_reuse() {
    let dir = scratch();
    let port = dir.join("custom-name.port");
    std::fs::write(&port, "12345").unwrap();
    let created = crate::platform::process_kill::process_creation_time(std::process::id()).unwrap();
    std::fs::write(port.with_extension("pid"), format_pid_file_contents(std::process::id(), created)).unwrap();
    // Test executable's stem is psmux-<hash>, outside the old filename whitelist.
    assert_eq!(pid_anchor_verdict(&port), Some(true));
    std::fs::write(port.with_extension("pid"), format_pid_file_contents(std::process::id(), created + 1)).unwrap();
    assert_eq!(pid_anchor_verdict(&port), Some(false));
    std::fs::write(port.with_extension("pid"), std::process::id().to_string()).unwrap();
    assert_eq!(pid_anchor_verdict(&port), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(windows)]
#[test]
fn explicit_cleanup_preserves_replacement_registry_generation() {
    let dir = scratch();
    let base = format!("reliability-dead-{}", dir.file_name().unwrap().to_string_lossy());
    let created = crate::platform::process_kill::process_creation_time(std::process::id()).unwrap();
    std::fs::write(dir.join(format!("{}.pid", base)), format_pid_file_contents(std::process::id(), created + 1)).unwrap();
    std::fs::write(dir.join(format!("{}.port", base)), "12345").unwrap();
    let snapshot = snapshot_kill_registry(&dir, &base).unwrap();
    std::fs::write(dir.join(format!("{}.key", base)), "replacement-key").unwrap();
    assert!(!cleanup_killed_registry(&snapshot).unwrap());
    assert!(dir.join(format!("{}.port", base)).exists());
    assert_eq!(std::fs::read(dir.join(format!("{}.key", base))).unwrap(), b"replacement-key");
    std::fs::remove_dir_all(dir).unwrap();
}


#[test]
fn internal_command_queue_is_bounded_nonblocking_and_fifo() {
    let (queue, receiver) = std::sync::mpsc::sync_channel(MAX_INTERNAL_COMMANDS);
    let start = std::time::Instant::now();
    for i in 0..MAX_INTERNAL_COMMANDS {
        queue_control_request(&queue, 1234, &format!("display-message {}\n", i), "key", None).unwrap();
    }
    assert_eq!(queue_control_request(&queue, 1234, "overflow\n", "key", None).unwrap_err().kind(), ErrorKind::WouldBlock);
    assert!(start.elapsed() < Duration::from_millis(250));
    for i in 0..MAX_INTERNAL_COMMANDS {
        assert_eq!(receiver.try_recv().unwrap().command, format!("display-message {}\n", i));
    }
    assert_eq!(queue_control_request(&queue, 1234, &"x".repeat(MAX_INTERNAL_COMMAND_BYTES), "key", None).unwrap_err().kind(), ErrorKind::InvalidInput);
    assert!(receiver.try_recv().is_err());
}
