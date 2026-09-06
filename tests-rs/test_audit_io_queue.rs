use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "worker did not reach expected state");
        thread::sleep(Duration::from_millis(1));
    }
}

struct Gated {
    entered: mpsc::Sender<()>,
    gate: mpsc::Receiver<()>,
    bytes: Arc<Mutex<Vec<u8>>>,
    largest: Arc<AtomicUsize>,
    first: bool,
}

impl Write for Gated {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.largest.fetch_max(bytes.len(), Ordering::Relaxed);
        if self.first {
            self.first = false;
            self.entered.send(()).unwrap();
            self.gate.recv().unwrap();
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

#[test]
fn input_budget_includes_blocked_inflight_bytes_and_rejects_atomically() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (gate_tx, gate_rx) = mpsc::channel();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let largest = Arc::new(AtomicUsize::new(0));
    let inner = Gated { entered: entered_tx, gate: gate_rx, bytes: bytes.clone(), largest: largest.clone(), first: true };
    let queue = ByteWriter::spawn("audit-input-budget", 12, true, move || Ok(Box::new(inner))).unwrap();
    assert_eq!(queue.enqueue(b"first!").unwrap(), 6);
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(queue.outstanding(), 6);
    assert_eq!(queue.enqueue(b"second").unwrap(), 6);
    let now = Instant::now();
    assert_eq!(queue.enqueue(b"never").unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(now.elapsed() < Duration::from_millis(250));
    assert_eq!(queue.outstanding(), 12);
    gate_tx.send(()).unwrap();
    until(|| queue.outstanding() == 0);
    assert_eq!(&*bytes.lock().unwrap(), b"first!second");
    assert!(largest.load(Ordering::Relaxed) <= WRITE_BATCH_LIMIT);
}

#[test]
fn accepted_large_write_is_delivered_in_bounded_batches() {
    struct Recorder(Arc<Mutex<Vec<u8>>>, Arc<AtomicUsize>);
    impl Write for Recorder {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.1.fetch_max(b.len(), Ordering::Relaxed);
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let largest = Arc::new(AtomicUsize::new(0));
    let inner = Recorder(output.clone(), largest.clone());
    let queue = ByteWriter::spawn("audit-input-batch", INPUT_BYTE_LIMIT, true, move || Ok(Box::new(inner))).unwrap();
    let bytes: Vec<_> = (0..300_000).map(|i| (i % 251) as u8).collect();
    queue.enqueue(&bytes).unwrap();
    until(|| queue.outstanding() == 0);
    assert_eq!(*output.lock().unwrap(), bytes);
    assert!(largest.load(Ordering::Relaxed) <= WRITE_BATCH_LIMIT);
}

#[test]
fn short_writes_and_transient_errors_preserve_unwritten_suffix() {
    struct Short(usize, Arc<Mutex<Vec<u8>>>);
    impl Write for Short {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0 += 1;
            if self.0 == 2 { return Err(io::ErrorKind::Interrupted.into()); }
            if self.0 == 4 { return Err(io::ErrorKind::WouldBlock.into()); }
            let n = b.len().min(2);
            self.1.lock().unwrap().extend_from_slice(&b[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let inner = Short(0, output.clone());
    let queue = ByteWriter::spawn("audit-input-short", 20, true, move || Ok(Box::new(inner))).unwrap();
    queue.enqueue(b"abcdefghijk").unwrap();
    until(|| queue.outstanding() == 0);
    assert_eq!(&*output.lock().unwrap(), b"abcdefghijk");
}

#[test]
fn permanent_failure_is_visible_and_retains_pty_handle_and_bytes_until_drop() {
    struct Broken(Arc<AtomicBool>);
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::ErrorKind::BrokenPipe.into()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl Drop for Broken { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }
    let dropped = Arc::new(AtomicBool::new(false));
    let inner = Broken(dropped.clone());
    let mut queue = ByteWriter::spawn("audit-input-failed", 100, true, move || Ok(Box::new(inner))).unwrap();
    queue.enqueue(b"accepted").unwrap();
    until(|| queue.failure().is_some());
    assert_eq!(queue.flush().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(queue.enqueue(b"rejected").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(queue.outstanding(), 8);
    assert!(!dropped.load(Ordering::Acquire));
    drop(queue);
    until(|| dropped.load(Ordering::Acquire));
}

#[test]
fn flush_error_is_observable() {
    struct FailsFlush;
    impl Write for FailsFlush {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> { Ok(b.len()) }
        fn flush(&mut self) -> io::Result<()> { Err(io::ErrorKind::PermissionDenied.into()) }
    }
    let queue = ByteWriter::spawn("audit-input-flush", 100, true, || Ok(Box::new(FailsFlush))).unwrap();
    queue.enqueue(b"hello").unwrap();
    until(|| queue.failure().is_some());
    assert_eq!(queue.check().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn normal_finish_drains_accepted_bytes() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (gate_tx, gate_rx) = mpsc::channel();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let inner = Gated { entered: entered_tx, gate: gate_rx, bytes: bytes.clone(), largest: Arc::default(), first: true };
    let queue = ByteWriter::spawn("audit-sink-finish", 100, false, move || Ok(Box::new(inner))).unwrap();
    queue.enqueue(b"before").unwrap();
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    queue.enqueue(b"after").unwrap();
    queue.finish();
    gate_tx.send(()).unwrap();
    until(|| queue.is_finished());
    assert_eq!(&*bytes.lock().unwrap(), b"beforeafter");
    assert!(queue.failure().is_none());
}

#[test]
fn cancellation_does_not_wait_for_blocked_writer() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (gate_tx, gate_rx) = mpsc::channel();
    let inner = Gated { entered: entered_tx, gate: gate_rx, bytes: Arc::default(), largest: Arc::default(), first: true };
    let queue = ByteWriter::spawn("audit-sink-cancel", 100, false, move || Ok(Box::new(inner))).unwrap();
    queue.enqueue(b"blocked").unwrap();
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let now = Instant::now();
    queue.cancel();
    assert!(now.elapsed() < Duration::from_millis(250));
    // A synthetic Condvar/channel is not Windows I/O and cannot be cancelled
    // with CancelSynchronousIo. Release it explicitly after proving promptness.
    gate_tx.send(()).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_cancellation_releases_a_real_blocked_pipe_without_a_reader() {
    use std::os::windows::io::FromRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(read: *mut *mut std::ffi::c_void, write: *mut *mut std::ffi::c_void,
            attributes: *mut std::ffi::c_void, size: u32) -> i32;
    }
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    assert_ne!(unsafe { CreatePipe(&mut read, &mut write, std::ptr::null_mut(), 4096) }, 0);
    // Keep the read endpoint open and never consume it. Cancellation, not EOF,
    // must release WriteFile and the worker even if it races the write start.
    let _reader = unsafe { std::fs::File::from_raw_handle(read) };
    let file = unsafe { std::fs::File::from_raw_handle(write) };
    let (tx, rx) = mpsc::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    struct PipeWriter(std::fs::File, mpsc::Sender<()>, Arc<AtomicBool>);
    impl Write for PipeWriter {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> { let _ = self.1.send(()); self.0.write(b) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl Drop for PipeWriter { fn drop(&mut self) { self.2.store(true, Ordering::Release); } }
    let inner = PipeWriter(file, tx, dropped.clone());
    let queue = ByteWriter::spawn("audit-real-blocked-pipe", INPUT_BYTE_LIMIT, false, move || Ok(Box::new(inner))).unwrap();
    queue.enqueue(&vec![42; WRITE_BATCH_LIMIT]).unwrap();
    rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(!dropped.load(Ordering::Acquire));
    queue.cancel();
    until(|| dropped.load(Ordering::Acquire));
}
