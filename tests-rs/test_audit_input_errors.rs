// PTY-free admission/error propagation regressions; no processes or sessions.
use super::*;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::types::Node;

const ROWS: u16 = 6;
const COLS: u16 = 40;

// ── PTY-free pane scaffolding ──────────────────────────────────────────────

#[derive(Debug)]
struct DummyChild;

#[derive(Debug)]
struct DummyWriter;

struct DummyMaster;

impl std::io::Write for DummyWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { Ok(buf.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl portable_pty::ChildKiller for DummyChild {
    fn kill(&mut self) -> std::io::Result<()> { Ok(()) }
    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(DummyChild)
    }
}

impl portable_pty::Child for DummyChild {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
    }
    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        Ok(portable_pty::ExitStatus::with_exit_code(0))
    }
    fn process_id(&self) -> Option<u32> { None }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> { None }
}

impl portable_pty::MasterPty for DummyMaster {
    fn resize(&self, _size: portable_pty::PtySize) -> Result<(), anyhow::Error> { Ok(()) }
    fn get_size(&self) -> Result<portable_pty::PtySize, anyhow::Error> {
        Ok(portable_pty::PtySize { rows: ROWS, cols: COLS, pixel_width: 0, pixel_height: 0 })
    }
    fn try_clone_reader(&self) -> Result<Box<dyn std::io::Read + Send>, anyhow::Error> {
        Ok(Box::new(std::io::empty()))
    }
    fn take_writer(&self) -> Result<Box<dyn std::io::Write + Send>, anyhow::Error> {
        Ok(Box::new(DummyWriter))
    }
    #[cfg(unix)]
    fn process_group_leader(&self) -> Option<i32> { None }
    #[cfg(unix)]
    fn as_raw_fd(&self) -> Option<std::os::unix::io::RawFd> { None }
    #[cfg(unix)]
    fn tty_name(&self) -> Option<std::path::PathBuf> { None }
}

fn make_pane(id: usize, rows: u16, cols: u16) -> crate::types::Pane {
    let term = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
    let epoch = Instant::now() - Duration::from_secs(2);
    crate::types::Pane {
        master: Box::new(DummyMaster),
        writer: Box::new(DummyWriter),
        child: Box::new(DummyChild),
        term,
        last_rows: rows,
        last_cols: cols,
        id,
        title: format!("pane{id}"),
        title_locked: false,
        child_pid: None,
        data_version: Arc::new(AtomicU64::new(0)),
        last_title_check: epoch,
        last_infer_title: epoch,
        dead: false,
        last_text_input: None,
        last_special_key: None,
        vt_bridge_cache: None,
        vti_mode_cache: None,
        mouse_input_cache: None, win32_input_latched: false,
        scroll_fg_cache: None, mouse_proto_owner: None, wheel_auth: None,
        cursor_shape: Arc::new(AtomicU8::new(0)),
        bell_pending: Arc::new(AtomicBool::new(false)),
        cpr_pending: Arc::new(AtomicBool::new(false)),
        color_query_pending: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        copy_state: None,
        pane_style: None, pane_options: Default::default(),
        squelch_until: None,
        output_ring: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        spawned_at: None,
        start_command: String::new(),
        cwd_hint: None,
    }
}

fn make_window(id: usize) -> crate::types::Window {
    crate::types::Window {
        root: Node::Split { kind: crate::types::LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: "w".to_string(),
        id,
        area: ratatui::layout::Rect::new(0, 0, 120, 30),
        window_size: None,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: vec![],
        zoom_saved: None,
        linked_from: None,
        floating: Vec::new(),
        floating_focus: None,
    }
}

fn app_with_writer(writer: Box<dyn Write + Send>) -> AppState {
    let mut app = AppState::new("input-error-unit".into());
    let mut pane = make_pane(17, ROWS, COLS);
    pane.writer = writer;
    let mut window = make_window(0);
    window.root = Node::Leaf(pane);
    app.windows.push(window);
    app.active_idx = 0;
    app
}

struct Reject(io::ErrorKind);
impl Write for Reject {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(self.0.into()) }
    fn flush(&mut self) -> io::Result<()> { Err(self.0.into()) }
}

#[test]
fn all_live_input_routes_return_backpressure_and_permanent_errors() {
    for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::BrokenPipe] {
        let mut app = app_with_writer(Box::new(Reject(kind)));
        assert_eq!(send_text_to_active(&mut app, "hello").unwrap_err().kind(), kind);
        assert_eq!(send_bytes_to_active(&mut app, &[0xff]).unwrap_err().kind(), kind);
        assert_eq!(send_key_to_active(&mut app, "enter").unwrap_err().kind(), kind);
        assert_eq!(send_key_to_active(&mut app, "f3").unwrap_err().kind(), kind);
        assert_eq!(send_key_to_active(&mut app, "C-/").unwrap_err().kind(), kind);
        assert_eq!(send_paste_to_active(&mut app, "hello\nworld").unwrap_err().kind(), kind);
        assert_eq!(forward_key_to_active(&mut app,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)).unwrap_err().kind(), kind);
    }
}

#[test]
fn synchronized_input_does_not_hide_failed_pane() {
    let mut app = app_with_writer(Box::new(Reject(io::ErrorKind::WouldBlock)));
    app.sync_input = true;
    assert!(send_text_to_active(&mut app, "hello").is_err());
    assert!(send_bytes_to_active(&mut app, &[0xff]).is_err());
    assert!(send_key_to_active(&mut app, "up").is_err());
    assert!(send_paste_to_active(&mut app, "paste").is_err());
}

#[test]
fn paste_brackets_and_normalized_text_are_one_admission() {
    struct Calls(Vec<Vec<u8>>);
    impl Write for Calls {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.0.push(bytes.to_vec()); Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let mut writer = Calls(Vec::new());
    write_paste_chunked(&mut writer, b"line1\r\nline2\n", true).unwrap();
    assert_eq!(writer.0, vec![b"\x1b[200~line1\rline2\r\x1b[201~".to_vec()]);
}

#[test]
fn rejected_paste_does_not_emit_an_unclosed_bracket() {
    struct Budget { bytes: Vec<u8>, limit: usize }
    impl Write for Budget {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            if self.bytes.len() + b.len() > self.limit { return Err(io::ErrorKind::WouldBlock.into()); }
            self.bytes.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let mut writer = Budget { bytes: Vec::new(), limit: 8 };
    assert_eq!(write_paste_chunked(&mut writer, b"hello", true).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(writer.bytes.is_empty());
}

#[test]
fn idle_input_failure_is_reported_once_and_recovers_after_replacement() {
    let mut app = app_with_writer(Box::new(Reject(io::ErrorKind::BrokenPipe)));
    let failures = crate::pane::take_input_failures(&mut app);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, 17);
    assert!(crate::pane::take_input_failures(&mut app).is_empty());
    crate::tree::active_pane_mut(&mut app.windows[0].root, &Vec::new()).unwrap().writer = Box::new(DummyWriter);
    assert!(crate::pane::take_input_failures(&mut app).is_empty());
    crate::tree::active_pane_mut(&mut app.windows[0].root, &Vec::new()).unwrap().writer = Box::new(Reject(io::ErrorKind::BrokenPipe));
    assert_eq!(crate::pane::take_input_failures(&mut app).len(), 1);
}
