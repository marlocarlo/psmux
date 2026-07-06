use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyModifiers};
use portable_pty::MasterPty;
use ratatui::prelude::Rect;
use chrono::Local;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Notifications emitted to control mode clients (tmux wire-compatible).
#[derive(Clone, Debug)]
pub enum ControlNotification {
    Output { pane_id: usize, data: String },
    WindowAdd { window_id: usize },
    WindowClose { window_id: usize },
    WindowRenamed { window_id: usize, name: String },
    WindowPaneChanged { window_id: usize, pane_id: usize },
    LayoutChange { window_id: usize, layout: String },
    SessionChanged { session_id: usize, name: String },
    SessionRenamed { name: String },
    SessionWindowChanged { session_id: usize, window_id: usize },
    SessionsChanged,
    PaneModeChanged { pane_id: usize },
    ClientDetached { client: String },
    Continue { pane_id: usize },
    Pause { pane_id: usize },
    /// Extended output with age information (when pause-after is active).
    ExtendedOutput { pane_id: usize, age_ms: u64, data: String },
    /// Subscription value changed notification.
    SubscriptionChanged {
        name: String,
        session_id: usize,
        window_id: usize,
        window_index: usize,
        pane_id: usize,
        value: String,
    },
    Exit { reason: Option<String> },
    PasteBufferChanged { name: String },
    PasteBufferDeleted { name: String },
    ClientSessionChanged { client: String, session_id: usize, name: String },
    Message { text: String },
}

/// Per-connection control mode client state.
pub struct ControlClient {
    pub client_id: u64,
    pub cmd_counter: u64,
    pub echo_enabled: bool,
    pub notification_tx: mpsc::SyncSender<ControlNotification>,
    pub paused_panes: HashSet<usize>,
    /// `refresh-client -B name:what:format` subscriptions.
    /// Key = subscription name, Value = (target, format_string).
    pub subscriptions: HashMap<String, (String, String)>,
    /// Last expanded value for each subscription (for change detection).
    pub subscription_values: HashMap<String, String>,
    /// Last time each subscription was checked (rate limit: 1/s per sub).
    pub subscription_last_check: HashMap<String, Instant>,
    /// `refresh-client -f pause-after=N`: pause output if client falls behind by N seconds.
    pub pause_after_secs: Option<u64>,
    /// Panes whose output is currently paused due to pause-after threshold.
    pub output_paused_panes: HashSet<usize>,
    /// Timestamp of last output sent per pane (for pause-after age tracking).
    pub pane_last_output: HashMap<usize, Instant>,
}

/// Per-client metadata stored in the server's client registry.
/// Tracks every attached PERSISTENT and CONTROL client.
#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub id: u64,
    pub width: u16,
    pub height: u16,
    pub connected_at: std::time::Instant,
    pub last_activity: std::time::Instant,
    /// Synthetic TTY name for display (e.g. "/dev/pts/1")
    pub tty_name: String,
    /// True for CONTROL/CONTROL_NOECHO clients
    pub is_control: bool,
}

pub struct Pane {
    pub master: Box<dyn MasterPty>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub child: Box<dyn portable_pty::Child>,
    pub term: Arc<Mutex<vt100::Parser>>,
    pub last_rows: u16,
    pub last_cols: u16,
    pub id: usize,
    pub title: String,
    /// When true, `infer_title_from_prompt` will not overwrite the title.
    /// Set by `select-pane -T` (explicit title). Cleared by `select-pane -T ""`.
    pub title_locked: bool,
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
    /// Timestamp of the last printable keystroke routed via the INTERACTIVE
    /// text-input route (`handle_key -> forward_key_to_active`); `None` until
    /// the first one. NOT updated by the injected route (`send-keys` /
    /// `send-paste` / `send-text`). Exposed read-only as the
    /// `#{pane_last_text_input}` format variable. Lives on the pane, so it's
    /// freed with it (no separate lifecycle / file).
    pub last_text_input: Option<Instant>,
    /// The last NON-text key routed via the INTERACTIVE input route
    /// (`handle_key -> forward_key_to_active`): its canonical bind-key name
    /// (`Escape`, `Enter`, `Up`, `F9`, `C-c`, `M-a`, ...) + the `Instant` it
    /// arrived; `None` until the first one. Same route contract as
    /// `last_text_input` (NOT updated by the injected route). The text vs
    /// non-text split is `is_text_input_key`. Exposed read-only as
    /// `#{pane_last_special_key}` / `#{pane_last_special_key_ms}`.
    pub last_special_key: Option<(Instant, String)>,
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
    /// Set by the PTY reader thread when ESC[6n (Cursor Position Request) is
    /// detected in the child's output.  Consumed by the server loop, which
    /// then injects ESC[row;colR into the pane's PTY input.  This handles
    /// the case where pwsh re-issues the CPR after lock/unlock — the single
    /// preemptive write at spawn time is no longer in the pipe at that point.
    pub cpr_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Per-pane copy mode state (tmux-style pane-local copy mode).
    /// Some(_) when this pane is in copy mode, None otherwise.
    pub copy_state: Option<CopyModeState>,
    /// Per-pane style string (set via `select-pane -P "bg=...,fg=..."`).
    /// Matches tmux's `window-style` / `window-active-style` pane option.
    /// Stored for API compatibility; ConPTY rendering doesn't support
    /// per-pane fg/bg tinting so this is not rendered yet.
    pub pane_style: Option<String>,
    /// When set, the layout serialiser renders this pane as blank until
    /// the deadline passes.  Used to hide injected cd+cls commands during
    /// warm session claiming so the user never sees a flash.
    pub squelch_until: Option<Instant>,
    /// Per-pane output ring buffer for control mode %output notifications.
    /// Filled by the PTY reader thread, drained by the server loop.
    pub output_ring: Arc<Mutex<VecDeque<u8>>>,
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
    pub cpr_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub child_pid: Option<u32>,
    pub pane_id: usize,
    pub rows: u16,
    pub cols: u16,
    pub output_ring: Arc<Mutex<VecDeque<u8>>>,
}

/// A pane extracted from this session for cross-session forwarding.
/// The real ConPTY stays alive here; I/O is tunneled over TCP to the target.
pub struct ForwardedPane {
    pub master: Box<dyn MasterPty>,
    pub child: Box<dyn portable_pty::Child>,
    pub listener_port: u16,
    pub pid: Option<u32>,
    pub title: String,
    pub rows: u16,
    pub cols: u16,
    /// Handle to the forwarding threads (so we can abort on kill).
    pub shutdown: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// If this window is a linked reference, stores the source window ID it was linked from.
    pub linked_from: Option<usize>,
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

// PopupPty has been removed: popups now store an actual Pane
// (see src/popup.rs for the popup-as-pane architecture).

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
    /// Popup window running a command.
    /// Interactive popups store a real `Pane` (same type as tiled panes),
    /// inheriting all pane features: vt100 parsing, colors, PTY I/O.
    PopupMode { 
        command: String, 
        output: String, 
        process: Option<std::process::Child>,
        width: u16,
        height: u16,
        close_on_exit: bool,
        /// Optional: full Pane powering the popup (for interactive programs)
        popup_pane: Option<Pane>,
        /// Scroll offset for static text popups (lines from top)
        scroll_offset: u16,
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
    /// Window index prompt (prefix ') — jump to window by number
    WindowIndexPrompt { input: String },
    /// Interactive option editor (tmux 3.2+ customize-mode)
    CustomizeMode {
        options: Vec<(String, String, String)>,
        selected: usize,
        scroll_offset: usize,
        editing: bool,
        edit_buffer: String,
        edit_cursor: usize,
        filter: String,
    },
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
    /// allow-predictions: when on, do not force PSReadLine PredictionSource to
    /// None after the profile loads, letting the user's own prediction settings
    /// take effect.  The pre-profile crash prevention (#109) still runs.
    /// Default: off
    pub allow_predictions: bool,
    pub drag: Option<DragState>,
    pub last_window_area: Rect,
    pub mouse_enabled: bool,
    /// bold-is-bright: when on (default), rewrite crossterm's 256-indexed
    /// `38;5;N`/`48;5;N` (N<=15) back to the standard 30-37/90-97 SGR codes so
    /// the outer terminal applies "bold is bright" to the 16 basic colors
    /// (issue #425).  Turn off to pass crossterm's output through untouched,
    /// which keeps explicit 256-indexed low colors byte-accurate at the cost of
    /// losing bold-is-bright on basic colors.
    pub bold_is_bright: bool,
    /// sixel: when on (default), render sixel graphics emitted by panes on the
    /// outer terminal.  Server always parses and stores sixel images; this flag
    /// only gates the client-side emission (issue #431).
    pub sixel: bool,
    /// Ids of sixel image blobs already shipped to attached clients this
    /// session.  The dump-state builder ships each blob's base64 bytes exactly
    /// once (in `image_blobs`) and records the id here; later frames carry only
    /// the tiny descriptor.  Cleared on client attach / refresh-client so a new
    /// (or re-attaching) client re-receives every currently-visible blob (M4).
    pub shipped_image_ids: std::collections::HashSet<u64>,
    /// scroll-enter-copy-mode: when off, mouse scroll at a shell prompt does NOT
    /// auto-enter copy mode.  Default: on (tmux parity).
    pub scroll_enter_copy_mode: bool,
    /// pwsh-mouse-selection: when on, client-side drag selection behaves like
    /// Windows 11 PowerShell — pane-aware clipping, no copy-on-release (copy
    /// only on right-click), word/line selection on double/triple-click.
    /// Default: off (preserves the legacy pwsh-style copy-on-release).
    pub pwsh_mouse_selection: bool,
    /// mouse-selection: when off, psmux disables its own client-side drag
    /// selection overlay so applications running inside a pane (opencode,
    /// nvim, etc.) can implement their own mouse selection without having
    /// psmux's selection rectangle drawn on top.  Mouse events are still
    /// forwarded to the application (click-to-focus, scroll, app-level
    /// mouse tracking continue to work).  Default: on.  (issue #245)
    pub mouse_selection: bool,
    /// paste-detection: when on (default), Ctrl+V Press is suppressed and the
    /// Windows paste detection mechanism intercepts clipboard content injected
    /// by the console host.  When off, Ctrl+V is forwarded as send-key C-v so
    /// child applications (e.g. neovim visual block mode) can receive it.
    pub paste_detection: bool,
    /// choose-tree-preview: when on, choose-session and choose-tree pickers
    /// open with the live preview pane already visible (no need to press `p`).
    /// Default: off (matches tmux which has no preview-on-by-default option).
    pub choose_tree_preview: bool,
    pub paste_buffers: Vec<String>,
    /// Named paste buffers (HashMap<name, content>). Named buffers are separate
    /// from the positional stack and are accessed via `set-buffer -b name`.
    pub named_buffers: std::collections::HashMap<String, String>,
    /// Auto-increment counter for unnamed buffer names (buffer0, buffer1, etc.)
    pub paste_next_index: u32,
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
    /// TTL cache for `#(cmd)` shell expansions. Without this the format
    /// engine spawns a fresh subprocess on every state_dirty push (~30/s
    /// during active typing), which serializes a slow helper (e.g. pwsh
    /// at ~280 ms cold-start) onto the server main loop and lags echo.
    /// Keyed by command string; entries expire after `status_interval`.
    pub format_shell_cache: std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, String)>>,
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
    /// Config parse warnings (unknown command/option, malformed value, missing
    /// args) collected during a config load or source-file, surfaced to the
    /// user instead of being silently ignored (issue #370 follow-up).
    pub config_warnings: Vec<String>,
    /// 1-based line number currently being parsed, used to prefix warnings as
    /// `file:line: message` (file comes from config::current_config_file()).
    /// None when parsing a single runtime command.
    pub config_warn_line: Option<usize>,
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
    /// Whether DEC private modes 47 / 1049 (alternate screen) are honoured
    /// for new panes (default: on).  When off, full-screen TUI apps that
    /// would normally enter the alt screen instead write straight to the
    /// main grid, so their output ends up in scrollback and is reachable
    /// by `capture-pane -S` and copy-mode (psmux issue #88).  Mirrors
    /// tmux's `set -g alternate-screen on/off`.
    pub allow_alternate_screen: bool,
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
            allow_predictions: false,
            drag: None,
            last_window_area: Rect { x: 0, y: 0, width: 120, height: 30 },
            mouse_enabled: true,
            bold_is_bright: true,
            sixel: true,
            shipped_image_ids: std::collections::HashSet::new(),
            scroll_enter_copy_mode: true,
            pwsh_mouse_selection: false,
            mouse_selection: true,
            paste_detection: true,
            choose_tree_preview: false,
            paste_buffers: Vec::new(),
            named_buffers: std::collections::HashMap::new(),
            paste_next_index: 0,
            status_left: "[#S] ".to_string(),
            status_right: "#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y".to_string(),
            window_base_index: 0,
            copy_anchor: None,
            copy_anchor_scroll_offset: 0,
            copy_pos: None,
            copy_mouse_down_cell: None,
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
            run_shell_rx: None,
            run_shell_tx: None,
            session_name,
            session_id: crate::session::allocate_session_id(),
            socket_name: None,
            attached_clients: 0,
            client_sizes: std::collections::HashMap::new(),
            latest_client_id: None,
            client_registry: std::collections::HashMap::new(),
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
            allow_set_title: false,
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
            user_set_options: std::collections::HashSet::new(),
            pane_border_style: String::new(),
            pane_active_border_style: "fg=green".to_string(),
            pane_border_hover_style: "fg=yellow".to_string(),
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
            command_vi_normal: false,
            status_interval: 15,
            last_status_interval_fire: std::time::Instant::now(),
            format_shell_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
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
            config_warnings: Vec::new(),
            config_warn_line: None,
            set_clipboard: "on".to_string(),
            clipboard_osc52: None,
            bell_forward: false,
            env_shim: true,
            claude_code_fix_tty: true,
            claude_code_force_interactive: true,
            last_hover_pos: None,
            last_mouse_x: 0,
            last_mouse_y: 0,
            status_message: None,
            warm_enabled: std::env::var("PSMUX_NO_WARM").map(|v| v != "1" && v != "true").unwrap_or(true),
            allow_alternate_screen: true,
            warm_pane: None,
            pending_plugin_scripts: Vec::new(),
            control_clients: HashMap::new(),
            session_group: None,
            defaults_suppressed: false,
            forwarded_panes: HashMap::new(),
            next_forward_id: 1,
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
    SessionChooser,
    ZoomPane,
    /// Switch to a named key table (switch-client -T)
    SwitchTable(String),
}

#[derive(Clone)]
pub struct Bind { pub key: (KeyCode, KeyModifiers), pub action: Action, pub repeat: bool }

pub enum CtrlReq {
    NewWindow(Option<String>, Option<String>, bool, Option<String>),  // cmd, name, detached, start_dir
    NewWindowPrint(Option<String>, Option<String>, bool, Option<String>, Option<String>, mpsc::Sender<String>),  // cmd, name, detached, start_dir, format, resp
    SplitWindow(LayoutKind, Option<String>, bool, Option<String>, Option<(u16, bool)>, mpsc::Sender<String>),  // kind, cmd, detached, start_dir, size (value, is_percent), error_resp
    SplitWindowPrint(LayoutKind, Option<String>, bool, Option<String>, Option<(u16, bool)>, Option<String>, mpsc::Sender<String>),  // kind, cmd, detached, start_dir, size (value, is_percent), format, resp
    KillPane,
    KillPaneById(usize),
    CapturePane(mpsc::Sender<String>),
    CapturePaneStyled(mpsc::Sender<String>, Option<i32>, Option<i32>),
    FocusWindow(usize),
    /// Focus window by @N id lookup
    FocusWindowById(usize),
    /// Focus window by name lookup
    FocusWindowByName(String),
    /// Temporary focus for -t targeting: server saves/restores active_idx
    FocusWindowTemp(usize),
    /// Temporary focus by @N id for -t targeting
    FocusWindowByIdTemp(usize),
    /// Temporary focus by name for -t targeting
    FocusWindowByNameTemp(String),
    FocusPane(usize),
    FocusPaneByIndex(usize),
    /// Temporary pane focus for -t targeting
    FocusPaneTemp(usize),
    FocusPaneByIndexTemp(usize),
    SessionInfo(mpsc::Sender<String>),
    /// `list-sessions -F <fmt>` — render the session row using a tmux format
    /// string. Drop-in compat with iTerm2 and other CC clients that always
    /// pass `-F` to get structured output.
    SessionInfoFormat(mpsc::Sender<String>, String),
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
    /// Client-side semantic mouse event: pane-relative coordinates, targeted by pane ID.
    /// Fields: client_id, pane_id, sgr_button, col_0based, row_0based, press
    PaneMouse(u64, usize, u8, i16, i16, bool),
    /// Client-side semantic scroll: targeted by pane ID.
    /// Fields: client_id, pane_id, up (true=up, false=down)
    PaneScroll(u64, usize, bool),
    /// Client-side semantic split resize: set sizes at a tree path.
    /// Fields: client_id, path, new sizes
    SplitSetSizes(u64, Vec<usize>, Vec<u16>),
    /// Client signals border drag is complete — trigger PTY resize.
    /// Fields: client_id
    SplitResizeDone(u64),
    NextWindow,
    PrevWindow,
    RenameWindow(String),
    ListWindows(mpsc::Sender<String>),
    ListWindowsTmux(mpsc::Sender<String>),
    ListWindowsFormat(mpsc::Sender<String>, String),
    ListTree(mpsc::Sender<String>),
    /// Issue #257: simplified layout (split kind/sizes + pane ids)
    /// for a specific window, used for choose-tree preview rendering.
    WindowLayout(usize, mpsc::Sender<String>),
    /// Issue #257: full styled `LayoutJson` (rows_v2 cell runs, titles,
    /// etc.) for a specific window. Lets cross-session previews reuse the
    /// exact same renderer the main viewport uses, instead of replaying
    /// `capture-pane -e` per pane and parsing ANSI by hand.
    WindowDump(usize, mpsc::Sender<String>),
    ToggleSync,
    SetPaneTitle(String),
    SetPaneStyle(String),
    SendKeys(String, bool),
    SendKeysX(String),  // send-keys -X copy-mode-command
    SelectPane(String, bool),
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
    /// swap-pane -t <target>: swap the active pane with the pane identified by
    /// (target, pane_is_id).  When `pane_is_id` is true the value is a pane id
    /// (`%N`); otherwise it is a user-facing pane index that is normalized
    /// using pane-base-index before resolving a positional pane path.
    SwapPaneTarget(usize, bool),
    /// swap-pane -s <src> -t <dst>: swap the two explicit panes named by
    /// `-s` and `-t` (issue #442).  Each pane is a (value, is_id) pair
    /// resolved the same way as `SwapPaneTarget`.  `detach` is true when
    /// `-d` was given: the active pane is left unchanged (following its pane
    /// to the new slot); otherwise, per tmux, the `-t` pane becomes active.
    SwapPaneSrcDst { src: usize, src_is_id: bool, dst: usize, dst_is_id: bool, detach: bool },
    /// swap-pane -t <token>: swap the active pane with the pane at a layout
    /// position token (e.g. `{top-right}`).  Layout-independent.
    SwapPanePosition(String),
    ResizePane(String, u16),
    SetBuffer(String),
    /// Set a named buffer: (name, content)
    SetNamedBuffer(String, String),
    ListBuffers(mpsc::Sender<String>),
    ListBuffersFormat(mpsc::Sender<String>, String),
    ShowBuffer(mpsc::Sender<String>),
    ShowBufferAt(mpsc::Sender<String>, usize),
    /// Show a named buffer by name
    ShowNamedBuffer(mpsc::Sender<String>, String),
    DeleteBuffer,
    DeleteBufferAt(usize),
    /// Delete a named buffer by name
    DeleteNamedBuffer(String),
    PasteBufferAt(usize),
    DisplayMessage(mpsc::Sender<String>, String, Option<usize>, bool, Option<u64>),  // resp, format, target_pane_idx, set_status_bar, duration_override_ms
    /// Like DisplayMessage but resolves -t %N pane ID instead of position. (Issue #332.)
    DisplayMessageById(mpsc::Sender<String>, String, usize, bool, Option<u64>),  // resp, format, pane_id, set_status_bar, duration_override_ms
    LastWindow,
    LastPane,
    RotateWindow(bool),
    DisplayPanes,
    DisplayPaneSelect(usize),
    BreakPane,
    /// join-pane: move a pane from source window into target window as a split.
    /// Fields: src_win (window index), src_pane (positional pane index), target_win,
    /// target_pane, horizontal (true = -h side-by-side, false = -v stacked).
    JoinPane {
        src_win: Option<usize>,
        src_pane: Option<usize>,
        target_win: Option<usize>,
        target_pane: Option<usize>,
        horizontal: bool,
    },
    RespawnPane(Option<String>, bool, Option<String>),  // optional workdir (-c), kill flag (-k), command (-- shell-command)
    BindKey(String, String, String, bool),  // table, key, command, repeat
    UnbindKey(String, Option<String>),  // key, optional table (None = prefix)
    UnbindAll,
    UnbindAllInTable(String),
    ListKeys(mpsc::Sender<String>),
    SetOption(String, String),
    SetOptionQuiet(String, String, bool),  // set-option with quiet flag
    SetOptionUnset(String),  // set-option -u
    SetOptionAppend(String, String),  // set-option -a
    SetOptionOnlyIfUnset(String, String),  // set-option -o
    ShowOptions(mpsc::Sender<String>),
    ShowWindowOptions(mpsc::Sender<String>),
    SourceFile(String),
    MoveWindow(Option<usize>),
    SwapWindow(usize),
    /// link-window: (source window index, target insertion index)
    LinkWindow(Option<usize>, Option<usize>),
    UnlinkWindow,
    /// Set session group (used by new-session -t)
    SetSessionGroup(String),
    FindWindow(mpsc::Sender<String>, String),
    /// move-pane: alias for join-pane
    MovePane {
        src_win: Option<usize>,
        src_pane: Option<usize>,
        target_win: Option<usize>,
        target_pane: Option<usize>,
        horizontal: bool,
    },
    /// Extract a pane and start I/O forwarding for cross-session transfer.
    /// Fields: window index, pane index, response channel.
    /// Response: "FORWARD <id> <port> <pid> <title> <rows> <cols> <screen_b64_len>\n<screen_b64>"
    PaneForwardExtract(usize, usize, mpsc::Sender<String>),
    /// Inject a proxy pane from a cross-session transfer.
    /// Fields: source_session, source_addr, source_key, forward_id, fwd_port,
    ///         pid, title, rows, cols, screen_b64, target_window, target_pane, horizontal
    PaneForwardInject {
        source_session: String,
        source_addr: String,
        source_key: String,
        forward_id: u64,
        fwd_port: u16,
        pid: u32,
        title: String,
        rows: u16,
        cols: u16,
        screen_b64: String,
        target_win: Option<usize>,
        target_pane: Option<usize>,
        horizontal: bool,
    },
    /// Resize a forwarded pane's real PTY. Fields: forward_id, rows, cols.
    PaneForwardResize(u64, u16, u16),
    /// Query child status of a forwarded pane. Fields: forward_id, response channel.
    PaneForwardStatus(u64, mpsc::Sender<String>),
    /// Kill a forwarded pane's child process. Fields: forward_id.
    PaneForwardKill(u64),
    PipePane(String, bool, bool, bool),
    SelectLayout(String),
    NextLayout,
    ListClients(mpsc::Sender<String>),
    ListClientsFormat(mpsc::Sender<String>, String),
    ForceDetachClient(u64),
    /// detach-client -t <tty>: force-detach a client by tty_name (e.g. "/dev/pts/2").
    /// `kill_parent` is the tmux `-P` flag: also tell the client to kill its parent
    /// shell before exiting (issue #275).
    ForceDetachClientByTty(String, bool),
    /// detach-client -a (or no-flag CLI invocation): detach every attached client
    /// of THIS session except the one whose ID is given.  Pass `u64::MAX` from the
    /// CLI one-shot path (no "current" client to exclude).  `kill_parent` honors
    /// the tmux `-P` flag for force-detached clients.
    DetachAllOtherClients(u64, bool),
    /// detach-client -s <session> (where session matches THIS server) or
    /// `psmux detach-client` from CLI: detach every attached client of this session.
    /// `kill_parent` honors the tmux `-P` flag.
    DetachAllClients(bool),
    /// switch-client -t <target> / -n / -p / -l: switch the attached client to another session.
    /// The String carries the resolved target session name (or "" for -n/-p/-l to be
    /// resolved server-side), and the second field carries the flag: 't', 'n', 'p', or 'l'.
    SwitchClient(String, char),
    LockClient,
    RefreshClient,
    /// `refresh-client -B name:what:format` subscription management.
    ControlSubscribe {
        client_id: u64,
        name: String,
        target: String,
        format: String,
    },
    /// `refresh-client -B name:` remove subscription.
    ControlUnsubscribe {
        client_id: u64,
        name: String,
    },
    /// `refresh-client -f pause-after=N` set pause-after flag.
    ControlSetPauseAfter {
        client_id: u64,
        pause_after_secs: Option<u64>,
    },
    /// `refresh-client -A '%N:continue'` resume paused pane output.
    ControlContinuePane {
        client_id: u64,
        pane_id: usize,
    },
    SuspendClient,
    CopyModePageUp,
    ClearHistory,
    SaveBuffer(String),
    LoadBuffer(String),
    SetEnvironment(String, String),
    UnsetEnvironment(String),
    ShowEnvironment(mpsc::Sender<String>),
    SetHook(String, String),
    AppendHook(String, String),
    ShowHooks(mpsc::Sender<String>),
    RemoveHook(String),
    KillServer,
    WaitFor(String, WaitForOp),
    DisplayMenu(String, Option<i16>, Option<i16>),
    DisplayMenuDirect(Menu),
    DisplayPopup(String, String, String, bool, Option<String>),
    ConfirmBefore(String, String),
    ClockMode,
    ResizePaneAbsolute(String, u16),
    ResizePanePercent(String, u8), // axis, percentage (0-100)
    ShowOptionValue(mpsc::Sender<String>, String),
    /// Read a window-scoped option value. Optional window index targets a
    /// specific window (from `show-options -w -t :N`); None falls back to
    /// the active window. Required so per-window overrides like
    /// `automatic-rename` (implicitly off for `-n NAME` windows, #266)
    /// can be reported correctly instead of returning the global value.
    ShowWindowOptionValue(mpsc::Sender<String>, String, Option<usize>),
    ChooseBuffer(mpsc::Sender<String>),
    ServerInfo(mpsc::Sender<String>),
    SendPrefix,
    PrevLayout,
    SwitchClientTable(String),
    ListCommands(mpsc::Sender<String>),
    ResizeWindow(String, u16),
    /// Control-mode client (iTerm2 etc.) reports its viewport size in cells.
    /// Sent on connect (`refresh-client -C w,h`) and whenever the user
    /// drag-resizes the iTerm2 window (`resize-window -x w -y h`).
    /// Updates `app.last_window_area` and resizes all panes accordingly.
    ControlClientResize(u16, u16),
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
    /// Show static text in a popup overlay (title, content).
    /// Used by the persistent client command prompt for list-* commands.
    ShowTextPopup(String, String),
    /// Set status bar message (fire-and-forget, no response channel needed).
    StatusMessage(String),
    /// Clear the command prompt history.
    ClearPromptHistory,
    /// Show the command prompt history in a popup.
    ShowPromptHistory(bool),
    /// Register a control mode client.
    ControlRegister {
        client_id: u64,
        echo: bool,
        notif_tx: mpsc::SyncSender<ControlNotification>,
    },
    /// Deregister a control mode client.
    ControlDeregister {
        client_id: u64,
    },
    /// Open customize-mode (interactive options editor)
    CustomizeMode,
    /// Navigate customize-mode (delta: -1 = up, +1 = down)
    CustomizeNavigate(i32),
    /// Begin editing the selected option in customize-mode
    CustomizeEdit,
    /// Update the edit buffer text in customize-mode
    CustomizeEditUpdate(String),
    /// Confirm the edit (apply value) in customize-mode
    CustomizeEditConfirm,
    /// Cancel the edit in customize-mode
    CustomizeEditCancel,
    /// Reset selected option to default in customize-mode
    CustomizeResetDefault,
    /// Set filter string in customize-mode
    CustomizeFilter(String),
    /// Run an arbitrary command through the server-side execute_command_string
    /// path (same path as keybindings and command prompt).  Response channel
    /// carries "OK" on success or an error string.
    RunCommand(String, mpsc::Sender<String>),
}

/// Global flag set by PTY reader threads when new output arrives.
/// The server loop checks this to use a shorter recv_timeout, reducing
/// keystroke-to-display latency for nested shells (e.g. WSL inside pwsh).
pub static PTY_DATA_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set by the parser thread when any pane's `cpr_pending` flag is raised.
/// Lets the server loop skip the tree walk when no CPR response is needed.
pub static CPR_DATA_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tracked persistent client TCP streams.
/// Connection handlers register clones here so the server can explicitly
/// `shutdown()` them before `process::exit(0)`.  Without this, Windows
/// does not reliably deliver TCP RST on loopback sockets when a process
/// exits, leaving the client's blocking `read_line()` stuck forever.
static PERSISTENT_STREAMS: std::sync::Mutex<Vec<(u64, std::net::TcpStream)>> = std::sync::Mutex::new(Vec::new());

/// Register a persistent client stream tagged with client_id (call from connection handler).
pub fn register_persistent_stream(client_id: u64, stream: &std::net::TcpStream) {
    if let Ok(cloned) = stream.try_clone() {
        if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
            v.push((client_id, cloned));
        }
    }
}

/// Remove a specific client's entry from PERSISTENT_STREAMS without shutting
/// it down. Called by the writer-thread Guard on normal disconnect — the socket
/// is already shut down via ws_shutdown at that point, so we only need to drop
/// the dead clone from the Vec to prevent unbounded accumulation.
pub fn deregister_persistent_stream(client_id: u64) {
    if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
        v.retain(|(cid, _)| *cid != client_id);
    }
}

/// Shut down all tracked persistent client streams so their readers get EOF.
pub fn shutdown_persistent_streams() {
    if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
        for (_, s) in v.drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Shut down a specific client's persistent stream and remove its frame sender.
/// Used by force-detach to disconnect a targeted client.
pub fn shutdown_client_stream(client_id: u64) {
    if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
        v.retain(|(cid, s)| {
            if *cid == client_id {
                let _ = s.shutdown(std::net::Shutdown::Both);
                false
            } else {
                true
            }
        });
    }
    if let Ok(mut v) = FRAME_PUSH_SLOTS.lock() {
        v.retain(|(cid, _)| *cid != client_id);
    }
    remove_directive_channel(client_id);
}

/// Server-push frame slot for persistent (attached) clients.
///
/// Each slot holds at most one pending frame. `push_frame()` overwrites any
/// unconsumed frame; frames are full snapshots, so a stale ready frame has
/// no value once a newer one exists. Memory is bounded to O(clients), not
/// O(frames).
///
/// The slot uses `Mutex<Option<String>>`. The producer (main loop's
/// `push_frame`) locks, replaces, unlocks. The consumer (writer thread)
/// locks, takes, unlocks, then writes to TCP *outside* the lock. Neither
/// side ever holds the lock across blocking I/O, so a slow client cannot
/// stall the main event loop.
///
/// Design constraints:
///   - The lock must not be held across blocking I/O. The writer thread
///     does TCP writes that can block (slow client, full kernel buffer).
///     A design where the writer holds a lock during the write -- and the
///     producer also takes that lock to enqueue -- lets a slow client
///     stall the server main loop.
///   - Per-client storage must be bounded. An unbounded queue (e.g. plain
///     `mpsc::channel`) leaks memory under sustained producer-faster-than-
///     consumer load (rapid copy-mode scroll).
///   - `std::sync::atomic` has no atomic-swap for owned heap values;
///     `AtomicPtr<String>` would require `unsafe` ownership management.
///     `arc-swap` would be lock-free but adds a third-party dependency
///     for a path that is not measured-hot.
pub type FrameSlot = std::sync::Arc<std::sync::Mutex<Option<String>>>;

static FRAME_PUSH_SLOTS: std::sync::Mutex<Vec<(u64, FrameSlot)>> =
    std::sync::Mutex::new(Vec::new());

/// Register a frame slot for a persistent connection's writer thread.
/// Returns the slot Arc for the writer thread to consume from.
pub fn register_frame_channel(client_id: u64) -> FrameSlot {
    let slot: FrameSlot = std::sync::Arc::new(std::sync::Mutex::new(None));
    if let Ok(mut v) = FRAME_PUSH_SLOTS.lock() {
        v.push((client_id, slot.clone()));
    }
    slot
}

/// Push a serialized frame to all persistent clients.
/// Overwrites any unconsumed frame; frames are full snapshots, so only
/// the latest matters. The lock is held only for the duration of an
/// Option::replace, never across I/O.
/// Dead slots (poisoned mutex) are pruned automatically.
pub fn push_frame(frame: &str) {
    if let Ok(mut slots) = FRAME_PUSH_SLOTS.lock() {
        slots.retain(|(_, slot)| {
            match slot.lock() {
                Ok(mut s) => { *s = Some(frame.to_string()); true }
                Err(_) => false, // writer thread panicked; prune
            }
        });
    }
}

/// Check if any persistent clients are registered for push.
pub fn has_frame_receivers() -> bool {
    FRAME_PUSH_SLOTS.lock().map_or(false, |v| !v.is_empty())
}

/// Remove the frame slot for a specific client. Called by the writer thread
/// on exit so the server stops pushing to dead slots and has_frame_receivers()
/// returns false when no live clients remain.
pub fn deregister_frame_channel(client_id: u64) {
    if let Ok(mut v) = FRAME_PUSH_SLOTS.lock() {
        v.retain(|(cid, _)| *cid != client_id);
    }
}

/// Per-client directive channels (queued, not overwritten like frame slots).
/// Used for sending commands/directives (e.g. SWITCH) to specific persistent clients
/// without risk of being overwritten by frame pushes.
static DIRECTIVE_CHANNELS: std::sync::Mutex<Vec<(u64, std::sync::mpsc::Sender<String>)>> =
    std::sync::Mutex::new(Vec::new());

/// Register a directive channel for a persistent client. Returns the receiver
/// for the writer thread to poll.
pub fn register_directive_channel(client_id: u64) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Ok(mut v) = DIRECTIVE_CHANNELS.lock() {
        v.push((client_id, tx));
    }
    rx
}

/// Send a directive to a specific persistent client. Returns true if sent.
pub fn send_directive_to_client(client_id: u64, directive: &str) -> bool {
    if let Ok(channels) = DIRECTIVE_CHANNELS.lock() {
        for (cid, tx) in channels.iter() {
            if *cid == client_id {
                return tx.send(directive.to_string()).is_ok();
            }
        }
    }
    false
}

/// Send a directive to ALL persistent clients.
pub fn send_directive_to_all_clients(directive: &str) {
    if let Ok(channels) = DIRECTIVE_CHANNELS.lock() {
        for (_, tx) in channels.iter() {
            let _ = tx.send(directive.to_string());
        }
    }
}

/// Remove a client's directive channel (called on disconnect).
pub fn remove_directive_channel(client_id: u64) {
    if let Ok(mut v) = DIRECTIVE_CHANNELS.lock() {
        v.retain(|(cid, _)| *cid != client_id);
    }
}

/// Global counter for control mode client IDs.
static NEXT_CONTROL_CLIENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Allocate a unique control mode client ID.
pub fn next_control_client_id() -> u64 {
    NEXT_CONTROL_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
    pub window_name: Option<String>,
    pub pane: Option<usize>,
    pub pane_is_id: bool,
    pub window_is_id: bool,
}

#[cfg(test)]
#[path = "../tests-rs/test_pr267_backpressure_proof.rs"]
mod tests_pr267_backpressure;
