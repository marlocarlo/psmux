// Regression tests for the per-pane writer thread fix (39c9f8a).
//
// Writing to a pane used to go straight to the ConPTY input pipe from the
// server loop. That pipe has a fixed 64KB buffer, so a pane child that
// stopped reading stdin made write_all block the single server thread,
// wedging every session. The fix routes every pane write through
// `spawn_pane_write_queue`: a per-pane queue drained by a dedicated thread,
// mirroring tmux's libevent bufferevent contract — writes complete
// immediately while below the byte limit, and stay ordered. The thread
// exits cleanly when the queue side is dropped with the pane.
//
// These tests wrap a dummy writer (no PTY, no spawn) and observe the queue
// contract: immediate completion, order preservation, and clean teardown.

use super::*;

use parking_lot::{Condvar, Mutex};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Poll `cond` until it is true or the deadline passes.
fn wait_until(what: &str, mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if cond() {
        return true;
    }
    eprintln!("wait_until timed out: {what}");
    false
}

/// A writer that records every byte it receives and flags its own drop.
#[derive(Debug, Default)]
struct RecordingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    attempts: Arc<Mutex<usize>>,
    dropped: Arc<AtomicBool>,
}

impl Write for RecordingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        *self.attempts.lock() += 1;
        self.bytes.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for RecordingWriter {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// A writer that blocks until its gate is opened (simulating a pane child
/// whose stdin pipe is full).
struct GateWriter {
    state: Arc<(Mutex<Vec<u8>>, Condvar)>,
    open: Arc<AtomicBool>,
}

impl Write for GateWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let (lock, cv) = &*self.state;
        let mut bytes = lock.lock();
        while !self.open.load(Ordering::SeqCst) {
            cv.wait(&mut bytes);
        }
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Writes complete immediately and are delivered to the underlying PTY
/// writer in the exact order they were issued.
#[test]
fn queued_writes_are_delivered_in_order() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let attempts = Arc::new(Mutex::new(0usize));
    let dropped = Arc::new(AtomicBool::new(false));
    let writer = RecordingWriter { bytes: bytes.clone(), attempts: attempts.clone(), dropped: dropped.clone() };

    let mut queue = spawn_pane_write_queue(Box::new(writer));

    // Each write must return the full length immediately (no blocking on
    // the inner writer).
    assert_eq!(queue.write(b"hello ").unwrap(), 6);
    assert_eq!(queue.write(b"world").unwrap(), 5);
    assert_eq!(queue.write(b"\n!").unwrap(), 2);

    assert!(
        wait_until("ordered delivery", || *bytes.lock() == b"hello world\n!".to_vec(), Duration::from_secs(5)),
        "queued writes must reach the inner writer in order"
    );
    assert!(!dropped.load(Ordering::SeqCst), "inner writer must stay alive while the pane is open");

    drop(queue);
    assert!(
        wait_until("thread exit drops inner writer", || dropped.load(Ordering::SeqCst), Duration::from_secs(5)),
        "dropping the queue must end the writer thread and release the inner writer"
    );
}

/// Backpressure absorption: while the underlying writer is blocked (child
/// not reading), a write still completes immediately and the bytes are
/// buffered in memory — the exact contract that replaced blocking the
/// server thread on a full 64KB pipe.
#[test]
fn write_completes_while_inner_writer_is_blocked() {
    let state = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    let open = Arc::new(AtomicBool::new(false));
    let inner = GateWriter { state: state.clone(), open: open.clone() };

    let mut queue = spawn_pane_write_queue(Box::new(inner));

    let t0 = Instant::now();
    assert_eq!(queue.write(b"payload").unwrap(), 7, "write must return without blocking");
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "write must not wait for the inner writer"
    );
    // Nothing has reached the inner writer yet.
    assert!(state.0.lock().is_empty());

    // Let the inner writer drain the queue. Lock-then-notify: the drainer
    // thread checks `open` while HOLDING the state mutex before parking in
    // cv.wait, so storing the flag and notifying without acquiring that
    // mutex could fire the wakeup in the window between its check and the
    // park — a lost wakeup that left the drainer blocked forever and the
    // wait_until below timing out (one-off failures under full parallel
    // runs, where preemption widens the window). Acquiring and releasing
    // the mutex after the store serializes against the check: either the
    // drainer has not checked yet and will see open=true, or it is already
    // parked and the notify lands.
    open.store(true, Ordering::SeqCst);
    drop(state.0.lock());
    state.1.notify_all();
    assert!(
        wait_until("blocked write drains", || *state.0.lock() == b"payload".to_vec(), Duration::from_secs(5)),
        "released inner writer must receive the queued bytes"
    );

    drop(queue);
}

/// Dropping the queue side (pane close) ends the writer thread and releases
/// the inner writer even when no write ever happened.
#[test]
fn pane_close_ends_writer_thread_cleanly() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let attempts = Arc::new(Mutex::new(0usize));
    let dropped = Arc::new(AtomicBool::new(false));
    let writer = RecordingWriter { bytes, attempts, dropped: dropped.clone() };

    let queue = spawn_pane_write_queue(Box::new(writer));
    drop(queue);

    assert!(
        wait_until("writer thread exits on queue drop", || dropped.load(Ordering::SeqCst), Duration::from_secs(5)),
        "the pane-writer thread must exit when the pane's queue is dropped"
    );
}

/// A burst of writes with no reader must still complete immediately and
/// deliver every byte once the inner writer catches up — the queue absorbs
/// backpressure up to its documented byte budget.
#[test]
fn burst_writes_survive_backpressure_and_arrive_complete() {
    let state = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    let open = Arc::new(AtomicBool::new(false));
    let inner = GateWriter { state: state.clone(), open: open.clone() };

    let mut queue = spawn_pane_write_queue(Box::new(inner));
    let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();

    let t0 = Instant::now();
    for chunk in payload.chunks(1024) {
        assert_eq!(queue.write(chunk).unwrap(), chunk.len());
    }
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "backpressured burst must enqueue without blocking"
    );

    // Lock-then-notify — see write_completes_while_inner_writer_is_blocked
    // for why notifying without the state mutex is a lost wakeup.
    open.store(true, Ordering::SeqCst);
    drop(state.0.lock());
    state.1.notify_all();
    assert!(
        wait_until("burst arrives intact", || *state.0.lock() == payload, Duration::from_secs(5)),
        "the full burst must arrive byte-identical and in order"
    );
    drop(queue);
}
