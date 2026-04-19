use super::*;
use std::net::TcpStream;
use crate::util::base64_decode;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "capture-pane" | "capturep" => {
        let print_stdout = args.iter().any(|a| *a == "-p");
        let join_lines = args.iter().any(|a| *a == "-J");
        let escape_seqs = args.iter().any(|a| *a == "-e");
        let s_arg = args.windows(2).find(|w| w[0] == "-S").map(|w| w[1]);
        let e_arg = args.windows(2).find(|w| w[0] == "-E").map(|w| w[1]);
        let start: Option<i32> = match s_arg {
            Some("-") => Some(0),
            Some(v) => v.parse::<i32>().ok(),
            None => None,
        };
        let end: Option<i32> = match e_arg {
            Some("-") => None,
            Some(v) => v.parse::<i32>().ok(),
            None => None,
        };
        let (rtx, rrx) = mpsc::channel::<String>();
        if escape_seqs {
            let _ = ctx.tx.send(CtrlReq::CapturePaneStyled(rtx, start, end));
        } else if s_arg.is_some() || e_arg.is_some() {
            let _ = ctx.tx.send(CtrlReq::CapturePaneRange(rtx, start, end));
        } else {
            let _ = ctx.tx.send(CtrlReq::CapturePane(rtx));
        }
        if let Ok(mut text) = rrx.recv() {
            if join_lines {
                text = text.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
            }
            if print_stdout {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("capture-pane".to_string(), text));
                } else {
                    let _ = ctx.write_stream.write_all(text.as_bytes());
                    let _ = ctx.write_stream.flush();
                }
                if !ctx.persistent { return DispatchResult::Break; }
            } else {
                let _ = ctx.tx.send(CtrlReq::SetBuffer(text));
            }
        }
        DispatchResult::Handled
    }
    "set-buffer" => {
        let content = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
        let _ = ctx.tx.send(CtrlReq::SetBuffer(content));
        DispatchResult::Handled
    }
    "paste-buffer" | "pasteb" => {
        let buf_idx: Option<usize> = args.windows(2).find(|w| w[0] == "-b").and_then(|w| w[1].parse().ok());
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(idx) = buf_idx {
            let _ = ctx.tx.send(CtrlReq::ShowBufferAt(rtx, idx));
        } else {
            let _ = ctx.tx.send(CtrlReq::ShowBuffer(rtx));
        }
        if let Ok(text) = rrx.recv() {
            let _ = ctx.tx.send(CtrlReq::SendText(text));
        }
        DispatchResult::Handled
    }
    "list-buffers" | "lsb" => {
        let fmt = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            let _ = ctx.tx.send(CtrlReq::ListBuffersFormat(rtx, fmt_str));
        } else {
            let _ = ctx.tx.send(CtrlReq::ListBuffers(rtx));
        }
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-buffers".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "show-buffer" | "showb" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ShowBuffer(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-buffer".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "delete-buffer" => {
        let buf_idx: Option<usize> = args.windows(2).find(|w| w[0] == "-b").and_then(|w| w[1].parse().ok());
        if let Some(idx) = buf_idx {
            let _ = ctx.tx.send(CtrlReq::DeleteBufferAt(idx));
        } else {
            let _ = ctx.tx.send(CtrlReq::DeleteBuffer);
        }
        DispatchResult::Handled
    }
    "delete-buffer-at" => {
        if let Some(idx_str) = args.get(0) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                let _ = ctx.tx.send(CtrlReq::DeleteBufferAt(idx));
            }
        }
        DispatchResult::Handled
    }
    "paste-buffer-at" => {
        if let Some(idx_str) = args.get(0) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                let _ = ctx.tx.send(CtrlReq::PasteBufferAt(idx));
            }
        }
        DispatchResult::Handled
    }
    "choose-buffer" | "chooseb" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ChooseBuffer(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("choose-buffer".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "save-buffer" | "saveb" => {
        let path = args.iter().find(|a| **a == "-" || !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = ctx.tx.send(CtrlReq::SaveBuffer(path));
        DispatchResult::Handled
    }
    "load-buffer" | "loadb" => {
        let path = args.iter().find(|a| **a == "-" || !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = ctx.tx.send(CtrlReq::LoadBuffer(path));
        DispatchResult::Handled
    }
    _ => DispatchResult::Unhandled,
    }
}
