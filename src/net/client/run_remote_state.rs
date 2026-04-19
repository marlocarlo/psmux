use super::*;
use super::run_remote_types::*;

pub(crate) struct RunRemoteState {
    pub(crate) quit: bool,
    pub(crate) prefix_armed: bool,
    pub(crate) prefix_armed_at: Instant,
    pub(crate) prefix_repeating: bool,
    pub(crate) repeat_time_ms: u64,
    pub(crate) renaming: bool,
    pub(crate) session_renaming: bool,
    pub(crate) rename_buf: String,
    pub(crate) pane_renaming: bool,
    pub(crate) pane_title_buf: String,
    pub(crate) command_input: bool,
    pub(crate) command_buf: String,
    pub(crate) command_cursor: usize,
    pub(crate) command_history: Vec<String>,
    pub(crate) command_history_idx: usize,
    pub(crate) window_idx_input: bool,
    pub(crate) window_idx_buf: String,

    // Tree chooser
    pub(crate) tree_chooser: bool,
    pub(crate) tree_entries: Vec<(bool, usize, usize, String, String)>,
    pub(crate) tree_selected: usize,
    pub(crate) tree_scroll: usize,

    // Buffer chooser
    pub(crate) buffer_chooser: bool,
    pub(crate) buffer_entries: Vec<(usize, usize, String)>,
    pub(crate) buffer_selected: usize,
    pub(crate) buffer_scroll: usize,

    // Session chooser
    pub(crate) session_chooser: bool,
    pub(crate) session_entries: Vec<(String, String)>,
    pub(crate) session_selected: usize,
    pub(crate) session_scroll: usize,

    pub(crate) confirm_cmd: Option<String>,
    pub(crate) last_sent_size: (u16, u16),
    pub(crate) last_status_lines: u16,
    pub(crate) last_dump_time: Instant,
    pub(crate) force_dump: bool,
    pub(crate) last_tree: Vec<WinTree>,

    // Prefix key config
    pub(crate) prefix_key: (KeyCode, KeyModifiers),
    pub(crate) prefix_raw_char: Option<char>,
    pub(crate) prefix2_key: Option<(KeyCode, KeyModifiers)>,
    pub(crate) prefix2_raw_char: Option<char>,

    // Status bar style
    pub(crate) status_fg: Color,
    pub(crate) status_bg: Color,
    pub(crate) status_bold: bool,
    pub(crate) custom_status_left: Option<String>,
    pub(crate) custom_status_right: Option<String>,
    pub(crate) pane_border_fg: Color,
    pub(crate) pane_active_border_fg: Color,
    pub(crate) pane_border_hover_fg: Color,
    pub(crate) win_status_fmt: String,
    pub(crate) win_status_current_fmt: String,
    pub(crate) win_status_sep: String,
    pub(crate) win_status_style: Option<(Option<Color>, Option<Color>, bool)>,
    pub(crate) win_status_current_style: Option<(Option<Color>, Option<Color>, bool)>,
    pub(crate) mode_style_str: String,
    pub(crate) status_position_str: String,
    pub(crate) status_justify_str: String,

    // Synced bindings
    pub(crate) synced_bindings: Vec<BindingEntry>,
    pub(crate) defaults_suppressed: bool,

    // Windows paste detection state
    #[cfg(windows)]
    pub(crate) paste_pend: String,
    #[cfg(windows)]
    pub(crate) paste_pend_start: Option<Instant>,
    #[cfg(windows)]
    pub(crate) paste_stage2: bool,
    #[cfg(windows)]
    pub(crate) paste_confirmed: bool,
    #[cfg(windows)]
    pub(crate) paste_stage2_last_len: usize,
    #[cfg(windows)]
    pub(crate) paste_suppress_until: Option<Instant>,
    #[cfg(windows)]
    pub(crate) modified_enter_press_handled: bool,

    // Keys viewer
    pub(crate) keys_viewer: bool,
    pub(crate) keys_viewer_lines: Vec<String>,
    pub(crate) keys_viewer_scroll: usize,

    // Server-side overlay state
    pub(crate) srv_popup_active: bool,
    pub(crate) srv_popup_command: String,
    pub(crate) srv_popup_width: u16,
    pub(crate) srv_popup_height: u16,
    pub(crate) srv_popup_lines: Vec<String>,
    pub(crate) srv_popup_rows: Vec<crate::layout::RowRunsJson>,
    pub(crate) srv_popup_has_pty: bool,
    pub(crate) srv_popup_scroll: u16,
    pub(crate) srv_confirm_active: bool,
    pub(crate) srv_confirm_prompt: String,
    pub(crate) srv_menu_active: bool,
    pub(crate) srv_menu_title: String,
    pub(crate) srv_menu_selected: usize,
    pub(crate) srv_menu_items: Vec<ServerMenuItem>,
    pub(crate) srv_display_panes: bool,
    pub(crate) srv_pane_base_index: usize,
    pub(crate) clock_active: bool,
    pub(crate) clock_colour_str: Option<String>,

    // Customize-mode overlay state
    pub(crate) srv_customize_active: bool,
    pub(crate) srv_customize_selected: usize,
    pub(crate) srv_customize_scroll: usize,
    pub(crate) srv_customize_editing: bool,
    pub(crate) srv_customize_cursor: usize,
    pub(crate) srv_customize_edit_buf: String,
    pub(crate) srv_customize_filter: String,
    pub(crate) srv_customize_options: Vec<CustomizeOption>,

    // Dump and batch state
    pub(crate) cmd_batch: Vec<String>,
    pub(crate) dump_buf: String,
    pub(crate) prev_dump_buf: String,
    pub(crate) last_key_send_time: Option<Instant>,
    pub(crate) dump_in_flight: bool,
    pub(crate) dump_flight_start: Instant,

    // Latency diagnostics
    pub(crate) latency_log: Option<std::fs::File>,
    pub(crate) loop_count: u64,
    pub(crate) _last_key_char: Option<char>,
    pub(crate) key_send_instant: Option<Instant>,

    // Text selection state
    pub(crate) rsel_start: Option<(u16, u16)>,
    pub(crate) rsel_end: Option<(u16, u16)>,
    pub(crate) rsel_pane_rect: Option<Rect>,
    pub(crate) rsel_dragged: bool,
    pub(crate) last_click: Option<(Instant, (u16, u16))>,
    pub(crate) click_count: u32,
    pub(crate) rsel_block: bool,
    pub(crate) selection_changed: bool,
    pub(crate) border_drag: bool,

    // Client layout tracking
    pub(crate) client_tab_positions: Vec<(usize, u16, u16)>,
    pub(crate) client_status_row: u16,
    pub(crate) client_base_index: usize,
    pub(crate) client_pane_rects: Vec<(usize, Rect)>,
    pub(crate) client_borders: Vec<(Vec<usize>, String, usize, u16, u16, Vec<u16>, Rect)>,
    pub(crate) client_content_area: Rect,
    pub(crate) client_copy_mode: bool,
    pub(crate) client_pwsh_selection: bool,
    pub(crate) client_zoomed: bool,
    pub(crate) client_drag: Option<ClientDragState>,
    pub(crate) hovered_border: Option<(u16, String, Rect)>,

    // Post-draw state
    pub(crate) pending_osc52: Option<String>,
    pub(crate) pending_bell: bool,
    pub(crate) last_mouse_enable: Instant,
    pub(crate) last_cursor_style: u8,
}

impl RunRemoteState {
    pub(crate) fn new(latency_log: Option<std::fs::File>) -> Self {
        Self {
            quit: false,
            prefix_armed: false,
            prefix_armed_at: Instant::now(),
            prefix_repeating: false,
            repeat_time_ms: 500,
            renaming: false,
            session_renaming: false,
            rename_buf: String::new(),
            pane_renaming: false,
            pane_title_buf: String::new(),
            command_input: false,
            command_buf: String::new(),
            command_cursor: 0,
            command_history: Vec::new(),
            command_history_idx: 0,
            window_idx_input: false,
            window_idx_buf: String::new(),

            tree_chooser: false,
            tree_entries: Vec::new(),
            tree_selected: 0,
            tree_scroll: 0,

            buffer_chooser: false,
            buffer_entries: Vec::new(),
            buffer_selected: 0,
            buffer_scroll: 0,

            session_chooser: false,
            session_entries: Vec::new(),
            session_selected: 0,
            session_scroll: 0,

            confirm_cmd: None,
            last_sent_size: (0, 0),
            last_status_lines: 1,
            last_dump_time: Instant::now() - Duration::from_millis(250),
            force_dump: true,
            last_tree: Vec::new(),

            prefix_key: (KeyCode::Char('b'), KeyModifiers::CONTROL),
            prefix_raw_char: Some('\x02'),
            prefix2_key: None,
            prefix2_raw_char: None,

            status_fg: Color::Black,
            status_bg: Color::Green,
            status_bold: false,
            custom_status_left: None,
            custom_status_right: None,
            pane_border_fg: Color::DarkGray,
            pane_active_border_fg: Color::Green,
            pane_border_hover_fg: Color::Yellow,
            win_status_fmt: "#I:#W#{?window_flags,#{window_flags}, }".to_string(),
            win_status_current_fmt: "#I:#W#{?window_flags,#{window_flags}, }".to_string(),
            win_status_sep: " ".to_string(),
            win_status_style: None,
            win_status_current_style: None,
            mode_style_str: "bg=yellow,fg=black".to_string(),
            status_position_str: "bottom".to_string(),
            status_justify_str: "left".to_string(),

            synced_bindings: Vec::new(),
            defaults_suppressed: false,

            #[cfg(windows)]
            paste_pend: String::new(),
            #[cfg(windows)]
            paste_pend_start: None,
            #[cfg(windows)]
            paste_stage2: false,
            #[cfg(windows)]
            paste_confirmed: false,
            #[cfg(windows)]
            paste_stage2_last_len: 0,
            #[cfg(windows)]
            paste_suppress_until: None,
            #[cfg(windows)]
            modified_enter_press_handled: false,

            keys_viewer: false,
            keys_viewer_lines: Vec::new(),
            keys_viewer_scroll: 0,

            srv_popup_active: false,
            srv_popup_command: String::new(),
            srv_popup_width: 80,
            srv_popup_height: 24,
            srv_popup_lines: Vec::new(),
            srv_popup_rows: Vec::new(),
            srv_popup_has_pty: false,
            srv_popup_scroll: 0,
            srv_confirm_active: false,
            srv_confirm_prompt: String::new(),
            srv_menu_active: false,
            srv_menu_title: String::new(),
            srv_menu_selected: 0,
            srv_menu_items: Vec::new(),
            srv_display_panes: false,
            srv_pane_base_index: 0,
            clock_active: false,
            clock_colour_str: None,

            srv_customize_active: false,
            srv_customize_selected: 0,
            srv_customize_scroll: 0,
            srv_customize_editing: false,
            srv_customize_cursor: 0,
            srv_customize_edit_buf: String::new(),
            srv_customize_filter: String::new(),
            srv_customize_options: Vec::new(),

            cmd_batch: Vec::new(),
            dump_buf: String::new(),
            prev_dump_buf: String::new(),
            last_key_send_time: None,
            dump_in_flight: false,
            dump_flight_start: Instant::now(),

            latency_log,
            loop_count: 0,
            _last_key_char: None,
            key_send_instant: None,

            rsel_start: None,
            rsel_end: None,
            rsel_pane_rect: None,
            rsel_dragged: false,
            last_click: None,
            click_count: 0,
            rsel_block: false,
            selection_changed: false,
            border_drag: false,

            client_tab_positions: Vec::new(),
            client_status_row: u16::MAX,
            client_base_index: 0,
            client_pane_rects: Vec::new(),
            client_borders: Vec::new(),
            client_content_area: Rect::default(),
            client_copy_mode: false,
            client_pwsh_selection: false,
            client_zoomed: false,
            client_drag: None,
            hovered_border: None,

            pending_osc52: None,
            pending_bell: false,
            last_mouse_enable: Instant::now(),
            last_cursor_style: 255,
        }
    }
}
