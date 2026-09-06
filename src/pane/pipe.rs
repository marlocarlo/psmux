//! Independent pipe-pane sinks. The registry stores queue handles, never raw
//! writers, and its mutex is never held while a sink performs blocking I/O.

use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::io_queue::ByteWriter;

const SINK_BYTE_LIMIT: usize = 1024 * 1024;

struct Sink {
    pane_id: usize,
    writer: Arc<ByteWriter>,
    tunnel: bool,
}

static SINKS: Mutex<Vec<Sink>> = Mutex::new(Vec::new());
static COUNT: AtomicUsize = AtomicUsize::new(0);

fn register_factory(
    pane_id: usize,
    tunnel: bool,
    open: impl FnOnce() -> io::Result<Box<dyn Write + Send>> + Send + 'static,
) -> io::Result<()> {
    let writer = ByteWriter::spawn("pipe-sink", SINK_BYTE_LIMIT, false, open)?;
    let mut sinks = SINKS.lock().unwrap_or_else(|e| e.into_inner());
    sinks.push(Sink { pane_id, writer: Arc::new(writer), tunnel });
    COUNT.store(sinks.len(), Ordering::Release);
    Ok(())
}

pub fn register(pane_id: usize, writer: Box<dyn Write + Send>) -> io::Result<()> {
    register_factory(pane_id, false, move || Ok(writer))
}

pub fn register_tunnel(pane_id: usize, writer: Box<dyn Write + Send>) -> io::Result<()> {
    register_factory(pane_id, true, move || Ok(writer))
}

/// Opening a path can block too (including through a local junction). Resolve
/// and open it on the cancellable worker, never on the server's event loop.
pub fn register_file(pane_id: usize, path: String, append: bool) -> io::Result<()> {
    register_factory(pane_id, false, move || {
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true);
        if append { opts.append(true); } else { opts.truncate(true); }
        opts.open(&path).map(|file| Box::new(file) as Box<dyn Write + Send>)
            .map_err(|error| io::Error::new(error.kind(), format!("can't open {}: {}", path, error)))
    })
}

pub fn is_registered(pane_id: usize) -> bool {
    SINKS.lock().unwrap_or_else(|e| e.into_inner()).iter()
        .any(|sink| sink.pane_id == pane_id && !sink.tunnel && sink.writer.check().is_ok())
}

pub fn unregister(pane_id: usize) {
    remove_where(|sink| sink.pane_id == pane_id);
}

/// Normal PTY EOF must deliver the bytes already admitted before closing the
/// sink's stdin. Explicit cancel/replace uses unregister instead.
pub(super) fn finish_pane(pane_id: usize) {
    let sinks = SINKS.lock().unwrap_or_else(|e| e.into_inner());
    for sink in sinks.iter().filter(|sink| sink.pane_id == pane_id) {
        sink.writer.finish();
    }
}

fn remove_where(mut remove: impl FnMut(&Sink) -> bool) {
    let removed = {
        let mut sinks = SINKS.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = Vec::new();
        let mut index = 0;
        while index < sinks.len() {
            if remove(&sinks[index]) { removed.push(sinks.remove(index)); }
            else { index += 1; }
        }
        COUNT.store(sinks.len(), Ordering::Release);
        removed
    };
    for sink in removed { sink.writer.cancel(); }
}

/// Called from PTY readers. Admission is bounded and never waits for sink I/O.
/// Overflow terminates that sink with a visible failure; it cannot deadlock
/// other sinks, the parser, or cancel/replace on the main loop.
pub(super) fn tee(pane_id: usize, bytes: &[u8]) {
    if COUNT.load(Ordering::Acquire) == 0 { return; }
    let writers: Vec<_> = SINKS.lock().unwrap_or_else(|e| e.into_inner()).iter()
        .filter(|sink| sink.pane_id == pane_id)
        .map(|sink| sink.writer.clone()).collect();
    for writer in writers {
        if let Err(error) = writer.enqueue(bytes) {
            writer.fail(io::Error::new(error.kind(), format!("pipe-pane sink stopped: {}", error)));
            writer.cancel();
        }
    }
}

/// The server drains this once per tick, reports the failure and kills/reaps
/// the corresponding sink child. A failed sink itself is the pending record:
/// there is no separate unbounded error queue, and idle sink failures are seen.
pub fn take_failures() -> Vec<(usize, String)> {
    let mut failures = Vec::new();
    let mut tunnel_failures = Vec::new();
    remove_where(|sink| {
        let finished = sink.writer.is_finished();
        if !finished && sink.writer.finish_expired(std::time::Duration::from_secs(5)) {
            sink.writer.fail(io::Error::new(io::ErrorKind::TimedOut,
                "pipe-pane sink did not drain within five seconds after pane EOF"));
        }
        if let Some(error) = sink.writer.failure() {
            if !sink.tunnel { failures.push((sink.pane_id, error.to_string())); }
            else { tunnel_failures.push((sink.pane_id, error.to_string())); }
            true
        } else { finished }
    });
    for (pane_id, error) in tunnel_failures {
        crate::debug_log::server_log("pipe", &format!("pane tunnel %{} stopped: {}", pane_id, error));
    }
    failures
}

#[cfg(test)]
#[path = "../../tests-rs/test_audit_pipe_sinks.rs"]
mod tests;
