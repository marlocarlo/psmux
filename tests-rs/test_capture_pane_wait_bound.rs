// Regression tests for the bounded one-shot capture-pane wait (47888d3).
//
// The CLI capture-pane path blocked forever on the server response channel
// when the server loop was wedged, hanging the client process. The fix
// bounds the wait (5 seconds, mirroring the control-mode capture path) and
// reports "ERROR: capture-pane timed out" instead of blocking indefinitely.
//
// Two layers are locked here:
//
//   - `recv_control_response`, the named bounded-wait helper behind the
//     control-mode capture path, directly: timeout and disconnect both
//     produce errors within the bound instead of hanging.
//   - `handle_connection`, the one-shot CLI connection handler, over real
//     loopback sockets with a never-answered server request: the response
//     arrives after the 5s bound with the exact error text.

use super::*;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TEST_KEY: &str = "unit-test-key";

/// Serve one connection through the real one-shot handler. `tx` is the
/// channel to the (absent) server loop; keep the receiving side alive but
/// never service it to simulate a wedged server.
fn spawn_handler(listener: TcpListener, tx: mpsc::Sender<CtrlReq>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("handler: accept");
        handle_connection(
            stream,
            tx,
            TEST_KEY,
            Arc::new(RwLock::new(std::collections::HashMap::new())),
        );
    })
}

/// Connect like the one-shot CLI client: AUTH, expect OK, return the socket.
fn connect_authenticated(addr: std::net::SocketAddr) -> TcpStream {
    let mut client = TcpStream::connect(addr).expect("client connect");
    client.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    write!(client, "AUTH {TEST_KEY}\n").unwrap();
    let mut reader = std::io::BufReader::new(client.try_clone().unwrap());
    let mut ok = String::new();
    reader.read_line(&mut ok).expect("read OK");
    assert_eq!(ok, "OK\n", "handler must authenticate the client");
    client
}

/// End-to-end: a wedged server (request never answered) must produce the
/// timed-out error after the 5s bound — never an indefinite hang.
#[test]
fn one_shot_capture_pane_wait_is_bounded_by_five_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let (tx, _rx) = mpsc::channel::<CtrlReq>(); // never serviced
    let handler = spawn_handler(listener.try_clone().unwrap(), tx);

    let mut client = connect_authenticated(listener.local_addr().unwrap());
    let t0 = Instant::now();
    write!(client, "capture-pane\n").unwrap();
    client.shutdown(Shutdown::Write).unwrap();

    let mut resp = String::new();
    client.read_to_string(&mut resp).expect("read response");
    let elapsed = t0.elapsed();
    handler.join().expect("handler thread");

    assert_eq!(
        resp, "ERROR: capture-pane timed out\n",
        "a wedged server must yield the explicit timeout error"
    );
    assert!(
        elapsed >= Duration::from_secs(4),
        "the wait must actually run the 5s bound, not fail instantly (took {elapsed:?})"
    );
    assert!(
        elapsed < Duration::from_secs(12),
        "the wait must be bounded (took {elapsed:?})"
    );
}

/// The helper behind the bounded wait errors out within the given timeout
/// instead of blocking forever.
#[test]
fn recv_control_response_times_out_with_the_command_error() {
    let (_tx, rx) = mpsc::channel::<String>();
    let t0 = Instant::now();
    let result = recv_control_response("capture-pane", rx, Duration::from_millis(100));
    assert_eq!(result, Err("capture-pane timed out".to_string()));
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "the bounded wait must return, not hang"
    );
}

/// A disconnected response channel reports an explicit error too.
#[test]
fn recv_control_response_reports_a_disconnected_channel() {
    let (tx, rx) = mpsc::channel::<String>();
    drop(tx);
    let result = recv_control_response("display-message", rx, Duration::from_secs(1));
    assert_eq!(
        result,
        Err("display-message response channel disconnected".to_string())
    );
}

/// The bounded wait still delivers a real response when the server answers.
#[test]
fn recv_control_response_delivers_queued_value() {
    let (tx, rx) = mpsc::channel::<String>();
    tx.send("pane text".to_string()).unwrap();
    let result = recv_control_response("capture-pane", rx, Duration::from_secs(1));
    assert_eq!(result, Ok("pane text".to_string()));
}
