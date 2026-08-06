// Regression test for the client debug-log truncation fix (3240985).
//
// The control-mode reader logged each OUT line truncated to 200 bytes with
// a raw byte slice. Any line whose 200th byte fell inside a multi-byte
// character — routine for TUI box-drawing output — panicked the reader
// thread and killed the whole control client, which the attached
// integration saw as a command timeout followed by a reconnect loop. The
// fix truncates on a char boundary instead.
//
// The truncation is inline inside `run_control_mode` (no callable seam),
// so this test drives the REAL client function end-to-end: a child process
// runs the test body in child mode with a temp data dir and a scripted
// fake control server on a loopback socket; the server sends a line whose
// 200th byte is mid-glyph. The parent then asserts the reader thread
// survived (no panic on stderr, OUT line logged, full line still echoed)
// and that the truncated log prefix ends exactly on the char boundary.
// The child's stdin is held open by the parent until the exchange
// completes, then closed so the client's input loop exits cleanly.

use super::*;

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_MODE_ENV: &str = "PSMUX_TRUNC_TEST_CHILD";

/// A line of 199 ASCII bytes followed by a 3-byte CJK glyph: truncating at
/// raw byte 200 lands inside the glyph.
fn payload_line() -> String {
    format!("{}{}", "a".repeat(199), "界")
}

fn temp_data_dir(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("psmux_rs_trunc_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

/// Removes every file the parent and child created, even when an assertion
/// fails mid-test.
struct Cleanup {
    files: Vec<std::path::PathBuf>,
    dir: std::path::PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Child side: run the real control-mode client against the parent's fake
/// server, then return.
fn child_run_client() {
    let result = run_control_mode(1);
    assert!(
        result.is_ok(),
        "run_control_mode must exit cleanly after the fake server closes: {result:?}"
    );
}

#[test]
fn multi_byte_out_line_does_not_panic_the_reader_thread() {
    if std::env::var(CHILD_MODE_ENV).is_ok() {
        child_run_client();
        return;
    }

    let session = format!("trunc_{}", std::process::id());
    let dir = temp_data_dir("main");
    let key = "unit-test-key-1234";

    // The child resolves its paths through its own PSMUX_DATA_DIR env; the
    // parent spells out the same derived paths directly (the spelling is the
    // contract locked by the PSMUX_DATA_DIR override tests) instead of
    // mutating the process-global env, which would race with other tests in
    // a full parallel run.
    let port_path = format!("{dir}\\{session}.port");
    let key_path = format!("{dir}\\{session}.key");
    let log_path = format!("{dir}\\cc_debug.log");
    let _cleanup = Cleanup {
        files: vec![
            std::path::PathBuf::from(&port_path),
            std::path::PathBuf::from(&key_path),
            std::path::PathBuf::from(&log_path),
        ],
        dir: std::path::PathBuf::from(&dir),
    };

    // Fake control server: auth, CONTROL handshake, then the adversarial
    // payload line followed by a marker line, then close.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let port = listener.local_addr().unwrap().port();
    std::fs::write(&port_path, port.to_string()).unwrap();
    std::fs::write(&key_path, key).unwrap();

    let payload = format!("{}\n", payload_line());
    let out_marker = format!("OUT ({} bytes):", payload.len());
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("fake server accept");
        let mut reader = std::io::BufReader::new(sock.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).expect("read AUTH");
        assert!(line.starts_with("AUTH "), "unexpected auth line: {line:?}");
        sock.write_all(b"OK\n").unwrap();
        sock.flush().unwrap();
        line.clear();
        reader.read_line(&mut line).expect("read mode");
        assert_eq!(line.trim(), "CONTROL", "client must request CONTROL mode");
        sock.write_all(payload.as_bytes()).unwrap();
        sock.write_all(b"tail marker\n").unwrap();
        sock.flush().unwrap();
        // Keep the connection open long enough for the reader thread to
        // log both lines, then close to end its read loop.
        std::thread::sleep(Duration::from_millis(1500));
    });

    let exe = std::env::current_exe().expect("test binary path");
    let test_name =
        "tests_client_debug_log_truncation::multi_byte_out_line_does_not_panic_the_reader_thread";
    let mut child = Command::new(exe)
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(CHILD_MODE_ENV, "1")
        .env("PSMUX_DATA_DIR", &dir)
        .env("PSMUX_SESSION_NAME", &session)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn test child");

    // Hold the child's stdin open so its input loop keeps running while
    // the reader thread processes the payload.
    let mut child_stdin = child.stdin.take().unwrap();

    // Poll the client's debug log for the truncated OUT line.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut log_text = String::new();
    let mut seen = false;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&log_path) {
            if text.contains(&out_marker) {
                log_text = text;
                seen = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(child_stdin); // EOF: let the child's stdin loop finish.

    let output = child.wait_with_output().expect("wait for test child");
    let _ = server.join();
    if !seen {
        let final_log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "the reader thread must log the OUT line\nlog {log_path:?}:\n{final_log}\nchild stderr:\n{stderr}"
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "truncating mid-glyph must not panic the reader thread; stderr: {stderr}"
    );
    assert!(
        output.status.success(),
        "child must exit successfully: {:?}",
        output.status
    );
    assert!(
        log_text.contains("tail marker"),
        "the reader thread must survive past the adversarial line and log the marker line"
    );
    // The truncation must stop exactly at the char boundary: 199 ASCII
    // bytes then the closing quote of the {:?} debug repr — no partial
    // glyph bytes, no over-truncation.
    assert!(
        log_text.contains(&format!("\"{}\"", "a".repeat(199))),
        "truncation must end on the char boundary (199 bytes); log:\n{log_text}"
    );
}
