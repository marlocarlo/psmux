#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[test]
fn ime_detection_ascii_only() {
    // Pure ASCII text should NOT be detected as IME input
    assert!(!paste_buffer_has_non_ascii("abc"));
    assert!(!paste_buffer_has_non_ascii("hello world"));
    assert!(!paste_buffer_has_non_ascii("12345"));
    assert!(!paste_buffer_has_non_ascii(""));
}

#[cfg(windows)]
#[test]
fn ime_detection_japanese() {
    // Japanese IME input should be detected as non-ASCII
    assert!(paste_buffer_has_non_ascii("日本語"));
    assert!(paste_buffer_has_non_ascii("にほんご"));
    assert!(paste_buffer_has_non_ascii("abc日本語"));
}

#[cfg(windows)]
#[test]
fn ime_detection_chinese() {
    assert!(paste_buffer_has_non_ascii("中文"));
    assert!(paste_buffer_has_non_ascii("你好世界"));
}

#[cfg(windows)]
#[test]
fn ime_detection_korean() {
    assert!(paste_buffer_has_non_ascii("한국어"));
}

#[cfg(windows)]
#[test]
fn ime_detection_mixed() {
    // Mixed ASCII + CJK should be detected as non-ASCII
    assert!(paste_buffer_has_non_ascii("hello世界"));
    assert!(paste_buffer_has_non_ascii("a日b"));
}

#[cfg(windows)]
#[test]
fn flush_paste_pend_ascii_sends_as_paste() {
    // ASCII buffer with ≥3 chars should send as send-paste (paste detection intact)
    let mut buf = String::from("abcdef");
    let mut start: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stage2 = true;
    let mut cmds: Vec<String> = Vec::new();
    flush_paste_pend_as_text(&mut buf, &mut start, &mut stage2, &mut cmds);
    assert_eq!(cmds.len(), 1);
    assert!(cmds[0].starts_with("send-paste "));
}

#[cfg(windows)]
#[test]
fn flush_paste_pend_cjk_sends_as_text() {
    // Non-ASCII buffer should NEVER send as send-paste, even with ≥3 chars.
    // This is the core fix for issue #91.
    let mut buf = String::from("日本語テスト");
    let mut start: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stage2 = false;
    let mut cmds: Vec<String> = Vec::new();
    flush_paste_pend_as_text(&mut buf, &mut start, &mut stage2, &mut cmds);
    // Each character should be sent as individual send-text
    assert!(cmds.len() > 1, "CJK should be sent as individual send-text commands");
    for cmd in &cmds {
        assert!(cmd.starts_with("send-text "), "CJK char should be send-text, got: {}", cmd);
    }
}

#[cfg(windows)]
#[test]
fn flush_paste_pend_short_ascii_sends_as_text() {
    // <3 ASCII chars should be sent as individual keystrokes
    let mut buf = String::from("ab");
    let mut start: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stage2 = false;
    let mut cmds: Vec<String> = Vec::new();
    flush_paste_pend_as_text(&mut buf, &mut start, &mut stage2, &mut cmds);
    assert_eq!(cmds.len(), 2);
    assert!(cmds[0].starts_with("send-text "));
    assert!(cmds[1].starts_with("send-text "));
}
