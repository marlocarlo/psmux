use crossterm::event::{self, Event, KeyCode, KeyModifiers, KeyEventKind};
use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
fn main() {
    enable_raw_mode().unwrap();
    println!("Press Ctrl+Q (then Ctrl+C to exit):");
    loop {
        if event::poll(std::time::Duration::from_secs(5)).unwrap() {
            match event::read().unwrap() {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    println!("Key: code={:?} modifiers={:?} char_byte={}", 
                        key.code, key.modifiers,
                        match key.code { KeyCode::Char(c) => format!("0x{:02x}", c as u32), _ => "N/A".into() });
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    disable_raw_mode().unwrap();
}
