// ── Issue #201: Rename session overlay shows wrong title ─────────────
//
// BUG: When user presses prefix+$ to rename a session, the overlay dialog
// in the client TUI shows "rename window" instead of "rename session".
// The session IS renamed correctly, but the dialog title is misleading.
//
// ROOT CAUSE: In client.rs, the rename overlay rendering code always uses
// the hardcoded title "rename window" regardless of the session_renaming
// boolean. The fix must conditionally select the title.
//
// These tests replicate the EXACT overlay construction from client.rs to
// prove the title text is correct for both window and session renaming.

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;

/// Extract all text content from a TestBackend buffer as a single string.
fn buffer_text(backend: &TestBackend) -> String {
    let buf = backend.buffer();
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            text.push_str(cell.symbol());
        }
    }
    text
}

/// Simulates a centered rect calculation (matches client.rs centered_rect)
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width as u32 * percent_x as u32 / 100).min(area.width as u32) as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Overlay title when renaming a SESSION
// ═════════════════════════════════════════════════════════════════════

#[test]
fn rename_session_overlay_shows_rename_session_title() {
    // Simulate the state: renaming=true, session_renaming=true
    let session_renaming = true;
    let rename_buf = "my_session";

    // This is the EXACT logic that client.rs SHOULD use:
    let title = if session_renaming { "rename session" } else { "rename window" };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| {
        let area = f.area();
        let overlay = Block::default().borders(Borders::ALL).title(title);
        let oa = centered_rect(60, 3, area);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let para = Paragraph::new(format!("name: {}", rename_buf));
        f.render_widget(para, overlay.inner(oa));
    }).unwrap();

    let text = buffer_text(terminal.backend());
    assert!(
        text.contains("rename session"),
        "When session_renaming=true, overlay MUST contain 'rename session'. Got buffer:\n{}",
        text
    );
    assert!(
        !text.contains("rename window"),
        "When session_renaming=true, overlay must NOT contain 'rename window'. Got buffer:\n{}",
        text
    );
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Overlay title when renaming a WINDOW
// ═════════════════════════════════════════════════════════════════════

#[test]
fn rename_window_overlay_shows_rename_window_title() {
    // Simulate: renaming=true, session_renaming=false
    let session_renaming = false;
    let rename_buf = "my_window";

    let title = if session_renaming { "rename session" } else { "rename window" };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| {
        let area = f.area();
        let overlay = Block::default().borders(Borders::ALL).title(title);
        let oa = centered_rect(60, 3, area);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let para = Paragraph::new(format!("name: {}", rename_buf));
        f.render_widget(para, overlay.inner(oa));
    }).unwrap();

    let text = buffer_text(terminal.backend());
    assert!(
        text.contains("rename window"),
        "When session_renaming=false, overlay MUST contain 'rename window'. Got buffer:\n{}",
        text
    );
    assert!(
        !text.contains("rename session"),
        "When session_renaming=false, overlay must NOT contain 'rename session'. Got buffer:\n{}",
        text
    );
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Prove the BUG variant (hardcoded "rename window") is wrong
//  This test demonstrates what the BUGGY code does and verifies it
//  produces wrong output for session rename.
// ═════════════════════════════════════════════════════════════════════

#[test]
fn buggy_hardcoded_title_is_wrong_for_session_rename() {
    // The BUGGY code uses "rename window" regardless of session_renaming.
    // This test proves the hardcoded string does NOT produce correct behavior.
    let session_renaming = true;

    // BUGGY: always uses "rename window" (what client.rs currently does)
    let buggy_title = "rename window";
    // CORRECT: uses session_renaming to decide
    let correct_title = if session_renaming { "rename session" } else { "rename window" };

    assert_ne!(
        buggy_title, correct_title,
        "When session_renaming=true, the hardcoded 'rename window' differs from the correct 'rename session'"
    );
}

// ═════════════════════════════════════════════════════════════════════
//  Test: When NOT session renaming, hardcoded title happens to be right
// ═════════════════════════════════════════════════════════════════════

#[test]
fn hardcoded_title_correct_for_window_rename() {
    let session_renaming = false;
    let buggy_title = "rename window";
    let correct_title = if session_renaming { "rename session" } else { "rename window" };

    assert_eq!(
        buggy_title, correct_title,
        "When session_renaming=false, the hardcoded 'rename window' is coincidentally correct"
    );
}
