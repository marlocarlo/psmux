use portable_pty::{native_pty_system, PtySize, CommandBuilder};
use std::io::{Read, Write};

fn read_output(reader: &mut dyn Read, mut writer: Option<&mut dyn Write>, timeout_secs: u64, expect: &str) -> String {
    let mut buf = [0u8; 4096];
    let mut all = String::new();
    let start = std::time::Instant::now();
    let mut responded = false;
    loop {
        if start.elapsed() > std::time::Duration::from_secs(timeout_secs) {
            println!("  [TIMEOUT after {}s]", timeout_secs);
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                println!("  Read {} bytes: {:?}", n, &chunk[..chunk.len().min(200)]);
                all.push_str(&chunk);
                // If we see \x1b[6n (DSR), respond with cursor position report
                if !responded && all.contains("\x1b[6n") {
                    if let Some(w) = writer.as_deref_mut() {
                        println!("  >> Responding to DSR with \\x1b[1;1R");
                        let _ = w.write_all(b"\x1b[1;1R");
                        let _ = w.flush();
                        responded = true;
                    }
                }
                if all.contains(expect) { break; }
            }
            Err(e) => { println!("  Read error: {}", e); break; }
        }
    }
    all
}

fn main() {
    let pty_system = native_pty_system();
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };

    // TEST A: Respond to DSR query
    println!("=== TEST A: Respond to DSR \\x1b[6n] with cursor position ===");
    {
        let pair = pty_system.openpty(size).expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(&["/C", "echo TESTA_HELLO"]);
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let mut writer = pair.master.take_writer().expect("writer");
        let out = read_output(&mut *reader, Some(&mut *writer), 8, "TESTA_HELLO");
        let _ = child.wait();
        println!("  Result: {}", if out.contains("TESTA_HELLO") { "PASS" } else { "FAIL - no output" });
    }

    // TEST B: Same but do NOT drop slave
    println!("\n=== TEST B: No slave drop + respond to DSR ===");
    {
        let pair = pty_system.openpty(size).expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(&["/C", "echo TESTB_HELLO"]);
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        // NOT dropping slave
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let mut writer = pair.master.take_writer().expect("writer");
        let out = read_output(&mut *reader, Some(&mut *writer), 8, "TESTB_HELLO");
        let _ = child.wait();
        drop(pair.slave);
        println!("  Result: {}", if out.contains("TESTB_HELLO") { "PASS" } else { "FAIL - no output" });
    }

    // TEST C: Preemptive DSR response (write \x1b[1;1R BEFORE reading)
    println!("\n=== TEST C: Preemptive DSR response (write before read) ===");
    {
        let pair = pty_system.openpty(size).expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(&["/C", "echo TESTC_HELLO"]);
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let mut writer = pair.master.take_writer().expect("writer");
        // Preemptive DSR response - write BEFORE any reading
        let _ = writer.write_all(b"\x1b[1;1R");
        let _ = writer.flush();
        println!("  >> Sent preemptive DSR response");
        let out = read_output(&mut *reader, None, 8, "TESTC_HELLO");
        let _ = child.wait();
        println!("  Result: {}", if out.contains("TESTC_HELLO") { "PASS" } else { "FAIL - no output" });
    }

    // TEST D: No DSR response at all (control - should hang)
    println!("\n=== TEST D: No DSR response (control - expect FAIL) ===");
    {
        let pair = pty_system.openpty(size).expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(&["/C", "echo TESTD_HELLO"]);
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let _writer = pair.master.take_writer().expect("writer");
        let out = read_output(&mut *reader, None, 5, "TESTD_HELLO");
        let _ = child.wait();
        println!("  Result: {}", if out.contains("TESTD_HELLO") { "PASS" } else { "FAIL - no output (expected)" });
    }

    println!("\n=== ALL TESTS COMPLETE ===");
}
