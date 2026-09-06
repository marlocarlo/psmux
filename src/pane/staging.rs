//! Lossless PTY output staging. Only the reader waits for capacity; snapshots,
//! the parser, and the server event loop never wait on a full output queue.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

pub(super) const OUTPUT_BYTE_LIMIT: usize = 1024 * 1024;
pub(super) const PARSE_BATCH_LIMIT: usize = 64 * 1024;

struct State {
    bytes: VecDeque<u8>,
    reader_done: bool,
    parser_done: bool,
}

pub(super) struct Staging {
    state: Mutex<State>,
    changed: Condvar,
    limit: usize,
}

impl Staging {
    pub(super) fn new(limit: usize) -> Self {
        assert!(limit > 0);
        Self {
            state: Mutex::new(State { bytes: VecDeque::with_capacity(limit), reader_done: false, parser_done: false }),
            changed: Condvar::new(),
            limit,
        }
    }

    pub(super) fn push(&self, mut bytes: &[u8]) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while !bytes.is_empty() {
            while state.bytes.len() == self.limit && !state.parser_done {
                state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.parser_done { return false; }
            let n = bytes.len().min(self.limit - state.bytes.len());
            state.bytes.extend(&bytes[..n]);
            bytes = &bytes[n..];
            self.changed.notify_all();
        }
        true
    }

    pub(super) fn wait_for_data(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.bytes.is_empty() && !state.reader_done {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        !state.bytes.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).bytes.len()
    }

    pub(super) fn take(&self, max: usize) -> Vec<u8> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let n = max.min(state.bytes.len());
        let bytes = state.bytes.drain(..n).collect();
        self.changed.notify_all();
        bytes
    }

    pub(super) fn finish_reader(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).reader_done = true;
        self.changed.notify_all();
    }

    pub(super) fn finish_parser(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).parser_done = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
#[path = "../../tests-rs/test_audit_output_staging.rs"]
mod tests;
