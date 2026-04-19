use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::net::TcpStream;

use crate::types::{CtrlReq, LayoutKind, WaitForOp, ControlNotification};
use crate::cli::parse_target;
use crate::util::base64_decode;
use crate::control;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
use crate::commands::parse_command_line;
use super::helpers::TMUX_COMMANDS;

/// Handle a single TCP connection from a client.
/// Parses auth, optional TARGET/PERSISTENT flags, then dispatches commands
/// to the main server event loop via the `tx` channel.
pub(crate) fn handle_connection(
    stream: TcpStream,
    tx: mpsc::Sender<CtrlReq>,
    session_key: &str,
    aliases: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
) {
let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
// Enable TCP_NODELAY for low-latency responses
let _ = stream.set_nodelay(true);
// Clone stream for writing, original goes into BufReader for reading
let mut write_stream = match stream.try_clone() {
    Ok(s) => s,
    Err(_) => return,
};

// Set initial timeout for auth (reduced from 5s - client sends immediately)
let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
let mut r = io::BufReader::new(stream);

// Read the authentication line
let mut auth_line = String::new();
if r.read_line(&mut auth_line).is_err() {
    return;
}

// Verify session key
let auth_line = auth_line.trim();
if !auth_line.starts_with("AUTH ") {
    // Legacy client without auth - reject for security
    let _ = write_stream.write_all(b"ERROR: Authentication required\n");
    let _ = write_stream.flush();
    return;
}
let provided_key = auth_line.strip_prefix("AUTH ").unwrap_or("");
if provided_key != session_key {
    let _ = write_stream.write_all(b"ERROR: Invalid session key\n");
    let _ = write_stream.flush();
    return;
}
// Auth successful - send OK and flush immediately
let _ = write_stream.write_all(b"OK\n");
let _ = write_stream.flush();

// Use a reasonable timeout for the first command after AUTH.
// Clients may have a small delay between AUTH and the actual command.
let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(2000)));

// Check for PERSISTENT flag and optional TARGET line
let mut persistent = false;
let mut resp_tx_opt: Option<mpsc::Sender<mpsc::Receiver<String>>> = None;
let mut global_target_win: Option<usize> = None;
let mut global_target_win_name: Option<String> = None;
let mut global_target_pane: Option<usize> = None;
let mut global_pane_is_id = false;
let mut line = String::new();
if r.read_line(&mut line).is_err() {
    return;
}

// Check if client requests persistent connection mode
if line.trim() == "PERSISTENT" {
    persistent = true;
    // Enable TCP_NODELAY for low-latency persistent connections
    let _ = r.get_ref().set_nodelay(true);
    let _ = write_stream.set_nodelay(true);
    // Use longer read timeout for persistent mode - client controls pacing
    let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(5000)));

    // Track this stream so the server can explicitly shut it down before
    // process::exit(0).  Without this, the client never gets EOF on
    // Windows loopback sockets.
    crate::types::register_persistent_stream(client_id, &write_stream);
    
    // Spawn a dedicated writer thread so the read loop never blocks
    // waiting for dump-state responses.  The read loop sends oneshot
    // receivers here; the writer thread waits for each response and
    // writes it to TCP in order.
    let mut ws_bg = write_stream.try_clone().unwrap();
    let (resp_tx, resp_rx) = mpsc::channel::<mpsc::Receiver<String>>();

    // Register a bounded frame channel for server-pushed frames (event-driven
    // rendering).  The channel queues up to FRAME_CHANNEL_CAPACITY frames,
    // allowing short bursts (e.g. fast typing) to be delivered without dropping
    // intermediate states, while still bounding memory for sustained throughput
    // scenarios (e.g. rapid scroll in copy mode).
    let frame_chan = crate::types::register_frame_channel(client_id);

    // Register a directive channel for queued directives (e.g. SWITCH).
    // Directives use a separate mpsc channel so they are never affected
    // by frame channel backpressure.
    let directive_rx = crate::types::register_directive_channel(client_id);

    std::thread::spawn(move || {
        let frame_rx = frame_chan.rx.lock().unwrap();
        loop {
            // 0. Check for queued directives (non-blocking) — these take priority
            while let Ok(directive) = directive_rx.try_recv() {
                if write!(ws_bg, "{}\n", directive).is_err() { return; }
                if ws_bg.flush().is_err() { return; }
            }
            // 1. Drain all pending command responses (non-blocking after first)
            match resp_rx.recv_timeout(Duration::from_millis(5)) {
                Ok(rrx) => {
                    if let Ok(text) = rrx.recv() {
                        if write!(ws_bg, "{}\n", text).is_err() { break; }
                        if ws_bg.flush().is_err() { break; }
                    }
                    while let Ok(rrx) = resp_rx.try_recv() {
                        if let Ok(text) = rrx.recv() {
                            if write!(ws_bg, "{}\n", text).is_err() { return; }
                            if ws_bg.flush().is_err() { return; }
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            // 2. Drain all queued frames from the bounded channel
            while let Ok(text) = frame_rx.try_recv() {
                if write!(ws_bg, "{}\n", text).is_err() { return; }
                if ws_bg.flush().is_err() { return; }
            }
        }
    });
    resp_tx_opt = Some(resp_tx);
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

// Check for CONTROL or CONTROL_NOECHO (control mode)
let control_echo = line.trim() == "CONTROL";
let control_noecho = line.trim() == "CONTROL_NOECHO";
if control_echo || control_noecho {
    let _ = r.get_ref().set_nodelay(true);
    let _ = write_stream.set_nodelay(true);
    let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(5000)));

    let ctrl_client_id = crate::types::next_control_client_id();
    crate::types::register_persistent_stream(ctrl_client_id, &write_stream);

    let (notif_tx, notif_rx) = std::sync::mpsc::sync_channel::<ControlNotification>(4096);

    // Register with server
    let _ = tx.send(CtrlReq::ControlRegister {
        client_id: ctrl_client_id,
        echo: control_echo,
        notif_tx: notif_tx,
    });

    // Spawn notification writer thread
    let mut ws_notif = match write_stream.try_clone() {
        Ok(s) => s,
        Err(_) => {
            let _ = tx.send(CtrlReq::ControlDeregister { client_id: ctrl_client_id });
            return;
        }
    };
    let cc_no_echo = control_noecho;
    let notif_thread = std::thread::spawn(move || {
        while let Ok(notif) = notif_rx.recv() {
            let is_exit = matches!(notif, ControlNotification::Exit { .. });
            let formatted = control::format_notification(&notif);
            if writeln!(ws_notif, "{}", formatted).is_err() { break; }
            // In -CC mode, send ST (ESC \) after %exit per tmux protocol
            if is_exit && cc_no_echo {
                let _ = ws_notif.write_all(b"\x1b\\");
            }
            if ws_notif.flush().is_err() { break; }
            if is_exit { break; }
        }
    });

    // Control mode command loop: read lines, dispatch, wrap in %begin/%end/%error
    let mut cmd_counter: u64 = 0;
    let tx_ctrl = tx.clone();
    let aliases_ctrl = aliases.clone();

    // Notify client that control mode is ready
    let _ = writeln!(write_stream);
    let _ = write_stream.flush();

    loop {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => break, // EOF
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
                    continue;
                }
                break;
            }
            Ok(_) => {}
        }

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        cmd_counter += 1;
        let ts = chrono::Utc::now().timestamp();

        // Echo the command if -C mode
        if control_echo {
            let _ = writeln!(write_stream, "{}", trimmed);
            let _ = write_stream.flush();
        }

        // Send %begin
        let _ = writeln!(write_stream, "{}", control::format_begin(ts, cmd_counter));
        let _ = write_stream.flush();

        // Dispatch the command
        let parsed = crate::cli::normalize_flag_equals(parse_command_line(trimmed));
        let raw_cmd = parsed.first().map(|s| s.as_str()).unwrap_or("");

        if raw_cmd.is_empty() {
            let _ = writeln!(write_stream, "{}", control::format_end(ts, cmd_counter));
            let _ = write_stream.flush();
            continue;
        }

        // Check aliases
        let alias_expanded = if let Ok(map) = aliases_ctrl.read() {
            map.get(raw_cmd).cloned()
        } else { None };

        let (cmd_name, cmd_args): (&str, Vec<&str>) = if let Some(ref expanded) = alias_expanded {
            let parts: Vec<&str> = expanded.split_whitespace().collect();
            let mut all: Vec<&str> = parts[1..].to_vec();
            all.extend(parsed.iter().skip(1).map(|s| s.as_str()));
            (parts.first().copied().unwrap_or(raw_cmd), all)
        } else {
            (raw_cmd, parsed.iter().skip(1).map(|s| s.as_str()).collect())
        };

        // Parse -t from command args
        let mut ctrl_target_win: Option<usize> = None;
        let mut ctrl_target_win_name: Option<String> = None;
        let mut ctrl_target_pane: Option<usize> = None;
        let mut ctrl_pane_is_id = false;
        let mut ctrl_raw_target: Option<String> = None;
        {
            let mut i = 0;
            while i < cmd_args.len() {
                if cmd_args[i] == "-t" {
                    if let Some(v) = cmd_args.get(i+1) {
                        ctrl_raw_target = Some(v.to_string());
                        let pt = parse_target(v);
                        if pt.window.is_some() { ctrl_target_win = pt.window; ctrl_target_win_name = None; }
                        else if pt.window_name.is_some() { ctrl_target_win_name = pt.window_name; ctrl_target_win = None; }
                        if pt.pane.is_some() {
                            ctrl_target_pane = pt.pane;
                            ctrl_pane_is_id = pt.pane_is_id;
                        }
                    }
                    i += 2; continue;
                }
                i += 1;
            }
        }

        // Build filtered args (without -t)
        let filtered_args: Vec<&str> = {
            let mut filtered = Vec::new();
            let mut i = 0;
            while i < cmd_args.len() {
                if cmd_args[i] == "-t" { i += 2; continue; }
                filtered.push(cmd_args[i]);
                i += 1;
            }
            filtered
        };

        // Apply target focus
        let is_focus_cmd = matches!(cmd_name, "select-window" | "selectw" | "select-pane" | "selectp");
        if let Some(wid) = ctrl_target_win {
            if is_focus_cmd {
                let _ = tx_ctrl.send(CtrlReq::FocusWindow(wid));
            } else {
                let _ = tx_ctrl.send(CtrlReq::FocusWindowTemp(wid));
            }
        } else if let Some(ref wname) = ctrl_target_win_name {
            if is_focus_cmd {
                let _ = tx_ctrl.send(CtrlReq::FocusWindowByName(wname.clone()));
            } else {
                let _ = tx_ctrl.send(CtrlReq::FocusWindowByNameTemp(wname.clone()));
            }
        }
        if let Some(pid) = ctrl_target_pane {
            if is_focus_cmd {
                if ctrl_pane_is_id {
                    let _ = tx_ctrl.send(CtrlReq::FocusPane(pid));
                } else {
                    let _ = tx_ctrl.send(CtrlReq::FocusPaneByIndex(pid));
                }
            } else {
                if ctrl_pane_is_id {
                    let _ = tx_ctrl.send(CtrlReq::FocusPaneTemp(pid));
                } else {
                    let _ = tx_ctrl.send(CtrlReq::FocusPaneByIndexTemp(pid));
                }
            }
        }

        // Dispatch command (use a oneshot for the response)
        let (resp_s, resp_r) = mpsc::channel::<String>();
        let dispatched = dispatch_control_command(
            cmd_name, &filtered_args, &tx_ctrl, resp_s,
            ctrl_target_pane, ctrl_pane_is_id, ctrl_raw_target.as_deref(),
            ctrl_client_id,
        );

        if dispatched {
            // Wait for response with timeout
            match resp_r.recv_timeout(Duration::from_secs(5)) {
                Ok(response) => {
                    if !response.is_empty() {
                        let _ = write!(write_stream, "{}", response);
                        if !response.ends_with('\n') {
                            let _ = writeln!(write_stream);
                        }
                    }
                    let _ = writeln!(write_stream, "{}", control::format_end(ts, cmd_counter));
                }
                Err(_) => {
                    let _ = writeln!(write_stream, "command timed out");
                    let _ = writeln!(write_stream, "{}", control::format_error(ts, cmd_counter));
                }
            }
        } else {
            // Command dispatched without response channel (fire and forget)
            let _ = writeln!(write_stream, "{}", control::format_end(ts, cmd_counter));
        }
        let _ = write_stream.flush();
    }

    // Deregister and clean up
    let _ = tx.send(CtrlReq::ControlDeregister { client_id: ctrl_client_id });
    drop(notif_thread);
    return;
}

// Check if this line is a TARGET specification
// Save raw target for relative pane specifiers like :.+ and :.-
let mut global_raw_target: Option<String> = None;
if line.trim().starts_with("TARGET ") {
    let target_spec = line.trim().strip_prefix("TARGET ").unwrap_or("");
    global_raw_target = Some(target_spec.to_string());
    let parsed = parse_target(target_spec);
    global_target_win = parsed.window;
    global_target_win_name = parsed.window_name;
    global_target_pane = parsed.pane;
    global_pane_is_id = parsed.pane_is_id;
    // Now read the actual command line
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

// Set short read timeout for batched command processing
let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(10)));

// Process commands in a loop to handle batching
let mut attached_sent = false;
let mut pending_chain: Vec<String> = Vec::new();
loop {
    // Check pending chained commands before reading from socket
    if !pending_chain.is_empty() {
        line = pending_chain.remove(0);
    } else if line.trim().is_empty() {
        // Try to read another command with timeout
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => {
                // EOF - client disconnected
                if attached_sent {
                    let _ = tx.send(CtrlReq::ClientDetach(client_id));
                }
                break;
            }
            Err(e) => {
                // In persistent mode, timeouts are expected - keep waiting
                if persistent && (e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut) {
                    line.clear(); // Clear any partial data from interrupted read
                    continue;
                }
                if attached_sent {
                    let _ = tx.send(CtrlReq::ClientDetach(client_id));
                }
                break; // Real error or non-persistent timeout
            }
            Ok(_) => continue, // Process the new line
        }
    }
    
    // Use quote-aware parser to preserve arguments with spaces
    // Handle command chaining (\; or ;) by splitting into sub-commands
    let sub_cmds = crate::config::split_chained_commands_pub(line.trim());
    let effective_line: String;
    if sub_cmds.len() > 1 {
        effective_line = sub_cmds[0].clone();
        pending_chain.extend(sub_cmds.into_iter().skip(1));
    } else {
        effective_line = line.trim().to_string();
    }
    let parsed = crate::cli::normalize_flag_equals(parse_command_line(&effective_line));
    let raw_cmd = parsed.get(0).map(|s| s.as_str()).unwrap_or("");
    // Check command aliases before normal dispatch
    let alias_expanded = if let Ok(map) = aliases.read() {
        map.get(raw_cmd).cloned()
    } else { None };
    let (cmd, args): (&str, Vec<&str>) = if let Some(ref expanded) = alias_expanded {
        // Alias expansion: replace command name, keep original args
        let expanded_parts: Vec<&str> = expanded.split_whitespace().collect();
        let mut all_args: Vec<&str> = expanded_parts[1..].to_vec();
        all_args.extend(parsed.iter().skip(1).map(|s| s.as_str()));
        (expanded_parts.first().copied().unwrap_or(raw_cmd), all_args)
    } else {
        (raw_cmd, parsed.iter().skip(1).map(|s| s.as_str()).collect())
    };

// Parse -t argument from command line (takes precedence over global TARGET)
let mut target_win: Option<usize> = global_target_win;
let mut target_win_name: Option<String> = global_target_win_name.clone();
let mut target_pane: Option<usize> = global_target_pane;
let mut pane_is_id = global_pane_is_id;
// Save raw -t value for relative pane targets like :.+ or :.-
// Falls back to global_raw_target from TARGET protocol line
let mut raw_target: Option<String> = global_raw_target.clone();
let mut i = 0;
while i < args.len() {
    if args[i] == "-t" {
        if let Some(v) = args.get(i+1) {
            raw_target = Some(v.to_string());
            // Parse the -t value using parse_target for consistent handling
            let pt = parse_target(v);
            if pt.window.is_some() { target_win = pt.window; target_win_name = None; }
            else if pt.window_name.is_some() { target_win_name = pt.window_name; target_win = None; }
            if pt.pane.is_some() { 
                target_pane = pt.pane;
                pane_is_id = pt.pane_is_id;
            }
        }
        i += 2; continue;
    }
    i += 1;
}
// Build args without -t and its value so command handlers get clean positional args
let args: Vec<&str> = {
    let mut filtered = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            i += 2; // skip -t and its value
            continue;
        }
        filtered.push(args[i]);
        i += 1;
    }
    filtered
};
// Commands that should permanently change focus when used with -t
let is_focus_cmd = matches!(cmd, "select-window" | "selectw" | "select-pane" | "selectp");
// Commands that handle -t internally and should NOT get FocusWindowTemp
let skip_target_focus = matches!(cmd, "join-pane" | "joinp" | "move-pane" | "movep");
if let Some(wid) = target_win {
    if is_focus_cmd {
        let _ = tx.send(CtrlReq::FocusWindow(wid));
    } else if !skip_target_focus {
        let _ = tx.send(CtrlReq::FocusWindowTemp(wid));
    }
} else if let Some(ref wname) = target_win_name {
    if is_focus_cmd {
        let _ = tx.send(CtrlReq::FocusWindowByName(wname.clone()));
    } else if !skip_target_focus {
        let _ = tx.send(CtrlReq::FocusWindowByNameTemp(wname.clone()));
    }
}
let targeted_kill_pane_id = if matches!(cmd, "kill-pane" | "killp") && pane_is_id {
    target_pane
} else {
    None
};
let skip_pane_focus = matches!(cmd, "display-message" | "display") || skip_target_focus;
if !skip_pane_focus && targeted_kill_pane_id.is_none() {
    if let Some(pid) = target_pane {
        if is_focus_cmd {
            if pane_is_id {
                let _ = tx.send(CtrlReq::FocusPane(pid));
            } else {
                let _ = tx.send(CtrlReq::FocusPaneByIndex(pid));
            }
        } else {
            if pane_is_id {
                let _ = tx.send(CtrlReq::FocusPaneTemp(pid));
            } else {
                let _ = tx.send(CtrlReq::FocusPaneByIndexTemp(pid));
            }
        }
    }
}
match cmd {
    "new-window" | "neww" => {
        let name: Option<String> = args.windows(2).find(|w| w[0] == "-n").map(|w| w[1].trim_matches('"').to_string());
        let start_dir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
        let detached = args.iter().any(|a| *a == "-d");
        let print_info = args.iter().any(|a| *a == "-P");
        let format_str: Option<String> = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
        let cmd_str: Option<String> = args.iter()
            .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-n" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)))
            .map(|s| s.trim_matches('"').to_string());
        if print_info {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::NewWindowPrint(cmd_str, name, detached, start_dir, format_str, rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        } else {
            let _ = tx.send(CtrlReq::NewWindow(cmd_str, name, detached, start_dir));
        }
    }
    "split-window" | "splitw" => {
        let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
        let detached = args.iter().any(|a| *a == "-d");
        let print_info = args.iter().any(|a| *a == "-P");
        let format_str: Option<String> = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
        let start_dir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
        // -p N = percentage, -l N = cell count, -l N% = percentage (tmux semantics)
        let split_size: Option<(u16, bool)> = args.windows(2).find(|w| w[0] == "-p")
            .and_then(|w| w[1].trim_matches('%').parse::<u16>().ok())
            .map(|v| (v, true))
            .or_else(|| args.windows(2).find(|w| w[0] == "-l")
                .and_then(|w| {
                    let raw = &w[1];
                    let is_pct = raw.ends_with('%');
                    raw.trim_end_matches('%').parse::<u16>().ok().map(|v| (v, is_pct))
                }));
        let cmd_str: Option<String> = args.iter()
            .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-p" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-l" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)))
            .map(|s| s.trim_matches('"').to_string());
        if print_info {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, split_size, format_str, rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, split_size, rtx));
            if let Ok(err_msg) = rrx.recv_timeout(Duration::from_millis(2000)) {
                if !err_msg.is_empty() {
                    let _ = write!(write_stream, "{}\n", err_msg);
                    let _ = write_stream.flush();
                }
            }
        }
    }
    "kill-pane" | "killp" => {
        if let Some(pid) = targeted_kill_pane_id {
            let _ = tx.send(CtrlReq::KillPaneById(pid));
        } else {
            let _ = tx.send(CtrlReq::KillPane);
        }
    }
    "capture-pane" | "capturep" => {
        let print_stdout = args.iter().any(|a| *a == "-p");
        let join_lines = args.iter().any(|a| *a == "-J");
        let escape_seqs = args.iter().any(|a| *a == "-e");
        // Parse -S start and -E end (negative = scrollback offset, - = entire scrollback)
        let s_arg = args.windows(2).find(|w| w[0] == "-S").map(|w| w[1]);
        let e_arg = args.windows(2).find(|w| w[0] == "-E").map(|w| w[1]);
        let start: Option<i32> = match s_arg {
            Some("-") => Some(0), // entire scrollback start
            Some(v) => v.parse::<i32>().ok(),
            None => None,
        };
        let end: Option<i32> = match e_arg {
            Some("-") => None, // to end of visible
            Some(v) => v.parse::<i32>().ok(),
            None => None,
        };
        let (rtx, rrx) = mpsc::channel::<String>();
        if escape_seqs {
            let _ = tx.send(CtrlReq::CapturePaneStyled(rtx, start, end));
        } else if s_arg.is_some() || e_arg.is_some() {
            let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start, end));
        } else {
            let _ = tx.send(CtrlReq::CapturePane(rtx));
        }
        if let Ok(mut text) = rrx.recv() {
            if join_lines {
                // Remove trailing whitespace from each line (join wrapped lines)
                text = text.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
            }
            if print_stdout {
                // Write text directly — it already ends with \n from capture
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("capture-pane".to_string(), text));
                } else {
                    let _ = write_stream.write_all(text.as_bytes());
                    let _ = write_stream.flush();
                }
                if !persistent { break; }
            } else {
                let _ = tx.send(CtrlReq::SetBuffer(text));
            }
        }
    }
    "dump-layout" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::DumpLayout(rtx));
        if let Ok(text) = rrx.recv() { 
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("dump-layout".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); 
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "dump-state" | "dump" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::DumpState(rtx, persistent));
        if let Some(ref rtx_bg) = resp_tx_opt {
            // Persistent mode: hand off to writer thread (non-blocking).
            // This lets the read loop keep processing keys immediately.
            let _ = rtx_bg.send(rrx);
        } else {
            // One-shot mode: block and respond inline
            if let Ok(text) = rrx.recv() { 
                let _ = write!(write_stream, "{}\n", text); 
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        }
    }
    "send-text" => {
        if let Some(payload) = args.get(0) { let _ = tx.send(CtrlReq::SendText(payload.to_string())); }
    }
    "send-paste" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = tx.send(CtrlReq::SendPaste(decoded));
            }
        }
    }
    "send-key" => {
        if let Some(payload) = args.get(0) { let _ = tx.send(CtrlReq::SendKey(payload.to_string())); }
    }
    "zoom-pane" | "resize-pane" | "resizep" if args.iter().any(|a| *a == "-Z") => { let _ = tx.send(CtrlReq::ZoomPane); }
    "zoom-pane" => { let _ = tx.send(CtrlReq::ZoomPane); }
    "prefix-begin" => { let _ = tx.send(CtrlReq::PrefixBegin); }
    "prefix-end" => { let _ = tx.send(CtrlReq::PrefixEnd); }
    "copy-enter" => { let _ = tx.send(CtrlReq::CopyEnter); }
    "copy-move" => {
        if args.len() >= 2 { if let (Ok(dx), Ok(dy)) = (args[0].parse::<i16>(), args[1].parse::<i16>()) { let _ = tx.send(CtrlReq::CopyMove(dx, dy)); } }
    }
    "copy-anchor" => { let _ = tx.send(CtrlReq::CopyAnchor); }
    "rectangle-toggle" => { let _ = tx.send(CtrlReq::CopyRectToggle); }
    "copy-yank" => { let _ = tx.send(CtrlReq::CopyYank); }
    "client-size" => {
        if args.len() >= 2 { if let (Ok(w), Ok(h)) = (args[0].parse::<u16>(), args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::ClientSize(client_id, w, h)); } }
    }
    "focus-pane" => {
        if let Some(pid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = tx.send(CtrlReq::FocusPaneCmd(pid)); }
    }
    "focus-window" => {
        if let Some(wid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = tx.send(CtrlReq::FocusWindowCmd(wid)); }
    }
    "mouse-down" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDown(client_id,x,y)); } }
    }
    "mouse-down-right" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDownRight(client_id,x,y)); } }
    }
    "mouse-down-middle" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDownMiddle(client_id,x,y)); } }
    }
    "mouse-drag" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDrag(client_id,x,y)); } }
    }
    "mouse-up" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUp(client_id,x,y)); } }
    }
    "mouse-up-right" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUpRight(client_id,x,y)); } }
    }
    "mouse-up-middle" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUpMiddle(client_id,x,y)); } }
    }
    "mouse-move" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseMove(client_id,x,y)); } }
    }
    "scroll-up" => {
        let x = args.get(0).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let y = args.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::ScrollUp(client_id, x, y));
    }
    "scroll-down" => {
        let x = args.get(0).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let y = args.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::ScrollDown(client_id, x, y));
    }
    "pane-mouse" => {
        // pane-mouse PANE_ID BUTTON COL ROW M|m
        if args.len() >= 5 {
            if let (Ok(pane_id), Ok(button), Ok(col), Ok(row)) = (
                args[0].parse::<usize>(), args[1].parse::<u8>(),
                args[2].parse::<i16>(), args[3].parse::<i16>()
            ) {
                let press = args[4] != "m";
                let _ = tx.send(CtrlReq::PaneMouse(client_id, pane_id, button, col, row, press));
            }
        }
    }
    "pane-scroll" => {
        // pane-scroll PANE_ID up|down
        if args.len() >= 2 {
            if let Ok(pane_id) = args[0].parse::<usize>() {
                let up = args[1] == "up";
                let _ = tx.send(CtrlReq::PaneScroll(client_id, pane_id, up));
            }
        }
    }
    "split-sizes" => {
        // split-sizes PATH SIZE1,SIZE2,...  (PATH is "_" for root, or dot-separated indices)
        if args.len() >= 2 {
            let path: Vec<usize> = if args[0] == "_" {
                Vec::new()
            } else {
                args[0].split('.').filter_map(|s| s.parse().ok()).collect()
            };
            let sizes: Vec<u16> = args[1].split(',').filter_map(|s| s.parse().ok()).collect();
            if sizes.len() >= 2 {
                let _ = tx.send(CtrlReq::SplitSetSizes(client_id, path, sizes));
            }
        }
    }
    "split-resize-done" => {
        let _ = tx.send(CtrlReq::SplitResizeDone(client_id));
    }
    "next-window" | "next" => { let _ = tx.send(CtrlReq::NextWindow); }
    "previous-window" | "prev" => { let _ = tx.send(CtrlReq::PrevWindow); }
    "rename-window" | "renamew" => { if let Some(name) = args.get(0) { let _ = tx.send(CtrlReq::RenameWindow((*name).to_string())); } }
    "list-windows" | "lsw" => {
        // Extract -F format if provided
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindowsFormat(rtx, fmt_str));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        } else if args.iter().any(|a| *a == "-J") {
            // JSON output for programmatic use
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindows(rtx));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        } else {
            // tmux-compatible text output (default)
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        }
        if !persistent { break; }
    }
    "list-tree" => { let (rtx, rrx) = mpsc::channel::<String>(); let _ = tx.send(CtrlReq::ListTree(rtx)); if let Ok(text) = rrx.recv() { if persistent { let _ = tx.send(CtrlReq::ShowTextPopup("list-tree".to_string(), text)); } else { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); } } if !persistent { break; } }
    "toggle-sync" => { let _ = tx.send(CtrlReq::ToggleSync); }
    "set-pane-title" => { let title = args.join(" "); let _ = tx.send(CtrlReq::SetPaneTitle(title)); }
    "send-keys" => {
        let literal = args.iter().any(|a| *a == "-l");
        let paste_mode = args.iter().any(|a| *a == "-p");
        let has_x = args.iter().any(|a| *a == "-X");
        // Parse -N <count> for repeat
        let mut repeat_count: usize = 1;
        if let Some(n_pos) = args.iter().position(|a| *a == "-N") {
            if let Some(count_str) = args.get(n_pos + 1) {
                repeat_count = count_str.parse::<usize>().unwrap_or(1).max(1);
            }
        }
        if has_x {
            // send-keys -X copy-mode-command
            let cmd_parts: Vec<&str> = args.iter().filter(|a| **a != "-X" && !a.starts_with('-')).copied().collect();
            for _ in 0..repeat_count {
                let _ = tx.send(CtrlReq::SendKeysX(cmd_parts.join(" ")));
            }
        } else {
            let keys: Vec<&str> = args.iter()
                .enumerate()
                .filter(|(i, a)| {
                    !a.starts_with('-') && **a != "-l" && **a != "-t"
                    // Skip the argument to -N
                    && !(i > &0 && args.get(i - 1).map_or(false, |prev| *prev == "-N"))
                })
                .map(|(_, a)| *a)
                .collect();
            for _ in 0..repeat_count {
                if paste_mode {
                    let _ = tx.send(CtrlReq::SendPaste(keys.join(" ")));
                } else {
                    let _ = tx.send(CtrlReq::SendKeys(keys.join(" "), literal));
                }
            }
        }
    }
    "select-pane" | "selectp" => {
        // Detect relative pane targets: -t :.+  or  -t :.-
        let is_next_pane = raw_target.as_deref().map_or(false, |t| t.contains(".+") || t == "+" || t == ":.+");
        let is_prev_pane = raw_target.as_deref().map_or(false, |t| t.contains(".-") || t == "-" || t == ":.-");
        let dir = if is_next_pane { "next" }
            else if is_prev_pane { "prev" }
            else if args.iter().any(|a| *a == "-U") { "U" }
            else if args.iter().any(|a| *a == "-D") { "D" }
            else if args.iter().any(|a| *a == "-L") { "L" }
            else if args.iter().any(|a| *a == "-R") { "R" }
            else if args.iter().any(|a| *a == "-l") { "last" }
            else if args.iter().any(|a| *a == "-m") { "mark" }
            else if args.iter().any(|a| *a == "-M") { "unmark" }
            else if args.iter().any(|a| *a == "-e") { "enable-input" }
            else if args.iter().any(|a| *a == "-d") { "disable-input" }
            else { "" };
        // Check for -T title
        let title = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string());
        if let Some(t) = title {
            let _ = tx.send(CtrlReq::SetPaneTitle(t));
        }
        // Handle -P style (per-pane style, e.g. "bg=default,fg=blue")
        // Claude Code uses this for agent pane coloring. Store silently
        // even if rendering doesn't support it yet.
        let pane_style = args.windows(2).find(|w| w[0] == "-P").map(|w| w[1].to_string());
        if let Some(style) = pane_style {
            let _ = tx.send(CtrlReq::SetPaneStyle(style));
        }
        if !dir.is_empty() {
            let _ = tx.send(CtrlReq::SelectPane(dir.to_string()));
        }
    }
    "select-window" | "selectw" => {
        let idx = args.iter().find(|a| !a.starts_with('-')).and_then(|s| s.parse::<usize>().ok())
            .or(target_win);
        if let Some(idx) = idx {
            let _ = tx.send(CtrlReq::SelectWindow(idx));
        }
        if args.iter().any(|a| *a == "-l") {
            let _ = tx.send(CtrlReq::LastWindow);
        }
        if args.iter().any(|a| *a == "-n") {
            let _ = tx.send(CtrlReq::NextWindow);
        }
        if args.iter().any(|a| *a == "-p") {
            let _ = tx.send(CtrlReq::PrevWindow);
        }
    }
    "list-panes" | "lsp" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        // tmux: -a = all panes across all sessions, -s = all panes in target session
        // psmux uses per-session servers, so -s is equivalent to listing the current
        // session's panes (same as no flag). -a lists all panes in this server too
        // since there's only one session per server.
        let all = args.iter().any(|a| *a == "-a");
        let session_scope = args.iter().any(|a| *a == "-s");
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            if all || session_scope {
                let _ = tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt_str));
            } else {
                let _ = tx.send(CtrlReq::ListPanesFormat(rtx, fmt_str));
            }
        } else {
            if all {
                let _ = tx.send(CtrlReq::ListAllPanes(rtx));
            } else if session_scope {
                // -s: list all panes in the targeted session (all windows)
                let _ = tx.send(CtrlReq::ListAllPanes(rtx));
            } else {
                let _ = tx.send(CtrlReq::ListPanes(rtx));
            }
        }
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-panes".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "kill-window" | "killw" => { let _ = tx.send(CtrlReq::KillWindow); }
    "kill-session" | "kill-ses" => {
        // If -t <target> is given, kill that session instead of self
        if let Some(ref tgt) = raw_target {
            let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
            let port_path = format!("{}\\.psmux\\{}.port", home, tgt);
            if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    let key = crate::session::read_session_key(tgt).unwrap_or_default();
                    let _ = crate::session::send_control_to_port(port, "kill-session\n", &key);
                }
            }
        } else {
            let _ = tx.send(CtrlReq::KillSession);
        }
    }
    "has-session" => {
        let (rtx, rrx) = mpsc::channel::<bool>();
        let _ = tx.send(CtrlReq::HasSession(rtx));
        if let Ok(exists) = rrx.recv() {
            if !exists { std::process::exit(1); }
        }
    }
    "rename-session" | "rename" => {
        if let Some(name) = args.iter().find(|a| !a.starts_with('-')) {
            let _ = tx.send(CtrlReq::RenameSession((*name).to_string()));
        }
    }
    "claim-session" => {
        // Warm-server claim: rename + synchronous response so CLI knows it's done.
        // Usage: claim-session <name> [<client-cwd>]
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(|s| &**s).collect();
        if let Some(name) = non_flag.first().copied() {
            let client_cwd = non_flag.get(1).map(|s| s.to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ClaimSession(name.to_string(), client_cwd, rtx));
            if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(5)) {
                let _ = write!(write_stream, "{}", resp);
                let _ = write_stream.flush();
            }
        }
    }
    "swap-pane" | "swapp" => {
        let dir = if args.iter().any(|a| *a == "-U") { "U" }
            else if args.iter().any(|a| *a == "-D") { "D" }
            else { "D" };
        let _ = tx.send(CtrlReq::SwapPane(dir.to_string()));
    }
    "resize-pane" | "resizep" => {
        // Check for zoom toggle first (issue #35)
        if args.iter().any(|a| *a == "-Z") {
            let _ = tx.send(CtrlReq::ZoomPane);
        } else
        // Check for absolute resize (-x N or -y N), supporting both
        // absolute values (e.g. "60") and percentage strings (e.g. "30%").
        if let Some(xval) = args.windows(2).find(|w| w[0] == "-x").map(|w| w[1]) {
            if let Some(pct) = xval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                let _ = tx.send(CtrlReq::ResizePanePercent("x".to_string(), pct));
            } else if let Ok(abs) = xval.parse::<u16>() {
                let _ = tx.send(CtrlReq::ResizePaneAbsolute("x".to_string(), abs));
            }
        } else if let Some(yval) = args.windows(2).find(|w| w[0] == "-y").map(|w| w[1]) {
            if let Some(pct) = yval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                let _ = tx.send(CtrlReq::ResizePanePercent("y".to_string(), pct));
            } else if let Ok(abs) = yval.parse::<u16>() {
                let _ = tx.send(CtrlReq::ResizePaneAbsolute("y".to_string(), abs));
            }
        } else {
            let amount = args.iter().find(|a| a.parse::<u16>().is_ok()).and_then(|s| s.parse::<u16>().ok()).unwrap_or(1);
            let dir = if args.iter().any(|a| *a == "-U") { "U" }
                else if args.iter().any(|a| *a == "-D") { "D" }
                else if args.iter().any(|a| *a == "-L") { "L" }
                else if args.iter().any(|a| *a == "-R") { "R" }
                else { "D" };
            let _ = tx.send(CtrlReq::ResizePane(dir.to_string(), amount));
        }
    }
    "set-buffer" => {
        let content = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
        let _ = tx.send(CtrlReq::SetBuffer(content));
    }
    "paste-buffer" | "pasteb" => {
        let buf_idx: Option<usize> = args.windows(2).find(|w| w[0] == "-b").and_then(|w| w[1].parse().ok());
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(idx) = buf_idx {
            let _ = tx.send(CtrlReq::ShowBufferAt(rtx, idx));
        } else {
            let _ = tx.send(CtrlReq::ShowBuffer(rtx));
        }
        if let Ok(text) = rrx.recv() {
            let _ = tx.send(CtrlReq::SendText(text));
        }
    }
    "list-buffers" | "lsb" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            let _ = tx.send(CtrlReq::ListBuffersFormat(rtx, fmt_str));
        } else {
            let _ = tx.send(CtrlReq::ListBuffers(rtx));
        }
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-buffers".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "show-buffer" | "showb" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowBuffer(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-buffer".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "delete-buffer" => {
        let buf_idx: Option<usize> = args.windows(2).find(|w| w[0] == "-b").and_then(|w| w[1].parse().ok());
        if let Some(idx) = buf_idx {
            let _ = tx.send(CtrlReq::DeleteBufferAt(idx));
        } else {
            let _ = tx.send(CtrlReq::DeleteBuffer);
        }
    }
    "delete-buffer-at" => {
        if let Some(idx_str) = args.get(0) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                let _ = tx.send(CtrlReq::DeleteBufferAt(idx));
            }
        }
    }
    "paste-buffer-at" => {
        if let Some(idx_str) = args.get(0) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                let _ = tx.send(CtrlReq::PasteBufferAt(idx));
            }
        }
    }
    "choose-buffer" | "chooseb" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ChooseBuffer(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("choose-buffer".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "display-message" | "display" => {
        // Parse tmux-like display-message flags without dropping message text.
        let mut print_stdout = false;
        let mut parts: Vec<&str> = Vec::new();
        let mut end_of_opts = false;
        let mut duration_ms: Option<u64> = None;
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if end_of_opts {
                parts.push(a);
                i += 1;
                continue;
            }
            match a {
                "--" => { end_of_opts = true; }
                "-p" => { print_stdout = true; }
                "-F" => { /* format mode */ }
                "-d" => {
                    if i + 1 < args.len() {
                        duration_ms = args[i + 1].parse::<u64>().ok();
                    }
                    i += 1;
                }
                "-I" => { i += 1; }
                _ if a.starts_with('-') => { parts.push(a); }
                _ => parts.push(a),
            }
            i += 1;
        }

        let fmt = if parts.is_empty() {
            crate::commands::DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
        } else {
            parts.join(" ")
        };
        // Pass target pane index for PANE_POS_OVERRIDE (#113).
        let target_pane_idx: Option<usize> = if !pane_is_id { target_pane } else { None };
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_stdout, duration_ms));
        if let Ok(text) = rrx.recv() {
            if print_stdout {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("display-message".to_string(), text));
                } else {
                    let _ = writeln!(write_stream, "{}", text);
                    let _ = write_stream.flush();
                }
            }
        }
        if !persistent { break; }
    }
    "last-window" | "last" => { let _ = tx.send(CtrlReq::LastWindow); }
    "last-pane" | "lastp" => { let _ = tx.send(CtrlReq::LastPane); }
    "rotate-window" | "rotatew" => {
        let reverse = args.iter().any(|a| *a == "-D");
        let _ = tx.send(CtrlReq::RotateWindow(reverse));
    }
    "display-panes" | "displayp" => { let _ = tx.send(CtrlReq::DisplayPanes); }
    "break-pane" | "breakp" => { let _ = tx.send(CtrlReq::BreakPane); }
    "join-pane" | "joinp" | "move-pane" | "movep" => {
        // Parse -s source and -h/-v direction.
        // -t target is already parsed by the global -t handler above into target_win / target_pane.
        let horizontal = args.iter().any(|a| *a == "-h");
        // Parse -s source (session:window.pane format)
        let mut src_win: Option<usize> = None;
        let mut src_pane: Option<usize> = None;
        {
            let mut si = 0;
            while si < args.len() {
                if args[si] == "-s" {
                    if let Some(sv) = args.get(si + 1) {
                        let pt = parse_target(sv);
                        src_win = pt.window;
                        src_pane = pt.pane;
                    }
                    si += 2; continue;
                }
                si += 1;
            }
        }
        // If no -s given, try bare integer as target window (legacy compat)
        let tgt_win = target_win.or_else(|| {
            args.iter()
                .find(|a| a.parse::<usize>().is_ok())
                .and_then(|s| s.parse::<usize>().ok())
        });
        // Always send the request (server will use defaults for None fields)
        let _ = tx.send(CtrlReq::JoinPane {
            src_win,
            src_pane,
            target_win: tgt_win,
            target_pane: target_pane,
            horizontal,
        });
    }
    "respawn-pane" | "respawnp" => {
        let workdir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
        let kill = args.iter().any(|a| *a == "-k");
        let _ = tx.send(CtrlReq::RespawnPane(workdir, kill));
    }
    // ── Cross-session pane forwarding commands ──────────────────────
    "pane-forward-extract" => {
        // Usage: pane-forward-extract <win>.<pane>
        let spec = args.first().copied().unwrap_or("0.0");
        let pt = parse_target(spec);
        let win = pt.window.unwrap_or(0);
        let pane = pt.pane.unwrap_or(0);
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::PaneForwardExtract(win, pane, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_millis(5000)) {
            let _ = write!(write_stream, "{}\n", resp);
            let _ = write_stream.flush();
        } else {
            let _ = write!(write_stream, "ERR timeout\n");
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "pane-forward-inject" => {
        // Usage: pane-forward-inject <src_session> <src_addr> <src_key> <fwd_id> <fwd_port>
        //        <pid> <title> <rows> <cols> <screen_b64_len> [-h] [-t win.pane]
        // Followed by optional screen base64 data on next line.
        if args.len() >= 10 {
            let source_session = args[0].to_string();
            let source_addr = args[1].to_string();
            let source_key = args[2].to_string();
            let forward_id: u64 = args[3].parse().unwrap_or(0);
            let fwd_port: u16 = args[4].parse().unwrap_or(0);
            let pid: u32 = args[5].parse().unwrap_or(0);
            let title = args[6].replace('\x01', " ");
            let rows: u16 = args[7].parse().unwrap_or(24);
            let cols: u16 = args[8].parse().unwrap_or(80);
            let screen_b64_len: usize = args[9].parse().unwrap_or(0);
            let horizontal = args.iter().any(|a| *a == "-h");
            // Read screen base64 data from remaining args/payload
            let screen_b64 = if screen_b64_len > 0 {
                // The base64 data may be appended after the args as a separate read
                let payload: String = args[10..].iter()
                    .filter(|a| **a != "-h" && !a.starts_with("-t"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                if payload.len() >= screen_b64_len {
                    payload[..screen_b64_len].to_string()
                } else {
                    payload
                }
            } else {
                String::new()
            };
            let _ = tx.send(CtrlReq::PaneForwardInject {
                source_session, source_addr, source_key,
                forward_id, fwd_port, pid, title, rows, cols, screen_b64,
                target_win: target_win, target_pane: target_pane, horizontal,
            });
            let _ = write!(write_stream, "OK\n");
            let _ = write_stream.flush();
        } else {
            let _ = write!(write_stream, "ERR not enough args\n");
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "pane-forward-resize" => {
        // Usage: pane-forward-resize <forward_id> <rows> <cols>
        if args.len() >= 3 {
            let fwd_id: u64 = args[0].parse().unwrap_or(0);
            let rows: u16 = args[1].parse().unwrap_or(24);
            let cols: u16 = args[2].parse().unwrap_or(80);
            let _ = tx.send(CtrlReq::PaneForwardResize(fwd_id, rows, cols));
            let _ = write!(write_stream, "OK\n");
        }
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "pane-forward-status" => {
        // Usage: pane-forward-status <forward_id>
        let fwd_id: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::PaneForwardStatus(fwd_id, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_millis(2000)) {
            let _ = write!(write_stream, "{}\n", resp);
        } else {
            let _ = write!(write_stream, "exited\n");
        }
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "pane-forward-kill" => {
        // Usage: pane-forward-kill <forward_id>
        let fwd_id: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::PaneForwardKill(fwd_id));
        let _ = write!(write_stream, "OK\n");
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "session-info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::SessionInfo(rtx));
        if let Ok(line) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("session-info".to_string(), line));
            } else {
                let _ = write!(write_stream, "{}\n", line); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "client-attach" => {
        if !attached_sent {
            let _ = tx.send(CtrlReq::ClientAttach(client_id));
            attached_sent = true;
        }
        if !persistent { let _ = write!(write_stream, "ok\n"); }
    }
    "client-detach" => {
        let _ = tx.send(CtrlReq::ClientDetach(client_id));
        attached_sent = false;
        if !persistent { let _ = write!(write_stream, "ok\n"); }
    }
    "bind-key" | "bind" => {
        let mut table = "prefix".to_string();
        let mut repeatable = false;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-T" if i + 1 < args.len() => {
                    table = args[i + 1].to_string();
                    i += 2; continue;
                }
                "-n" => { table = "root".to_string(); i += 1; continue; }
                "-r" => { repeatable = true; i += 1; continue; }
                _ => break,
            }
        }
        if i < args.len() && i + 1 < args.len() {
            let key = args[i].to_string();
            let command = args[i + 1..].join(" ");
            let _ = tx.send(CtrlReq::BindKey(table, key, command, repeatable));
        }
    }
    "unbind-key" | "unbind" => {
        if args.iter().any(|a| *a == "-a" || (a.starts_with('-') && a.contains('a'))) {
            // Check if -T or -n was explicitly specified
            let mut has_table = false;
            let mut table = String::new();
            for (j, a) in args.iter().enumerate() {
                if *a == "-T" { if let Some(t) = args.get(j + 1) { table = t.to_string(); has_table = true; } }
                if *a == "-n" { table = "root".to_string(); has_table = true; }
            }
            if has_table {
                let _ = tx.send(CtrlReq::UnbindAllInTable(table));
            } else {
                let _ = tx.send(CtrlReq::UnbindAll);
            }
        } else {
            // Parse -n / -T flags for table-specific individual unbind
            let mut table: Option<String> = None;
            let mut t_value_idx: Option<usize> = None;
            let mut target_session_idx: Option<usize> = None;
            for (j, a) in args.iter().enumerate() {
                if *a == "-T" {
                    if let Some(t) = args.get(j + 1) {
                        table = Some(t.to_string());
                        t_value_idx = Some(j + 1);
                    }
                }
                if *a == "-n" { table = Some("root".to_string()); }
                // -t <session> is the target flag; skip its value
                if *a == "-t" { target_session_idx = Some(j + 1); }
            }
            // Find the key argument: first non-flag arg that isn't the -T table value
            // or the -t session target value
            let key_arg = args.iter().enumerate()
                .filter(|(i, a)| !a.starts_with('-') && Some(*i) != t_value_idx && Some(*i) != target_session_idx)
                .map(|(_, a)| *a)
                .next();
            if let Some(key) = key_arg {
                let _ = tx.send(CtrlReq::UnbindKey(key.to_string(), table));
            }
        }
    }
    "list-keys" | "lsk" => {
        // Parse -T <table> for filtering by key table
        let table_filter = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string());
        // Remaining non-flag args are optional key filter
        let key_filter: Option<String> = args.iter()
            .enumerate()
            .filter(|(i, a)| {
                !a.starts_with('-')
                && !(i > &0 && args.get(i - 1).map_or(false, |prev| *prev == "-T"))
            })
            .map(|(_, a)| a.to_string())
            .next();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListKeys(rtx));
        if let Ok(text) = rrx.recv() {
            let filtered = if table_filter.is_some() || key_filter.is_some() {
                text.lines().filter(|line| {
                    if let Some(ref tbl) = table_filter {
                        // list-keys output format: "bind-key -T <table> <key> <command>"
                        let parts: Vec<&str> = line.splitn(5, ' ').collect();
                        if parts.len() >= 3 {
                            if parts[2] != tbl.as_str() {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    if let Some(ref key) = key_filter {
                        // Filter by key name (4th field)
                        let parts: Vec<&str> = line.splitn(5, ' ').collect();
                        if parts.len() >= 4 {
                            if parts[3] != key.as_str() {
                                return false;
                            }
                        }
                    }
                    true
                }).collect::<Vec<&str>>().join("\n")
            } else {
                text
            };
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-keys".to_string(), filtered));
            } else {
                let _ = write!(write_stream, "{}\n", filtered); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "set-option" | "set" | "set-window-option" | "setw" => {
        // Support combined flag tokens like -ga, -gu, -gq (tmux compat)
        let combined_has_set = |ch: char| -> bool {
            args.iter().any(|a| {
                if *a == format!("-{}", ch) { return true; }
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
            })
        };
        let has_u = combined_has_set('u');
        let has_a = combined_has_set('a');
        let has_q = combined_has_set('q');
        let has_o = combined_has_set('o');
        // Skip -t TARGET value (TARGET is not a positional option/value)
        let t_targets: std::collections::HashSet<&str> = args.windows(2)
            .filter(|w| w[0] == "-t" || w[0] == "-p" || w[0] == "-w")
            .map(|w| w[1]).collect();
        let non_flag_args: Vec<&str> = args.iter()
            .filter(|a| (!a.starts_with('-') || a.starts_with('@')) && !t_targets.contains(*a))
            .copied().collect();
        if has_u {
            if let Some(option) = non_flag_args.first() {
                let _ = tx.send(CtrlReq::SetOptionUnset(option.to_string()));
            }
        } else if non_flag_args.len() >= 2 {
            let option = non_flag_args[0].to_string();
            let value = non_flag_args[1..].join(" ");
            if has_a {
                let _ = tx.send(CtrlReq::SetOptionAppend(option, value));
            } else if has_o {
                let _ = tx.send(CtrlReq::SetOptionOnlyIfUnset(option, value));
            } else {
                let _ = tx.send(CtrlReq::SetOptionQuiet(option, value, has_q));
            }
        } else if non_flag_args.len() == 1 && has_q {
            // set -q <option> with no value — silently ignore
        }
    }
    "show-options" | "show" | "show-window-options" | "showw" => {
        // Support combined flag tokens like -gv, -wv, -Av (tmux compat)
        let combined_has = |ch: char| -> bool {
            args.iter().any(|a| {
                if *a == format!("-{}", ch) { return true; }
                // Check combined tokens like -gv, -wvs, etc.
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
            })
        };
        let has_a = combined_has('A');
        let _has_s = combined_has('s');
        let has_w = combined_has('w');
        let window_scope = matches!(cmd, "show-window-options" | "showw") || has_w;
        let has_v = combined_has('v');
        let has_q = combined_has('q');
        let opt_name: Option<&str> = args.iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .last();
        if has_v && opt_name.is_some() || (opt_name.is_some() && !has_q) {
            // Single-option query: show-options -v <name> or show <name>
            if let Some(name) = opt_name {
                let (rtx, rrx) = mpsc::channel::<String>();
                if window_scope {
                    let _ = tx.send(CtrlReq::ShowWindowOptionValue(rtx, name.to_string()));
                } else {
                    let _ = tx.send(CtrlReq::ShowOptionValue(rtx, name.to_string()));
                }
                if let Ok(text) = rrx.recv() {
                    let resolved = if text.is_empty() && window_scope && has_a {
                        let (frtx, frrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowOptionValue(frtx, name.to_string()));
                        frrx.recv().unwrap_or_default()
                    } else {
                        text
                    };
                    if !(has_q && resolved.is_empty()) {
                        let output = if has_v {
                            format!("{}\n", resolved)
                        } else {
                            format!("{} {}\n", name, resolved)
                        };
                        if persistent {
                            let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), output));
                        } else {
                            let _ = write_stream.write_all(output.as_bytes());
                            let _ = write_stream.flush();
                        }
                    }
                }
            }
        } else if has_v && opt_name.is_none() {
            // -v without option name: list all options, values only
            let (rtx, rrx) = mpsc::channel::<String>();
            if window_scope {
                let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
            } else {
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
            }
            if let Ok(text) = rrx.recv() {
                // Extract values only (each line is "option_name value")
                let values_only: String = text.lines()
                    .filter_map(|line| {
                        let trimmed = line.trim();
                        if trimmed.is_empty() { return None; }
                        // Split at first space: name value
                        if let Some(pos) = trimmed.find(' ') {
                            Some(&trimmed[pos + 1..])
                        } else {
                            Some(trimmed) // option with no value
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let output = if values_only.is_empty() { String::new() } else { format!("{}\n", values_only) };
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), output));
                } else {
                    let _ = write_stream.write_all(output.as_bytes());
                    let _ = write_stream.flush();
                }
            }
        } else {
            if window_scope {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
                if let Ok(mut text) = rrx.recv() {
                    if has_a {
                        let (srtx, srrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowOptions(srtx));
                        if let Ok(session_text) = srrx.recv() {
                            if !text.ends_with('\n') && !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&session_text);
                        }
                    }
                    if persistent {
                        let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), text));
                    } else {
                        let _ = write!(write_stream, "{}\n", text);
                        let _ = write_stream.flush();
                    }
                }
            } else {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
                if let Ok(text) = rrx.recv() {
                    if persistent {
                        let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), text));
                    } else {
                        let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                    }
                }
            }
        }
        if !persistent { break; }
    }
    "source-file" | "source" => {
        let format_expand = args.iter().any(|a| *a == "-F");
        let parse_only = args.iter().any(|a| *a == "-n");
        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if !parse_only {
            if let Some(path) = non_flag_args.first() {
                let source_spec = if format_expand {
                    format!("-F {}", path)
                } else {
                    path.to_string()
                };
                let _ = tx.send(CtrlReq::SourceFile(source_spec));
            }
        }
    }
    "move-window" | "movew" => {
        let target = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok());
        let _ = tx.send(CtrlReq::MoveWindow(target));
    }
    "swap-window" | "swapw" => {
        if let Some(target) = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok()) {
            let _ = tx.send(CtrlReq::SwapWindow(target));
        }
    }
    "link-window" | "linkw" => {
        // Parse -s source_window and -t target_index
        let src_idx = args.windows(2).find(|w| w[0] == "-s")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        let dst_idx = args.windows(2).find(|w| w[0] == "-t")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        let _ = tx.send(CtrlReq::LinkWindow(src_idx, dst_idx));
    }
    "unlink-window" | "unlinkw" => {
        let _ = tx.send(CtrlReq::UnlinkWindow);
    }
    "find-window" | "findw" => {
        let pattern = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::FindWindow(rtx, pattern));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("find-window".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "pipe-pane" | "pipep" => {
        let stdin_flag = args.iter().any(|a| *a == "-I");
        let stdout_flag = args.iter().any(|a| *a == "-O");
        let toggle = args.iter().any(|a| *a == "-o");
        let cmd = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
        let (stdin, stdout) = if !stdin_flag && !stdout_flag {
            (false, true)
        } else {
            (stdin_flag, stdout_flag)
        };
        let _ = tx.send(CtrlReq::PipePane(cmd, stdin, stdout, toggle));
    }
    "select-layout" | "selectl" => {
        let layout = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"tiled").to_string();
        let _ = tx.send(CtrlReq::SelectLayout(layout));
    }
    "next-layout" | "nextl" => {
        let _ = tx.send(CtrlReq::NextLayout);
    }
    "list-clients" | "lsc" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            let _ = tx.send(CtrlReq::ListClientsFormat(rtx, fmt_str));
        } else {
            let _ = tx.send(CtrlReq::ListClients(rtx));
        }
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-clients".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "switch-client" | "switchc" => {
        let has_big_t = args.windows(2).any(|w| w[0] == "-T");
        if has_big_t {
            let table = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string()).unwrap_or_default();
            let _ = tx.send(CtrlReq::SwitchClientTable(table));
        } else if args.contains(&"-n") {
            let _ = tx.send(CtrlReq::SwitchClient(String::new(), 'n'));
        } else if args.contains(&"-p") {
            let _ = tx.send(CtrlReq::SwitchClient(String::new(), 'p'));
        } else if args.contains(&"-l") {
            let _ = tx.send(CtrlReq::SwitchClient(String::new(), 'l'));
        } else {
            // -t <target> was already extracted into raw_target by the global -t parser.
            // Use raw_target which holds the original -t value (session name, not window id).
            let target = raw_target.clone().unwrap_or_default();
            // Strip any window/pane suffix (e.g. "session:window.pane" -> "session")
            let session_target = if let Some(pos) = target.find(':') {
                target[..pos].to_string()
            } else {
                target
            };
            let _ = tx.send(CtrlReq::SwitchClient(session_target, 't'));
        }
    }
    "lock-client" | "lockc" => {
        let _ = tx.send(CtrlReq::LockClient);
    }
    "refresh-client" | "refresh" => {
        let _ = tx.send(CtrlReq::RefreshClient);
    }
    "suspend-client" | "suspendc" => {
        let _ = tx.send(CtrlReq::SuspendClient);
    }
    "copy-mode-page-up" => {
        let _ = tx.send(CtrlReq::CopyModePageUp);
    }
    "clear-history" | "clearhist" => {
        let _ = tx.send(CtrlReq::ClearHistory);
    }
    "save-buffer" | "saveb" => {
        let path = args.iter().find(|a| **a == "-" || !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::SaveBuffer(path));
    }
    "load-buffer" | "loadb" => {
        let path = args.iter().find(|a| **a == "-" || !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::LoadBuffer(path));
    }
    "set-environment" | "setenv" => {
        let has_u = args.iter().any(|a| {
            if *a == "-u" { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('u')
        });
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_u {
            if let Some(key) = non_flag.first() {
                let _ = tx.send(CtrlReq::UnsetEnvironment(key.to_string()));
            }
        } else if non_flag.len() >= 2 {
            let _ = tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), non_flag[1].to_string()));
        } else if non_flag.len() == 1 {
            let _ = tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), String::new()));
        }
    }
    "show-environment" | "showenv" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowEnvironment(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-environment".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "set-hook" => {
        let has_unset = args.iter().any(|a| *a == "-u" || *a == "-gu" || *a == "-ug");
        let has_append = args.iter().any(|a| *a == "-a" || *a == "-ga" || *a == "-ag");
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_unset {
            // set-hook -gu <hook-name>  →  remove the hook
            if let Some(name) = non_flag.first() {
                let _ = tx.send(CtrlReq::RemoveHook(name.to_string()));
            }
        } else if non_flag.len() >= 2 {
            // Extract hook command from raw line to preserve quoting
            // (join of parsed tokens loses quotes around paths with spaces)
            let hook_name = non_flag[0];
            let hook_cmd = if let Some(pos) = line.find(hook_name) {
                line[pos + hook_name.len()..].trim().to_string()
            } else {
                non_flag[1..].join(" ")
            };
            if has_append {
                let _ = tx.send(CtrlReq::AppendHook(hook_name.to_string(), hook_cmd));
            } else {
                let _ = tx.send(CtrlReq::SetHook(hook_name.to_string(), hook_cmd));
            }
        }
    }
    "show-hooks" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowHooks(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-hooks".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "wait-for" => {
        let lock = args.iter().any(|a| *a == "-L");
        let signal = args.iter().any(|a| *a == "-S");
        let unlock = args.iter().any(|a| *a == "-U");
        let channel = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let op = if lock { WaitForOp::Lock }
            else if signal { WaitForOp::Signal }
            else if unlock { WaitForOp::Unlock }
            else { WaitForOp::Wait };
        let _ = tx.send(CtrlReq::WaitFor(channel, op));
    }
    "display-menu" | "menu" => {
        let mut x_pos: Option<i16> = None;
        let mut y_pos: Option<i16> = None;
        let mut title = String::new();
        let mut skip_indices = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-x" => { if let Some(v) = args.get(i+1) { x_pos = v.parse().ok(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-y" => { if let Some(v) = args.get(i+1) { y_pos = v.parse().ok(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-T" => { if let Some(v) = args.get(i+1) { title = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                _ => {}
            }
            i += 1;
        }
        // Collect remaining positional args (name, key, command triplets)
        let positional: Vec<&str> = args.iter().enumerate()
            .filter(|(idx, a)| !skip_indices.contains(idx) && !a.starts_with('-'))
            .map(|(_, a)| *a).collect();
        // Build menu from triplets
        let mut menu = crate::types::Menu { title, items: Vec::new(), selected: 0, x: x_pos, y: y_pos };
        let mut pi = 0;
        while pi < positional.len() {
            let name = positional[pi];
            if name.is_empty() || name == "-" {
                menu.items.push(crate::types::MenuItem { name: String::new(), key: None, command: String::new(), is_separator: true });
                pi += 1;
            } else {
                let key = positional.get(pi + 1).and_then(|k| k.chars().next());
                let command = positional.get(pi + 2).map(|c| c.to_string()).unwrap_or_default();
                menu.items.push(crate::types::MenuItem { name: name.to_string(), key, command, is_separator: false });
                pi += 3;
            }
        }
        if !menu.items.is_empty() {
            let _ = tx.send(CtrlReq::DisplayMenuDirect(menu));
        }
    }
    "display-popup" | "popup" => {
        // Default close-on-exit = true (tmux parity: popup closes when command finishes)
        let close_on_exit = !args.iter().any(|a| *a == "-K");
        let mut width_spec = "80".to_string();
        let mut height_spec = "24".to_string();
        let mut start_dir: Option<String> = None;
        let mut skip_indices = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" => { if let Some(v) = args.get(i+1) { width_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-h" => { if let Some(v) = args.get(i+1) { height_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-d" | "-c" => { if let Some(v) = args.get(i+1) { start_dir = Some(v.to_string()); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-E" | "-K" => { skip_indices.insert(i); }
                _ => {}
            }
            i += 1;
        }
        let content = args.iter().enumerate().filter(|(idx, _)| !skip_indices.contains(idx)).map(|(_, a)| *a).collect::<Vec<&str>>().join(" ");
        let _ = tx.send(CtrlReq::DisplayPopup(content, width_spec, height_spec, close_on_exit, start_dir));
    }
    "confirm-before" | "confirm" => {
        let mut prompt: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-p" {
                if let Some(p) = args.get(i+1) { prompt = Some(p.to_string()); i += 1; }
            }
            i += 1;
        }
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-') && Some(&a.to_string()) != prompt.as_ref()).copied().collect();
        let command = non_flag.join(" ");
        let prompt_str = prompt.unwrap_or_else(|| format!("Run '{}'", command));
        let _ = tx.send(CtrlReq::ConfirmBefore(prompt_str, command));
    }
    // tmux standard aliases
    "detach-client" | "detach" => {
        // Check for -t flag targeting a specific client (already parsed into raw_target)
        let target_cid: Option<u64> = raw_target.as_ref()
            .and_then(|t| t.trim_start_matches('%').parse::<u64>().ok());
        if let Some(cid) = target_cid {
            let _ = tx.send(CtrlReq::ForceDetachClient(cid));
        } else {
            let _ = tx.send(CtrlReq::ClientDetach(client_id));
            attached_sent = false;
        }
    }
    "attach-session" | "attach" => {
        if !attached_sent {
            let _ = tx.send(CtrlReq::ClientAttach(client_id));
            attached_sent = true;
        }
    }
    "kill-server" => { let _ = tx.send(CtrlReq::KillServer); }
    "choose-tree" | "choose-window" | "choose-session" => {
        // These are interactive choosers — send a dump that client handles
        // For now, map to listing which the client renders as a chooser
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListTree(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("choose-tree".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "copy-mode" => {
        if args.iter().any(|a| *a == "-u") {
            let _ = tx.send(CtrlReq::CopyEnterPageUp);
        } else {
            let _ = tx.send(CtrlReq::CopyEnter);
        }
    }
    "clock-mode" => { let _ = tx.send(CtrlReq::ClockMode); }
    // Overlay interaction commands (sent by client during active overlays)
    "popup-input" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = tx.send(CtrlReq::PopupInput(decoded.into_bytes()));
            }
        }
    }
    "popup-input-raw" => {
        // Raw bytes (not base64) for single-byte key sequences
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = tx.send(CtrlReq::PopupInput(decoded.into_bytes()));
            }
        }
    }
    "overlay-close" => { let _ = tx.send(CtrlReq::OverlayClose); }
    "display-panes-select" => {
        if let Some(idx) = args.get(0).and_then(|s| s.parse::<usize>().ok()) {
            let _ = tx.send(CtrlReq::DisplayPaneSelect(idx));
        }
    }
    "confirm-respond" => {
        let yes = args.get(0).map(|a| *a == "y" || *a == "yes").unwrap_or(false);
        let _ = tx.send(CtrlReq::ConfirmRespond(yes));
    }
    "menu-select" => {
        if let Some(idx) = args.get(0).and_then(|s| s.parse::<usize>().ok()) {
            let _ = tx.send(CtrlReq::MenuSelect(idx));
        }
    }
    "menu-navigate" => {
        let delta = args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::MenuNavigate(delta));
    }
    "customize-navigate" => {
        let delta = args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::CustomizeNavigate(delta));
    }
    "customize-edit" => {
        let _ = tx.send(CtrlReq::CustomizeEdit);
    }
    "customize-edit-update" => {
        let text = args.join(" ");
        let _ = tx.send(CtrlReq::CustomizeEditUpdate(text));
    }
    "customize-edit-confirm" => {
        let _ = tx.send(CtrlReq::CustomizeEditConfirm);
    }
    "customize-edit-cancel" => {
        let _ = tx.send(CtrlReq::CustomizeEditCancel);
    }
    "customize-reset-default" => {
        let _ = tx.send(CtrlReq::CustomizeResetDefault);
    }
    "customize-filter" => {
        let text = args.join(" ");
        let _ = tx.send(CtrlReq::CustomizeFilter(text));
    }
    "show-messages" | "showmsgs" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowMessages(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-messages".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "command-prompt" => {
        let initial = args.windows(2).find(|w| w[0] == "-I").map(|w| w[1].to_string()).unwrap_or_default();
        let _ = tx.send(CtrlReq::CommandPrompt(initial));
    }
    "run-shell" | "run" => {
        let background = args.iter().any(|a| *a == "-b");
        let cmd_parts: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        let shell_cmd = cmd_parts.join(" ");
        let shell_cmd = shell_cmd.trim_matches(|c: char| c == '\'' || c == '"').to_string();
        // Expand ~ to home directory + XDG fallback for plugin paths
        let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd);
        if shell_cmd.is_empty() {
            if !persistent {
                let _ = write!(write_stream, "usage: run-shell [-b] shell-command\n");
                let _ = write_stream.flush();
            }
        } else {
            if background {
                let mut c = crate::commands::build_run_shell_command(&shell_cmd);
                let _ = c.spawn();
            } else {
                let mut c = crate::commands::build_run_shell_command(&shell_cmd);
                let result = c.output();
                match result {
                    Ok(out) => {
                        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                        let stderr_text = String::from_utf8_lossy(&out.stderr);
                        if !stderr_text.is_empty() {
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            text.push_str(&stderr_text);
                        }
                        if !text.is_empty() {
                            if persistent {
                                let _ = tx.send(CtrlReq::ShowTextPopup("run-shell".to_string(), text));
                            } else {
                                let _ = write!(write_stream, "{}", text);
                                let _ = write_stream.flush();
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("run-shell: {}\n", e);
                        if persistent {
                            let _ = tx.send(CtrlReq::StatusMessage(err_msg));
                        } else {
                            let _ = write!(write_stream, "{}", err_msg);
                            let _ = write_stream.flush();
                        }
                    }
                }
            }
        }
    }
    "if-shell" | "if" => {
        let format_mode = args.iter().any(|a| *a == "-F" || *a == "-bF" || *a == "-Fb");
        // Collect positional args (skip flags like -b, -F, -bF)
        let positional: Vec<&str> = args.iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if positional.len() >= 2 {
            let condition = positional[0];
            let true_cmd = positional[1];
            let false_cmd = positional.get(2).copied();
            let success = if format_mode {
                let (rtx, rrx) = std::sync::mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::DisplayMessage(rtx, condition.to_string(), None, false, None));
                let expanded = rrx.recv().unwrap_or_default();
                !expanded.is_empty() && expanded != "0"
            } else if condition == "true" || condition == "1" {
                true
            } else if condition == "false" || condition == "0" {
                false
            } else {
                // Use resolve_run_shell for consistent shell fallback
                let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
                let mut c = std::process::Command::new(&shell_prog);
                for a in &shell_args { c.arg(a); }
                c.arg(condition);
                c.stdout(std::process::Stdio::null());
                c.stderr(std::process::Stdio::null());
                { use crate::platform::HideWindowCommandExt; c.hide_window(); }
                c.status().map(|s| s.success()).unwrap_or(false)
            };
            let cmd_to_run = if success { Some(true_cmd) } else { false_cmd };
            if let Some(chosen) = cmd_to_run {
                // Feed the chosen command back into the line buffer so the
                // main dispatch loop processes it as a regular command.
                line.clear();
                line.push_str(chosen);
                line.push('\n');
                continue;  // re-enter the dispatch loop with the new command
            }
        }
    }
    "list-sessions" | "ls" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt_str, None, false, None));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-sessions".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SessionInfo(rtx));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-sessions".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        }
        if !persistent { break; }
    }
    "new-session" | "new" => {
        // new-session -t target: set session group on this server
        if let Some(target) = args.windows(2).find(|w| w[0] == "-t").map(|w| w[1].to_string()) {
            let _ = tx.send(CtrlReq::SetSessionGroup(target));
        } else {
            // Issue #200: spawn a new session from inside a running session.
            // Parse flags
            let mut sess_name: Option<String> = None;
            let mut detached = false;
            let mut window_name: Option<String> = None;
            let mut start_dir: Option<String> = None;
            let mut init_width: Option<String> = None;
            let mut init_height: Option<String> = None;
            let mut env_vars: Vec<(String, String)> = Vec::new();
            let mut env_parse_err: Option<String> = None;
            let mut initial_command: Option<String> = None;
            {
                let mut i = 0;
                while i < args.len() {
                    match args[i] {
                        "-s" => { i += 1; if i < args.len() { sess_name = Some(args[i].trim_matches('"').to_string()); } }
                        "-n" => { i += 1; if i < args.len() { window_name = Some(args[i].trim_matches('"').to_string()); } }
                        "-c" => { i += 1; if i < args.len() { start_dir = Some(args[i].trim_matches('"').to_string()); } }
                        "-x" => { i += 1; if i < args.len() { init_width = Some(args[i].to_string()); } }
                        "-y" => { i += 1; if i < args.len() { init_height = Some(args[i].to_string()); } }
                        "-e" => {
                            i += 1;
                            match crate::util::parse_new_session_e_value_token(args.get(i).copied()) {
                                Ok(p) => env_vars.push(p),
                                Err(e) => {
                                    env_parse_err = Some(e);
                                    break;
                                }
                            }
                        }
                        "-d" => { detached = true; }
                        "-t" => { i += 1; /* already handled above */ }
                        "-F" | "-f" => { i += 1; /* skip value */ }
                        other => {
                            // Positional arg: initial shell command (issue #229)
                            if !other.starts_with('-') {
                                initial_command = Some(args[i..].iter().map(|s| s.trim_matches('"').to_string()).collect::<Vec<_>>().join(" "));
                                break;
                            }
                        }
                    }
                    i += 1;
                }
            }

            if let Some(ref err) = env_parse_err {
                let msg = format!("psmux: {}\n", err);
                if persistent {
                    let _ = tx.send(CtrlReq::StatusMessage(msg.trim().to_string()));
                } else {
                    let _ = write!(write_stream, "{}", msg);
                    let _ = write_stream.flush();
                }
                if !persistent { break; }
            } else {

            // Note: socket_name (from -L flag) is not directly available here;
            // the client-side handler in commands.rs has it via app.socket_name.
            let name = sess_name.unwrap_or_else(|| crate::session::next_session_name(None));

            let port_file_base = name.clone();

            let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
            let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);

            // Check if session already exists
            let already_exists = if std::path::Path::new(&port_path).exists() {
                if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                    if let Ok(port) = port_str.trim().parse::<u16>() {
                        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
                        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(100)).is_ok()
                    } else { false }
                } else { false }
            } else { false };

            if already_exists {
                if persistent {
                    let _ = tx.send(CtrlReq::StatusMessage(format!("session '{}' already exists", name)));
                } else {
                    let _ = write!(write_stream, "session '{}' already exists\n", name);
                    let _ = write_stream.flush();
                    break;
                }
            } else {
                // Spawn new server
                let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
                let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), name.clone()];

                if let Some(ref dir) = start_dir {
                    server_args.push("-d".into());
                    server_args.push(dir.clone());
                }
                if let Some(ref wn) = window_name {
                    server_args.push("-n".into());
                    server_args.push(wn.clone());
                }
                // Pass initial command to server (issue #229)
                if let Some(ref cmd) = initial_command {
                    server_args.push("-c".into());
                    server_args.push(cmd.clone());
                }
                // Pass -x/-y initial dimensions to server
                if let Some(ref w) = init_width {
                    server_args.push("-x".into());
                    server_args.push(w.clone());
                }
                if let Some(ref h) = init_height {
                    server_args.push("-y".into());
                    server_args.push(h.clone());
                }
                // Pass -e environment variables to server
                for (k, v) in &env_vars {
                    server_args.push("-e".into());
                    server_args.push(format!("{}={}", k, v));
                }
                #[cfg(windows)]
                { let _ = crate::platform::spawn_server_hidden(&exe, &server_args); }
                #[cfg(not(windows))]
                {
                    let mut cmd_proc = std::process::Command::new(&exe);
                    for a in &server_args { cmd_proc.arg(a); }
                    cmd_proc.stdin(std::process::Stdio::null());
                    cmd_proc.stdout(std::process::Stdio::null());
                    cmd_proc.stderr(std::process::Stdio::null());
                    let _ = cmd_proc.spawn();
                }

                // Wait for port file
                for _ in 0..500 {
                    if std::path::Path::new(&port_path).exists() { break; }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }

                if std::path::Path::new(&port_path).exists() {
                    if !detached {
                        let _ = tx.send(CtrlReq::SwitchClient(name.clone(), 't'));
                    }
                    if persistent {
                        let _ = tx.send(CtrlReq::StatusMessage(format!("created session '{}'", name)));
                    } else {
                        let _ = write!(write_stream, "OK\n");
                        let _ = write_stream.flush();
                    }
                } else {
                    if persistent {
                        let _ = tx.send(CtrlReq::StatusMessage(format!("failed to create session '{}'", name)));
                    } else {
                        let _ = write!(write_stream, "failed to create session '{}'\n", name);
                        let _ = write_stream.flush();
                    }
                }
            }
            } // env_parse_err else
        }
    }
    "list-commands" | "lscm" => {
        let cmds = TMUX_COMMANDS.join("\n");
        if persistent {
            let _ = tx.send(CtrlReq::ShowTextPopup("list-commands".to_string(), cmds));
        } else {
            let _ = write!(write_stream, "{}\n", cmds);
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "server-info" | "info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ServerInfo(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("server-info".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "start-server" => {
        // Server is already running if we're here, no-op
        if !persistent { break; }
    }
    "send-prefix" => {
        let _ = tx.send(CtrlReq::SendPrefix);
    }
    "previous-layout" | "prevl" => {
        let _ = tx.send(CtrlReq::PrevLayout);
    }
    "resize-window" | "resizew" => {
        let abs_x = args.windows(2).find(|w| w[0] == "-x").and_then(|w| w[1].parse::<u16>().ok());
        let abs_y = args.windows(2).find(|w| w[0] == "-y").and_then(|w| w[1].parse::<u16>().ok());
        if let Some(xv) = abs_x {
            let _ = tx.send(CtrlReq::ResizeWindow("x".to_string(), xv));
        } else if let Some(yv) = abs_y {
            let _ = tx.send(CtrlReq::ResizeWindow("y".to_string(), yv));
        }
    }
    "respawn-window" | "respawnw" => {
        let _ = tx.send(CtrlReq::RespawnWindow);
    }
    "lock-server" | "lock-session" | "lock" | "locks" => {
        // Lock is a no-op on Windows (no terminal locking concept)
        // Stub for compatibility
    }
    "focus-in" => { let _ = tx.send(CtrlReq::FocusIn); }
    "focus-out" => { let _ = tx.send(CtrlReq::FocusOut); }
    "choose-client" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListClients(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("choose-client".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}", text);
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "customize-mode" => {
        // tmux 3.2+ customize-mode: interactive options editor
        let _ = tx.send(CtrlReq::CustomizeMode);
    }
    "clear-prompt-history" | "clearphist" => {
        let _ = tx.send(CtrlReq::ClearPromptHistory);
    }
    "show-prompt-history" | "showphist" => {
        let _ = tx.send(CtrlReq::ShowPromptHistory(persistent));
    }
    "server-access" => {
        // Multi-user server access — not applicable to psmux
    }
    "run-command" | "runcmd" => {
        // Route command through the server-side execute_command_string path
        // (same code path as keybindings and command prompt).
        let full_cmd = args.join(" ");
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::RunCommand(full_cmd, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(15)) {
            if persistent {
                let _ = tx.send(CtrlReq::StatusMessage(resp));
            } else {
                let _ = write!(write_stream, "{}\n", resp);
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    _ => {}
}
    // Process pending chained commands before reading from socket
    if !pending_chain.is_empty() {
        line = pending_chain.remove(0);
        continue;
    }
    // Try to read next command for batching (with timeout)
    line.clear();
    match r.read_line(&mut line) {
        Ok(0) => {
            // EOF - client disconnected
            if attached_sent {
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
            }
            break;
        }
        Err(e) => {
            if persistent && (e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut) {
                line.clear(); // Clear any partial data from interrupted read
                continue; // Persistent mode - keep waiting
            }
            if attached_sent {
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
            }
            break; // Non-persistent timeout or real error
        }
        Ok(_) => {} // Continue processing
    }
} // end command loop
}

/// Dispatch a command from a control mode client.
/// Returns true if a response was sent through `resp_tx`, false for fire-and-forget commands.
fn dispatch_control_command(
    cmd: &str,
    args: &[&str],
    tx: &mpsc::Sender<CtrlReq>,
    resp_tx: mpsc::Sender<String>,
    target_pane: Option<usize>,
    pane_is_id: bool,
    _raw_target: Option<&str>,
    client_id: u64,
) -> bool {
    match cmd {
        "list-windows" | "lsw" => {
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt) = format_str {
                let _ = tx.send(CtrlReq::ListWindowsFormat(rtx, fmt));
            } else {
                let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "list-panes" | "lsp" => {
            let all = args.iter().any(|a| *a == "-a");
            let session_scope = args.iter().any(|a| *a == "-s");
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            if all || session_scope {
                if let Some(fmt) = format_str {
                    let _ = tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt));
                } else {
                    let _ = tx.send(CtrlReq::ListAllPanes(rtx));
                }
            } else {
                if let Some(fmt) = format_str {
                    let _ = tx.send(CtrlReq::ListPanesFormat(rtx, fmt));
                } else {
                    let _ = tx.send(CtrlReq::ListPanes(rtx));
                }
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "display-message" | "display" => {
            let print_mode = args.iter().any(|a| *a == "-p");
            let raw_fmt = args.last().map(|s| s.trim_matches('"').to_string()).unwrap_or_default();
            let fmt = if raw_fmt.is_empty() {
                crate::commands::DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
            } else {
                raw_fmt
            };
            let target_pane_idx = if pane_is_id { None } else { target_pane };
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_mode, None));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "new-window" | "neww" => {
            let name = args.windows(2).find(|w| w[0] == "-n").map(|w| w[1].trim_matches('"').to_string());
            let start_dir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
            let detached = args.iter().any(|a| *a == "-d");
            let print_info = args.iter().any(|a| *a == "-P");
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
            let cmd_str: Option<String> = args.iter()
                .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-n" && w[1] == **a))
                    && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a))
                    && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)))
                .map(|s| s.trim_matches('"').to_string());
            if print_info {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::NewWindowPrint(cmd_str, name, detached, start_dir, format_str, rtx));
                if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                    let _ = resp_tx.send(text);
                }
                true
            } else {
                let _ = tx.send(CtrlReq::NewWindow(cmd_str, name, detached, start_dir));
                let _ = resp_tx.send(String::new());
                true
            }
        }
        "split-window" | "splitw" => {
            let kind = if args.iter().any(|a| *a == "-h") {
                LayoutKind::Horizontal
            } else {
                LayoutKind::Vertical
            };
            let cmd_str = args.windows(2).find(|w| w[0] == "-c").map(|_| ()).and(None);
            let start_dir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
            let detached = args.iter().any(|a| *a == "-d");
            let print_info = args.iter().any(|a| *a == "-P");
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
            // -p N = percentage, -l N = cell count, -l N% = percentage (tmux semantics)
            let split_size: Option<(u16, bool)> = args.windows(2).find(|w| w[0] == "-p")
                .and_then(|w| w[1].trim_end_matches('%').parse::<u16>().ok())
                .map(|v| (v, true))
                .or_else(|| args.windows(2).find(|w| w[0] == "-l")
                    .and_then(|w| {
                        let raw = &w[1];
                        let is_pct = raw.ends_with('%');
                        raw.trim_end_matches('%').parse::<u16>().ok().map(|v| (v, is_pct))
                    }));
            let (rtx, rrx) = mpsc::channel::<String>();
            if print_info {
                let _ = tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, split_size, format_str, rtx));
            } else {
                let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, split_size, rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "send-keys" => {
            let literal = args.iter().any(|a| *a == "-l");
            let keys: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            let text = keys.join(" ");
            let _ = tx.send(CtrlReq::SendKeys(text, literal));
            let _ = resp_tx.send(String::new());
            true
        }
        "capture-pane" | "capturep" => {
            let start = args.windows(2).find(|w| w[0] == "-S").and_then(|w| w[1].parse::<i32>().ok());
            let end = args.windows(2).find(|w| w[0] == "-E").and_then(|w| w[1].parse::<i32>().ok());
            let styled = args.iter().any(|a| *a == "-e");
            let (rtx, rrx) = mpsc::channel::<String>();
            if styled {
                let _ = tx.send(CtrlReq::CapturePaneStyled(rtx, start, end));
            } else if start.is_some() || end.is_some() {
                let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start, end));
            } else {
                let _ = tx.send(CtrlReq::CapturePane(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "kill-pane" | "killp" => {
            if pane_is_id {
                if let Some(pid) = target_pane {
                    let _ = tx.send(CtrlReq::KillPaneById(pid));
                }
            } else {
                let _ = tx.send(CtrlReq::KillPane);
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "kill-window" | "killw" => {
            let _ = tx.send(CtrlReq::KillWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "unlink-window" | "unlinkw" => {
            let _ = tx.send(CtrlReq::UnlinkWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "select-window" | "selectw" => {
            // Already handled by target focus above
            let _ = resp_tx.send(String::new());
            true
        }
        "select-pane" | "selectp" => {
            // Handle -T title setting
            if let Some(t) = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].trim_matches('"').to_string()) {
                let _ = tx.send(CtrlReq::SetPaneTitle(t));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "rename-window" | "renamew" => {
            if let Some(name) = args.last() {
                let _ = tx.send(CtrlReq::RenameWindow(name.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "rename-session" | "rename" => {
            if let Some(name) = args.last() {
                let _ = tx.send(CtrlReq::RenameSession(name.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "set-option" | "set" | "set-window-option" | "setw" => {
            // Support combined flag tokens like -ga, -gu, -gq (tmux compat)
            let combined_has_set2 = |ch: char| -> bool {
                args.iter().any(|a| {
                    if *a == format!("-{}", ch) { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
                })
            };
            let quiet = combined_has_set2('q');
            let unset = combined_has_set2('u');
            let append = combined_has_set2('a');
            let global = combined_has_set2('g');
            let only_if_unset = combined_has_set2('o');
            // Skip values that follow flag args (-t TARGET, -p PANE, -w WINDOW)
            let t_vals2: std::collections::HashSet<&str> = args.windows(2)
                .filter(|w| w[0] == "-t" || w[0] == "-p" || w[0] == "-w")
                .map(|w| w[1]).collect();
            let positional: Vec<&str> = args.iter()
                .filter(|a| (!a.starts_with('-') || a.starts_with('@')) && !t_vals2.contains(*a))
                .copied().collect();
            if unset && !positional.is_empty() {
                let _ = tx.send(CtrlReq::SetOptionUnset(positional[0].to_string()));
            } else if positional.len() >= 2 {
                let key = positional[0].to_string();
                let val = positional[1].trim_matches('"').to_string();
                if append {
                    let _ = tx.send(CtrlReq::SetOptionAppend(key, val));
                } else if only_if_unset {
                    let _ = tx.send(CtrlReq::SetOptionOnlyIfUnset(key, val));
                } else if quiet || global {
                    let _ = tx.send(CtrlReq::SetOptionQuiet(key, val, quiet));
                } else {
                    let _ = tx.send(CtrlReq::SetOption(key, val));
                }
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "show-options" | "show" | "show-window-options" | "showw" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let combined_has2 = |ch: char| -> bool {
                args.iter().any(|a| {
                    if *a == format!("-{}", ch) { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
                })
            };
            let value_only = combined_has2('v');
            let window_scope2 = matches!(cmd, "show-window-options" | "showw") || combined_has2('w');
            let opt_name = args.iter().filter(|a| !a.starts_with('-')).next().map(|s| s.to_string());
            let has_opt_name = opt_name.is_some();
            if let Some(name) = opt_name {
                if value_only {
                    let _ = tx.send(CtrlReq::ShowOptionValue(rtx, name));
                } else if window_scope2 {
                    let _ = tx.send(CtrlReq::ShowWindowOptionValue(rtx, name));
                } else {
                    let _ = tx.send(CtrlReq::ShowOptionValue(rtx, name));
                }
            } else if value_only {
                // -v/-gv without option name: list all, values only
                if window_scope2 {
                    let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
                } else {
                    let _ = tx.send(CtrlReq::ShowOptions(rtx));
                }
            } else if window_scope2 {
                let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
            } else {
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                if value_only && !has_opt_name {
                    // Strip option names, keep values only
                    let values_only: String = text.lines()
                        .filter_map(|line| {
                            let t = line.trim();
                            if t.is_empty() { return None; }
                            if let Some(pos) = t.find(' ') { Some(&t[pos + 1..]) } else { Some(t) }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = resp_tx.send(values_only);
                } else {
                    let _ = resp_tx.send(text);
                }
            }
            true
        }
        "list-keys" | "lsk" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListKeys(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "list-sessions" | "ls" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SessionInfo(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "list-buffers" | "lsb" => {
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt) = format_str {
                let _ = tx.send(CtrlReq::ListBuffersFormat(rtx, fmt));
            } else {
                let _ = tx.send(CtrlReq::ListBuffers(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "show-buffer" | "showb" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowBuffer(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "has-session" | "has" => {
            let (rtx, rrx) = mpsc::channel::<bool>();
            let _ = tx.send(CtrlReq::HasSession(rtx));
            if let Ok(exists) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(if exists { String::new() } else { "session not found".to_string() });
            }
            true
        }
        "list-clients" | "lsc" => {
            let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt_str) = fmt {
                let _ = tx.send(CtrlReq::ListClientsFormat(rtx, fmt_str));
            } else {
                let _ = tx.send(CtrlReq::ListClients(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "kill-session" => {
            let _ = tx.send(CtrlReq::KillSession);
            let _ = resp_tx.send(String::new());
            true
        }
        "kill-server" => {
            let _ = tx.send(CtrlReq::KillServer);
            let _ = resp_tx.send(String::new());
            true
        }
        "select-layout" | "selectl" => {
            if let Some(layout) = args.first() {
                let _ = tx.send(CtrlReq::SelectLayout(layout.to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "next-layout" | "nextl" => {
            let _ = tx.send(CtrlReq::NextLayout);
            let _ = resp_tx.send(String::new());
            true
        }
        "resize-pane" | "resizep" => {
            if args.iter().any(|a| *a == "-Z") {
                let _ = tx.send(CtrlReq::ZoomPane);
            } else if let Some(xval) = args.windows(2).find(|w| w[0] == "-x").map(|w| w[1]) {
                if let Some(pct) = xval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                    let _ = tx.send(CtrlReq::ResizePanePercent("x".to_string(), pct));
                } else if let Ok(abs) = xval.parse::<u16>() {
                    let _ = tx.send(CtrlReq::ResizePaneAbsolute("x".to_string(), abs));
                }
            } else if let Some(yval) = args.windows(2).find(|w| w[0] == "-y").map(|w| w[1]) {
                if let Some(pct) = yval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                    let _ = tx.send(CtrlReq::ResizePanePercent("y".to_string(), pct));
                } else if let Ok(abs) = yval.parse::<u16>() {
                    let _ = tx.send(CtrlReq::ResizePaneAbsolute("y".to_string(), abs));
                }
            } else {
                let amount = args.iter().filter(|a| !a.starts_with('-')).next()
                    .and_then(|s| s.parse::<u16>().ok()).unwrap_or(1);
                let dir = if args.iter().any(|a| *a == "-U") { "U" }
                    else if args.iter().any(|a| *a == "-D") { "D" }
                    else if args.iter().any(|a| *a == "-L") { "L" }
                    else if args.iter().any(|a| *a == "-R") { "R" }
                    else { "D" };
                let _ = tx.send(CtrlReq::ResizePane(dir.to_string(), amount));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "swap-pane" | "swapp" => {
            let direction = if args.iter().any(|a| *a == "-U") { "-U".to_string() }
                           else if args.iter().any(|a| *a == "-D") { "-D".to_string() }
                           else { "-D".to_string() };
            let _ = tx.send(CtrlReq::SwapPane(direction));
            let _ = resp_tx.send(String::new());
            true
        }
        "bind-key" | "bind" => {
            // Parse bind-key's own flags, then treat everything after
            // the key name as the verbatim command (preserving flags like -c).
            let mut table_name = "prefix".to_string();
            let mut repeat = false;
            let mut i = 0;
            while i < args.len() {
                match args[i] {
                    "-T" if i + 1 < args.len() => {
                        table_name = args[i + 1].to_string();
                        i += 2; continue;
                    }
                    "-n" => { table_name = "root".to_string(); i += 1; continue; }
                    "-r" => { repeat = true; i += 1; continue; }
                    _ => break,
                }
            }
            if i < args.len() && i + 1 < args.len() {
                let key = args[i].to_string();
                let command = args[i + 1..].join(" ");
                let _ = tx.send(CtrlReq::BindKey(table_name, key, command, repeat));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "unbind-key" | "unbind" => {
            if args.iter().any(|a| *a == "-a" || (a.starts_with('-') && a.contains('a'))) {
                let mut has_table = false;
                let mut table = String::new();
                for (j, a) in args.iter().enumerate() {
                    if *a == "-T" { if let Some(t) = args.get(j + 1) { table = t.to_string(); has_table = true; } }
                    if *a == "-n" { table = "root".to_string(); has_table = true; }
                }
                if has_table {
                    let _ = tx.send(CtrlReq::UnbindAllInTable(table));
                } else {
                    let _ = tx.send(CtrlReq::UnbindAll);
                }
            } else {
                // Parse -n / -T flags for table-specific individual unbind
                let mut table: Option<String> = None;
                let mut t_value_idx: Option<usize> = None;
                let mut target_session_idx: Option<usize> = None;
                for (j, a) in args.iter().enumerate() {
                    if *a == "-T" {
                        if let Some(t) = args.get(j + 1) {
                            table = Some(t.to_string());
                            t_value_idx = Some(j + 1);
                        }
                    }
                    if *a == "-n" { table = Some("root".to_string()); }
                    // -t <session> is the target flag; skip its value
                    if *a == "-t" { target_session_idx = Some(j + 1); }
                }
                // Find the key argument: first non-flag arg that isn't the -T table value
                // or the -t session target value
                let key_arg = args.iter().enumerate()
                    .filter(|(i, a)| !a.starts_with('-') && Some(*i) != t_value_idx && Some(*i) != target_session_idx)
                    .map(|(_, a)| *a)
                    .next();
                if let Some(key) = key_arg {
                    let _ = tx.send(CtrlReq::UnbindKey(key.to_string(), table));
                }
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "source-file" | "source" => {
            if let Some(path) = args.first() {
                let _ = tx.send(CtrlReq::SourceFile(path.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "set-environment" | "setenv" => {
            let unset = args.iter().any(|a| {
                if *a == "-u" { return true; }
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('u')
            });
            let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            if unset && !positional.is_empty() {
                let _ = tx.send(CtrlReq::UnsetEnvironment(positional[0].to_string()));
            } else if positional.len() >= 2 {
                let _ = tx.send(CtrlReq::SetEnvironment(positional[0].to_string(), positional[1].to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "show-environment" | "showenv" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowEnvironment(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "set-hook" => {
            let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            if positional.len() >= 2 {
                let name = positional[0].to_string();
                // Re-quote tokens that contain spaces to preserve paths like "Psmux Plugins"
                let command = positional[1..].iter().map(|s| {
                    if s.contains(' ') { format!("'{}'", s) } else { s.to_string() }
                }).collect::<Vec<_>>().join(" ");
                let has_append = args.iter().any(|a| {
                    if *a == "-a" { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('a')
                });
                if has_append {
                    let _ = tx.send(CtrlReq::AppendHook(name, command));
                } else {
                    let _ = tx.send(CtrlReq::SetHook(name, command));
                }
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "show-hooks" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowHooks(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "server-info" | "info" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ServerInfo(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "list-commands" | "lscm" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListCommands(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "dump-state" | "dump" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DumpState(rtx, false));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "zoom-pane" | "resizep -Z" => {
            let _ = tx.send(CtrlReq::ZoomPane);
            let _ = resp_tx.send(String::new());
            true
        }
        "last-window" | "last" => {
            let _ = tx.send(CtrlReq::LastWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "last-pane" | "lastp" => {
            let _ = tx.send(CtrlReq::LastPane);
            let _ = resp_tx.send(String::new());
            true
        }
        "next-window" | "next" => {
            let _ = tx.send(CtrlReq::NextWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "previous-window" | "prev" => {
            let _ = tx.send(CtrlReq::PrevWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "rotate-window" | "rotatew" => {
            let upward = args.iter().any(|a| *a == "-U");
            let _ = tx.send(CtrlReq::RotateWindow(upward));
            let _ = resp_tx.send(String::new());
            true
        }
        "break-pane" | "breakp" => {
            let _ = tx.send(CtrlReq::BreakPane);
            let _ = resp_tx.send(String::new());
            true
        }
        "respawn-pane" | "respawnp" => {
            let workdir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
            let kill = args.iter().any(|a| *a == "-k");
            let _ = tx.send(CtrlReq::RespawnPane(workdir, kill));
            let _ = resp_tx.send(String::new());
            true
        }
        "wait-for" | "wait" => {
            let op = if args.iter().any(|a| *a == "-L") { WaitForOp::Lock }
                     else if args.iter().any(|a| *a == "-U") { WaitForOp::Unlock }
                     else if args.iter().any(|a| *a == "-S") { WaitForOp::Signal }
                     else { WaitForOp::Wait };
            if let Some(channel) = args.iter().find(|a| !a.starts_with('-')) {
                let _ = tx.send(CtrlReq::WaitFor(channel.to_string(), op));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "refresh-client" | "refresh" => {
            // Parse -B name:what:format (subscription management)
            let mut i = 0;
            while i < args.len() {
                if args[i] == "-B" {
                    if let Some(spec) = args.get(i + 1) {
                        // Format: "name:what:format" or "name:" (remove)
                        let spec = spec.trim_matches('"');
                        if let Some(colon1) = spec.find(':') {
                            let name = spec[..colon1].to_string();
                            let rest = &spec[colon1 + 1..];
                            if rest.is_empty() {
                                // Remove subscription: "name:"
                                let _ = tx.send(CtrlReq::ControlUnsubscribe {
                                    client_id,
                                    name,
                                });
                            } else if let Some(colon2) = rest.find(':') {
                                let target = rest[..colon2].to_string();
                                let format = rest[colon2 + 1..].to_string();
                                let _ = tx.send(CtrlReq::ControlSubscribe {
                                    client_id,
                                    name,
                                    target,
                                    format,
                                });
                            }
                        }
                    }
                    i += 2;
                    continue;
                }
                // Parse -f flags (e.g. pause-after=N)
                if args[i] == "-f" {
                    if let Some(flag_val) = args.get(i + 1) {
                        let flag_val = flag_val.trim_matches('"');
                        if let Some(stripped) = flag_val.strip_prefix("pause-after=") {
                            let secs = stripped.parse::<u64>().ok();
                            let _ = tx.send(CtrlReq::ControlSetPauseAfter {
                                client_id,
                                pause_after_secs: secs,
                            });
                        } else if flag_val == "no-pause" {
                            let _ = tx.send(CtrlReq::ControlSetPauseAfter {
                                client_id,
                                pause_after_secs: None,
                            });
                        }
                    }
                    i += 2;
                    continue;
                }
                // Parse -A '%N:continue' (resume paused pane)
                if args[i] == "-A" {
                    if let Some(spec) = args.get(i + 1) {
                        let spec = spec.trim_matches('"').trim_matches('\'');
                        // Format: %N:continue or %N:pause
                        if let Some(colon) = spec.find(':') {
                            let pane_spec = &spec[..colon];
                            let action = &spec[colon + 1..];
                            if action == "continue" {
                                if let Some(pid_str) = pane_spec.strip_prefix('%') {
                                    if let Ok(pid) = pid_str.parse::<usize>() {
                                        let _ = tx.send(CtrlReq::ControlContinuePane {
                                            client_id,
                                            pane_id: pid,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    i += 2;
                    continue;
                }
                i += 1;
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "run-command" | "runcmd" => {
            let full_cmd = args.join(" ");
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::RunCommand(full_cmd, rtx));
            if let Ok(resp) = rrx.recv_timeout(Duration::from_secs(15)) {
                let _ = resp_tx.send(resp);
            } else {
                let _ = resp_tx.send("timeout".to_string());
            }
            true
        }
        _ => {
            // Unknown command
            let _ = resp_tx.send(format!("unknown command: {}", cmd));
            true
        }
    }
}
