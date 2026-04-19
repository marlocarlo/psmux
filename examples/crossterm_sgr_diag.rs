/// Diagnostic: verify what crossterm emits for CROSSED_OUT, HIDDEN modifiers.
/// Run with: cargo run --example crossterm_sgr_diag
use std::io::Write;

fn main() {
    let mut out: Vec<u8> = Vec::new();
    {
        use crossterm::style::{Attribute, SetAttribute, SetForegroundColor, Color as CtColor};
        use crossterm::QueueableCommand;
        let mut c = std::io::Cursor::new(&mut out);

        // Test CROSSED_OUT (should emit SGR 9)
        c.queue(SetAttribute(Attribute::CrossedOut)).unwrap();
        c.write_all(b"STRIKE").unwrap();
        c.queue(SetAttribute(Attribute::NotCrossedOut)).unwrap();
        c.write_all(b" ").unwrap();

        // Test HIDDEN (should emit SGR 8)
        c.queue(SetAttribute(Attribute::Hidden)).unwrap();
        c.write_all(b"HIDDEN").unwrap();
        c.queue(SetAttribute(Attribute::NoHidden)).unwrap();
        c.write_all(b" ").unwrap();

        // Test named color Red vs Indexed(1)
        c.queue(SetForegroundColor(CtColor::Red)).unwrap();
        c.write_all(b"RED").unwrap();
        c.queue(SetForegroundColor(CtColor::Reset)).unwrap();
        c.write_all(b" ").unwrap();

        c.queue(SetForegroundColor(CtColor::AnsiValue(1))).unwrap();
        c.write_all(b"IDX1").unwrap();
        c.queue(SetForegroundColor(CtColor::Reset)).unwrap();

        c.flush().unwrap();
    }

    println!("=== Raw bytes ({}) ===", out.len());
    // Show escape sequences
    let mut i = 0;
    while i < out.len() {
        if out[i] == 0x1b {
            let start = i;
            i += 1;
            while i < out.len() && !out[i].is_ascii_alphabetic() {
                i += 1;
            }
            if i < out.len() {
                i += 1;
            }
            let seq = &out[start..i];
            let seq_str = String::from_utf8_lossy(seq);
            println!("  ESC: {:?}", seq_str);
        } else if out[i].is_ascii_graphic() || out[i] == b' ' {
            let start = i;
            while i < out.len() && (out[i].is_ascii_graphic() || out[i] == b' ') {
                i += 1;
            }
            println!("  TXT: {:?}", String::from_utf8_lossy(&out[start..i]));
        } else {
            i += 1;
        }
    }

    // Also check ratatui Color mapping
    println!("\n=== ratatui Color → crossterm Color mapping ===");
    use ratatui::style::Color;
    println!("  Color::Red       = {:?}", Color::Red);
    println!("  Color::Indexed(1) = {:?}", Color::Indexed(1));
    println!("  Color::LightRed  = {:?}", Color::LightRed);
    println!("  Color::Indexed(9) = {:?}", Color::Indexed(9));
}
