use std::io::{self, Write, BufRead as _};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::net::TcpListener;
use std::env;

use crossterm::event::{self, Event, KeyEventKind};
use portable_pty::{PtySize, native_pty_system};
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ratatui::style::{Style, Modifier};
use chrono::Local;

use crate::types::{AppState, CtrlReq, LayoutKind, Mode};
use crate::tree::{active_pane_mut, compute_rects, resize_all_panes, kill_all_children,
    find_window_index_by_id, focus_pane_by_id, focus_pane_by_index, reap_children};
use crate::pane::{create_window, split_active_with_command, kill_active_pane, kill_pane_by_id};
use crate::input::{handle_key, handle_mouse, send_text_to_active, send_key_to_active, send_paste_to_active};
use crate::rendering::{render_window, parse_status, centered_rect};
use crate::style::{parse_tmux_style, parse_inline_styles, spans_visual_width, truncate_spans_to_width};
use crate::config::load_config;
use crate::cli::parse_target;
use crate::copy_mode::{enter_copy_mode, move_copy_cursor, current_prompt_pos, yank_selection,
    capture_active_pane_text, capture_active_pane_range, capture_active_pane_styled};
use crate::layout::dump_layout_json;
use crate::window_ops::{toggle_zoom, remote_mouse_down, remote_mouse_drag, remote_mouse_up,
    remote_mouse_button, remote_mouse_motion, remote_scroll_up, remote_scroll_down};
use crate::util::{list_windows_json, list_tree_json, list_windows_tmux};

// ── Bracket Paste Detector ───────────────────────────────────────────────────
//
// Crossterm 0.28 on Windows does NOT emit Event::Paste.  The outer terminal
// (Windows Terminal) sends \e[200~<text>\e[201~ as individual KEY_EVENT records,
// and crossterm delivers each character as Event::Key.
//
// This state machine detects bracket paste sequences from individual key events
// and accumulates the paste content.  When the close sequence is detected, the
// complete text is returned for delivery via send_paste_to_active().
//
// The SSH input path already has its own bracket paste parser in ssh_input.rs;
// this detector covers the local (non-SSH, crossterm) input path on Windows.

#[cfg(windows)]
mod bracket_paste_detect {
    use crossterm::event::{KeyCode, KeyEvent};
    use std::time::Instant;

    const OPEN:  &[u8] = b"\x1b[200~";
    const CLOSE: &[u8] = b"\x1b[201~";

    pub enum State {
        /// Normal operation; watching for start of \e[200~.
        Idle,
        /// Matching characters of the open sequence at index `idx`.
        MatchOpen { idx: usize, pending: Vec<KeyEvent>, started: Instant },
        /// Accumulating paste content between open and close sequences.
        Pasting { buf: String },
        /// Inside paste, matching characters of the close sequence.
        MatchClose { idx: usize, buf: String },
    }

    pub enum Action {
        /// Key should be forwarded to handle_key normally.
        Forward(KeyEvent),
        /// Key events that were buffered during a failed open match.
        /// Replay them all through handle_key.
        Replay(Vec<KeyEvent>, KeyEvent),
        /// Key was consumed (part of bracket sequence or paste content).
        Consumed,
        /// A complete paste was detected.
        Paste(String),
    }

    /// Returned by flush_timeout when buffered keys expire.
    pub enum TimeoutAction {
        /// Nothing buffered, no action needed.
        None,
        /// Buffered keys should be replayed through handle_key.
        Replay(Vec<KeyEvent>),
    }

    impl State {
        pub fn new() -> Self { State::Idle }
    }

    /// Check if the bracket paste detector has buffered an ESC that
    /// hasn't been followed by the rest of \e[200~ within 5ms.
    /// If so, flush the pending events.  This prevents Ctrl+[ (ESC)
    /// from being swallowed indefinitely while the detector waits
    /// for a `[` that never arrives.
    pub fn flush_timeout(state: &mut State) -> TimeoutAction {
        // Check if we're in MatchOpen and the timeout expired WITHOUT
        // moving the state (avoids borrow issues).
        let expired = matches!(state, State::MatchOpen { started, .. } if started.elapsed().as_millis() >= 5);
        if expired {
            let old = std::mem::replace(state, State::Idle);
            if let State::MatchOpen { pending, .. } = old {
                return TimeoutAction::Replay(pending);
            }
        }
        TimeoutAction::None
    }

    fn key_byte(key: &KeyEvent) -> Option<u8> {
        match key.code {
            KeyCode::Esc => Some(0x1b),
            KeyCode::Char(c) if (c as u32) < 128 => Some(c as u8),
            KeyCode::Enter => Some(b'\r'),
            _ => None,
        }
    }

    pub fn feed(state: &mut State, key: KeyEvent) -> Action {
        // We need to take ownership of state to replace it.
        let old = std::mem::replace(state, State::Idle);
        match old {
            State::Idle => {
                if let Some(b) = key_byte(&key) {
                    if b == OPEN[0] {
                        // Potential bracket paste start — buffer this ESC.
                        *state = State::MatchOpen {
                            idx: 1,
                            pending: vec![key],
                            started: Instant::now(),
                        };
                        return Action::Consumed;
                    }
                }
                Action::Forward(key)
            }
            State::MatchOpen { idx, mut pending, .. } => {
                if let Some(b) = key_byte(&key) {
                    if b == OPEN[idx] {
                        pending.push(key);
                        let next = idx + 1;
                        if next >= OPEN.len() {
                            // Full open sequence matched!
                            *state = State::Pasting { buf: String::new() };
                            return Action::Consumed;
                        }
                        *state = State::MatchOpen { idx: next, pending, started: Instant::now() };
                        return Action::Consumed;
                    }
                }
                // Mismatch — replay buffered keys and process current.
                *state = State::Idle;
                Action::Replay(pending, key)
            }
            State::Pasting { mut buf } => {
                if let Some(b) = key_byte(&key) {
                    if b == CLOSE[0] {
                        // Potential close sequence start.
                        *state = State::MatchClose { idx: 1, buf };
                        return Action::Consumed;
                    }
                }
                // Regular paste content.
                match key.code {
                    KeyCode::Char(c) => buf.push(c),
                    KeyCode::Enter   => buf.push('\r'),
                    KeyCode::Tab     => buf.push('\t'),
                    KeyCode::Esc     => buf.push('\x1b'),
                    _ => {} // ignore non-text keys during paste
                }
                *state = State::Pasting { buf };
                Action::Consumed
            }
            State::MatchClose { idx, mut buf } => {
                if let Some(b) = key_byte(&key) {
                    if b == CLOSE[idx] {
                        let next = idx + 1;
                        if next >= CLOSE.len() {
                            // Full close sequence — paste complete!
                            *state = State::Idle;
                            return Action::Paste(buf);
                        }
                        *state = State::MatchClose { idx: next, buf };
                        return Action::Consumed;
                    }
                }
                // Close match failed — flush partial close chars into paste buf.
                for i in 0..idx {
                    buf.push(CLOSE[i] as char);
                }
                // Re-check current key: it might start a new close sequence.
                *state = State::Pasting { buf };
                return feed(state, key);
            }
        }
    }

    #[cfg(test)]
    #[path = "../../../tests-rs/test_app_bracket_paste.rs"]
    mod tests;
}

pub fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let pty_system = native_pty_system();

    let mut app = AppState::new(
        env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| "default".to_string())
    );
    app.last_window_area = Rect { x: 0, y: 0, width: 0, height: 0 };
    app.attached_clients = 1;

    load_config(&mut app);

    create_window(&*pty_system, &mut app, None, None)?;

    let (tx, rx) = mpsc::channel::<CtrlReq>();
    app.control_rx = Some(rx);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    app.control_port = Some(port);
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let dir = format!("{}\\.psmux", home);
    let _ = std::fs::create_dir_all(&dir);
    let regpath = format!("{}\\{}.port", dir, app.port_file_base());
    let _ = std::fs::write(&regpath, port.to_string());
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(stream) = conn {
                let tx = tx.clone();
                // Handle each connection in its own thread so rapid-fire
                // commands (e.g. 200x new-window) don't queue behind each
                // other on the accept loop.
                thread::spawn(move || {
                let mut stream = stream;
                let mut line = String::new();
                let mut r = io::BufReader::new(stream.try_clone().unwrap());
                let _ = r.read_line(&mut line);
                
                // Check for optional TARGET line (for session:window.pane addressing)
                let mut global_target_win: Option<usize> = None;
                let mut global_target_pane: Option<usize> = None;
                let mut global_pane_is_id = false;
                if line.trim().starts_with("TARGET ") {
                    let target_spec = line.trim().strip_prefix("TARGET ").unwrap_or("");
                    let parsed = parse_target(target_spec);
                    global_target_win = parsed.window;
                    global_target_pane = parsed.pane;
                    global_pane_is_id = parsed.pane_is_id;
                    // Now read the actual command line
                    line.clear();
                    let _ = r.read_line(&mut line);
                }
                
                let mut parts = line.split_whitespace();
                let cmd = parts.next().unwrap_or("");
                // parse optional target specifier
                let args: Vec<&str> = parts.by_ref().collect();
                let mut target_win: Option<usize> = global_target_win;
                let mut target_pane: Option<usize> = global_target_pane;
                let mut pane_is_id = global_pane_is_id;
                let mut start_line: Option<i32> = None;
                let mut end_line: Option<i32> = None;
                let mut i = 0;
                while i < args.len() {
                    if args[i] == "-t" {
                        if let Some(v) = args.get(i+1) {
                            // Parse using parse_target for consistent handling
                            let pt = parse_target(v);
                            if pt.window.is_some() { target_win = pt.window; }
                            if pt.pane.is_some() { 
                                target_pane = pt.pane;
                                pane_is_id = pt.pane_is_id;
                            }
                        }
                        i += 2; continue;
                    } else if args[i] == "-S" {
                        if let Some(v) = args.get(i+1) { if let Ok(n) = v.parse::<i32>() { start_line = Some(n); } }
                        i += 2; continue;
                    } else if args[i] == "-E" {
                        if let Some(v) = args.get(i+1) { if let Ok(n) = v.parse::<i32>() { end_line = Some(n); } }
                        i += 2; continue;
                    }
                    i += 1;
                }
                let is_focus_cmd = matches!(cmd, "select-window" | "selectw" | "select-pane" | "selectp");
                if let Some(wid) = target_win {
                    if is_focus_cmd {
                        let _ = tx.send(CtrlReq::FocusWindow(wid));
                    } else {
                        let _ = tx.send(CtrlReq::FocusWindowTemp(wid));
                    }
                }
                let targeted_kill_pane_id = if matches!(cmd, "kill-pane") && pane_is_id {
                    target_pane
                } else {
                    None
                };
                if targeted_kill_pane_id.is_none() {
                    if let Some(pid) = target_pane {
                        if is_focus_cmd {
                            if pane_is_id {
                                let _ = tx.send(CtrlReq::FocusPane(pid));
                            } else {
                                let _ = tx.send(CtrlReq::FocusPaneByIndex(pid));
                            }
                        } else {
                            if pane_is_id {
                                let _ = tx.send(CtrlReq::FocusPaneTemp(pid));
                            } else {
                                let _ = tx.send(CtrlReq::FocusPaneByIndexTemp(pid));
                            }
                        }
                    }
                }
                match cmd {
                    "new-window" => {
                        let name: Option<String> = args.windows(2).find(|w| w[0] == "-n").map(|w| w[1].trim_matches('"').to_string());
                        let cmd_str: Option<String> = args.iter()
                            .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-n" && w[1] == **a)))
                            .map(|s| s.trim_matches('"').to_string());
                        let _ = tx.send(CtrlReq::NewWindow(cmd_str, name, false, None));
                        // Write immediate acknowledgment so the client's read()
                        // returns promptly instead of waiting for stream close.
                        let _ = write!(stream, "OK\n");
                        let _ = stream.flush();
                    }
                    "split-window" => {
                        let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
                        // Parse optional command - find first non-flag argument after flags
                        let cmd_str: Option<String> = args.iter()
                            .find(|a| !a.starts_with('-'))
                            .map(|s| s.trim_matches('"').to_string());
                        let (rtx, _rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, false, None, None, rtx));
                        let _ = write!(stream, "OK\n");
                        let _ = stream.flush();
                    }
                    "kill-pane" => {
                        if let Some(pid) = targeted_kill_pane_id {
                            let _ = tx.send(CtrlReq::KillPaneById(pid));
                        } else {
                            let _ = tx.send(CtrlReq::KillPane);
                        }
                        let _ = write!(stream, "OK\n");
                        let _ = stream.flush();
                    }
                    "capture-pane" => {
                        let escape_seqs = args.iter().any(|a| *a == "-e");
                        let (rtx, rrx) = mpsc::channel::<String>();
                        if escape_seqs {
                            let _ = tx.send(CtrlReq::CapturePaneStyled(rtx, start_line, end_line));
                        } else if start_line.is_some() || end_line.is_some() {
                            let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start_line, end_line));
                        } else {
                            let _ = tx.send(CtrlReq::CapturePane(rtx));
                        }
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "client-attach" => { let _ = tx.send(CtrlReq::ClientAttach(0)); let _ = write!(stream, "ok\n"); }
                    "client-detach" => { let _ = tx.send(CtrlReq::ClientDetach(0)); let _ = write!(stream, "ok\n"); }
                    "session-info" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::SessionInfo(rtx));
                        if let Ok(line) = rrx.recv() { let _ = write!(stream, "{}", line); let _ = stream.flush(); }
                    }
                    "list-windows" | "lsw" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        if args.iter().any(|a| *a == "-J") {
                            let _ = tx.send(CtrlReq::ListWindows(rtx));
                        } else {
                            let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
                        }
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}\n", text); let _ = stream.flush(); }
                    }
                    "list-panes" | "lsp" => {
                        let all = args.iter().any(|a| *a == "-a" || *a == "-s");
                        let (rtx, rrx) = mpsc::channel::<String>();
                        if all {
                            let _ = tx.send(CtrlReq::ListAllPanes(rtx));
                        } else {
                            let _ = tx.send(CtrlReq::ListPanes(rtx));
                        }
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}\n", text); let _ = stream.flush(); }
                    }
                    "list-clients" | "lsc" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ListClients(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}\n", text); let _ = stream.flush(); }
                    }
                    "show-hooks" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowHooks(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}\n", text); let _ = stream.flush(); }
                    }
                    "list-commands" | "lscm" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ListCommands(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}\n", text); let _ = stream.flush(); }
                    }
                    "source-file" | "source" => {
                        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if let Some(path) = non_flag_args.first() {
                            let _ = tx.send(CtrlReq::SourceFile(path.to_string()));
                        }
                    }
                    _ => {}
                }
                }); // end per-connection thread
            }
        }
    });

    let mut last_resize = Instant::now();
    let mut last_reap = Instant::now();
    let mut quit = false;
    #[cfg(windows)]
    let mut bp_state = bracket_paste_detect::State::new();
    // Cache last-sent DECSCUSR code to avoid redundant writes that
    // reset Windows Terminal's cursor blink timer every frame.
    let mut last_cursor_style: u8 = 255;
    loop {
        // Hide cursor before diff-write so the cursor doesn't visibly
        // jump around while ratatui writes cell changes.
        let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide);
        terminal.draw(|f| {
            let area = f.area();
            let status_at_top = app.status_position == "top";
            let status_h: u16 = if app.status_visible { 1 } else { 0 };
            let constraints = if status_at_top {
                vec![Constraint::Length(status_h), Constraint::Min(1)]
            } else {
                vec![Constraint::Min(1), Constraint::Length(status_h)]
            };
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(area);

            let (content_chunk, status_chunk) = if status_at_top {
                (chunks[1], chunks[0])
            } else {
                (chunks[0], chunks[1])
            };

            app.last_window_area = content_chunk;
            render_window(f, &mut app, content_chunk);

            let _mode_str = match app.mode { 
                Mode::Passthrough => "", 
                Mode::Prefix { .. } => "PREFIX", 
                Mode::CommandPrompt { .. } => ":", 
                Mode::WindowChooser { .. } => "W", 
                Mode::RenamePrompt { .. } => "REN", 
                Mode::RenameSessionPrompt { .. } => "REN-S",
                Mode::CopyMode => "CPY", 
                Mode::CopySearch { .. } => "SEARCH",
                Mode::PaneChooser { .. } => "PANE",
                Mode::MenuMode { .. } => "MENU",
                Mode::PopupMode { .. } => "POPUP",
                Mode::ConfirmMode { .. } => "CONFIRM",
                Mode::ClockMode => "CLOCK",
                Mode::BufferChooser { .. } => "BUF",
                Mode::WindowIndexPrompt { .. } => "WIN#",
                Mode::CustomizeMode { .. } => "CUSTOMIZE",
            };
            let time_str = Local::now().format("%H:%M").to_string();

            // Parse status-style to get the base status bar style (tmux default: bg=green,fg=black)
            let base_status_style = parse_tmux_style(&app.status_style);
            
            // Expand status-left using the format engine for full format var support
            let expanded_left = crate::format::expand_format(&app.status_left, &app);
            let status_spans = parse_status(&expanded_left, &app, &time_str);
            
            // Expand status-right using the format engine
            let expanded_right = crate::format::expand_format(&app.status_right, &app);
            let mut right_spans = parse_status(&expanded_right, &app, &time_str);

            // Build status bar: left status + window tabs + right-aligned time
            let left_style = if app.status_left_style.is_empty() {
                base_status_style
            } else {
                parse_tmux_style(&app.status_left_style)
            };
            let mut combined: Vec<Span<'static>> = status_spans.into_iter().map(|s| {
                // Apply left style as base, but let inline #[...] overrides win
                if s.style == Style::default() {
                    Span::styled(s.content.into_owned(), left_style)
                } else { s }
            }).collect();
            // Enforce status-left-length (tmux truncates left portion to this width)
            truncate_spans_to_width(&mut combined, app.status_left_length);
            combined.push(Span::styled(" ".to_string(), base_status_style));

            // Track x position for tab click detection
            let status_x = chunks[1].x;
            let mut cursor_x: u16 = status_x;
            for s in combined.iter() {
                cursor_x += unicode_width::UnicodeWidthStr::width(s.content.as_ref()) as u16;
            }

            // Parse window-status styles
            let ws_style = if app.window_status_style.is_empty() {
                base_status_style
            } else {
                parse_tmux_style(&app.window_status_style)
            };
            let wsc_style = if app.window_status_current_style.is_empty() {
                // tmux default: no special current style, just same as status
                base_status_style
            } else {
                parse_tmux_style(&app.window_status_current_style)
            };
            let wsa_style = if app.window_status_activity_style.is_empty() {
                base_status_style.add_modifier(Modifier::REVERSED)
            } else {
                parse_tmux_style(&app.window_status_activity_style)
            };
            let wsb_style = if app.window_status_bell_style.is_empty() {
                base_status_style.add_modifier(Modifier::REVERSED)
            } else {
                parse_tmux_style(&app.window_status_bell_style)
            };
            let wsl_style = if app.window_status_last_style.is_empty() {
                base_status_style
            } else {
                parse_tmux_style(&app.window_status_last_style)
            };

            // Render window tabs using window-status-format / window-status-current-format
            let mut tab_pos: Vec<(usize, u16, u16)> = Vec::new();
            let sep = &app.window_status_separator;
            for (i, _w) in app.windows.iter().enumerate() {
                if i > 0 {
                    // Parse inline styles in separator (e.g. "#[fg=#44475a]|")
                    let sep_spans = parse_inline_styles(sep, base_status_style);
                    let sep_w = spans_visual_width(&sep_spans) as u16;
                    combined.extend(sep_spans);
                    cursor_x += sep_w;
                }
                let fmt = if i == app.active_idx {
                    &app.window_status_current_format
                } else {
                    &app.window_status_format
                };
                let label = crate::format::expand_format_for_window(fmt, &app, i);
                
                // Choose style based on window state
                let win = &app.windows[i];
                let fallback_style = if i == app.active_idx {
                    wsc_style
                } else if win.bell_flag {
                    wsb_style
                } else if win.activity_flag {
                    wsa_style
                } else if i == app.last_window_idx {
                    wsl_style
                } else {
                    ws_style
                };
                // Parse inline #[fg=...,bg=...] style directives from theme format strings
                let tab_spans = parse_inline_styles(&label, fallback_style);
                let start_x = cursor_x;
                let visual_w = spans_visual_width(&tab_spans) as u16;
                cursor_x += visual_w;
                tab_pos.push((i, start_x, cursor_x));
                combined.extend(tab_spans);
            }
            app.tab_positions = tab_pos;

            // Right-align the status-right
            let right_style = if app.status_right_style.is_empty() {
                base_status_style
            } else {
                parse_tmux_style(&app.status_right_style)
            };
            // Enforce status-right-length (tmux truncates right portion to this width)
            let mut right_styled: Vec<Span<'static>> = right_spans.drain(..).map(|s| {
                if s.style == Style::default() {
                    Span::styled(s.content.into_owned(), right_style)
                } else { s }
            }).collect();
            truncate_spans_to_width(&mut right_styled, app.status_right_length);
            combined.push(Span::styled(" ".to_string(), base_status_style));
            combined.extend(right_styled);
            // Truncate overall status line to fit the available width
            truncate_spans_to_width(&mut combined, status_chunk.width as usize);
            let status_bar = Paragraph::new(Line::from(combined)).style(base_status_style);
            f.render_widget(Clear, status_chunk);
            f.render_widget(status_bar, status_chunk);

            // Display-message override: show transient message on status bar
            if let Some((ref msg, since)) = app.status_message {
                if since.elapsed().as_millis() < app.display_time_ms as u128 {
                    let msg_style = parse_tmux_style(&app.message_style);
                    let para = Paragraph::new(msg.as_str()).style(msg_style);
                    f.render_widget(Clear, status_chunk);
                    f.render_widget(para, status_chunk);
                } else {
                    app.status_message = None;
                }
            }

            // Command prompt — render at bottom (tmux style), not centered popup
            if let Mode::CommandPrompt { input, cursor } = &app.mode {
                let msg_style = parse_tmux_style(&app.message_command_style);
                let prompt_text = format!(":{}", input);
                let prompt_area = status_chunk; // Replace the status bar line
                let para = Paragraph::new(prompt_text).style(msg_style);
                f.render_widget(Clear, prompt_area);
                f.render_widget(para, prompt_area);
                // Place cursor at the right position in the prompt
                let cx = prompt_area.x + 1 + *cursor as u16; // +1 for ':'
                f.set_cursor_position((cx, prompt_area.y));
            }

            if let Mode::WindowChooser { selected, ref tree } = app.mode {
                let mut lines: Vec<Line> = Vec::new();
                for (i, entry) in tree.iter().enumerate() {
                    let marker = if i == selected { ">" } else { " " };
                    if entry.is_session_header {
                        let tag = if entry.is_current_session { " (attached)" } else { "" };
                        lines.push(Line::from(format!("{} {} {}{}",
                            marker,
                            if entry.is_current_session { "▼" } else { "▶" },
                            entry.session_name,
                            tag,
                        )).style(Style::default().fg(Color::Yellow).add_modifier(ratatui::style::Modifier::BOLD)));
                    } else {
                        let active_mark = if entry.is_active_window { "*" } else { " " };
                        let wi = entry.window_index.unwrap_or(0);
                        lines.push(Line::from(format!("{}   {}: {}{} ({} panes) [{}]",
                            marker, wi, entry.window_name, active_mark,
                            entry.window_panes, entry.window_size,
                        )));
                    }
                }
                // Cap overlay height to available terminal space
                let height = (lines.len() as u16 + 2)
                    .min(20)
                    .min(area.height.saturating_sub(2));
                let overlay = Paragraph::new(Text::from(lines)).block(Block::default().borders(Borders::ALL).title("choose-tree"));
                let oa = centered_rect(70, height, area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::BufferChooser { selected } = app.mode {
                let mut lines: Vec<Line> = Vec::new();
                if app.paste_buffers.is_empty() {
                    lines.push(Line::from("  (no buffers)"));
                } else {
                    for (i, buf) in app.paste_buffers.iter().enumerate() {
                        let marker = if i == selected { ">" } else { " " };
                        let preview: String = buf.chars().take(40).map(|c| if c == '\n' { '↵' } else { c }).collect();
                        lines.push(Line::from(format!("{} {:>2}: {:>5} bytes  {}", marker, i, buf.len(), preview)));
                    }
                }
                let height = (lines.len() as u16 + 2).min(15);
                let overlay = Paragraph::new(Text::from(lines)).block(Block::default().borders(Borders::ALL).title("choose-buffer (enter=paste, d=delete, esc=close)"));
                let oa = centered_rect(70, height, area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::WindowIndexPrompt { input } = &app.mode {
                let msg_style = parse_tmux_style(&app.message_command_style);
                let prompt_text = format!("index: {}", input);
                let prompt_area = status_chunk;
                let para = Paragraph::new(prompt_text).style(msg_style);
                f.render_widget(Clear, prompt_area);
                f.render_widget(para, prompt_area);
                let cx = prompt_area.x + 7 + input.len() as u16; // 7 for "index: "
                f.set_cursor_position((cx, prompt_area.y));
            }

            if let Mode::RenamePrompt { input } = &app.mode {
                let overlay = Paragraph::new(format!("rename: {}", input)).block(Block::default().borders(Borders::ALL).title("rename window"));
                let oa = centered_rect(60, 3, area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::RenameSessionPrompt { input } = &app.mode {
                let overlay = Paragraph::new(format!("rename: {}", input)).block(Block::default().borders(Borders::ALL).title("rename session"));
                let oa = centered_rect(60, 3, area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::PaneChooser { .. } = &app.mode {
                let win = &app.windows[app.active_idx];
                let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
                compute_rects(&win.root, app.last_window_area, &mut rects);
                for (i, (_, r)) in rects.iter().enumerate() {
                    if i >= 10 { break; }
                    let disp = (i + app.pane_base_index) % 10;
                    let bw = 7u16;
                    let bh = 3u16;
                    let bx = r.x + r.width.saturating_sub(bw) / 2;
                    let by = r.y + r.height.saturating_sub(bh) / 2;
                    let b = Rect { x: bx, y: by, width: bw, height: bh };
                    let block = Block::default().borders(Borders::ALL).style(Style::default().bg(Color::Yellow).fg(Color::Black));
                    let inner = block.inner(b);
                    let line = Line::from(Span::styled(format!(" {} ", disp), Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)));
                    let para = Paragraph::new(line).alignment(Alignment::Center);
                    f.render_widget(Clear, b);
                    f.render_widget(block, b);
                    f.render_widget(para, inner);
                }
            }

            // Render Menu mode
            if let Mode::MenuMode { menu } = &app.mode {
                let item_count = menu.items.len();
                let height = (item_count as u16 + 2).min(20);
                let width = (menu.items.iter()
                    .map(|i| unicode_width::UnicodeWidthStr::width(i.name.as_str()))
                    .max().unwrap_or(10)
                    .max(unicode_width::UnicodeWidthStr::width(menu.title.as_str())) as u16 + 8)
                    .min(area.width.saturating_sub(2));
                
                // Calculate position based on x/y or center
                let menu_area = if let (Some(x), Some(y)) = (menu.x, menu.y) {
                    let x = if x < 0 { (area.width as i16 + x).max(0) as u16 } else { x as u16 };
                    let y = if y < 0 { (area.height as i16 + y).max(0) as u16 } else { y as u16 };
                    let clamped_x = x.min(area.width.saturating_sub(width));
                    let clamped_w = width.min(area.width.saturating_sub(clamped_x));
                    Rect { x: clamped_x, y: y.min(area.height.saturating_sub(height)), width: clamped_w, height }
                } else {
                    centered_rect((width * 100 / area.width.max(1)).max(30), height, area)
                };
                
                let title = if menu.title.is_empty() { "Menu" } else { &menu.title };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title);
                
                let mut lines: Vec<Line> = Vec::new();
                for (i, item) in menu.items.iter().enumerate() {
                    if item.is_separator {
                        lines.push(Line::from("─".repeat(width.saturating_sub(2) as usize)));
                    } else {
                        let marker = if i == menu.selected { ">" } else { " " };
                        let key_str = item.key.map(|k| format!("({})", k)).unwrap_or_default();
                        let style = if i == menu.selected {
                            Style::default().bg(Color::Blue).fg(Color::White)
                        } else {
                            Style::default()
                        };
                        lines.push(Line::from(Span::styled(
                            format!("{} {} {}", marker, item.name, key_str),
                            style
                        )));
                    }
                }
                
                let para = Paragraph::new(Text::from(lines)).block(block);
                f.render_widget(Clear, menu_area);
                f.render_widget(para, menu_area);
            }

            // Render Popup mode (delegated to popup module)
            crate::popup::render_popup_overlay(f, area, &app);

            // Render Confirm mode
            if let Mode::ConfirmMode { prompt, input, .. } = &app.mode {
                let width = (prompt.len() as u16 + 10).min(80);
                let confirm_area = centered_rect((width * 100 / area.width.max(1)).max(40), 3, area);
                
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title("Confirm");
                
                let text = format!("{} {}", prompt, input);
                let para = Paragraph::new(text).block(block);
                
                f.render_widget(Clear, confirm_area);
                f.render_widget(para, confirm_area);
            }

            // Render Copy-mode search prompt
            if let Mode::CopySearch { input, forward } = &app.mode {
                let dir = if *forward { "/" } else { "?" };
                let width = (input.len() as u16 + 10).min(80).max(30);
                let search_area = Rect {
                    x: area.x,
                    y: area.y + area.height.saturating_sub(2),
                    width: width.min(area.width),
                    height: 1,
                };
                let text = format!("{}{}", dir, input);
                let para = Paragraph::new(text)
                    .style(Style::default().fg(Color::Yellow).bg(Color::Black));
                f.render_widget(para, search_area);
            }
        })?;

        // Forward active pane's cursor shape (DECSCUSR) to the real terminal.
        // Write directly to stdout (not through ratatui backend) to avoid
        // any buffering interference with the next draw cycle.
        //
        // When cursor_shape is CURSOR_SHAPE_UNSET (255), ConPTY consumed the
        // child's DECSCUSR (Windows 10 without passthrough mode).  In that
        // case, fall back to the user-configured cursor-style option so TUI
        // apps like Claude still get a sensible cursor (default: bar).
        //
        // Only send when the effective style changes to avoid resetting
        // Windows Terminal's cursor blink timer every frame.
        {
            let win = &app.windows[app.active_idx];
            if let Some(pane) = crate::tree::active_pane(&win.root, &win.active_path) {
                let shape = pane.cursor_shape.load(std::sync::atomic::Ordering::Relaxed);
                let effective = if shape <= 6 {
                    shape
                } else {
                    crate::rendering::configured_cursor_code()
                };
                if effective != last_cursor_style {
                    last_cursor_style = effective;
                    use crossterm::cursor::SetCursorStyle;
                    let style = match effective {
                        0 => SetCursorStyle::DefaultUserShape,
                        1 => SetCursorStyle::BlinkingBlock,
                        2 => SetCursorStyle::SteadyBlock,
                        3 => SetCursorStyle::BlinkingUnderScore,
                        4 => SetCursorStyle::SteadyUnderScore,
                        5 => SetCursorStyle::BlinkingBar,
                        6 => SetCursorStyle::SteadyBar,
                        _ => SetCursorStyle::DefaultUserShape,
                    };
                    let _ = crossterm::execute!(std::io::stdout(), style);
                }
            }
        }

        // Update Win32 system caret for accessibility / speech-to-text tools.
        // Compute the active pane's screen-global cursor position and mirror
        // it onto the console caret so tools like Wispr Flow can detect the
        // text insertion point.
        {
            let win = &app.windows[app.active_idx];
            if let Some(pane) = crate::tree::active_pane(&win.root, &win.active_path) {
                if let Ok(parser) = pane.term.lock() {
                    if !parser.screen().hide_cursor() {
                        let (cr, cc) = parser.screen().cursor_position();
                        if let Some(inner) = crate::rendering::compute_active_rect_pub(
                            &win.root, &win.active_path, app.last_window_area,
                        ) {
                            let cx = inner.x + cc.min(inner.width.saturating_sub(1));
                            let cy = inner.y + cr.min(inner.height.saturating_sub(1));
                            crate::platform::caret::update(cx, cy);
                        }
                    }
                }
            }
        }

        if let Mode::PaneChooser { opened_at } = &app.mode {
            if opened_at.elapsed() > Duration::from_millis(app.display_panes_time_ms) { app.mode = Mode::Passthrough; }
        }

        // Use a shorter poll timeout when PTY data is pending to keep rendering
        // responsive. When there are no pending control requests and no PTY
        // output, use the full 20ms timeout to reduce CPU usage.
        let has_pty_data = crate::types::PTY_DATA_READY.swap(false, std::sync::atomic::Ordering::AcqRel);
        // Use fast polling when bracket paste detector has buffered an ESC
        // so the timeout flush fires promptly (within ~1-2ms).
        #[cfg(windows)]
        let bp_pending = matches!(bp_state, bracket_paste_detect::State::MatchOpen { .. });
        #[cfg(not(windows))]
        let bp_pending = false;
        let poll_ms = if bp_pending { 1 } else if has_pty_data { 1 } else { 20 };
        if event::poll(Duration::from_millis(poll_ms))? {
            match event::read()? {
                Event::Key(mut key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
                    // On Windows, ConPTY interprets Shift+Enter's ESC+CR as
                    // Alt+Enter.  Poll the physical keyboard to detect the
                    // real modifier — same fix as client.rs.
                    #[cfg(windows)]
                    crate::platform::augment_enter_shift(&mut key);
                    // On Windows, crossterm does not emit Event::Paste — bracket
                    // paste sequences arrive as individual Key events.  Feed each
                    // key through the detector; when a complete paste is found,
                    // deliver it via send_paste_to_active() instead of forwarding
                    // characters one-by-one (which defeats bracket paste mode and
                    // causes compounding indentation in editors like nvim).
                    #[cfg(windows)]
                    {
                        match bracket_paste_detect::feed(&mut bp_state, key) {
                            bracket_paste_detect::Action::Forward(k) => {
                                if handle_key(&mut app, k)? { quit = true; }
                            }
                            bracket_paste_detect::Action::Replay(pending, current) => {
                                for pk in pending {
                                    if handle_key(&mut app, pk)? { quit = true; break; }
                                }
                                if !quit {
                                    if handle_key(&mut app, current)? { quit = true; }
                                }
                            }
                            bracket_paste_detect::Action::Consumed => {}
                            bracket_paste_detect::Action::Paste(text) => {
                                crate::debug_log::input_log("paste", &format!(
                                    "bracket_paste_detect: captured paste len={} preview={:?}",
                                    text.len(), &text.chars().take(100).collect::<String>()));
                                send_paste_to_active(&mut app, &text)?;
                            }
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        if handle_key(&mut app, key)? {
                            quit = true;
                        }
                    }
                }
                Event::Mouse(me) => {
                    if app.mouse_enabled {
                        let area = app.last_window_area;
                        handle_mouse(&mut app, me, area)?;
                    }
                }
                Event::Resize(cols, rows) => {
                    if last_resize.elapsed() > Duration::from_millis(50) {
                        let win = &mut app.windows[app.active_idx];
                        if let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) {
                            let _ = pane.master.resize(PtySize { rows: rows as u16, cols: cols as u16, pixel_width: 0, pixel_height: 0 });
                            if let Ok(mut parser) = pane.term.lock() {
                                parser.screen_mut().set_size(rows, cols);
                            }
                        }
                        last_resize = Instant::now();
                    }
                }
                Event::Paste(text) => {
                    crate::debug_log::input_log("paste", &format!("Event::Paste received, len={} text={:?}", text.len(), &text.chars().take(200).collect::<String>()));
                    send_paste_to_active(&mut app, &text)?;
                }
                _ => {}
            }
        }

        // Flush bracket paste detector if ESC was buffered but no follow-up
        // key arrived within the timeout (5ms).  Without this, pressing
        // Ctrl+[ (ESC) would be permanently stuck in the detector when no
        // subsequent key is pressed — breaking nvim's insert→normal switch.
        #[cfg(windows)]
        {
            match bracket_paste_detect::flush_timeout(&mut bp_state) {
                bracket_paste_detect::TimeoutAction::Replay(pending) => {
                    for pk in pending {
                        if handle_key(&mut app, pk)? { quit = true; break; }
                    }
                }
                bracket_paste_detect::TimeoutAction::None => {}
            }
        }

        loop {
            let req = if let Some(rx) = app.control_rx.as_ref() { rx.try_recv().ok() } else { None };
            let Some(req) = req else { break; };
            match req {
                CtrlReq::NewWindow(cmd, name, _detached, start_dir) => {
                    create_window(&*pty_system, &mut app, cmd.as_deref(), start_dir.as_deref())?;
                    if let Some(n) = name { app.windows.last_mut().map(|w| w.name = n); }
                    resize_all_panes(&mut app);
                }
                CtrlReq::SplitWindow(k, cmd, _detached, start_dir, _size_pct, resp) => { let _ = resp.send(if let Err(e) = split_active_with_command(&mut app, k, cmd.as_deref(), Some(&*pty_system), start_dir.as_deref()) { format!("{e}") } else { String::new() }); resize_all_panes(&mut app); }
                CtrlReq::KillPane => { let _ = kill_active_pane(&mut app); resize_all_panes(&mut app); }
                CtrlReq::KillPaneById(pid) => { let _ = kill_pane_by_id(&mut app, pid); resize_all_panes(&mut app); }
                CtrlReq::CapturePane(resp) => {
                    if let Some(text) = capture_active_pane_text(&mut app)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneStyled(resp, s, e) => {
                    if let Some(text) = capture_active_pane_styled(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneRange(resp, s, e) => {
                    if let Some(text) = capture_active_pane_range(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::FocusWindow(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::FocusWindowTemp(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::FocusPane(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::FocusPaneByIndex(idx) => { focus_pane_by_index(&mut app, idx); }
                CtrlReq::FocusPaneTemp(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::FocusPaneByIndexTemp(idx) => { focus_pane_by_index(&mut app, idx); }
                CtrlReq::SessionInfo(resp) => {
                    let attached = if app.attached_clients > 0 { "(attached)" } else { "(detached)" };
                    let windows = app.windows.len();
                    let (w,h) = {
                        let win = &mut app.windows[app.active_idx];
                        let mut size = (0,0);
                        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { size = (p.last_cols as i32, p.last_rows as i32); }
                        size
                    };
                    let created = app.created_at.format("%a %b %e %H:%M:%S %Y");
                    let line = format!("{}: {} windows (created {}) [{}x{}] {}\n", app.session_name, windows, created, w, h, attached);
                    let _ = resp.send(line);
                }
                CtrlReq::ClientAttach(_cid) => { app.attached_clients = app.attached_clients.saturating_add(1); }
                CtrlReq::ClientDetach(_cid) => { app.attached_clients = app.attached_clients.saturating_sub(1); }
                CtrlReq::DumpLayout(resp) => {
                    let json = dump_layout_json(&mut app)?;
                    let _ = resp.send(json);
                }
                CtrlReq::SendText(s) => { send_text_to_active(&mut app, &s)?; }
                CtrlReq::SendKey(k) => { send_key_to_active(&mut app, &k)?; }
                CtrlReq::SendPaste(s) => { send_paste_to_active(&mut app, &s)?; }
                CtrlReq::ZoomPane => { toggle_zoom(&mut app); }
                CtrlReq::PrefixBegin => { app.client_prefix_active = true; }
                CtrlReq::PrefixEnd => { app.client_prefix_active = false; }
                CtrlReq::CopyEnter => { enter_copy_mode(&mut app); }
                CtrlReq::CopyMove(dx, dy) => { move_copy_cursor(&mut app, dx, dy); }
                CtrlReq::CopyAnchor => { if let Some((r,c)) = current_prompt_pos(&mut app) { app.copy_anchor = Some((r,c)); app.copy_pos = Some((r,c)); } }
                CtrlReq::CopyYank => { let _ = yank_selection(&mut app); app.mode = Mode::Passthrough; }
                CtrlReq::CopyRectToggle => {
                    app.copy_selection_mode = match app.copy_selection_mode {
                        crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                        _ => crate::types::SelectionMode::Rect,
                    };
                }
                CtrlReq::ClientSize(_cid, w, h) => { 
                    app.last_window_area = Rect { x: 0, y: 0, width: w, height: h }; 
                    resize_all_panes(&mut app);
                }
                CtrlReq::FocusPaneCmd(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::FocusWindowCmd(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::MouseDown(_,x,y) => { remote_mouse_down(&mut app, x, y); }
                CtrlReq::MouseDownRight(_,x,y) => { remote_mouse_button(&mut app, x, y, 2, true); }
                CtrlReq::MouseDownMiddle(_,x,y) => { remote_mouse_button(&mut app, x, y, 1, true); }
                CtrlReq::MouseDrag(_,x,y) => { remote_mouse_drag(&mut app, x, y); }
                CtrlReq::MouseUp(_,x,y) => { remote_mouse_up(&mut app, x, y); }
                CtrlReq::MouseUpRight(_,x,y) => { remote_mouse_button(&mut app, x, y, 2, false); }
                CtrlReq::MouseUpMiddle(_,x,y) => { remote_mouse_button(&mut app, x, y, 1, false); }
                CtrlReq::MouseMove(_,x,y) => { remote_mouse_motion(&mut app, x, y); }
                CtrlReq::ScrollUp(_, x, y) => { remote_scroll_up(&mut app, x, y); }
                CtrlReq::ScrollDown(_, x, y) => { remote_scroll_down(&mut app, x, y); }
                CtrlReq::NextWindow => { if !app.windows.is_empty() { app.active_idx = (app.active_idx + 1) % app.windows.len(); } }
                CtrlReq::PrevWindow => { if !app.windows.is_empty() { app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); } }
                CtrlReq::RenameWindow(name) => { let win = &mut app.windows[app.active_idx]; win.name = name; }
                CtrlReq::ListWindows(resp) => { let json = list_windows_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ListWindowsTmux(resp) => { let text = list_windows_tmux(&app); let _ = resp.send(text); }
                CtrlReq::ListTree(resp) => { let json = list_tree_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ListPanes(resp) => {
                    let win = &app.windows[app.active_idx];
                    fn lp_collect(node: &crate::types::Node, panes: &mut Vec<(usize, u16, u16)>) {
                        match node {
                            crate::types::Node::Leaf(p) => { panes.push((p.id, p.last_cols, p.last_rows)); }
                            crate::types::Node::Split { children, .. } => { for c in children { lp_collect(c, panes); } }
                        }
                    }
                    let mut panes = Vec::new();
                    lp_collect(&win.root, &mut panes);
                    let active_id = crate::tree::get_active_pane_id(&win.root, &win.active_path);
                    let mut output = String::new();
                    for (pos, (id, cols, rows)) in panes.iter().enumerate() {
                        let idx = pos + app.pane_base_index;
                        let marker = if active_id == Some(*id) { " (active)" } else { "" };
                        output.push_str(&format!("{}: [{}x{}] [history {}/{}, 0 bytes] %{}{}\n",
                            idx, cols, rows, app.history_limit, app.history_limit, id, marker));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListAllPanes(resp) => {
                    fn lap_collect(node: &crate::types::Node, panes: &mut Vec<(usize, u16, u16)>) {
                        match node {
                            crate::types::Node::Leaf(p) => { panes.push((p.id, p.last_cols, p.last_rows)); }
                            crate::types::Node::Split { children, .. } => { for c in children { lap_collect(c, panes); } }
                        }
                    }
                    let mut output = String::new();
                    for (wi, win) in app.windows.iter().enumerate() {
                        let mut panes = Vec::new();
                        lap_collect(&win.root, &mut panes);
                        let active_id = crate::tree::get_active_pane_id(&win.root, &win.active_path);
                        for (pos, (id, cols, rows)) in panes.iter().enumerate() {
                            let idx = pos + app.pane_base_index;
                            let marker = if active_id == Some(*id) && wi == app.active_idx { " (active)" } else { "" };
                            output.push_str(&format!("{}:{}: [{}x{}] %{}{}\n",
                                wi + app.window_base_index, idx, cols, rows, id, marker));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListClients(resp) => {
                    let output = format!("/dev/pts/0: {}: {} [{}x{}] (utf8)\n",
                        app.session_name,
                        app.windows[app.active_idx].name,
                        app.last_window_area.width,
                        app.last_window_area.height);
                    let _ = resp.send(output);
                }
                CtrlReq::ShowHooks(resp) => {
                    let mut output = String::new();
                    for (name, commands) in &app.hooks {
                        if commands.len() == 1 {
                            output.push_str(&format!("{} -> {}\n", name, commands[0]));
                        } else {
                            for (i, cmd) in commands.iter().enumerate() {
                                output.push_str(&format!("{}[{}] -> {}\n", name, i, cmd));
                            }
                        }
                    }
                    if output.is_empty() {
                        output.push_str("(no hooks)\n");
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ListCommands(resp) => {
                    let cmds = crate::help::cli_command_lines().join("\n");
                    let _ = resp.send(cmds);
                }
                CtrlReq::ToggleSync => { app.sync_input = !app.sync_input; }
                CtrlReq::SetPaneTitle(title) => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                        p.title_locked = !title.is_empty();
                        p.title = title;
                    }
                }
                CtrlReq::KillServer | CtrlReq::KillSession => {
                    // Kill all child processes and exit
                    for win in app.windows.iter_mut() {
                        kill_all_children(&mut win.root);
                    }
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}/.psmux/{}.port", home, app.port_file_base());
                    let keypath = format!("{}/.psmux/{}.key", home, app.port_file_base());
                    let _ = std::fs::remove_file(&regpath);
                    let _ = std::fs::remove_file(&keypath);
                    std::process::exit(0);
                }
                CtrlReq::SourceFile(path) => {
                    crate::config::source_file(&mut app, &path);
                }
                // For attach mode, we just ignore the new commands - they're handled by the server
                _ => {}
            }
        }

        // Throttle reap_children to ~500ms to avoid O(N_panes) try_wait()
        // syscalls on every 20ms frame. With hundreds of panes this saves
        // significant CPU and reduces event-loop latency for command processing.
        if last_reap.elapsed() > Duration::from_millis(500) {
            last_reap = Instant::now();
            let (all_empty, any_pruned) = reap_children(&mut app)?;
            if any_pruned {
                resize_all_panes(&mut app);
            }
            if all_empty {
                quit = true;
            }
        }

        if quit { break; }
    }
    // teardown: kill all pane children
    for win in app.windows.iter_mut() {
        kill_all_children(&mut win.root);
    }
    Ok(())
}
