use super::*;
use std::sync::{mpsc, atomic::AtomicBool};
use std::time::{Duration, Instant};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "sink did not reach expected state");
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Blocked(mpsc::Sender<()>, mpsc::Receiver<()>);
impl Write for Blocked {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let _ = self.0.send(());
        let _ = self.1.recv();
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

struct Record(Arc<Mutex<Vec<u8>>>);
impl Write for Record {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> { self.0.lock().unwrap().extend_from_slice(b); Ok(b.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

#[test]
fn blocked_sink_cannot_block_other_sink_or_unregister() {
    let _test = TEST_LOCK.lock().unwrap();
    let pane = usize::MAX - 101;
    let other = pane - 1;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    register(pane, Box::new(Blocked(entered_tx, release_rx))).unwrap();
    tee(pane, b"blocked");
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    register(other, Box::new(Record(bytes.clone()))).unwrap();
    tee(other, b"unrelated output");
    until(|| !bytes.lock().unwrap().is_empty());
    assert_eq!(&*bytes.lock().unwrap(), b"unrelated output");
    let now = Instant::now();
    unregister(pane);
    assert!(now.elapsed() < Duration::from_millis(250));
    assert!(!is_registered(pane));
    release_tx.send(()).unwrap();
    unregister(other);
}

#[test]
fn overflow_stops_sink_and_reports_it_once() {
    let _test = TEST_LOCK.lock().unwrap();
    let pane = usize::MAX - 103;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    register(pane, Box::new(Blocked(entered_tx, release_rx))).unwrap();
    tee(pane, b"first");
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    tee(pane, &vec![b'x'; SINK_BYTE_LIMIT]);
    let failures = take_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, pane);
    assert!(failures[0].1.contains("queue full"));
    assert!(!is_registered(pane));
    assert!(take_failures().is_empty());
    release_tx.send(()).unwrap();
}

#[test]
fn idle_sink_failure_is_reported_without_next_pty_chunk() {
    let _test = TEST_LOCK.lock().unwrap();
    let pane = usize::MAX - 104;
    let failed = Arc::new(AtomicBool::new(false));
    let failed_worker = failed.clone();
    register_factory(pane, false, move || {
        failed_worker.store(true, Ordering::Release);
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "open failed"))
    }).unwrap();
    until(|| failed.load(Ordering::Acquire));
    let mut failures = Vec::new();
    until(|| { failures = take_failures(); !failures.is_empty() });
    assert_eq!(failures[0].0, pane);
    assert!(failures[0].1.contains("open failed"));
    assert!(!is_registered(pane));
}

#[test]
fn normal_eof_delivers_queued_transcript() {
    let _test = TEST_LOCK.lock().unwrap();
    let pane = usize::MAX - 105;
    let bytes = Arc::new(Mutex::new(Vec::new()));
    register(pane, Box::new(Record(bytes.clone()))).unwrap();
    tee(pane, b"final transcript");
    finish_pane(pane);
    until(|| bytes.lock().unwrap().len() == 16);
    assert_eq!(&*bytes.lock().unwrap(), b"final transcript");
    assert!(take_failures().is_empty());
    unregister(pane);
}
