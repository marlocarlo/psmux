use super::*;
use std::net::TcpStream;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "dump-layout" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::DumpLayout(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("dump-layout".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text);
                let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "dump-state" | "dump" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::DumpState(rtx, ctx.persistent));
        if let Some(ref rtx_bg) = ctx.resp_tx_opt {
            let _ = rtx_bg.send(rrx);
        } else {
            if let Ok(text) = rrx.recv() {
                let _ = write!(ctx.write_stream, "{}\n", text);
                let _ = ctx.write_stream.flush();
            }
            if !ctx.persistent { return DispatchResult::Break; }
        }
        DispatchResult::Handled
    }
    "toggle-sync" => { let _ = ctx.tx.send(CtrlReq::ToggleSync); DispatchResult::Handled }
    "last-pane" | "lastp" => { let _ = ctx.tx.send(CtrlReq::LastPane); DispatchResult::Handled }
    "run-shell" | "run" => {
        let background = args.iter().any(|a| *a == "-b");
        let cmd_parts: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        let shell_cmd = cmd_parts.join(" ");
        let shell_cmd = shell_cmd.trim_matches(|c: char| c == '\'' || c == '"').to_string();
        let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd);
        if shell_cmd.is_empty() {
            if !ctx.persistent {
                let _ = write!(ctx.write_stream, "usage: run-shell [-b] shell-command\n");
                let _ = ctx.write_stream.flush();
            }
        } else {
            if background {
                let mut c = crate::commands::build_run_shell_command(&shell_cmd);
                let _ = c.spawn();
            } else {
                let mut c = crate::commands::build_run_shell_command(&shell_cmd);
                let result = c.output();
                match result {
                    Ok(out) => {
                        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                        let stderr_text = String::from_utf8_lossy(&out.stderr);
                        if !stderr_text.is_empty() {
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            text.push_str(&stderr_text);
                        }
                        if !text.is_empty() {
                            if ctx.persistent {
                                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("run-shell".to_string(), text));
                            } else {
                                let _ = write!(ctx.write_stream, "{}", text);
                                let _ = ctx.write_stream.flush();
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("run-shell: {}\n", e);
                        if ctx.persistent {
                            let _ = ctx.tx.send(CtrlReq::StatusMessage(err_msg));
                        } else {
                            let _ = write!(ctx.write_stream, "{}", err_msg);
                            let _ = ctx.write_stream.flush();
                        }
                    }
                }
            }
        }
        DispatchResult::Handled
    }
    "if-shell" | "if" => {
        let format_mode = args.iter().any(|a| *a == "-F" || *a == "-bF" || *a == "-Fb");
        let positional: Vec<&str> = args.iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if positional.len() >= 2 {
            let condition = positional[0];
            let true_cmd = positional[1];
            let false_cmd = positional.get(2).copied();
            let success = if format_mode {
                let (rtx, rrx) = std::sync::mpsc::channel::<String>();
                let _ = ctx.tx.send(CtrlReq::DisplayMessage(rtx, condition.to_string(), None, false, None));
                let expanded = rrx.recv().unwrap_or_default();
                !expanded.is_empty() && expanded != "0"
            } else if condition == "true" || condition == "1" {
                true
            } else if condition == "false" || condition == "0" {
                false
            } else {
                let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
                let mut c = std::process::Command::new(&shell_prog);
                for a in &shell_args { c.arg(a); }
                c.arg(condition);
                c.stdout(std::process::Stdio::null());
                c.stderr(std::process::Stdio::null());
                { use crate::platform::HideWindowCommandExt; c.hide_window(); }
                c.status().map(|s| s.success()).unwrap_or(false)
            };
            let cmd_to_run = if success { Some(true_cmd) } else { false_cmd };
            if let Some(chosen) = cmd_to_run {
                return DispatchResult::ContinueWith(chosen.to_string());
            }
        }
        DispatchResult::Handled
    }
    "wait-for" => {
        let lock = args.iter().any(|a| *a == "-L");
        let signal = args.iter().any(|a| *a == "-S");
        let unlock = args.iter().any(|a| *a == "-U");
        let channel = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let op = if lock { WaitForOp::Lock }
            else if signal { WaitForOp::Signal }
            else if unlock { WaitForOp::Unlock }
            else { WaitForOp::Wait };
        let _ = ctx.tx.send(CtrlReq::WaitFor(channel, op));
        DispatchResult::Handled
    }
    "kill-server" => { let _ = ctx.tx.send(CtrlReq::KillServer); DispatchResult::Handled }
    "choose-tree" | "choose-window" | "choose-session" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ListTree(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("choose-tree".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "clear-history" | "clearhist" => { let _ = ctx.tx.send(CtrlReq::ClearHistory); DispatchResult::Handled }
    "lock-client" | "lockc" => { let _ = ctx.tx.send(CtrlReq::LockClient); DispatchResult::Handled }
    "refresh-client" | "refresh" => { let _ = ctx.tx.send(CtrlReq::RefreshClient); DispatchResult::Handled }
    "suspend-client" | "suspendc" => { let _ = ctx.tx.send(CtrlReq::SuspendClient); DispatchResult::Handled }
    "lock-server" | "lock-session" | "lock" | "locks" => { DispatchResult::Handled }
    "start-server" => {
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "list-commands" | "lscm" => {
        let cmds = TMUX_COMMANDS.join("\n");
        if ctx.persistent {
            let _ = ctx.tx.send(CtrlReq::ShowTextPopup("list-commands".to_string(), cmds));
        } else {
            let _ = write!(ctx.write_stream, "{}\n", cmds);
            let _ = ctx.write_stream.flush();
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "server-info" | "info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ServerInfo(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("server-info".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "run-command" | "runcmd" => {
        let full_cmd = args.join(" ");
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::RunCommand(full_cmd, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(15)) {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::StatusMessage(resp));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", resp);
                let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "clear-prompt-history" | "clearphist" => { let _ = ctx.tx.send(CtrlReq::ClearPromptHistory); DispatchResult::Handled }
    "show-prompt-history" | "showphist" => { let _ = ctx.tx.send(CtrlReq::ShowPromptHistory(ctx.persistent)); DispatchResult::Handled }
    "server-access" => { DispatchResult::Handled }
    _ => DispatchResult::Unhandled,
    }
}
