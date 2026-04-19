use super::*;
use std::net::TcpStream;
use crate::util::base64_decode;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "display-message" | "display" => {
        let mut print_stdout = false;
        let mut parts: Vec<&str> = Vec::new();
        let mut end_of_opts = false;
        let mut duration_ms: Option<u64> = None;
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
                "-d" => {
                    if i + 1 < args.len() {
                        duration_ms = args[i + 1].parse::<u64>().ok();
                    }
                    i += 1;
                }
                "-I" => { i += 1; }
                _ if a.starts_with('-') => { parts.push(a); }
                _ => parts.push(a),
            }
            i += 1;
        }
        let fmt = if parts.is_empty() {
            crate::commands::DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
        } else {
            parts.join(" ")
        };
        let target_pane_idx: Option<usize> = if !ctx.pane_is_id { ctx.target_pane } else { None };
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_stdout, duration_ms));
        if let Ok(text) = rrx.recv() {
            if print_stdout {
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("display-message".to_string(), text));
                } else {
                    let _ = writeln!(ctx.write_stream, "{}", text);
                    let _ = ctx.write_stream.flush();
                }
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "display-popup" | "popup" => {
        let close_on_exit = !args.iter().any(|a| *a == "-K");
        let mut width_spec = "80".to_string();
        let mut height_spec = "24".to_string();
        let mut start_dir: Option<String> = None;
        let mut skip_indices = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" => { if let Some(v) = args.get(i+1) { width_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-h" => { if let Some(v) = args.get(i+1) { height_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-d" | "-c" => { if let Some(v) = args.get(i+1) { start_dir = Some(v.to_string()); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-E" | "-K" => { skip_indices.insert(i); }
                _ => {}
            }
            i += 1;
        }
        let content = args.iter().enumerate().filter(|(idx, _)| !skip_indices.contains(idx)).map(|(_, a)| *a).collect::<Vec<&str>>().join(" ");
        let _ = ctx.tx.send(CtrlReq::DisplayPopup(content, width_spec, height_spec, close_on_exit, start_dir));
        DispatchResult::Handled
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
        let positional: Vec<&str> = args.iter().enumerate()
            .filter(|(idx, a)| !skip_indices.contains(idx) && !a.starts_with('-'))
            .map(|(_, a)| *a).collect();
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
            let _ = ctx.tx.send(CtrlReq::DisplayMenuDirect(menu));
        }
        DispatchResult::Handled
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
        let _ = ctx.tx.send(CtrlReq::ConfirmBefore(prompt_str, command));
        DispatchResult::Handled
    }
    "show-messages" | "showmsgs" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ShowMessages(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-messages".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "command-prompt" => {
        let initial = args.windows(2).find(|w| w[0] == "-I").map(|w| w[1].to_string()).unwrap_or_default();
        let _ = ctx.tx.send(CtrlReq::CommandPrompt(initial));
        DispatchResult::Handled
    }
    "clock-mode" => { let _ = ctx.tx.send(CtrlReq::ClockMode); DispatchResult::Handled }
    "popup-input" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = ctx.tx.send(CtrlReq::PopupInput(decoded.into_bytes()));
            }
        }
        DispatchResult::Handled
    }
    "popup-input-raw" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = ctx.tx.send(CtrlReq::PopupInput(decoded.into_bytes()));
            }
        }
        DispatchResult::Handled
    }
    "overlay-close" => { let _ = ctx.tx.send(CtrlReq::OverlayClose); DispatchResult::Handled }
    "display-panes-select" => {
        if let Some(idx) = args.get(0).and_then(|s| s.parse::<usize>().ok()) {
            let _ = ctx.tx.send(CtrlReq::DisplayPaneSelect(idx));
        }
        DispatchResult::Handled
    }
    "confirm-respond" => {
        let yes = args.get(0).map(|a| *a == "y" || *a == "yes").unwrap_or(false);
        let _ = ctx.tx.send(CtrlReq::ConfirmRespond(yes));
        DispatchResult::Handled
    }
    "menu-select" => {
        if let Some(idx) = args.get(0).and_then(|s| s.parse::<usize>().ok()) {
            let _ = ctx.tx.send(CtrlReq::MenuSelect(idx));
        }
        DispatchResult::Handled
    }
    "menu-navigate" => {
        let delta = args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let _ = ctx.tx.send(CtrlReq::MenuNavigate(delta));
        DispatchResult::Handled
    }
    "customize-navigate" => {
        let delta = args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let _ = ctx.tx.send(CtrlReq::CustomizeNavigate(delta));
        DispatchResult::Handled
    }
    "customize-edit" => { let _ = ctx.tx.send(CtrlReq::CustomizeEdit); DispatchResult::Handled }
    "customize-edit-update" => {
        let text = args.join(" ");
        let _ = ctx.tx.send(CtrlReq::CustomizeEditUpdate(text));
        DispatchResult::Handled
    }
    "customize-edit-confirm" => { let _ = ctx.tx.send(CtrlReq::CustomizeEditConfirm); DispatchResult::Handled }
    "customize-edit-cancel" => { let _ = ctx.tx.send(CtrlReq::CustomizeEditCancel); DispatchResult::Handled }
    "customize-reset-default" => { let _ = ctx.tx.send(CtrlReq::CustomizeResetDefault); DispatchResult::Handled }
    "customize-filter" => {
        let text = args.join(" ");
        let _ = ctx.tx.send(CtrlReq::CustomizeFilter(text));
        DispatchResult::Handled
    }
    "customize-mode" => { let _ = ctx.tx.send(CtrlReq::CustomizeMode); DispatchResult::Handled }
    _ => DispatchResult::Unhandled,
    }
}
