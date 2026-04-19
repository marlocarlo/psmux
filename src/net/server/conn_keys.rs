use super::*;
use std::net::TcpStream;
use crate::util::base64_decode;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "send-text" => {
        if let Some(payload) = args.get(0) { let _ = ctx.tx.send(CtrlReq::SendText(payload.to_string())); }
        DispatchResult::Handled
    }
    "send-paste" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = ctx.tx.send(CtrlReq::SendPaste(decoded));
            }
        }
        DispatchResult::Handled
    }
    "send-key" => {
        if let Some(payload) = args.get(0) { let _ = ctx.tx.send(CtrlReq::SendKey(payload.to_string())); }
        DispatchResult::Handled
    }
    "send-keys" => {
        let literal = args.iter().any(|a| *a == "-l");
        let paste_mode = args.iter().any(|a| *a == "-p");
        let has_x = args.iter().any(|a| *a == "-X");
        let mut repeat_count: usize = 1;
        if let Some(n_pos) = args.iter().position(|a| *a == "-N") {
            if let Some(count_str) = args.get(n_pos + 1) {
                repeat_count = count_str.parse::<usize>().unwrap_or(1).max(1);
            }
        }
        if has_x {
            let cmd_parts: Vec<&str> = args.iter().filter(|a| **a != "-X" && !a.starts_with('-')).copied().collect();
            for _ in 0..repeat_count {
                let _ = ctx.tx.send(CtrlReq::SendKeysX(cmd_parts.join(" ")));
            }
        } else {
            let keys: Vec<&str> = args.iter()
                .enumerate()
                .filter(|(i, a)| {
                    !a.starts_with('-') && **a != "-l" && **a != "-t"
                    && !(i > &0 && args.get(i - 1).map_or(false, |prev| *prev == "-N"))
                })
                .map(|(_, a)| *a)
                .collect();
            for _ in 0..repeat_count {
                if paste_mode {
                    let _ = ctx.tx.send(CtrlReq::SendPaste(keys.join(" ")));
                } else {
                    let _ = ctx.tx.send(CtrlReq::SendKeys(keys.join(" "), literal));
                }
            }
        }
        DispatchResult::Handled
    }
    "send-prefix" => { let _ = ctx.tx.send(CtrlReq::SendPrefix); DispatchResult::Handled }
    "prefix-begin" => { let _ = ctx.tx.send(CtrlReq::PrefixBegin); DispatchResult::Handled }
    "prefix-end" => { let _ = ctx.tx.send(CtrlReq::PrefixEnd); DispatchResult::Handled }
    "copy-enter" => { let _ = ctx.tx.send(CtrlReq::CopyEnter); DispatchResult::Handled }
    "copy-move" => {
        if args.len() >= 2 { if let (Ok(dx), Ok(dy)) = (args[0].parse::<i16>(), args[1].parse::<i16>()) { let _ = ctx.tx.send(CtrlReq::CopyMove(dx, dy)); } }
        DispatchResult::Handled
    }
    "copy-anchor" => { let _ = ctx.tx.send(CtrlReq::CopyAnchor); DispatchResult::Handled }
    "rectangle-toggle" => { let _ = ctx.tx.send(CtrlReq::CopyRectToggle); DispatchResult::Handled }
    "copy-yank" => { let _ = ctx.tx.send(CtrlReq::CopyYank); DispatchResult::Handled }
    "copy-mode" => {
        if args.iter().any(|a| *a == "-u") {
            let _ = ctx.tx.send(CtrlReq::CopyEnterPageUp);
        } else {
            let _ = ctx.tx.send(CtrlReq::CopyEnter);
        }
        DispatchResult::Handled
    }
    "copy-mode-page-up" => { let _ = ctx.tx.send(CtrlReq::CopyModePageUp); DispatchResult::Handled }
    "bind-key" | "bind" => {
        let mut table = "prefix".to_string();
        let mut repeatable = false;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-T" if i + 1 < args.len() => {
                    table = args[i + 1].to_string();
                    i += 2; continue;
                }
                "-n" => { table = "root".to_string(); i += 1; continue; }
                "-r" => { repeatable = true; i += 1; continue; }
                _ => break,
            }
        }
        if i < args.len() && i + 1 < args.len() {
            let key = args[i].to_string();
            let command = args[i + 1..].join(" ");
            let _ = ctx.tx.send(CtrlReq::BindKey(table, key, command, repeatable));
        }
        DispatchResult::Handled
    }
    "unbind-key" | "unbind" => {
        if args.iter().any(|a| *a == "-a" || (a.starts_with('-') && a.contains('a'))) {
            let mut has_table = false;
            let mut table = String::new();
            for (j, a) in args.iter().enumerate() {
                if *a == "-T" { if let Some(t) = args.get(j + 1) { table = t.to_string(); has_table = true; } }
                if *a == "-n" { table = "root".to_string(); has_table = true; }
            }
            if has_table {
                let _ = ctx.tx.send(CtrlReq::UnbindAllInTable(table));
            } else {
                let _ = ctx.tx.send(CtrlReq::UnbindAll);
            }
        } else {
            let mut table: Option<String> = None;
            let mut t_value_idx: Option<usize> = None;
            let mut target_session_idx: Option<usize> = None;
            for (j, a) in args.iter().enumerate() {
                if *a == "-T" {
                    if let Some(t) = args.get(j + 1) {
                        table = Some(t.to_string());
                        t_value_idx = Some(j + 1);
                    }
                }
                if *a == "-n" { table = Some("root".to_string()); }
                if *a == "-t" { target_session_idx = Some(j + 1); }
            }
            let key_arg = args.iter().enumerate()
                .filter(|(i, a)| !a.starts_with('-') && Some(*i) != t_value_idx && Some(*i) != target_session_idx)
                .map(|(_, a)| *a)
                .next();
            if let Some(key) = key_arg {
                let _ = ctx.tx.send(CtrlReq::UnbindKey(key.to_string(), table));
            }
        }
        DispatchResult::Handled
    }
    "list-keys" | "lsk" => {
        let table_filter = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string());
        let key_filter: Option<String> = args.iter()
            .enumerate()
            .filter(|(i, a)| {
                !a.starts_with('-')
                && !(i > &0 && args.get(i - 1).map_or(false, |prev| *prev == "-T"))
            })
            .map(|(_, a)| a.to_string())
            .next();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ListKeys(rtx));
        if let Ok(text) = rrx.recv() {
            let filtered = if table_filter.is_some() || key_filter.is_some() {
                text.lines().filter(|line| {
                    if let Some(ref tbl) = table_filter {
                        let parts: Vec<&str> = line.splitn(5, ' ').collect();
                        if parts.len() >= 3 {
                            if parts[2] != tbl.as_str() {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    if let Some(ref key) = key_filter {
                        let parts: Vec<&str> = line.splitn(5, ' ').collect();
                        if parts.len() >= 4 {
                            if parts[3] != key.as_str() {
                                return false;
                            }
                        }
                    }
                    true
                }).collect::<Vec<&str>>().join("\n")
            } else {
                text
            };
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-keys".to_string(), filtered));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", filtered); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    _ => DispatchResult::Unhandled,
    }
}
