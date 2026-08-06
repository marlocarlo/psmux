// Regression tests for the bounded-notification-line fix (b85ecad).
//
// Draining a full pane output ring used to produce ONE %output line whose
// escaped payload could reach ~256KB. tmux never does this: every PTY read
// is bounded, so clients see a stream of moderate lines. `chunk_output`
// splits drained output at 4096 bytes on UTF-8 boundaries so every chunk is
// independently escapable and no single line explodes.
//
// `chunk_output` is a pure function; `format_notification` turns each chunk
// into the actual wire line. Both are driven directly here — no server.

use super::*;

const MAX: usize = crate::control::OUTPUT_CHUNK_MAX;

/// A short payload is passed through as one chunk; an empty payload yields
/// no chunks at all.
#[test]
fn short_payload_is_a_single_chunk() {
    assert_eq!(chunk_output("hello").collect::<Vec<_>>(), vec!["hello"]);
    assert_eq!(chunk_output("").count(), 0);
}

/// An oversized payload is split into multiple chunks, each bounded by
/// OUTPUT_CHUNK_MAX, and the concatenation reproduces the input exactly.
#[test]
fn oversized_payload_splits_into_bounded_chunks() {
    let data = "x".repeat(MAX * 2 + 100);
    let chunks: Vec<&str> = chunk_output(&data).collect();

    assert_eq!(chunks.len(), 3, "expected 3 chunks for 2*MAX+100 bytes");
    assert!(
        chunks.iter().all(|c| !c.is_empty() && c.len() <= MAX),
        "every chunk must be non-empty and bounded by OUTPUT_CHUNK_MAX"
    );
    assert_eq!(chunks.concat(), data, "chunks must reassemble the payload");
}

/// Splitting must never cut through a multi-byte UTF-8 sequence: every
/// chunk must itself be valid UTF-8, even when a CJK glyph straddles the
/// chunk boundary.
#[test]
fn chunking_never_splits_a_utf8_sequence() {
    // Fill the first chunk so the 3-byte CJK glyph straddles the boundary.
    let mut data = "a".repeat(MAX - 1);
    data.push_str("终端输出");
    data.push_str(&"b".repeat(MAX));

    let chunks: Vec<&str> = chunk_output(&data).collect();
    assert!(chunks.len() >= 3, "straddling glyph must still force a split");
    for chunk in &chunks {
        assert!(
            std::str::from_utf8(chunk.as_bytes()).is_ok(),
            "chunk must not contain a partial UTF-8 sequence"
        );
        assert!(!chunk.is_empty() && chunk.len() <= MAX);
    }
    assert_eq!(chunks.concat(), data);
}

/// A payload that is exactly one chunk long is not split.
#[test]
fn exact_chunk_size_is_a_single_chunk() {
    let data = "x".repeat(MAX);
    let chunks: Vec<&str> = chunk_output(&data).collect();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], data);
}

/// The wire-level contract: each chunk becomes its own %output line, the
/// escaped payload of every line is bounded, and the notification stream
/// carries the same bytes the single oversized frame would have.
#[test]
fn chunked_output_formats_as_multiple_bounded_output_lines() {
    let data = "y".repeat(MAX + 100); // printable ASCII escapes 1:1
    let chunks: Vec<&str> = chunk_output(&data).collect();
    assert!(chunks.len() >= 2);

    let mut reassembled = String::new();
    for chunk in chunks {
        let line = format_notification(&ControlNotification::Output {
            pane_id: 7,
            data: chunk.to_string(),
        });
        assert!(
            line.starts_with("%output %7 "),
            "each chunk must be its own %output line: {line:?}"
        );
        assert!(
            line.len() <= "%output %7 ".len() + MAX,
            "a single %output line must stay bounded by OUTPUT_CHUNK_MAX payload bytes"
        );
        reassembled.push_str(chunk);
    }
    assert_eq!(reassembled, data, "the chunk stream must carry the full payload");
}

/// The same boundedness applies to %extended-output frames (pause-after
/// mode), which carry an age field before the payload.
#[test]
fn extended_output_chunks_keep_the_age_prefix() {
    let data = "z".repeat(MAX + 1);
    let chunks: Vec<&str> = chunk_output(&data).collect();
    for chunk in chunks {
        let line = format_notification(&ControlNotification::ExtendedOutput {
            pane_id: 3,
            age_ms: 42,
            data: chunk.to_string(),
        });
        assert!(line.starts_with("%extended-output %3 42 : "));
        assert!(line.len() <= "%extended-output %3 42 : ".len() + MAX);
    }
}

/// Chunking is deterministic and allocation-free on the string itself:
/// iterating twice yields identical splits.
#[test]
fn chunking_is_deterministic() {
    let mut data = "a".repeat(MAX - 2);
    data.push_str("你你你");
    let first: Vec<&str> = chunk_output(&data).collect();
    let second: Vec<&str> = chunk_output(&data).collect();
    assert_eq!(first, second);
}

/// A payload of exactly N chunk sizes splits into exactly N chunks.
#[test]
fn chunk_count_for_moderate_payloads_is_sane() {
    let data = "x".repeat(MAX * 10);
    let chunks: Vec<&str> = chunk_output(&data).collect();
    assert_eq!(chunks.len(), 10);
}
