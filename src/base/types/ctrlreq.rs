#[allow(unused_imports)]
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyModifiers};
use portable_pty::MasterPty;
use ratatui::prelude::Rect;
use chrono::Local;

use super::*;

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
    /// Focus window by name lookup
    FocusWindowByName(String),
    /// Temporary focus for -t targeting: server saves/restores active_idx
    FocusWindowTemp(usize),
    /// Temporary focus by name for -t targeting
    FocusWindowByNameTemp(String),
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
    DeleteBufferAt(usize),
    PasteBufferAt(usize),
    DisplayMessage(mpsc::Sender<String>, String, Option<usize>, bool, Option<u64>),  // resp, format, target_pane_idx, set_status_bar, duration_override_ms
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
    RespawnPane(Option<String>, bool),  // optional workdir (-c), kill flag (-k)
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

/// Tracked persistent client TCP streams.
/// Connection handlers register clones here so the server can explicitly
/// `shutdown()` them before `process::exit(0)`.  Without this, Windows
/// does not reliably deliver TCP RST on loopback sockets when a process
/// exits, leaving the client's blocking `read_line()` stuck forever.
pub(crate) static PERSISTENT_STREAMS: std::sync::Mutex<Vec<(u64, std::net::TcpStream)>> = std::sync::Mutex::new(Vec::new());

/// Register a persistent client stream tagged with client_id (call from connection handler).
pub fn register_persistent_stream(client_id: u64, stream: &std::net::TcpStream) {
    if let Ok(cloned) = stream.try_clone() {
        if let Ok(mut v) = PERSISTENT_STREAMS.lock() {
            v.push((client_id, cloned));
        }
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
    if let Ok(mut v) = FRAME_PUSH_CHANNELS.lock() {
        v.retain(|(cid, _)| *cid != client_id);
    }
    remove_directive_channel(client_id);
}

/// Server-push frame channels for persistent (attached) clients.
/// Uses a bounded `sync_channel` with a small capacity to allow short bursts
/// of frames to queue without dropping, while still bounding memory.
///
/// When the channel is full (sustained high-throughput, e.g. rapid scroll in
/// copy mode), the oldest unconsumed frame is drained before pushing the new
/// one, so the client always receives the latest frame without unbounded
/// memory growth.
///
/// Previous single-slot design (694156e) overwrote unconsumed frames, which
/// fixed a memory leak during copy-mode scrolling but dropped intermediate
/// frames during fast typing — the cursor advanced but characters were not
/// rendered.  A bounded channel preserves intermediate frames under normal
/// typing speeds while still capping memory for pathological scroll bursts.
pub(crate) const FRAME_CHANNEL_CAPACITY: usize = 16;

pub type FrameChannel = std::sync::Arc<FrameChannelInner>;

pub struct FrameChannelInner {
    pub tx: std::sync::mpsc::SyncSender<String>,
    pub rx: std::sync::Mutex<std::sync::mpsc::Receiver<String>>,
}

pub(crate) static FRAME_PUSH_CHANNELS: std::sync::Mutex<Vec<(u64, std::sync::mpsc::SyncSender<String>)>> =
    std::sync::Mutex::new(Vec::new());

/// Register a bounded frame channel for a persistent connection's writer
/// thread, tagged with client_id for targeted operations (e.g. force-detach).
/// Returns the channel Arc for the writer thread to consume from.
pub fn register_frame_channel(client_id: u64) -> FrameChannel {
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(FRAME_CHANNEL_CAPACITY);
    if let Ok(mut v) = FRAME_PUSH_CHANNELS.lock() {
        v.push((client_id, tx.clone()));
    }
    std::sync::Arc::new(FrameChannelInner {
        tx,
        rx: std::sync::Mutex::new(rx),
    })
}

