// examples/latency_harness.rs
//
// ConPTY-based latency harness for psmux.
//
// This is the most accurate test possible: it creates a real pseudo-terminal
// (exactly like Windows Terminal does), spawns psmux inside it, sends
// keystrokes through the PTY input pipe, and measures when output appears.
//
// Full pipeline measured:
//   keystroke → crossterm poll → TCP → server → ConPTY(WSL) echo
//   → vt100 parse → JSON serialize → TCP → JSON parse → ratatui render
//   → crossterm stdout → ConPTY output pipe → THIS harness detects it
//
// Usage:
//   cargo run --release --example latency_harness
//   cargo run --release --example latency_harness -- --pwsh
//   cargo run --release --example latency_harness -- --chars 80 --delay 200

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, thread};

fn main() {
    let args: Vec<String> = env::args().collect();
    let use_pwsh = args.iter().any(|a| a == "--pwsh");
    let char_count: usize = args
        .windows(2)
        .find(|w| w[0] == "--chars")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(80);
    let inter_delay_ms: u64 = args
        .windows(2)
        .find(|w| w[0] == "--delay")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(200);

    let shell = if use_pwsh { "pwsh" } else { "wsl" };
    println!("=== ConPTY Latency Harness ===");
    println!(
        "Shell: {}, Chars: {}, Inter-key delay: {}ms",
        shell, char_count, inter_delay_ms
    );
    println!();

    let psmux_exe = find_psmux_exe();
    let session_name = format!("harness_{}", std::process::id());
    let home = env::var("USERPROFILE").unwrap_or_default();
    let port_file = format!("{}\\.psmux\\{}.port", home, session_name);
    let key_file = format!("{}\\.psmux\\{}.key", home, session_name);

    // ── 1. Start detached psmux server ──
    println!("[1] Starting psmux server...");
    {
        let mut cmd = std::process::Command::new(&psmux_exe);
        cmd.args(["new-session", "-d", "-s", &session_name]);
        if !use_pwsh {
            cmd.arg("wsl");
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("Failed to start psmux server");
    }
    wait_for_files(&port_file, &key_file, Duration::from_secs(10));
    let port: u16 = std::fs::read_to_string(&port_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let key = std::fs::read_to_string(&key_file).unwrap().trim().to_string();
    println!("  Server on port {}", port);

    // ── 2. Disable status-bar clock via TCP ──
    // This prevents the status bar time from causing periodic re-renders
    // that confuse our output detection.
    println!("[2] Disabling status bar clock...");
    send_oneshot(&psmux_exe, &session_name, "set status-right \"\"");
    send_oneshot(&psmux_exe, &session_name, "set status-left \"test\"");
    thread::sleep(Duration::from_millis(200));

    // ── 3. Create ConPTY, spawn psmux attach ──
    println!("[3] Creating ConPTY and attaching client...");
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(&psmux_exe);
    cmd.args(["attach", "-t", &session_name]);
    let _child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn psmux client");
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().expect("clone reader");
    let mut pty_writer = pair.master.take_writer().expect("take writer");

    // ── 4. Output tracker thread ──
    // Track both total bytes AND last-activity timestamp (nanos since epoch)
    let epoch = Instant::now();
    let total_bytes = Arc::new(AtomicU64::new(0));
    let last_output_nanos = Arc::new(AtomicU64::new(0));
    {
        let tb = Arc::clone(&total_bytes);
        let lon = Arc::clone(&last_output_nanos);
        let ep = epoch;
        thread::spawn(move || {
            let mut r = reader;
            let mut buf = [0u8; 65536];
            loop {
                match r.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        tb.fetch_add(n as u64, Ordering::Release);
                        let now_ns = ep.elapsed().as_nanos() as u64;
                        lon.store(now_ns, Ordering::Release);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // ── 5. Wait for initial render + WSL startup ──
    println!("[4] Waiting for shell startup...");
    thread::sleep(Duration::from_secs(2));
    if !use_pwsh {
        thread::sleep(Duration::from_secs(1));
    }

    // ── 6. Clear screen ──
    println!("[5] Clearing screen...");
    for ch in b"clear" {
        pty_writer.write_all(&[*ch]).unwrap();
        pty_writer.flush().unwrap();
        thread::sleep(Duration::from_millis(30));
    }
    pty_writer.write_all(b"\r").unwrap();
    pty_writer.flush().unwrap();
    thread::sleep(Duration::from_millis(1500));

    // ── 7. Type characters and measure latency ──
    println!(
        "[6] Typing {} chars ({}ms gap). Measuring full pipeline latency...",
        char_count, inter_delay_ms
    );
    println!();

    let chars: Vec<u8> = b"abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz"
        .iter()
        .cycle()
        .take(char_count)
        .copied()
        .collect();

    let mut latencies: Vec<f64> = Vec::with_capacity(char_count);

    for (i, &ch) in chars.iter().enumerate() {
        // ── Wait for output to fully quiesce ──
        // We need at least 100ms of zero output to be sure nothing is happening.
        wait_for_quiesce(&last_output_nanos, &epoch, Duration::from_millis(100));

        // Record the send time (nanos since epoch)
        let send_nanos = epoch.elapsed().as_nanos() as u64;
        let send_instant = Instant::now();

        // ── Send keystroke ──
        pty_writer.write_all(&[ch]).unwrap();
        pty_writer.flush().unwrap();

        // ── Wait for output that arrives AFTER our keystroke ──
        // This guarantees we measure the actual echo, not lingering output.
        let timeout = Duration::from_millis(2000);
        let mut latency_ms: f64 = 2000.0;
        loop {
            let last_ns = last_output_nanos.load(Ordering::Acquire);
            if last_ns > send_nanos {
                // Output arrived after we sent the key!
                latency_ms = send_instant.elapsed().as_secs_f64() * 1000.0;
                break;
            }
            if send_instant.elapsed() > timeout {
                eprintln!(
                    "  TIMEOUT: No output for '{}' (idx {}) after 2s",
                    ch as char, i
                );
                break;
            }
            thread::sleep(Duration::from_micros(50));
        }

        latencies.push(latency_ms);

        // Let the full frame render before next iteration
        thread::sleep(Duration::from_millis(5));

        // Progress every 10 chars
        if (i + 1) % 10 == 0 {
            let s = if i >= 9 { i - 9 } else { 0 };
            let slice = &latencies[s..=i];
            let avg: f64 = slice.iter().sum::<f64>() / slice.len() as f64;
            let max: f64 = slice.iter().cloned().fold(0.0f64, f64::max);
            let min: f64 = slice.iter().cloned().fold(f64::MAX, f64::min);
            println!(
                "  [{:3}-{:3}] avg={:6.1}ms  min={:5.1}ms  max={:6.1}ms",
                s + 1,
                i + 1,
                avg,
                min,
                max
            );
        }

        // Inter-key delay
        if inter_delay_ms > 0 && i < char_count - 1 {
            thread::sleep(Duration::from_millis(inter_delay_ms));
        }
    }

    // Print remaining chars if not multiple of 10
    let rem = char_count % 10;
    if rem != 0 {
        let s = char_count - rem;
        let slice = &latencies[s..];
        let avg: f64 = slice.iter().sum::<f64>() / slice.len() as f64;
        let max: f64 = slice.iter().cloned().fold(0.0f64, f64::max);
        let min: f64 = slice.iter().cloned().fold(f64::MAX, f64::min);
        println!(
            "  [{:3}-{:3}] avg={:6.1}ms  min={:5.1}ms  max={:6.1}ms",
            s + 1,
            char_count,
            avg,
            min,
            max
        );
    }

    // ── 8. Analysis ──
    println!();
    println!("=== Results: {} ===", shell.to_uppercase());

    let n = latencies.len() as f64;
    let avg = latencies.iter().sum::<f64>() / n;
    let min = latencies.iter().cloned().fold(f64::MAX, f64::min);
    let max = latencies.iter().cloned().fold(0.0f64, f64::max);
    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = sorted[(sorted.len() as f64 * 0.5) as usize];
    let p90 = sorted[(sorted.len() as f64 * 0.9) as usize];
    let p95 = sorted[((sorted.len() as f64 * 0.95) as usize).min(sorted.len() - 1)];
    let p99 = sorted[((sorted.len() as f64 * 0.99) as usize).min(sorted.len() - 1)];

    println!(
        "  Avg={:.1}ms  P50={:.1}ms  P90={:.1}ms  P95={:.1}ms  P99={:.1}ms",
        avg, p50, p90, p95, p99
    );
    println!("  Min={:.1}ms  Max={:.1}ms", min, max);

    // Degradation analysis (crucial for "slower and slower" claim)
    let q_len = char_count / 4;
    if q_len > 0 {
        let q1: f64 = latencies[..q_len].iter().sum::<f64>() / q_len as f64;
        let q2: f64 =
            latencies[q_len..q_len * 2].iter().sum::<f64>() / q_len as f64;
        let q3: f64 =
            latencies[q_len * 2..q_len * 3].iter().sum::<f64>() / q_len as f64;
        let q4: f64 = latencies[q_len * 3..].iter().sum::<f64>()
            / (char_count - q_len * 3) as f64;
        let degrade = if q1 > 0.0 {
            ((q4 - q1) / q1) * 100.0
        } else {
            0.0
        };

        println!();
        println!("  Degradation trend (chars split into quarters):");
        println!(
            "    Q1 [{:3}-{:3}] = {:6.1}ms avg",
            1, q_len, q1
        );
        println!(
            "    Q2 [{:3}-{:3}] = {:6.1}ms avg",
            q_len + 1,
            q_len * 2,
            q2
        );
        println!(
            "    Q3 [{:3}-{:3}] = {:6.1}ms avg",
            q_len * 2 + 1,
            q_len * 3,
            q3
        );
        println!(
            "    Q4 [{:3}-{:3}] = {:6.1}ms avg",
            q_len * 3 + 1,
            char_count,
            q4
        );
        println!("    Q1->Q4 change: {:+.1}%", degrade);

        if degrade.abs() < 15.0 {
            println!("    VERDICT: No significant degradation");
        } else if degrade > 0.0 {
            println!(
                "    VERDICT: *** DEGRADATION DETECTED ({:+.0}%) ***",
                degrade
            );
        } else {
            println!("    VERDICT: Improved over time");
        }
    }

    // Distribution
    println!();
    let buckets: Vec<(&str, f64, f64)> = vec![
        ("0-10ms", 0.0, 10.0),
        ("10-20ms", 10.0, 20.0),
        ("20-40ms", 20.0, 40.0),
        ("40-60ms", 40.0, 60.0),
        ("60-100ms", 60.0, 100.0),
        ("100-200ms", 100.0, 200.0),
        ("200ms+", 200.0, 99999.0),
    ];
    for (name, lo, hi) in &buckets {
        let cnt = latencies.iter().filter(|&&v| v >= *lo && v < *hi).count();
        if cnt > 0 {
            let pct = (cnt as f64 / char_count as f64 * 100.0) as usize;
            let bar: String = "#".repeat(pct.min(50));
            println!("    {:>8}: {:3} ({:3}%) {}", name, cnt, pct, bar);
        }
    }

    println!();
    print!("  Raw: ");
    for (i, v) in latencies.iter().enumerate() {
        if i > 0 {
            print!(", ");
        }
        print!("{:.1}", v);
    }
    println!();

    // ── 9. Cleanup ──
    println!();
    println!("Cleaning up...");
    drop(pty_writer);

    let _ = std::process::Command::new(&psmux_exe)
        .args(["kill-server", "-t", &session_name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    thread::sleep(Duration::from_millis(500));

    let _ = std::fs::remove_file(&port_file);
    let _ = std::fs::remove_file(&key_file);
    println!("Done.");
}

fn find_psmux_exe() -> std::path::PathBuf {
    let self_exe = env::current_exe().unwrap();
    let mut dir = self_exe.parent().unwrap().to_path_buf();
    loop {
        let candidate = dir.join("psmux.exe");
        if candidate.exists() {
            return candidate;
        }
        if !dir.pop() {
            panic!("Could not find psmux.exe");
        }
    }
}

fn wait_for_files(port_file: &str, key_file: &str, timeout: Duration) {
    let start = Instant::now();
    while !std::path::Path::new(port_file).exists()
        || !std::path::Path::new(key_file).exists()
    {
        if start.elapsed() > timeout {
            panic!("Timeout waiting for server files");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn send_oneshot(psmux_exe: &std::path::Path, session: &str, cmd: &str) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let mut command = std::process::Command::new(psmux_exe);
    for p in &parts {
        command.arg(p);
    }
    command.args(["-t", session]);
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = command.status();
}

fn wait_for_quiesce(
    last_output_nanos: &Arc<AtomicU64>,
    epoch: &Instant,
    quiet_duration: Duration,
) {
    let quiet_ns = quiet_duration.as_nanos() as u64;
    loop {
        let last_ns = last_output_nanos.load(Ordering::Acquire);
        let now_ns = epoch.elapsed().as_nanos() as u64;
        if now_ns.saturating_sub(last_ns) >= quiet_ns {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
