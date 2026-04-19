use super::*;
use std::net::TcpStream;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "client-size" => {
        if args.len() >= 2 { if let (Ok(w), Ok(h)) = (args[0].parse::<u16>(), args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::ClientSize(ctx.client_id, w, h)); } }
        DispatchResult::Handled
    }
    "focus-pane" => {
        if let Some(pid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = ctx.tx.send(CtrlReq::FocusPaneCmd(pid)); }
        DispatchResult::Handled
    }
    "focus-window" => {
        if let Some(wid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = ctx.tx.send(CtrlReq::FocusWindowCmd(wid)); }
        DispatchResult::Handled
    }
    "mouse-down" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseDown(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-down-right" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseDownRight(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-down-middle" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseDownMiddle(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-drag" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseDrag(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-up" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseUp(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-up-right" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseUpRight(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-up-middle" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseUpMiddle(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "mouse-move" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = ctx.tx.send(CtrlReq::MouseMove(ctx.client_id,x,y)); } }
        DispatchResult::Handled
    }
    "scroll-up" => {
        let x = args.get(0).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let y = args.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let _ = ctx.tx.send(CtrlReq::ScrollUp(ctx.client_id, x, y));
        DispatchResult::Handled
    }
    "scroll-down" => {
        let x = args.get(0).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let y = args.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let _ = ctx.tx.send(CtrlReq::ScrollDown(ctx.client_id, x, y));
        DispatchResult::Handled
    }
    "pane-mouse" => {
        if args.len() >= 5 {
            if let (Ok(pane_id), Ok(button), Ok(col), Ok(row)) = (
                args[0].parse::<usize>(), args[1].parse::<u8>(),
                args[2].parse::<i16>(), args[3].parse::<i16>()
            ) {
                let press = args[4] != "m";
                let _ = ctx.tx.send(CtrlReq::PaneMouse(ctx.client_id, pane_id, button, col, row, press));
            }
        }
        DispatchResult::Handled
    }
    "pane-scroll" => {
        if args.len() >= 2 {
            if let Ok(pane_id) = args[0].parse::<usize>() {
                let up = args[1] == "up";
                let _ = ctx.tx.send(CtrlReq::PaneScroll(ctx.client_id, pane_id, up));
            }
        }
        DispatchResult::Handled
    }
    "split-sizes" => {
        if args.len() >= 2 {
            let path: Vec<usize> = if args[0] == "_" {
                Vec::new()
            } else {
                args[0].split('.').filter_map(|s| s.parse().ok()).collect()
            };
            let sizes: Vec<u16> = args[1].split(',').filter_map(|s| s.parse().ok()).collect();
            if sizes.len() >= 2 {
                let _ = ctx.tx.send(CtrlReq::SplitSetSizes(ctx.client_id, path, sizes));
            }
        }
        DispatchResult::Handled
    }
    "split-resize-done" => {
        let _ = ctx.tx.send(CtrlReq::SplitResizeDone(ctx.client_id));
        DispatchResult::Handled
    }
    "focus-in" => { let _ = ctx.tx.send(CtrlReq::FocusIn); DispatchResult::Handled }
    "focus-out" => { let _ = ctx.tx.send(CtrlReq::FocusOut); DispatchResult::Handled }
    _ => DispatchResult::Unhandled,
    }
}
