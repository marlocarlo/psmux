// Multi-binary crate (psmux, pmux, tmux) sharing all modules —
// suppress dead_code warnings for functions only used by a subset of binaries.
#![allow(dead_code)]

mod types;
mod platform;
mod cli;
mod session;
mod tree;
mod style;
mod rendering;
mod config;
mod commands;
mod pane;
mod popup;
mod copy_mode;
mod input;
mod layout;
mod window_ops;
mod util;
mod format;
mod help;
mod server;
mod client;
mod app;
mod ssh_input;
mod debug_log;
mod control;

use std::io::{self, Write, Read as _, BufRead as _, IsTerminal};
use std::time::Duration;
use std::env;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use crossterm::terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute};
use crossterm::cursor::{EnableBlinking, DisableBlinking};
use crossterm::event::{EnableMouseCapture, DisableMouseCapture, EnableBracketedPaste, DisableBracketedPaste};

use crate::platform::enable_virtual_terminal_processing;
use crate::cli::{print_help, print_version, print_commands};
use crate::session::{cleanup_stale_port_files, read_session_key, send_control,
    send_control_with_response, resolve_last_session_name, resolve_default_session_name,
    kill_remaining_server_processes};
use crate::rendering::apply_cursor_style;
use crate::server::run_server;
use crate::client::run_remote;
use crate::ssh_input::{send_mouse_enable, InputSource};

fn main() {
    if let Err(e) = run_main() {
        // Print a user-friendly error message instead of Rust's Debug format
        // which shows "Error: Custom { kind: Other, error: \"...\" }"  (fixes #47)
        let msg = e.to_string();
        eprintln!("psmux: {}", msg);
        std::process::exit(1);
    }
}

fn run_main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    
    // Clean up any stale port files at startup
    cleanup_stale_port_files();
    
    // Parse -L flag early (tmux-compatible: names the server socket for namespace isolation)
    // In psmux, -L <name> creates a namespace prefix for session port/key files.
    // Sessions under -L "foo" are stored as "foo__sessionname.port".
    // IMPORTANT: Only recognize -L as a global flag when it appears BEFORE the subcommand.
    // This avoids conflict with subcommand flags (e.g. select-pane -L, resize-pane -L).
    let mut l_socket_name: Option<String> = None;
    let mut f_config_file: Option<String> = None;
    let mut control_mode: u8 = 0; // 0=off, 1=-C (echo), 2=-CC (no echo)
    {
        let mut i = 1; // skip binary name
        while i < args.len() {
            let arg = &args[i];
            if arg == "-CC" {
                control_mode = 2;
                i += 1;
            } else if arg == "-C" {
                control_mode = 1;
                i += 1;
            } else if arg == "-L" && i + 1 < args.len() {
                l_socket_name = Some(args[i + 1].clone());
                i += 2;
            } else if arg == "-f" && i + 1 < args.len() {
                f_config_file = Some(args[i + 1].clone());
                i += 2;
            } else if (arg == "-S" || arg == "-t") && i + 1 < args.len() {
                i += 2; // skip other global flag-value pairs
            } else if arg.starts_with('-') {
                i += 1; // skip single global flags (e.g. -v, -V)
            } else {
                break; // hit the subcommand name — stop scanning for global flags
            }
        }
    }

    // Set PSMUX_CONFIG_FILE if -f was provided, so load_config() picks it up.
    if let Some(ref cf) = f_config_file {
        env::set_var("PSMUX_CONFIG_FILE", cf);
    }

    // Parse -t flag early to set target session for all commands
    // Supports session:window.pane format (e.g., "dev:0.1")
    // PSMUX_TARGET_SESSION stores the port file base name (for port file lookup)
    // PSMUX_TARGET_FULL stores the full target (session:window.pane) for the server
    if let Some(pos) = args.iter().position(|a| a == "-t") {
        if let Some(target) = args.get(pos + 1) {
            // Store the full target for the server to parse
            env::set_var("PSMUX_TARGET_FULL", target);
            // Extract just the session name for port file lookup
            let parsed_target = crate::cli::parse_target(target);
            let has_explicit_session = parsed_target.session.is_some();
            let session = parsed_target.session.unwrap_or_else(|| "default".to_string());
            // Apply -L namespace prefix for port file lookup
            let port_file_base = if let Some(ref l) = l_socket_name {
                format!("{}__{}", l, session)
            } else {
                session.clone()
            };
            // If the -t target includes an explicit session name, use it
            // directly. Otherwise (e.g. -t %2, -t :1.0) fall through to
            // the TMUX env var resolution below so we connect to the right
            // server when invoked from inside a psmux pane.
            if has_explicit_session {
                env::set_var("PSMUX_TARGET_SESSION", &port_file_base);
            }
        }
    }
    if env::var("PSMUX_TARGET_SESSION").is_err() {
        // No explicit session from -t: try to resolve from TMUX env var (set inside psmux panes)
        // TMUX format: /tmp/psmux-<pid>/<socket_name>,<port>,<session_idx>
        if let Ok(tmux_val) = env::var("TMUX") {
            // Extract the port from the TMUX value
            let parts: Vec<&str> = tmux_val.split(',').collect();
            if parts.len() >= 2 {
                if let Ok(port) = parts[1].trim().parse::<u16>() {
                    // Look up which session owns this port (port file base
                    // already includes -L namespace prefix if applicable)
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let psmux_dir = format!("{}\\.psmux", home);
                    if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().map(|e| e == "port").unwrap_or(false) {
                                if let Ok(port_str) = std::fs::read_to_string(&path) {
                                    if let Ok(file_port) = port_str.trim().parse::<u16>() {
                                        if file_port == port {
                                            if let Some(port_file_base) = path.file_stem().and_then(|s| s.to_str()) {
                                                // Skip warm (standby) sessions — they are internal-only
                                                if !crate::session::is_warm_session(port_file_base) {
                                                    env::set_var("PSMUX_TARGET_SESSION", port_file_base);
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Fallback: if no -t flag and session still not resolved (e.g. TMUX pointed
    // to a warm session, or no TMUX at all), pick the most recent real session.
    if env::var("PSMUX_TARGET_SESSION").is_err() {
        if let Some(name) = crate::session::resolve_last_session_name() {
            env::set_var("PSMUX_TARGET_SESSION", &name);
        }
    }
    
    // Find the actual command by skipping global -t/-L and their arguments.
    // -t is stripped everywhere (the global handler already set PSMUX_TARGET_SESSION).
    // -L is only stripped BEFORE the subcommand (global socket namespace flag);
    // after the subcommand, -L is kept (e.g. select-pane -L, resize-pane -L).
    let cmd_args: Vec<&String> = {
        let mut result = Vec::new();
        let mut i = 1; // skip binary name
        let mut found_subcommand = false;
        while i < args.len() {
            if !found_subcommand {
                // Before subcommand: skip global flags with values
                if (args[i] == "-t" || args[i] == "-L" || args[i] == "-f" || args[i] == "-S") && i + 1 < args.len() {
                    i += 2; // skip flag and its value
                    continue;
                } else if args[i] == "-h" || args[i] == "--help"
                       || args[i] == "-V" || args[i] == "-v" || args[i] == "--version" {
                    // Treat help/version flags as the subcommand itself
                    found_subcommand = true;
                    // fall through to push
                } else if args[i].starts_with('-') {
                    i += 1; // skip single global flags (e.g. -v)
                    continue;
                } else {
                    found_subcommand = true;
                    // fall through to push the subcommand name
                }
            } else {
                // After subcommand: strip only -t (and its value)
                if args[i] == "-t" && i + 1 < args.len() {
                    i += 2;
                    continue;
                }
            }
            result.push(&args[i]);
            i += 1;
        }
        result
    };
    
    let cmd = cmd_args.first().map(|s| s.as_str()).unwrap_or("");
    
    // Handle help and version flags first
    match cmd {
        "-h" | "--help" | "help" => {
            print_help();
            return Ok(());
        }
        "-V" | "-v" | "--version" | "version" => {
            print_version();
            return Ok(());
        }
        "list-commands" | "lscm" => {
            print_commands();
            return Ok(());
        }
        _ => {}
    }

    match cmd {
        // kill-server MUST be handled early before any potential fall-through
        "kill-server" => {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            let psmux_dir = format!("{}\\.psmux", home);
            // Compute namespace prefix for -L filtering (matches list-sessions behavior)
            let ns_prefix = l_socket_name.as_ref().map(|l| format!("{l}__"));
            let mut targets: Vec<(std::path::PathBuf, u16, String)> = Vec::new();
            let mut stale_ports: Vec<std::path::PathBuf> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "port").unwrap_or(false) {
                        if let Some(session_name) = path.file_stem().and_then(|s| s.to_str()) {
                            // Apply -L namespace filtering:
                            // With -L: only kill sessions under that namespace
                            // Without -L: kill ALL sessions (tmux behavior)
                            if let Some(ref pfx) = ns_prefix {
                                if !session_name.starts_with(pfx.as_str()) { continue; }
                            }
                            if let Ok(port_str) = std::fs::read_to_string(&path) {
                                if let Ok(port) = port_str.trim().parse::<u16>() {
                                    let sess_key = read_session_key(session_name).unwrap_or_default();
                                    targets.push((path.clone(), port, sess_key));
                                }
                            } else {
                                stale_ports.push(path.clone());
                            }
                        }
                    }
                }
            }
            // Send kill-server to all sessions in parallel via threads
            let handles: Vec<std::thread::JoinHandle<()>> = targets.into_iter().map(|(path, port, sess_key)| {
                std::thread::spawn(move || {
                    let addr = format!("127.0.0.1:{}", port);
                    if let Ok(mut stream) = std::net::TcpStream::connect_timeout(
                        &addr.parse().unwrap(),
                        Duration::from_millis(500),
                    ) {
                        let _ = stream.set_nodelay(true);
                        let _ = write!(stream, "AUTH {}\n", sess_key);
                        let _ = stream.flush();
                        let _ = std::io::Write::write_all(&mut stream, b"kill-server\n");
                        let _ = stream.flush();
                        let _ = stream.shutdown(std::net::Shutdown::Write);
                        // Wait for server to exit (EOF = done)
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
                        let mut buf = [0u8; 64];
                        loop {
                            match std::io::Read::read(&mut stream, &mut buf) {
                                Ok(0) => break,
                                Err(_) => break,
                                Ok(_) => continue,
                            }
                        }
                    }
                    // Remove port/key files regardless
                    let _ = std::fs::remove_file(&path);
                    let key_path = path.with_extension("key");
                    let _ = std::fs::remove_file(&key_path);
                })
            }).collect();
            // Wait for all threads to complete
            for h in handles { let _ = h.join(); }
            // Clean up stale port/key files
            for path in &stale_ports {
                let _ = std::fs::remove_file(path);
                let key_path = path.with_extension("key");
                let _ = std::fs::remove_file(&key_path);
            }
            // Brief wait then verify no processes remain; if any do, force-kill them.
            // Only do the nuclear fallback when not using -L namespace filtering.
            std::thread::sleep(Duration::from_millis(50));
            if ns_prefix.is_none() {
                kill_remaining_server_processes();
            }
            return Ok(());
        }
        "ls" | "list-sessions" => {
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let dir = format!("{}\\.psmux", home);
                // Compute namespace prefix for -L filtering
                let ns_prefix = l_socket_name.as_ref().map(|l| format!("{l}__"));
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        if let Some(name) = e.file_name().to_str() {
                            if let Some((base, ext)) = name.rsplit_once('.') {
                                if ext == "port" {
                                    // Skip warm (standby) sessions — internal-only
                                    if crate::session::is_warm_session(base) { continue; }
                                    // Filter by -L namespace: when -L is given, only show
                                    // sessions with that prefix; when no -L, only show
                                    // sessions without any namespace prefix
                                    if let Some(ref pfx) = ns_prefix {
                                        if !base.starts_with(pfx.as_str()) { continue; }
                                    } else {
                                        if base.contains("__") { continue; }
                                    }
                                    if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                                        if let Ok(_p) = port_str.trim().parse::<u16>() {
                                            let addr = format!("127.0.0.1:{}", port_str.trim());
                                            if let Ok(mut s) = std::net::TcpStream::connect_timeout(
                                                &addr.parse().unwrap(),
                                                Duration::from_millis(50)
                                            ) {
                                                let _ = s.set_read_timeout(Some(Duration::from_millis(50)));
                                                // Read session key and authenticate
                                                let key_path = format!("{}\\.psmux\\{}.key", home, base);
                                                if let Ok(key) = std::fs::read_to_string(&key_path) {
                                                    let _ = std::io::Write::write_all(&mut s, format!("AUTH {}\n", key.trim()).as_bytes());
                                                }
                                                let _ = std::io::Write::write_all(&mut s, b"session-info\n");
                                                let mut br = std::io::BufReader::new(s);
                                                let mut line = String::new();
                                                // Skip "OK" response from AUTH
                                                let _ = br.read_line(&mut line);
                                                if line.trim() == "OK" {
                                                    line.clear();
                                                    let _ = br.read_line(&mut line);
                                                }
                                                if !line.trim().is_empty() && line.trim() != "ERROR: Authentication required" { 
                                                    println!("{}", line.trim_end()); 
                                                } else { 
                                                    println!("{}", base); 
                                                }
                                            } else {
                                                // stale port file - remove it along with matching key
                                                let _ = std::fs::remove_file(e.path());
                                                let key_path = e.path().with_extension("key");
                                                let _ = std::fs::remove_file(&key_path);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                return Ok(());
            }
            "a" | "at" | "attach" | "attach-session" => {
                let name = args
                    .iter()
                    .position(|a| a == "-t")
                    .and_then(|i| args.get(i + 1))
                    .map(|s| s.clone())
                    .or_else(resolve_default_session_name)
                    .or_else(resolve_last_session_name)
                    .unwrap_or_else(|| "0".to_string());
                env::set_var("PSMUX_SESSION_NAME", name);
                env::set_var("PSMUX_REMOTE_ATTACH", "1");
            }
            "server" => {
                // Internal command - run headless server (used when spawning background server)
                let name = args.iter().position(|a| a == "-s").and_then(|i| args.get(i+1)).map(|s| s.clone()).unwrap_or_else(|| "default".to_string());
                // Parse -L socket name for namespace isolation
                let server_socket_name = args.iter().position(|a| a == "-L").and_then(|i| args.get(i+1)).map(|s| s.clone());
                // Check for initial command via -c flag (shell-wrapped)
                let initial_cmd = args.iter().position(|a| a == "-c").and_then(|i| args.get(i+1)).map(|s| s.clone());
                // Parse start directory via -d flag
                let srv_start_dir = args.iter().position(|a| a == "-d").and_then(|i| args.get(i+1)).map(|s| s.clone());
                // Parse window name via -n flag
                let srv_window_name = args.iter().position(|a| a == "-n").and_then(|i| args.get(i+1)).map(|s| s.clone());
                // Parse initial dimensions via -x / -y flags
                let srv_init_width = args.iter().position(|a| a == "-x").and_then(|i| args.get(i+1)).and_then(|s| s.parse::<u16>().ok());
                let srv_init_height = args.iter().position(|a| a == "-y").and_then(|i| args.get(i+1)).and_then(|s| s.parse::<u16>().ok());
                let srv_init_size = match (srv_init_width, srv_init_height) {
                    (Some(w), Some(h)) => Some((w, h)),
                    (Some(w), None) => Some((w, 24)),
                    (None, Some(h)) => Some((80, h)),
                    _ => None,
                };
                // Parse session group target via -g flag
                let srv_group_target = args.iter().position(|a| a == "-g").and_then(|i| args.get(i+1)).map(|s| s.clone());
                // Check for raw command after -- (direct execution)
                let raw_cmd: Option<Vec<String>> = args.iter().position(|a| a == "--").map(|pos| {
                    args.iter().skip(pos + 1).cloned().collect()
                }).filter(|v: &Vec<String>| !v.is_empty());
                return run_server(name, server_socket_name, initial_cmd, raw_cmd, srv_start_dir, srv_window_name, srv_init_size, srv_group_target);
            }
            "new-session" | "new" => {
                // Prevent nesting: block new-session inside an existing psmux session
                if env::var("PSMUX_ALLOW_NESTING").ok().as_deref() != Some("1") {
                    if env::var("PSMUX_ACTIVE").ok().as_deref() == Some("1")
                        || env::var("PSMUX_SESSION").ok().filter(|v| !v.is_empty()).is_some()
                    {
                        eprintln!("psmux: sessions should be nested with care, unset PSMUX_SESSION to force");
                        return Ok(());
                    }
                }
                // Strict getopt-style parsing for new-session flags.
                // tmux template: "Ac:dDe:EF:f:n:Ps:t:x:Xy:"
                // Flags that take a value (letter followed by ':'):
                //   -s (session name), -n (window name), -F (format),
                //   -c (start dir), -x (width), -y (height), -e (env),
                //   -f (client flags), -t (target session)
                // Boolean flags: -A, -d, -D, -E, -P, -X
                let mut session_name: Option<String> = None;
                let mut detached = false;
                let mut print_info = false;
                let mut format_str: Option<String> = None;
                let mut window_name: Option<String> = None;
                let mut start_dir: Option<String> = None;
                let mut attach_if_exists = false;
                let mut init_width: Option<u16> = None;
                let mut init_height: Option<u16> = None;
                let mut group_target: Option<String> = None;
                let mut positional_args: Vec<String> = Vec::new();
                let mut raw_cmd_after_dd: Option<Vec<String>> = None;

                {
                    let mut i = 1; // skip command name (cmd_args[0])
                    while i < cmd_args.len() {
                        let a = cmd_args[i].as_str();
                        if a == "--" {
                            // Everything after -- is raw command
                            raw_cmd_after_dd = Some(cmd_args[i+1..].iter().map(|s| s.to_string()).collect());
                            break;
                        }
                        // tmux uses getopt, which allows combined short flags
                        // like `-As main` (= `-A -s main`) or `-dP` (= `-d -P`).
                        // We expand combined flags inline.
                        if !a.starts_with('-') {
                            // Positional argument — collect it and everything after
                            positional_args.extend(cmd_args[i..].iter().map(|s| s.to_string()));
                            break;
                        }

                        let chars: Vec<char> = if a.len() > 2 && !a.starts_with("--") {
                            a[1..].chars().collect()
                        } else if a.len() == 2 {
                            vec![a.chars().nth(1).unwrap()]
                        } else {
                            // Unknown long flag, skip
                            i += 1; continue;
                        };

                        let mut k = 0;
                        while k < chars.len() {
                            let c = chars[k];
                            // Value-consuming flags: when in a combined group,
                            // the value is the next cmd_args element (getopt style).
                            match c {
                            's' => { i += 1; if i < cmd_args.len() { session_name = Some(cmd_args[i].to_string()); } break; }
                            'n' => { i += 1; if i < cmd_args.len() { window_name = Some(cmd_args[i].to_string()); } break; }
                            'F' => { i += 1; if i < cmd_args.len() { format_str = Some(cmd_args[i].trim_matches('"').to_string()); } break; }
                            'c' => { i += 1; if i < cmd_args.len() { start_dir = Some(cmd_args[i].trim_matches('"').to_string()); } break; }
                            'x' => { i += 1; if i < cmd_args.len() { init_width = cmd_args[i].parse::<u16>().ok(); } break; }
                            'y' => { i += 1; if i < cmd_args.len() { init_height = cmd_args[i].parse::<u16>().ok(); } break; }
                            'e' | 'f' => { i += 1; break; /* skip value */ }
                            't' => { i += 1; if i < cmd_args.len() { group_target = Some(cmd_args[i].to_string()); } break; }
                            // Boolean flags
                            'd' => { detached = true; }
                            'P' => { print_info = true; }
                            'A' => { attach_if_exists = true; }
                            'D' | 'E' | 'X' => { /* ignored for compatibility */ }
                            _ => { /* unknown flag, skip */ }
                            }
                            k += 1;
                        }
                        i += 1;
                    }
                }

                let name = session_name.unwrap_or_else(|| {
                    // tmux-compatible: auto-generate numeric name (0, 1, 2, ...)
                    crate::session::next_session_name(l_socket_name.as_deref())
                });
                // Compute port file base name: with -L namespace prefix if specified
                let port_file_base = if let Some(ref l) = l_socket_name {
                    format!("{}__{}", l, name)
                } else {
                    name.clone()
                };
                // Check for -- separator: everything after it is a raw command (direct execution)
                let raw_cmd_args: Option<Vec<String>> = raw_cmd_after_dd.filter(|v| !v.is_empty());
                // Parse initial command from positional args (legacy mode, no --)
                let initial_cmd: Option<String> = if raw_cmd_args.is_some() || positional_args.is_empty() {
                    None
                } else {
                    Some(positional_args.join(" "))
                };
                
                // Check if session already exists AND is actually running
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);
                if std::path::Path::new(&port_path).exists() {
                    // Verify server is actually running
                    let server_alive = if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let addr = format!("127.0.0.1:{}", port);
                            std::net::TcpStream::connect_timeout(
                                &addr.parse().unwrap(),
                                Duration::from_millis(100)
                            ).is_ok()
                        } else { false }
                    } else { false };
                    
                    if server_alive {
                        if attach_if_exists {
                            // -A flag: attach to existing session instead of erroring
                            env::set_var("PSMUX_SESSION_NAME", &port_file_base);
                            env::set_var("PSMUX_REMOTE_ATTACH", "1");
                            // Skip server creation, jump straight to attach
                            // (handled at the bottom of this match block)
                        } else {
                            eprintln!("psmux: session '{}' already exists", name);
                            return Ok(());
                        }
                    } else {
                        // Stale port file - remove it and continue
                        let _ = std::fs::remove_file(&port_path);
                    }
                }
                
                // If -A attached to an existing session, skip server creation
                if env::var("PSMUX_REMOTE_ATTACH").ok().as_deref() == Some("1") {
                    // Already set up for attach — skip server spawn
                } else {
                // Fast path: try to claim a pre-spawned warm server.
                // The warm server has config loaded and shell already running,
                // so claiming it avoids the full cold-start latency.
                // Only eligible when no custom command/dir is requested.
                // Skipped when PSMUX_NO_WARM=1 is set or config has 'set -g warm off'.
                let warm_disabled = std::env::var("PSMUX_NO_WARM").map(|v| v == "1" || v == "true").unwrap_or(false)
                    || crate::config::is_warm_disabled_by_config();
                let claimed_warm = if !warm_disabled && initial_cmd.is_none() && raw_cmd_args.is_none() && start_dir.is_none() {
                    let warm_base = if let Some(ref l) = l_socket_name {
                        format!("{}____warm__", l)
                    } else {
                        "__warm__".to_string()
                    };
                    let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
                    if std::path::Path::new(&warm_port_path).exists() {
                        if let Ok(warm_port_str) = std::fs::read_to_string(&warm_port_path) {
                            if let Ok(warm_port) = warm_port_str.trim().parse::<u16>() {
                                let warm_addr = format!("127.0.0.1:{}", warm_port);
                                if std::net::TcpStream::connect_timeout(
                                    &warm_addr.parse().unwrap(),
                                    Duration::from_millis(100),
                                ).is_ok() {
                                    let warm_key = crate::session::read_session_key(&warm_base).unwrap_or_default();
                                    if !warm_key.is_empty() {
                                        let client_cwd = std::env::current_dir()
                                            .ok()
                                            .and_then(|p| p.to_str().map(|s| s.to_string()));
                                        let claim_cmd = if let Some(ref cwd) = client_cwd {
                                            format!("claim-session {} {}\n", crate::util::quote_arg(&name), crate::util::quote_arg(cwd))
                                        } else {
                                            format!("claim-session {}\n", crate::util::quote_arg(&name))
                                        };
                                        match crate::session::send_auth_cmd_response(
                                            &warm_addr, &warm_key,
                                            claim_cmd.as_bytes(),
                                        ) {
                                            Ok(resp) if resp.contains("OK") => {
                                                if let Some(ref wn) = window_name {
                                                    let new_key = crate::session::read_session_key(&port_file_base).unwrap_or_default();
                                                    let _ = crate::session::send_auth_cmd(
                                                        &warm_addr, &new_key,
                                                        format!("rename-window {}\n", crate::util::quote_arg(wn)).as_bytes(),
                                                    );
                                                }
                                                true
                                            }
                                            _ => false,
                                        }
                                    } else { false }
                                } else {
                                    let _ = std::fs::remove_file(&warm_port_path);
                                    false
                                }
                            } else { false }
                        } else { false }
                    } else { false }
                } else { false };

                if !claimed_warm {
                // Cold path: spawn a background server from scratch
                let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
                let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), name.clone()];
                // Pass -L socket name to server for namespace isolation
                if let Some(ref l) = l_socket_name {
                    server_args.push("-L".into());
                    server_args.push(l.clone());
                }
                // Pass initial command if provided
                if let Some(ref init_cmd) = initial_cmd {
                    server_args.push("-c".into());
                    server_args.push(init_cmd.clone());
                }
                // Pass start directory to server
                if let Some(ref dir) = start_dir {
                    server_args.push("-d".into());
                    server_args.push(dir.clone());
                }
                // Pass window name to server
                if let Some(ref wn) = window_name {
                    server_args.push("-n".into());
                    server_args.push(wn.clone());
                }
                // Pass initial dimensions to server
                if let Some(w) = init_width {
                    server_args.push("-x".into());
                    server_args.push(w.to_string());
                }
                if let Some(h) = init_height {
                    server_args.push("-y".into());
                    server_args.push(h.to_string());
                }
                // Pass session group target to server
                if let Some(ref gt) = group_target {
                    server_args.push("-g".into());
                    server_args.push(gt.clone());
                }
                // Pass raw command args (direct execution) if -- was used
                if let Some(ref raw_args) = raw_cmd_args {
                    server_args.push("--".into());
                    for a in raw_args {
                        server_args.push(a.clone());
                    }
                }
                // On Windows, mark parent's stdout/stderr as non-inheritable before
                // spawning the server. This prevents the server from inheriting
                // PowerShell's redirect pipes (which would cause the parent to hang
                // waiting for the pipe to close). The server creates its own ConPTY
                // handles so it doesn't need the parent's stdio.
                #[cfg(windows)]
                {
                    #[link(name = "kernel32")]
                    extern "system" {
                        fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
                        fn SetHandleInformation(hObject: *mut std::ffi::c_void, dwMask: u32, dwFlags: u32) -> i32;
                    }
                    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5u32; // -11i32 as u32
                    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4u32;  // -12i32 as u32
                    const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
                    unsafe {
                        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
                        let stderr = GetStdHandle(STD_ERROR_HANDLE);
                        SetHandleInformation(stdout, HANDLE_FLAG_INHERIT, 0);
                        SetHandleInformation(stderr, HANDLE_FLAG_INHERIT, 0);
                    }
                }
                // Spawn server with a hidden console window via CreateProcessW.
                // This gives ConPTY a real console while keeping the window invisible.
                #[cfg(windows)]
                crate::platform::spawn_server_hidden(&exe, &server_args)?;
                #[cfg(not(windows))]
                {
                    let mut cmd = std::process::Command::new(&exe);
                    for a in &server_args { cmd.arg(a); }
                    cmd.stdin(std::process::Stdio::null());
                    cmd.stdout(std::process::Stdio::null());
                    cmd.stderr(std::process::Stdio::null());
                    let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn server: {e}")))?;
                }
                } // end if !claimed_warm (cold path)
                } // end else (not PSMUX_REMOTE_ATTACH)
                
                // Wait for server to create port file (up to 5 seconds)
                // Poll fast (10ms) — the server writes the port file early,
                // before spawning ConPTY/pwsh, so it should appear quickly.
                for _ in 0..500 {
                    if std::path::Path::new(&port_path).exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }

                // Verify the server is actually alive — the TCP listener is
                // already active when the port file appears (we moved file write
                // before create_window), so this connect should succeed instantly.
                if !std::path::Path::new(&port_path).exists() {
                    eprintln!("psmux: failed to create session '{}'", name);
                    std::process::exit(1);
                }
                {
                    let server_alive = if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let addr = format!("127.0.0.1:{}", port);
                            std::net::TcpStream::connect_timeout(
                                &addr.parse().unwrap(),
                                Duration::from_millis(100)
                            ).is_ok()
                        } else { false }
                    } else { false };
                    if !server_alive {
                        let _ = std::fs::remove_file(&port_path);
                        eprintln!("psmux: session '{}' exited immediately (check shell command)", name);
                        std::process::exit(1);
                    }
                }
                
                if detached {
                    // If -P flag, print pane info before returning
                    if print_info {
                        // Set target session so send_control_with_response connects to the right server
                        env::set_var("PSMUX_TARGET_SESSION", &port_file_base);
                        // Give server a moment to initialize
                        std::thread::sleep(Duration::from_millis(200));
                        // Query the server for pane info using display-message
                        let fmt = if let Some(ref f) = format_str {
                            f.clone()
                        } else {
                            // tmux default: new-session -P prints "session_name:"
                            "#{session_name}:".to_string()
                        };
                        match send_control_with_response(format!("display-message -p {}\n", fmt)) {
                            Ok(resp) => { let trimmed = resp.trim(); if !trimmed.is_empty() { println!("{}", trimmed); } }
                            Err(_) => {}
                        }
                    }
                    return Ok(());
                } else {
                    // User wants attached session - set env vars to attach
                    env::set_var("PSMUX_SESSION_NAME", &port_file_base);
                    env::set_var("PSMUX_REMOTE_ATTACH", "1");
                    // Continue to attach below...
                }
            }
            "new-window" | "neww" => {
                // Strict getopt-style parsing for new-window flags.
                // tmux template: "ac:dDe:F:kn:Pt:S:"
                let mut name_arg: Option<String> = None;
                let mut detached = false;
                let mut print_info = false;
                let mut format_str: Option<String> = None;
                let mut start_dir: Option<String> = None;
                let mut nw_positional: Vec<String> = Vec::new();
                {
                    let mut i = 1;
                    while i < cmd_args.len() {
                        let a = cmd_args[i].as_str();
                        if a == "--" { nw_positional.extend(cmd_args[i+1..].iter().map(|s| s.to_string())); break; }
                        match a {
                            "-n" => { i += 1; if i < cmd_args.len() { name_arg = Some(cmd_args[i].trim_matches('"').to_string()); } }
                            "-F" => { i += 1; if i < cmd_args.len() { format_str = Some(cmd_args[i].trim_matches('"').to_string()); } }
                            "-c" => { i += 1; if i < cmd_args.len() { start_dir = Some(cmd_args[i].trim_matches('"').to_string()); } }
                            "-t" | "-e" | "-S" => { i += 1; /* skip value */ }
                            "-d" => { detached = true; }
                            "-P" => { print_info = true; }
                            "-a" | "-D" | "-k" => { /* ignored for compatibility */ }
                            _ if a.starts_with('-') => { /* unknown flag, skip */ }
                            _ => { nw_positional.extend(cmd_args[i..].iter().map(|s| s.to_string())); break; }
                        }
                        i += 1;
                    }
                }
                let cmd_arg = nw_positional.join(" ");
                let cmd_arg = cmd_arg.as_str();
                let mut cmd_line = "new-window".to_string();
                if detached { cmd_line.push_str(" -d"); }
                if print_info { cmd_line.push_str(" -P"); }
                if let Some(ref fmt) = format_str {
                    cmd_line.push_str(&format!(" -F \"{}\"", fmt.replace("\"", "\\\"")));
                }
                if let Some(name) = &name_arg {
                    cmd_line.push_str(&format!(" -n \"{}\"", name.replace("\"", "\\\"")));
                }
                if let Some(dir) = &start_dir {
                    cmd_line.push_str(&format!(" -c \"{}\"", dir.replace("\"", "\\\"")));
                }
                if !cmd_arg.is_empty() {
                    cmd_line.push_str(&format!(" \"{}\"", cmd_arg.replace("\"", "\\\"")));
                }
                cmd_line.push('\n');
                if print_info {
                    let resp = send_control_with_response(cmd_line)?;
                    print!("{}", resp);
                } else {
                    send_control(cmd_line)?;
                }
                return Ok(());
            }
            "split-window" | "splitw" => {
                // Strict getopt-style parsing for split-window flags.
                // tmux template: "bc:de:F:fhIl:p:Pt:vZ"
                let mut flag = "-v";
                let mut detached = false;
                let mut print_info = false;
                let mut format_str: Option<String> = None;
                let mut start_dir: Option<String> = None;
                let mut size_pct: Option<String> = None;
                let mut size_cells: Option<String> = None;
                let mut sw_positional: Vec<String> = Vec::new();
                {
                    let mut i = 1;
                    while i < cmd_args.len() {
                        let a = cmd_args[i].as_str();
                        if a == "--" { sw_positional.extend(cmd_args[i+1..].iter().map(|s| s.to_string())); break; }
                        match a {
                            "-F" => { i += 1; if i < cmd_args.len() { format_str = Some(cmd_args[i].trim_matches('"').to_string()); } }
                            "-c" => { i += 1; if i < cmd_args.len() { start_dir = Some(cmd_args[i].trim_matches('"').to_string()); } }
                            "-p" => { i += 1; if i < cmd_args.len() { size_pct = Some(cmd_args[i].to_string()); size_cells = None; } }
                            "-l" => { i += 1; if i < cmd_args.len() { size_cells = Some(cmd_args[i].to_string()); size_pct = None; } }
                            "-t" | "-e" => { i += 1; /* skip value */ }
                            "-h" => { flag = "-h"; }
                            "-v" => { flag = "-v"; }
                            "-d" => { detached = true; }
                            "-P" => { print_info = true; }
                            "-b" | "-f" | "-I" | "-Z" => { /* ignored for compatibility */ }
                            _ if a.starts_with('-') => { /* unknown flag, skip */ }
                            _ => { sw_positional.extend(cmd_args[i..].iter().map(|s| s.to_string())); break; }
                        }
                        i += 1;
                    }
                }
                let cmd_arg = sw_positional.join(" ");
                let cmd_arg = cmd_arg.as_str();
                let mut cmd_line = format!("split-window {}", flag);
                if detached { cmd_line.push_str(" -d"); }
                if print_info { cmd_line.push_str(" -P"); }
                if let Some(ref fmt) = format_str {
                    cmd_line.push_str(&format!(" -F \"{}\"", fmt.replace("\"", "\\\"")));
                }
                if let Some(dir) = &start_dir {
                    cmd_line.push_str(&format!(" -c \"{}\"", dir.replace("\"", "\\\"")));
                }
                if let Some(pct) = &size_pct {
                    cmd_line.push_str(&format!(" -p {}", pct));
                } else if let Some(cells) = &size_cells {
                    cmd_line.push_str(&format!(" -l {}", cells));
                }
                if !cmd_arg.is_empty() {
                    cmd_line.push_str(&format!(" \"{}\"", cmd_arg.replace("\"", "\\\"")));
                }
                cmd_line.push('\n');
                if print_info {
                    let resp = send_control_with_response(cmd_line)?;
                    print!("{}", resp);
                } else {
                    let resp = send_control_with_response(cmd_line)?;
                    if !resp.is_empty() {
                        eprint!("{}", resp);
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "kill-pane" | "killp" => { send_control("kill-pane\n".to_string())?; return Ok(()); }
            "capture-pane" | "capturep" => {
                // Parse optional flags - cmd_args[0] is command, start from 1
                let mut cmd = "capture-pane".to_string();
                let mut print_stdout = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(target) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", target));
                                i += 1;
                            }
                        }
                        "-S" => {
                            if let Some(start) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -S {}", start));
                                i += 1;
                            }
                        }
                        "-E" => {
                            if let Some(end) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -E {}", end));
                                i += 1;
                            }
                        }
                        "-p" => { cmd.push_str(" -p"); print_stdout = true; }
                        "-e" => { cmd.push_str(" -e"); }
                        "-J" => { cmd.push_str(" -J"); }
                        "-b" => {
                            if let Some(buf) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -b {}", buf));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                if print_stdout {
                    let resp = send_control_with_response(cmd)?;
                    print!("{}", resp);
                } else {
                    send_control(cmd)?;
                }
                return Ok(());
            }
            // send-keys - Send keys to a pane (critical for scripting)
            "send-keys" | "send" | "send-key" => {
                let mut literal = false;
                let mut keys: Vec<String> = Vec::new();
                // Getopt-style parsing: -t consumes next arg, -l/-R are boolean
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-l" => { literal = true; }
                        "-R" => { keys.push("__RESET__".to_string()); }
                        "-t" => { i += 1; } // consume target value (already handled globally)
                        "-N" => { i += 1; } // repeat count, consume value
                        _ => { keys.push(cmd_args[i].to_string()); }
                    }
                    i += 1;
                }
                let mut cmd = "send-keys".to_string();
                if literal { cmd.push_str(" -l"); }
                // Quote arguments that contain spaces to preserve them
                for k in keys { 
                    if k.contains(' ') || k.contains('\t') || k.contains('"') {
                        // Escape embedded double-quotes and wrap in quotes.
                        // Do NOT escape backslashes: the server parser treats
                        // them as literal (Windows path separator).
                        let escaped = k.replace('"', "\\\"");
                        cmd.push_str(&format!(" \"{}\"", escaped));
                    } else {
                        cmd.push_str(&format!(" {}", k)); 
                    }
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // send-paste - Paste base64-encoded text to a pane
            "send-paste" => {
                let mut payload = String::new();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => { i += 1; } // consume target (handled globally)
                        _ => { payload = cmd_args[i].to_string(); }
                    }
                    i += 1;
                }
                if !payload.is_empty() {
                    send_control(format!("send-paste {}\n", payload))?;
                }
                return Ok(());
            }
            // select-pane - Select the active pane
            "select-pane" | "selectp" => {
                let mut cmd = "select-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-T" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -T \"{}\"", t));
                                i += 1;
                            }
                        }
                        "-P" => {
                            if let Some(s) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -P \"{}\"", s));
                                i += 1;
                            }
                        }
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-L" => { cmd.push_str(" -L"); }
                        "-R" => { cmd.push_str(" -R"); }
                        "-l" => { cmd.push_str(" -l"); }
                        "-Z" => { cmd.push_str(" -Z"); }
                        "-e" => { cmd.push_str(" -e"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-m" => { cmd.push_str(" -m"); }
                        "-M" => { cmd.push_str(" -M"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // select-window - Select a window
            "select-window" | "selectw" => {
                let mut cmd = "select-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-l" => { cmd.push_str(" -l"); }
                        "-n" => { cmd.push_str(" -n"); }
                        "-p" => { cmd.push_str(" -p"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // list-panes - List all panes
            "list-panes" | "lsp" => {
                let mut cmd = "list-panes".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { cmd.push_str(" -a"); }
                        "-s" => { cmd.push_str(" -s"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-F" => {
                            if let Some(f) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -F \"{}\"", f.trim_matches('"').replace("\"", "\\\"")));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // list-windows - List all windows
            "list-windows" | "lsw" => {
                let mut cmd = "list-windows".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { cmd.push_str(" -a"); }
                        "-J" => { cmd.push_str(" -J"); }
                        "-F" => {
                            if let Some(f) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -F \"{}\"", f.trim_matches('"').replace("\"", "\\\"")));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // kill-window - Kill a window
            "kill-window" | "killw" => {
                let mut cmd = "kill-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-a" => { cmd.push_str(" -a"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // kill-session - Kill a session
            "kill-session" | "kill-ses" => {
                let mut target: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                // Apply -L namespace prefix for port file lookup
                                let namespaced = if let Some(ref l) = l_socket_name {
                                    format!("{}__{}", l, t)
                                } else {
                                    t.to_string()
                                };
                                target = Some(namespaced);
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let session_name = target.clone().unwrap_or_else(|| {
                    env::var("PSMUX_TARGET_SESSION").unwrap_or_else(|_| {
                        // Apply -L namespace prefix to default
                        if let Some(ref l) = l_socket_name {
                            format!("{}__{}", l, "default")
                        } else {
                            "default".to_string()
                        }
                    })
                });
                if let Some(ref t) = target {
                    env::set_var("PSMUX_TARGET_SESSION", t);
                }
                // Try to send kill command to server
                if send_control("kill-session\n".to_string()).is_err() {
                    // Server not responding - clean up stale port file
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let port_path = format!("{}\\.psmux\\{}.port", home, session_name);
                    let _ = std::fs::remove_file(&port_path);
                }
                return Ok(());
            }
            // has-session - Check if session exists (for scripting)
            "has-session" | "has" => {
                // Get target from env (set from -t flag) or from remaining args
                let target = env::var("PSMUX_TARGET_SESSION").unwrap_or_else(|_| {
                    // Try to get session name from cmd_args
                    let mut t = "default".to_string();
                    let mut i = 1;
                    while i < cmd_args.len() {
                        if cmd_args[i].as_str() == "-t" {
                            if let Some(v) = cmd_args.get(i + 1) { t = v.to_string(); }
                            i += 1;
                        } else if !cmd_args[i].starts_with('-') {
                            t = cmd_args[i].to_string();
                            break;
                        }
                        i += 1;
                    }
                    // Apply -L namespace prefix for port file lookup
                    if let Some(ref l) = l_socket_name {
                        format!("{}__{}", l, t)
                    } else {
                        t
                    }
                });
                // Warm (standby) sessions are internal-only — treat as non-existent
                if crate::session::is_warm_session(&target) {
                    std::process::exit(1);
                }
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let path = format!("{}\\.psmux\\{}.port", home, target);
                if let Ok(port_str) = std::fs::read_to_string(&path) {
                    if let Ok(port) = port_str.trim().parse::<u16>() {
                        let addr = format!("127.0.0.1:{}", port);
                        // Actually authenticate and query the server to ensure it's healthy
                        let session_key = read_session_key(&target).unwrap_or_default();
                        if let Ok(mut s) = std::net::TcpStream::connect_timeout(
                            &addr.parse().unwrap(),
                            Duration::from_millis(500)
                        ) {
                            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                            let _ = write!(s, "AUTH {}\n", session_key);
                            let _ = write!(s, "session-info\n");
                            let _ = s.flush();
                            let mut buf = [0u8; 256];
                            if let Ok(n) = std::io::Read::read(&mut s, &mut buf) {
                                if n > 0 {
                                    let resp = String::from_utf8_lossy(&buf[..n]);
                                    if resp.contains("OK") {
                                        std::process::exit(0);
                                    }
                                }
                            }
                            // Fallback: connection succeeded so session likely exists
                            std::process::exit(0);
                        } else {
                            // Stale port file - clean it up
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                std::process::exit(1);
            }
            // rename-session - Rename a session
            "rename-session" | "rename" => {
                let mut new_name: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    if !cmd_args[i].starts_with('-') {
                        new_name = Some(cmd_args[i].to_string());
                        break;
                    }
                    i += 1;
                }
                if let Some(name) = new_name {
                    send_control(format!("rename-session {}\n", crate::util::quote_arg(&name)))?;
                }
                return Ok(());
            }
            // swap-pane - Swap panes
            "swap-pane" | "swapp" => {
                let mut cmd = "swap-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // resize-pane - Resize a pane
            "resize-pane" | "resizep" => {
                let mut cmd = "resize-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-L" => { cmd.push_str(" -L"); }
                        "-R" => { cmd.push_str(" -R"); }
                        "-Z" => { cmd.push_str(" -Z"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-x" => {
                            if let Some(v) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -x {}", v));
                                i += 1;
                            }
                        }
                        "-y" => {
                            if let Some(v) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -y {}", v));
                                i += 1;
                            }
                        }
                        s if s.parse::<i32>().is_ok() => {
                            cmd.push_str(&format!(" {}", s));
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // paste-buffer - Paste buffer into pane
            "paste-buffer" | "pasteb" => {
                let mut cmd = "paste-buffer".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -b {}", b));
                                i += 1;
                            }
                        }
                        "-d" => { cmd.push_str(" -d"); }
                        "-p" => { cmd.push_str(" -p"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // set-buffer - Set buffer contents
            "set-buffer" | "setb" => {
                let mut buffer_name: Option<String> = None;
                let mut data: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => {
                            data = Some(s.to_string());
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let mut cmd = "set-buffer".to_string();
                if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
                if let Some(d) = data { cmd.push_str(&format!(" {}", d)); }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // list-buffers - List paste buffers
            "list-buffers" | "lsb" => {
                let mut format_str: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-F" => {
                            if let Some(f) = cmd_args.get(i + 1) {
                                format_str = Some(f.to_string());
                                i += 1;
                            }
                        }
                        "-t" => { i += 1; } // skip target
                        _ => {}
                    }
                    i += 1;
                }
                let cmd = if let Some(fmt) = format_str {
                    format!("list-buffers -F {}\n", fmt)
                } else {
                    "list-buffers\n".to_string()
                };
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // show-buffer - Show buffer contents
            "show-buffer" | "showb" => {
                let mut buffer_name: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let mut cmd = "show-buffer".to_string();
                if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // delete-buffer - Delete a paste buffer
            "delete-buffer" | "deleteb" => {
                let mut buffer_name: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let mut cmd = "delete-buffer".to_string();
                if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // display-message - Display a message
            "display-message" | "display" => {
                let mut message: Vec<String> = Vec::new();
                let mut target: Option<String> = None;
                let mut print_to_stdout = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                target = Some(t.to_string());
                                i += 1;
                            }
                        }
                        "-p" => { print_to_stdout = true; }
                        s => { message.push(s.to_string()); }
                    }
                    i += 1;
                }
                let msg = message.join(" ");
                let mut cmd = "display-message".to_string();
                if let Some(t) = target { cmd.push_str(&format!(" -t {}", t)); }
                if print_to_stdout { cmd.push_str(" -p"); }
                cmd.push_str(&format!(" {}", msg));
                cmd.push('\n');
                if print_to_stdout {
                    let resp = send_control_with_response(cmd)?;
                    print!("{}", resp);
                } else {
                    send_control(cmd)?;
                }
                return Ok(());
            }
            // run-shell - Run a shell command
            "run-shell" | "run" => {
                let mut cmd_to_run: Vec<String> = Vec::new();
                let mut background = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => { background = true; }
                        s => { cmd_to_run.push(s.to_string()); }
                    }
                    i += 1;
                }
                let shell_cmd = crate::util::expand_run_shell_path(&cmd_to_run.join(" "));
                // Run the command using the system shell
                if background {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("pwsh")
                            .args(["-NoProfile", "-Command", &shell_cmd])
                            .spawn();
                    }
                } else {
                    #[cfg(windows)]
                    {
                        let output = std::process::Command::new("pwsh")
                            .args(["-NoProfile", "-Command", &shell_cmd])
                            .output()?;
                        io::stdout().write_all(&output.stdout)?;
                        io::stderr().write_all(&output.stderr)?;
                        std::process::exit(output.status.code().unwrap_or(0));
                    }
                }
                return Ok(());
            }
            // respawn-pane - Restart the pane's process
            "respawn-pane" | "respawnp" | "resp" => {
                let mut cmd = "respawn-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-k" => { cmd.push_str(" -k"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // last-window - Select last used window
            "last-window" | "last" => {
                send_control("last-window\n".to_string())?;
                return Ok(());
            }
            // last-pane - Select last used pane
            "last-pane" | "lastp" => {
                send_control("last-pane\n".to_string())?;
                return Ok(());
            }
            // next-window - Move to next window
            "next-window" | "next" => {
                send_control("next-window\n".to_string())?;
                return Ok(());
            }
            // previous-window - Move to previous window
            "previous-window" | "prev" => {
                send_control("previous-window\n".to_string())?;
                return Ok(());
            }
            // rotate-window - Rotate panes in window
            "rotate-window" | "rotatew" => {
                let mut cmd = "rotate-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // display-panes - Show pane numbers
            "display-panes" | "displayp" => {
                send_control("display-panes\n".to_string())?;
                return Ok(());
            }
            // break-pane - Break pane out to a new window
            "break-pane" | "breakp" => {
                let mut cmd = "break-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-d" => { cmd.push_str(" -d"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // join-pane - Join a pane to another window
            "join-pane" | "joinp" => {
                let mut cmd = "join-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-h" => { cmd.push_str(" -h"); }
                        "-v" => { cmd.push_str(" -v"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // rename-window - Rename current window
            "rename-window" | "renamew" => {
                // cmd_args[0] is the command, cmd_args[1] should be the new name
                if let Some(name) = cmd_args.get(1) {
                    if !name.starts_with('-') {
                        send_control(format!("rename-window {}\n", crate::util::quote_arg(name)))?;
                    }
                }
                return Ok(());
            }
            // zoom-pane - Toggle pane zoom
            "zoom-pane" | "resizep -Z" => {
                send_control("zoom-pane\n".to_string())?;
                return Ok(());
            }
            // source-file - Load a configuration file
            "source-file" | "source" => {
                let mut quiet = false;
                let mut file_path: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-q" => { quiet = true; }
                        "-n" => { /* parse only, don't execute */ }
                        "-v" => { /* verbose */ }
                        s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(path) = file_path {
                    // Expand ~ to home directory
                    let expanded = if path.starts_with('~') {
                        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                        path.replacen('~', &home, 1)
                    } else {
                        path
                    };
                    if let Err(e) = std::fs::read_to_string(&expanded) {
                        if !quiet {
                            eprintln!("psmux: {}: {}", expanded, e);
                            std::process::exit(1);
                        }
                    } else {
                        // Send source-file command to server if attached
                        send_control(format!("source-file {}\n", crate::util::quote_arg(&expanded)))?;
                    }
                }
                return Ok(());
            }
            // list-keys - List all key bindings
            "list-keys" | "lsk" => {
                let resp = send_control_with_response("list-keys\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // bind-key - Bind a key to a command
            "bind-key" | "bind" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                match send_control(format!("{}\n", cmd_str)) {
                    Ok(()) => {},
                    Err(e) if e.to_string().contains("no session") => {
                        eprintln!("warning: no active session; bind-key will take effect when set inside a session or via config file");
                    },
                    Err(e) => return Err(e),
                }
                return Ok(());
            }
            // unbind-key - Unbind a key
            "unbind-key" | "unbind" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                match send_control(format!("{}\n", cmd_str)) {
                    Ok(()) => {},
                    Err(e) if e.to_string().contains("no session") => {
                        eprintln!("warning: no active session; unbind-key will take effect when set inside a session or via config file");
                    },
                    Err(e) => return Err(e),
                }
                return Ok(());
            }
            // set-option / set - Set an option
            "set-option" | "set" => {
                let cmd_str: String = cmd_args.iter().map(|s| {
                    let s = s.as_str();
                    if s.contains(' ') {
                        format!("\"{}\"", s.replace('"', "\\\""))
                    } else {
                        s.to_string()
                    }
                }).collect::<Vec<String>>().join(" ");
                match send_control(format!("{}\n", cmd_str)) {
                    Ok(()) => {},
                    Err(e) if e.to_string().contains("no session") => {
                        eprintln!("warning: no active session; option will take effect when set inside a session or via config file");
                    },
                    Err(e) => return Err(e),
                }
                return Ok(());
            }
            // show-options / show / show-window-options / showw - Show options
            "show-options" | "show" | "show-window-options" | "showw" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                let resp = send_control_with_response(format!("{}\n", cmd_str))?;
                print!("{}", resp);
                return Ok(());
            }
            // if-shell - Conditional execution
            "if-shell" | "if" => {
                let mut background = false;
                let mut condition: Option<String> = None;
                let mut cmd_true: Option<String> = None;
                let mut cmd_false: Option<String> = None;
                let mut format_mode = false;
                let mut i = 1;
                
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => { background = true; }
                        "-F" => { format_mode = true; }
                        "-t" => { i += 1; } // Skip target
                        s if !s.starts_with('-') => {
                            if condition.is_none() {
                                condition = Some(s.to_string());
                            } else if cmd_true.is_none() {
                                cmd_true = Some(s.to_string());
                            } else if cmd_false.is_none() {
                                cmd_false = Some(s.to_string());
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                
                if let (Some(cond), Some(true_cmd)) = (condition, cmd_true) {
                    if background && !format_mode {
                        // -b flag: run the condition check in a background thread
                        // and dispatch the result command asynchronously (like tmux)
                        let cmd_false_bg = cmd_false.clone();
                        std::thread::spawn(move || {
                            let success = {
                                #[cfg(windows)]
                                {
                                    std::process::Command::new("pwsh")
                                        .args(["-NoProfile", "-Command", &cond])
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .status()
                                        .map(|s| s.success())
                                        .unwrap_or(false)
                                }
                                #[cfg(not(windows))]
                                {
                                    std::process::Command::new("sh")
                                        .args(["-c", &cond])
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .status()
                                        .map(|s| s.success())
                                        .unwrap_or(false)
                                }
                            };
                            let cmd_to_run = if success { Some(true_cmd) } else { cmd_false_bg };
                            if let Some(cmd) = cmd_to_run {
                                let tcp_cmd = format!("{}\n", cmd);
                                let _ = send_control_with_response(tcp_cmd);
                            }
                        });
                        // Return immediately — condition runs in background
                        return Ok(());
                    }

                    let success = if format_mode {
                        // Treat condition as format string - non-empty and non-zero is true
                        !cond.is_empty() && cond != "0"
                    } else if cond == "true" || cond == "1" {
                        true
                    } else if cond == "false" || cond == "0" {
                        false
                    } else {
                        // Run shell command - suppress stdout/stderr so it doesn't leak to terminal
                        #[cfg(windows)]
                        {
                            std::process::Command::new("pwsh")
                                .args(["-NoProfile", "-Command", &cond])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false)
                        }
                        #[cfg(not(windows))]
                        {
                            std::process::Command::new("sh")
                                .args(["-c", &cond])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false)
                        }
                    };
                    
                    let cmd_to_run = if success { Some(true_cmd) } else { cmd_false };
                    
                    if let Some(cmd) = cmd_to_run {
                        // Re-quote multi-word arguments for TCP transport
                        let needs_quoting = cmd.contains(' ');
                        let tcp_cmd = if needs_quoting {
                            // The command string may contain spaces (e.g. "display-message -p hello")
                            // Send it as-is since it's already a full command line
                            format!("{}\n", cmd)
                        } else {
                            format!("{}\n", cmd)
                        };
                        // Use send_control_with_response to capture any output from the chosen command
                        let resp = send_control_with_response(tcp_cmd)?;
                        if !resp.is_empty() {
                            print!("{}", resp);
                        }
                    }
                }
                return Ok(());
            }
            // wait-for - Wait for a signal
            "wait-for" | "wait" => {
                let mut lock = false;
                let mut signal = false;
                let mut unlock = false;
                let mut channel: Option<String> = None;
                let mut i = 1;
                
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-L" => { lock = true; }
                        "-S" => { signal = true; }
                        "-U" => { unlock = true; }
                        s if !s.starts_with('-') => { channel = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                
                if let Some(ch) = channel {
                    if signal {
                        send_control(format!("wait-for -S {}\n", ch))?;
                    } else if lock {
                        send_control(format!("wait-for -L {}\n", ch))?;
                    } else if unlock {
                        send_control(format!("wait-for -U {}\n", ch))?;
                    } else {
                        // Wait for channel - this blocks
                        let resp = send_control_with_response(format!("wait-for {}\n", ch))?;
                        if !resp.is_empty() {
                            print!("{}", resp);
                        }
                    }
                }
                return Ok(());
            }
            // select-layout - Select a layout for the window
            "select-layout" | "selectl" => {
                let mut layout: Option<String> = None;
                let mut next = false;
                let mut prev = false;
                let mut i = 1;
                
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-n" => { next = true; }
                        "-p" => { prev = true; }
                        "-o" => { /* last layout */ }
                        "-E" => { /* spread evenly */ }
                        "-t" => { i += 1; } // Skip target
                        s if !s.starts_with('-') => { layout = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                
                if next {
                    send_control("next-layout\n".to_string())?;
                } else if prev {
                    send_control("previous-layout\n".to_string())?;
                } else if let Some(l) = layout {
                    send_control(format!("select-layout {}\n", l))?;
                } else {
                    send_control("select-layout\n".to_string())?;
                }
                return Ok(());
            }
            // move-window - Move a window
            "move-window" | "movew" => {
                let mut cmd = "move-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { cmd.push_str(" -a"); }
                        "-b" => { cmd.push_str(" -b"); }
                        "-r" => { cmd.push_str(" -r"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-k" => { cmd.push_str(" -k"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // swap-window - Swap windows
            "swap-window" | "swapw" => {
                let mut cmd = "swap-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-d" => { cmd.push_str(" -d"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // list-clients - List all clients
            "list-clients" | "lsc" => {
                let resp = send_control_with_response("list-clients\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // switch-client - Switch the current client to another session
            "switch-client" | "switchc" => {
                let mut cmd = "switch-client".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-l" => { cmd.push_str(" -l"); }
                        "-n" => { cmd.push_str(" -n"); }
                        "-p" => { cmd.push_str(" -p"); }
                        "-r" => { cmd.push_str(" -r"); }
                        "-c" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -c {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // copy-mode - Enter copy mode
            "copy-mode" => {
                let mut cmd = "copy-mode".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-u" => { cmd.push_str(" -u"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-e" => { cmd.push_str(" -e"); }
                        "-H" => { cmd.push_str(" -H"); }
                        "-q" => { cmd.push_str(" -q"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // clock-mode - Display a clock
            "clock-mode" => {
                send_control("clock-mode\n".to_string())?;
                return Ok(());
            }
            // choose-buffer - List paste buffers interactively
            "choose-buffer" | "chooseb" => {
                let resp = send_control_with_response("choose-buffer\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // set-environment / setenv - Set environment variable
            "set-environment" | "setenv" => {
                let mut cmd = "set-environment".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-g" => { cmd.push_str(" -g"); }
                        "-r" => { cmd.push_str(" -r"); }
                        "-u" => { cmd.push_str(" -u"); }
                        "-h" => { cmd.push_str(" -h"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        s => { cmd.push_str(&format!(" {}", s)); }
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // show-environment / showenv - Show environment variables
            "show-environment" | "showenv" => {
                let mut cmd = "show-environment".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-g" => { cmd.push_str(" -g"); }
                        "-s" => { cmd.push_str(" -s"); }
                        "-h" => { cmd.push_str(" -h"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => { cmd.push_str(&format!(" {}", s)); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // load-buffer - Load a paste buffer from a file
            "load-buffer" | "loadb" => {
                let mut buffer_name: Option<String> = None;
                let mut file_path: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(path) = file_path {
                    let content = if path == "-" {
                        let mut input = String::new();
                        io::stdin().read_to_string(&mut input)?;
                        input
                    } else {
                        std::fs::read_to_string(&path)?
                    };
                    let mut cmd = "set-buffer".to_string();
                    if let Some(b) = buffer_name {
                        cmd.push_str(&format!(" -b {}", b));
                    }
                    // Escape the content for transmission
                    let escaped = content.replace('\n', "\\n").replace('\r', "\\r");
                    cmd.push_str(&format!(" {}", escaped));
                    cmd.push('\n');
                    send_control(cmd)?;
                }
                return Ok(());
            }
            // save-buffer - Save a paste buffer to a file
            "save-buffer" | "saveb" => {
                let mut buffer_name: Option<String> = None;
                let mut file_path: Option<String> = None;
                let mut append = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { append = true; }
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(path) = file_path {
                    let mut cmd = "show-buffer".to_string();
                    if let Some(b) = buffer_name {
                        cmd.push_str(&format!(" -b {}", b));
                    }
                    cmd.push('\n');
                    let content = send_control_with_response(cmd)?;
                    if path == "-" {
                        print!("{}", content);
                    } else if append {
                        use std::fs::OpenOptions;
                        let mut file = OpenOptions::new().append(true).create(true).open(&path)?;
                        file.write_all(content.as_bytes())?;
                    } else {
                        std::fs::write(&path, &content)?;
                    }
                }
                return Ok(());
            }
            // clear-history - Clear pane history
            "clear-history" | "clearhist" => {
                let mut cmd = "clear-history".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-H" => { cmd.push_str(" -H"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // pipe-pane - Pipe pane output to a command
            "pipe-pane" | "pipep" => {
                let mut cmd = "pipe-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-I" => { cmd.push_str(" -I"); }
                        "-O" => { cmd.push_str(" -O"); }
                        "-o" => { cmd.push_str(" -o"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        s => { cmd.push_str(&format!(" {}", s)); }
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // find-window - Search for a window
            "find-window" | "findw" => {
                let mut pattern: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-C" | "-N" | "-T" | "-i" | "-r" | "-Z" => {}
                        "-t" => { i += 1; }
                        s if !s.starts_with('-') => { pattern = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(p) = pattern {
                    let resp = send_control_with_response(format!("find-window {}\n", p))?;
                    print!("{}", resp);
                }
                return Ok(());
            }
            // list-commands - List all commands (duplicate handled above but kept for match completeness)
            "list-commands" | "lscm" => {
                print_commands();
                return Ok(());
            }
            // set-hook - Set a hook
            "set-hook" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                send_control(format!("{}\n", cmd_str))?;
                return Ok(());
            }
            // show-hooks - Show hooks
            "show-hooks" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                let resp = send_control_with_response(format!("{}\n", cmd_str))?;
                print!("{}", resp);
                return Ok(());
            }
            // next-layout - Cycle to next layout
            "next-layout" => {
                send_control("next-layout\n".to_string())?;
                return Ok(());
            }
            // previous-layout - Cycle to previous layout
            "previous-layout" => {
                send_control("previous-layout\n".to_string())?;
                return Ok(());
            }
            // choose-tree / choose-window / choose-session — interactive TUI choosers.
            // From the CLI just forward to the server so the attached client
            // (if any) can display the chooser.  Return exit 0.
            "choose-tree" | "choose-window" | "choose-session" => {
                send_control(format!("{}\n", cmd))?;
                return Ok(());
            }
            // command-prompt - Open interactive command prompt
            "command-prompt" => {
                let mut cmd = "command-prompt".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-I" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -I {}", t));
                                i += 1;
                            }
                        }
                        "-p" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -p {}", t));
                                i += 1;
                            }
                        }
                        "-1" => { cmd.push_str(" -1"); }
                        "-N" => { cmd.push_str(" -N"); }
                        "-W" => { cmd.push_str(" -W"); }
                        "-T" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -T {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // display-menu - Display a menu
            "display-menu" | "menu" => {
                let parts: Vec<String> = cmd_args.iter().map(|s| {
                    if s.contains(' ') || s.contains('"') { format!("\"{}\"" , s.replace('"', "\\\"")) } else { s.to_string() }
                }).collect();
                send_control(format!("{}\n", parts.join(" ")))?;
                return Ok(());
            }
            // display-popup - Display a popup window
            "display-popup" | "popup" => {
                let parts: Vec<String> = cmd_args.iter().map(|s| {
                    if s.contains(' ') || s.contains('"') { format!("\"{}\"" , s.replace('"', "\\\"")) } else { s.to_string() }
                }).collect();
                send_control(format!("{}\n", parts.join(" ")))?;
                return Ok(());
            }
            // server-info - Show server information
            "server-info" | "info" => {
                let resp = send_control_with_response("server-info\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // start-server / warmup - Pre-spawn a warm server
            "start-server" | "start" | "warmup" => {
                // Pre-spawn a warm __warm__ server so the next new-session is
                // instant.  Also triggers Windows Defender's scan cache on the
                // binary, eliminating the ~200-400ms first-run penalty.
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let warm_base = if let Some(ref l) = l_socket_name {
                    format!("{}____warm__", l)
                } else {
                    "__warm__".to_string()
                };
                let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
                // Check if warm server is already running
                let already_running = if std::path::Path::new(&warm_port_path).exists() {
                    if let Ok(port_str) = std::fs::read_to_string(&warm_port_path) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            std::net::TcpStream::connect_timeout(
                                &format!("127.0.0.1:{}", port).parse().unwrap(),
                                Duration::from_millis(100),
                            ).is_ok()
                        } else { false }
                    } else { false }
                } else { false };
                if already_running {
                    return Ok(());
                }
                // Clean up stale port file if any
                let _ = std::fs::remove_file(&warm_port_path);
                // Spawn the warm server
                let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
                let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), "__warm__".into()];
                if let Some(ref l) = l_socket_name {
                    server_args.push("-L".into());
                    server_args.push(l.clone());
                }
                // Detect terminal size for the warm server
                if let Ok((tw, th)) = crossterm::terminal::size() {
                    let h = th.saturating_sub(1);
                    if tw > 0 && h > 0 {
                        server_args.push("-x".into());
                        server_args.push(tw.to_string());
                        server_args.push("-y".into());
                        server_args.push(h.to_string());
                    }
                }
                #[cfg(windows)]
                crate::platform::spawn_server_hidden(&exe, &server_args)?;
                #[cfg(not(windows))]
                {
                    let mut cmd = std::process::Command::new(&exe);
                    for a in &server_args { cmd.arg(a); }
                    cmd.stdin(std::process::Stdio::null());
                    cmd.stdout(std::process::Stdio::null());
                    cmd.stderr(std::process::Stdio::null());
                    let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn warm server: {e}")))?;
                }
                return Ok(());
            }
            // confirm-before - Ask for confirmation before running a command
            "confirm-before" | "confirm" => {
                let parts: Vec<String> = cmd_args.iter().map(|s| {
                    if s.contains(' ') || s.contains('"') { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() }
                }).collect();
                send_control(format!("{}\n", parts.join(" ")))?;
                return Ok(());
            }
            // refresh-client - Refresh the client display
            "refresh-client" | "refresh" => {
                let mut cmd = "refresh-client".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-S" => { cmd.push_str(" -S"); }
                        "-l" => { cmd.push_str(" -l"); }
                        "-C" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -C {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // send-prefix - Send the prefix key to the active pane
            "send-prefix" => {
                send_control("send-prefix\n".to_string())?;
                return Ok(());
            }
            // show-messages - Show message log
            "show-messages" | "showmsgs" => {
                let resp = send_control_with_response("show-messages\n".to_string())?;
                if !resp.trim().is_empty() {
                    print!("{}", resp);
                }
                return Ok(());
            }
            // suspend-client - Suspend client (no-op on Windows)
            "suspend-client" | "suspendc" => {
                // No-op on Windows — no SIGTSTP concept
                return Ok(());
            }
            // lock-client / lock-server / lock-session (no-op on Windows)
            "lock-client" | "lockc" | "lock-server" | "lock" | "lock-session" | "locks" => {
                // No-op on Windows — no terminal locking concept
                return Ok(());
            }
            // resize-window - Resize window (no-op on Windows)
            "resize-window" | "resizew" => {
                // On Windows, window size is controlled by the terminal emulator
                return Ok(());
            }
            // customize-mode - tmux 3.2+ customize mode
            "customize-mode" => {
                send_control("customize-mode\n".to_string())?;
                return Ok(());
            }
            // choose-client - List clients interactively
            "choose-client" => {
                // Single-client model — returns current client info
                let resp = send_control_with_response("list-clients\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // respawn-window - Respawn active pane in window
            "respawn-window" | "respawnw" => {
                send_control("respawn-window\n".to_string())?;
                return Ok(());
            }
            // link-window - Link a window
            "link-window" | "linkw" => {
                let full = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                send_control(format!("{}\n", full))?;
                return Ok(());
            }
            // unlink-window - Unlink a window
            "unlink-window" | "unlinkw" => {
                send_control("unlink-window\n".to_string())?;
                return Ok(());
            }
            _ => {
                // Unknown command - print error and exit
                if !cmd.is_empty() {
                    eprintln!("psmux: unknown command: {}", cmd);
                    eprintln!("Run 'psmux --help' for usage information.");
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown command: {}", cmd)));
                }
            }
        }
    
    // Default behavior (bare `psmux` with no command):
    // tmux-compatible: always create a new session with the next available
    // numeric name (0, 1, 2, ...) and attach to it.

    // Control mode: connect to server with CONTROL/CONTROL_NOECHO protocol
    // instead of launching the TUI client. Must be checked before the
    // is_terminal() gate since control mode reads from piped stdin.
    if control_mode > 0 {
        return run_control_mode(control_mode);
    }

    //
    // If stdin is not a terminal (headless/non-interactive environment, e.g.
    // winget validation pipeline), print version and exit cleanly — starting
    // a TUI session would fail without an interactive console.
    if !std::io::stdin().is_terminal() {
        print_version();
        return Ok(());
    }
    if env::var("PSMUX_REMOTE_ATTACH").ok().as_deref() != Some("1") {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        let session_name = env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| {
            crate::session::next_session_name(l_socket_name.as_deref())
        });
        let port_file_base = if let Some(ref l) = l_socket_name {
            format!("{}__{}", l, session_name)
        } else {
            session_name.clone()
        };
        let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);

        // Try warm server claim first (fast path)
        // Skipped when PSMUX_NO_WARM=1 is set or config has 'set -g warm off'.
        let warm_disabled = std::env::var("PSMUX_NO_WARM").map(|v| v == "1" || v == "true").unwrap_or(false)
            || crate::config::is_warm_disabled_by_config();
        let warm_base = if let Some(ref l) = l_socket_name {
            format!("{}____warm__", l)
        } else {
            "__warm__".to_string()
        };
        let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
        let mut warm_claimed = false;
        if !warm_disabled && std::path::Path::new(&warm_port_path).exists() {
            let warm_key = crate::session::read_session_key(&warm_base).unwrap_or_default();
            if let Ok(port_str) = std::fs::read_to_string(&warm_port_path) {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    let addr = format!("127.0.0.1:{}", port);
                    if let Ok(mut stream) = std::net::TcpStream::connect_timeout(
                        &addr.parse().unwrap(),
                        Duration::from_millis(500),
                    ) {
                        let _ = stream.set_nodelay(true);
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(3000)));
                        let _ = write!(stream, "AUTH {}\n", warm_key);
                        let client_cwd = std::env::current_dir()
                            .ok()
                            .and_then(|p| p.to_str().map(|s| s.to_string()));
                        if let Some(ref cwd) = client_cwd {
                            let _ = write!(stream, "claim-session {} {}\n", crate::util::quote_arg(&session_name), crate::util::quote_arg(cwd));
                        } else {
                            let _ = write!(stream, "claim-session {}\n", crate::util::quote_arg(&session_name));
                        }
                        let _ = stream.flush();
                        // Use send_auth_cmd_response pattern: read AUTH
                        // "OK" line first, then read the claim-session
                        // response.  Previously a single raw read() would
                        // pick up only the AUTH "OK" and proceed before
                        // the server finished renaming port/key files,
                        // causing "auth failed" on the subsequent attach
                        // (issue #136).
                        if let Ok(reader_stream) = stream.try_clone() {
                            let mut br = std::io::BufReader::new(reader_stream);
                            let mut auth_line = String::new();
                            if std::io::BufRead::read_line(&mut br, &mut auth_line).unwrap_or(0) > 0
                                && auth_line.trim().starts_with("OK")
                            {
                                // Auth succeeded — now wait for the claim
                                // response so files are renamed before we
                                // try to attach.
                                let mut claim_line = String::new();
                                if std::io::BufRead::read_line(&mut br, &mut claim_line).unwrap_or(0) > 0
                                    && claim_line.contains("OK")
                                {
                                    warm_claimed = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        if !warm_claimed {
            // Cold path: spawn a new background server
            let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
            let server_args: Vec<String> = vec!["server".into(), "-s".into(), session_name.clone()];
            #[cfg(windows)]
            crate::platform::spawn_server_hidden(&exe, &server_args)?;
            #[cfg(not(windows))]
            {
                let mut cmd = std::process::Command::new(&exe);
                for a in &server_args { cmd.arg(a); }
                cmd.stdin(std::process::Stdio::null());
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn server: {e}")))?;
            }

            // Wait for server to start (fast polling — port file is written early)
            for _ in 0..500 {
                if std::path::Path::new(&port_path).exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        // Now attach to the session
        env::set_var("PSMUX_SESSION_NAME", &port_file_base);
        env::set_var("PSMUX_REMOTE_ATTACH", "1");
    }
    
    // Prevent nesting: similar to tmux checking $TMUX.
    // PSMUX_ACTIVE is set on the client process itself.
    // PSMUX_SESSION is set on child panes spawned by the server.
    // Both indicate we are already inside psmux.
    // Override with PSMUX_ALLOW_NESTING=1 if nesting is intentional.
    if env::var("PSMUX_ALLOW_NESTING").ok().as_deref() != Some("1") {
        if env::var("PSMUX_ACTIVE").ok().as_deref() == Some("1")
            || env::var("PSMUX_SESSION").ok().filter(|v| !v.is_empty()).is_some()
        {
            eprintln!("psmux: sessions should be nested with care, unset PSMUX_SESSION to force");
            return Ok(());
        }
    }
    env::set_var("PSMUX_ACTIVE", "1");

    let mut stdout = crate::platform::create_writer();
    enable_virtual_terminal_processing();
    enable_raw_mode()?;

    // Detect terminal type for input handling.
    // Use VT input parsing for SSH sessions and terminals that send VT mouse
    // sequences through ConPTY (e.g. JetBrains JediTerm).
    let use_vt_input = crate::ssh_input::needs_vt_input();

    // For standard terminals (not SSH), clear VTI flag from stdin if
    // crossterm or another layer set it. Keeps normal ReadConsoleInputW
    // behavior via proper INPUT_RECORDs.
    if !use_vt_input {
        crate::platform::disable_vti_on_stdin();
    }

    execute!(stdout, EnterAlternateScreen, EnableBlinking, EnableMouseCapture, EnableBracketedPaste)?;
    apply_cursor_style(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let input = InputSource::new(use_vt_input)?;

    // For VT input mode (SSH / JetBrains), explicitly (re-)send mouse-enable
    // escape sequences.  ConPTY may have consumed crossterm's
    // EnableMouseCapture output without forwarding it.
    if use_vt_input {
        send_mouse_enable();
    }

    // Loop to handle session switching without spawning new processes
    let result = loop {
        let result = run_remote(&mut terminal, &input);
        
        // Check if we should switch to another session
        if let Ok(switch_to) = env::var("PSMUX_SWITCH_TO") {
            env::remove_var("PSMUX_SWITCH_TO");
            env::set_var("PSMUX_SESSION_NAME", &switch_to);
            // Update last_session file
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            let last_path = format!("{}\\.psmux\\last_session", home);
            let _ = std::fs::write(&last_path, &switch_to);
            // Continue loop to attach to new session
            continue;
        }
        
        break result;
    };

    // Terminal cleanup — always runs, even on error, to prevent leaked
    // SGR attributes (invisible text), stuck raw mode, or stale cursor style.
    let _ = disable_raw_mode();
    let out = terminal.backend_mut();
    // Reset all SGR attributes (fg/bg color, bold, hidden, etc.) BEFORE
    // leaving the alternate screen.  SGR state is global and NOT restored
    // by the alternate-screen save/restore mechanism (\x1b[?1049l).
    // Without this, the last ratatui frame's foreground color can persist
    // into the main screen, making typed text invisible.
    let _ = execute!(out, crossterm::style::Print("\x1b[0m"));
    // Reset cursor style to terminal default (\x1b[0 q)
    let _ = execute!(out, crossterm::style::Print("\x1b[0 q"));
    let _ = execute!(out, DisableBlinking, DisableMouseCapture, DisableBracketedPaste, LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}

/// Run as a control mode client (psmux -C or psmux -CC).
/// Connects to the server via TCP, sends CONTROL/CONTROL_NOECHO,
/// reads commands from stdin and prints responses/notifications to stdout.
fn run_control_mode(mode: u8) -> io::Result<()> {
    use std::net::TcpStream;

    let session_name = env::var("PSMUX_SESSION_NAME")
        .unwrap_or_else(|_| "default".to_string());
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME"))
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    let psmux_dir = format!("{}\\.psmux", home);

    // Read port and key
    let port_path = format!("{}\\{}.port", psmux_dir, session_name);
    let key_path = format!("{}\\{}.key", psmux_dir, session_name);

    let port_str = std::fs::read_to_string(&port_path)
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("session '{}' not found (no port file)", session_name)))?;
    let port: u16 = port_str.trim().parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "corrupted port file"))?;
    let key = std::fs::read_to_string(&key_path)
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "session key file not found"))?
        .trim().to_string();

    // Connect
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, format!("cannot connect to session: {}", e)))?;
    let _ = stream.set_nodelay(true);

    // Auth
    write!(stream, "AUTH {}\n", key)?;
    stream.flush()?;

    // Read OK response
    let mut reader = io::BufReader::new(stream.try_clone()?);
    let mut ok_line = String::new();
    reader.read_line(&mut ok_line)?;
    if !ok_line.trim().starts_with("OK") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("auth failed: {}", ok_line.trim())));
    }

    // Send CONTROL or CONTROL_NOECHO
    let mode_str = if mode == 1 { "CONTROL" } else { "CONTROL_NOECHO" };
    let mut write_stream = reader.get_ref().try_clone()?;
    write!(write_stream, "{}\n", mode_str)?;
    write_stream.flush()?;

    // Spawn a thread to read server responses/notifications and print to stdout
    let reader_stream = reader.get_ref().try_clone()?;
    let reader_thread = std::thread::spawn(move || {
        let mut br = io::BufReader::new(reader_stream);
        let mut line = String::new();
        let stdout = io::stdout();
        loop {
            line.clear();
            match br.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let mut out = stdout.lock();
                    let _ = out.write_all(line.as_bytes());
                    let _ = out.flush();
                }
            }
        }
    });

    // Read commands from stdin and send to server
    let stdin = io::stdin();
    let mut stdin_line = String::new();
    loop {
        stdin_line.clear();
        match stdin.read_line(&mut stdin_line) {
            Ok(0) => break, // EOF
            Err(_) => break,
            Ok(_) => {
                if write!(write_stream, "{}", stdin_line).is_err() { break; }
                if write_stream.flush().is_err() { break; }
            }
        }
    }

    // After stdin EOF, shut down the write side only so the server
    // sees EOF and sends its final responses.  The reader thread
    // keeps running until the server closes *its* side.
    let _ = write_stream.shutdown(std::net::Shutdown::Write);

    // Wait briefly for the reader thread to drain remaining responses,
    // then forcibly close.  The server may take up to 5s (its read timeout)
    // to notice the client is gone.
    let handle = reader_thread;
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done2 = done.clone();
    std::thread::spawn(move || {
        let _ = handle.join();
        done2.store(true, std::sync::atomic::Ordering::Release);
    });
    // Drain for up to 2 seconds, then exit
    for _ in 0..40 {
        if done.load(std::sync::atomic::Ordering::Acquire) { break; }
        std::thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}
