// ── Issue #155: Strikethrough and Hidden SGR attributes ──────────────
//
// Verifies that SGR 8 (hidden), SGR 9 (strikethrough), and their reset
// codes (28, 29) are correctly parsed, stored on cells, included in
// escape-code diff generation, and preserved through contents_formatted().

// ── Parser → Cell attribute tests ──────────────────────────────────

#[test]
fn sgr9_sets_strikethrough_on_cell() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    // ESC[9m = strikethrough on, then write "abc"
    parser.process(b"\x1b[9mabc");
    let screen = parser.screen();
    let cell = screen.cell(0, 0).unwrap();
    assert!(cell.strikethrough(), "cell(0,0) should have strikethrough after SGR 9");
    assert!(screen.cell(0, 1).unwrap().strikethrough());
    assert!(screen.cell(0, 2).unwrap().strikethrough());
}

#[test]
fn sgr29_clears_strikethrough_on_cell() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    // SGR 9 on, write "ab", SGR 29 off, write "cd"
    parser.process(b"\x1b[9mab\x1b[29mcd");
    let screen = parser.screen();
    assert!(screen.cell(0, 0).unwrap().strikethrough());
    assert!(screen.cell(0, 1).unwrap().strikethrough());
    assert!(!screen.cell(0, 2).unwrap().strikethrough(), "cell after SGR 29 should not have strikethrough");
    assert!(!screen.cell(0, 3).unwrap().strikethrough());
}

#[test]
fn sgr8_sets_hidden_on_cell() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[8mhidden");
    let screen = parser.screen();
    for i in 0..6 {
        assert!(screen.cell(0, i).unwrap().hidden(), "cell(0,{i}) should be hidden after SGR 8");
    }
}

#[test]
fn sgr28_clears_hidden_on_cell() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[8mab\x1b[28mcd");
    let screen = parser.screen();
    assert!(screen.cell(0, 0).unwrap().hidden());
    assert!(screen.cell(0, 1).unwrap().hidden());
    assert!(!screen.cell(0, 2).unwrap().hidden(), "cell after SGR 28 should not be hidden");
    assert!(!screen.cell(0, 3).unwrap().hidden());
}

#[test]
fn sgr0_resets_strikethrough_and_hidden() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[8;9mab\x1b[0mcd");
    let screen = parser.screen();
    assert!(screen.cell(0, 0).unwrap().hidden());
    assert!(screen.cell(0, 0).unwrap().strikethrough());
    assert!(!screen.cell(0, 2).unwrap().hidden(), "SGR 0 should clear hidden");
    assert!(!screen.cell(0, 2).unwrap().strikethrough(), "SGR 0 should clear strikethrough");
}

// ── Escape code diff / contents_formatted tests ────────────────────

#[test]
fn contents_formatted_includes_sgr9_for_strikethrough() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[9mstrike\x1b[29mnormal");
    let formatted = parser.screen().contents_formatted();
    let s = String::from_utf8_lossy(&formatted);
    // The formatted output should contain SGR 9 (strikethrough on) somewhere
    assert!(
        s.contains("\x1b[9m") || s.contains(";9m") || s.contains(";9;"),
        "contents_formatted() should emit SGR 9 for strikethrough text, got: {:?}",
        s
    );
}

#[test]
fn contents_formatted_includes_sgr29_to_clear_strikethrough() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[9mstrike\x1b[29mnormal");
    let formatted = parser.screen().contents_formatted();
    let s = String::from_utf8_lossy(&formatted);
    // After the strikethrough text, the formatted output should reset it
    // (either via SGR 29, SGR 0, or a combined sequence)
    let strike_pos = s.find("strike").unwrap();
    let after_strike = &s[strike_pos + 6..];
    assert!(
        after_strike.contains("\x1b[29m") || after_strike.contains(";29m")
            || after_strike.contains(";29;") || after_strike.contains("\x1b[0m")
            || after_strike.contains(";0m") || after_strike.contains("\x1b[m"),
        "contents_formatted() should reset strikethrough after struck text, remainder: {:?}",
        after_strike
    );
}

#[test]
fn contents_formatted_includes_sgr8_for_hidden() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[8msecret\x1b[28mvisible");
    let formatted = parser.screen().contents_formatted();
    let s = String::from_utf8_lossy(&formatted);
    assert!(
        s.contains("\x1b[8m") || s.contains(";8m") || s.contains(";8;"),
        "contents_formatted() should emit SGR 8 for hidden text, got: {:?}",
        s
    );
}

#[test]
fn combined_strikethrough_hidden_bold_roundtrip() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[1;8;9mcombined\x1b[0mplain");
    let screen = parser.screen();
    let cell = screen.cell(0, 0).unwrap();
    assert!(cell.bold());
    assert!(cell.hidden());
    assert!(cell.strikethrough());
    let plain_cell = screen.cell(0, 8).unwrap();
    assert!(!plain_cell.bold());
    assert!(!plain_cell.hidden());
    assert!(!plain_cell.strikethrough());

    // Verify formatted output roundtrips: parse the formatted output
    // into a second parser and verify cell attributes match
    let formatted = screen.contents_formatted();
    let mut parser2 = vt100::Parser::new(24, 80, 0);
    parser2.process(&formatted);
    let cell2 = parser2.screen().cell(0, 0).unwrap();
    assert!(cell2.bold(), "bold should survive roundtrip");
    assert!(cell2.hidden(), "hidden should survive roundtrip");
    assert!(cell2.strikethrough(), "strikethrough should survive roundtrip");
}
