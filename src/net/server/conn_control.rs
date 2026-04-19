use super::*;
use std::net::TcpStream;
use crate::cli::parse_target;

/// Dispatch a command from a control mode client.
/// Returns true if a response was sent through `resp_tx`, false for fire-and-forget commands.
pub(crate) fn dispatch_control_command(
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
        "list-windows" | "lsw" => {
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt) = format_str {
                let _ = tx.send(CtrlReq::ListWindowsFormat(rtx, fmt));
            } else {
                let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "list-panes" | "lsp" => {
            let all = args.iter().any(|a| *a == "-a");
            let session_scope = args.iter().any(|a| *a == "-s");
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            if all || session_scope {
                if let Some(fmt) = format_str {
                    let _ = tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt));
                } else {
                    let _ = tx.send(CtrlReq::ListAllPanes(rtx));
                }
            } else {
                if let Some(fmt) = format_str {
                    let _ = tx.send(CtrlReq::ListPanesFormat(rtx, fmt));
                } else {
                    let _ = tx.send(CtrlReq::ListPanes(rtx));
                }
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "display-message" | "display" => {
            let print_mode = args.iter().any(|a| *a == "-p");
            let raw_fmt = args.last().map(|s| s.trim_matches('"').to_string()).unwrap_or_default();
            let fmt = if raw_fmt.is_empty() {
                crate::commands::DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
            } else {
                raw_fmt
            };
            let target_pane_idx = if pane_is_id { None } else { target_pane };
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_mode, None));
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "new-window" | "neww" => {
            let name = args.windows(2).find(|w| w[0] == "-n").map(|w| w[1].trim_matches('"').to_string());
            let start_dir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
            let detached = args.iter().any(|a| *a == "-d");
            let print_info = args.iter().any(|a| *a == "-P");
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
            let cmd_str: Option<String> = args.iter()
                .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-n" && w[1] == **a))
                    && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a))
                    && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)))
                .map(|s| s.trim_matches('"').to_string());
            if print_info {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::NewWindowPrint(cmd_str, name, detached, start_dir, format_str, rtx));
                if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                    let _ = resp_tx.send(text);
                }
                true
            } else {
                let _ = tx.send(CtrlReq::NewWindow(cmd_str, name, detached, start_dir));
                let _ = resp_tx.send(String::new());
                true
            }
        }
        "split-window" | "splitw" => {
            let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
            let cmd_str = args.windows(2).find(|w| w[0] == "-c").map(|_| ()).and(None);
            let start_dir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
            let detached = args.iter().any(|a| *a == "-d");
            let print_info = args.iter().any(|a| *a == "-P");
            let format_str = args.windows(2).find(|w| w[0] == "-F").map(|w| w[1].trim_matches('"').to_string());
            let split_size: Option<(u16, bool)> = args.windows(2).find(|w| w[0] == "-p")
                .and_then(|w| w[1].trim_end_matches('%').parse::<u16>().ok())
                .map(|v| (v, true))
                .or_else(|| args.windows(2).find(|w| w[0] == "-l")
                    .and_then(|w| {
                        let raw = &w[1];
                        let is_pct = raw.ends_with('%');
                        raw.trim_end_matches('%').parse::<u16>().ok().map(|v| (v, is_pct))
                    }));
            let (rtx, rrx) = mpsc::channel::<String>();
            if print_info {
                let _ = tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, split_size, format_str, rtx));
            } else {
                let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, split_size, rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "send-keys" => {
            let literal = args.iter().any(|a| *a == "-l");
            let keys: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            let text = keys.join(" ");
            let _ = tx.send(CtrlReq::SendKeys(text, literal));
            let _ = resp_tx.send(String::new());
            true
        }
        "capture-pane" | "capturep" => {
            let start = args.windows(2).find(|w| w[0] == "-S").and_then(|w| w[1].parse::<i32>().ok());
            let end = args.windows(2).find(|w| w[0] == "-E").and_then(|w| w[1].parse::<i32>().ok());
            let styled = args.iter().any(|a| *a == "-e");
            let (rtx, rrx) = mpsc::channel::<String>();
            if styled {
                let _ = tx.send(CtrlReq::CapturePaneStyled(rtx, start, end));
            } else if start.is_some() || end.is_some() {
                let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start, end));
            } else {
                let _ = tx.send(CtrlReq::CapturePane(rtx));
            }
            if let Ok(text) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = resp_tx.send(text);
            }
            true
        }
        "kill-pane" | "killp" => {
            if pane_is_id {
                if let Some(pid) = target_pane {
                    let _ = tx.send(CtrlReq::KillPaneById(pid));
                }
            } else {
                let _ = tx.send(CtrlReq::KillPane);
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "kill-window" | "killw" => {
            let _ = tx.send(CtrlReq::KillWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "unlink-window" | "unlinkw" => {
            let _ = tx.send(CtrlReq::UnlinkWindow);
            let _ = resp_tx.send(String::new());
            true
        }
        "select-window" | "selectw" => {
            let _ = resp_tx.send(String::new());
            true
        }
        "select-pane" | "selectp" => {
            if let Some(t) = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].trim_matches('"').to_string()) {
                let _ = tx.send(CtrlReq::SetPaneTitle(t));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "rename-window" | "renamew" => {
            if let Some(name) = args.last() {
                let _ = tx.send(CtrlReq::RenameWindow(name.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        "rename-session" | "rename" => {
            if let Some(name) = args.last() {
                let _ = tx.send(CtrlReq::RenameSession(name.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(String::new());
            true
        }
        _ => super::conn_control_ext::dispatch_control_ext(
            cmd, args, tx, resp_tx, target_pane, pane_is_id, _raw_target, client_id,
        ),
    }
}
