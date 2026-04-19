use super::*;

pub(crate) fn is_window_option(name: &str) -> bool {
    matches!(
        name,
        "automatic-rename"
            | "monitor-activity"
            | "remain-on-exit"
            | "window-status-format"
            | "window-status-current-format"
            | "window-status-separator"
            | "window-status-style"
            | "window-status-current-style"
            | "window-status-activity-style"
            | "window-status-bell-style"
            | "window-status-last-style"
            | "main-pane-width"
            | "main-pane-height"
            | "window-size"
    )
}

pub(crate) fn get_window_option_value(app: &AppState, name: &str) -> String {
    if is_window_option(name) {
        get_option_value(app, name)
    } else {
        String::new()
    }
}

pub(crate) fn render_window_options(app: &AppState) -> String {
    let names = [
        "automatic-rename",
        "monitor-activity",
        "remain-on-exit",
        "window-status-format",
        "window-status-current-format",
        "window-status-separator",
        "window-status-style",
        "window-status-current-style",
        "window-status-activity-style",
        "window-status-bell-style",
        "window-status-last-style",
        "main-pane-width",
        "main-pane-height",
        "window-size",
    ];

    let mut output = String::new();
    for name in names {
        output.push_str(&format!("{} {}\n", name, get_option_value(app, name)));
    }
    output
}
