/// Diagnostic: verify which SGR attributes survive ConPTY passthrough mode.
/// Run with: cargo run --example pty_sgr_diag
use portable_pty::{native_pty_system, PtySize, CommandBuilder};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
    let pty_system = native_pty_system();
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(size).expect("openpty failed");

    let mut cmd = CommandBuilder::new("pwsh.exe");
    cmd.args(["-NoProfile", "-NoLogo", "-Command",
        concat!(
            "Write-Host \"`e[9mSTRIKE`e[29m `e[8mHIDDEN`e[28m `e[1;31mBOLDRED`e[0m `e[38;2;255;128;0mRGB`e[0m done\"; ",
            "Start-Sleep -Milliseconds 200; ",
            "exit"
        )
    ]);

    let _child = pair.slave.spawn_command(cmd).expect("spawn failed");
    drop(pair.slave);

    // Send preemptive DSR response — ConPTY sends \e[6n and blocks until
    // it gets a cursor position report back
    let mut writer = pair.master.take_writer().expect("take writer");
    writer.write_all(b"\x1b[1;1R").expect("write DSR");
    writer.flush().expect("flush DSR");

    let mut reader = pair.master.try_clone_reader().expect("clone reader");

    let all_data: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let data_clone = all_data.clone();

    // Reader thread (read blocks on ConPTY pipe)
    let reader_handle = std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    data_clone.lock().unwrap().extend_from_slice(&buf[..n]);
                }
                Err(_) => break,
            }
        }
    });

    // Wait up to 8 seconds for PowerShell to start and produce output
    std::thread::sleep(Duration::from_secs(8));

    let data = all_data.lock().unwrap().clone();
    analyze_output(&data);

    // Don't wait for reader thread — just exit
    drop(reader_handle);
    std::process::exit(0);
}

fn analyze_output(all_data: &[u8]) {
    let text = String::from_utf8_lossy(all_data);
    println!("=== Raw output ({} bytes) ===", all_data.len());

    // Show hex dump of escape sequences
    let mut i = 0;
    while i < all_data.len() {
        if all_data[i] == 0x1b {
            let start = i;
            i += 1;
            while i < all_data.len() && !all_data[i].is_ascii_alphabetic() {
                i += 1;
            }
            if i < all_data.len() {
                i += 1;
            }
            let seq = &all_data[start..i];
            let seq_str = String::from_utf8_lossy(seq);
            print!("  ESC seq: {:?} = ", seq_str);
            for b in seq {
                print!("{:02x} ", b);
            }
            println!();
        } else {
            i += 1;
        }
    }

    println!("\n=== SGR Attribute Check ===");
    let has_sgr9 = text.contains("\x1b[9m") || text.contains(";9m") || text.contains(";9;");
    let has_sgr8 = text.contains("\x1b[8m") || text.contains(";8m") || text.contains(";8;");
    let has_sgr1_31 = text.contains("\x1b[1;31m") || text.contains("\x1b[31;1m");
    let has_rgb = text.contains("38;2;");
    let has_indexed_1 = text.contains("38;5;1");

    println!("SGR 9  (strikethrough): {}", if has_sgr9 { "FOUND" } else { "MISSING" });
    println!("SGR 8  (hidden):        {}", if has_sgr8 { "FOUND" } else { "MISSING" });
    println!("SGR 1;31 (bold red):    {}", if has_sgr1_31 { "FOUND" } else { "MISSING" });
    println!("RGB color (38;2;):      {}", if has_rgb { "FOUND" } else { "MISSING" });
    println!("Indexed (38;5;1):       {}", if has_indexed_1 { "FOUND (ConPTY re-encoded)" } else { "not present" });

    println!("\n=== vt100 Parser Check ===");
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(all_data);
    let screen = parser.screen();
    for row in 0..4 {
        for col in 0..80 {
            if let Some(cell) = screen.cell(row, col) {
                let ch = cell.contents();
                if !ch.is_empty() && ch != " " {
                    let attrs = format!(
                        "{}{}{}{}{}{}{}{}",
                        if cell.bold() { "B" } else { "." },
                        if cell.dim() { "D" } else { "." },
                        if cell.italic() { "I" } else { "." },
                        if cell.underline() { "U" } else { "." },
                        if cell.inverse() { "V" } else { "." },
                        if cell.blink() { "K" } else { "." },
                        if cell.hidden() { "H" } else { "." },
                        if cell.strikethrough() { "S" } else { "." },
                    );
                    let fg = format!("{:?}", cell.fgcolor());
                    print!("  r={} c={:2} ch='{}' attrs=[{}] fg={}", row, col, ch, attrs, fg);
                    if cell.strikethrough() { print!(" <<< STRIKETHROUGH"); }
                    if cell.hidden() { print!(" <<< HIDDEN"); }
                    println!();
                }
            }
        }
    }
}
