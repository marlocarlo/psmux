use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
struct TrackedPane {
    pane_id: String,
    window_id: String,
    actual_pane_id: String,
    actual_window_id: String,
    session: String,
    last_capture: Option<String>,
    closed: bool,
}

type TrackedPanes = Arc<Mutex<HashMap<String, TrackedPane>>>;
type ControlWriter = Arc<Mutex<io::BufWriter<io::Stdout>>>;

pub(crate) fn run_control_mode(
    namespace: Option<&str>,
    attached_session: Option<&str>,
    no_echo: bool,
) -> io::Result<()> {
    let tracked: TrackedPanes = Arc::new(Mutex::new(HashMap::new()));
    let writer: ControlWriter = Arc::new(Mutex::new(io::BufWriter::new(io::stdout())));
    let stop_notifications = Arc::new(AtomicBool::new(false));
    let _notification_thread = start_notification_thread(
        namespace.map(str::to_string),
        tracked.clone(),
        stop_notifications.clone(),
        writer.clone(),
    );

    {
        let mut out = writer
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "control writer poisoned"))?;
        if no_echo {
            out.write_all(b"\x1bP1000p")?;
        }
        if let Some(session) = attached_session {
            if !session.is_empty() {
                writeln!(
                    out,
                    "%session-changed $0 {}",
                    session.rsplit_once("__").map(|(_, s)| s).unwrap_or(session)
                )?;
            }
        }
        out.flush()?;
    }

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let req_id = next_command_id();
        let ts = unix_time();
        match execute_control_command(namespace, trimmed, &tracked) {
            Ok(result) => {
                let mut out = writer
                    .lock()
                    .map_err(|_| io::Error::new(io::ErrorKind::Other, "control writer poisoned"))?;
                writeln!(out, "%begin {ts} {req_id} 1")?;
                if !result.stdout.is_empty() {
                    write!(out, "{}", result.stdout)?;
                    if !result.stdout.ends_with('\n') {
                        writeln!(out)?;
                    }
                }
                let created = result.created;
                writeln!(out, "%end {ts} {req_id} 1")?;
                for notification in result.notifications {
                    writeln!(out, "{notification}")?;
                }
                if let Some((pane_id, window_id, actual_pane_id, actual_window_id, session)) =
                    created
                {
                    let pane = TrackedPane {
                        pane_id: pane_id.clone(),
                        window_id: window_id.clone(),
                        actual_pane_id,
                        actual_window_id,
                        session,
                        last_capture: None,
                        closed: false,
                    };
                    let pane = TrackedPane {
                        last_capture: capture_pane(&pane).ok().or_else(|| Some(String::new())),
                        ..pane
                    };
                    if let Ok(mut map) = tracked.lock() {
                        map.insert(pane_id, pane);
                    }
                    writeln!(out, "%window-add {window_id}")?;
                }
                out.flush()?;
            }
            Err(err) => {
                let mut out = writer
                    .lock()
                    .map_err(|_| io::Error::new(io::ErrorKind::Other, "control writer poisoned"))?;
                writeln!(out, "%begin {ts} {req_id} 1")?;
                writeln!(out, "{err}")?;
                writeln!(out, "%error {ts} {req_id} 1")?;
                out.flush()?;
            }
        }
    }

    stop_notifications.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(50));
    if no_echo {
        let mut out = writer
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "control writer poisoned"))?;
        writeln!(out, "%exit")?;
        out.write_all(b"\x1b\\")?;
        out.flush()?;
    }
    Ok(())
}

#[derive(Default)]
struct CommandResult {
    stdout: String,
    created: Option<(String, String, String, String, String)>,
    notifications: Vec<String>,
}

fn execute_control_command(
    namespace: Option<&str>,
    line: &str,
    tracked: &TrackedPanes,
) -> io::Result<CommandResult> {
    let parts = crate::commands::parse_command_line(line);
    if parts.is_empty() {
        return Ok(CommandResult::default());
    }

    let mut args = Vec::new();
    let target = target_arg(&parts);
    let created_session_hint = if matches!(parts[0].as_str(), "new-session" | "new")
        && session_arg(&parts).or_else(|| target_arg(&parts)).is_none()
    {
        Some(crate::session::next_session_name(namespace))
    } else {
        None
    };
    let target_pane = target
        .as_deref()
        .and_then(|target| resolve_tracked_pane(tracked, namespace, target));
    let target_session_env = target_pane.as_ref().map(|pane| pane.session.clone());

    if should_pass_namespace(namespace, &parts, target.as_deref()) {
        if let Some(ns) = namespace {
            args.push("-L".to_string());
            args.push(ns.to_string());
        }
    }
    args.extend(rewrite_target_arg(&parts, target_pane.as_ref()));

    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut cmd = Command::new(exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("PSMUX_REMOTE_ATTACH")
        .env_remove("PSMUX_SESSION_NAME");
    if let Some(session) = target_session_env {
        cmd.env("PSMUX_TARGET_SESSION", session);
    }
    let output = cmd.output()?;
    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let msg = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(io::Error::new(io::ErrorKind::Other, msg.to_string()));
    }

    let created = if matches!(parts[0].as_str(), "new-session" | "new") {
        parse_created_session(
            &parts,
            &mut stdout,
            namespace,
            created_session_hint.as_deref(),
        )
    } else {
        None
    };
    let mut notifications = Vec::new();
    if matches!(parts[0].as_str(), "kill-session" | "kill-ses") {
        let session = target.clone().or_else(|| session_arg(&parts));
        let closed = panes_for_session(tracked, namespace, session.as_deref());
        mark_closed_by_session(tracked, namespace, session.as_deref());
        for pane in closed {
            notifications.push(format!("%window-close {} 0", pane.window_id));
        }
    }

    if stdout.ends_with("\r\n") {
        stdout = stdout.replace("\r\n", "\n");
    }
    Ok(CommandResult {
        stdout,
        created,
        notifications,
    })
}

fn should_pass_namespace(namespace: Option<&str>, parts: &[String], target: Option<&str>) -> bool {
    let Some(ns) = namespace else {
        return false;
    };
    if matches!(
        parts[0].as_str(),
        "new-session" | "new" | "has-session" | "list-sessions" | "ls"
    ) {
        return true;
    }
    if let Some(target) = target {
        return !target.starts_with(&format!("{ns}__"));
    }
    true
}

fn target_arg(parts: &[String]) -> Option<String> {
    parts
        .windows(2)
        .find(|w| w[0] == "-t")
        .map(|w| w[1].clone())
}

fn session_arg(parts: &[String]) -> Option<String> {
    parts
        .windows(2)
        .find(|w| w[0] == "-s")
        .map(|w| w[1].clone())
}

fn rewrite_target_arg(parts: &[String], target_pane: Option<&TrackedPane>) -> Vec<String> {
    let Some(pane) = target_pane else {
        return parts.to_vec();
    };
    let mut rewritten = parts.to_vec();
    let mut i = 0;
    while i + 1 < rewritten.len() {
        if rewritten[i] == "-t" {
            if rewritten[i + 1] == pane.pane_id {
                rewritten[i + 1] = pane.actual_pane_id.clone();
            } else if rewritten[i + 1] == pane.window_id {
                rewritten[i + 1] = pane.actual_window_id.clone();
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    rewritten
}

fn parse_created_session(
    parts: &[String],
    stdout: &mut String,
    namespace: Option<&str>,
    fallback_session: Option<&str>,
) -> Option<(String, String, String, String, String)> {
    let session = session_arg(parts)
        .or_else(|| target_arg(parts))
        .or_else(|| fallback_session.map(str::to_string))?;
    let mut actual_pane_id = None;
    let mut actual_window_id = None;
    for token in stdout.split_whitespace() {
        if token.starts_with('%') {
            actual_pane_id = Some(token.to_string());
        } else if token.starts_with('@') {
            actual_window_id = Some(token.to_string());
        }
    }
    let actual_pane_id = actual_pane_id?;
    let actual_window_id = actual_window_id?;
    let synthetic = next_control_id();
    let pane_id = format!("%{synthetic}");
    let window_id = format!("@{synthetic}");
    *stdout = stdout
        .replace(&actual_pane_id, &pane_id)
        .replace(&actual_window_id, &window_id);
    let session = if session.contains("__") {
        session
    } else if let Some(ns) = namespace {
        format!("{ns}__{session}")
    } else {
        session
    };
    Some((
        pane_id,
        window_id,
        actual_pane_id,
        actual_window_id,
        session,
    ))
}

fn mark_closed_by_session(tracked: &TrackedPanes, namespace: Option<&str>, target: Option<&str>) {
    let Some(target) = target else {
        return;
    };
    let session = if target.contains("__") {
        target.to_string()
    } else if let Some(ns) = namespace {
        format!("{ns}__{target}")
    } else {
        target.to_string()
    };
    if let Ok(mut map) = tracked.lock() {
        map.retain(|_, pane| pane.session != session);
    }
}

fn resolve_tracked_pane(
    tracked: &TrackedPanes,
    namespace: Option<&str>,
    target: &str,
) -> Option<TrackedPane> {
    let map = tracked.lock().ok()?;
    if target.starts_with('%') {
        return map.get(target).cloned();
    }
    let session = if target.contains("__") {
        target.to_string()
    } else if let Some(ns) = namespace {
        format!("{ns}__{target}")
    } else {
        target.to_string()
    };
    map.values().find(|pane| pane.session == session).cloned()
}

fn panes_for_session(
    tracked: &TrackedPanes,
    namespace: Option<&str>,
    target: Option<&str>,
) -> Vec<TrackedPane> {
    let Some(target) = target else {
        return Vec::new();
    };
    let session = if target.contains("__") {
        target.to_string()
    } else if let Some(ns) = namespace {
        format!("{ns}__{target}")
    } else {
        target.to_string()
    };
    tracked
        .lock()
        .map(|map| {
            map.values()
                .filter(|pane| pane.session == session)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn start_notification_thread(
    namespace: Option<String>,
    tracked: TrackedPanes,
    stop: Arc<AtomicBool>,
    writer: ControlWriter,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
        if stop.load(Ordering::Acquire) {
            break;
        }
        let panes: Vec<TrackedPane> = tracked
            .lock()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default();
        for mut pane in panes {
            if pane.closed || !session_exists(namespace.as_deref(), &pane.session) {
                emit_notification(&writer, &format!("%window-close {} 0", pane.window_id));
                if let Ok(mut map) = tracked.lock() {
                    map.remove(&pane.pane_id);
                }
                continue;
            }
            if let Ok(mut captured) = capture_pane(&pane) {
                let Some(last_capture) = pane.last_capture.as_deref() else {
                    if let Ok(mut map) = tracked.lock() {
                        if let Some(slot) = map.get_mut(&pane.pane_id) {
                            slot.last_capture = Some(captured);
                        }
                    }
                    continue;
                };
                if captured != last_capture {
                    std::thread::sleep(Duration::from_millis(200));
                    if let Ok(stable_capture) = capture_pane(&pane) {
                        captured = stable_capture;
                    }
                    let delta = if captured.starts_with(last_capture) {
                        &captured[last_capture.len()..]
                    } else {
                        captured.as_str()
                    };
                    if !delta.is_empty() {
                        if !stop.load(Ordering::Acquire) {
                            emit_notification(
                                &writer,
                                &format!(
                                    "%output {} {}",
                                    pane.pane_id,
                                    escape_control_output(delta)
                                ),
                            );
                        }
                    }
                    pane.last_capture = Some(captured);
                    if let Ok(mut map) = tracked.lock() {
                        if let Some(slot) = map.get_mut(&pane.pane_id) {
                            slot.last_capture = pane.last_capture.clone();
                        }
                    }
                }
            }
        }
    })
}

fn emit_notification(writer: &ControlWriter, line: &str) {
    if let Ok(mut out) = writer.lock() {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

fn capture_pane(pane: &TrackedPane) -> io::Result<String> {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let output = Command::new(exe)
        .args(["capture-pane", "-t", pane.actual_pane_id.as_str(), "-p"])
        .env("PSMUX_TARGET_SESSION", &pane.session)
        .env_remove("PSMUX_REMOTE_ATTACH")
        .env_remove("PSMUX_SESSION_NAME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "capture-pane failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"))
}

fn session_exists(namespace: Option<&str>, session: &str) -> bool {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut cmd = Command::new(exe);
    if let Some(ns) = namespace {
        let logical = session.strip_prefix(&format!("{ns}__")).unwrap_or(session);
        cmd.args(["-L", ns, "has-session", "-t", logical]);
    } else {
        cmd.args(["has-session", "-t", session]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("PSMUX_REMOTE_ATTACH")
        .env_remove("PSMUX_SESSION_NAME")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn escape_control_output(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if (ch as u32) < 32 || ch == '\\' {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("\\{:03o}", b));
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn next_command_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_control_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_control_bytes_like_tmux_control_output() {
        assert_eq!(escape_control_output("a\nb\\c\r"), "a\\012b\\134c\\015");
    }

    #[test]
    fn preserves_utf8_printable_text_in_control_output() {
        assert_eq!(escape_control_output("한글\n"), "한글\\012");
    }

    #[test]
    fn parses_created_session_from_print_output() {
        let parts = crate::commands::parse_command_line(
            "new-session -d -s worker -P -F '#{pane_id} #{window_id}'",
        );
        let mut stdout = "%7 @3\n".to_string();
        let parsed = parse_created_session(&parts, &mut stdout, Some("paperclip"), None).unwrap();
        assert!(parsed.0.starts_with('%'));
        assert!(parsed.1.starts_with('@'));
        assert_ne!(parsed.0, "%7");
        assert_ne!(parsed.1, "@3");
        assert_eq!(parsed.2, "%7");
        assert_eq!(parsed.3, "@3");
        assert_eq!(parsed.4, "paperclip__worker");
    }

    #[test]
    fn parses_created_session_with_fallback_session_name() {
        let parts =
            crate::commands::parse_command_line("new-session -d -P -F '#{pane_id} #{window_id}'");
        let mut stdout = "%8 @4\n".to_string();
        let parsed =
            parse_created_session(&parts, &mut stdout, Some("paperclip"), Some("0")).unwrap();
        assert_eq!(parsed.2, "%8");
        assert_eq!(parsed.3, "@4");
        assert_eq!(parsed.4, "paperclip__0");
        assert!(stdout.starts_with('%'));
        assert!(!stdout.contains("%8"));
        assert!(!stdout.contains("@4"));
    }

    #[test]
    fn rewrites_only_target_synthetic_ids() {
        let pane = TrackedPane {
            pane_id: "%1000".to_string(),
            window_id: "@1000".to_string(),
            actual_pane_id: "%7".to_string(),
            actual_window_id: "@3".to_string(),
            session: "paperclip__worker".to_string(),
            last_capture: None,
            closed: false,
        };
        let parts = crate::commands::parse_command_line("send-keys -t %1000 echo %1000 Enter");
        let rewritten = rewrite_target_arg(&parts, Some(&pane));
        assert_eq!(rewritten[2], "%7");
        assert_eq!(rewritten[4], "%1000");
    }

    #[test]
    fn does_not_double_namespace_full_targets() {
        let parts =
            crate::commands::parse_command_line("send-keys -t paperclip__worker hello Enter");
        assert!(!should_pass_namespace(
            Some("paperclip"),
            &parts,
            Some("paperclip__worker")
        ));
    }
}
