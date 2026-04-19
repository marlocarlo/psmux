use super::*;
use std::net::TcpStream;
use crate::cli::parse_target;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "kill-pane" | "killp" => {
        let targeted_kill_pane_id = if ctx.pane_is_id { ctx.target_pane } else { None };
        if let Some(pid) = targeted_kill_pane_id {
            let _ = ctx.tx.send(CtrlReq::KillPaneById(pid));
        } else {
            let _ = ctx.tx.send(CtrlReq::KillPane);
        }
        DispatchResult::Handled
    }
    "select-pane" | "selectp" => {
        let is_next_pane = ctx.raw_target.as_deref().map_or(false, |t| t.contains(".+") || t == "+" || t == ":.+");
        let is_prev_pane = ctx.raw_target.as_deref().map_or(false, |t| t.contains(".-") || t == "-" || t == ":.-");
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
        let title = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string());
        if let Some(t) = title {
            let _ = ctx.tx.send(CtrlReq::SetPaneTitle(t));
        }
        let pane_style = args.windows(2).find(|w| w[0] == "-P").map(|w| w[1].to_string());
        if let Some(style) = pane_style {
            let _ = ctx.tx.send(CtrlReq::SetPaneStyle(style));
        }
        if !dir.is_empty() {
            let _ = ctx.tx.send(CtrlReq::SelectPane(dir.to_string()));
        }
        DispatchResult::Handled
    }
    "swap-pane" | "swapp" => {
        let dir = if args.iter().any(|a| *a == "-U") { "U" }
            else if args.iter().any(|a| *a == "-D") { "D" }
            else { "D" };
        let _ = ctx.tx.send(CtrlReq::SwapPane(dir.to_string()));
        DispatchResult::Handled
    }
    "zoom-pane" => { let _ = ctx.tx.send(CtrlReq::ZoomPane); DispatchResult::Handled }
    "resize-pane" | "resizep" => {
        if args.iter().any(|a| *a == "-Z") {
            let _ = ctx.tx.send(CtrlReq::ZoomPane);
        } else if let Some(xval) = args.windows(2).find(|w| w[0] == "-x").map(|w| w[1]) {
            if let Some(pct) = xval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                let _ = ctx.tx.send(CtrlReq::ResizePanePercent("x".to_string(), pct));
            } else if let Ok(abs) = xval.parse::<u16>() {
                let _ = ctx.tx.send(CtrlReq::ResizePaneAbsolute("x".to_string(), abs));
            }
        } else if let Some(yval) = args.windows(2).find(|w| w[0] == "-y").map(|w| w[1]) {
            if let Some(pct) = yval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                let _ = ctx.tx.send(CtrlReq::ResizePanePercent("y".to_string(), pct));
            } else if let Ok(abs) = yval.parse::<u16>() {
                let _ = ctx.tx.send(CtrlReq::ResizePaneAbsolute("y".to_string(), abs));
            }
        } else {
            let amount = args.iter().find(|a| a.parse::<u16>().is_ok()).and_then(|s| s.parse::<u16>().ok()).unwrap_or(1);
            let dir = if args.iter().any(|a| *a == "-U") { "U" }
                else if args.iter().any(|a| *a == "-D") { "D" }
                else if args.iter().any(|a| *a == "-L") { "L" }
                else if args.iter().any(|a| *a == "-R") { "R" }
                else { "D" };
            let _ = ctx.tx.send(CtrlReq::ResizePane(dir.to_string(), amount));
        }
        DispatchResult::Handled
    }
    "break-pane" | "breakp" => { let _ = ctx.tx.send(CtrlReq::BreakPane); DispatchResult::Handled }
    "join-pane" | "joinp" | "move-pane" | "movep" => {
        let horizontal = args.iter().any(|a| *a == "-h");
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
        let tgt_win = ctx.target_win.or_else(|| {
            args.iter()
                .find(|a| a.parse::<usize>().is_ok())
                .and_then(|s| s.parse::<usize>().ok())
        });
        let _ = ctx.tx.send(CtrlReq::JoinPane {
            src_win,
            src_pane,
            target_win: tgt_win,
            target_pane: ctx.target_pane,
            horizontal,
        });
        DispatchResult::Handled
    }
    "respawn-pane" | "respawnp" => {
        let workdir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
        let kill = args.iter().any(|a| *a == "-k");
        let _ = ctx.tx.send(CtrlReq::RespawnPane(workdir, kill));
        DispatchResult::Handled
    }
    "pipe-pane" | "pipep" => {
        let stdin_flag = args.iter().any(|a| *a == "-I");
        let stdout_flag = args.iter().any(|a| *a == "-O");
        let toggle = args.iter().any(|a| *a == "-o");
        let cmd_str = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
        let (stdin, stdout) = if !stdin_flag && !stdout_flag {
            (false, true)
        } else {
            (stdin_flag, stdout_flag)
        };
        let _ = ctx.tx.send(CtrlReq::PipePane(cmd_str, stdin, stdout, toggle));
        DispatchResult::Handled
    }
    "display-panes" | "displayp" => { let _ = ctx.tx.send(CtrlReq::DisplayPanes); DispatchResult::Handled }
    "set-pane-title" => { let title = args.join(" "); let _ = ctx.tx.send(CtrlReq::SetPaneTitle(title)); DispatchResult::Handled }
    "list-panes" | "lsp" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        let all = args.iter().any(|a| *a == "-a");
        let session_scope = args.iter().any(|a| *a == "-s");
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            if all || session_scope {
                let _ = ctx.tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt_str));
            } else {
                let _ = ctx.tx.send(CtrlReq::ListPanesFormat(rtx, fmt_str));
            }
        } else {
            if all {
                let _ = ctx.tx.send(CtrlReq::ListAllPanes(rtx));
            } else if session_scope {
                let _ = ctx.tx.send(CtrlReq::ListAllPanes(rtx));
            } else {
                let _ = ctx.tx.send(CtrlReq::ListPanes(rtx));
            }
        }
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-panes".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "pane-forward-extract" => {
        let spec = args.first().copied().unwrap_or("0.0");
        let pt = parse_target(spec);
        let win = pt.window.unwrap_or(0);
        let pane = pt.pane.unwrap_or(0);
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::PaneForwardExtract(win, pane, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_millis(5000)) {
            let _ = write!(ctx.write_stream, "{}\n", resp);
            let _ = ctx.write_stream.flush();
        } else {
            let _ = write!(ctx.write_stream, "ERR timeout\n");
            let _ = ctx.write_stream.flush();
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "pane-forward-inject" => {
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
            let screen_b64 = if screen_b64_len > 0 {
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
            let _ = ctx.tx.send(CtrlReq::PaneForwardInject {
                source_session, source_addr, source_key,
                forward_id, fwd_port, pid, title, rows, cols, screen_b64,
                target_win: ctx.target_win, target_pane: ctx.target_pane, horizontal,
            });
            let _ = write!(ctx.write_stream, "OK\n");
            let _ = ctx.write_stream.flush();
        } else {
            let _ = write!(ctx.write_stream, "ERR not enough args\n");
            let _ = ctx.write_stream.flush();
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "pane-forward-resize" => {
        if args.len() >= 3 {
            let fwd_id: u64 = args[0].parse().unwrap_or(0);
            let rows: u16 = args[1].parse().unwrap_or(24);
            let cols: u16 = args[2].parse().unwrap_or(80);
            let _ = ctx.tx.send(CtrlReq::PaneForwardResize(fwd_id, rows, cols));
            let _ = write!(ctx.write_stream, "OK\n");
        }
        let _ = ctx.write_stream.flush();
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "pane-forward-status" => {
        let fwd_id: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::PaneForwardStatus(fwd_id, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_millis(2000)) {
            let _ = write!(ctx.write_stream, "{}\n", resp);
        } else {
            let _ = write!(ctx.write_stream, "exited\n");
        }
        let _ = ctx.write_stream.flush();
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "pane-forward-kill" => {
        let fwd_id: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
        let _ = ctx.tx.send(CtrlReq::PaneForwardKill(fwd_id));
        let _ = write!(ctx.write_stream, "OK\n");
        let _ = ctx.write_stream.flush();
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    _ => DispatchResult::Unhandled,
    }
}
