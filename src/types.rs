use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use portable_pty::MasterPty;
use ratatui::prelude::Rect;
use chrono::Local;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Pane {
    pub master: Box<dyn MasterPty>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub child: Box<dyn portable_pty::Child>,
    pub term: Arc<Mutex<vt100::Parser>>,
    pub last_rows: u16,
    pub last_cols: u16,
    pub id: usize,
    pub title: String,
    /// Cached child process PID for Windows console mouse injection.
    /// Lazily extracted on first mouse event.
    pub child_pid: Option<u32>,
    /// Monotonic counter incremented by the PTY reader thread each time new
    /// output is processed.  Checked by the server to know when the screen
    /// has actually changed (avoids serialising stale frames).
    pub data_version: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Timestamp of the last auto-rename foreground-process check (throttled to ~1/s).
    pub last_title_check: Instant,
    /// Timestamp of the last infer_title_from_prompt call in layout serialisation (throttled to ~2/s).
    pub last_infer_title: Instant,
    /// True when the child process has exited but remain-on-exit keeps the pane visible.
    pub dead: bool,
    /// Cached VT bridge detection result (for mouse injection).
    /// Updated on first mouse event and refreshed every 2 seconds.
    pub vt_bridge_cache: Option<(Instant, bool)>,
    /// Cached ENABLE_VIRTUAL_TERMINAL_INPUT query result (for mouse injection).
    /// When true, the child's console input has VTI set, meaning VT mouse
    /// sequences can be delivered.  Refreshed every 2 seconds.
    pub vti_mode_cache: Option<(Instant, bool)>,
    /// Cached ENABLE_MOUSE_INPUT query result (for mouse injection heuristic).
    /// When true, the child's console has ENABLE_MOUSE_INPUT set, meaning it
    /// reads MOUSE_EVENT records via ReadConsoleInputW (crossterm/ratatui apps).
    /// When false, the child expects VT SGR mouse sequences (nvim, vim).
    /// Refreshed every 2 seconds.
    pub mouse_input_cache: Option<(Instant, bool)>,
    /// Last cursor shape requested by the child process via DECSCUSR (`\x1b[N q`).
    /// 0 = no override (use PSMUX_CURSOR_STYLE default), 1-6 = DECSCUSR values.
    pub cursor_shape: std::sync::Arc<std::sync::atomic::AtomicU8>,
    /// Set by the PTY reader thread when a BEL character (\x07) is detected.
    /// Consumed by the server loop to set the window's bell_flag.
    pub bell_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Per-pane copy mode state (tmux-style pane-local copy mode).
    /// Some(_) when this pane is in copy mode, None otherwise.
    pub copy_state: Option<CopyModeState>,
    /// Per-pane style string (set via `select-pane -P "bg=...,fg=..."`).
    /// Matches tmux's `window-style` / `window-active-style` pane option.
    /// Stored for API compatibility; ConPTY rendering doesn't support
    /// per-pane fg/bg tinting so this is not rendered yet.
    pub pane_style: Option<String>,
}

/// Pre-spawned shell ready to be transplanted into a new window instantly.
/// The shell has already loaded its profile (~470ms for pwsh), so the prompt
/// appears immediately when the user creates a new window — matching wezterm's
/// perceived "instant tab" experience.
pub struct WarmPane {
    pub master: Box<dyn MasterPty>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub child: Box<dyn portable_pty::Child>,
    pub term: Arc<Mutex<vt100::Parser>>,
    pub data_version: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub cursor_shape: std::sync::Arc<std::sync::atomic::AtomicU8>,
    pub bell_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub child_pid: Option<u32>,
    pub pane_id: usize,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LayoutKind { Horizontal, Vertical }

pub enum Node {
    Leaf(Pane),
    Split { kind: LayoutKind, sizes: Vec<u16>, children: Vec<Node> },
}

pub struct Window {
    pub root: Node,
    pub active_path: Vec<usize>,
    pub name: String,
    pub id: usize,
    /// Activity flag: set when pane output is received while window is not active
    pub activity_flag: bool,
    /// Bell flag: set when a bell (\x07) is detected in a pane
    pub bell_flag: bool,
    /// Silence flag: set when no output for monitor-silence seconds
    pub silence_flag: bool,
    /// Last output timestamp for silence detection
    pub last_output_time: std::time::Instant,
    /// Last observed combined data_version for activity detection
    pub last_seen_version: u64,
    /// True when the user has manually renamed this window (auto-rename won't override).
    /// Cleared when `set automatic-rename on` is explicitly set.
    pub manual_rename: bool,
    /// Current position in the named layout cycle (0..4)
    pub layout_index: usize,
    /// Per-pane MRU (most-recently-used) order: pane IDs ordered by recency.
    /// Front = most recently focused.  Used for:
    ///  - Directional navigation tie-breaking (issue #70)
    ///  - Focus selection after kill-pane (issue #71)
    pub pane_mru: Vec<usize>,
    /// Per-window zoom state (tmux parity: each window tracks its own zoom independently).
    /// When `Some(...)`, one pane in this window is zoomed; the vec stores saved split sizes
    /// for restoration on unzoom.
    pub zoom_saved: Option<Vec<(Vec<usize>, Vec<u16>)>>,
}

/// A menu item for display-menu
#[derive(Clone)]
pub struct MenuItem {
    pub name: String,
    pub key: Option<char>,
    pub command: String,
    pub is_separator: bool,
}

/// A parsed menu structure
#[derive(Clone)]
pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub selected: usize,
    pub x: Option<i16>,
    pub y: Option<i16>,
}

/// Hook definition - command to run on certain events
#[derive(Clone)]
pub struct Hook {
    pub name: String,
    pub command: String,
}

/// Interactive PTY for popup window (supports fzf, etc.)
pub struct PopupPty {
    pub master: Box<dyn portable_pty::MasterPty>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub child: Box<dyn portable_pty::Child>,
    pub term: std::sync::Arc<std::sync::Mutex<vt100::Parser>>,
}

/// Pipe pane state - process piping pane output
pub struct PipePaneState {
    pub pane_id: usize,
    pub process: Option<std::process::Child>,
    pub stdin: bool,
    pub stdout: bool,
}

/// Wait-for channel state
pub struct WaitChannel {
    pub locked: bool,
    pub waiters: Vec<mpsc::Sender<()>>,
}

pub enum Mode {
    Passthrough,
    Prefix { armed_at: Instant },
    CommandPrompt { input: String, cursor: usize },
    WindowChooser { selected: usize, tree: Vec<crate::session::TreeEntry> },
    RenamePrompt { input: String },
    RenameSessionPrompt { input: String },
    CopyMode,
    PaneChooser { opened_at: Instant },
    /// Interactive menu mode
    MenuMode { menu: Menu },
    /// Popup window running a command (with optional PTY for interactive programs)
    PopupMode { 
        command: String, 
        output: String, 
        process: Option<std::process::Child>,
        width: u16,
        height: u16,
        close_on_exit: bool,
        /// Optional: interactive PTY for the popup (fzf, etc.)  
        popup_pty: Option<PopupPty>,
    },
    /// Confirmation prompt before command
    ConfirmMode { 
        prompt: String, 
        command: String,
        input: String,
    },
    /// Copy-mode search input
    CopySearch {
        input: String,
        forward: bool,
    },
    /// Big clock display (tmux clock-mode)
    ClockMode,
    /// Interactive buffer chooser (prefix =)
    BufferChooser { selected: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectionMode { Char, Line, Rect }

/// Per-pane copy mode state, saved/restored on pane focus changes to provide
/// tmux-style pane-local copy mode.
#[derive(Clone)]
pub struct CopyModeState {
    pub anchor: Option<(u16, u16)>,
    pub anchor_scroll_offset: usize,
    pub pos: Option<(u16, u16)>,
    pub scroll_offset: usize,
    pub selection_mode: SelectionMode,
    pub search_query: String,
    pub count: Option<usize>,
    pub search_matches: Vec<(u16, u16, u16)>,
    pub search_idx: usize,
    pub search_forward: bool,
    pub find_char_pending: Option<u8>,
    pub text_object_pending: Option<u8>,
    pub register_pending: bool,
    pub register: Option<char>,
    /// true when the pane was in CopySearch (not CopyMode)
    pub in_search: bool,
    /// search input buffer (only meaningful when in_search == true)
    pub search_input: String,
    /// search direction for CopySearch
    pub search_input_forward: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusDir { Left, Right, Up, Down }

pub struct AppState {
    pub windows: Vec<Window>,
    pub active_idx: usize,
    pub mode: Mode,
    pub escape_time_ms: u64,
    pub repeat_time_ms: u64,
    /// True when prefix mode was re-armed by a repeatable binding (not initial prefix press).
    pub prefix_repeating: bool,
    pub prefix_key: (KeyCode, KeyModifiers),
    pub prefix2_key: Option<(KeyCode, KeyModifiers)>,
    pub prediction_dimming: bool,
    pub drag: Option<DragState>,
    pub last_window_area: Rect,
    pub mouse_enabled: bool,
    pub paste_buffers: Vec<String>,
    pub status_left: String,
    pub status_right: String,
    pub window_base_index: usize,
    pub copy_anchor: Option<(u16,u16)>,
    /// Scroll offset when copy_anchor was set (for viewport-relative adjustment)
    pub copy_anchor_scroll_offset: usize,
    pub copy_pos: Option<(u16,u16)>,
    pub copy_scroll_offset: usize,
    /// Selection mode: Char (default), Line (V), Rect (C-v)
    pub copy_selection_mode: SelectionMode,
    /// Copy-mode search query
    pub copy_search_query: String,    /// Numeric prefix count for copy-mode motions (vi-style)
    pub copy_count: Option<usize>,    /// Copy-mode search matches: (row, col_start, col_end) in screen coords
    pub copy_search_matches: Vec<(u16, u16, u16)>,
    /// Current match index in copy_search_matches
    pub copy_search_idx: usize,
    /// Search direction: true = forward (/), false = backward (?)
    pub copy_search_forward: bool,
    /// Pending find-char operation: (f=0,F=1,t=2,T=3) for next char input
    pub copy_find_char_pending: Option<u8>,
    /// Pending text-object prefix: 0 = 'a' (a-word), 1 = 'i' (inner-word)
    pub copy_text_object_pending: Option<u8>,
    /// Pending register selection: true when '"' was pressed, waiting for a-z
    pub copy_register_pending: bool,
    /// Currently selected named register (a-z), None = default unnamed
    pub copy_register: Option<char>,
    /// Named registers a-z for copy-mode yank/paste
    pub named_registers: std::collections::HashMap<char, String>,
    pub display_map: Vec<(usize, Vec<usize>)>,
    /// Key tables: "prefix" (default), "root", "copy-mode-vi", "copy-mode-emacs", etc.
    pub key_tables: std::collections::HashMap<String, Vec<Bind>>,
    /// Current key table for switch-client -T (None = normal mode)
    pub current_key_table: Option<String>,
    pub control_rx: Option<mpsc::Receiver<CtrlReq>>,
    pub control_port: Option<u16>,
    pub session_key: String,
    pub session_name: String,
    /// Numeric session ID (tmux-compatible: $0, $1, $2...).
    pub session_id: usize,
    /// -L socket name for namespace isolation (tmux compatible).
    /// When set, port/key files are stored as `{socket_name}__{session_name}.port`.
    pub socket_name: Option<String>,
    pub attached_clients: usize,
    /// Per-client terminal sizes for multi-client resize tracking.
    pub client_sizes: std::collections::HashMap<u64, (u16, u16)>,
    /// The most recently active client ID (for window_size="latest").
    pub latest_client_id: Option<u64>,
    pub created_at: chrono::DateTime<Local>,
    pub next_win_id: usize,
    pub next_pane_id: usize,
    /// Whether the attached client is currently in prefix mode (for `client_prefix` format var).
    pub client_prefix_active: bool,
    pub sync_input: bool,
    /// Hooks: map of hook name to list of commands
    pub hooks: std::collections::HashMap<String, Vec<String>>,
    /// Wait-for channels: map of channel name to list of waiting senders
    pub wait_channels: std::collections::HashMap<String, WaitChannel>,
    /// Pipe pane processes
    pub pipe_panes: Vec<PipePaneState>,
    /// Last active window index (for last-window command)
    pub last_window_idx: usize,
    /// Last active pane path (for last-pane command)
    pub last_pane_path: Vec<usize>,
    /// Tab positions on status bar: (window_index, x_start, x_end)
    pub tab_positions: Vec<(usize, u16, u16)>,
    /// history-limit: scrollback buffer size (default 2000)
    pub history_limit: usize,
    /// display-time: how long messages are shown (ms, default 750)
    pub display_time_ms: u64,
    /// display-panes-time: how long pane overlay is shown (ms, default 1000)
    pub display_panes_time_ms: u64,
    /// pane-base-index: first pane id (default 0)
    pub pane_base_index: usize,
    /// focus-events: pass focus events to apps
    pub focus_events: bool,
    /// mode-keys: vi or emacs (stored for compat, default emacs)
    pub mode_keys: String,
    /// status: whether status bar is shown
    pub status_visible: bool,
    /// status-position: "top" or "bottom" (default "bottom")
    pub status_position: String,
    /// status-style: stored for compat
    pub status_style: String,
    /// default-command / default-shell: shell to launch for new panes
    pub default_shell: String,
    /// word-separators: characters that delimit words in copy mode
    pub word_separators: String,
    /// renumber-windows: auto-renumber on close
    pub renumber_windows: bool,
    /// automatic-rename: update window name from active pane's running command
    pub automatic_rename: bool,
    /// allow-rename: allow programs to set window title via escape sequences
    pub allow_rename: bool,
    /// monitor-activity / visual-activity: stored for compat
    pub monitor_activity: bool,
    pub visual_activity: bool,
    /// activity-action: what to do on activity ("any", "none", "current", "other")
    pub activity_action: String,
    /// silence-action: what to do on silence ("any", "none", "current", "other")
    pub silence_action: String,
    /// remain-on-exit: keep panes open after process exits
    pub remain_on_exit: bool,
    /// destroy-unattached: exit server when no clients remain attached
    pub destroy_unattached: bool,
    /// exit-empty: exit server when all panes/windows are empty
    pub exit_empty: bool,
    /// aggressive-resize: resize window to smallest attached client
    pub aggressive_resize: bool,
    /// set-titles: update terminal title
    pub set_titles: bool,
    /// set-titles-string: format for terminal title
    pub set_titles_string: String,
    /// update-environment: list of env var names to update from client on attach
    pub update_environment: Vec<String>,
    /// Environment variables set via set-environment
    pub environment: std::collections::HashMap<String, String>,
    /// User/plugin options (@-prefixed, tmux convention).
    /// Stored separately from `environment` so they are NOT passed as
    /// shell environment variables to child panes (#105).
    pub user_options: std::collections::HashMap<String, String>,
    /// pane-border-style: style for inactive pane borders
    pub pane_border_style: String,
    /// pane-active-border-style: style for active pane borders
    pub pane_active_border_style: String,
    /// window-status-format: format for inactive window tabs
    pub window_status_format: String,
    /// window-status-current-format: format for active window tab
    pub window_status_current_format: String,
    /// window-status-separator: between window status entries
    pub window_status_separator: String,
    /// window-status-style: style for inactive window status
    pub window_status_style: String,
    /// window-status-current-style: style for active window status
    pub window_status_current_style: String,
    /// window-status-activity-style: style for windows with activity
    pub window_status_activity_style: String,
    /// window-status-bell-style: style for windows with bell
    pub window_status_bell_style: String,
    /// window-status-last-style: style for last active window
    pub window_status_last_style: String,
    /// message-style: style for status-line messages
    pub message_style: String,
    /// message-command-style: style for command prompt
    pub message_command_style: String,
    /// mode-style: style for copy-mode highlighting
    pub mode_style: String,
    /// status-left-style: style for status-left area
    pub status_left_style: String,
    /// status-right-style: style for status-right area
    pub status_right_style: String,
    /// Marked pane: (window_index, pane_id) — set by select-pane -m
    pub marked_pane: Option<(usize, usize)>,
    /// monitor-silence: seconds of silence before flagging (0 = off)
    pub monitor_silence: u64,
    /// bell-action: "any", "none", "current", "other"
    pub bell_action: String,
    /// visual-bell: show visual indicator on bell
    pub visual_bell: bool,
    /// Command prompt history
    pub command_history: Vec<String>,
    /// Command prompt history index (for up/down navigation)
    pub command_history_idx: usize,
    /// status-interval: seconds between status-line refreshes (default 15)
    pub status_interval: u64,
    /// Last time the status-interval hook was fired
    pub last_status_interval_fire: std::time::Instant,
    /// status-justify: left, centre, right, absolute-centre
    pub status_justify: String,
    /// main-pane-width: percentage for main pane in main-vertical layout (0 = use 60% heuristic)
    pub main_pane_width: u16,
    /// main-pane-height: percentage for main pane in main-horizontal layout (0 = use 60% heuristic)
    pub main_pane_height: u16,
    /// status-left-length: max display width for status-left (default 10)
    pub status_left_length: usize,
    /// status-right-length: max display width for status-right (default 40)
    pub status_right_length: usize,
    /// status lines: number of status bar lines (default 1, set via `set status N`)
    pub status_lines: usize,
    /// status-format: custom format strings for each status line (index 1+)
    pub status_format: Vec<String>,
    /// window-size: "smallest", "largest", "manual", "latest" (default "latest")
    pub window_size: String,
    /// allow-passthrough: "on", "off", "all" (default "off")
    pub allow_passthrough: String,
    /// copy-command: command to pipe yanked text to (default empty)
    pub copy_command: String,
    /// command-alias: map of alias name to expansion
    pub command_aliases: std::collections::HashMap<String, String>,
    /// set-clipboard: "on", "off", "external" (default "on")
    pub set_clipboard: String,
    /// One-shot clipboard text to be sent to the client via OSC 52 (set by yank, consumed by dump-state).
    pub clipboard_osc52: Option<String>,
    /// env-shim: inject a Unix-compatible `env` function into PowerShell panes
    /// so that `env VAR=val command` syntax works (required by Claude Code, etc.).
    /// Default: on
    pub env_shim: bool,
    /// claude-code-fix-tty: inject a Node.js preload script via NODE_OPTIONS
    /// that patches process.stdout.isTTY = true inside ConPTY panes.  Works around
    /// Claude Code's isTTY gate that forces in-process agent mode on Windows
    /// (claude-code#26244).  Once Claude Code fixes the bug upstream, users can
    /// disable this with: set -g claude-code-fix-tty off
    /// Default: on
    pub claude_code_fix_tty: bool,
    /// claude-code-force-interactive: set CLAUDE_CODE_FORCE_INTERACTIVE=1 in
    /// pane environments so Claude Code treats the session as interactive even
    /// when its own heuristics disagree.  This prevents the non-interactive
    /// fast-path that bypasses teammateMode entirely.
    /// Once Claude Code fixes the bug upstream, disable with:
    ///   set -g claude-code-force-interactive off
    /// Default: on
    pub claude_code_force_interactive: bool,
    /// Last mouse hover position (col, row) for same-coordinate deduplication.
    /// Windows Terminal suppresses consecutive MOUSE_MOVED at the same position.
    pub last_hover_pos: Option<(u16, u16)>,
    /// Transient status-bar message from display-message (without -p).
    /// Tuple of (message_text, timestamp_when_set).
    pub status_message: Option<(String, std::time::Instant)>,
    /// Pre-spawned warm pane: shell already loaded, ready for instant new-window.
    pub warm_pane: Option<WarmPane>,
    /// Plugin .ps1 scripts queued during config loading for post-startup execution.
    /// These need the server to be running (TCP listener) before they can apply.
    pub pending_plugin_scripts: Vec<String>,
}

impl AppState {
    /// Create a new AppState with sensible defaults.
    /// Caller should set `session_name` and call `load_config()` after construction.
    pub fn new(session_name: String) -> Self {
        Self {
            windows: Vec::new(),
            active_idx: 0,
            mode: Mode::Passthrough,
            escape_time_ms: 500,
            repeat_time_ms: 500,
            prefix_repeating: false,
            prefix_key: (crossterm::event::KeyCode::Char('b'), crossterm::event::KeyModifiers::CONTROL),
            prefix2_key: None,
            prediction_dimming: std::env::var("PSMUX_DIM_PREDICTIONS")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false),
            drag: None,
            last_window_area: Rect { x: 0, y: 0, width: 120, height: 30 },
            mouse_enabled: true,
            paste_buffers: Vec::new(),
            status_left: "[#S] ".to_string(),
            status_right: "#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y".to_string(),
            window_base_index: 0,
            copy_anchor: None,
            copy_anchor_scroll_offset: 0,
            copy_pos: None,
            copy_scroll_offset: 0,
            copy_selection_mode: SelectionMode::Char,
            copy_count: None,
            copy_search_query: String::new(),
            copy_search_matches: Vec::new(),
            copy_search_idx: 0,
            copy_search_forward: true,
            copy_find_char_pending: None,
            copy_text_object_pending: None,
            copy_register_pending: false,
            copy_register: None,
            named_registers: std::collections::HashMap::new(),
            display_map: Vec::new(),
            key_tables: std::collections::HashMap::new(),
            current_key_table: None,
            control_rx: None,
            control_port: None,
            session_key: String::new(),
            session_name,
            session_id: {
                static NEXT_SESSION_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                NEXT_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            socket_name: None,
            attached_clients: 0,
            client_sizes: std::collections::HashMap::new(),
            latest_client_id: None,
            created_at: Local::now(),
            next_win_id: 1,
            next_pane_id: 1,
            client_prefix_active: false,
            sync_input: false,
            hooks: std::collections::HashMap::new(),
            wait_channels: std::collections::HashMap::new(),
            pipe_panes: Vec::new(),
            last_window_idx: 0,
            last_pane_path: Vec::new(),
            tab_positions: Vec::new(),
            history_limit: 2000,
            display_time_ms: 750,
            display_panes_time_ms: 1000,
            pane_base_index: 0,
            focus_events: false,
            mode_keys: "emacs".to_string(),
            status_visible: true,
            status_position: "bottom".to_string(),
            status_style: "bg=green,fg=black".to_string(),
            default_shell: String::new(),
            word_separators: " -_@".to_string(),
            renumber_windows: false,
            automatic_rename: true,
            allow_rename: true,
            monitor_activity: false,
            visual_activity: false,
            activity_action: "other".to_string(),
            silence_action: "other".to_string(),
            remain_on_exit: false,
            destroy_unattached: false,
            exit_empty: true,
            aggressive_resize: false,
            set_titles: false,
            set_titles_string: String::new(),
            update_environment: vec![
                "DISPLAY".to_string(),
                "KRB5CCNAME".to_string(),
                "SSH_ASKPASS".to_string(),
                "SSH_AUTH_SOCK".to_string(),
                "SSH_AGENT_PID".to_string(),
                "SSH_CONNECTION".to_string(),
                "WINDOWID".to_string(),
                "XAUTHORITY".to_string(),
            ],
            environment: std::collections::HashMap::new(),
            user_options: std::collections::HashMap::new(),
            pane_border_style: String::new(),
            pane_active_border_style: "fg=green".to_string(),
            window_status_format: "#I:#W#{?window_flags,#{window_flags}, }".to_string(),
            window_status_current_format: "#I:#W#{?window_flags,#{window_flags}, }".to_string(),
            window_status_separator: " ".to_string(),
            window_status_style: String::new(),
            window_status_current_style: String::new(),
            window_status_activity_style: "reverse".to_string(),
            window_status_bell_style: "reverse".to_string(),
            window_status_last_style: String::new(),
            message_style: "bg=yellow,fg=black".to_string(),
            message_command_style: "bg=black,fg=yellow".to_string(),
            mode_style: "bg=yellow,fg=black".to_string(),
            status_left_style: String::new(),
            status_right_style: String::new(),
            marked_pane: None,
            monitor_silence: 0,
            bell_action: "any".to_string(),
            visual_bell: false,
            command_history: Vec::new(),
            command_history_idx: 0,
            status_interval: 15,
            last_status_interval_fire: std::time::Instant::now(),
            status_justify: "left".to_string(),
            main_pane_width: 0,
            main_pane_height: 0,
            status_left_length: 10,
            status_right_length: 40,
            status_lines: 1,
            status_format: Vec::new(),
            window_size: "latest".to_string(),
            allow_passthrough: "off".to_string(),
            copy_command: String::new(),
            command_aliases: std::collections::HashMap::new(),
            set_clipboard: "on".to_string(),
            clipboard_osc52: None,
            env_shim: true,
            claude_code_fix_tty: true,
            claude_code_force_interactive: true,
            last_hover_pos: None,
            status_message: None,
            warm_pane: None,
            pending_plugin_scripts: Vec::new(),
        }
    }

    /// Get the port/key file base name, incorporating socket_name for -L namespace isolation.
    /// When socket_name is set (via -L flag), files are stored as `{socket_name}__{session_name}`.
    /// Otherwise, just the session_name is used.
    pub fn port_file_base(&self) -> String {
        if let Some(ref sn) = self.socket_name {
            format!("{}__{}", sn, self.session_name)
        } else {
            self.session_name.clone()
        }
    }
}

pub struct DragState {
    pub split_path: Vec<usize>,
    pub kind: LayoutKind,
    pub index: usize,
    pub start_x: u16,
    pub start_y: u16,
    pub left_initial: u16,
    pub _right_initial: u16,
    /// Total pixel dimension of the parent split area along the split axis.
    pub total_pixels: u16,
}

#[derive(Clone)]
pub enum Action { 
    DisplayPanes, 
    MoveFocus(FocusDir),
    /// Execute an arbitrary tmux-style command string
    Command(String),
    /// Execute multiple tmux-style commands in sequence (`;` chaining)
    CommandChain(Vec<String>),
    /// Common actions with direct handling
    NewWindow,
    SplitHorizontal,
    SplitVertical,
    KillPane,
    NextWindow,
    PrevWindow,
    CopyMode,
    Paste,
    Detach,
    RenameWindow,
    WindowChooser,
    ZoomPane,
    /// Switch to a named key table (switch-client -T)
    SwitchTable(String),
}

#[derive(Clone)]
pub struct Bind { pub key: (KeyCode, KeyModifiers), pub action: Action, pub repeat: bool }

pub enum CtrlReq {
    NewWindow(Option<String>, Option<String>, bool, Option<String>),  // cmd, name, detached, start_dir
    NewWindowPrint(Option<String>, Option<String>, bool, Option<String>, Option<String>, mpsc::Sender<String>),  // cmd, name, detached, start_dir, format, resp
    SplitWindow(LayoutKind, Option<String>, bool, Option<String>, Option<u16>, mpsc::Sender<String>),  // kind, cmd, detached, start_dir, size_percent, error_resp
    SplitWindowPrint(LayoutKind, Option<String>, bool, Option<String>, Option<u16>, Option<String>, mpsc::Sender<String>),  // kind, cmd, detached, start_dir, size_percent, format, resp
    KillPane,
    KillPaneById(usize),
    CapturePane(mpsc::Sender<String>),
    CapturePaneStyled(mpsc::Sender<String>, Option<i32>, Option<i32>),
    FocusWindow(usize),
    /// Temporary focus for -t targeting: server saves/restores active_idx
    FocusWindowTemp(usize),
    FocusPane(usize),
    FocusPaneByIndex(usize),
    /// Temporary pane focus for -t targeting
    FocusPaneTemp(usize),
    FocusPaneByIndexTemp(usize),
    SessionInfo(mpsc::Sender<String>),
    CapturePaneRange(mpsc::Sender<String>, Option<i32>, Option<i32>),
    ClientAttach(u64),
    ClientDetach(u64),
    DumpLayout(mpsc::Sender<String>),
    DumpState(mpsc::Sender<String>, bool),  // (resp, allow_nc)
    SendText(String),
    SendKey(String),
    SendPaste(String),
    ZoomPane,
    PrefixBegin,
    PrefixEnd,
    CopyEnter,
    CopyEnterPageUp,
    CopyMove(i16, i16),
    CopyAnchor,
    CopyYank,
    CopyRectToggle,
    ClientSize(u64, u16, u16),
    FocusPaneCmd(usize),
    FocusWindowCmd(usize),
    MouseDown(u64,u16,u16),
    MouseDownRight(u64,u16,u16),
    MouseDownMiddle(u64,u16,u16),
    MouseDrag(u64,u16,u16),
    MouseUp(u64,u16,u16),
    MouseUpRight(u64,u16,u16),
    MouseUpMiddle(u64,u16,u16),
    MouseMove(u64,u16,u16),
    ScrollUp(u64,u16, u16),
    ScrollDown(u64,u16, u16),
    NextWindow,
    PrevWindow,
    RenameWindow(String),
    ListWindows(mpsc::Sender<String>),
    ListWindowsTmux(mpsc::Sender<String>),
    ListWindowsFormat(mpsc::Sender<String>, String),
    ListTree(mpsc::Sender<String>),
    ToggleSync,
    SetPaneTitle(String),
    SetPaneStyle(String),
    SendKeys(String, bool),
    SendKeysX(String),  // send-keys -X copy-mode-command
    SelectPane(String),
    SelectWindow(usize),
    ListPanes(mpsc::Sender<String>),
    ListPanesFormat(mpsc::Sender<String>, String),
    ListAllPanes(mpsc::Sender<String>),
    ListAllPanesFormat(mpsc::Sender<String>, String),
    KillWindow,
    KillSession,
    HasSession(mpsc::Sender<bool>),
    RenameSession(String),
    /// Claim a warm server: rename session + send response so CLI knows it's done.
    /// Fields: session name, optional client CWD, response sender.
    ClaimSession(String, Option<String>, mpsc::Sender<String>),
    SwapPane(String),
    ResizePane(String, u16),
    SetBuffer(String),
    ListBuffers(mpsc::Sender<String>),
    ListBuffersFormat(mpsc::Sender<String>, String),
    ShowBuffer(mpsc::Sender<String>),
    ShowBufferAt(mpsc::Sender<String>, usize),
    DeleteBuffer,
    DisplayMessage(mpsc::Sender<String>, String, Option<usize>, bool),  // resp, format, target_pane_idx, set_status_bar
    LastWindow,
    LastPane,
    RotateWindow(bool),
    DisplayPanes,
    DisplayPaneSelect(usize),
    BreakPane,
    JoinPane(usize),
    RespawnPane,
    BindKey(String, String, String, bool),  // table, key, command, repeat
    UnbindKey(String),
    ListKeys(mpsc::Sender<String>),
    SetOption(String, String),
    SetOptionQuiet(String, String, bool),  // set-option with quiet flag
    SetOptionUnset(String),  // set-option -u
    SetOptionAppend(String, String),  // set-option -a
    ShowOptions(mpsc::Sender<String>),
    ShowWindowOptions(mpsc::Sender<String>),
    SourceFile(String),
    MoveWindow(Option<usize>),
    SwapWindow(usize),
    LinkWindow(String),
    UnlinkWindow,
    FindWindow(mpsc::Sender<String>, String),
    MovePane(usize),
    PipePane(String, bool, bool, bool),
    SelectLayout(String),
    NextLayout,
    ListClients(mpsc::Sender<String>),
    SwitchClient(String),
    LockClient,
    RefreshClient,
    SuspendClient,
    CopyModePageUp,
    ClearHistory,
    SaveBuffer(String),
    LoadBuffer(String),
    SetEnvironment(String, String),
    UnsetEnvironment(String),
    ShowEnvironment(mpsc::Sender<String>),
    SetHook(String, String),
    ShowHooks(mpsc::Sender<String>),
    RemoveHook(String),
    KillServer,
    WaitFor(String, WaitForOp),
    DisplayMenu(String, Option<i16>, Option<i16>),
    DisplayMenuDirect(Menu),
    DisplayPopup(String, u16, u16, bool),
    ConfirmBefore(String, String),
    ClockMode,
    ResizePaneAbsolute(String, u16),
    ResizePanePercent(String, u8), // axis, percentage (0-100)
    ShowOptionValue(mpsc::Sender<String>, String),
    ShowWindowOptionValue(mpsc::Sender<String>, String),
    ChooseBuffer(mpsc::Sender<String>),
    ServerInfo(mpsc::Sender<String>),
    SendPrefix,
    PrevLayout,
    SwitchClientTable(String),
    ListCommands(mpsc::Sender<String>),
    ResizeWindow(String, u16),
    RespawnWindow,
    FocusIn,
    FocusOut,
    CommandPrompt(String),
    ShowMessages(mpsc::Sender<String>),
    /// Forward raw bytes to the popup PTY (base64-decoded by connection handler)
    PopupInput(Vec<u8>),
    /// Close the current overlay (popup, menu, confirm, etc.)
    OverlayClose,
    /// Respond to confirm-before prompt (true = yes, false = no)
    ConfirmRespond(bool),
    /// Select a menu item by index
    MenuSelect(usize),
    /// Navigate menu up/down (delta: -1 = up, +1 = down)
    MenuNavigate(i32),
}

/// Global flag set by PTY reader threads when new output arrives.
/// The server loop checks this to use a shorter recv_timeout, reducing
/// keystroke-to-display latency for nested shells (e.g. WSL inside pwsh).
pub static PTY_DATA_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tracked persistent client TCP streams.
/// Connection handlers register clones here so the server can explicitly
/// `shutdown()` them before `process::exit(0)`.  Without this, Windows
/// does not reliably deliver TCP RST on loopback sockets when a process
/// exits, leaving the client's blocking `read_line()` stuck forever.
static PERSISTENT_STREAMS: std::sync::Mutex<Vec<std::net::TcpStream>> = std::sync::Mutex::new(Vec::new());

/// Register a persistent client stream (call from connection handler).
pub fn register_persistent_stream(stream: &std::net::TcpStream) {
    if let Ok(cloned) = stream.try_clone() {
        if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
            v.push(cloned);
        }
    }
}

/// Shut down all tracked persistent client streams so their readers get EOF.
pub fn shutdown_persistent_streams() {
    if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
        for s in v.drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Server-push frame senders for persistent (attached) clients.
/// Instead of clients polling dump-state, the server proactively pushes
/// serialized frames through these channels whenever state changes.
/// Each sender feeds a `Receiver<String>` into the persistent connection's
/// existing writer-thread pipeline (which expects oneshot receivers).
static FRAME_PUSH_SENDERS: std::sync::Mutex<Vec<std::sync::mpsc::Sender<std::sync::mpsc::Receiver<String>>>> =
    std::sync::Mutex::new(Vec::new());

/// Register a persistent connection's resp_tx clone for server-pushed frames.
pub fn register_frame_sender(tx: std::sync::mpsc::Sender<std::sync::mpsc::Receiver<String>>) {
    if let Ok(mut v) = FRAME_PUSH_SENDERS.lock() {
        v.push(tx);
    }
}

/// Push a serialized frame to all persistent clients.  Dead senders are pruned.
pub fn push_frame(frame: &str) {
    if let Ok(mut senders) = FRAME_PUSH_SENDERS.lock() {
        senders.retain(|tx| {
            let (rtx, rrx) = std::sync::mpsc::channel();
            // Send the frame through a oneshot so it fits the existing writer thread protocol
            if rtx.send(frame.to_string()).is_err() { return false; }
            tx.send(rrx).is_ok()
        });
    }
}

/// Check if any persistent clients are registered for push.
pub fn has_frame_receivers() -> bool {
    FRAME_PUSH_SENDERS.lock().map_or(false, |v| !v.is_empty())
}

/// Wait-for operation types
#[derive(Clone, Copy)]
pub enum WaitForOp {
    Wait,
    Lock,
    Signal,
    Unlock,
}

/// Parsed target specification from -t argument.
#[derive(Debug, Clone, Default)]
pub struct ParsedTarget {
    pub session: Option<String>,
    pub window: Option<usize>,
    pub pane: Option<usize>,
    pub pane_is_id: bool,
    pub window_is_id: bool,
}
