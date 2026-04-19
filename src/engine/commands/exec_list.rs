use super::*;

pub(crate) fn handle_list(app: &mut AppState, cmd: &str, parts: &[&str]) -> Option<io::Result<()>> {
    match parts[0] {
        "list-windows" | "lsw" => {
            let output = generate_list_windows(app);
            show_output_popup(app, "list-windows", output);
        }
        "list-panes" | "lsp" => {
            let output = generate_list_panes(app);
            show_output_popup(app, "list-panes", output);
        }
        "list-clients" | "lsc" => {
            let output = generate_list_clients(app);
            show_output_popup(app, "list-clients", output);
        }
        "list-commands" | "lscm" => {
            let output = generate_list_commands();
            show_output_popup(app, "list-commands", output);
        }
        "show-hooks" => {
            let output = generate_show_hooks(app);
            show_output_popup(app, "show-hooks", output);
        }
        "list-sessions" | "ls" => {
            // Show all sessions from filesystem
            let output = crate::session::list_session_names().join("\n") + "\n";
            show_output_popup(app, "list-sessions", output);
        }
        "list-keys" | "lsk" => {
            let mut output = String::new();
            for (table_name, binds) in &app.key_tables {
                for bind in binds {
                    let key_str = crate::config::format_key_binding(&bind.key);
                    let cmd_str = format_action(&bind.action);
                    output.push_str(&format!("bind-key -T {} {} {}\n", table_name, key_str, cmd_str));
                }
            }
            if output.is_empty() { output.push_str("(no bindings)\n"); }
            show_output_popup(app, "list-keys", output);
        }
        "show-options" | "show" | "show-window-options" | "showw" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            } else {
                let output = generate_show_options(app);
                show_output_popup(app, "show-options", output);
            }
        }
        "server-info" | "info" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "server-info\n", &app.session_key);
            } else {
                let output = format!("psmux {}\nSession: {}\nWindows: {}\nActive: {}\n",
                    crate::types::VERSION, app.session_name, app.windows.len(), app.active_idx);
                show_output_popup(app, "server-info", output);
            }
        }
        _ => return None,
    }
    Some(Ok(()))
}
