use super::*;
use crossterm::event::Event;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Feed a string into the VtParser char by char and collect all emitted events.
fn feed_str(parser: &mut VtParser, s: &str) -> Vec<Event> {
    let mut events = Vec::new();
    for ch in s.chars() {
        parser.feed(ch, &mut |evt| events.push(evt));
    }
    events
}

/// Convenience: create a fresh parser, feed a string, return events.
fn parse(s: &str) -> Vec<Event> {
    let mut p = VtParser::new();
    feed_str(&mut p, s)
}

/// Extract the text from an Event::Paste variant.
fn paste_text(evt: &Event) -> Option<&str> {
    match evt {
        Event::Paste(t) => Some(t.as_str()),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. NORMAL PASTE (complete bracket sequences through VT parser)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn normal_paste_short_text() {
    let events = parse("\x1b[200~hello world\x1b[201~");
    let pastes: Vec<&str> = events.iter().filter_map(paste_text).collect();
    assert_eq!(pastes, vec!["hello world"]);
}

#[test]
fn normal_paste_multiline() {
    let payload = "line1\rline2\rline3";
    let seq = format!("\x1b[200~{}\x1b[201~", payload);
    let events = parse(&seq);
    let pastes: Vec<&str> = events.iter().filter_map(paste_text).collect();
    assert_eq!(pastes, vec![payload]);
}

#[test]
fn normal_paste_with_indentation() {
    let payload = "def foo():\r    return 42\r";
    let seq = format!("\x1b[200~{}\x1b[201~", payload);
    let events = parse(&seq);
    let text = paste_text(events.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap();
    assert_eq!(text, payload);
    assert!(text.contains("    return"));
}

#[test]
fn normal_paste_containing_esc_not_close() {
    // ESC inside paste followed by non-[ should be captured in paste text
    let seq = "\x1b[200~before\x1bxafter\x1b[201~";
    let events = parse(seq);
    let text = paste_text(events.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap();
    assert!(text.contains("before"));
    assert!(text.contains("\x1bx"));
    assert!(text.contains("after"));
}

#[test]
fn normal_paste_containing_esc_bracket_not_201() {
    // \x1b[100~ inside paste should not end it
    let seq = "\x1b[200~before\x1b[100~after\x1b[201~";
    let events = parse(seq);
    let text = paste_text(events.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap();
    assert!(text.contains("before"));
    assert!(text.contains("\x1b[100~"));  // partial CSI absorbed into paste
    assert!(text.contains("after"));
}

#[test]
fn consecutive_pastes() {
    let mut p = VtParser::new();
    let e1 = feed_str(&mut p, "\x1b[200~first\x1b[201~");
    let e2 = feed_str(&mut p, "\x1b[200~second\x1b[201~");

    assert_eq!(paste_text(e1.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap(), "first");
    assert_eq!(paste_text(e2.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap(), "second");
    assert_eq!(p.state, PS::Ground);
}

#[test]
fn normal_key_between_pastes() {
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~first\x1b[201~");
    assert_eq!(p.state, PS::Ground);
    // Normal typing
    let key_events = feed_str(&mut p, "abc");
    assert_eq!(key_events.len(), 3);
    // All should be Key events
    for e in &key_events {
        assert!(matches!(e, Event::Key(_)), "expected Key, got {:?}", e);
    }
    // Another paste
    let e3 = feed_str(&mut p, "\x1b[200~third\x1b[201~");
    assert_eq!(paste_text(e3.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap(), "third");
}

#[test]
fn large_paste() {
    let mut payload = String::new();
    for i in 0..500 {
        let indent = " ".repeat(i % 8);
        payload.push_str(&format!("{}line {}\r", indent, i));
    }
    let seq = format!("\x1b[200~{}\x1b[201~", payload);
    let events = parse(&seq);
    let text = paste_text(events.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap();
    assert_eq!(text, payload);
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. ISSUE #197: CLOSE SEQUENCE LOST (timeout flush scenarios)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn timeout_flush_emits_paste_and_enters_paste_drain() {
    // Simulate: open sequence arrives, content arrives, close sequence NEVER comes.
    // After 2s timeout, flush_stale_paste() should emit Event::Paste and
    // transition to PasteDrain.
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~hello timeout");
    assert_eq!(p.state, PS::Paste);
    assert!(p.paste_start.is_some());

    // Manipulate timestamp to simulate 3 seconds elapsed
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));

    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));

    assert_eq!(events.len(), 1);
    assert_eq!(paste_text(&events[0]).unwrap(), "hello timeout");
    assert_eq!(p.state, PS::PasteDrain, "should be in PasteDrain after timeout flush");
    // paste_start is reused as drain deadline (set to now by flush_stale_paste)
    assert!(p.paste_start.is_some(), "paste_start should be set as drain deadline");
}

#[test]
fn paste_drain_absorbs_tilde() {
    // THE CORE BUG: After a timeout flush, a trailing '~' from the stripped
    // close sequence (\x1b[201~) should be ABSORBED, not emitted as a key.
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~test data");
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));
    assert_eq!(p.state, PS::PasteDrain);

    // The '~' leaking from ConPTY stripping the close sequence
    let tilde_events = feed_str(&mut p, "~");
    // MUST be empty: '~' should be silently consumed
    assert!(tilde_events.is_empty(),
        "tilde after paste timeout flush MUST be absorbed, but got {:?}", tilde_events);
}

#[test]
fn paste_drain_absorbs_bracket_and_digits() {
    // ConPTY might strip only \x1b from the close sequence \x1b[201~,
    // leaving [201~ to leak through
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~content");
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));
    assert_eq!(p.state, PS::PasteDrain);

    let residue_events = feed_str(&mut p, "[201~");
    // All should be absorbed
    assert!(residue_events.is_empty(),
        "[201~ residue should be absorbed in PasteDrain, got {:?}", residue_events);
}

#[test]
fn paste_drain_passes_normal_char_through() {
    // After the drain period, a normal character should be forwarded normally
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~data");
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));
    assert_eq!(p.state, PS::PasteDrain);

    // Absorb residue
    let _ = feed_str(&mut p, "~");

    // Next real character
    let normal_events = feed_str(&mut p, "a");
    assert_eq!(normal_events.len(), 1);
    assert!(matches!(normal_events[0], Event::Key(_)),
        "normal char after drain should be a Key event, got {:?}", normal_events[0]);
    assert_eq!(p.state, PS::Ground);
}

#[test]
fn paste_drain_esc_transitions_to_escape_state() {
    // ESC arriving during drain could be the start of a new sequence
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~data");
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));
    assert_eq!(p.state, PS::PasteDrain);

    // ESC in drain
    let esc_events = feed_str(&mut p, "\x1b");
    assert!(esc_events.is_empty(), "ESC during drain should not emit anything yet");
    assert_eq!(p.state, PS::Escape, "should transition to Escape on ESC");
}

#[test]
fn timeout_flush_from_paste_esc_state() {
    // Parser in PasteEsc state when timeout fires (received \x1b inside paste but no [)
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~some text\x1b");
    assert_eq!(p.state, PS::PasteEsc);

    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));

    assert_eq!(events.len(), 1);
    assert_eq!(paste_text(&events[0]).unwrap(), "some text");
    // After PasteEsc flush, should be in Escape state (to process the [ that might follow)
    assert_eq!(p.state, PS::Escape,
        "PasteEsc timeout should transition to Escape, not {:?}", p.state);
}

#[test]
fn timeout_flush_from_paste_brk_state() {
    // Parser in PasteBrk state: received \x1b[ while in paste
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~text\x1b[");
    assert_eq!(p.state, PS::PasteBrk);

    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));

    assert_eq!(events.len(), 1);
    assert_eq!(paste_text(&events[0]).unwrap(), "text");
    // After PasteBrk flush, should be in CsiEntry (to process remaining digits + ~)
    assert_eq!(p.state, PS::CsiEntry,
        "PasteBrk timeout should transition to CsiEntry, not {:?}", p.state);
}

#[test]
fn timeout_flush_from_paste_num_state() {
    // Parser in PasteNum state: received \x1b[20 while in paste (partial close)
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~text\x1b[20");
    assert_eq!(p.state, PS::PasteNum);

    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));

    assert_eq!(events.len(), 1);
    assert_eq!(paste_text(&events[0]).unwrap(), "text");
    // After PasteNum flush, should be in CsiParam (to process remaining 1~)
    assert_eq!(p.state, PS::CsiParam,
        "PasteNum timeout should transition to CsiParam, not {:?}", p.state);
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. ISSUE #197: ConPTY delivering ESC as VK_ESCAPE (u_char=0)
//    When the parser is in Paste state and a VK_ESCAPE arrives, the reader
//    thread feeds '\x1b' to the parser.  Test that this works.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn vk_escape_in_paste_feeds_esc_to_parser() {
    // Simulate: open sequence + content arrive as u_char data,
    // then ESC of close sequence arrives as VK_ESCAPE (u_char=0)
    let mut p = VtParser::new();
    // Open sequence + content
    let _ = feed_str(&mut p, "\x1b[200~pasted text");
    assert_eq!(p.state, PS::Paste);
    assert!(p.is_in_paste());

    // VK_ESCAPE would be fed as '\x1b' by reader thread
    let esc_events = feed_str(&mut p, "\x1b");
    assert!(esc_events.is_empty());
    assert_eq!(p.state, PS::PasteEsc);

    // Then [201~ follows as u_char data
    let close_events = feed_str(&mut p, "[201~");
    let pastes: Vec<&str> = close_events.iter().filter_map(paste_text).collect();
    assert_eq!(pastes, vec!["pasted text"], "paste should complete after VK_ESCAPE + [201~");
    assert_eq!(p.state, PS::Ground);
}

#[test]
fn vk_escape_then_close_sequence_completes_paste() {
    // Full scenario: content, VK_ESCAPE feeds \x1b, then [201~ arrives
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~hello");
    assert!(p.is_in_paste());

    // Simulate VK_ESCAPE → feed \x1b
    feed_str(&mut p, "\x1b");
    assert_eq!(p.state, PS::PasteEsc);

    // Then the rest of the close sequence
    let events = feed_str(&mut p, "[201~");
    assert_eq!(paste_text(&events.last().unwrap()).unwrap(), "hello");
    assert_eq!(p.state, PS::Ground);

    // Verify no tilde leaks — next char should be normal
    let next = feed_str(&mut p, "x");
    assert_eq!(next.len(), 1);
    assert!(matches!(next[0], Event::Key(_)));
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. ISSUE #197: Trailing '~' leak after ConPTY strips close sequence
//    ConPTY may strip \x1b[201 from close and only leave '~'.
//    After timeout flush + PasteDrain, '~' MUST NOT appear as visible text.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn full_scenario_conpty_strips_close_only_tilde_remains() {
    // Exact reproduction of issue #197:
    // 1. \x1b[200~ opens paste (as u_char data)
    // 2. paste content arrives
    // 3. close sequence \x1b[201~ gets stripped by ConPTY except the '~'
    // 4. Parser times out in Paste state
    // 5. After flush, '~' arrives
    // 6. '~' must NOT leak as visible text

    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~copied text from clipboard");
    assert_eq!(p.state, PS::Paste);

    // Simulate 2+ seconds passing (close sequence never arrived)
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut flush_events = Vec::new();
    p.flush_stale_paste(&mut |evt| flush_events.push(evt));

    assert_eq!(flush_events.len(), 1);
    assert_eq!(paste_text(&flush_events[0]).unwrap(), "copied text from clipboard");
    assert_eq!(p.state, PS::PasteDrain);

    // The '~' that ConPTY leaked
    let tilde_events = feed_str(&mut p, "~");
    assert!(tilde_events.is_empty(),
        "ISSUE #197 REGRESSION: tilde leaked as visible character! Got {:?}", tilde_events);

    // Verify parser returns to normal ground state after drain
    // (either by normal char or by flush_escape timeout)
    let normal = feed_str(&mut p, "a");
    assert_eq!(normal.len(), 1);
    assert!(matches!(normal[0], Event::Key(_)));
    assert_eq!(p.state, PS::Ground);
}

#[test]
fn full_scenario_conpty_strips_esc_bracket_leaves_201_tilde() {
    // ConPTY strips \x1b[ but leaves 201~ to leak through
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~content");
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut flush_events = Vec::new();
    p.flush_stale_paste(&mut |evt| flush_events.push(evt));
    assert_eq!(p.state, PS::PasteDrain);

    // Residue: 201~
    let residue = feed_str(&mut p, "201~");
    assert!(residue.is_empty(),
        "201~ residue after paste flush should be absorbed, got {:?}", residue);

    // Normal operation resumes
    let normal = feed_str(&mut p, "x");
    assert_eq!(normal.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. PasteDrain timeout (flush_escape clears PasteDrain to Ground)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn paste_drain_expires_on_flush_escape() {
    // PasteDrain should NOT expire immediately on flush_escape — it has a
    // 2000ms window.  Only after the deadline has passed should it transition
    // to Ground.
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~data");
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));
    assert_eq!(p.state, PS::PasteDrain);

    // Immediately after flush, PasteDrain should still be active (drain
    // deadline was just set to now).
    let mut timeout_events = Vec::new();
    p.flush_escape(&mut |evt| timeout_events.push(evt));
    assert_eq!(p.state, PS::PasteDrain,
        "PasteDrain should NOT expire immediately — 2000ms window not elapsed");
    assert!(timeout_events.is_empty());

    // After the 2000ms deadline expires, flush_escape should transition to Ground.
    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_millis(2100));
    let mut expired_events = Vec::new();
    p.flush_escape(&mut |evt| expired_events.push(evt));
    assert_eq!(p.state, PS::Ground, "PasteDrain should expire after 2000ms window");
    assert!(expired_events.is_empty(), "no events should be emitted on drain expiry");
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Edge cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn empty_paste() {
    let events = parse("\x1b[200~\x1b[201~");
    // Empty paste should produce Event::Paste("")
    let pastes: Vec<&str> = events.iter().filter_map(paste_text).collect();
    assert_eq!(pastes, vec![""]);
}

#[test]
fn paste_with_only_escs() {
    // Multiple ESCs inside paste, none starting a close sequence
    let events = parse("\x1b[200~\x1ba\x1bb\x1bc\x1b[201~");
    let text = paste_text(events.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap();
    assert_eq!(text, "\x1ba\x1bb\x1bc");
}

#[test]
fn paste_then_immediately_another_paste() {
    // No gap between close and next open
    let events = parse("\x1b[200~first\x1b[201~\x1b[200~second\x1b[201~");
    let pastes: Vec<&str> = events.iter().filter_map(paste_text).collect();
    assert_eq!(pastes, vec!["first", "second"]);
}

#[test]
fn paste_state_tracked_by_is_in_paste() {
    let mut p = VtParser::new();
    assert!(!p.is_in_paste());

    let _ = feed_str(&mut p, "\x1b[200~");
    assert!(p.is_in_paste());

    let _ = feed_str(&mut p, "text");
    assert!(p.is_in_paste());

    let _ = feed_str(&mut p, "\x1b[201~");
    assert!(!p.is_in_paste());
}

#[test]
fn needs_vti_recheck_set_on_paste_start() {
    let mut p = VtParser::new();
    assert!(!p.needs_vti_recheck);

    let _ = feed_str(&mut p, "\x1b[200~");
    assert!(p.needs_vti_recheck, "needs_vti_recheck should be set when paste starts");

    // Simulate reader thread resetting it
    p.needs_vti_recheck = false;
    let _ = feed_str(&mut p, "text\x1b[201~");
    assert!(!p.needs_vti_recheck, "should not re-set on close");
}

#[test]
fn paste_preserves_exact_content_including_special_chars() {
    let payload = "Hello\ttab\rCR\nLF\x00null\r\nCRLF  spaces   end";
    let seq = format!("\x1b[200~{}\x1b[201~", payload);
    let events = parse(&seq);
    let text = paste_text(events.iter().find(|e| matches!(e, Event::Paste(_))).unwrap()).unwrap();
    assert_eq!(text, payload);
}

#[test]
fn dispatch_tilde_ignores_param_201() {
    // After a PasteNum timeout and transition to CsiParam,
    // if "1~" arrives (completing CSI 201 ~), dispatch_tilde should
    // IGNORE it (param 201 is not a valid function key)
    let mut p = VtParser::new();
    let _ = feed_str(&mut p, "\x1b[200~text\x1b[20");
    assert_eq!(p.state, PS::PasteNum);

    p.paste_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    let mut events = Vec::new();
    p.flush_stale_paste(&mut |evt| events.push(evt));
    assert_eq!(p.state, PS::CsiParam);

    // Feed "1~" to complete CSI 201 ~
    let csi_events = feed_str(&mut p, "1~");
    // dispatch_tilde with param 201 should return without emitting
    assert!(csi_events.is_empty(),
        "CSI 201~ should be silently ignored, got {:?}", csi_events);
    assert_eq!(p.state, PS::Ground);
}
