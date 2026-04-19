#[allow(unused_imports)]
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyModifiers};
use portable_pty::MasterPty;
use ratatui::prelude::Rect;
use chrono::Local;

use super::*;

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
