#[allow(unused_imports)]
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyModifiers};
use portable_pty::MasterPty;
use ratatui::prelude::Rect;
use chrono::Local;

use super::*;

/// Check if any persistent clients are registered for push.
pub fn has_frame_receivers() -> bool {
    FRAME_PUSH_CHANNELS.lock().map_or(false, |v| !v.is_empty())
}

/// Per-client directive channels (queued, not overwritten like frame slots).
/// Used for sending commands/directives (e.g. SWITCH) to specific persistent clients
/// without risk of being overwritten by frame pushes.
pub(crate) static DIRECTIVE_CHANNELS: std::sync::Mutex<Vec<(u64, std::sync::mpsc::Sender<String>)>> =
    std::sync::Mutex::new(Vec::new());

/// Register a directive channel for a persistent client. Returns the receiver
/// for the writer thread to poll.
pub fn register_directive_channel(client_id: u64) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Ok(mut v) = DIRECTIVE_CHANNELS.lock() {
        v.push((client_id, tx));
    }
    rx
}

/// Send a directive to a specific persistent client. Returns true if sent.
pub fn send_directive_to_client(client_id: u64, directive: &str) -> bool {
    if let Ok(channels) = DIRECTIVE_CHANNELS.lock() {
        for (cid, tx) in channels.iter() {
            if *cid == client_id {
                return tx.send(directive.to_string()).is_ok();
            }
        }
    }
    false
}

/// Send a directive to ALL persistent clients.
pub fn send_directive_to_all_clients(directive: &str) {
    if let Ok(channels) = DIRECTIVE_CHANNELS.lock() {
        for (_, tx) in channels.iter() {
            let _ = tx.send(directive.to_string());
        }
    }
}

/// Remove a client's directive channel (called on disconnect).
pub fn remove_directive_channel(client_id: u64) {
    if let Ok(mut v) = DIRECTIVE_CHANNELS.lock() {
        v.retain(|(cid, _)| *cid != client_id);
    }
}

/// Global counter for control mode client IDs.
pub(crate) static NEXT_CONTROL_CLIENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Allocate a unique control mode client ID.
pub fn next_control_client_id() -> u64 {
    NEXT_CONTROL_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Wait-for operation types
#[derive(Clone, Copy)]
pub enum WaitForOp {
    Wait,
    Lock,
    Signal,
    Unlock,
}

/// Parsed target specification from -t argument.
#[derive(Debug, Clone, Default)]
pub struct ParsedTarget {
    pub session: Option<String>,
    pub window: Option<usize>,
    pub window_name: Option<String>,
    pub pane: Option<usize>,
    pub pane_is_id: bool,
    pub window_is_id: bool,
}
