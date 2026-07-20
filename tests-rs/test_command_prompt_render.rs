// The ':' command prompt renders on the STATUS LINE (tmux parity), not as a
// centered box — deterministic render proof.
//
// Drives the real client `render_command_prompt` through a headless TestBackend
// and asserts on cells. Ground truth for a client-rendered overlay: capture-pane
// cannot see it, and dump-state carries no prompt state at all.

use crate::client::{render_command_prompt, CommandPromptView};
use ratatui::layout::Rect;

const W: u16 = 80;
const H: u16 = 24;

struct Rendered {
    buf: ratatui::buffer::Buffer,
    cursor: Option<(u16, u16)>,
    view_off: usize,
    completion_scroll: usize,
}

/// Render with a status bar of `status_lines` rows at top or bottom, laid out
/// exactly as `run_remote` does.
fn render(
    buf_text: &str,
    cursor: usize,
    label: Option<&str>,
    completions: &[String],
    completion_sel: usize,
    status_lines: usize,
    status_at_top: bool,
    mut view_off: usize,
    mut completion_scroll: usize,
) -> Rendered {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let sl = status_lines as u16;
    let (content_chunk, status_chunk) = if status_at_top {
        (Rect::new(0, sl, W, H - sl), Rect::new(0, 0, W, sl))
    } else {
        (Rect::new(0, 0, W, H - sl), Rect::new(0, H - sl, W, sl))
    };
    let backend = TestBackend::new(W, H);
    let mut term = Terminal::new(backend).unwrap();
    let mut cursor_out = None;
    term.draw(|f| {
        let view = CommandPromptView {
            buf: buf_text,
            cursor,
            label,
            message_style: "",
            mode_style: "",
            completions,
            completion_sel,
        };
        cursor_out = render_command_prompt(
            f,
            content_chunk,
            status_chunk,
            status_lines,
            status_at_top,
            &view,
            &mut view_off,
            &mut completion_scroll,
        );
    })
    .unwrap();
    Rendered {
        buf: term.backend().buffer().clone(),
        cursor: cursor_out,
        view_off,
        completion_scroll,
    }
}

/// Simple bottom-status render: 1 status line, no completions.
fn render_simple(buf_text: &str, cursor: usize) -> Rendered {
    render(buf_text, cursor, None, &[], 0, 1, false, 0, 0)
}

fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    let w = buf.area.width as usize;
    (0..w)
        .map(|x| buf.content[(y as usize) * w + x].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

// ── position ────────────────────────────────────────────────────────────────

#[test]
fn prompt_renders_on_the_bottom_status_row() {
    let r = render_simple("split-window -h", 15);
    // Bottom row of an 80x24 screen, NOT a centered box.
    assert_eq!(row_text(&r.buf, H - 1), ":split-window -h");
    // And the middle of the screen is untouched — no floating overlay.
    assert_eq!(row_text(&r.buf, H / 2), "");
}

#[test]
fn prompt_renders_on_the_top_row_when_status_is_at_top() {
    let r = render("neww", 4, None, &[], 0, 1, true, 0, 0);
    assert_eq!(row_text(&r.buf, 0), ":neww");
    assert_eq!(row_text(&r.buf, H - 1), "");
}

#[test]
fn prompt_uses_the_last_status_line_when_status_is_two_rows() {
    // `set -g status 2`: line 0 keeps the window list, the prompt takes line 1.
    let r = render("neww", 4, None, &[], 0, 2, false, 0, 0);
    assert_eq!(row_text(&r.buf, H - 1), ":neww");
    assert_eq!(row_text(&r.buf, H - 2), "", "first status line is left alone");
}

#[test]
fn prompt_borrows_a_content_row_when_the_status_bar_is_hidden() {
    // `set -g status off` → status_lines == 0, so there is no status row.
    let r = render("neww", 4, None, &[], 0, 0, false, 0, 0);
    assert_eq!(row_text(&r.buf, H - 1), ":neww");
}

#[test]
fn custom_label_replaces_the_colon_prefix() {
    // command-prompt -p "name:" -I "dev"
    let r = render("dev", 3, Some("name:"), &[], 0, 1, false, 0, 0);
    assert_eq!(row_text(&r.buf, H - 1), "name:dev");
}

// ── cursor ──────────────────────────────────────────────────────────────────

#[test]
fn cursor_sits_after_the_typed_text() {
    let r = render_simple("neww", 4);
    // 1 col for ':' + 4 typed chars.
    assert_eq!(r.cursor, Some((5, H - 1)));
}

#[test]
fn cursor_tracks_a_mid_buffer_position() {
    let r = render_simple("neww", 2);
    assert_eq!(r.cursor, Some((3, H - 1)));
}

#[test]
fn cursor_is_a_display_column_not_a_byte_offset() {
    // Two double-width chars: 6 bytes, but 4 display columns (issue #345 —
    // command_cursor is a byte offset and must be converted for rendering).
    let s = "中文";
    let r = render_simple(s, s.len());
    assert_eq!(s.len(), 6, "precondition: 6 bytes");
    assert_eq!(r.cursor, Some((1 + 4, H - 1)), "cursor at 4 columns, not 6");
}

// ── horizontal scrolling ────────────────────────────────────────────────────

#[test]
fn long_input_scrolls_to_keep_the_cursor_visible() {
    // 100 chars into an 80-col row: the view must shift and the cursor must
    // stay on screen rather than running off the right edge.
    let long = "a".repeat(100);
    let r = render_simple(&long, long.len());
    assert!(r.view_off > 0, "view should have scrolled, got {}", r.view_off);
    let (cx, _) = r.cursor.unwrap();
    assert!(cx < W, "cursor must stay on screen, got {}", cx);
    // The tail of the buffer is what is visible.
    assert_eq!(row_text(&r.buf, H - 1).len(), W as usize - 1);
}

#[test]
fn short_input_does_not_scroll() {
    let r = render_simple("neww", 4);
    assert_eq!(r.view_off, 0);
}

// ── completion list ─────────────────────────────────────────────────────────

fn cands() -> Vec<String> {
    ["new-session", "new-window", "new", "neww"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn completion_list_opens_above_a_bottom_prompt() {
    let c = cands();
    let r = render("new", 3, None, &c, 0, 1, false, 0, 0);
    // Prompt still on the bottom row.
    assert_eq!(row_text(&r.buf, H - 1), ":new");
    // 4 candidates + 2 border rows = 6, sitting directly above the prompt.
    let top = H - 1 - 6;
    assert!(row_text(&r.buf, top).starts_with('┌'), "list top border above prompt");
    assert!(row_text(&r.buf, H - 2).starts_with('└'), "list bottom border abuts prompt");
    // Candidates in between.
    let body: String = (top + 1..H - 2).map(|y| row_text(&r.buf, y)).collect();
    for expected in ["new-session", "new-window", "neww"] {
        assert!(body.contains(expected), "expected {} in list, got {:?}", expected, body);
    }
}

#[test]
fn completion_list_opens_below_a_top_prompt() {
    let c = cands();
    let r = render("new", 3, None, &c, 0, 1, true, 0, 0);
    assert_eq!(row_text(&r.buf, 0), ":new");
    assert!(row_text(&r.buf, 1).starts_with('┌'), "list opens below a top prompt");
}

#[test]
fn completion_list_is_absent_when_there_are_no_candidates() {
    let r = render_simple("new", 3);
    assert_eq!(row_text(&r.buf, H - 2), "", "no list box when completions are empty");
}

#[test]
fn long_candidate_list_is_capped_and_scrolls_to_the_selection() {
    let many: Vec<String> = (0..40).map(|i| format!("cmd-{:02}", i)).collect();
    // Select an entry far down the list.
    let r = render("cmd", 3, None, &many, 25, 1, false, 0, 0);
    assert!(
        r.completion_scroll > 0,
        "scroll should follow the selection, got {}",
        r.completion_scroll
    );
    // Capped at 10 rows + 2 borders.
    let top = H - 1 - 12;
    assert!(row_text(&r.buf, top).starts_with('┌'), "list capped at 10 rows");
    let body: String = (top + 1..H - 2).map(|y| row_text(&r.buf, y)).collect();
    assert!(body.contains("cmd-25"), "selected entry must be visible, got {:?}", body);
}

// ── degenerate geometry ─────────────────────────────────────────────────────

#[test]
fn zero_height_content_and_no_status_renders_nothing() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let backend = TestBackend::new(W, H);
    let mut term = Terminal::new(backend).unwrap();
    let mut out = Some((0, 0));
    let (mut vo, mut cs) = (0, 0);
    term.draw(|f| {
        let view = CommandPromptView {
            buf: "neww",
            cursor: 4,
            label: None,
            message_style: "",
            mode_style: "",
            completions: &[],
            completion_sel: 0,
        };
        out = render_command_prompt(
            f,
            Rect::new(0, 0, W, 0),
            Rect::new(0, 0, W, 0),
            0,
            false,
            &view,
            &mut vo,
            &mut cs,
        );
    })
    .unwrap();
    assert_eq!(out, None, "no room to draw → no cursor");
}

#[test]
fn very_narrow_terminal_does_not_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    // Narrower than the label, with a completion list open.
    let backend = TestBackend::new(4, 6);
    let mut term = Terminal::new(backend).unwrap();
    let c = cands();
    let (mut vo, mut cs) = (0, 0);
    term.draw(|f| {
        let view = CommandPromptView {
            buf: "new-window",
            cursor: 10,
            label: Some("a-very-long-label:"),
            message_style: "",
            mode_style: "",
            completions: &c,
            completion_sel: 3,
        };
        let _ = render_command_prompt(
            f,
            Rect::new(0, 0, 4, 5),
            Rect::new(0, 5, 4, 1),
            1,
            false,
            &view,
            &mut vo,
            &mut cs,
        );
    })
    .unwrap();
}
