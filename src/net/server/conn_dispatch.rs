use std::sync::mpsc;
use std::net::TcpStream;
use crate::types::CtrlReq;

/// Result of dispatching a command to a handler group.
pub(crate) enum DispatchResult {
    /// Command was handled, continue the command loop
    Handled,
    /// Command was handled, break out of the command loop
    Break,
    /// Command was handled, re-enter loop with new command (for if-shell)
    ContinueWith(String),
    /// Command was not recognized by this handler group
    Unhandled,
}

/// Shared context for command dispatch functions.
pub(crate) struct DispatchCtx<'a> {
    pub tx: &'a mpsc::Sender<CtrlReq>,
    pub write_stream: &'a mut TcpStream,
    pub persistent: bool,
    pub resp_tx_opt: &'a Option<mpsc::Sender<mpsc::Receiver<String>>>,
    pub client_id: u64,
    pub target_win: Option<usize>,
    pub target_pane: Option<usize>,
    pub pane_is_id: bool,
    pub raw_target: Option<String>,
    pub line: String,
    pub attached_sent: &'a mut bool,
}
