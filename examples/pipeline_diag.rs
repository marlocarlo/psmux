// Diagnostic: replicate the EXACT psmux rendering pipeline end-to-end
// and verify whether strikethrough (SGR 9) actually appears in the
// terminal output bytes.
//
// Pipeline: vt100 parser → cell extraction → Span/Line building →
//           Clear + Paragraph → ratatui Terminal::draw() → CrosstermBackend → bytes

use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr;

/// Identical to rendering.rs vt_to_color
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

/// Identical to the cell → span logic in render_node
fn build_lines_from_screen(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut spans: Vec<Span> = Vec::with_capacity(cols as usize);
        let mut c = 0;
        while c < cols {
            if let Some(cell) = screen.cell(r, c) {
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
                let text = if cell.hidden() {
                    " ".to_string()
                } else {
                    cell.contents().to_string()
                };
                let w = UnicodeWidthStr::width(text.as_str()) as u16;
                if w == 0 {
                    spans.push(Span::styled(" ".to_string(), style));
                    c += 1;
                } else if w >= 2 {
                    if c + w > cols {
                        spans.push(Span::styled(" ".to_string(), style));
                        c += 1;
                    } else {
                        spans.push(Span::styled(text, style));
                        c += 2;
                    }
                } else {
                    spans.push(Span::styled(text, style));
                    c += 1;
                }
            } else {
                spans.push(Span::raw(" ".to_string()));
                c += 1;
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn main() {
    let rows: u16 = 3;
    let cols: u16 = 40;

    // Step 1: Parse VT input exactly as psmux does
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(b"\x1b[9mSTRIKE\x1b[29m NORMAL \x1b[8mHIDDEN\x1b[28m VIS\r\n");
    parser.process(b"\x1b[1;31mBOLD_RED\x1b[0m plain\r\n");
    parser.process(b"\x1b[37mIDX7\x1b[0m \x1b[97mIDX15\x1b[0m");

    let screen = parser.screen();

    // Verify parser state
    println!("=== Parser cell state ===");
    for col in 0..6 {
        let cell = screen.cell(0, col).unwrap();
        println!("  cell(0,{col}): '{}' strikethrough={} hidden={}",
            cell.contents(), cell.strikethrough(), cell.hidden());
    }
    let hcell = screen.cell(0, 15).unwrap();
    println!("  cell(0,15): '{}' hidden={}", hcell.contents(), hcell.hidden());

    // Step 2: Build lines exactly as render_node does
    let lines = build_lines_from_screen(screen, rows, cols);

    // Step 3: Inspect the spans
    println!("\n=== Span inspection ===");
    for (i, line) in lines.iter().enumerate() {
        for span in line.spans.iter() {
            let has_crossed = span.style.add_modifier.contains(Modifier::CROSSED_OUT);
            let has_bold = span.style.add_modifier.contains(Modifier::BOLD);
            if has_crossed || has_bold || span.content.trim() != "" {
                let content_preview: String = span.content.chars().take(20).collect();
                println!("  line[{i}] span '{content_preview}': crossed_out={has_crossed} bold={has_bold} fg={:?}",
                    span.style.fg);
            }
        }
    }

    // Step 4: Render through ratatui Terminal, EXACTLY as psmux does
    // (Clear + Paragraph, through Terminal::draw())
    // Test MULTIPLE frames like the real psmux render loop
    let output_bytes: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::new());
    // Use a shared writer so we can inspect between frames
    struct SharedWriter<'a>(&'a std::cell::RefCell<Vec<u8>>);
    impl<'a> std::io::Write for SharedWriter<'a> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    {
        let backend = CrosstermBackend::new(SharedWriter(&output_bytes));
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.resize(Rect::new(0, 0, cols, rows)).unwrap();

        // Frame 1: initial render
        terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Clear, area);
            let para = Paragraph::new(Text::from(lines.clone()));
            f.render_widget(para, area);
        }).unwrap();

        let frame1_len = output_bytes.borrow().len();
        println!("\n=== Frame 1 output: {} bytes ===", frame1_len);
        let f1 = String::from_utf8_lossy(&output_bytes.borrow()).to_string();
        let f1_esc: String = f1.chars().map(|c| {
            if c == '\x1b' { "\\e".to_string() }
            else if c.is_control() { format!("\\x{:02x}", c as u32) }
            else { c.to_string() }
        }).collect();
        println!("{}", &f1_esc[..f1_esc.len().min(400)]);
        println!("Frame 1 has \\e[9m: {}", f1.contains("\x1b[9m"));

        // Frame 2: same content (simulates steady-state redraw)
        terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Clear, area);
            let para = Paragraph::new(Text::from(lines.clone()));
            f.render_widget(para, area);
        }).unwrap();

        let total_after_f2 = output_bytes.borrow().len();
        let frame2_bytes = total_after_f2 - frame1_len;
        println!("\n=== Frame 2 output: {} more bytes (diff only) ===", frame2_bytes);
        if frame2_bytes > 0 {
            let all = output_bytes.borrow();
            let f2_slice = &all[frame1_len..];
            let f2 = String::from_utf8_lossy(f2_slice).to_string();
            let f2_esc: String = f2.chars().map(|c| {
                if c == '\x1b' { "\\e".to_string() }
                else if c.is_control() { format!("\\x{:02x}", c as u32) }
                else { c.to_string() }
            }).collect();
            println!("{}", &f2_esc[..f2_esc.len().min(400)]);
            // Check if frame 2 has a stray \e[0m that resets everything
            if f2.contains("\x1b[0m") {
                println!("WARNING: Frame 2 has \\e[0m reset!");
            }
        } else {
            println!("(no diff - content identical, as expected)");
        }

        // Frame 3: content changes (cursor moves, new text)
        let mut parser2 = vt100::Parser::new(rows, cols, 0);
        parser2.process(b"\x1b[9mNEW_STRIKE\x1b[29m rest\r\n");
        parser2.process(b"line2\r\n");
        parser2.process(b"line3");
        let lines2 = build_lines_from_screen(parser2.screen(), rows, cols);

        terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Clear, area);
            let para = Paragraph::new(Text::from(lines2));
            f.render_widget(para, area);
        }).unwrap();

        let total_after_f3 = output_bytes.borrow().len();
        let frame3_bytes = total_after_f3 - total_after_f2;
        println!("\n=== Frame 3 output: {} more bytes (new content) ===", frame3_bytes);
        if frame3_bytes > 0 {
            let all = output_bytes.borrow();
            let f3_slice = &all[total_after_f2..];
            let f3 = String::from_utf8_lossy(f3_slice).to_string();
            let f3_esc: String = f3.chars().map(|c| {
                if c == '\x1b' { "\\e".to_string() }
                else if c.is_control() { format!("\\x{:02x}", c as u32) }
                else { c.to_string() }
            }).collect();
            println!("{}", &f3_esc[..f3_esc.len().min(400)]);
            println!("Frame 3 has \\e[9m: {}", f3.contains("\x1b[9m"));
        }
    }

    // Step 5: Analyze the output bytes
    let binding = output_bytes.borrow();
    let out_str = String::from_utf8_lossy(&binding);
    println!("\n=== CrosstermBackend output analysis ===");
    println!("Total bytes: {}", output_bytes.borrow().len());

    // Search for SGR 9 (strikethrough)
    let has_sgr9 = out_str.contains("\x1b[9m");
    println!("Contains \\e[9m (strikethrough): {has_sgr9}");

    // Search for SGR 8 (hidden) -- should NOT be present
    let has_sgr8 = out_str.contains("\x1b[8m");
    println!("Contains \\e[8m (hidden): {has_sgr8} (should be false)");

    // Search for CROSSED_OUT in various forms
    let crossed_patterns = ["\x1b[9m", ";9m", ";9;"];
    for pat in &crossed_patterns {
        if out_str.contains(pat) {
            println!("  Found pattern: {:?}", pat);
        }
    }

    // Dump the first 500 chars of escaped output
    println!("\n=== Raw output (escaped, first 800 chars) ===");
    let escaped: String = out_str.chars().take(800).map(|c| {
        if c == '\x1b' { "\\e".to_string() }
        else if c == '\r' { "\\r".to_string() }
        else if c == '\n' { "\\n".to_string() }
        else if c.is_control() { format!("\\x{:02x}", c as u32) }
        else { c.to_string() }
    }).collect();
    println!("{escaped}");

    // Step 6: Check the ratatui buffer state directly
    println!("\n=== Buffer cell modifier check ===");
    {
        let mut backend2 = CrosstermBackend::new(Vec::<u8>::new());
        let mut terminal2 = Terminal::new(backend2).unwrap();
        terminal2.resize(Rect::new(0, 0, cols, rows)).unwrap();
        let frame_result = terminal2.draw(|f| {
            let area = f.area();
            f.render_widget(Clear, area);
            let para2 = Paragraph::new(Text::from(lines.clone()));
            f.render_widget(para2, area);
            // Check buffer cells
            let buf = f.buffer_mut();
            for col in 0..6u16 {
                let bcell = &buf[(col, 0u16)];
                println!("  buf[({col},0)]: '{}' modifier={:?}",
                    bcell.symbol(), bcell.modifier);
            }
            println!("  buf[(7,0)]: '{}' modifier={:?}", buf[(7u16, 0u16)].symbol(), buf[(7u16, 0u16)].modifier);
        });
    }

    // Final verdict
    println!("\n=== VERDICT ===");
    if has_sgr9 {
        println!("PASS: Strikethrough (\\e[9m) IS emitted in terminal output");
    } else {
        println!("FAIL: Strikethrough (\\e[9m) is MISSING from terminal output!");
        println!("  The bug is in the ratatui rendering pipeline.");
    }
    if !has_sgr8 {
        println!("PASS: Hidden (\\e[8m) is NOT in output (workaround working)");
    } else {
        println!("FAIL: Hidden (\\e[8m) leaked into output");
    }
}

