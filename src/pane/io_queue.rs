//! Byte-budgeted admission to a writer that may block indefinitely.
//!
//! The budget includes bytes in flight, not only queued messages. Producers
//! never perform I/O or wait for space. A successful write means admission;
//! a subsequent `flush`/write exposes asynchronous failure. PTY handles remain
//! alive after failure until their owner drops the queue, avoiding spurious EOF.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) const INPUT_BYTE_LIMIT: usize = 1024 * 1024;
pub(super) const WRITE_BATCH_LIMIT: usize = 64 * 1024;
const MAX_WRITER_WORKERS: usize = 256;
static WRITER_WORKERS: AtomicUsize = AtomicUsize::new(0);

/// Permits belong to the JoinHandle, not its thread closure. A cancelled
/// operation that cannot be interrupted keeps its permit until confirmed exit;
/// repeated replace/cancel therefore cannot leak arbitrarily many workers.
struct WorkerPermit;
impl WorkerPermit {
    fn acquire() -> io::Result<Self> {
        WRITER_WORKERS.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |n| (n < MAX_WRITER_WORKERS).then_some(n + 1))
            .map(|_| Self)
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock,
                "writer task limit reached (256 active or cancelling writers)"))
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) { WRITER_WORKERS.fetch_sub(1, Ordering::AcqRel); }
}

struct Worker {
    thread: JoinHandle<()>,
    _permit: WorkerPermit,
}

static CANCELLED: Mutex<Vec<Worker>> = Mutex::new(Vec::new());
static CANCEL_WAKE: Condvar = Condvar::new();
static REAPER_STARTED: OnceLock<Result<(), String>> = OnceLock::new();

fn ensure_reaper() -> io::Result<()> {
    REAPER_STARTED.get_or_init(|| {
        thread::Builder::new().name("writer-cancel-reaper".into()).spawn(|| loop {
            let pending = {
                let mut cancelled = CANCELLED.lock().unwrap_or_else(|e| e.into_inner());
                while cancelled.is_empty() {
                    cancelled = CANCEL_WAKE.wait(cancelled).unwrap_or_else(|e| e.into_inner());
                }
                std::mem::take(&mut *cancelled)
            };
            let mut still_running = Vec::new();
            for worker in pending {
                if worker.thread.is_finished() {
                    let Worker { thread, _permit } = worker;
                    let _ = thread.join();
                } else {
                    cancel_synchronous_io(&worker.thread);
                    still_running.push(worker);
                }
            }
            let mut cancelled = CANCELLED.lock().unwrap_or_else(|e| e.into_inner());
            cancelled.extend(still_running);
            if !cancelled.is_empty() {
                let _ = CANCEL_WAKE.wait_timeout(cancelled, Duration::from_millis(10));
            }
        }).map(|_| ()).map_err(|error| error.to_string())
    }).as_ref().map(|_| ()).map_err(|error| io::Error::other(error.clone()))
}

#[derive(Clone)]
struct Failure {
    kind: io::ErrorKind,
    message: String,
}

impl Failure {
    fn error(&self) -> io::Error { io::Error::new(self.kind, self.message.clone()) }
}

#[derive(Default)]
struct State {
    chunks: VecDeque<Vec<u8>>,
    /// Includes the worker's current batch until the underlying write succeeds.
    bytes: usize,
    closed: bool,
    cancelled: bool,
    finished_at: Option<Instant>,
    failure: Option<Failure>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    limit: usize,
}

impl Shared {
    fn fail(&self, error: io::Error) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.failure.is_none() {
            state.failure = Some(Failure {
                kind: error.kind(),
                message: error.to_string().chars().take(512).collect(),
            });
        }
        self.changed.notify_all();
    }
}

pub(super) struct ByteWriter {
    shared: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
}

impl ByteWriter {
    pub(super) fn spawn(
        name: &str,
        limit: usize,
        retain_after_failure: bool,
        open: impl FnOnce() -> io::Result<Box<dyn Write + Send>> + Send + 'static,
    ) -> io::Result<Self> {
        ensure_reaper()?;
        let permit = WorkerPermit::acquire()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            limit,
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new().name(name.into()).spawn(move || {
            // A panic must become visible to producers even if it happened in
            // user-supplied writer code. No registry or PTY cleanup in a hook.
            struct ExitGuard(Arc<Shared>);
            impl Drop for ExitGuard {
                fn drop(&mut self) {
                    if thread::panicking() {
                        self.0.fail(io::Error::other("writer task panicked"));
                    }
                }
            }
            let _guard = ExitGuard(worker_shared.clone());
            let mut inner = match open() {
                Ok(inner) => inner,
                Err(error) => { worker_shared.fail(error); return; }
            };
            run_writer(&worker_shared, &mut *inner, retain_after_failure);
        })?;
        Ok(Self { shared, worker: Mutex::new(Some(Worker { thread: worker, _permit: permit })) })
    }

    pub(super) fn enqueue(&self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure { return Err(error.error()); }
        if state.closed { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer closed")); }
        if bytes.len() > self.shared.limit.saturating_sub(state.bytes) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                format!("writer input queue full ({} byte limit); retry after the pane drains", self.shared.limit)));
        }
        // Allocate only after the complete write has been admitted. Partial
        // admission makes retrying a failed write_all duplicate input prefixes.
        let mut remaining = bytes;
        if let Some(last) = state.chunks.back_mut() {
            let append = remaining.len().min(WRITE_BATCH_LIMIT - last.len());
            last.extend_from_slice(&remaining[..append]);
            remaining = &remaining[append..];
        }
        for chunk in remaining.chunks(WRITE_BATCH_LIMIT) {
            state.chunks.push_back(chunk.to_vec());
        }
        state.bytes += bytes.len();
        self.shared.changed.notify_one();
        Ok(bytes.len())
    }

    pub(super) fn check(&self) -> io::Result<()> {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure { return Err(error.error()); }
        if state.closed { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer closed")); }
        Ok(())
    }

    pub(super) fn fail(&self, error: io::Error) { self.shared.fail(error); }

    pub(super) fn failure(&self) -> Option<io::Error> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).failure.as_ref().map(Failure::error)
    }

    pub(super) fn finish(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.finished_at = Some(Instant::now());
        self.shared.changed.notify_all();
    }

    pub(super) fn finish_expired(&self, timeout: Duration) -> bool {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).finished_at
            .is_some_and(|at| at.elapsed() >= timeout)
    }

    pub(super) fn is_finished(&self) -> bool {
        self.worker.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_none_or(|worker| worker.thread.is_finished())
    }

    /// Does not wait for the writer. Windows cancellation is repeated on a
    /// shared reaper because a single CancelSynchronousIo can race the worker
    /// between its closed check and entering WriteFile/CreateFile.
    pub(super) fn cancel(&self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state.cancelled = true;
            self.shared.changed.notify_all();
        }
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            cancel_synchronous_io(&worker.thread);
            CANCELLED.lock().unwrap_or_else(|e| e.into_inner()).push(worker);
            CANCEL_WAKE.notify_one();
        }
    }

    #[cfg(test)]
    fn outstanding(&self) -> usize { self.shared.state.lock().unwrap().bytes }
}

impl Write for ByteWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.enqueue(bytes) }
    fn flush(&mut self) -> io::Result<()> { self.check() }
}

impl Drop for ByteWriter {
    fn drop(&mut self) {
        // Owner drop is explicit cancellation (pane closed/replaced). Normal
        // transcript EOF instead calls finish() and keeps its owner registered
        // until the accepted suffix drains. Cancel blocked PTY writes too.
        self.cancel();
    }
}

#[cfg(windows)]
fn cancel_synchronous_io(worker: &JoinHandle<()>) {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CancelSynchronousIo(thread: *mut std::ffi::c_void) -> i32;
    }
    // The owned JoinHandle keeps the OS thread handle valid throughout the call.
    unsafe { CancelSynchronousIo(worker.as_raw_handle()); }
}

#[cfg(not(windows))]
fn cancel_synchronous_io(_worker: &JoinHandle<()>) {}

fn run_writer(shared: &Shared, inner: &mut dyn Write, retain_after_failure: bool) {
    loop {
        let batch = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if state.failure.is_some() {
                    if !retain_after_failure || state.closed { return; }
                } else if state.cancelled {
                    return;
                } else if let Some(chunk) = state.chunks.pop_front() {
                    break chunk;
                } else if state.closed { return; }
                state = shared.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let mut offset = 0;
        while offset < batch.len() {
            {
                let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.cancelled || state.failure.is_some() { break; }
            }
            match inner.write(&batch[offset..]) {
                Ok(0) => { shared.fail(io::Error::new(io::ErrorKind::WriteZero, "writer returned zero bytes")); break; }
                Ok(n) if n <= batch.len() - offset => {
                    offset += n;
                    shared.state.lock().unwrap_or_else(|e| e.into_inner()).bytes -= n;
                }
                Ok(_) => { shared.fail(io::Error::other("writer returned an invalid byte count")); break; }
                Err(error) if matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) => {
                    // Keep precisely the unwritten suffix; never replay an
                    // already accepted prefix after a short/transient write.
                    let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = shared.changed.wait_timeout(state, Duration::from_millis(2));
                }
                Err(error) => { shared.fail(error); break; }
            }
        }
        if offset == batch.len() {
            if let Err(error) = inner.flush() { shared.fail(error); }
        } else {
            // Preserve accepted but unwritten bytes until the failure is
            // observed/the queue is dropped. This is bounded by the byte budget.
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.chunks.push_front(batch[offset..].to_vec());
        }
    }
}

#[cfg(test)]
#[path = "../../tests-rs/test_audit_io_queue.rs"]
mod tests;
