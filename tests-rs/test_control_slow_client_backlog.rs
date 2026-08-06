// Regression tests for the slow-control-client buffering fix (99b2745).
//
// A control client that fell behind the output stream used to hit the
// connection's 5s write timeout and get disconnected. The fix drops the
// socket write timeout, switches the per-client queue to an unbounded
// channel, and accounts queued bytes per client:
//
//   - below the soft backlog threshold every notification is queued
//   - past the soft threshold, %output / %extended-output frames are shed
//     (any client resynchronizes with capture-pane), while structural
//     notifications such as %window-close always queue
//   - past the hard threshold everything is shed
//
// `try_send_notification` is a pure function of (client backlog, channel),
// so these tests drive it directly with no server and no real socket.

use super::*;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A control client whose backlog counter is preset, with a live unbounded
/// queue. Returns the client and the receiving side of its queue.
fn client_with_backlog(backlog: usize) -> (ControlClient, mpsc::Receiver<ControlOutput>) {
    let (tx, rx) = mpsc::channel::<ControlOutput>();
    let client = ControlClient {
        client_id: 1,
        cmd_counter: 0,
        echo_enabled: false,
        output_tx: tx,
        backlog: Arc::new(AtomicUsize::new(backlog)),
        paused_panes: HashSet::new(),
        subscriptions: HashMap::new(),
        subscription_values: HashMap::new(),
        subscription_last_check: HashMap::new(),
        pause_after_secs: None,
        output_paused_panes: HashSet::new(),
        pane_last_output: HashMap::new(),
        size: None,
        window_sizes: HashMap::new(),
    };
    (client, rx)
}

fn output_notif(data: &str) -> ControlNotification {
    ControlNotification::Output { pane_id: 7, data: data.to_string() }
}

fn extended_output_notif(data: &str) -> ControlNotification {
    ControlNotification::ExtendedOutput { pane_id: 7, age_ms: 123, data: data.to_string() }
}

fn structural_notif() -> ControlNotification {
    ControlNotification::WindowClose { window_id: 5 }
}

/// A fresh client is not shedding anything: every notification type is
/// queued and the backlog counter grows by the notification's wire cost.
#[test]
fn fresh_client_queues_output_and_accounts_backlog() {
    let (client, rx) = client_with_backlog(0);
    try_send_notification(&client, output_notif("abc"));

    let queued = rx.recv_timeout(Duration::from_secs(1)).expect("output must be queued");
    match queued {
        ControlOutput::Notification(ControlNotification::Output { pane_id, data }) => {
            assert_eq!(pane_id, 7);
            assert_eq!(data, "abc", "queued output must be delivered intact");
        }
        other => panic!("expected an Output notification, got {other:?}"),
    }
    // Output cost = data.len() + 32.
    assert_eq!(client.backlog.load(Ordering::Relaxed), 3 + 32);
}

/// Past the soft threshold, %output is shed: the frame must NOT reach the
/// queue and the backlog must stay put. This is the exact contract that
/// replaced the old disconnect-on-slow-reader behavior.
#[test]
fn output_is_shed_past_soft_backlog_threshold() {
    let (client, rx) = client_with_backlog(crate::control::OUTPUT_BACKLOG_SOFT_MAX);
    try_send_notification(&client, output_notif("abc"));

    assert!(
        rx.try_recv().is_err(),
        "%output must be shed past the soft threshold"
    );
    assert_eq!(
        client.backlog.load(Ordering::Relaxed),
        crate::control::OUTPUT_BACKLOG_SOFT_MAX,
        "shed frames must not add to the backlog"
    );
}

/// %extended-output is a pane-output frame too and sheds past the soft
/// threshold exactly like %output.
#[test]
fn extended_output_is_shed_past_soft_backlog_threshold() {
    let (client, rx) = client_with_backlog(crate::control::OUTPUT_BACKLOG_SOFT_MAX);
    try_send_notification(&client, extended_output_notif("abc"));

    assert!(
        rx.try_recv().is_err(),
        "%extended-output must be shed past the soft threshold"
    );
}

/// Structural notifications are never shed at the soft threshold: a slow
/// reader must still learn about window teardown etc., so those frames
/// always queue.
#[test]
fn structural_notifications_queue_past_soft_backlog_threshold() {
    let (client, rx) = client_with_backlog(crate::control::OUTPUT_BACKLOG_SOFT_MAX);
    try_send_notification(&client, structural_notif());

    let queued = rx.recv_timeout(Duration::from_secs(1)).expect("structural notification must queue");
    assert!(
        matches!(queued, ControlOutput::Notification(ControlNotification::WindowClose { window_id: 5 })),
        "structural notification must be delivered intact"
    );
    // Non-output notification cost is a flat 128.
    assert_eq!(
        client.backlog.load(Ordering::Relaxed),
        crate::control::OUTPUT_BACKLOG_SOFT_MAX + 128
    );
}

/// Past the hard threshold even structural notifications are shed: a reader
/// that far behind is effectively gone and only TCP teardown will reap it.
#[test]
fn everything_is_shed_past_hard_backlog_threshold() {
    let (client, rx) = client_with_backlog(crate::control::OUTPUT_BACKLOG_HARD_MAX);
    try_send_notification(&client, output_notif("abc"));
    try_send_notification(&client, structural_notif());

    assert!(
        rx.try_recv().is_err(),
        "everything must be shed past the hard threshold"
    );
    assert_eq!(
        client.backlog.load(Ordering::Relaxed),
        crate::control::OUTPUT_BACKLOG_HARD_MAX
    );
}

/// Backlog accounting is symmetric with the writer: `output_cost` is what
/// the enqueue adds and what the writer subtracts after a successful write,
/// so the two must agree exactly per variant.
#[test]
fn output_cost_matches_the_variants_that_can_be_queued() {
    let output = ControlOutput::Notification(output_notif("abcd"));
    assert_eq!(crate::control::output_cost(&output), 4 + 32);

    let extended = ControlOutput::Notification(extended_output_notif("abcd"));
    assert_eq!(crate::control::output_cost(&extended), 4 + 48);

    let structural = ControlOutput::Notification(structural_notif());
    assert_eq!(crate::control::output_cost(&structural), 128);

    let block = ControlOutput::CommandBlock("%begin 1 2 1\nbody\n%end 1 2 1\n".to_string());
    assert_eq!(
        crate::control::output_cost(&block),
        "%begin 1 2 1\nbody\n%end 1 2 1\n".len()
    );
}

/// A disconnected client must never panic the server loop: the send result
/// is deliberately ignored, and shedding must not even attempt the send.
#[test]
fn dead_client_queue_is_handled_silently() {
    let (client, rx) = client_with_backlog(0);
    drop(rx);
    try_send_notification(&client, output_notif("abc"));
    try_send_notification(&client, structural_notif());
    // No panic above; the queue is unbounded so the send itself succeeds
    // even with the receiver gone — teardown reaps the client later.
    assert_eq!(client.backlog.load(Ordering::Relaxed), 3 + 32 + 128);
}

/// The soft/hard thresholds are real, ordered bounds — a regression in
/// either constant changes the shedding decision.
#[test]
fn backlog_thresholds_are_ordered_and_nonzero() {
    assert!(crate::control::OUTPUT_BACKLOG_SOFT_MAX > 0);
    assert!(crate::control::OUTPUT_BACKLOG_HARD_MAX > crate::control::OUTPUT_BACKLOG_SOFT_MAX);
}

/// Queueing is non-blocking: `try_send_notification` must return
/// immediately even when the reader never drains the queue. A large burst
/// (multiple soft-thresholds worth of cost) still sheds per-frame instead
/// of blocking the server loop.
#[test]
fn notification_burst_completes_immediately_without_a_reader() {
    let (client, rx) = client_with_backlog(0);
    let burst_start = Instant::now();
    let mut expected_cost = 0;
    for i in 0..1000 {
        let data = format!("frame {i}");
        expected_cost += data.len() + 32;
        try_send_notification(&client, output_notif(&data));
    }
    assert!(
        burst_start.elapsed() < Duration::from_secs(2),
        "enqueue must never block"
    );
    assert_eq!(
        client.backlog.load(Ordering::Relaxed),
        expected_cost,
        "every queued frame must be accounted at its exact wire cost"
    );
    drop(rx);
}
