/// Diagnostic: verify ratatui crossterm backend emits correct SGR for all modifiers.
/// Run with: cargo run --example ratatui_render_diag
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;
use std::io::Write;

fn main() {
    // Create terminal that writes to an in-memory buffer
    let mut raw_buf: Vec<u8> = Vec::new();
    {
        let backend = CrosstermBackend::new(&mut raw_buf);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| {
            let area = Rect::new(0, 0, 60, 3);

            // Row 0: Test various modifiers
            let buf = frame.buffer_mut();

            // STRIKE (cols 0-5)
            let strike_style = Style::default().add_modifier(Modifier::CROSSED_OUT);
            for (i, ch) in "STRIKE".chars().enumerate() {
                buf[(area.x + i as u16, area.y)].set_char(ch).set_style(strike_style);
            }

            // Space
            buf[(area.x + 6, area.y)].set_char(' ');

            // HIDDEN (cols 7-12)
            let hidden_style = Style::default().add_modifier(Modifier::HIDDEN);
            for (i, ch) in "HIDDEN".chars().enumerate() {
                buf[(area.x + 7 + i as u16, area.y)].set_char(ch).set_style(hidden_style);
            }

            // Space
            buf[(area.x + 13, area.y)].set_char(' ');

            // BOLDRED (cols 14-20) — using named Color::Red (should emit SGR 31)
            let boldred_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
            for (i, ch) in "BOLDRED".chars().enumerate() {
                buf[(area.x + 14 + i as u16, area.y)].set_char(ch).set_style(boldred_style);
            }

            // Space
            buf[(area.x + 21, area.y)].set_char(' ');

            // IDX1 (cols 22-25) — using Indexed(1) for comparison
            let idx1_style = Style::default().fg(Color::Indexed(1)).add_modifier(Modifier::BOLD);
            for (i, ch) in "IDX1".chars().enumerate() {
                buf[(area.x + 22 + i as u16, area.y)].set_char(ch).set_style(idx1_style);
            }
        }).unwrap();
    }

    // Write raw bytes to file for clean analysis
    std::fs::write("target/ratatui_sgr_dump.bin", &raw_buf).unwrap();

    // Analyze the raw bytes
    println!("=== Raw ratatui output ({} bytes) ===", raw_buf.len());
    println!("Written to target/ratatui_sgr_dump.bin");
    println!();

    // Extract and print all escape sequences
    let mut i = 0;
    while i < raw_buf.len() {
        if raw_buf[i] == 0x1b {
            let start = i;
            i += 1;
            while i < raw_buf.len() && !raw_buf[i].is_ascii_alphabetic() {
                i += 1;
            }
            if i < raw_buf.len() {
                i += 1;
            }
            let seq = &raw_buf[start..i];
            let seq_str = String::from_utf8_lossy(seq);

            // Check if the next few bytes are printable text
            let text_start = i;
            while i < raw_buf.len() && raw_buf[i] >= 0x20 && raw_buf[i] < 0x7f && raw_buf[i] != 0x1b {
                i += 1;
            }
            let text_after = if i > text_start {
                String::from_utf8_lossy(&raw_buf[text_start..i]).to_string()
            } else {
                String::new()
            };

            if !text_after.is_empty() {
                println!("  {} → {:?}", seq_str, text_after);
            } else {
                println!("  {}", seq_str);
            }
        } else if raw_buf[i] >= 0x20 && raw_buf[i] < 0x7f {
            let start = i;
            while i < raw_buf.len() && raw_buf[i] >= 0x20 && raw_buf[i] < 0x7f && raw_buf[i] != 0x1b {
                i += 1;
            }
            println!("  TEXT: {:?}", String::from_utf8_lossy(&raw_buf[start..i]));
        } else {
            i += 1;
        }
    }

    // Check for specific sequences
    let text = String::from_utf8_lossy(&raw_buf);
    println!("\n=== Key SGR Checks ===");
    println!("Contains \\e[9m  (strikethrough): {}", text.contains("\x1b[9m") || text.contains(";9m"));
    println!("Contains \\e[8m  (hidden):        {}", text.contains("\x1b[8m") || text.contains(";8m"));
    println!("Contains \\e[31m (dark red):      {}", text.contains("\x1b[31m") || text.contains(";31m"));
    println!("Contains \\e[38;5;1m (idx 1):     {}", text.contains("38;5;1m") || text.contains("38;5;1;"));
    println!("Contains \\e[1m  (bold):          {}", text.contains("\x1b[1m") || text.contains(";1m") || text.contains(";1;"));
}
