use super::*;
use std::net::TcpStream;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "kill-session" | "kill-ses" => {
        if let Some(ref tgt) = ctx.raw_target {
            let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
            let port_path = format!("{}\\.psmux\\{}.port", home, tgt);
            if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    let key = crate::session::read_session_key(tgt).unwrap_or_default();
                    let _ = crate::session::send_control_to_port(port, "kill-session\n", &key);
                }
            }
        } else {
            let _ = ctx.tx.send(CtrlReq::KillSession);
        }
        DispatchResult::Handled
    }
    "has-session" => {
        let (rtx, rrx) = mpsc::channel::<bool>();
        let _ = ctx.tx.send(CtrlReq::HasSession(rtx));
        if let Ok(exists) = rrx.recv() {
            if !exists { std::process::exit(1); }
        }
        DispatchResult::Handled
    }
    "rename-session" | "rename" => {
        if let Some(name) = args.iter().find(|a| !a.starts_with('-')) {
            let _ = ctx.tx.send(CtrlReq::RenameSession((*name).to_string()));
        }
        DispatchResult::Handled
    }
    "claim-session" => {
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(|s| &**s).collect();
        if let Some(name) = non_flag.first().copied() {
            let client_cwd = non_flag.get(1).map(|s| s.to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::ClaimSession(name.to_string(), client_cwd, rtx));
            if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(5)) {
                let _ = write!(ctx.write_stream, "{}", resp);
                let _ = ctx.write_stream.flush();
            }
        }
        DispatchResult::Handled
    }
    "session-info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::SessionInfo(rtx));
        if let Ok(line) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("session-info".to_string(), line));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", line); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "client-attach" => {
        if !*ctx.attached_sent {
            let _ = ctx.tx.send(CtrlReq::ClientAttach(ctx.client_id));
            *ctx.attached_sent = true;
        }
        if !ctx.persistent { let _ = write!(ctx.write_stream, "ok\n"); }
        DispatchResult::Handled
    }
    "client-detach" => {
        let _ = ctx.tx.send(CtrlReq::ClientDetach(ctx.client_id));
        *ctx.attached_sent = false;
        if !ctx.persistent { let _ = write!(ctx.write_stream, "ok\n"); }
        DispatchResult::Handled
    }
    "detach-client" | "detach" => {
        let target_cid: Option<u64> = ctx.raw_target.as_ref()
            .and_then(|t| t.trim_start_matches('%').parse::<u64>().ok());
        if let Some(cid) = target_cid {
            let _ = ctx.tx.send(CtrlReq::ForceDetachClient(cid));
        } else {
            let _ = ctx.tx.send(CtrlReq::ClientDetach(ctx.client_id));
            *ctx.attached_sent = false;
        }
        DispatchResult::Handled
    }
    "attach-session" | "attach" => {
        if !*ctx.attached_sent {
            let _ = ctx.tx.send(CtrlReq::ClientAttach(ctx.client_id));
            *ctx.attached_sent = true;
        }
        DispatchResult::Handled
    }
    "switch-client" | "switchc" => {
        let has_big_t = args.windows(2).any(|w| w[0] == "-T");
        if has_big_t {
            let table = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string()).unwrap_or_default();
            let _ = ctx.tx.send(CtrlReq::SwitchClientTable(table));
        } else if args.contains(&"-n") {
            let _ = ctx.tx.send(CtrlReq::SwitchClient(String::new(), 'n'));
        } else if args.contains(&"-p") {
            let _ = ctx.tx.send(CtrlReq::SwitchClient(String::new(), 'p'));
        } else if args.contains(&"-l") {
            let _ = ctx.tx.send(CtrlReq::SwitchClient(String::new(), 'l'));
        } else {
            let target = ctx.raw_target.clone().unwrap_or_default();
            let session_target = if let Some(pos) = target.find(':') {
                target[..pos].to_string()
            } else {
                target
            };
            let _ = ctx.tx.send(CtrlReq::SwitchClient(session_target, 't'));
        }
        DispatchResult::Handled
    }
    "list-clients" | "lsc" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            let _ = ctx.tx.send(CtrlReq::ListClientsFormat(rtx, fmt_str));
        } else {
            let _ = ctx.tx.send(CtrlReq::ListClients(rtx));
        }
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-clients".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "choose-client" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ListClients(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("choose-client".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}", text);
                let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "list-sessions" | "ls" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::DisplayMessage(rtx, fmt_str, None, false, None));
            if let Ok(text) = rrx.recv() {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-sessions".to_string(), text));
                } else {
                    let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
                }
            }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::SessionInfo(rtx));
            if let Ok(text) = rrx.recv() {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-sessions".to_string(), text));
                } else {
                    let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
                }
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "new-session" | "new" => {
        if let Some(target) = args.windows(2).find(|w| w[0] == "-t").map(|w| w[1].to_string()) {
            let _ = ctx.tx.send(CtrlReq::SetSessionGroup(target));
        } else {
            dispatch_new_session(ctx, args);
        }
        DispatchResult::Handled
    }
    _ => DispatchResult::Unhandled,
    }
}

fn dispatch_new_session(ctx: &mut DispatchCtx, args: &[&str]) {
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
                        Err(e) => { env_parse_err = Some(e); break; }
                    }
                }
                "-d" => { detached = true; }
                "-t" => { i += 1; }
                "-F" | "-f" => { i += 1; }
                other => {
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
        if ctx.persistent {
            let _ = ctx.tx.send(CtrlReq::StatusMessage(msg.trim().to_string()));
        } else {
            let _ = write!(ctx.write_stream, "{}", msg);
            let _ = ctx.write_stream.flush();
        }
        return;
    }

    let name = sess_name.unwrap_or_else(|| crate::session::next_session_name(None));
    let port_file_base = name.clone();
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let port_path = format!("{}\\.psmux\\{}.port", home, port_file_base);

    let already_exists = if std::path::Path::new(&port_path).exists() {
        if let Ok(port_str) = std::fs::read_to_string(&port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
                std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(100)).is_ok()
            } else { false }
        } else { false }
    } else { false };

    if already_exists {
        if ctx.persistent {
            let _ = ctx.tx.send(CtrlReq::StatusMessage(format!("session '{}' already exists", name)));
        } else {
            let _ = write!(ctx.write_stream, "session '{}' already exists\n", name);
            let _ = ctx.write_stream.flush();
        }
        return;
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), name.clone()];
    if let Some(ref dir) = start_dir { server_args.push("-d".into()); server_args.push(dir.clone()); }
    if let Some(ref wn) = window_name { server_args.push("-n".into()); server_args.push(wn.clone()); }
    if let Some(ref cmd) = initial_command { server_args.push("-c".into()); server_args.push(cmd.clone()); }
    if let Some(ref w) = init_width { server_args.push("-x".into()); server_args.push(w.clone()); }
    if let Some(ref h) = init_height { server_args.push("-y".into()); server_args.push(h.clone()); }
    for (k, v) in &env_vars { server_args.push("-e".into()); server_args.push(format!("{}={}", k, v)); }

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

    for _ in 0..500 {
        if std::path::Path::new(&port_path).exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    if std::path::Path::new(&port_path).exists() {
        if !detached {
            let _ = ctx.tx.send(CtrlReq::SwitchClient(name.clone(), 't'));
        }
        if ctx.persistent {
            let _ = ctx.tx.send(CtrlReq::StatusMessage(format!("created session '{}'", name)));
        } else {
            let _ = write!(ctx.write_stream, "OK\n");
            let _ = ctx.write_stream.flush();
        }
    } else {
        if ctx.persistent {
            let _ = ctx.tx.send(CtrlReq::StatusMessage(format!("failed to create session '{}'", name)));
        } else {
            let _ = write!(ctx.write_stream, "failed to create session '{}'\n", name);
            let _ = ctx.write_stream.flush();
        }
    }
}
