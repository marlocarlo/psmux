// Simple test to verify PTY behavior with portable-pty 0.9
use portable_pty::{native_pty_system, CommandBuilder, PtySize, PtySystem};
use std::io::Read;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating PTY system...");
    let pty_system = native_pty_system();

    let size = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };

    println!("Opening PTY with size {}x{}...", size.cols, size.rows);
    let pair = pty_system.openpty(size)?;

    println!("Creating shell command...");
    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.args(["-NoLogo", "-NoProfile"]);

    println!("Spawning shell...");
    let mut child = pair.slave.spawn_command(cmd)?;
    println!("Shell spawned! PID: {:?}", child.process_id());

    // Get reader
    let mut reader = pair.master.try_clone_reader()?;
    println!("Got PTY reader");

    // Read in a thread
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut total = 0;
        println!("Reader thread started, waiting for data...");
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    total += n;
                    let text = String::from_utf8_lossy(&buf[..n]);
                    println!("Read {} bytes (total: {}): {:?}", n, total, text);
                }
                Ok(_) => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    println!("Reader error: {:?}", e);
                    break;
                }
            }
        }
        println!("Reader thread exiting, total bytes: {}", total);
    });

    // Check child status periodically
    for i in 0..50 {  // 5 seconds
        thread::sleep(Duration::from_millis(100));
        match child.try_wait() {
            Ok(Some(status)) => {
                println!("Child exited with status: {:?} (after {}ms)", status, (i+1)*100);
                break;
            }
            Ok(None) => {
                if i % 10 == 0 {
                    println!("Child still running ({}ms)...", (i+1)*100);
                }
            }
            Err(e) => {
                println!("Error checking child: {:?}", e);
                break;
            }
        }
    }

    // Final check
    match child.try_wait() {
        Ok(Some(status)) => println!("Final: Child exited with status: {:?}", status),
        Ok(None) => println!("Final: Child still running after 5 seconds"),
        Err(e) => println!("Final: Error checking child: {:?}", e),
    }

    // Kill child if still running
    let _ = child.kill();
    let _ = handle.join();

    println!("Test complete.");
    Ok(())
}
