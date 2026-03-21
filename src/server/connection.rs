use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::net::TcpStream;

use crate::types::{CtrlReq, LayoutKind, WaitForOp};
use crate::cli::parse_target;
use crate::util::base64_decode;

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

// Set short read timeout for batched command processing
let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(10)));

// Check for PERSISTENT flag and optional TARGET line
let mut persistent = false;
let mut resp_tx_opt: Option<mpsc::Sender<mpsc::Receiver<String>>> = None;
let mut global_target_win: Option<usize> = None;
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
    crate::types::register_persistent_stream(&write_stream);
    
    // Spawn a dedicated writer thread so the read loop never blocks
    // waiting for dump-state responses.  The read loop sends oneshot
    // receivers here; the writer thread waits for each response and
    // writes it to TCP in order.
    let mut ws_bg = write_stream.try_clone().unwrap();
    let (resp_tx, resp_rx) = mpsc::channel::<mpsc::Receiver<String>>();

    // Register a clone for server-pushed frames (event-driven rendering).
    // The server auto-pushes serialized frames when PTY output arrives,
    // eliminating the need for the client to poll dump-state.
    crate::types::register_frame_sender(resp_tx.clone());

    std::thread::spawn(move || {
        while let Ok(rrx) = resp_rx.recv() {
            if let Ok(text) = rrx.recv() {
                // Break on write errors so the thread exits when the
                // TCP connection drops.  This lets push_frame() prune
                // dead senders via retain() on the next call.
                if write!(ws_bg, "{}\n", text).is_err() { break; }
                if ws_bg.flush().is_err() { break; }
            }
        }
    });
    resp_tx_opt = Some(resp_tx);
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

// Check if this line is a TARGET specification
// Save raw target for relative pane specifiers like :.+ and :.-
let mut global_raw_target: Option<String> = None;
if line.trim().starts_with("TARGET ") {
    let target_spec = line.trim().strip_prefix("TARGET ").unwrap_or("");
    global_raw_target = Some(target_spec.to_string());
    let parsed = parse_target(target_spec);
    global_target_win = parsed.window;
    global_target_pane = parsed.pane;
    global_pane_is_id = parsed.pane_is_id;
    // Now read the actual command line
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

// Process commands in a loop to handle batching
let mut attached_sent = false;
loop {
    if line.trim().is_empty() {
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
    let parsed = parse_command_line(&line);
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
            if pt.window.is_some() { target_win = pt.window; }
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
let is_focus_cmd = matches!(cmd, "select-window" | "selectw" | "select-pane" | "selectp")
    || (matches!(cmd, "split-window" | "splitw") && !args.iter().any(|a| *a == "-d"));
if let Some(wid) = target_win {
    if is_focus_cmd {
        let _ = tx.send(CtrlReq::FocusWindow(wid));
    } else {
        let _ = tx.send(CtrlReq::FocusWindowTemp(wid));
    }
}
let targeted_kill_pane_id = if matches!(cmd, "kill-pane" | "killp") && pane_is_id {
    target_pane
} else {
    None
};
let skip_pane_focus = matches!(cmd, "display-message" | "display");
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
        let size_pct: Option<u16> = args.windows(2).find(|w| w[0] == "-p").and_then(|w| w[1].trim_matches('%').parse().ok())
            .or_else(|| args.windows(2).find(|w| w[0] == "-l").and_then(|w| {
                let s = w[1].trim_matches('%');
                s.parse().ok()
            }));
        let cmd_str: Option<String> = args.iter()
            .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-p" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-l" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)))
            .map(|s| s.trim_matches('"').to_string());
        if print_info {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, size_pct, format_str, rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, size_pct, rtx));
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
                let _ = write_stream.write_all(text.as_bytes());
                let _ = write_stream.flush();
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
            let _ = write!(write_stream, "{}\n", text); 
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "dump-state" => {
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
    "next-window" | "next" => { let _ = tx.send(CtrlReq::NextWindow); }
    "previous-window" | "prev" => { let _ = tx.send(CtrlReq::PrevWindow); }
    "rename-window" | "renamew" => { if let Some(name) = args.get(0) { let _ = tx.send(CtrlReq::RenameWindow((*name).to_string())); } }
    "list-windows" | "lsw" => {
        // Extract -F format if provided
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindowsFormat(rtx, fmt_str));
            if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        } else if args.iter().any(|a| *a == "-J") {
            // JSON output for programmatic use
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindows(rtx));
            if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        } else {
            // tmux-compatible text output (default)
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
            if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        }
        if !persistent { break; }
    }
    "list-tree" => { let (rtx, rrx) = mpsc::channel::<String>(); let _ = tx.send(CtrlReq::ListTree(rtx)); if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); } if !persistent { break; } }
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
        let all = args.iter().any(|a| *a == "-a" || *a == "-s");
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            if all {
                let _ = tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt_str));
            } else {
                let _ = tx.send(CtrlReq::ListPanesFormat(rtx, fmt_str));
            }
        } else {
            if all {
                let _ = tx.send(CtrlReq::ListAllPanes(rtx));
            } else {
                let _ = tx.send(CtrlReq::ListPanes(rtx));
            }
        }
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "kill-window" | "killw" => { let _ = tx.send(CtrlReq::KillWindow); }
    "kill-session" => { let _ = tx.send(CtrlReq::KillSession); }
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
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "show-buffer" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowBuffer(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "delete-buffer" => { let _ = tx.send(CtrlReq::DeleteBuffer); }
    "choose-buffer" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ChooseBuffer(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "display-message" | "display" => {
        // Parse tmux-like display-message flags without dropping message text.
        let mut print_stdout = false;
        let mut parts: Vec<&str> = Vec::new();
        let mut end_of_opts = false;
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
                "-I" | "-d" => { i += 1; }
                _ if a.starts_with('-') => { parts.push(a); }
                _ => parts.push(a),
            }
            i += 1;
        }

        let fmt = parts.join(" ");
        // Pass target pane index for PANE_POS_OVERRIDE (#113).
        let target_pane_idx: Option<usize> = if !pane_is_id { target_pane } else { None };
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_stdout));
        if let Ok(text) = rrx.recv() {
            if print_stdout {
                let _ = writeln!(write_stream, "{}", text);
                let _ = write_stream.flush();
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
    "join-pane" | "joinp" => {
        if let Some(wid) = args.iter().find(|a| !a.starts_with('-')).and_then(|s| s.parse::<usize>().ok()) {
            let _ = tx.send(CtrlReq::JoinPane(wid));
        }
    }
    "respawn-pane" | "respawnp" => { let _ = tx.send(CtrlReq::RespawnPane); }
    "session-info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::SessionInfo(rtx));
        if let Ok(line) = rrx.recv() { let _ = write!(write_stream, "{}\n", line); let _ = write_stream.flush(); }
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
        // Parse bind-key's own flags first, then extract key + command.
        // bind-key flags: -T <table>, -n (root), -r (repeatable)
        // Everything after the key is the command string verbatim.
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-T" if i + 1 < args.len() => {
                    table = args[i + 1].to_string();
                    i += 2; continue;
                }
                "-n" => { table = "root".to_string(); i += 1; continue; }
                "-r" => { repeatable = true; i += 1; continue; }
                _ => break, // First non-flag arg = the key
            }
        }
        // args[i] = the key, args[i+1..] = the command (preserve all flags)
        if i < args.len() && i + 1 < args.len() {
            let key = args[i].to_string();
            let command = args[i + 1..].join(" ");
            let _ = tx.send(CtrlReq::BindKey(table, key, command, repeatable));
        }
    }
    "unbind-key" | "unbind" => {
        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if let Some(key) = non_flag_args.first() {
            let _ = tx.send(CtrlReq::UnbindKey(key.to_string()));
        }
    }
    "list-keys" | "lsk" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListKeys(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "set-option" | "set" | "set-window-option" | "setw" => {
        let has_u = args.iter().any(|a| *a == "-u");
        let has_a = args.iter().any(|a| *a == "-a");
        let has_q = args.iter().any(|a| *a == "-q");
        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_u {
            if let Some(option) = non_flag_args.first() {
                let _ = tx.send(CtrlReq::SetOptionUnset(option.to_string()));
            }
        } else if non_flag_args.len() >= 2 {
            let option = non_flag_args[0].to_string();
            let value = non_flag_args[1..].join(" ");
            if has_a {
                let _ = tx.send(CtrlReq::SetOptionAppend(option, value));
            } else {
                let _ = tx.send(CtrlReq::SetOptionQuiet(option, value, has_q));
            }
        } else if non_flag_args.len() == 1 && has_q {
            // set -q <option> with no value — silently ignore
        }
    }
    "show-options" | "show" | "show-window-options" | "showw" => {
        let has_a = args.iter().any(|a| *a == "-A");
        let _has_s = args.iter().any(|a| *a == "-s");
        let has_w = args.iter().any(|a| *a == "-w");
        let window_scope = matches!(cmd, "show-window-options" | "showw") || has_w;
        let has_v = args.iter().any(|a| *a == "-v");
        let has_q = args.iter().any(|a| *a == "-q");
        let opt_name: Option<&str> = args.iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .last();
        if has_v || (opt_name.is_some() && !has_q) {
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
                        if has_v {
                            let _ = write!(write_stream, "{}\n", resolved);
                        } else {
                            let _ = write!(write_stream, "{} {}\n", name, resolved);
                        }
                        let _ = write_stream.flush();
                    }
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
                    let _ = write!(write_stream, "{}\n", text);
                    let _ = write_stream.flush();
                }
            } else {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
                if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
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
        let target = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::LinkWindow(target));
    }
    "unlink-window" | "unlinkw" => {
        let _ = tx.send(CtrlReq::UnlinkWindow);
    }
    "find-window" | "findw" => {
        let pattern = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::FindWindow(rtx, pattern));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "move-pane" | "movep" => {
        if let Some(target) = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok()) {
            let _ = tx.send(CtrlReq::MovePane(target));
        }
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
    "next-layout" => {
        let _ = tx.send(CtrlReq::NextLayout);
    }
    "list-clients" | "lsc" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListClients(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "switch-client" | "switchc" => {
        let has_t_flag = args.windows(2).any(|w| w[0] == "-T");
        if has_t_flag {
            let table = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string()).unwrap_or_default();
            let _ = tx.send(CtrlReq::SwitchClientTable(table));
        } else {
            let target = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
            let _ = tx.send(CtrlReq::SwitchClient(target));
        }
    }
    "lock-client" => {
        let _ = tx.send(CtrlReq::LockClient);
    }
    "refresh-client" => {
        let _ = tx.send(CtrlReq::RefreshClient);
    }
    "suspend-client" => {
        let _ = tx.send(CtrlReq::SuspendClient);
    }
    "copy-mode-page-up" => {
        let _ = tx.send(CtrlReq::CopyModePageUp);
    }
    "clear-history" => {
        let _ = tx.send(CtrlReq::ClearHistory);
    }
    "save-buffer" | "saveb" => {
        let path = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::SaveBuffer(path));
    }
    "load-buffer" | "loadb" => {
        let path = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::LoadBuffer(path));
    }
    "set-environment" | "setenv" => {
        let has_u = args.iter().any(|a| *a == "-u");
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
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        if !persistent { break; }
    }
    "set-hook" => {
        let has_unset = args.iter().any(|a| *a == "-u" || *a == "-gu" || *a == "-ug");
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_unset {
            // set-hook -gu <hook-name>  →  remove the hook
            if let Some(name) = non_flag.first() {
                let _ = tx.send(CtrlReq::RemoveHook(name.to_string()));
            }
        } else if non_flag.len() >= 2 {
            let _ = tx.send(CtrlReq::SetHook(non_flag[0].to_string(), non_flag[1..].join(" ")));
        }
    }
    "show-hooks" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowHooks(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
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
        let mut width: u16 = 80;
        let mut height: u16 = 24;
        let mut skip_indices = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" => { if let Some(v) = args.get(i+1) { width = v.trim_end_matches('%').parse().unwrap_or(80); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-h" => { if let Some(v) = args.get(i+1) { height = v.trim_end_matches('%').parse().unwrap_or(24); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-E" | "-K" => { skip_indices.insert(i); }
                _ => {}
            }
            i += 1;
        }
        let content = args.iter().enumerate().filter(|(idx, _)| !skip_indices.contains(idx)).map(|(_, a)| *a).collect::<Vec<&str>>().join(" ");
        let _ = tx.send(CtrlReq::DisplayPopup(content, width, height, close_on_exit));
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
        let _ = tx.send(CtrlReq::ClientDetach(client_id));
        attached_sent = false;
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
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
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
    "show-messages" | "showmsgs" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowMessages(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}", text); let _ = write_stream.flush(); }
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
        // Expand ~ to home directory
        let shell_cmd = if shell_cmd.contains('~') {
            let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
            shell_cmd.replace("~/", &format!("{}/", home)).replace("~\\", &format!("{}\\", home))
        } else {
            shell_cmd
        };
        if !shell_cmd.is_empty() {
            if background {
                let _ = std::process::Command::new("pwsh").args(["-NoProfile", "-Command", &shell_cmd]).spawn();
            } else {
                let output = std::process::Command::new("pwsh")
                    .args(["-NoProfile", "-Command", &shell_cmd]).output();
                if let Ok(out) = output {
                    let text = String::from_utf8_lossy(&out.stdout);
                    if !text.is_empty() {
                        let _ = write!(write_stream, "{}", text);
                        let _ = write_stream.flush();
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
                !condition.is_empty() && condition != "0"
            } else if condition == "true" || condition == "1" {
                true
            } else if condition == "false" || condition == "0" {
                false
            } else {
                // Try pwsh first, fall back to cmd /c
                std::process::Command::new("pwsh")
                    .args(["-NoProfile", "-Command", condition])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or_else(|_| {
                        std::process::Command::new("cmd")
                            .args(["/c", condition])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .map(|s| s.success()).unwrap_or(false)
                    })
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
            let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt_str, None, false));
            if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SessionInfo(rtx));
            if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
        }
        if !persistent { break; }
    }
    "new-session" | "new" => {
        // Accept but ignore in server context (requires a new process)
    }
    "list-commands" | "lscm" => {
        let cmds = TMUX_COMMANDS.join("\n");
        let _ = write!(write_stream, "{}\n", cmds);
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "server-info" | "info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ServerInfo(rtx));
        if let Ok(text) = rrx.recv() { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); }
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
    "lock-server" | "lock-session" | "lock" => {
        // Lock is a no-op on Windows (no terminal locking concept)
        // Stub for compatibility
    }
    "focus-in" => { let _ = tx.send(CtrlReq::FocusIn); }
    "focus-out" => { let _ = tx.send(CtrlReq::FocusOut); }
    "choose-client" => {
        // Single-client model — choose-client is a no-op
    }
    "customize-mode" => {
        // tmux 3.2+ customize-mode — stub for compatibility
    }
    _ => {}
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
