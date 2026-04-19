use super::*;
use std::net::TcpStream;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
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
            let _ = ctx.tx.send(CtrlReq::NewWindowPrint(cmd_str, name, detached, start_dir, format_str, rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(ctx.write_stream, "{}\n", text);
                let _ = ctx.write_stream.flush();
            }
            if !ctx.persistent { return DispatchResult::Break; }
        } else {
            let _ = ctx.tx.send(CtrlReq::NewWindow(cmd_str, name, detached, start_dir));
        }
        DispatchResult::Handled
    }
    "split-window" | "splitw" => {
        let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
        let detached = args.iter().any(|a| *a == "-d");
        let print_info = args.iter().any(|a| *a == "-P");
        let format_str: Option<String> = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
        let start_dir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
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
            let _ = ctx.tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, split_size, format_str, rtx));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(ctx.write_stream, "{}\n", text);
                let _ = ctx.write_stream.flush();
            }
            if !ctx.persistent { return DispatchResult::Break; }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, split_size, rtx));
            if let Ok(err_msg) = rrx.recv_timeout(Duration::from_millis(2000)) {
                if !err_msg.is_empty() {
                    let _ = write!(ctx.write_stream, "{}\n", err_msg);
                    let _ = ctx.write_stream.flush();
                }
            }
        }
        DispatchResult::Handled
    }
    "kill-window" | "killw" => { let _ = ctx.tx.send(CtrlReq::KillWindow); DispatchResult::Handled }
    "next-window" | "next" => { let _ = ctx.tx.send(CtrlReq::NextWindow); DispatchResult::Handled }
    "previous-window" | "prev" => { let _ = ctx.tx.send(CtrlReq::PrevWindow); DispatchResult::Handled }
    "rename-window" | "renamew" => { if let Some(name) = args.get(0) { let _ = ctx.tx.send(CtrlReq::RenameWindow((*name).to_string())); } DispatchResult::Handled }
    "list-windows" | "lsw" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::ListWindowsFormat(rtx, fmt_str));
            if let Ok(text) = rrx.recv() {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
                }
            }
        } else if args.iter().any(|a| *a == "-J") {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::ListWindows(rtx));
            if let Ok(text) = rrx.recv() {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
                }
            }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = ctx.tx.send(CtrlReq::ListWindowsTmux(rtx));
            if let Ok(text) = rrx.recv() {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
                }
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "list-tree" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ListTree(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-tree".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "select-window" | "selectw" => {
        let idx = args.iter().find(|a| !a.starts_with('-')).and_then(|s| s.parse::<usize>().ok())
            .or(ctx.target_win);
        if let Some(idx) = idx {
            let _ = ctx.tx.send(CtrlReq::SelectWindow(idx));
        }
        if args.iter().any(|a| *a == "-l") {
            let _ = ctx.tx.send(CtrlReq::LastWindow);
        }
        if args.iter().any(|a| *a == "-n") {
            let _ = ctx.tx.send(CtrlReq::NextWindow);
        }
        if args.iter().any(|a| *a == "-p") {
            let _ = ctx.tx.send(CtrlReq::PrevWindow);
        }
        DispatchResult::Handled
    }
    "last-window" | "last" => { let _ = ctx.tx.send(CtrlReq::LastWindow); DispatchResult::Handled }
    "move-window" | "movew" => {
        let target = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok());
        let _ = ctx.tx.send(CtrlReq::MoveWindow(target));
        DispatchResult::Handled
    }
    "swap-window" | "swapw" => {
        if let Some(target) = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok()) {
            let _ = ctx.tx.send(CtrlReq::SwapWindow(target));
        }
        DispatchResult::Handled
    }
    "link-window" | "linkw" => {
        let src_idx = args.windows(2).find(|w| w[0] == "-s")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        let dst_idx = args.windows(2).find(|w| w[0] == "-t")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        let _ = ctx.tx.send(CtrlReq::LinkWindow(src_idx, dst_idx));
        DispatchResult::Handled
    }
    "unlink-window" | "unlinkw" => {
        let _ = ctx.tx.send(CtrlReq::UnlinkWindow);
        DispatchResult::Handled
    }
    "find-window" | "findw" => {
        let pattern = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::FindWindow(rtx, pattern));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("find-window".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "select-layout" | "selectl" => {
        let layout = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"tiled").to_string();
        let _ = ctx.tx.send(CtrlReq::SelectLayout(layout));
        DispatchResult::Handled
    }
    "next-layout" | "nextl" => {
        let _ = ctx.tx.send(CtrlReq::NextLayout);
        DispatchResult::Handled
    }
    "previous-layout" | "prevl" => {
        let _ = ctx.tx.send(CtrlReq::PrevLayout);
        DispatchResult::Handled
    }
    "rotate-window" | "rotatew" => {
        let reverse = args.iter().any(|a| *a == "-D");
        let _ = ctx.tx.send(CtrlReq::RotateWindow(reverse));
        DispatchResult::Handled
    }
    "resize-window" | "resizew" => {
        let abs_x = args.windows(2).find(|w| w[0] == "-x").and_then(|w| w[1].parse::<u16>().ok());
        let abs_y = args.windows(2).find(|w| w[0] == "-y").and_then(|w| w[1].parse::<u16>().ok());
        if let Some(xv) = abs_x {
            let _ = ctx.tx.send(CtrlReq::ResizeWindow("x".to_string(), xv));
        } else if let Some(yv) = abs_y {
            let _ = ctx.tx.send(CtrlReq::ResizeWindow("y".to_string(), yv));
        }
        DispatchResult::Handled
    }
    "respawn-window" | "respawnw" => {
        let _ = ctx.tx.send(CtrlReq::RespawnWindow);
        DispatchResult::Handled
    }
    _ => DispatchResult::Unhandled,
    }
}
