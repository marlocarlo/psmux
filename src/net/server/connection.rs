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
use super::conn_dispatch::{DispatchCtx, DispatchResult};

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
let _ = stream.set_nodelay(true);
let mut write_stream = match stream.try_clone() {
    Ok(s) => s,
    Err(_) => return,
};

let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
let mut r = io::BufReader::new(stream);

// Read the authentication line
let mut auth_line = String::new();
if r.read_line(&mut auth_line).is_err() {
    return;
}

let auth_line = auth_line.trim();
if !auth_line.starts_with("AUTH ") {
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
let _ = write_stream.write_all(b"OK\n");
let _ = write_stream.flush();

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

if line.trim() == "PERSISTENT" {
    persistent = true;
    let _ = r.get_ref().set_nodelay(true);
    let _ = write_stream.set_nodelay(true);
    let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(5000)));

    crate::types::register_persistent_stream(client_id, &write_stream);

    let mut ws_bg = write_stream.try_clone().unwrap();
    let (resp_tx, resp_rx) = mpsc::channel::<mpsc::Receiver<String>>();

    let frame_chan = crate::types::register_frame_channel(client_id);
    let directive_rx = crate::types::register_directive_channel(client_id);

    std::thread::spawn(move || {
        let frame_rx = frame_chan.rx.lock().unwrap();
        loop {
            while let Ok(directive) = directive_rx.try_recv() {
                if write!(ws_bg, "{}\n", directive).is_err() { return; }
                if ws_bg.flush().is_err() { return; }
            }
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
    super::conn_control_mode::handle_control_mode(
        &mut r, &mut write_stream, &tx, control_echo, control_noecho, aliases,
    );
    return;
}

// Check if this line is a TARGET specification
let mut global_raw_target: Option<String> = None;
if line.trim().starts_with("TARGET ") {
    let target_spec = line.trim().strip_prefix("TARGET ").unwrap_or("");
    global_raw_target = Some(target_spec.to_string());
    let parsed = parse_target(target_spec);
    global_target_win = parsed.window;
    global_target_win_name = parsed.window_name;
    global_target_pane = parsed.pane;
    global_pane_is_id = parsed.pane_is_id;
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(10)));

// Process commands in a loop to handle batching
let mut attached_sent = false;
let mut pending_chain: Vec<String> = Vec::new();
loop {
    if !pending_chain.is_empty() {
        line = pending_chain.remove(0);
    } else if line.trim().is_empty() {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => {
                if attached_sent {
                    let _ = tx.send(CtrlReq::ClientDetach(client_id));
                }
                break;
            }
            Err(e) => {
                if persistent && (e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut) {
                    line.clear();
                    continue;
                }
                if attached_sent {
                    let _ = tx.send(CtrlReq::ClientDetach(client_id));
                }
                break;
            }
            Ok(_) => continue,
        }
    }

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
    let alias_expanded = if let Ok(map) = aliases.read() {
        map.get(raw_cmd).cloned()
    } else { None };
    let (cmd, args): (&str, Vec<&str>) = if let Some(ref expanded) = alias_expanded {
        let expanded_parts: Vec<&str> = expanded.split_whitespace().collect();
        let mut all_args: Vec<&str> = expanded_parts[1..].to_vec();
        all_args.extend(parsed.iter().skip(1).map(|s| s.as_str()));
        (expanded_parts.first().copied().unwrap_or(raw_cmd), all_args)
    } else {
        (raw_cmd, parsed.iter().skip(1).map(|s| s.as_str()).collect())
    };

// Parse -t argument from command line
let mut target_win: Option<usize> = global_target_win;
let mut target_win_name: Option<String> = global_target_win_name.clone();
let mut target_pane: Option<usize> = global_target_pane;
let mut pane_is_id = global_pane_is_id;
let mut raw_target: Option<String> = global_raw_target.clone();
let mut i = 0;
while i < args.len() {
    if args[i] == "-t" {
        if let Some(v) = args.get(i+1) {
            raw_target = Some(v.to_string());
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
let args: Vec<&str> = {
    let mut filtered = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            i += 2;
            continue;
        }
        filtered.push(args[i]);
        i += 1;
    }
    filtered
};
let is_focus_cmd = matches!(cmd, "select-window" | "selectw" | "select-pane" | "selectp");
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

// Dispatch command to handler modules
let mut ctx = DispatchCtx {
    tx: &tx,
    write_stream: &mut write_stream,
    persistent,
    resp_tx_opt: &resp_tx_opt,
    client_id,
    target_win,
    target_pane,
    pane_is_id,
    raw_target: raw_target.clone(),
    line: effective_line.clone(),
    attached_sent: &mut attached_sent,
};

let result = dispatch_command(&mut ctx, cmd, &args);
match result {
    DispatchResult::Break => break,
    DispatchResult::ContinueWith(new_cmd) => {
        line.clear();
        line.push_str(&new_cmd);
        line.push('\n');
        continue;
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
            if attached_sent {
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
            }
            break;
        }
        Err(e) => {
            if persistent && (e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut) {
                line.clear();
                continue;
            }
            if attached_sent {
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
            }
            break;
        }
        Ok(_) => {}
    }
} // end command loop
}

fn dispatch_command(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    macro_rules! try_dispatch {
        ($mod:ident) => {
            let r = super::$mod::dispatch(ctx, cmd, args);
            if !matches!(r, DispatchResult::Unhandled) { return r; }
        }
    }
    try_dispatch!(conn_window);
    try_dispatch!(conn_pane);
    try_dispatch!(conn_buffer);
    try_dispatch!(conn_keys);
    try_dispatch!(conn_options);
    try_dispatch!(conn_display);
    try_dispatch!(conn_session);
    try_dispatch!(conn_mouse);
    try_dispatch!(conn_misc);
    DispatchResult::Handled // _ => {} case: unknown commands are silently ignored
}
