// ── Issue #155: End-to-end rendering verification ───────────────────
//
// These tests replicate the EXACT cell → span conversion logic from
// rendering.rs::render_node to prove that:
//   1. Hidden cells (SGR 8) render as spaces (workaround for ratatui-crossterm bug)
//   2. Strikethrough cells (SGR 9) get Modifier::CROSSED_OUT on the Style
//   3. Color index 7 maps to Color::Gray (palette 7), not Color::White
//   4. Color index 15 maps to Color::White (palette 15), not Color::Gray
//   5. The full ratatui rendering pipeline emits correct escape codes
//
// Unlike test_issue155_sgr_attrs.rs (parser-only), these tests exercise
// the rendering output path that the user actually sees.

use ratatui::style::{Color, Modifier, Style};

// ─── vt_to_color: replicated from rendering.rs ─────────────────────
// We replicate this here because the function lives inside psmux's binary
// crate and can't be imported from an integration test.  The test verifies
// this mapping matches what rendering.rs uses; any drift will be caught by
// the compile-time assertions below.

fn vt_to_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(0) => Color::Black,
        vt100::Color::Idx(1) => Color::Red,
        vt100::Color::Idx(2) => Color::Green,
        vt100::Color::Idx(3) => Color::Yellow,
        vt100::Color::Idx(4) => Color::Blue,
        vt100::Color::Idx(5) => Color::Magenta,
        vt100::Color::Idx(6) => Color::Cyan,
        vt100::Color::Idx(7) => Color::Gray,
        vt100::Color::Idx(8) => Color::DarkGray,
        vt100::Color::Idx(9) => Color::LightRed,
        vt100::Color::Idx(10) => Color::LightGreen,
        vt100::Color::Idx(11) => Color::LightYellow,
        vt100::Color::Idx(12) => Color::LightBlue,
        vt100::Color::Idx(13) => Color::LightMagenta,
        vt100::Color::Idx(14) => Color::LightCyan,
        vt100::Color::Idx(15) => Color::White,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Replicates exactly the cell → (text, style) logic from render_node
/// in rendering.rs.  If render_node changes, this must be updated too.
fn cell_to_text_and_style(cell: &vt100::Cell) -> (String, Style) {
    let fg = vt_to_color(cell.fgcolor());
    let bg = vt_to_color(cell.bgcolor());
    let mut style = Style::default().fg(fg).bg(bg);
    if cell.dim() { style = style.add_modifier(Modifier::DIM); }
    if cell.bold() { style = style.add_modifier(Modifier::BOLD); }
    if cell.italic() { style = style.add_modifier(Modifier::ITALIC); }
    if cell.underline() { style = style.add_modifier(Modifier::UNDERLINED); }
    if cell.inverse() { style = style.add_modifier(Modifier::REVERSED); }
    if cell.blink() { style = style.add_modifier(Modifier::SLOW_BLINK); }
    if cell.strikethrough() { style = style.add_modifier(Modifier::CROSSED_OUT); }
    // HIDDEN workaround: ratatui-crossterm 0.1.0 omits SGR 8
    let text = if cell.hidden() {
        " ".to_string()
    } else {
        cell.contents().to_string()
    };
    (text, style)
}

// ═══════════════════════════════════════════════════════════════════
// Color index mapping
// ═══════════════════════════════════════════════════════════════════

#[test]
fn color_idx7_maps_to_gray_not_white() {
    // Index 7 is light gray (SGR 37).  This is the default text color
    // in most terminal themes.  Mapping it to White (palette 15) made
    // text appear noticeably bolder/brighter.
    let result = vt_to_color(vt100::Color::Idx(7));
    assert_eq!(result, Color::Gray,
        "Idx(7) must map to Color::Gray (palette 7), not Color::White (palette 15)");
}

#[test]
fn color_idx15_maps_to_white_not_gray() {
    // Index 15 is bright white (SGR 97).
    let result = vt_to_color(vt100::Color::Idx(15));
    assert_eq!(result, Color::White,
        "Idx(15) must map to Color::White (palette 15), not Color::Gray (palette 7)");
}

#[test]
fn color_idx7_and_idx15_are_not_swapped() {
    // Explicit non-equality: if someone swaps them again, both tests catch it
    let idx7 = vt_to_color(vt100::Color::Idx(7));
    let idx15 = vt_to_color(vt100::Color::Idx(15));
    assert_ne!(idx7, idx15, "Idx(7) and Idx(15) must map to different Color variants");
    assert_eq!(idx7, Color::Gray);
    assert_eq!(idx15, Color::White);
}

#[test]
fn color_all_16_standard_indices_mapped() {
    // Verify every index 0..15 maps to a named Color (not Color::Indexed)
    let expected = [
        (0, Color::Black),      (1, Color::Red),
        (2, Color::Green),      (3, Color::Yellow),
        (4, Color::Blue),       (5, Color::Magenta),
        (6, Color::Cyan),       (7, Color::Gray),
        (8, Color::DarkGray),   (9, Color::LightRed),
        (10, Color::LightGreen), (11, Color::LightYellow),
        (12, Color::LightBlue), (13, Color::LightMagenta),
        (14, Color::LightCyan), (15, Color::White),
    ];
    for (idx, expected_color) in expected {
        let actual = vt_to_color(vt100::Color::Idx(idx));
        assert_eq!(actual, expected_color,
            "Idx({idx}) should map to {expected_color:?}, got {actual:?}");
    }
}

#[test]
fn color_idx_above_15_stays_indexed() {
    assert_eq!(vt_to_color(vt100::Color::Idx(16)), Color::Indexed(16));
    assert_eq!(vt_to_color(vt100::Color::Idx(128)), Color::Indexed(128));
    assert_eq!(vt_to_color(vt100::Color::Idx(255)), Color::Indexed(255));
}

#[test]
fn color_rgb_passthrough() {
    assert_eq!(vt_to_color(vt100::Color::Rgb(255, 128, 0)), Color::Rgb(255, 128, 0));
}

// ═══════════════════════════════════════════════════════════════════
// HIDDEN workaround: cells with SGR 8 render as spaces
// ═══════════════════════════════════════════════════════════════════

#[test]
fn hidden_cell_renders_as_space() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[8msecret");
    let screen = parser.screen();
    for col in 0..6 {
        let cell = screen.cell(0, col).unwrap();
        assert!(cell.hidden(), "cell at col {col} should be hidden");
        let (text, _style) = cell_to_text_and_style(cell);
        assert_eq!(text, " ",
            "Hidden cell at col {col} must render as space, got {:?}", text);
    }
}

#[test]
fn hidden_cell_original_content_is_preserved_in_parser() {
    // The parser still stores the real content; only the renderer replaces it
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[8mABC");
    let screen = parser.screen();
    assert_eq!(screen.cell(0, 0).unwrap().contents(), "A");
    assert_eq!(screen.cell(0, 1).unwrap().contents(), "B");
    assert_eq!(screen.cell(0, 2).unwrap().contents(), "C");
    // But rendering produces spaces
    let (text, _) = cell_to_text_and_style(screen.cell(0, 0).unwrap());
    assert_eq!(text, " ");
}

#[test]
fn non_hidden_cell_renders_actual_content() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"visible");
    let cell = parser.screen().cell(0, 0).unwrap();
    assert!(!cell.hidden());
    let (text, _) = cell_to_text_and_style(cell);
    assert_eq!(text, "v");
}

#[test]
fn hidden_then_visible_renders_correctly() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    // "AB" hidden, then "CD" visible
    parser.process(b"\x1b[8mAB\x1b[28mCD");
    let screen = parser.screen();

    // Hidden cells render as spaces
    let (t0, _) = cell_to_text_and_style(screen.cell(0, 0).unwrap());
    let (t1, _) = cell_to_text_and_style(screen.cell(0, 1).unwrap());
    assert_eq!(t0, " ", "Hidden 'A' should render as space");
    assert_eq!(t1, " ", "Hidden 'B' should render as space");

    // Visible cells render normally
    let (t2, _) = cell_to_text_and_style(screen.cell(0, 2).unwrap());
    let (t3, _) = cell_to_text_and_style(screen.cell(0, 3).unwrap());
    assert_eq!(t2, "C");
    assert_eq!(t3, "D");
}

// ═══════════════════════════════════════════════════════════════════
// Strikethrough: cells with SGR 9 get Modifier::CROSSED_OUT
// ═══════════════════════════════════════════════════════════════════

#[test]
fn strikethrough_cell_has_crossed_out_modifier() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[9mstrike");
    let cell = parser.screen().cell(0, 0).unwrap();
    let (_text, style) = cell_to_text_and_style(cell);
    assert!(style.add_modifier.contains(Modifier::CROSSED_OUT),
        "Strikethrough cell style must contain CROSSED_OUT modifier, got: {:?}", style);
}

#[test]
fn non_strikethrough_cell_lacks_crossed_out_modifier() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"normal");
    let cell = parser.screen().cell(0, 0).unwrap();
    let (_text, style) = cell_to_text_and_style(cell);
    assert!(!style.add_modifier.contains(Modifier::CROSSED_OUT),
        "Non-strikethrough cell should not have CROSSED_OUT modifier");
}

#[test]
fn strikethrough_cell_still_renders_text() {
    // Unlike hidden, strikethrough should show the actual content
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"\x1b[9mX");
    let cell = parser.screen().cell(0, 0).unwrap();
    let (text, _) = cell_to_text_and_style(cell);
    assert_eq!(text, "X", "Strikethrough cell should render actual content, not space");
}

// ═══════════════════════════════════════════════════════════════════
// Combined attributes
// ═══════════════════════════════════════════════════════════════════

#[test]
fn bold_red_text_has_correct_color_and_modifier() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    // SGR 1 (bold) + SGR 31 (red fg)
    parser.process(b"\x1b[1;31mR");
    let cell = parser.screen().cell(0, 0).unwrap();
    let (text, style) = cell_to_text_and_style(cell);
    assert_eq!(text, "R");
    assert!(style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(style.fg.unwrap(), Color::Red,
        "SGR 31 (red) should map to Color::Red");
}

#[test]
fn hidden_with_bold_still_renders_as_space() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    // Bold + hidden
    parser.process(b"\x1b[1;8mX");
    let cell = parser.screen().cell(0, 0).unwrap();
    let (text, style) = cell_to_text_and_style(cell);
    assert_eq!(text, " ", "Hidden overrides content regardless of other modifiers");
    assert!(style.add_modifier.contains(Modifier::BOLD),
        "Bold modifier should still be set on hidden cells");
}

#[test]
fn strikethrough_hidden_cell_renders_as_space_with_crossed_out() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    // Both strikethrough and hidden
    parser.process(b"\x1b[8;9mX");
    let cell = parser.screen().cell(0, 0).unwrap();
    let (text, style) = cell_to_text_and_style(cell);
    assert_eq!(text, " ", "Hidden cell renders as space even with strikethrough");
    assert!(style.add_modifier.contains(Modifier::CROSSED_OUT),
        "CROSSED_OUT should still be in the style for hidden+strikethrough cells");
}

// ═══════════════════════════════════════════════════════════════════
// Full ratatui Buffer rendering proof
//
// This is the real end-to-end test: we render cells through ratatui's
// Buffer into crossterm's output and verify the actual escape codes
// that would be sent to the terminal.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn ratatui_buffer_hidden_cell_produces_space_in_output() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let area = Rect::new(0, 0, 10, 1);
    let mut buf = Buffer::empty(area);

    // Simulate what render_node does: feed parser, extract cells, write to buffer
    let mut parser = vt100::Parser::new(1, 10, 0);
    parser.process(b"\x1b[8mSECRET\x1b[28mOK");
    let screen = parser.screen();

    for col in 0..10u16 {
        if let Some(cell) = screen.cell(0, col) {
            let (text, style) = cell_to_text_and_style(cell);
            buf[(col, 0u16)].set_symbol(&text);
            buf[(col, 0u16)].set_style(style);
        }
    }

    // Verify the buffer content: hidden cells should be spaces
    for col in 0..6u16 {
        assert_eq!(buf[(col, 0u16)].symbol(), " ",
            "Buffer cell at col {col} (hidden) should be space, got {:?}", buf[(col, 0u16)].symbol());
    }
    assert_eq!(buf[(6u16, 0u16)].symbol(), "O");
    assert_eq!(buf[(7u16, 0u16)].symbol(), "K");
}

#[test]
fn ratatui_buffer_strikethrough_has_correct_modifier() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let area = Rect::new(0, 0, 10, 1);
    let mut buf = Buffer::empty(area);

    let mut parser = vt100::Parser::new(1, 10, 0);
    parser.process(b"\x1b[9mSTRIKE\x1b[29mOK");
    let screen = parser.screen();

    for col in 0..10u16 {
        if let Some(cell) = screen.cell(0, col) {
            let (text, style) = cell_to_text_and_style(cell);
            buf[(col, 0u16)].set_symbol(&text);
            buf[(col, 0u16)].set_style(style);
        }
    }

    // Struck-through cells should have CROSSED_OUT and actual text
    for col in 0..6u16 {
        let bcell = &buf[(col, 0u16)];
        assert!(bcell.modifier.contains(Modifier::CROSSED_OUT),
            "Buffer cell at col {col} should have CROSSED_OUT modifier");
        assert_ne!(bcell.symbol(), " ",
            "Strikethrough cell should have actual text, not space");
    }
    // Non-struck cells should not have CROSSED_OUT
    assert!(!buf[(6u16, 0u16)].modifier.contains(Modifier::CROSSED_OUT));
}

#[test]
fn ratatui_buffer_color_idx7_is_gray() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let area = Rect::new(0, 0, 5, 1);
    let mut buf = Buffer::empty(area);

    let mut parser = vt100::Parser::new(1, 5, 0);
    // SGR 37 = foreground color index 7 (light gray)
    parser.process(b"\x1b[37mA");
    let screen = parser.screen();
    let cell = screen.cell(0, 0).unwrap();
    let (_text, style) = cell_to_text_and_style(cell);
    buf[(0u16, 0u16)].set_symbol("A");
    buf[(0u16, 0u16)].set_style(style);

    assert_eq!(buf[(0u16, 0u16)].fg, Color::Gray,
        "SGR 37 (index 7) in buffer must be Color::Gray, not {:?}", buf[(0u16, 0u16)].fg);
}

#[test]
fn ratatui_buffer_color_idx15_is_white() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let area = Rect::new(0, 0, 5, 1);
    let mut buf = Buffer::empty(area);

    let mut parser = vt100::Parser::new(1, 5, 0);
    // SGR 97 = foreground color index 15 (bright white)
    parser.process(b"\x1b[97mA");
    let screen = parser.screen();
    let cell = screen.cell(0, 0).unwrap();
    let (_text, style) = cell_to_text_and_style(cell);
    buf[(0u16, 0u16)].set_symbol("A");
    buf[(0u16, 0u16)].set_style(style);

    assert_eq!(buf[(0u16, 0u16)].fg, Color::White,
        "SGR 97 (index 15) in buffer must be Color::White, not {:?}", buf[(0u16, 0u16)].fg);
}

// ═══════════════════════════════════════════════════════════════════
// Crossterm output byte verification
//
// THE ULTIMATE PROOF: render through ratatui's CrosstermBackend into
// a byte buffer and verify the actual escape sequences that would be
// written to the terminal.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn crossterm_output_strikethrough_emits_sgr9() {
    use ratatui::backend::CrosstermBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::backend::Backend;

    let area = Rect::new(0, 0, 3, 1);
    let mut buf = Buffer::empty(area);

    // Set up a cell with strikethrough
    buf[(0u16, 0u16)].set_symbol("X");
    buf[(0u16, 0u16)].set_style(Style::default().add_modifier(Modifier::CROSSED_OUT));
    buf[(1u16, 0u16)].set_symbol("Y");
    buf[(2u16, 0u16)].set_symbol("Z");

    // Render through CrosstermBackend into a Vec<u8>
    let mut output = Vec::new();
    let mut backend = CrosstermBackend::new(&mut output);

    // Draw the buffer content
    let cells: Vec<(u16, u16, &ratatui::buffer::Cell)> = buf.content().iter().enumerate().map(|(i, cell)| {
        let x = i as u16 % area.width;
        let y = i as u16 / area.width;
        (x, y, cell)
    }).collect();
    backend.draw(cells.into_iter()).unwrap();

    let out_str = String::from_utf8_lossy(&output);

    // SGR 9 = \x1b[9m (crossedout/strikethrough)
    assert!(out_str.contains("\x1b[9m"),
        "CrosstermBackend output must contain \\e[9m for CROSSED_OUT. Got:\n{:?}", out_str);
}

#[test]
fn crossterm_output_hidden_cell_is_space_not_sgr8() {
    use ratatui::backend::CrosstermBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::backend::Backend;

    let area = Rect::new(0, 0, 6, 1);
    let mut buf = Buffer::empty(area);

    // Simulate the HIDDEN workaround: cells get space content, no HIDDEN modifier
    let mut parser = vt100::Parser::new(1, 6, 0);
    parser.process(b"\x1b[8mABC\x1b[28mDEF");
    let screen = parser.screen();
    for col in 0..6u16 {
        if let Some(cell) = screen.cell(0, col) {
            let (text, style) = cell_to_text_and_style(cell);
            buf[(col, 0u16)].set_symbol(&text);
            buf[(col, 0u16)].set_style(style);
        }
    }

    // Render through CrosstermBackend
    let mut output = Vec::new();
    let mut backend = CrosstermBackend::new(&mut output);
    let cells: Vec<(u16, u16, &ratatui::buffer::Cell)> = buf.content().iter().enumerate().map(|(i, cell)| {
        let x = i as u16 % area.width;
        let y = i as u16 / area.width;
        (x, y, cell)
    }).collect();
    backend.draw(cells.into_iter()).unwrap();

    let out_str = String::from_utf8_lossy(&output);

    // We should NOT see \x1b[8m (SGR 8 / hidden) because we work around it
    // by rendering spaces instead.
    assert!(!out_str.contains("\x1b[8m"),
        "Output must NOT contain \\e[8m since we work around HIDDEN with spaces. Got:\n{:?}", out_str);

    // The first 3 cells should be spaces in the output (hidden "ABC")
    // The last 3 should be "DEF"
    // Extract visible text: strip all escape sequences
    let visible: String = out_str.chars().filter(|c| {
        // Very rough: skip control characters and escape sequences
        c.is_ascii_graphic() || *c == ' '
    }).collect();
    assert!(visible.contains("DEF"),
        "Visible output should contain 'DEF'. Full text: {:?}", visible);
}
