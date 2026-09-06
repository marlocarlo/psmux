use super::*;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

#[test]
fn staging_blocks_only_reader_at_capacity_and_preserves_order() {
    let staging = Arc::new(Staging::new(8));
    assert!(staging.push(b"12345678"));
    let (tx, rx) = mpsc::channel();
    let producer = staging.clone();
    let worker = thread::spawn(move || { tx.send(producer.push(b"90ab")).unwrap(); producer.finish_reader(); });
    assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
    assert_eq!(staging.len(), 8);
    assert_eq!(staging.take(4), b"1234");
    assert!(rx.recv_timeout(Duration::from_secs(3)).unwrap());
    assert_eq!(staging.len(), 8);
    assert_eq!(staging.take(3), b"567");
    assert_eq!(staging.take(99), b"890ab");
    assert!(!staging.wait_for_data());
    worker.join().unwrap();
}

#[test]
fn parser_shutdown_wakes_backpressured_reader() {
    let staging = Arc::new(Staging::new(1));
    assert!(staging.push(b"a"));
    let producer = staging.clone();
    let worker = thread::spawn(move || producer.push(b"b"));
    staging.finish_parser();
    assert!(!worker.join().unwrap());
    assert_eq!(staging.take(5), b"a");
}

#[test]
fn eof_wakes_parser_and_drains_final_bytes() {
    let staging = Staging::new(8);
    staging.push(b"last");
    staging.finish_reader();
    assert!(staging.wait_for_data());
    assert_eq!(staging.take(8), b"last");
    assert!(!staging.wait_for_data());
}

#[test]
fn large_stream_is_lossless_and_batches_are_bounded() {
    let staging = Arc::new(Staging::new(16 * 1024));
    let expected: Vec<_> = (0..300_000).map(|i| (i % 251) as u8).collect();
    let input = expected.clone();
    let producer = staging.clone();
    let worker = thread::spawn(move || { assert!(producer.push(&input)); producer.finish_reader(); });
    let mut result = Vec::new();
    while staging.wait_for_data() {
        assert!(staging.len() <= 16 * 1024);
        let batch = staging.take(1024);
        assert!(batch.len() <= 1024);
        result.extend_from_slice(&batch);
    }
    worker.join().unwrap();
    assert_eq!(result, expected);
}
