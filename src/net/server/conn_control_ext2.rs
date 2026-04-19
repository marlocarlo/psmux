use super::*;
use std::net::TcpStream;

/// Extended control mode dispatch, part 2 (bind/unbind, env, hooks, misc).
pub(crate) fn dispatch_control_ext2(
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
        "bind-key" | "bind" => {
            let mut table_name = "prefix".to_string();
            let mut repeat = false;
            let mut i = 0;
            while i < args.len() {
                match args[i] {
                    "-T" if i + 1 < args.len() => { table_name = args[i + 1].to_string(); i += 2; continue; }
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
                if has_table { let _ = tx.send(CtrlReq::UnbindAllInTable(table)); }
                else { let _ = tx.send(CtrlReq::UnbindAll); }
            } else {
                let mut table: Option<String> = None;
                let mut t_value_idx: Option<usize> = None;
                let mut target_session_idx: Option<usize> = None;
                for (j, a) in args.iter().enumerate() {
                    if *a == "-T" { if let Some(t) = args.get(j + 1) { table = Some(t.to_string()); t_value_idx = Some(j + 1); } }
                    if *a == "-n" { table = Some("root".to_string()); }
                    if *a == "-t" { target_session_idx = Some(j + 1); }
                }
                let key_arg = args.iter().enumerate()
                    .filter(|(i, a)| !a.starts_with('-') && Some(*i) != t_value_idx && Some(*i) != target_session_idx)
                    .map(|(_, a)| *a).next();
                if let Some(key) = key_arg { let _ = tx.send(CtrlReq::UnbindKey(key.to_string(), table)); }
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
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) { let _ = resp_tx.send(text); }
            true
        }
        "set-hook" => {
            let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            if positional.len() >= 2 {
                let name = positional[0].to_string();
                let command = positional[1..].iter().map(|s| {
                    if s.contains(' ') { format!("'{}'", s) } else { s.to_string() }
                }).collect::<Vec<_>>().join(" ");
                let has_append = args.iter().any(|a| {
                    if *a == "-a" { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('a')
                });
                if has_append { let _ = tx.send(CtrlReq::AppendHook(name, command)); }
                else { let _ = tx.send(CtrlReq::SetHook(name, command)); }
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "show-hooks" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowHooks(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) { let _ = resp_tx.send(text); }
            true
        }
        "server-info" | "info" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ServerInfo(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) { let _ = resp_tx.send(text); }
            true
        }
        "list-commands" | "lscm" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListCommands(rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) { let _ = resp_tx.send(text); }
            true
        }
        "dump-state" | "dump" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DumpState(rtx, false));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) { let _ = resp_tx.send(text); }
            true
        }
        "zoom-pane" | "resizep -Z" => {
            let _ = tx.send(CtrlReq::ZoomPane);
            let _ = resp_tx.send(String::new());
            true
        }
        "last-window" | "last" => { let _ = tx.send(CtrlReq::LastWindow); let _ = resp_tx.send(String::new()); true }
        "last-pane" | "lastp" => { let _ = tx.send(CtrlReq::LastPane); let _ = resp_tx.send(String::new()); true }
        "next-window" | "next" => { let _ = tx.send(CtrlReq::NextWindow); let _ = resp_tx.send(String::new()); true }
        "previous-window" | "prev" => { let _ = tx.send(CtrlReq::PrevWindow); let _ = resp_tx.send(String::new()); true }
        "rotate-window" | "rotatew" => {
            let upward = args.iter().any(|a| *a == "-U");
            let _ = tx.send(CtrlReq::RotateWindow(upward));
            let _ = resp_tx.send(String::new());
            true
        }
        "break-pane" | "breakp" => { let _ = tx.send(CtrlReq::BreakPane); let _ = resp_tx.send(String::new()); true }
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
            let mut i = 0;
            while i < args.len() {
                if args[i] == "-B" {
                    if let Some(spec) = args.get(i + 1) {
                        let spec = spec.trim_matches('"');
                        if let Some(colon1) = spec.find(':') {
                            let name = spec[..colon1].to_string();
                            let rest = &spec[colon1 + 1..];
                            if rest.is_empty() {
                                let _ = tx.send(CtrlReq::ControlUnsubscribe { client_id, name });
                            } else if let Some(colon2) = rest.find(':') {
                                let target = rest[..colon2].to_string();
                                let format = rest[colon2 + 1..].to_string();
                                let _ = tx.send(CtrlReq::ControlSubscribe { client_id, name, target, format });
                            }
                        }
                    }
                    i += 2; continue;
                }
                if args[i] == "-f" {
                    if let Some(flag_val) = args.get(i + 1) {
                        let flag_val = flag_val.trim_matches('"');
                        if let Some(stripped) = flag_val.strip_prefix("pause-after=") {
                            let secs = stripped.parse::<u64>().ok();
                            let _ = tx.send(CtrlReq::ControlSetPauseAfter { client_id, pause_after_secs: secs });
                        } else if flag_val == "no-pause" {
                            let _ = tx.send(CtrlReq::ControlSetPauseAfter { client_id, pause_after_secs: None });
                        }
                    }
                    i += 2; continue;
                }
                if args[i] == "-A" {
                    if let Some(spec) = args.get(i + 1) {
                        let spec = spec.trim_matches('"').trim_matches('\'');
                        if let Some(colon) = spec.find(':') {
                            let pane_spec = &spec[..colon];
                            let action = &spec[colon + 1..];
                            if action == "continue" {
                                if let Some(pid_str) = pane_spec.strip_prefix('%') {
                                    if let Ok(pid) = pid_str.parse::<usize>() {
                                        let _ = tx.send(CtrlReq::ControlContinuePane { client_id, pane_id: pid });
                                    }
                                }
                            }
                        }
                    }
                    i += 2; continue;
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
            let _ = resp_tx.send(format!("unknown command: {}", cmd));
            true
        }
    }
}
