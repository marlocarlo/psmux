use super::*;

pub(crate) fn handle_layout_zoom(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "zoom-pane" | "zoom" | "resizep -Z" => {
            toggle_zoom(app);
        }
        "resize-pane" | "resizep" => {
            if parts.iter().any(|p| *p == "-Z") {
                toggle_zoom(app);
            } else if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                // Local resize
                let amount = parts.windows(2).find(|w| w[0] == "-x" || w[0] == "-y")
                    .and_then(|w| w[1].parse::<i16>().ok());
                if parts.iter().any(|p| *p == "-U" || *p == "-D") {
                    let amt = amount.unwrap_or(1);
                    let adj = if parts.iter().any(|p| *p == "-U") { -amt } else { amt };
                    crate::window_ops::resize_pane_vertical(app, adj);
                } else if parts.iter().any(|p| *p == "-L" || *p == "-R") {
                    let amt = amount.unwrap_or(1);
                    let adj = if parts.iter().any(|p| *p == "-L") { -amt } else { amt };
                    crate::window_ops::resize_pane_horizontal(app, adj);
                }
            }
        }
        "swap-pane" | "swapp" => {
            if let Some(port) = app.control_port {
                let dir = if parts.iter().any(|p| *p == "-U") { "-U" } else { "-D" };
                let _ = send_control_to_port(port, &format!("swap-pane {}\n", dir), &app.session_key);
            } else {
                let dir = if parts.iter().any(|p| *p == "-U") { FocusDir::Up } else { FocusDir::Down };
                crate::window_ops::swap_pane(app, dir);
            }
        }
        "rotate-window" | "rotatew" => {
            if let Some(port) = app.control_port {
                let flag = if parts.iter().any(|p| *p == "-D") { "-D" } else { "" };
                let _ = send_control_to_port(port, &format!("rotate-window {}\n", flag), &app.session_key);
            } else {
                crate::window_ops::rotate_panes(app, !parts.iter().any(|p| *p == "-D"));
            }
        }
        "select-layout" | "selectl" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let layout = parts.get(1).unwrap_or(&"tiled");
                crate::layout::apply_layout(app, layout);
            }
        }
        "next-layout" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "next-layout\n", &app.session_key);
            } else {
                crate::layout::cycle_layout(app);
            }
        }
        "previous-layout" | "prevl" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "previous-layout\n", &app.session_key);
            } else {
                crate::layout::cycle_layout_reverse(app);
            }
        }
        "resize-window" | "resizew" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
            // resize-window depends on terminal size, only meaningful on server
        }
        _ => return None,
    }
    Some(Ok(()))
}
