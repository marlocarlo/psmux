use super::*;
use std::io::{self, BufRead};
use std::net::TcpStream;
use crate::types::ControlNotification;
use crate::cli::parse_target;
use crate::commands::parse_command_line;

/// Handle a control mode connection (CONTROL or CONTROL_NOECHO).
pub(crate) fn handle_control_mode(
    r: &mut io::BufReader<TcpStream>,
    write_stream: &mut TcpStream,
    tx: &mpsc::Sender<CtrlReq>,
    control_echo: bool,
    control_noecho: bool,
    aliases: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
) {
    let _ = r.get_ref().set_nodelay(true);
    let _ = write_stream.set_nodelay(true);
    let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(5000)));

    let ctrl_client_id = crate::types::next_control_client_id();
    crate::types::register_persistent_stream(ctrl_client_id, write_stream);

    let (notif_tx, notif_rx) = std::sync::mpsc::sync_channel::<ControlNotification>(4096);

    let _ = tx.send(CtrlReq::ControlRegister {
        client_id: ctrl_client_id,
        echo: control_echo,
        notif_tx,
    });

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
            if is_exit && cc_no_echo {
                let _ = ws_notif.write_all(b"\x1b\\");
            }
            if ws_notif.flush().is_err() { break; }
            if is_exit { break; }
        }
    });

    let mut cmd_counter: u64 = 0;
    let tx_ctrl = tx.clone();
    let aliases_ctrl = aliases.clone();
    let mut line = String::new();

    let _ = writeln!(write_stream);
    let _ = write_stream.flush();

    loop {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => break,
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

        if control_echo {
            let _ = writeln!(write_stream, "{}", trimmed);
            let _ = write_stream.flush();
        }

        let _ = writeln!(write_stream, "{}", control::format_begin(ts, cmd_counter));
        let _ = write_stream.flush();

        let parsed = crate::cli::normalize_flag_equals(parse_command_line(trimmed));
        let raw_cmd = parsed.first().map(|s| s.as_str()).unwrap_or("");

        if raw_cmd.is_empty() {
            let _ = writeln!(write_stream, "{}", control::format_end(ts, cmd_counter));
            let _ = write_stream.flush();
            continue;
        }

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

        let (resp_s, resp_r) = mpsc::channel::<String>();
        let dispatched = super::conn_control::dispatch_control_command(
            cmd_name, &filtered_args, &tx_ctrl, resp_s,
            ctrl_target_pane, ctrl_pane_is_id, ctrl_raw_target.as_deref(),
            ctrl_client_id,
        );

        if dispatched {
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
            let _ = writeln!(write_stream, "{}", control::format_end(ts, cmd_counter));
        }
        let _ = write_stream.flush();
    }

    let _ = tx.send(CtrlReq::ControlDeregister { client_id: ctrl_client_id });
    drop(notif_thread);
}
