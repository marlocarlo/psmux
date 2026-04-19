use super::*;

pub(crate) fn handle_options(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "set-option" | "set" | "set-window-option" | "setw" => {
            // Always apply locally first (fix #179: TCP server drops these)
            crate::config::parse_config_line(app, cmd);
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "bind-key" | "bind" => {
            // Always apply locally first (fix #179: TCP server drops these)
            crate::config::parse_config_line(app, cmd);
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "unbind-key" | "unbind" => {
            // Always apply locally first (fix #179: TCP server drops these)
            crate::config::parse_config_line(app, cmd);
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "source-file" | "source" => {
            // Always apply locally first for immediate visual feedback,
            // then forward to server for authoritative state update.
            if let Some(path) = parts.get(1) {
                crate::config::source_file(app, path);
            }
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
