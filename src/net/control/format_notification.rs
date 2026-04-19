#[allow(unused_imports)]
use crate::types::{AppState, ControlNotification};

/// Format a control mode notification as a tmux wire-compatible line.
use super::*;

pub fn format_notification(notif: &ControlNotification) -> String {
    match notif {
        ControlNotification::Output { pane_id, data } => {
            format!("%output %{} {}", pane_id, escape_output(data))
        }
        ControlNotification::WindowAdd { window_id } => {
            format!("%window-add @{}", window_id)
        }
        ControlNotification::WindowClose { window_id } => {
            format!("%window-close @{}", window_id)
        }
        ControlNotification::WindowRenamed { window_id, name } => {
            format!("%window-renamed @{} {}", window_id, name)
        }
        ControlNotification::WindowPaneChanged { window_id, pane_id } => {
            format!("%window-pane-changed @{} %{}", window_id, pane_id)
        }
        ControlNotification::LayoutChange { window_id, layout } => {
            // tmux sends: %layout-change @WID layout visible_layout flags
            // visible_layout and flags mirror layout and empty flags for now
            format!("%layout-change @{} {} {} *", window_id, layout, layout)
        }
        ControlNotification::SessionChanged { session_id, name } => {
            format!("%session-changed ${} {}", session_id, name)
        }
        ControlNotification::SessionRenamed { name } => {
            format!("%session-renamed {}", name)
        }
        ControlNotification::SessionWindowChanged { session_id, window_id } => {
            format!("%session-window-changed ${} @{}", session_id, window_id)
        }
        ControlNotification::SessionsChanged => {
            "%sessions-changed".to_string()
        }
        ControlNotification::PaneModeChanged { pane_id } => {
            format!("%pane-mode-changed %{}", pane_id)
        }
        ControlNotification::ClientDetached { client } => {
            format!("%client-detached {}", client)
        }
        ControlNotification::Continue { pane_id } => {
            format!("%continue %{}", pane_id)
        }
        ControlNotification::Pause { pane_id } => {
            format!("%pause %{}", pane_id)
        }
        ControlNotification::ExtendedOutput { pane_id, age_ms, data } => {
            format!("%extended-output %{} {} : {}", pane_id, age_ms, escape_output(data))
        }
        ControlNotification::SubscriptionChanged { name, session_id, window_id, window_index, pane_id, value } => {
            format!("%subscription-changed {} ${} @{} {} %{} - {}", name, session_id, window_id, window_index, pane_id, value)
        }
        ControlNotification::Exit { reason } => {
            if let Some(r) = reason {
                format!("%exit {}", r)
            } else {
                "%exit".to_string()
            }
        }
        ControlNotification::PasteBufferChanged { name } => {
            format!("%paste-buffer-changed {}", name)
        }
        ControlNotification::PasteBufferDeleted { name } => {
            format!("%paste-buffer-deleted {}", name)
        }
        ControlNotification::ClientSessionChanged { client, session_id, name } => {
            format!("%client-session-changed {} ${} {}", client, session_id, name)
        }
        ControlNotification::Message { text } => {
            format!("%message {}", text)
        }
    }
}

/// Escape non-printable bytes as octal \\NNN sequences (tmux compatible).
/// Printable ASCII (0x20..=0x7E), space, and tab are passed through.
/// Backslash is escaped as \\134 (octal) per the tmux protocol.
pub fn escape_output(data: &str) -> String {
    let mut out = String::with_capacity(data.len());
    for b in data.bytes() {
        match b {
            b'\\' => out.push_str("\\134"),
            0x20..=0x7E => out.push(b as char),
            b'\t' => out.push('\t'),
            _ => {
                out.push_str(&format!("\\{:03o}", b));
            }
        }
    }
    out
}

/// Format the %begin header for a command response.
pub fn format_begin(timestamp: i64, cmd_number: u64) -> String {
    format!("%begin {} {} 1", timestamp, cmd_number)
}

/// Format the %end footer for a successful command response.
pub fn format_end(timestamp: i64, cmd_number: u64) -> String {
    format!("%end {} {} 1", timestamp, cmd_number)
}

/// Format the %error footer for a failed command response.
pub fn format_error(timestamp: i64, cmd_number: u64) -> String {
    format!("%error {} {} 1", timestamp, cmd_number)
}

/// Emit a control notification to all connected control mode clients.
/// Non-blocking: if a client's channel is full, the notification is dropped for that client.
pub fn emit_notification(app: &AppState, notif: ControlNotification) {
    for client in app.control_clients.values() {
        if let ControlNotification::Output { pane_id, .. } = &notif {
            if client.paused_panes.contains(pane_id) {
                continue;
            }
        }
        let _ = client.notification_tx.try_send(notif.clone());
    }
}

/// Check if any control mode clients are connected.
pub fn has_control_clients(app: &AppState) -> bool {
    !app.control_clients.is_empty()
}
