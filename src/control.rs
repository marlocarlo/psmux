use crate::types::{AppState, ControlNotification};

/// Format a control mode notification as a tmux wire-compatible line.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_output_printable() {
        assert_eq!(escape_output("hello world"), "hello world");
    }

    #[test]
    fn test_escape_output_backslash() {
        // tmux escapes backslash as octal \134
        assert_eq!(escape_output("a\\b"), "a\\134b");
    }

    #[test]
    fn test_escape_output_control_chars() {
        // \r = 0x0D = octal 015, \n = 0x0A = octal 012
        assert_eq!(escape_output("a\r\nb"), "a\\015\\012b");
    }

    #[test]
    fn test_escape_output_tab_passthrough() {
        assert_eq!(escape_output("a\tb"), "a\tb");
    }

    #[test]
    fn test_escape_output_high_bytes() {
        // U+FFFD replacement character = UTF-8 bytes ef bf bd = octal 357 277 275
        let data = String::from_utf8_lossy(b"x\xffy").to_string();
        assert_eq!(escape_output(&data), "x\\357\\277\\275y");
    }

    #[test]
    fn test_format_begin_end_error() {
        assert_eq!(format_begin(1700000000, 1), "%begin 1700000000 1 1");
        assert_eq!(format_end(1700000000, 1), "%end 1700000000 1 1");
        assert_eq!(format_error(1700000000, 1), "%error 1700000000 1 1");
    }

    #[test]
    fn test_format_notification_window_add() {
        let line = format_notification(&ControlNotification::WindowAdd { window_id: 3 });
        assert_eq!(line, "%window-add @3");
    }

    #[test]
    fn test_format_notification_output() {
        let line = format_notification(&ControlNotification::Output {
            pane_id: 1,
            data: "hello\r\n".to_string(),
        });
        assert_eq!(line, "%output %1 hello\\015\\012");
    }

    #[test]
    fn test_format_notification_exit() {
        let line = format_notification(&ControlNotification::Exit { reason: None });
        assert_eq!(line, "%exit");
        let line = format_notification(&ControlNotification::Exit {
            reason: Some("too far behind".to_string()),
        });
        assert_eq!(line, "%exit too far behind");
    }

    #[test]
    fn test_format_notification_session_renamed() {
        let line = format_notification(&ControlNotification::SessionRenamed {
            name: "my-session".to_string(),
        });
        assert_eq!(line, "%session-renamed my-session");
    }

    #[test]
    fn test_format_notification_layout_change() {
        let line = format_notification(&ControlNotification::LayoutChange {
            window_id: 2,
            layout: "5e08,120x30,0,0,1".to_string(),
        });
        // tmux format: %layout-change @WID layout visible_layout flags
        assert_eq!(line, "%layout-change @2 5e08,120x30,0,0,1 5e08,120x30,0,0,1 *");
    }

    #[test]
    fn test_format_notification_window_close() {
        let line = format_notification(&ControlNotification::WindowClose { window_id: 7 });
        assert_eq!(line, "%window-close @7");
    }

    #[test]
    fn test_format_notification_window_renamed() {
        let line = format_notification(&ControlNotification::WindowRenamed {
            window_id: 0,
            name: "editor".to_string(),
        });
        assert_eq!(line, "%window-renamed @0 editor");
    }

    #[test]
    fn test_format_notification_session_changed() {
        let line = format_notification(&ControlNotification::SessionChanged {
            session_id: 0,
            name: "main".to_string(),
        });
        assert_eq!(line, "%session-changed $0 main");
    }

    #[test]
    fn test_format_notification_session_window_changed() {
        let line = format_notification(&ControlNotification::SessionWindowChanged {
            session_id: 0,
            window_id: 5,
        });
        assert_eq!(line, "%session-window-changed $0 @5");
    }

    #[test]
    fn test_format_notification_window_pane_changed() {
        let line = format_notification(&ControlNotification::WindowPaneChanged {
            window_id: 2,
            pane_id: 4,
        });
        assert_eq!(line, "%window-pane-changed @2 %4");
    }

    #[test]
    fn test_format_notification_continue_pause() {
        assert_eq!(format_notification(&ControlNotification::Continue { pane_id: 1 }), "%continue %1");
        assert_eq!(format_notification(&ControlNotification::Pause { pane_id: 1 }), "%pause %1");
    }

    #[test]
    fn test_format_notification_client_detached() {
        let line = format_notification(&ControlNotification::ClientDetached { client: "client0".to_string() });
        assert_eq!(line, "%client-detached client0");
    }

    #[test]
    fn test_has_control_clients_empty() {
        let app = AppState::new("test".to_string());
        assert!(!has_control_clients(&app));
    }

    #[test]
    fn test_has_control_clients_with_client() {
        let mut app = AppState::new("test".to_string());
        let (tx, _rx) = std::sync::mpsc::sync_channel(16);
        app.control_clients.insert(1, crate::types::ControlClient {
            client_id: 1,
            cmd_counter: 0,
            echo_enabled: true,
            notification_tx: tx,
            paused_panes: std::collections::HashSet::new(),
            subscriptions: std::collections::HashMap::new(),
            subscription_values: std::collections::HashMap::new(),
            subscription_last_check: std::collections::HashMap::new(),
            pause_after_secs: None,
            output_paused_panes: std::collections::HashSet::new(),
            pane_last_output: std::collections::HashMap::new(),
        });
        assert!(has_control_clients(&app));
    }

    #[test]
    fn test_emit_notification_to_clients() {
        let mut app = AppState::new("test".to_string());
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        app.control_clients.insert(1, crate::types::ControlClient {
            client_id: 1,
            cmd_counter: 0,
            echo_enabled: false,
            notification_tx: tx,
            paused_panes: std::collections::HashSet::new(),
            subscriptions: std::collections::HashMap::new(),
            subscription_values: std::collections::HashMap::new(),
            subscription_last_check: std::collections::HashMap::new(),
            pause_after_secs: None,
            output_paused_panes: std::collections::HashSet::new(),
            pane_last_output: std::collections::HashMap::new(),
        });
        emit_notification(&app, ControlNotification::WindowAdd { window_id: 5 });
        let notif = rx.try_recv().unwrap();
        assert!(matches!(notif, ControlNotification::WindowAdd { window_id: 5 }));
    }

    #[test]
    fn test_emit_notification_skips_paused_pane() {
        let mut app = AppState::new("test".to_string());
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        let mut paused = std::collections::HashSet::new();
        paused.insert(3usize);
        app.control_clients.insert(1, crate::types::ControlClient {
            client_id: 1,
            cmd_counter: 0,
            echo_enabled: false,
            notification_tx: tx,
            paused_panes: paused,
            subscriptions: std::collections::HashMap::new(),
            subscription_values: std::collections::HashMap::new(),
            subscription_last_check: std::collections::HashMap::new(),
            pause_after_secs: None,
            output_paused_panes: std::collections::HashSet::new(),
            pane_last_output: std::collections::HashMap::new(),
        });
        // Output for paused pane 3 should be dropped
        emit_notification(&app, ControlNotification::Output { pane_id: 3, data: "test".into() });
        assert!(rx.try_recv().is_err(), "paused pane output should not be sent");
        // Output for different pane should go through
        emit_notification(&app, ControlNotification::Output { pane_id: 5, data: "ok".into() });
        assert!(rx.try_recv().is_ok(), "non-paused pane output should be sent");
    }

    #[test]
    fn test_escape_output_empty() {
        assert_eq!(escape_output(""), "");
    }

    #[test]
    fn test_escape_output_mixed() {
        // Mix of printable, backslash, control, and tab
        assert_eq!(escape_output("a\\b\tc\x01d"), "a\\134b\tc\\001d");
    }

    #[test]
    fn test_format_notification_extended_output() {
        let line = format_notification(&ControlNotification::ExtendedOutput {
            pane_id: 2,
            age_ms: 150,
            data: "hello\r\n".to_string(),
        });
        assert_eq!(line, "%extended-output %2 150 : hello\\015\\012");
    }

    #[test]
    fn test_format_notification_subscription_changed() {
        let line = format_notification(&ControlNotification::SubscriptionChanged {
            name: "mysub".to_string(),
            session_id: 0,
            window_id: 1,
            window_index: 0,
            pane_id: 3,
            value: "pwsh".to_string(),
        });
        assert_eq!(line, "%subscription-changed mysub $0 @1 0 %3 - pwsh");
    }

    #[test]
    fn test_format_notification_paste_buffer_changed() {
        let line = format_notification(&ControlNotification::PasteBufferChanged {
            name: "buffer0".to_string(),
        });
        assert_eq!(line, "%paste-buffer-changed buffer0");
    }

    #[test]
    fn test_format_notification_paste_buffer_deleted() {
        let line = format_notification(&ControlNotification::PasteBufferDeleted {
            name: "buffer1".to_string(),
        });
        assert_eq!(line, "%paste-buffer-deleted buffer1");
    }

    #[test]
    fn test_format_notification_client_session_changed() {
        let line = format_notification(&ControlNotification::ClientSessionChanged {
            client: "/dev/pts/0".to_string(),
            session_id: 2,
            name: "work".to_string(),
        });
        assert_eq!(line, "%client-session-changed /dev/pts/0 $2 work");
    }

    #[test]
    fn test_format_notification_message() {
        let line = format_notification(&ControlNotification::Message {
            text: "hello world".to_string(),
        });
        assert_eq!(line, "%message hello world");
    }

    #[test]
    fn test_format_notification_sessions_changed() {
        let line = format_notification(&ControlNotification::SessionsChanged);
        assert_eq!(line, "%sessions-changed");
    }

    #[test]
    fn test_format_notification_pane_mode_changed() {
        let line = format_notification(&ControlNotification::PaneModeChanged { pane_id: 7 });
        assert_eq!(line, "%pane-mode-changed %7");
    }

    #[test]
    fn test_format_notification_extended_output_with_escape() {
        let line = format_notification(&ControlNotification::ExtendedOutput {
            pane_id: 0,
            age_ms: 5000,
            data: "line1\\line2".to_string(),
        });
        assert_eq!(line, "%extended-output %0 5000 : line1\\134line2");
    }

    #[test]
    fn test_format_notification_subscription_changed_empty_value() {
        let line = format_notification(&ControlNotification::SubscriptionChanged {
            name: "test_sub".to_string(),
            session_id: 1,
            window_id: 2,
            window_index: 3,
            pane_id: 4,
            value: String::new(),
        });
        assert_eq!(line, "%subscription-changed test_sub $1 @2 3 %4 - ");
    }
}
