use super::*;
use std::net::TcpStream;
use super::conn_dispatch::{DispatchCtx, DispatchResult};

pub(crate) fn dispatch(ctx: &mut DispatchCtx, cmd: &str, args: &[&str]) -> DispatchResult {
    match cmd {
    "set-option" | "set" | "set-window-option" | "setw" => {
        let combined_has_set = |ch: char| -> bool {
            args.iter().any(|a| {
                if *a == format!("-{}", ch) { return true; }
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
            })
        };
        let has_u = combined_has_set('u');
        let has_a = combined_has_set('a');
        let has_q = combined_has_set('q');
        let has_o = combined_has_set('o');
        let t_targets: std::collections::HashSet<&str> = args.windows(2)
            .filter(|w| w[0] == "-t" || w[0] == "-p" || w[0] == "-w")
            .map(|w| w[1]).collect();
        let non_flag_args: Vec<&str> = args.iter()
            .filter(|a| (!a.starts_with('-') || a.starts_with('@')) && !t_targets.contains(*a))
            .copied().collect();
        if has_u {
            if let Some(option) = non_flag_args.first() {
                let _ = ctx.tx.send(CtrlReq::SetOptionUnset(option.to_string()));
            }
        } else if non_flag_args.len() >= 2 {
            let option = non_flag_args[0].to_string();
            let value = non_flag_args[1..].join(" ");
            if has_a {
                let _ = ctx.tx.send(CtrlReq::SetOptionAppend(option, value));
            } else if has_o {
                let _ = ctx.tx.send(CtrlReq::SetOptionOnlyIfUnset(option, value));
            } else {
                let _ = ctx.tx.send(CtrlReq::SetOptionQuiet(option, value, has_q));
            }
        } else if non_flag_args.len() == 1 && has_q {
            // set -q <option> with no value: silently ignore
        }
        DispatchResult::Handled
    }
    "show-options" | "show" | "show-window-options" | "showw" => {
        let combined_has = |ch: char| -> bool {
            args.iter().any(|a| {
                if *a == format!("-{}", ch) { return true; }
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
            })
        };
        let has_a = combined_has('A');
        let _has_s = combined_has('s');
        let has_w = combined_has('w');
        let window_scope = matches!(cmd, "show-window-options" | "showw") || has_w;
        let has_v = combined_has('v');
        let has_q = combined_has('q');
        let opt_name: Option<&str> = args.iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .last();
        if has_v && opt_name.is_some() || (opt_name.is_some() && !has_q) {
            if let Some(name) = opt_name {
                let (rtx, rrx) = mpsc::channel::<String>();
                if window_scope {
                    let _ = ctx.tx.send(CtrlReq::ShowWindowOptionValue(rtx, name.to_string()));
                } else {
                    let _ = ctx.tx.send(CtrlReq::ShowOptionValue(rtx, name.to_string()));
                }
                if let Ok(text) = rrx.recv() {
                    let resolved = if text.is_empty() && window_scope && has_a {
                        let (frtx, frrx) = mpsc::channel::<String>();
                        let _ = ctx.tx.send(CtrlReq::ShowOptionValue(frtx, name.to_string()));
                        frrx.recv().unwrap_or_default()
                    } else {
                        text
                    };
                    if !(has_q && resolved.is_empty()) {
                        let output = if has_v {
                            format!("{}\n", resolved)
                        } else {
                            format!("{} {}\n", name, resolved)
                        };
                        if ctx.persistent {
                            let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), output));
                        } else {
                            let _ = ctx.write_stream.write_all(output.as_bytes());
                            let _ = ctx.write_stream.flush();
                        }
                    }
                }
            }
        } else if has_v && opt_name.is_none() {
            let (rtx, rrx) = mpsc::channel::<String>();
            if window_scope {
                let _ = ctx.tx.send(CtrlReq::ShowWindowOptions(rtx));
            } else {
                let _ = ctx.tx.send(CtrlReq::ShowOptions(rtx));
            }
            if let Ok(text) = rrx.recv() {
                let values_only: String = text.lines()
                    .filter_map(|line| {
                        let trimmed = line.trim();
                        if trimmed.is_empty() { return None; }
                        if let Some(pos) = trimmed.find(' ') {
                            Some(&trimmed[pos + 1..])
                        } else {
                            Some(trimmed)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let output = if values_only.is_empty() { String::new() } else { format!("{}\n", values_only) };
                if ctx.persistent {
                    let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), output));
                } else {
                    let _ = ctx.write_stream.write_all(output.as_bytes());
                    let _ = ctx.write_stream.flush();
                }
            }
        } else {
            if window_scope {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = ctx.tx.send(CtrlReq::ShowWindowOptions(rtx));
                if let Ok(mut text) = rrx.recv() {
                    if has_a {
                        let (srtx, srrx) = mpsc::channel::<String>();
                        let _ = ctx.tx.send(CtrlReq::ShowOptions(srtx));
                        if let Ok(session_text) = srrx.recv() {
                            if !text.ends_with('\n') && !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&session_text);
                        }
                    }
                    if ctx.persistent {
                        let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), text));
                    } else {
                        let _ = write!(ctx.write_stream, "{}\n", text);
                        let _ = ctx.write_stream.flush();
                    }
                }
            } else {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = ctx.tx.send(CtrlReq::ShowOptions(rtx));
                if let Ok(text) = rrx.recv() {
                    if ctx.persistent {
                        let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), text));
                    } else {
                        let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
                    }
                }
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "source-file" | "source" => {
        let format_expand = args.iter().any(|a| *a == "-F");
        let parse_only = args.iter().any(|a| *a == "-n");
        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if !parse_only {
            if let Some(path) = non_flag_args.first() {
                let source_spec = if format_expand {
                    format!("-F {}", path)
                } else {
                    path.to_string()
                };
                let _ = ctx.tx.send(CtrlReq::SourceFile(source_spec));
            }
        }
        DispatchResult::Handled
    }
    "set-environment" | "setenv" => {
        let has_u = args.iter().any(|a| {
            if *a == "-u" { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('u')
        });
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_u {
            if let Some(key) = non_flag.first() {
                let _ = ctx.tx.send(CtrlReq::UnsetEnvironment(key.to_string()));
            }
        } else if non_flag.len() >= 2 {
            let _ = ctx.tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), non_flag[1].to_string()));
        } else if non_flag.len() == 1 {
            let _ = ctx.tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), String::new()));
        }
        DispatchResult::Handled
    }
    "show-environment" | "showenv" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ShowEnvironment(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-environment".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    "set-hook" => {
        let has_unset = args.iter().any(|a| *a == "-u" || *a == "-gu" || *a == "-ug");
        let has_append = args.iter().any(|a| *a == "-a" || *a == "-ga" || *a == "-ag");
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_unset {
            if let Some(name) = non_flag.first() {
                let _ = ctx.tx.send(CtrlReq::RemoveHook(name.to_string()));
            }
        } else if non_flag.len() >= 2 {
            let hook_name = non_flag[0];
            let hook_cmd = if let Some(pos) = ctx.line.find(hook_name) {
                ctx.line[pos + hook_name.len()..].trim().to_string()
            } else {
                non_flag[1..].join(" ")
            };
            if has_append {
                let _ = ctx.tx.send(CtrlReq::AppendHook(hook_name.to_string(), hook_cmd));
            } else {
                let _ = ctx.tx.send(CtrlReq::SetHook(hook_name.to_string(), hook_cmd));
            }
        }
        DispatchResult::Handled
    }
    "show-hooks" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = ctx.tx.send(CtrlReq::ShowHooks(rtx));
        if let Ok(text) = rrx.recv() {
            if ctx.persistent {
                let _ = ctx.tx.send(CtrlReq::ShowTextPopup("show-hooks".to_string(), text));
            } else {
                let _ = write!(ctx.write_stream, "{}\n", text); let _ = ctx.write_stream.flush();
            }
        }
        if ctx.persistent { DispatchResult::Handled } else { DispatchResult::Break }
    }
    _ => DispatchResult::Unhandled,
    }
}
