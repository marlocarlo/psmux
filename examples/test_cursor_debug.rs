use vt100::Parser;

fn main() {
    // Test: what happens when we process "\x1b[?25l" (hide cursor) after RMCUP
    let mut p = Parser::new(24, 80, 0);
    
    // Simulate shell prompt
    p.process(b"PS C:\\Users\\test> ");
    let (r, c) = p.screen().cursor_position();
    println!("Prompt cursor: row={} col={} hide={}", r, c, p.screen().hide_cursor());
    
    // TUI enters alt screen
    p.process(b"\x1b[?1049h");
    // TUI hides cursor (many TUI apps do this)
    p.process(b"\x1b[?25l");
    println!("In alt after hide: hide={} alt={}", p.screen().hide_cursor(), p.screen().alternate_screen());
    
    // TUI draws
    p.process(b"\x1b[1;1HTUI CONTENT");
    
    // TUI sends RMCUP
    p.process(b"\x1b[?1049l");
    let (r, c) = p.screen().cursor_position();
    println!("After RMCUP: row={} col={} hide={} alt={}", r, c, p.screen().hide_cursor(), p.screen().alternate_screen());
    
    // Apply FULL_MODE_RESET (includes ?25h)
    p.process(b"\x1b[0m\x1b[?25h\x1b[?1l\x1b[?9l\x1b[?47l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?2004l");
    let (r, c) = p.screen().cursor_position();
    println!("After FULL_MODE_RESET: row={} col={} hide={}", r, c, p.screen().hide_cursor());
    
    // Simulate what ConPTY might send after TUI exit
    // ConPTY might send cursor position queries, screen updates, etc.
    // Some apps send hide cursor just before RMCUP
    
    // Test: what if post-mortem data includes hide cursor?
    p.process(b"\x1b[?25l");
    println!("After post-mortem hide: hide={}", p.screen().hide_cursor());
    
    // FULL_MODE_RESET again
    p.process(b"\x1b[0m\x1b[?25h\x1b[?1l\x1b[?9l\x1b[?47l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?2004l");
    println!("After 2nd FULL_MODE_RESET: hide={}", p.screen().hide_cursor());
    
    // Test alternate screen save/restore of hide_cursor state
    let mut p2 = Parser::new(24, 80, 0);
    p2.process(b"before");  // normal screen, cursor visible
    println!("\np2: hide={}", p2.screen().hide_cursor());
    p2.process(b"\x1b[?25l");  // hide on normal screen
    println!("p2 after hide on normal: hide={}", p2.screen().hide_cursor());
    p2.process(b"\x1b[?1049h");  // switch to alt - does it save hide state?
    println!("p2 in alt: hide={}", p2.screen().hide_cursor());
    p2.process(b"\x1b[?25h"); // show on alt
    println!("p2 show on alt: hide={}", p2.screen().hide_cursor());
    p2.process(b"\x1b[?1049l");  // back to normal - does it restore hide state?
    println!("p2 after RMCUP: hide={}", p2.screen().hide_cursor());
}
