/// Diagnostic tool: dumps ALL raw crossterm events (Press, Release, Repeat)
/// for Enter and modified-Enter to prove what each terminal emulator reports.
/// Run inside Windows Terminal and WezTerm to compare behavior.
/// Press Ctrl+C to exit.
///
/// Writes to both stdout (for the user) and ~/.psmux/enter_diag_raw.log (for analysis).
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use std::io::Write;
use std::time::Instant;

fn main() {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let log_path = format!("{}/.psmux/enter_diag_raw.log", home);
    let _ = std::fs::create_dir_all(format!("{}/.psmux", home));
    let mut log = std::fs::OpenOptions::new()
        .create(true).truncate(true).write(true)
        .open(&log_path).expect("Cannot open log file");

    enable_raw_mode().unwrap();
    let start = Instant::now();
    let header = format!("=== Crossterm Raw Event Dumper (log: {}) ===", log_path);
    println!("{}\r", header);
    writeln!(log, "{}", header).ok();
    println!("Press Shift+Enter, Alt+Enter, Ctrl+Enter, plain Enter\r");
    println!("Press Ctrl+C to exit\r");
    println!("ALL Enter events (Press, Release, Repeat) are logged.\r");
    println!("---\r");
    loop {
        if event::poll(std::time::Duration::from_millis(50)).unwrap() {
            let evt = event::read().unwrap();
            let t = start.elapsed().as_millis();
            match &evt {
                Event::Key(key) => {
                    if matches!(key.code, KeyCode::Enter) || 
                       (matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL)) {
                        let line = format!("T+{:>6}ms  {:?}  code={:?}  mods={:?}  state={:?}",
                            t, key.kind, key.code, key.modifiers, key.state);
                        println!("{}\r", line);
                        writeln!(log, "{}", line).ok();
                        log.flush().ok();
                    }
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    disable_raw_mode().unwrap();
    println!("\r\nDone. Log saved to: {}\r", log_path);
}
