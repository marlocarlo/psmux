//! Nonblocking admission with both count and allocated-byte limits. A full
//! queue is a rejected command, never an invitation to block the server loop.
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}, mpsc};
use std::time::Duration;
use super::{CtrlReq, LayoutKind, Menu, MenuItem, WaitForOp, WindowDumpFormat};

const MAX_REQUESTS: usize = 256;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

impl CtrlReq {
    pub fn queued_bytes(&self) -> usize { std::mem::size_of::<Self>().saturating_add(self.heap_bytes()) }
}

struct Envelope {
    request: Option<CtrlReq>,
    bytes: usize,
    budget: Arc<AtomicUsize>,
}
impl Drop for Envelope {
    fn drop(&mut self) { self.budget.fetch_sub(self.bytes, Ordering::AcqRel); }
}

#[derive(Clone)]
pub struct ControlSender { tx: mpsc::SyncSender<Envelope>, budget: Arc<AtomicUsize> }
pub struct ControlReceiver { rx: mpsc::Receiver<Envelope> }

pub fn control_channel() -> (ControlSender, ControlReceiver) {
    let (tx, rx) = mpsc::sync_channel(MAX_REQUESTS);
    let budget = Arc::new(AtomicUsize::new(0));
    (ControlSender { tx, budget }, ControlReceiver { rx })
}

impl ControlSender {
    pub fn send(&self, request: CtrlReq) -> Result<(), mpsc::SendError<CtrlReq>> {
        let bytes = request.queued_bytes();
        if bytes > MAX_REQUEST_BYTES || self.budget.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |used| used.checked_add(bytes).filter(|total| *total <= MAX_BYTES)).is_err() {
            return Err(mpsc::SendError(request));
        }
        let envelope = Envelope { request: Some(request), bytes, budget: self.budget.clone() };
        self.tx.try_send(envelope).map_err(|error| {
            let mut envelope = match error { mpsc::TrySendError::Full(e) | mpsc::TrySendError::Disconnected(e) => e };
            mpsc::SendError(envelope.request.take().unwrap())
        })
    }
}

impl ControlReceiver {
    pub fn try_recv(&self) -> Result<CtrlReq, mpsc::TryRecvError> {
        self.rx.try_recv().map(|mut envelope| envelope.request.take().unwrap())
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<CtrlReq, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout).map(|mut envelope| envelope.request.take().unwrap())
    }
    pub fn recv(&self) -> Result<CtrlReq, mpsc::RecvError> {
        self.rx.recv().map(|mut envelope| envelope.request.take().unwrap())
    }
}

trait HeapBytes { fn heap_bytes(&self) -> usize; }
impl HeapBytes for String { fn heap_bytes(&self) -> usize { self.capacity() } }
impl<T: HeapBytes> HeapBytes for Vec<T> {
    fn heap_bytes(&self) -> usize { self.capacity().saturating_mul(std::mem::size_of::<T>())
        .saturating_add(self.iter().map(HeapBytes::heap_bytes).sum::<usize>()) }
}
impl<T: HeapBytes> HeapBytes for Option<T> { fn heap_bytes(&self) -> usize { self.as_ref().map_or(0, HeapBytes::heap_bytes) } }
impl<T: HeapBytes> HeapBytes for Box<T> { fn heap_bytes(&self) -> usize { std::mem::size_of::<T>() + (**self).heap_bytes() } }
impl<A: HeapBytes, B: HeapBytes> HeapBytes for (A, B) { fn heap_bytes(&self) -> usize { self.0.heap_bytes() + self.1.heap_bytes() } }
impl<T> HeapBytes for mpsc::Sender<T> { fn heap_bytes(&self) -> usize { 0 } }
impl<T> HeapBytes for mpsc::SyncSender<T> { fn heap_bytes(&self) -> usize { 0 } }
macro_rules! no_heap { ($($t:ty),*) => { $(impl HeapBytes for $t { fn heap_bytes(&self) -> usize { 0 } })* }; }
no_heap!(bool, u8, u16, u32, u64, usize, i16, i32, char, LayoutKind, WaitForOp, WindowDumpFormat);
impl HeapBytes for MenuItem { fn heap_bytes(&self) -> usize { self.name.heap_bytes() + self.command.heap_bytes() } }
impl HeapBytes for Menu { fn heap_bytes(&self) -> usize { self.title.heap_bytes() + self.items.heap_bytes() } }
impl HeapBytes for crate::resize_window::ResizeWindowRequest {
    fn heap_bytes(&self) -> usize {
        if let crate::resize_window::WindowTarget::Name(name) = &self.target { name.heap_bytes() } else { 0 }
    }
}

// Exhaustive per-variant accounting is below: adding a request with owned
// payload must update its admission accounting at compile time.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_queue_rejects_without_waiting_and_preserves_fifo() {
        let (tx, rx) = control_channel();
        for i in 0..MAX_REQUESTS { assert!(tx.send(CtrlReq::FocusWindow(i)).is_ok()); }
        assert!(tx.send(CtrlReq::FocusWindow(999)).is_err());
        for i in 0..MAX_REQUESTS {
            assert!(matches!(rx.try_recv(), Ok(CtrlReq::FocusWindow(value)) if value == i));
        }
        assert_eq!(tx.budget.load(Ordering::Acquire), 0);
        assert!(tx.send(CtrlReq::KillPane).is_ok());
    }

    #[test]
    fn allocated_capacity_and_wrapped_payload_share_byte_limit() {
        let (tx, rx) = control_channel();
        let oversized = String::with_capacity(MAX_REQUEST_BYTES + 1);
        assert!(tx.send(CtrlReq::SendText(oversized)).is_err());
        assert_eq!(tx.budget.load(Ordering::Acquire), 0);
        let mut count = 0;
        loop {
            let (ack, _) = mpsc::channel();
            let req = CtrlReq::CommandRequest(Box::new(CtrlReq::SendText("x".repeat(700_000))), ack);
            if tx.send(req).is_err() { break; }
            count += 1;
        }
        assert!(count > 1 && count < 8);
        assert!(tx.budget.load(Ordering::Acquire) <= MAX_BYTES);
        drop(rx);
        assert_eq!(tx.budget.load(Ordering::Acquire), 0);
        assert!(tx.send(CtrlReq::KillPane).is_err());
        assert_eq!(tx.budget.load(Ordering::Acquire), 0);
    }
}

impl HeapBytes for CtrlReq {
    fn heap_bytes(&self) -> usize {
        match self {
            CtrlReq::CommandRequest(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::NewWindow(v0, v1, v2, v3, v4, v5, v6) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes() + v5.heap_bytes() + v6.heap_bytes(),
            CtrlReq::NewWindowPrint(v0, v1, v2, v3, v4, v5, v6, v7, v8) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes() + v5.heap_bytes() + v6.heap_bytes() + v7.heap_bytes() + v8.heap_bytes(),
            CtrlReq::SplitWindow(v0, v1, v2, v3, v4, v5, v6, v7, v8) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes() + v5.heap_bytes() + v6.heap_bytes() + v7.heap_bytes() + v8.heap_bytes(),
            CtrlReq::SplitWindowPrint(v0, v1, v2, v3, v4, v5, v6, v7, v8, v9) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes() + v5.heap_bytes() + v6.heap_bytes() + v7.heap_bytes() + v8.heap_bytes() + v9.heap_bytes(),
            CtrlReq::NewFloat { command, x, y, w, h, border, title, start_dir, detached, empty, resp } => command.heap_bytes() + x.heap_bytes() + y.heap_bytes() + w.heap_bytes() + h.heap_bytes() + border.heap_bytes() + title.heap_bytes() + start_dir.heap_bytes() + detached.heap_bytes() + empty.heap_bytes() + resp.heap_bytes(),
            CtrlReq::KillPane => 0,
            CtrlReq::KillPaneById(v0) => v0.heap_bytes(),
            CtrlReq::CapturePane(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::CapturePaneStyled(v0, v1, v2, v3, v4) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes(),
            CtrlReq::FocusWindow(v0) => v0.heap_bytes(),
            CtrlReq::FocusWindowById(v0) => v0.heap_bytes(),
            CtrlReq::FocusWindowByName(v0) => v0.heap_bytes(),
            CtrlReq::FocusPane(v0) => v0.heap_bytes(),
            CtrlReq::FocusPaneByIndex(v0) => v0.heap_bytes(),
            CtrlReq::FocusTargetTemp { win, win_is_id, win_name, pane, pane_is_id, resp } => win.heap_bytes() + win_is_id.heap_bytes() + win_name.heap_bytes() + pane.heap_bytes() + pane_is_id.heap_bytes() + resp.heap_bytes(),
            CtrlReq::SessionInfo(v0) => v0.heap_bytes(),
            CtrlReq::SessionInfoFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::CapturePaneRange(v0, v1, v2, v3, v4) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes(),
            CtrlReq::ClientAttach(v0) => v0.heap_bytes(),
            CtrlReq::ClientDetach(v0) => v0.heap_bytes(),
            CtrlReq::DumpLayout(v0) => v0.heap_bytes(),
            CtrlReq::DumpState(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::SendText(v0) => v0.heap_bytes(),
            CtrlReq::SendKey(v0) => v0.heap_bytes(),
            CtrlReq::SendPaste(v0) => v0.heap_bytes(),
            CtrlReq::ZoomPane => 0,
            CtrlReq::PrefixBegin => 0,
            CtrlReq::PrefixEnd => 0,
            CtrlReq::CopyEnter => 0,
            CtrlReq::CopyEnterPageUp => 0,
            CtrlReq::CopyMove(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::CopyAnchor => 0,
            CtrlReq::CopyYank => 0,
            CtrlReq::CopyRectToggle => 0,
            CtrlReq::ClientSize(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::HostColors(v0) => v0.heap_bytes(),
            CtrlReq::FocusPaneCmd(v0) => v0.heap_bytes(),
            CtrlReq::FocusWindowCmd(v0) => v0.heap_bytes(),
            CtrlReq::MouseDown(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseDownRight(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseDownMiddle(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseDrag(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseUp(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseUpRight(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseUpMiddle(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::MouseMove(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::ScrollUp(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::ScrollDown(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::PaneMouse(v0, v1, v2, v3, v4, v5) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes() + v5.heap_bytes(),
            CtrlReq::PaneScroll(v0, v1, v2, v3) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes(),
            CtrlReq::CopyDragBegin(v0, v1, v2, v3, v4, v5, v6) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes() + v5.heap_bytes() + v6.heap_bytes(),
            CtrlReq::SplitSetSizes(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::SplitResizeDone(v0) => v0.heap_bytes(),
            CtrlReq::NextWindow => 0,
            CtrlReq::PrevWindow => 0,
            CtrlReq::RenameWindow(v0) => v0.heap_bytes(),
            CtrlReq::ListWindows(v0) => v0.heap_bytes(),
            CtrlReq::ListWindowsTmux(v0) => v0.heap_bytes(),
            CtrlReq::ListWindowsFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ListTree(v0) => v0.heap_bytes(),
            CtrlReq::WindowLayout(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::WindowDump(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::ToggleSync => 0,
            CtrlReq::SetPaneTitle(v0) => v0.heap_bytes(),
            CtrlReq::SetPaneStyle(v0) => v0.heap_bytes(),
            CtrlReq::SetPaneAttrs { title, style } => title.heap_bytes() + style.heap_bytes(),
            CtrlReq::SendKeys(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SendBytes(v0) => v0.heap_bytes(),
            CtrlReq::SendKeysX(v0) => v0.heap_bytes(),
            CtrlReq::SelectPane(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SelectWindow(v0) => v0.heap_bytes(),
            CtrlReq::ListPanes(v0) => v0.heap_bytes(),
            CtrlReq::ListPanesFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ListAllPanes(v0) => v0.heap_bytes(),
            CtrlReq::ListAllPanesFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::KillWindow => 0,
            CtrlReq::KillWindowTarget { win, win_is_id, name, resp } => win.heap_bytes() + win_is_id.heap_bytes() + name.heap_bytes() + resp.heap_bytes(),
            CtrlReq::KillSession => 0,
            CtrlReq::HasSession(v0) => v0.heap_bytes(),
            CtrlReq::RenameSession(v0) => v0.heap_bytes(),
            CtrlReq::ClaimSession(v0, v1, v2, v3) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes(),
            CtrlReq::SwapPane(v0) => v0.heap_bytes(),
            CtrlReq::SwapPaneTarget(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SwapPaneSrcDst { src, src_is_id, dst, dst_is_id, detach } => src.heap_bytes() + src_is_id.heap_bytes() + dst.heap_bytes() + dst_is_id.heap_bytes() + detach.heap_bytes(),
            CtrlReq::SwapPanePosition(v0) => v0.heap_bytes(),
            CtrlReq::ResizePane(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SetBuffer(v0) => v0.heap_bytes(),
            CtrlReq::SetNamedBuffer(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ListBuffers(v0) => v0.heap_bytes(),
            CtrlReq::ListBuffersFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ShowBuffer(v0) => v0.heap_bytes(),
            CtrlReq::ShowBufferAt(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ShowNamedBuffer(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::DeleteBuffer => 0,
            CtrlReq::DeleteBufferAt(v0) => v0.heap_bytes(),
            CtrlReq::DeleteNamedBuffer(v0) => v0.heap_bytes(),
            CtrlReq::PasteBufferAt(v0) => v0.heap_bytes(),
            CtrlReq::DisplayMessage(v0, v1, v2, v3, v4) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes(),
            CtrlReq::DisplayMessageById(v0, v1, v2, v3, v4) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes(),
            CtrlReq::LastWindow => 0,
            CtrlReq::LastPane => 0,
            CtrlReq::RotateWindow(v0) => v0.heap_bytes(),
            CtrlReq::DisplayPanes => 0,
            CtrlReq::DisplayPaneSelect(v0) => v0.heap_bytes(),
            CtrlReq::BreakPane => 0,
            CtrlReq::JoinPane { src_win, src_pane, target_win, target_pane, horizontal } => src_win.heap_bytes() + src_pane.heap_bytes() + target_win.heap_bytes() + target_pane.heap_bytes() + horizontal.heap_bytes(),
            CtrlReq::RespawnPane(v0, v1, v2, v3) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes(),
            CtrlReq::SetPaneOption(v0, v1, v2, v3) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes(),
            CtrlReq::ShowPaneOptions(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::BindKey(v0, v1, v2, v3) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes(),
            CtrlReq::UnbindKey(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::UnbindAll => 0,
            CtrlReq::UnbindAllInTable(v0) => v0.heap_bytes(),
            CtrlReq::ListKeys(v0) => v0.heap_bytes(),
            CtrlReq::SetOption(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SetOptionQuiet(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::SetOptionUnset(v0) => v0.heap_bytes(),
            CtrlReq::SetOptionAppend(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SetOptionOnlyIfUnset(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::SetOptionToggle(v0) => v0.heap_bytes(),
            CtrlReq::SetWindowSize(v0) => v0.heap_bytes(),
            CtrlReq::ShowOptions(v0) => v0.heap_bytes(),
            CtrlReq::ShowWindowOptions(v0) => v0.heap_bytes(),
            CtrlReq::SourceFile(v0) => v0.heap_bytes(),
            CtrlReq::ExpandFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::MoveWindow { src, dst, detach, kill, renumber, after, before, resp } => src.heap_bytes() + dst.heap_bytes() + detach.heap_bytes() + kill.heap_bytes() + renumber.heap_bytes() + after.heap_bytes() + before.heap_bytes() + resp.heap_bytes(),
            CtrlReq::SwapWindow { src, dst, detach, resp } => src.heap_bytes() + dst.heap_bytes() + detach.heap_bytes() + resp.heap_bytes(),
            CtrlReq::LinkWindow(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::UnlinkWindow => 0,
            CtrlReq::SetSessionGroup(v0) => v0.heap_bytes(),
            CtrlReq::FindWindow(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::MovePane { src_win, src_pane, target_win, target_pane, horizontal } => src_win.heap_bytes() + src_pane.heap_bytes() + target_win.heap_bytes() + target_pane.heap_bytes() + horizontal.heap_bytes(),
            CtrlReq::PaneForwardExtract(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::PaneForwardInject { source_session, source_addr, source_key, forward_id, fwd_port, pid, title, rows, cols, screen_b64, target_win, target_pane, horizontal } => source_session.heap_bytes() + source_addr.heap_bytes() + source_key.heap_bytes() + forward_id.heap_bytes() + fwd_port.heap_bytes() + pid.heap_bytes() + title.heap_bytes() + rows.heap_bytes() + cols.heap_bytes() + screen_b64.heap_bytes() + target_win.heap_bytes() + target_pane.heap_bytes() + horizontal.heap_bytes(),
            CtrlReq::PaneForwardResize(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::PaneForwardStatus(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::PaneForwardKill(v0) => v0.heap_bytes(),
            CtrlReq::PipePane(v0, v1, v2, v3, v4) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes(),
            CtrlReq::SelectLayout(v0) => v0.heap_bytes(),
            CtrlReq::NextLayout => 0,
            CtrlReq::ListClients(v0) => v0.heap_bytes(),
            CtrlReq::ListClientsFormat(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ForceDetachClient(v0) => v0.heap_bytes(),
            CtrlReq::ForceDetachClientByTty(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::DetachAllOtherClients(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::DetachAllClients(v0) => v0.heap_bytes(),
            CtrlReq::SetClientLastSession(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::SwitchClient(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::SwitchClientTarget(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::LockClient => 0,
            CtrlReq::RefreshClient => 0,
            CtrlReq::ControlSubscribe { client_id, name, target, format } => client_id.heap_bytes() + name.heap_bytes() + target.heap_bytes() + format.heap_bytes(),
            CtrlReq::ControlUnsubscribe { client_id, name } => client_id.heap_bytes() + name.heap_bytes(),
            CtrlReq::ControlSetPauseAfter { client_id, pause_after_secs } => client_id.heap_bytes() + pause_after_secs.heap_bytes(),
            CtrlReq::ControlContinuePane { client_id, pane_id } => client_id.heap_bytes() + pane_id.heap_bytes(),
            CtrlReq::SuspendClient => 0,
            CtrlReq::CopyModePageUp => 0,
            CtrlReq::ClearHistory => 0,
            CtrlReq::SaveBuffer(v0) => v0.heap_bytes(),
            CtrlReq::LoadBuffer(v0) => v0.heap_bytes(),
            CtrlReq::SetEnvironment(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::UnsetEnvironment(v0) => v0.heap_bytes(),
            CtrlReq::ShowEnvironment(v0) => v0.heap_bytes(),
            CtrlReq::SetHook(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::AppendHook(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ShowHooks(v0) => v0.heap_bytes(),
            CtrlReq::RemoveHook(v0) => v0.heap_bytes(),
            CtrlReq::KillServer => 0,
            CtrlReq::WaitFor(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::DisplayMenu(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::DisplayMenuDirect(v0) => v0.heap_bytes(),
            CtrlReq::DisplayPopup(v0, v1, v2, v3, v4) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes() + v3.heap_bytes() + v4.heap_bytes(),
            CtrlReq::ConfirmBefore(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ClockMode => 0,
            CtrlReq::ResizePaneAbsolute(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ResizePanePercent(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ShowOptionValue(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ShowWindowOptionValue(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::ChooseBuffer(v0) => v0.heap_bytes(),
            CtrlReq::ServerInfo(v0) => v0.heap_bytes(),
            CtrlReq::SendPrefix => 0,
            CtrlReq::PrevLayout => 0,
            CtrlReq::SwitchClientTable(v0) => v0.heap_bytes(),
            CtrlReq::ListCommands(v0) => v0.heap_bytes(),
            CtrlReq::ResizeWindow(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::ControlClientResize { client_id, window_id, size } => client_id.heap_bytes() + window_id.heap_bytes() + size.heap_bytes(),
            CtrlReq::RespawnWindow(v0, v1, v2) => v0.heap_bytes() + v1.heap_bytes() + v2.heap_bytes(),
            CtrlReq::FocusIn => 0,
            CtrlReq::FocusOut => 0,
            CtrlReq::CommandPrompt(v0) => v0.heap_bytes(),
            CtrlReq::ShowMessages(v0) => v0.heap_bytes(),
            CtrlReq::PopupInput(v0) => v0.heap_bytes(),
            CtrlReq::OverlayClose => 0,
            CtrlReq::ConfirmRespond(v0) => v0.heap_bytes(),
            CtrlReq::MenuSelect(v0) => v0.heap_bytes(),
            CtrlReq::MenuNavigate(v0) => v0.heap_bytes(),
            CtrlReq::ShowTextPopup(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
            CtrlReq::StatusMessage(v0) => v0.heap_bytes(),
            CtrlReq::ClearPromptHistory => 0,
            CtrlReq::ShowPromptHistory(v0) => v0.heap_bytes(),
            CtrlReq::ControlRegister { client_id, echo, notif_tx } => client_id.heap_bytes() + echo.heap_bytes() + notif_tx.heap_bytes(),
            CtrlReq::ControlDeregister { client_id } => client_id.heap_bytes(),
            CtrlReq::CustomizeMode => 0,
            CtrlReq::CustomizeNavigate(v0) => v0.heap_bytes(),
            CtrlReq::CustomizeEdit => 0,
            CtrlReq::CustomizeEditUpdate(v0) => v0.heap_bytes(),
            CtrlReq::CustomizeEditConfirm => 0,
            CtrlReq::CustomizeEditCancel => 0,
            CtrlReq::CustomizeResetDefault => 0,
            CtrlReq::CustomizeFilter(v0) => v0.heap_bytes(),
            CtrlReq::RunCommand(v0, v1) => v0.heap_bytes() + v1.heap_bytes(),
        }
    }
}
