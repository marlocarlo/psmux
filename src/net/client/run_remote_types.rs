use super::*;

pub(crate) fn default_base_index() -> usize { 1 }
pub(crate) fn default_prediction_dimming() -> bool { dim_predictions_enabled() }
pub(crate) fn default_status_left_length() -> usize { 10 }
pub(crate) fn default_status_right_length() -> usize { 40 }
pub(crate) fn default_status_lines() -> usize { 1 }
pub(crate) fn default_status_visible() -> bool { true }
pub(crate) fn default_repeat_time() -> u64 { 500 }

#[derive(serde::Deserialize, Default)]
pub(crate) struct WinStatus {
    pub(crate) id: usize,
    pub(crate) name: String,
    pub(crate) active: bool,
    #[serde(default)]
    pub(crate) activity: bool,
    #[serde(default)]
    pub(crate) tab_text: String,
}

/// A single key binding synced from the server.
#[derive(serde::Deserialize, Clone, Debug)]
pub(crate) struct BindingEntry {
    /// Key table name (e.g. "prefix", "root")
    pub(crate) t: String,
    /// Key string (e.g. "C-a", "-", "F12")
    pub(crate) k: String,
    /// Command string (e.g. "split-window -v")
    pub(crate) c: String,
    /// Whether the binding is repeatable
    #[serde(default)]
    pub(crate) r: bool,
}

/// A menu item from server-side MenuMode
#[derive(serde::Deserialize, Clone, Debug, Default)]
pub(crate) struct ServerMenuItem {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) key: Option<String>,
    #[serde(default)]
    pub(crate) sep: bool,
}

/// A customize-mode option row from server
#[derive(serde::Deserialize, Clone, Debug, Default)]
pub(crate) struct CustomizeOption {
    /// Original index in the full options list
    pub(crate) i: usize,
    /// Option name
    pub(crate) n: String,
    /// Current value
    pub(crate) v: String,
    /// Scope (server/session/window/pane)
    pub(crate) s: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct DumpState {
    pub(crate) layout: LayoutJson,
    pub(crate) windows: Vec<WinStatus>,
    #[serde(default)]
    pub(crate) prefix: Option<String>,
    #[serde(default)]
    pub(crate) prefix2: Option<String>,
    #[serde(default)]
    pub(crate) tree: Vec<WinTree>,
    #[serde(default = "default_base_index")]
    pub(crate) base_index: usize,
    #[serde(default = "default_prediction_dimming")]
    pub(crate) prediction_dimming: bool,
    #[serde(default)]
    pub(crate) status_style: Option<String>,
    #[serde(default)]
    pub(crate) status_left: Option<String>,
    #[serde(default)]
    pub(crate) status_right: Option<String>,
    #[serde(default)]
    pub(crate) pane_border_style: Option<String>,
    #[serde(default)]
    pub(crate) pane_active_border_style: Option<String>,
    #[serde(default)]
    pub(crate) pane_border_hover_style: Option<String>,
    #[serde(default)]
    pub(crate) pane_border_status: Option<String>,
    #[serde(default)]
    pub(crate) pane_border_format: Option<String>,
    /// window-status-format (short key to save bandwidth)
    #[serde(default)]
    pub(crate) wsf: Option<String>,
    /// window-status-current-format
    #[serde(default)]
    pub(crate) wscf: Option<String>,
    /// window-status-separator
    #[serde(default)]
    pub(crate) wss: Option<String>,
    /// window-status-style
    #[serde(default)]
    pub(crate) ws_style: Option<String>,
    /// window-status-current-style
    #[serde(default)]
    pub(crate) wsc_style: Option<String>,
    /// clock-mode active
    #[serde(default)]
    pub(crate) clock_mode: bool,
    /// clock-mode-colour (tmux option)
    #[serde(default)]
    pub(crate) clock_colour: Option<String>,
    /// Dynamic key bindings from server
    #[serde(default)]
    pub(crate) bindings: Vec<BindingEntry>,
    /// When true, hardcoded default keybindings are suppressed (set by unbind-key -a)
    #[serde(default)]
    pub(crate) defaults_suppressed: bool,
    /// pwsh-mouse-selection option (mirror of server-side AppState field)
    #[serde(default)]
    pub(crate) pwsh_mouse_selection: bool,
    /// status-left-length (max display width for left status)
    #[serde(default = "default_status_left_length")]
    pub(crate) status_left_length: usize,
    /// status-right-length (max display width for right status)
    #[serde(default = "default_status_right_length")]
    pub(crate) status_right_length: usize,
    /// Number of status bar lines
    #[serde(default = "default_status_lines")]
    pub(crate) status_lines: usize,
    /// Custom format strings for additional status lines
    #[serde(default)]
    pub(crate) status_format: Vec<String>,
    /// mode-style for copy mode selection highlighting
    #[serde(default)]
    pub(crate) mode_style: Option<String>,
    /// status-position: "top" or "bottom"
    #[serde(default)]
    pub(crate) status_position: Option<String>,
    /// status-justify: "left", "centre", or "right"
    #[serde(default)]
    pub(crate) status_justify: Option<String>,
    /// Whether the status bar is visible (true) or hidden (false).
    /// Corresponds to `set-option status on/off`.
    #[serde(default = "default_status_visible")]
    pub(crate) status_visible: bool,
    /// Configured cursor style as DECSCUSR code (0-6) from server.
    /// Used as fallback when no child process has set a cursor shape.
    #[serde(default)]
    pub(crate) cursor_style_code: Option<u8>,
    /// One-shot clipboard text (base64-encoded) for OSC 52 delivery.
    #[serde(default)]
    pub(crate) clipboard_osc52: Option<String>,
    /// One-shot bell flag: server signals client to emit \x07 to the host terminal.
    #[serde(default)]
    pub(crate) bell: bool,
    /// Repeat key timeout in ms (default: 500, synced from server)
    #[serde(default = "default_repeat_time")]
    pub(crate) repeat_time: u64,
    /// Whether a pane is currently zoomed (borders should be hidden)
    #[serde(default)]
    pub(crate) zoomed: bool,
    // ── Server-side overlay state ──
    /// Popup overlay active
    #[serde(default)]
    pub(crate) popup_active: bool,
    #[serde(default)]
    pub(crate) popup_command: Option<String>,
    #[serde(default)]
    pub(crate) popup_width: Option<u16>,
    #[serde(default)]
    pub(crate) popup_height: Option<u16>,
    #[serde(default)]
    pub(crate) popup_lines: Vec<String>,
    #[serde(default)]
    pub(crate) popup_rows: Vec<crate::layout::RowRunsJson>,
    #[serde(default)]
    pub(crate) popup_has_pty: bool,
    /// Confirm overlay active
    #[serde(default)]
    pub(crate) confirm_active: bool,
    #[serde(default)]
    pub(crate) confirm_prompt: Option<String>,
    /// Menu overlay active
    #[serde(default)]
    pub(crate) menu_active: bool,
    #[serde(default)]
    pub(crate) menu_title: Option<String>,
    #[serde(default)]
    pub(crate) menu_selected: usize,
    #[serde(default)]
    pub(crate) menu_items: Vec<ServerMenuItem>,
    /// Display-panes overlay active
    #[serde(default)]
    pub(crate) display_panes: bool,
    /// Pane base index for display-panes numbering
    #[serde(default)]
    pub(crate) pane_base_index: usize,
    /// Status bar message from display-message (without -p)
    #[serde(default)]
    pub(crate) status_message: Option<String>,
    /// Customize-mode overlay active
    #[serde(default)]
    pub(crate) customize_active: bool,
    #[serde(default)]
    pub(crate) customize_selected: usize,
    #[serde(default)]
    pub(crate) customize_scroll: usize,
    #[serde(default)]
    pub(crate) customize_editing: bool,
    #[serde(default)]
    pub(crate) customize_cursor: usize,
    #[serde(default)]
    pub(crate) customize_edit_buf: Option<String>,
    #[serde(default)]
    pub(crate) customize_filter: Option<String>,
    #[serde(default)]
    pub(crate) customize_options: Vec<CustomizeOption>,
}
