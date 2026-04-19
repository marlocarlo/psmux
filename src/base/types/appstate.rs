#[allow(unused_imports)]
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyModifiers};
use portable_pty::MasterPty;
use ratatui::prelude::Rect;
use chrono::Local;

use super::*;

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
    /// allow-predictions: when on, do not force PSReadLine PredictionSource to
    /// None after the profile loads, letting the user's own prediction settings
    /// take effect.  The pre-profile crash prevention (#109) still runs.
    /// Default: off
    pub allow_predictions: bool,
    pub drag: Option<DragState>,
    pub last_window_area: Rect,
    pub mouse_enabled: bool,
    /// scroll-enter-copy-mode: when off, mouse scroll at a shell prompt does NOT
    /// auto-enter copy mode.  Default: on (tmux parity).
    pub scroll_enter_copy_mode: bool,
    /// pwsh-mouse-selection: when on, client-side drag selection behaves like
    /// Windows 11 PowerShell — pane-aware clipping, no copy-on-release (copy
    /// only on right-click), word/line selection on double/triple-click.
    /// Default: off (preserves the legacy pwsh-style copy-on-release).
    pub pwsh_mouse_selection: bool,
    pub paste_buffers: Vec<String>,
    pub status_left: String,
    pub status_right: String,
    pub window_base_index: usize,
    pub copy_anchor: Option<(u16,u16)>,
    /// Scroll offset when copy_anchor was set (for viewport-relative adjustment)
    pub copy_anchor_scroll_offset: usize,
    pub copy_pos: Option<(u16,u16)>,
    /// Cell where mouse was pressed down in copy mode (for click vs drag detection, #199)
    pub copy_mouse_down_cell: Option<(u16,u16)>,
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
    /// Receiver for async run-shell results (title, output).
    /// Commands are spawned in background threads and results polled each frame.
    pub run_shell_rx: Option<mpsc::Receiver<(String, String)>>,
    /// Sender cloned into each run-shell background thread.
    pub run_shell_tx: Option<mpsc::Sender<(String, String)>>,
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
    /// Client registry: all active PERSISTENT and CONTROL clients.
    pub client_registry: std::collections::HashMap<u64, ClientInfo>,
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
    /// allow-set-title: allow programs to set pane title via OSC 0/2 escape sequences
    pub allow_set_title: bool,
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
    /// Tracks which options have been explicitly set by the user or config.
    /// Used by set-option -o (only-if-unset) to distinguish defaults from
    /// explicitly configured values.
    pub user_set_options: std::collections::HashSet<String>,
    /// pane-border-style: style for inactive pane borders
    pub pane_border_style: String,
    /// pane-active-border-style: style for active pane borders
    pub pane_active_border_style: String,
    /// pane-border-hover-style: style for border hover highlight
    pub pane_border_hover_style: String,
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
    /// Whether the command prompt vi mode is in normal (true) vs insert (false)
    pub command_vi_normal: bool,
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
    /// One-shot bell forward flag: set when an audible bell should be emitted on the client terminal.
    pub bell_forward: bool,
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
    /// Last mouse event position (col, row) for #{mouse_x}, #{mouse_y} format variables.
    pub last_mouse_x: u16,
    pub last_mouse_y: u16,
    /// Transient status-bar message from display-message (without -p).
    /// Tuple of (message_text, timestamp_when_set, optional per_message_duration_ms).
    pub status_message: Option<(String, std::time::Instant, Option<u64>)>,
    /// Whether warm pane/server pre-spawning is enabled (default: on).
    /// When off, new sessions/windows always cold-spawn a fresh shell.
    pub warm_enabled: bool,
    /// Pre-spawned warm pane: shell already loaded, ready for instant new-window.
    pub warm_pane: Option<WarmPane>,
    /// Plugin .ps1 scripts queued during config loading for post-startup execution.
    /// These need the server to be running (TCP listener) before they can apply.
    pub pending_plugin_scripts: Vec<String>,
    /// Connected control mode clients (keyed by client_id).
    pub control_clients: HashMap<u64, ControlClient>,
    /// Session group name (set by `new-session -t target` for tmux group semantics).
    /// Sessions in the same group logically share a window list.
    pub session_group: Option<String>,
    /// When true, hardcoded default keybindings are suppressed (set by unbind-key -a).
    pub defaults_suppressed: bool,
    /// Panes extracted for cross-session forwarding, keyed by forward_id.
    /// The source server keeps these alive so the real ConPTY continues running.
    pub forwarded_panes: HashMap<u64, ForwardedPane>,
    /// Counter for generating unique forward IDs.
    pub next_forward_id: u64,
}
