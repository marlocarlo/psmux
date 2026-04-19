use super::*;
use std::net::TcpStream;

/// Extended control mode dispatch (second half of commands).
pub(crate) fn dispatch_control_ext(
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
        "set-option" | "set" | "set-window-option" | "setw" => {
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
        _ => super::conn_control_ext2::dispatch_control_ext2(
            cmd, args, tx, resp_tx, target_pane, pane_is_id, _raw_target, client_id,
        ),
    }
}
