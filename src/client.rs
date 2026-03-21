use std::io::{self, Write, BufRead, BufReader};
use std::time::{Duration, Instant};
use std::env;

use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyModifiers, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::layout::LayoutJson;
use crate::help;
use crate::util::{WinTree, base64_encode, quote_arg};
use crate::session::read_session_key;
use crate::rendering::{dim_predictions_enabled, map_color, dim_color, centered_rect, fix_border_intersections};
use crate::style::parse_tmux_style_components;
use crate::config::{parse_key_string, normalize_key_for_binding};
use crate::copy_mode::{copy_to_system_clipboard, read_from_system_clipboard};
use crate::debug_log::{client_log, client_log_enabled, input_log, input_log_enabled};
use crate::layout::RowRunsJson;
use crate::tree::split_with_gaps;

/// Build a send-key name with modifier prefix (e.g. "C-Left", "S-Right", "C-S-Up").
fn modified_key_name(base: &str, mods: KeyModifiers) -> String {
    let mut prefix = String::new();
    if mods.contains(KeyModifiers::CONTROL) { prefix.push_str("C-"); }
    if mods.contains(KeyModifiers::ALT) { prefix.push_str("M-"); }
    if mods.contains(KeyModifiers::SHIFT) { prefix.push_str("S-"); }
    if prefix.is_empty() {
        base.to_lowercase()
    } else {
        format!("{}{}", prefix, base)
    }
}

/// Extract selected text from the layout tree given absolute terminal coordinates.
/// Computes pane areas via the same Layout splitting render_json uses, then reads
/// characters from the run-length-encoded rows_v2 data.
fn extract_selection_text(
    layout: &LayoutJson,
    term_width: u16,
    content_height: u16, // excluding status bar
    start: (u16, u16),   // (col, row)
    end: (u16, u16),
) -> String {
    // Normalise so (r0,c0) <= (r1,c1) in reading order
    let (r0, c0, r1, c1) = if (start.1, start.0) <= (end.1, end.0) {
        (start.1, start.0, end.1, end.0)
    } else {
        (end.1, end.0, start.1, start.0)
    };

    // Collect leaf panes with their inner areas and content
    struct PaneLeaf<'a> {
        inner: Rect,
        rows_v2: &'a [RowRunsJson],
    }

    fn collect_leaves<'a>(node: &'a LayoutJson, area: Rect, out: &mut Vec<PaneLeaf<'a>>) {
        match node {
            LayoutJson::Leaf { rows_v2, .. } => {
                // No borders — content fills entire area (tmux-style)
                out.push(PaneLeaf { inner: area, rows_v2 });
            }
            LayoutJson::Split { kind, sizes, children } => {
                let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                    sizes.clone()
                } else {
                    vec![(100 / children.len().max(1)) as u16; children.len()]
                };
                let is_horizontal = kind == "Horizontal";
                let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
                for (i, child) in children.iter().enumerate() {
                    if i < rects.len() {
                        collect_leaves(child, rects[i], out);
                    }
                }
            }
        }
    }

    let content_area = Rect { x: 0, y: 0, width: term_width, height: content_height };
    let mut leaves: Vec<PaneLeaf> = Vec::new();
    collect_leaves(layout, content_area, &mut leaves);

    // Helper: get character at a local column position within a row's runs
    fn char_at_col(runs: &[crate::layout::CellRunJson], local_col: usize) -> char {
        let mut cursor = 0usize;
        for run in runs {
            let run_width = run.width.max(1) as usize;
            if local_col >= cursor && local_col < cursor + run_width {
                let offset = local_col - cursor;
                // Run text may be shorter than run_width (e.g. single char repeated)
                // or multi-char for wide chars. Pick the nth char if available.
                return run.text.chars().nth(offset).unwrap_or(' ');
            }
            cursor += run_width;
        }
        ' '
    }

    let mut result = String::new();
    for row in r0..=r1 {
        let col_start = if row == r0 { c0 } else { 0 };
        let col_end = if row == r1 { c1 } else { term_width.saturating_sub(1) };

        let mut line = String::new();
        for col in col_start..=col_end {
            let mut ch = ' ';
            for leaf in &leaves {
                let inner = &leaf.inner;
                if col >= inner.x && col < inner.x + inner.width
                    && row >= inner.y && row < inner.y + inner.height
                {
                    let local_row = (row - inner.y) as usize;
                    let local_col = (col - inner.x) as usize;
                    if local_row < leaf.rows_v2.len() {
                        ch = char_at_col(&leaf.rows_v2[local_row].runs, local_col);
                    }
                    break;
                }
            }
            line.push(ch);
        }
        // Trim trailing whitespace per line
        let trimmed = line.trim_end();
        result.push_str(trimmed);
        if row < r1 {
            result.push('\n');
        }
    }

    result
}

/// Check if the active pane is running a fullscreen TUI app (alternate screen).
/// Used to decide whether right-click should paste (shell prompt) or forward
/// as a mouse event to the child (TUI app like htop, Claude Code, etc.).
fn active_pane_in_alt_screen(layout: &LayoutJson) -> bool {
    match layout {
        LayoutJson::Leaf { active, alternate_screen, .. } => *active && *alternate_screen,
        LayoutJson::Split { children, .. } => children.iter().any(|c| active_pane_in_alt_screen(c)),
    }
}

/// Check if the active pane is in server-side copy mode.
/// When true, the client should NOT start its own text selection —
/// the server handles cursor positioning and selection in copy mode.
fn active_pane_in_copy_mode(layout: &LayoutJson) -> bool {
    match layout {
        LayoutJson::Leaf { active, copy_mode, .. } => *active && *copy_mode,
        LayoutJson::Split { children, .. } => children.iter().any(|c| active_pane_in_copy_mode(c)),
    }
}

/// Check if screen coordinates (x, y) fall on a separator line in the layout.
/// Used to distinguish border-drag (resize) from text selection on left-click.
fn is_on_separator(layout: &LayoutJson, area: Rect, x: u16, y: u16) -> bool {
    match layout {
        LayoutJson::Leaf { .. } => false,
        LayoutJson::Split { kind, sizes, children } => {
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![(100 / children.len().max(1)) as u16; children.len()]
            };
            let is_horizontal = kind == "Horizontal";
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);

            // Check if (x, y) is on any separator between children
            for i in 0..children.len().saturating_sub(1) {
                if i >= rects.len() { break; }
                if is_horizontal {
                    let sep_x = rects[i].x + rects[i].width;
                    if x == sep_x && y >= area.y && y < area.y + area.height {
                        return true;
                    }
                } else {
                    let sep_y = rects[i].y + rects[i].height;
                    if y == sep_y && x >= area.x && x < area.x + area.width {
                        return true;
                    }
                }
            }

            // Recurse into children
            for (i, child) in children.iter().enumerate() {
                if i < rects.len() && is_on_separator(child, rects[i], x, y) {
                    return true;
                }
            }

            false
        }
    }
}

/// Check if any leaf in a LayoutJson subtree is the active pane.
/// Compute the rectangle of the active pane by searching the LayoutJson tree.
fn compute_active_rect_json(node: &LayoutJson, area: Rect) -> Option<Rect> {
    match node {
        LayoutJson::Leaf { active, .. } => {
            if *active { Some(area) } else { None }
        }
        LayoutJson::Split { kind, sizes, children } => {
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![(100 / children.len().max(1)) as u16; children.len()]
            };
            let is_horizontal = kind == "Horizontal";
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
            for (i, child) in children.iter().enumerate() {
                if i < rects.len() {
                    if let Some(r) = compute_active_rect_json(child, rects[i]) {
                        return Some(r);
                    }
                }
            }
            None
        }
    }
}

pub fn run_remote(terminal: &mut Terminal<CrosstermBackend<crate::platform::PsmuxWriter>>, input: &crate::ssh_input::InputSource) -> io::Result<()> {
    let name = env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| "default".to_string());
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let path = format!("{}\\.psmux\\{}.port", home, name);
    let port = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, format!("can't find session '{}' (no server running)", name)))?;
    let addr = format!("127.0.0.1:{}", port);
    let session_key = read_session_key(&name).unwrap_or_default();
    let last_path = format!("{}\\.psmux\\last_session", home);
    if !crate::session::is_warm_session(&name) {
        let _ = std::fs::write(&last_path, &name);
    }

    // ── Open persistent TCP connection ───────────────────────────────────
    let stream = std::net::TcpStream::connect(&addr)?;
    stream.set_nodelay(true)?; // Disable Nagle's algorithm for low latency
    let mut writer = stream.try_clone()?;
    writer.set_nodelay(true)?;
    let mut reader = BufReader::new(stream);

    // AUTH handshake
    let _ = writer.write_all(format!("AUTH {}\n", session_key).as_bytes());
    let _ = writer.flush();
    let mut auth_line = String::new();
    reader.read_line(&mut auth_line)?;
    if !auth_line.trim().starts_with("OK") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "auth failed"));
    }

    // Enter persistent mode + attach
    let _ = writer.write_all(b"PERSISTENT\n");
    let _ = writer.write_all(b"client-attach\n");
    let _ = writer.flush();

    // Spawn a dedicated reader thread so the event loop never blocks on I/O.
    // The reader thread reads lines from the server and sends them via channel.
    // Use a 2-second read timeout so the thread unblocks periodically.
    // Without this, process::exit(0) on the server side may not deliver a
    // TCP RST promptly on Windows, leaving read_line() blocked forever and
    // the client stuck after the last pane exits.
    let _ = reader.get_ref().set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = String::with_capacity(64 * 1024);
        loop {
            buf.clear();
            loop {
                match reader.read_line(&mut buf) {
                    Ok(0) => return, // EOF — server closed connection
                    Ok(_) => break,  // Got a complete line, send it
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // Timeout: buf may contain a partial line from a
                        // previous fill_buf.  Do NOT clear it — read_line
                        // will resume appending on the next call.  This
                        // keeps the protocol stream intact.
                        continue;
                    }
                    Err(_) => return, // Real error — connection died
                }
            }
            let line = std::mem::take(&mut buf);
            buf = String::with_capacity(64 * 1024);
            if frame_tx.send(line).is_err() { return; }
        }
    });

    let mut quit = false;
    let mut prefix_armed = false;
    let mut prefix_armed_at = Instant::now();
    let mut prefix_repeating = false;
    let mut repeat_time_ms: u64 = 500;
    let mut renaming = false;
    let mut session_renaming = false;
    let mut rename_buf = String::new();
    let mut pane_renaming = false;
    let mut pane_title_buf = String::new();
    let mut command_input = false;
    let mut command_buf = String::new();

    let mut tree_chooser = false;
    let mut tree_entries: Vec<(bool, usize, usize, String, String)> = Vec::new();  // (is_win, id, sub_id, label, session_name)
    let mut tree_selected: usize = 0;
    let mut tree_scroll: usize = 0;
    let mut session_chooser = false;
    let mut session_entries: Vec<(String, String)> = Vec::new();
    let mut session_selected: usize = 0;
    let mut confirm_cmd: Option<String> = None;  // pending kill confirmation
    let current_session = name.clone();
    let mut last_sent_size: (u16, u16) = (0, 0);
    let mut last_status_lines: u16 = 1; // track server's status_lines for correct client-size height
    let mut last_dump_time = Instant::now() - Duration::from_millis(250);
    let mut force_dump = true;
    let mut last_tree: Vec<WinTree> = Vec::new();
    // Default prefix is Ctrl+B, updated dynamically from server config
    let mut prefix_key: (KeyCode, KeyModifiers) = (KeyCode::Char('b'), KeyModifiers::CONTROL);
    // Precompute the raw control character for the default prefix
    let mut prefix_raw_char: Option<char> = Some('\x02');
    // Secondary prefix key (prefix2), default None
    let mut prefix2_key: Option<(KeyCode, KeyModifiers)> = None;
    let mut prefix2_raw_char: Option<char> = None;
    // Status bar style from server (parsed from tmux status-style format)
    let mut status_fg: Color = Color::Black;
    let mut status_bg: Color = Color::Green;
    let mut status_bold: bool = false;
    let mut custom_status_left: Option<String> = None;
    let mut custom_status_right: Option<String> = None;
    let mut pane_border_fg: Color = Color::DarkGray;
    let mut pane_active_border_fg: Color = Color::Green;
    let mut win_status_fmt: String = "#I:#W#{?window_flags,#{window_flags}, }".to_string();
    let mut win_status_current_fmt: String = "#I:#W#{?window_flags,#{window_flags}, }".to_string();
    let mut win_status_sep: String = " ".to_string();
    let mut win_status_style: Option<(Option<Color>, Option<Color>, bool)> = None;
    let mut win_status_current_style: Option<(Option<Color>, Option<Color>, bool)> = None;
    let mut mode_style_str: String = "bg=yellow,fg=black".to_string();
    let mut status_position_str: String = "bottom".to_string();
    let mut status_justify_str: String = "left".to_string();
    // Synced bindings from server (updated each frame from DumpState)
    let mut synced_bindings: Vec<BindingEntry> = Vec::new();

    // ── Windows paste detection state ──────────────────────────────────
    // On Windows, Ctrl+V paste injects individual Key events BEFORE the
    // Ctrl+V Release event arrives (~184ms later).  We buffer ALL printable
    // chars for a short 20ms window.  If ≥3 chars arrive within 20ms, it's
    // almost certainly a paste — hold the buffer until Ctrl+V Release confirms
    // (up to 300ms), then send as a single bracketed paste (send-paste).
    // If <3 chars arrive within 20ms, flush them as normal send-text.
    // Pending chars being examined for paste detection.
    #[cfg(windows)]
    let mut paste_pend: String = String::new();
    // When the first char of the current pending group arrived.
    #[cfg(windows)]
    let mut paste_pend_start: Option<Instant> = None;
    // True once the 20ms window showed ≥3 chars — waiting for Ctrl+V Release.
    #[cfg(windows)]
    let mut paste_stage2: bool = false;
    // Set to true when Ctrl+V Release is seen — confirms the burst was a paste.
    #[cfg(windows)]
    let mut paste_confirmed: bool = false;
    // Buffer size at previous stage2 timeout check — for growth detection.
    #[cfg(windows)]
    let mut paste_stage2_last_len: usize = 0;
    // Suppression window: after right-click copy, discard text key events
    // for a short period to prevent VS Code ConPTY duplicate injection.
    #[cfg(windows)]
    let mut paste_suppress_until: Option<Instant> = None;

    // list-keys overlay state (C-b ?)
    let mut keys_viewer = false;
    let mut keys_viewer_lines: Vec<String> = Vec::new();
    let mut keys_viewer_scroll: usize = 0;

    // ── Server-side overlay state (updated each frame) ──
    // Initial values are overwritten on the first render frame; defaults
    // are kept here for safety in case the first state message is delayed.
    #[allow(unused_assignments)]
    let mut srv_popup_active = false;
    #[allow(unused_assignments)]
    let mut srv_popup_command = String::new();
    #[allow(unused_assignments)]
    let mut srv_popup_width: u16 = 80;
    #[allow(unused_assignments)]
    let mut srv_popup_height: u16 = 24;
    #[allow(unused_assignments)]
    let mut srv_popup_lines: Vec<String> = Vec::new();
    #[allow(unused_assignments)]
    let mut srv_confirm_active = false;
    #[allow(unused_assignments)]
    let mut srv_confirm_prompt = String::new();
    #[allow(unused_assignments)]
    let mut srv_menu_active = false;
    #[allow(unused_assignments)]
    let mut srv_menu_title = String::new();
    #[allow(unused_assignments)]
    let mut srv_menu_selected: usize = 0;
    #[allow(unused_assignments)]
    let mut srv_menu_items: Vec<ServerMenuItem> = Vec::new();
    #[allow(unused_assignments)]
    let mut srv_display_panes = false;
    #[allow(unused_assignments)]
    let mut srv_pane_base_index: usize = 0;
    #[allow(unused_assignments)]
    let mut clock_active = false;

    #[derive(serde::Deserialize, Default)]
    struct WinStatus { id: usize, name: String, active: bool, #[serde(default)] activity: bool, #[serde(default)] tab_text: String }
    
    fn default_base_index() -> usize { 1 }
    fn default_prediction_dimming() -> bool { dim_predictions_enabled() }
    fn default_status_left_length() -> usize { 10 }
    fn default_status_right_length() -> usize { 40 }
    fn default_status_lines() -> usize { 1 }
    fn default_status_visible() -> bool { true }
    fn default_repeat_time() -> u64 { 500 }

    /// A single key binding synced from the server.
    #[derive(serde::Deserialize, Clone, Debug)]
    struct BindingEntry {
        /// Key table name (e.g. "prefix", "root")
        t: String,
        /// Key string (e.g. "C-a", "-", "F12")
        k: String,
        /// Command string (e.g. "split-window -v")
        c: String,
        /// Whether the binding is repeatable
        #[serde(default)]
        r: bool,
    }

    /// A menu item from server-side MenuMode
    #[derive(serde::Deserialize, Clone, Debug, Default)]
    struct ServerMenuItem {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        sep: bool,
    }

    #[derive(serde::Deserialize)]
    struct DumpState {
        layout: LayoutJson,
        windows: Vec<WinStatus>,
        #[serde(default)]
        prefix: Option<String>,
        #[serde(default)]
        prefix2: Option<String>,
        #[serde(default)]
        tree: Vec<WinTree>,
        #[serde(default = "default_base_index")]
        base_index: usize,
        #[serde(default = "default_prediction_dimming")]
        prediction_dimming: bool,
        #[serde(default)]
        status_style: Option<String>,
        #[serde(default)]
        status_left: Option<String>,
        #[serde(default)]
        status_right: Option<String>,
        #[serde(default)]
        pane_border_style: Option<String>,
        #[serde(default)]
        pane_active_border_style: Option<String>,
        /// window-status-format (short key to save bandwidth)
        #[serde(default)]
        wsf: Option<String>,
        /// window-status-current-format
        #[serde(default)]
        wscf: Option<String>,
        /// window-status-separator
        #[serde(default)]
        wss: Option<String>,
        /// window-status-style
        #[serde(default)]
        ws_style: Option<String>,
        /// window-status-current-style
        #[serde(default)]
        wsc_style: Option<String>,
        /// clock-mode active
        #[serde(default)]
        clock_mode: bool,
        /// Dynamic key bindings from server
        #[serde(default)]
        bindings: Vec<BindingEntry>,
        /// status-left-length (max display width for left status)
        #[serde(default = "default_status_left_length")]
        status_left_length: usize,
        /// status-right-length (max display width for right status)
        #[serde(default = "default_status_right_length")]
        status_right_length: usize,
        /// Number of status bar lines
        #[serde(default = "default_status_lines")]
        status_lines: usize,
        /// Custom format strings for additional status lines
        #[serde(default)]
        status_format: Vec<String>,
        /// mode-style for copy mode selection highlighting
        #[serde(default)]
        mode_style: Option<String>,
        /// status-position: "top" or "bottom"
        #[serde(default)]
        status_position: Option<String>,
        /// status-justify: "left", "centre", or "right"
        #[serde(default)]
        status_justify: Option<String>,
        /// Whether the status bar is visible (true) or hidden (false).
        /// Corresponds to `set-option status on/off`.
        #[serde(default = "default_status_visible")]
        status_visible: bool,
        /// Configured cursor style as DECSCUSR code (0-6) from server.
        /// Used as fallback when no child process has set a cursor shape.
        #[serde(default)]
        cursor_style_code: Option<u8>,
        /// One-shot clipboard text (base64-encoded) for OSC 52 delivery.
        #[serde(default)]
        clipboard_osc52: Option<String>,
        /// Repeat key timeout in ms (default: 500, synced from server)
        #[serde(default = "default_repeat_time")]
        repeat_time: u64,
        /// Whether a pane is currently zoomed (borders should be hidden)
        #[serde(default)]
        zoomed: bool,
        // ── Server-side overlay state ──
        /// Popup overlay active
        #[serde(default)]
        popup_active: bool,
        #[serde(default)]
        popup_command: Option<String>,
        #[serde(default)]
        popup_width: Option<u16>,
        #[serde(default)]
        popup_height: Option<u16>,
        #[serde(default)]
        popup_lines: Vec<String>,
        /// Confirm overlay active
        #[serde(default)]
        confirm_active: bool,
        #[serde(default)]
        confirm_prompt: Option<String>,
        /// Menu overlay active
        #[serde(default)]
        menu_active: bool,
        #[serde(default)]
        menu_title: Option<String>,
        #[serde(default)]
        menu_selected: usize,
        #[serde(default)]
        menu_items: Vec<ServerMenuItem>,
        /// Display-panes overlay active
        #[serde(default)]
        display_panes: bool,
        /// Pane base index for display-panes numbering
        #[serde(default)]
        pane_base_index: usize,
        /// Status bar message from display-message (without -p)
        #[serde(default)]
        status_message: Option<String>,
    }

    let mut cmd_batch: Vec<String> = Vec::new();
    let mut dump_buf = String::new();
    let mut prev_dump_buf = String::new();
    let mut last_key_send_time: Option<Instant> = None;
    let mut dump_in_flight = false;
    let mut dump_flight_start: Instant = Instant::now();

    // Diagnostic latency log: set PSMUX_LATENCY_LOG=1 to enable
    let latency_log_enabled = env::var("PSMUX_LATENCY_LOG").unwrap_or_default() == "1";
    let mut latency_log: Option<std::fs::File> = if latency_log_enabled {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        let path = format!("{}\\.psmux\\latency.log", home);
        std::fs::File::create(&path).ok()
    } else { None };
    let mut loop_count: u64 = 0;
    let mut _last_key_char: Option<char> = None;
    let mut key_send_instant: Option<Instant> = None; // when the key was SENT to server

    // Text selection state (client-side only, left-click drag like pwsh)
    let mut rsel_start: Option<(u16, u16)> = None;  // (col, row) in terminal coords
    let mut rsel_end: Option<(u16, u16)> = None;
    let mut rsel_dragged = false;
    let mut selection_changed = false; // forces redraw for selection overlay
    let mut border_drag = false; // true when dragging a pane separator (resize)
    // Buffered OSC 52 clipboard text — written AFTER terminal.draw() to
    // avoid corrupting ratatui's output buffer.
    let mut pending_osc52: Option<String> = None;
    // VT input mode: periodically re-send mouse-enable escape sequences.
    // Covers SSH sessions and JetBrains JediTerm (which sends VT mouse
    // sequences through ConPTY instead of native MOUSE_EVENT records).
    let is_ssh_mode = crate::ssh_input::needs_vt_input();
    let mut last_mouse_enable = Instant::now();
    // ── Cursor blink stabilisation ──────────────────────────────────
    // Cache the last-sent DECSCUSR code so we only write it when it
    // actually changes (avoids resetting WT's blink timer every frame).
    let mut last_cursor_style: u8 = 255;
    loop {
        // Expire stale key_send_instant after 30ms — ConPTY echo should
        // have arrived by then; stop force-dumping to save CPU.
        if let Some(ks) = key_send_instant {
            if ks.elapsed().as_millis() > 30 { key_send_instant = None; }
        }
        // Safety valve: if dump_in_flight is stuck for >500ms (e.g. server
        // did not respond), release it so the client doesn't spin at 1ms.
        if dump_in_flight && dump_flight_start.elapsed().as_millis() > 500 {
            dump_in_flight = false;
        }
        // ── STEP 0: Receive latest frame from reader thread (non-blocking) ──
        // Drain channel, keeping only the most recent frame.
        let mut got_frame = false;
        let mut _nc_count = 0u32;
        loop {
            match frame_rx.try_recv() {
                Ok(line) => {
                    if line.trim() == "NC" {
                        _nc_count += 1;
                        // Server says nothing changed — release dump_in_flight
                        // without touching dump_buf (saves 50-100KB clone + parse).
                        dump_in_flight = false;
                        last_dump_time = Instant::now();
                        // If we're waiting for a key echo, force an
                        // immediate dump-state re-request (~1ms TCP RTT)
                        // instead of waiting the full 10ms typing interval.
                        if key_send_instant.is_some() {
                            force_dump = true;
                        }
                    } else {
                        if client_log_enabled() {
                            client_log("frame", &format!("received {} bytes", line.len()));
                        }
                        dump_buf = line; got_frame = true; dump_in_flight = false;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { quit = true; break; }
            }
        }
        if quit && !got_frame { break; }

        // ── STEP 1: Poll events with adaptive timeout ────────────────────
        let since_dump = last_dump_time.elapsed().as_millis() as u64;
        // Expire typing timer after 100ms of no new keys
        if let Some(kt) = last_key_send_time {
            if kt.elapsed().as_millis() > 100 { last_key_send_time = None; }
        }
        let typing_active = last_key_send_time.is_some();
        // When typing: cap at ~100fps to avoid flooding the server with
        // dump-state requests (each one is ~50-100KB of JSON over TCP).
        // When idle: 50ms refresh (20fps) saves CPU.
        // Use fast poll when paste chars are pending (need timely detection)
        #[cfg(windows)]
        let paste_pend_active = !paste_pend.is_empty();
        #[cfg(not(windows))]
        let paste_pend_active = false;

        let poll_ms = if paste_pend_active { 1 }
            else if got_frame { 0 }
            else if dump_in_flight { 5 }
            else if force_dump { 0 }
            else if typing_active {
                // Rate-limit to ~100fps (10ms) when typing.  The snapshot-
                // based serialisation in dump_layout_json_fast now holds
                // the parser mutex for only ~1ms (cell snapshot), so
                // polling at 10ms no longer starves the ConPTY reader
                // thread.  10ms is notably shorter than ConPTY's ~16ms
                // render interval, avoiding systematic alignment delays.
                let remaining = 10u64.saturating_sub(since_dump);
                remaining
            }
            else {
                // Server pushes frames proactively via auto-push —
                // no need for fast idle polling.  16ms (~60fps) ensures
                // pushed frames render within one vsync while using
                // negligible CPU (vs 50ms poll + dump-state roundtrip).
                16
            };

        cmd_batch.clear();

        // ── Windows paste pending-buffer management ────────────────────
        // Flush or promote chars based on how long they've been buffered.
        #[cfg(windows)]
        {
            if let Some(start) = paste_pend_start {
                let elapsed = start.elapsed();
                if paste_confirmed {
                    // Ctrl+V Release already seen — send as paste now
                    if !paste_pend.is_empty() {
                        if input_log_enabled() {
                            input_log("paste", &format!("paste CONFIRMED (top), sending {} chars as send-paste: {:?}",
                                paste_pend.len(), &paste_pend.chars().take(200).collect::<String>()));
                        }
                        let encoded = base64_encode(&paste_pend);
                        cmd_batch.push(format!("send-paste {}\n", encoded));
                    }
                    paste_pend.clear();
                    paste_pend_start = None;
                    paste_stage2 = false;
                    paste_confirmed = false;
                } else if !paste_stage2 && elapsed > Duration::from_millis(20) {
                    // 20ms window expired
                    let has_non_ascii = paste_pend.chars().any(|c| !c.is_ascii());
                    if paste_pend.len() >= 3 && !has_non_ascii {
                        // ≥3 ASCII chars in 20ms → likely paste, enter stage 2.
                        // Non-ASCII chars (IME composition, CJK input) are excluded
                        // because IME routinely generates 3+ chars in <20ms and would
                        // trigger a false-positive 300ms delay (fixes #91).
                        paste_stage2 = true;
                        paste_stage2_last_len = paste_pend.len();
                        if input_log_enabled() {
                            input_log("paste", &format!("stage2: {} chars in 20ms, waiting for Ctrl+V Release", paste_pend.len()));
                        }
                    } else if paste_pend.len() >= 20 && has_non_ascii {
                        // ≥20 non-ASCII chars in 20ms — almost certainly a paste
                        // containing Unicode content (em-dashes, CJK, etc.), not
                        // IME composition (which rarely exceeds a few chars).
                        paste_stage2 = true;
                        paste_stage2_last_len = paste_pend.len();
                        if input_log_enabled() {
                            input_log("paste", &format!("stage2 (large non-ASCII): {} chars in 20ms", paste_pend.len()));
                        }
                    } else if paste_pend.len() >= 3 && has_non_ascii {
                        // ≥3 chars but contains non-ASCII (IME input) — flush
                        // immediately as normal text to avoid 300ms delay.
                        if input_log_enabled() {
                            input_log("paste", &format!("flush {} chars as normal (non-ASCII / IME detected)", paste_pend.len()));
                        }
                        for c in paste_pend.chars() {
                            match c {
                                '\n' => { cmd_batch.push("send-key enter\n".into()); }
                                '\t' => { cmd_batch.push("send-key tab\n".into()); }
                                ' '  => { cmd_batch.push("send-key space\n".into()); }
                                _ => {
                                    let escaped = match c {
                                        '"' => "\\\"".to_string(),
                                        '\\' => "\\\\".to_string(),
                                        _ => c.to_string(),
                                    };
                                    cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                                }
                            }
                        }
                        paste_pend.clear();
                        paste_pend_start = None;
                    } else {
                        // <3 chars → normal typing, flush as send-text
                        if input_log_enabled() {
                            input_log("paste", &format!("flush {} chars as normal (< 3 in 20ms)", paste_pend.len()));
                        }
                        for c in paste_pend.chars() {
                            match c {
                                '\n' => { cmd_batch.push("send-key enter\n".into()); }
                                '\t' => { cmd_batch.push("send-key tab\n".into()); }
                                ' '  => { cmd_batch.push("send-key space\n".into()); }
                                _ => {
                                    let escaped = match c {
                                        '"' => "\\\"".to_string(),
                                        '\\' => "\\\\".to_string(),
                                        _ => c.to_string(),
                                    };
                                    cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                                }
                            }
                        }
                        paste_pend.clear();
                        paste_pend_start = None;
                    }
                } else if paste_stage2 && elapsed > Duration::from_millis(300) {
                    // Stage 2 timeout — no Ctrl+V Release arrived.
                    // Growth detection: if the buffer grew since last check,
                    // ConPTY is still injecting characters (large paste).
                    // Extend the window instead of splitting the paste.
                    if paste_pend.len() > paste_stage2_last_len {
                        paste_stage2_last_len = paste_pend.len();
                        paste_pend_start = Some(Instant::now() - Duration::from_millis(280));
                    } else {
                        // Buffer stopped growing — send accumulated chars as
                        // send-paste so the server wraps in bracketed paste.
                        if input_log_enabled() {
                            input_log("paste", &format!("stage2 timeout, sending {} chars as send-paste", paste_pend.len()));
                        }
                        let encoded = base64_encode(&paste_pend);
                        cmd_batch.push(format!("send-paste {}\n", encoded));
                        paste_pend.clear();
                        paste_pend_start = None;
                        paste_stage2 = false;
                        paste_stage2_last_len = 0;
                    }
                }
            }
        }

        {
            let mut _pending_evt = input.read_timeout(Duration::from_millis(poll_ms))?;
            while let Some(_cur_evt) = _pending_evt {
                // Input debug: log every raw event BEFORE filtering
                if input_log_enabled() {
                    match &_cur_evt {
                        Event::Key(key) => {
                            input_log("event", &format!(
                                "Key code={:?} mods={:?} kind={:?} state={:?}",
                                key.code, key.modifiers, key.kind, key.state
                            ));
                        }
                        Event::Mouse(me) => {
                            input_log("event", &format!("Mouse {:?}", me.kind));
                        }
                        Event::Resize(w, h) => {
                            input_log("event", &format!("Resize {}x{}", w, h));
                        }
                        Event::Paste(d) => {
                            input_log("event", &format!("Paste ({} bytes)", d.len()));
                        }
                        other => {
                            input_log("event", &format!("Other {:?}", other));
                        }
                    }
                }
                match _cur_evt {
                    // ── Windows Ctrl+V paste interception ────────────────
                    // On Windows, Windows Terminal intercepts Ctrl+V Press,
                    // reads the clipboard, and injects the paste content as
                    // a byte stream into the ConPTY input pipe — bypassing
                    // the console input buffer that crossterm reads via
                    // ReadConsoleInputW.  Only the Ctrl+V *Release* event
                    // leaks through.  We use that Release as a trigger to
                    // read the clipboard ourselves and forward the content
                    // as a bracketed-paste so child apps (Claude CLI, etc.)
                    // can distinguish paste from typed input.
                    #[cfg(windows)]
                    Event::Key(key) if key.kind == KeyEventKind::Release
                        && matches!(key.code, KeyCode::Char('v'))
                        && key.modifiers == KeyModifiers::CONTROL =>
                    {
                        if input_log_enabled() {
                            input_log("paste", &format!("Ctrl+V Release detected, paste_pend len={}", paste_pend.len()));
                        }
                        paste_confirmed = true;
                    }
                    Event::Key(mut key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
                        // On Windows, VS Code's xterm.js sends ESC+CR for
                        // Shift+Enter.  ConPTY interprets the ESC as Alt, so
                        // crossterm reports Alt+Enter.  Poll the physical
                        // keyboard to detect the real modifier.
                        #[cfg(windows)]
                        crate::platform::augment_enter_shift(&mut key);
                        // Flush pending paste buffer before processing any non-bufferable key.
                        // Bufferable keys are: plain Char, Space, Enter (if pend non-empty), Tab (if pend non-empty).
                        #[cfg(windows)]
                        {
                            if !paste_pend.is_empty() {
                                let is_bufferable = match key.code {
                                    KeyCode::Char(' ') => true,
                                    KeyCode::Char(c) => {
                                        // AltGr on Windows is reported as Ctrl+Alt.
                                        // Non-letter chars with Ctrl+Alt are AltGr-produced
                                        // (e.g. \ @ { } on German/Czech keyboards) and
                                        // should be bufferable like normal text.
                                        let is_altgr = key.modifiers.contains(KeyModifiers::CONTROL)
                                            && key.modifiers.contains(KeyModifiers::ALT)
                                            && !c.is_ascii_lowercase();
                                        is_altgr || (!key.modifiers.contains(KeyModifiers::CONTROL)
                                                  && !key.modifiers.contains(KeyModifiers::ALT))
                                    }
                                    KeyCode::Enter | KeyCode::Tab => true, // buffered when pend non-empty
                                    _ => false,
                                };
                                if !is_bufferable {
                                    flush_paste_pend_as_text(&mut paste_pend, &mut paste_pend_start, &mut paste_stage2, &mut cmd_batch);
                                }
                            }
                        }
                        // Dynamic prefix key check (default: Ctrl+B, configurable via .psmux.conf)
                        let is_prefix = (key.code, key.modifiers) == prefix_key
                            || prefix_raw_char.map_or(false, |c| matches!(key.code, KeyCode::Char(ch) if ch == c))
                            || prefix2_key.map_or(false, |p2| (key.code, key.modifiers) == p2)
                            || prefix2_raw_char.map_or(false, |c| matches!(key.code, KeyCode::Char(ch) if ch == c));

                        // Expire repeat-mode prefix if repeat-time has elapsed.
                        // This ensures keys are forwarded to the PTY rather than
                        // being interpreted as prefix bindings (tmux parity).
                        if prefix_armed && prefix_repeating
                            && prefix_armed_at.elapsed().as_millis() >= repeat_time_ms as u128
                        {
                            prefix_armed = false;
                            prefix_repeating = false;
                            cmd_batch.push("prefix-end\n".into());
                        }

                        // Overlay Esc must be checked BEFORE selection-Esc so that
                        // pressing Esc always closes the active overlay first.
                        // ── Server-side overlay key handling ─────────────────
                        // When a server overlay is active, intercept ALL keys and
                        // forward them to the server via overlay-specific commands.
                        if srv_popup_active {
                            match key.code {
                                KeyCode::Esc => { cmd_batch.push("overlay-close\n".into()); }
                                KeyCode::Char(c) => {
                                    let bytes = if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                                        vec![(c as u8) & 0x1F]
                                    } else {
                                        let mut buf = [0u8; 4];
                                        let s = c.encode_utf8(&mut buf);
                                        s.as_bytes().to_vec()
                                    };
                                    let encoded = crate::util::base64_encode(std::str::from_utf8(&bytes).unwrap_or(""));
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Enter => {
                                    let encoded = crate::util::base64_encode("\r");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Backspace => {
                                    let encoded = crate::util::base64_encode("\x7f");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Tab => {
                                    let encoded = crate::util::base64_encode("\t");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Up => {
                                    let encoded = crate::util::base64_encode("\x1b[A");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Down => {
                                    let encoded = crate::util::base64_encode("\x1b[B");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Right => {
                                    let encoded = crate::util::base64_encode("\x1b[C");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Left => {
                                    let encoded = crate::util::base64_encode("\x1b[D");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Home => {
                                    let encoded = crate::util::base64_encode("\x1b[H");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::End => {
                                    let encoded = crate::util::base64_encode("\x1b[F");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::PageUp => {
                                    let encoded = crate::util::base64_encode("\x1b[5~");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::PageDown => {
                                    let encoded = crate::util::base64_encode("\x1b[6~");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                KeyCode::Delete => {
                                    let encoded = crate::util::base64_encode("\x1b[3~");
                                    cmd_batch.push(format!("popup-input {}\n", encoded));
                                }
                                _ => {}
                            }
                        }
                        else if srv_confirm_active {
                            match key.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') => {
                                    cmd_batch.push("confirm-respond y\n".into());
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    cmd_batch.push("confirm-respond n\n".into());
                                }
                                _ => {} // Ignore other keys during confirm
                            }
                        }
                        else if srv_menu_active {
                            match key.code {
                                KeyCode::Up | KeyCode::Char('k') => { cmd_batch.push("menu-navigate -1\n".into()); }
                                KeyCode::Down | KeyCode::Char('j') => { cmd_batch.push("menu-navigate 1\n".into()); }
                                KeyCode::Enter => {
                                    cmd_batch.push(format!("menu-select {}\n", srv_menu_selected));
                                }
                                KeyCode::Esc | KeyCode::Char('q') => { cmd_batch.push("overlay-close\n".into()); }
                                KeyCode::Char(c) => {
                                    // Shortcut key: find menu item with matching key
                                    if let Some(idx) = srv_menu_items.iter().position(|item| {
                                        item.key.as_ref().map(|k| k.len() == 1 && k.chars().next() == Some(c)).unwrap_or(false)
                                    }) {
                                        cmd_batch.push(format!("menu-select {}\n", idx));
                                    }
                                }
                                _ => {}
                            }
                        }
                        else if srv_display_panes {
                            match key.code {
                                KeyCode::Char(d) if d.is_ascii_digit() => {
                                    let digit = d.to_digit(10).unwrap() as usize;
                                    cmd_batch.push(format!("display-panes-select {}\n", digit));
                                }
                                _ => { cmd_batch.push("overlay-close\n".into()); }
                            }
                        }
                        else if matches!(key.code, KeyCode::Esc) && (command_input || renaming || pane_renaming || tree_chooser || session_chooser || confirm_cmd.is_some() || keys_viewer) {
                            command_input = false;
                            renaming = false;
                            pane_renaming = false;
                            tree_chooser = false;
                            session_chooser = false;
                            keys_viewer = false;
                            confirm_cmd = None;
                            // Also clear any lingering selection
                            rsel_start = None;
                            rsel_end = None;
                            selection_changed = true;
                        }
                        else if rsel_start.is_some() && matches!(key.code, KeyCode::Esc) {
                            // Escape clears any active text selection
                            rsel_start = None;
                            rsel_end = None;
                            selection_changed = true;
                        }
                        else if is_prefix { prefix_armed = true; prefix_armed_at = Instant::now(); prefix_repeating = false; cmd_batch.push("prefix-begin\n".into()); }
                        // Check root-table bindings (bind-key -n / bind-key -T root)
                        // These fire without prefix, before keys are forwarded to PTY
                        else if !command_input && !renaming && !pane_renaming && !tree_chooser && !session_chooser && !keys_viewer && confirm_cmd.is_none() && {
                            let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
                            synced_bindings.iter().any(|b| b.t == "root" && parse_key_string(&b.k).map_or(false, |k| normalize_key_for_binding(k) == key_tuple))
                        } {
                            let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
                            if let Some(entry) = synced_bindings.iter().find(|b| {
                                b.t == "root" && parse_key_string(&b.k).map_or(false, |k| normalize_key_for_binding(k) == key_tuple)
                            }) {
                                if entry.c == "detach-client" || entry.c == "detach" {
                                    quit = true;
                                } else {
                                    cmd_batch.push(format!("{}\n", entry.c));
                                }
                            }
                        }
                        else if prefix_armed {
                            // Check user-defined synced bindings FIRST (like server-side input.rs).
                            // This lets users override any default hardcoded key binding.
                            let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
                            let user_binding = synced_bindings.iter().find(|b| {
                                b.t == "prefix" && parse_key_string(&b.k).map_or(false, |k| normalize_key_for_binding(k) == key_tuple)
                            });
                            if let Some(entry) = user_binding {
                                // User-defined binding takes priority
                                if entry.c == "detach-client" || entry.c == "detach" {
                                    quit = true;
                                } else if entry.c.starts_with("confirm-before") || entry.c == "kill-pane" {
                                    confirm_cmd = Some(entry.c.clone());
                                } else {
                                    cmd_batch.push(format!("{}\n", entry.c));
                                }
                            } else {
                            // Default hardcoded bindings (only reached if no user override)
                            match key.code {
                                KeyCode::Char('c') => { cmd_batch.push("new-window\n".into()); }
                                KeyCode::Char('%') => { cmd_batch.push("split-window -h\n".into()); }
                                KeyCode::Char('"') => { cmd_batch.push("split-window -v\n".into()); }
                                KeyCode::Char('x') => { confirm_cmd = Some("kill-pane".into()); }
                                KeyCode::Char('&') => { confirm_cmd = Some("kill-window".into()); }
                                KeyCode::Char('z') => { cmd_batch.push("zoom-pane\n".into()); }
                                KeyCode::Char('[') => { cmd_batch.push("copy-enter\n".into()); }
                                KeyCode::Char(']') => { cmd_batch.push("paste-buffer\n".into()); }
                                KeyCode::Char('{') => { cmd_batch.push("swap-pane -U\n".into()); }
                                KeyCode::Char('}') => { cmd_batch.push("swap-pane -D\n".into()); }
                                KeyCode::Char('n') => { cmd_batch.push("next-window\n".into()); }
                                KeyCode::Char('p') => { cmd_batch.push("previous-window\n".into()); }
                                KeyCode::Char('l') => { cmd_batch.push("last-window\n".into()); }
                                KeyCode::Char(';') => { cmd_batch.push("last-pane\n".into()); }
                                KeyCode::Char(' ') => { cmd_batch.push("next-layout\n".into()); }
                                KeyCode::Char('!') => { cmd_batch.push("break-pane\n".into()); }
                                KeyCode::Char(d) if d.is_ascii_digit() => {
                                    let idx = d.to_digit(10).unwrap() as usize;
                                    cmd_batch.push(format!("select-window {}\n", idx));
                                }
                                KeyCode::Char('o') => { cmd_batch.push("select-pane -t :.+\n".into()); }
                                // Alt+Arrow: resize pane by 5 (must be before plain Arrow)
                                KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -U 5\n".into()); }
                                KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -D 5\n".into()); }
                                KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -L 5\n".into()); }
                                KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("resize-pane -R 5\n".into()); }
                                // Ctrl+Arrow: resize pane by 1
                                KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -U 1\n".into()); }
                                KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -D 1\n".into()); }
                                KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -L 1\n".into()); }
                                KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => { cmd_batch.push("resize-pane -R 1\n".into()); }
                                // Plain Arrow: select pane
                                KeyCode::Up => { cmd_batch.push("select-pane -U\n".into()); }
                                KeyCode::Down => { cmd_batch.push("select-pane -D\n".into()); }
                                KeyCode::Left => { cmd_batch.push("select-pane -L\n".into()); }
                                KeyCode::Right => { cmd_batch.push("select-pane -R\n".into()); }
                                KeyCode::Char('d') => { quit = true; }
                                KeyCode::Char(',') => { renaming = true; rename_buf.clear(); }
                                KeyCode::Char('$') => {
                                    // Rename session — reuse rename overlay
                                    renaming = true;
                                    rename_buf.clear();
                                    // Mark that we're renaming the session, not a window
                                    // We'll detect this by checking if pane_renaming is used as a flag
                                    session_renaming = true;
                                }
                                KeyCode::Char('?') => {
                                    // Build comprehensive help overlay from help.rs
                                    keys_viewer_scroll = 0;
                                    let user_binds: Vec<(bool, String, String, String)> = synced_bindings
                                        .iter()
                                        .map(|b| (b.r, b.t.clone(), b.k.clone(), b.c.clone()))
                                        .collect();
                                    keys_viewer_lines = help::build_overlay_lines(&user_binds);
                                    keys_viewer = true;
                                }
                                KeyCode::Char('t') => { cmd_batch.push("clock-mode\n".into()); }
                                KeyCode::Char('=') => { cmd_batch.push("choose-buffer\n".into()); }
                                KeyCode::Char(':') => { command_input = true; command_buf.clear(); }
                                KeyCode::Char('w') => {
                                    tree_chooser = true;
                                    tree_entries.clear();
                                    tree_selected = 0;
                                    tree_scroll = 0;
                                    // Query ALL sessions (like tmux choose-tree)
                                    let dir = format!("{}\\.psmux", home);
                                    if let Ok(entries) = std::fs::read_dir(&dir) {
                                        let mut sessions: Vec<(String, Vec<(usize, String, Vec<(usize, String)>)>)> = Vec::new();
                                        for e in entries.flatten() {
                                            if let Some(fname) = e.file_name().to_str().map(|s| s.to_string()) {
                                                if let Some((base, ext)) = fname.rsplit_once('.') {
                                                    if ext == "port" {
                                                        // Hide warm (standby) sessions from user
                                                        if crate::session::is_warm_session(base) { continue; }
                                                        if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                                                            if let Ok(p) = port_str.trim().parse::<u16>() {
                                                                let sess_addr = format!("127.0.0.1:{}", p);
                                                                let sess_key = read_session_key(base).unwrap_or_default();
                                                                if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                                                                    &sess_addr.parse().unwrap(), Duration::from_millis(50)
                                                                ) {
                                                                    let _ = ss.set_read_timeout(Some(Duration::from_millis(100)));
                                                                    let _ = write!(ss, "AUTH {}\n", sess_key);
                                                                    let _ = ss.write_all(b"list-tree\n");
                                                                    let _ = ss.flush();
                                                                    let mut br = BufReader::new(ss);
                                                                    let mut al = String::new();
                                                                    let _ = br.read_line(&mut al); // AUTH OK
                                                                    let mut tree_line = String::new();
                                                                    if br.read_line(&mut tree_line).is_ok() {
                                                                        // Parse JSON array of WinTree
                                                                        if let Ok(wins) = serde_json::from_str::<Vec<WinTree>>(&tree_line.trim()) {
                                                                            let mut win_data = Vec::new();
                                                                            for w in &wins {
                                                                                let panes: Vec<(usize, String)> = w.panes.iter().map(|p| (p.id, p.title.clone())).collect();
                                                                                win_data.push((w.id, w.name.clone(), panes));
                                                                            }
                                                                            sessions.push((base.to_string(), win_data));
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // Sort sessions: current session first, then alphabetical
                                        sessions.sort_by(|a, b| {
                                            if a.0 == current_session { std::cmp::Ordering::Less }
                                            else if b.0 == current_session { std::cmp::Ordering::Greater }
                                            else { a.0.cmp(&b.0) }
                                        });
                                        // Build tree entries: session > window > pane
                                        // tree_entries format: (is_win, id, sub_id, label, session_name)
                                        // For session headers: is_win=true with id=usize::MAX as sentinel
                                        // For windows: is_win=true
                                        // For panes: is_win=false
                                        for (sess_name, wins) in &sessions {
                                            let is_current = sess_name == &current_session;
                                            let attached = if is_current { " (attached)" } else { "" };
                                            let nw = wins.len();
                                            // Session header line
                                            tree_entries.push((true, usize::MAX, 0,
                                                format!("{}: {} windows{}", sess_name, nw, attached),
                                                sess_name.clone()));
                                            if is_current {
                                                // Show windows and panes for current session
                                                for (wi, (wid, wname, panes)) in wins.iter().enumerate() {
                                                    let flag = if panes.len() > 0 { "" } else { "" };
                                                    tree_entries.push((true, *wid, 0,
                                                        format!("  {}: {}{} ({} panes)", wi, wname, flag, panes.len()),
                                                        sess_name.clone()));
                                                    for (pid, ptitle) in panes {
                                                        tree_entries.push((false, *wid, *pid,
                                                            format!("    {}", ptitle),
                                                            sess_name.clone()));
                                                    }
                                                }
                                            } else {
                                                // Show windows for other sessions (collapsed)
                                                for (wi, (wid, wname, panes)) in wins.iter().enumerate() {
                                                    tree_entries.push((true, *wid, 0,
                                                        format!("  {}: {} ({} panes)", wi, wname, panes.len()),
                                                        sess_name.clone()));
                                                }
                                            }
                                        }
                                    }
                                    // Fallback: if no sessions found, use current session data
                                    if tree_entries.is_empty() {
                                        for wi in &last_tree {
                                            tree_entries.push((true, wi.id, 0, wi.name.clone(), current_session.clone()));
                                            for pi in &wi.panes {
                                                tree_entries.push((false, wi.id, pi.id, pi.title.clone(), current_session.clone()));
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('s') => {
                                    session_chooser = true;
                                    session_entries.clear();
                                    session_selected = 0;
                                    let dir = format!("{}\\.psmux", home);
                                    if let Ok(entries) = std::fs::read_dir(&dir) {
                                        for e in entries.flatten() {
                                            if let Some(fname) = e.file_name().to_str() {
                                                if let Some((base, ext)) = fname.rsplit_once('.') {
                                                    if ext == "port" {
                                                        if crate::session::is_warm_session(base) { continue; }
                                                        if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                                                            if let Ok(p) = port_str.trim().parse::<u16>() {
                                                                let sess_addr = format!("127.0.0.1:{}", p);
                                                                let sess_key = read_session_key(base).unwrap_or_default();
                                                                let info = if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                                                                    &sess_addr.parse().unwrap(), Duration::from_millis(25)
                                                                ) {
                                                                    let _ = ss.set_read_timeout(Some(Duration::from_millis(25)));
                                                                    let _ = write!(ss, "AUTH {}\n", sess_key);
                                                                    let _ = ss.write_all(b"session-info\n");
                                                                    let mut br = BufReader::new(ss);
                                                                    let mut al = String::new();
                                                                    let _ = br.read_line(&mut al);
                                                                    let mut line = String::new();
                                                                    if br.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                                                                        line.trim().to_string()
                                                                    } else {
                                                                        format!("{}: (no info)", base)
                                                                    }
                                                                } else {
                                                                    format!("{}: (not responding)", base)
                                                                };
                                                                session_entries.push((base.to_string(), info));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if session_entries.is_empty() {
                                        session_entries.push((current_session.clone(), format!("{}: (current)", current_session)));
                                    }
                                    for (i, (sname, _)) in session_entries.iter().enumerate() {
                                        if sname == &current_session { session_selected = i; break; }
                                    }
                                }
                                KeyCode::Char('q') => { cmd_batch.push("display-panes\n".into()); }
                                KeyCode::Char('v') => { cmd_batch.push("rectangle-toggle\n".into()); }
                                KeyCode::Char('y') => { cmd_batch.push("copy-yank\n".into()); }
                                // Session navigation (like tmux prefix+( and prefix+))
                                KeyCode::Char('(') | KeyCode::Char(')') => {
                                    let dir_next = key.code == KeyCode::Char(')');
                                    // Enumerate sessions
                                    let dir = format!("{}\\.psmux", home);
                                    let mut names: Vec<String> = Vec::new();
                                    if let Ok(entries) = std::fs::read_dir(&dir) {
                                        for e in entries.flatten() {
                                            if let Some(fname) = e.file_name().to_str() {
                                                if let Some((base, ext)) = fname.rsplit_once('.') {
                                                    if ext == "port" {
                                                        if crate::session::is_warm_session(base) { continue; }
                                                        if let Ok(ps) = std::fs::read_to_string(e.path()) {
                                                            if let Ok(p) = ps.trim().parse::<u16>() {
                                                                let a = format!("127.0.0.1:{}", p);
                                                                if std::net::TcpStream::connect_timeout(
                                                                    &a.parse().unwrap(), Duration::from_millis(25)
                                                                ).is_ok() {
                                                                    names.push(base.to_string());
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    names.sort();
                                    if names.len() > 1 {
                                        if let Some(cur_pos) = names.iter().position(|n| *n == current_session) {
                                            let next_pos = if dir_next {
                                                (cur_pos + 1) % names.len()
                                            } else {
                                                (cur_pos + names.len() - 1) % names.len()
                                            };
                                            let next_name = names[next_pos].clone();
                                            cmd_batch.push("client-detach\n".into());
                                            env::set_var("PSMUX_SWITCH_TO", &next_name);
                                            quit = true;
                                        }
                                    }
                                }
                                // Meta+1..5 preset layouts (like tmux)
                                KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout even-horizontal\n".into()); }
                                KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout even-vertical\n".into()); }
                                KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout main-horizontal\n".into()); }
                                KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout main-vertical\n".into()); }
                                KeyCode::Char('5') if key.modifiers.contains(KeyModifiers::ALT) => { cmd_batch.push("select-layout tiled\n".into()); }
                                // Display pane info
                                KeyCode::Char('i') => { cmd_batch.push("display-message\n".into()); }
                                _ => {
                                    // No default binding for this key (user bindings already checked above)
                                }
                            }
                            } // end of else (no user binding override)
                            // Arrow keys are repeatable by default (tmux -r flag).
                            // User-defined bindings also respect the repeat flag.
                            let is_repeatable_default = matches!(key.code,
                                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
                            );
                            let is_user_repeat = user_binding.map_or(false, |e| e.r);
                            if is_repeatable_default || is_user_repeat {
                                prefix_armed_at = Instant::now();
                                prefix_repeating = true;
                            } else {
                                prefix_armed = false;
                                prefix_repeating = false;
                                cmd_batch.push("prefix-end\n".into());
                            }
                        } else {
                            match key.code {
                                KeyCode::Up if session_chooser => { if session_selected > 0 { session_selected -= 1; } }
                                KeyCode::Down if session_chooser => { if session_selected + 1 < session_entries.len() { session_selected += 1; } }
                                KeyCode::Enter if session_chooser => {
                                    if let Some((sname, _)) = session_entries.get(session_selected) {
                                        if sname != &current_session {
                                            cmd_batch.push("client-detach\n".into());
                                            env::set_var("PSMUX_SWITCH_TO", sname);
                                            quit = true;
                                        }
                                        session_chooser = false;
                                    }
                                }
                                KeyCode::Esc if session_chooser => { session_chooser = false; }
                                KeyCode::Char('x') if session_chooser => {
                                    // Kill the selected session (like tmux session chooser)
                                    if let Some((sname, _)) = session_entries.get(session_selected) {
                                        let sname = sname.clone();
                                        if sname == current_session {
                                            // Killing current session — exit after kill
                                            cmd_batch.push("kill-session\n".into());
                                            session_chooser = false;
                                            quit = true;
                                        } else {
                                            // Kill another session by connecting to it
                                            let h = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                                            let port_path = format!("{}\\.psmux\\{}.port", h, sname);
                                            let key_path = format!("{}\\.psmux\\{}.key", h, sname);
                                            if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                                                if let Ok(port) = port_str.trim().parse::<u16>() {
                                                    let addr = format!("127.0.0.1:{}", port);
                                                    let sess_key = std::fs::read_to_string(&key_path).unwrap_or_default();
                                                    if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                                                        &addr.parse().unwrap(), Duration::from_millis(100)
                                                    ) {
                                                        let _ = write!(ss, "AUTH {}\n", sess_key.trim());
                                                        let _ = ss.write_all(b"kill-session\n");
                                                    }
                                                }
                                            }
                                            // Remove the killed session from the list
                                            session_entries.remove(session_selected);
                                            if session_selected >= session_entries.len() && session_selected > 0 {
                                                session_selected -= 1;
                                            }
                                            if session_entries.is_empty() {
                                                session_chooser = false;
                                            }
                                        }
                                    }
                                }
                                KeyCode::Up if tree_chooser => { if tree_selected > 0 { tree_selected -= 1; } }
                                KeyCode::Down if tree_chooser => { if tree_selected + 1 < tree_entries.len() { tree_selected += 1; } }
                                KeyCode::Enter if tree_chooser => {
                                    if let Some((is_win, wid, pid, _label, sess_name)) = tree_entries.get(tree_selected) {
                                        if *wid == usize::MAX {
                                            // Session header — switch to that session
                                            if *sess_name != current_session {
                                                cmd_batch.push("client-detach\n".into());
                                                env::set_var("PSMUX_SWITCH_TO", sess_name);
                                                quit = true;
                                            }
                                            tree_chooser = false;
                                        } else if *sess_name != current_session {
                                            // Window/pane in another session — switch to that session
                                            cmd_batch.push("client-detach\n".into());
                                            env::set_var("PSMUX_SWITCH_TO", sess_name);
                                            quit = true;
                                            tree_chooser = false;
                                        } else if *is_win {
                                            cmd_batch.push(format!("focus-window {}\n", wid));
                                            tree_chooser = false;
                                        } else {
                                            cmd_batch.push(format!("focus-pane {}\n", pid));
                                            tree_chooser = false;
                                        }
                                    }
                                }
                                KeyCode::Esc if tree_chooser => { tree_chooser = false; }
                                // --- list-keys viewer (C-b ?) ---
                                KeyCode::Up if keys_viewer => { if keys_viewer_scroll > 0 { keys_viewer_scroll -= 1; } }
                                KeyCode::Down if keys_viewer => { keys_viewer_scroll += 1; }
                                KeyCode::PageUp if keys_viewer => { keys_viewer_scroll = keys_viewer_scroll.saturating_sub(20); }
                                KeyCode::PageDown if keys_viewer => { keys_viewer_scroll += 20; }
                                KeyCode::Home if keys_viewer => { keys_viewer_scroll = 0; }
                                KeyCode::End if keys_viewer => { keys_viewer_scroll = keys_viewer_lines.len().saturating_sub(1); }
                                KeyCode::Char('q') if keys_viewer => { keys_viewer = false; }
                                KeyCode::Esc if keys_viewer => { keys_viewer = false; }
                                KeyCode::Char('k') if keys_viewer => { if keys_viewer_scroll > 0 { keys_viewer_scroll -= 1; } }
                                KeyCode::Char('j') if keys_viewer => { keys_viewer_scroll += 1; }
                                // --- kill confirmation: y/Y/Enter confirms, n/N/Esc cancels ---
                                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter if confirm_cmd.is_some() => {
                                    if let Some(cmd) = confirm_cmd.take() {
                                        cmd_batch.push(format!("{}\n", cmd));
                                    }
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc if confirm_cmd.is_some() => {
                                    confirm_cmd = None;
                                }
                                KeyCode::Char(c) if renaming && !key.modifiers.contains(KeyModifiers::CONTROL) => { rename_buf.push(c); }
                                KeyCode::Char(c) if pane_renaming && !key.modifiers.contains(KeyModifiers::CONTROL) => { pane_title_buf.push(c); }
                                KeyCode::Char(c) if command_input && !key.modifiers.contains(KeyModifiers::CONTROL) => { command_buf.push(c); }
                                KeyCode::Backspace if renaming => { let _ = rename_buf.pop(); }
                                KeyCode::Backspace if pane_renaming => { let _ = pane_title_buf.pop(); }
                                KeyCode::Backspace if command_input => { let _ = command_buf.pop(); }
                                KeyCode::Enter if renaming => {
                                    if session_renaming {
                                        cmd_batch.push(format!("rename-session {}\n", quote_arg(&rename_buf)));
                                        session_renaming = false;
                                    } else {
                                        cmd_batch.push(format!("rename-window {}\n", quote_arg(&rename_buf)));
                                    }
                                    renaming = false;
                                }
                                KeyCode::Enter if pane_renaming => { cmd_batch.push(format!("set-pane-title {}\n", quote_arg(&pane_title_buf))); pane_renaming = false; }
                                KeyCode::Enter if command_input => {
                                    let trimmed = command_buf.trim().to_string();
                                    if !trimmed.is_empty() {
                                        cmd_batch.push(format!("{}\n", trimmed));
                                    }
                                    command_input = false;
                                }
                                KeyCode::Esc if renaming => { renaming = false; session_renaming = false; }
                                KeyCode::Esc if pane_renaming => { pane_renaming = false; }
                                KeyCode::Esc if command_input => { command_input = false; }

                                KeyCode::Char(' ') => {
                                    #[cfg(windows)]
                                    {
                                        paste_pend.push(' ');
                                        if paste_pend_start.is_none() {
                                            paste_pend_start = Some(Instant::now());
                                        }
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        cmd_batch.push("send-key space\n".into());
                                    }
                                }
                                // AltGr detection: On Windows, AltGr is reported as
                                // Ctrl+Alt.  Non-lowercase-letter chars with Ctrl+Alt
                                // are AltGr-produced (e.g. \ @ { } [ ] | ~ on
                                // German/Czech keyboards) — treat as plain text.
                                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL)
                                    && key.modifiers.contains(KeyModifiers::ALT)
                                    && !c.is_ascii_lowercase() => {
                                    #[cfg(windows)]
                                    {
                                        paste_pend.push(c);
                                        if paste_pend_start.is_none() {
                                            paste_pend_start = Some(Instant::now());
                                        }
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        let escaped = match c {
                                            '"' => "\\\"".to_string(),
                                            '\\' => "\\\\".to_string(),
                                            _ => c.to_string(),
                                        };
                                        cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                                    }
                                }
                                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::ALT) => {
                                    cmd_batch.push(format!("send-key C-M-{}\n", c.to_ascii_lowercase()));
                                }
                                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
                                    cmd_batch.push(format!("send-key M-{}\n", c));
                                }
                                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    cmd_batch.push(format!("send-key C-{}\n", c.to_ascii_lowercase()));
                                }
                                KeyCode::Char(c) if (c as u32) >= 0x01 && (c as u32) <= 0x1A => {
                                    let ctrl_letter = ((c as u8) + b'a' - 1) as char;
                                    cmd_batch.push(format!("send-key C-{}\n", ctrl_letter));
                                }
                                KeyCode::Char(c) => {
                                    #[cfg(windows)]
                                    {
                                        // Suppress text key events during the post-copy
                                        // suppression window (VS Code ConPTY duplicate).
                                        let suppressed = paste_suppress_until
                                            .map_or(false, |t| Instant::now() < t);
                                        if suppressed {
                                            if input_log_enabled() {
                                                input_log("paste", &format!("suppressed char '{}' during paste suppress window", c));
                                            }
                                        } else {
                                            paste_suppress_until = None;
                                            paste_pend.push(c);
                                            if paste_pend_start.is_none() {
                                                paste_pend_start = Some(Instant::now());
                                            }
                                        }
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        let escaped = match c {
                                            '"' => "\\\"".to_string(),
                                            '\\' => "\\\\".to_string(),
                                            _ => c.to_string(),
                                        };
                                        cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                                    }
                                }
                                KeyCode::Enter => {
                                    #[cfg(windows)]
                                    {
                                        if !paste_pend.is_empty() {
                                            paste_pend.push('\n');
                                        } else {
                                            cmd_batch.push(format!("send-key {}\n", modified_key_name("Enter", key.modifiers)));
                                        }
                                    }
                                    #[cfg(not(windows))]
                                    { cmd_batch.push(format!("send-key {}\n", modified_key_name("Enter", key.modifiers))); }
                                }
                                KeyCode::Tab => {
                                    #[cfg(windows)]
                                    {
                                        if !paste_pend.is_empty() {
                                            paste_pend.push('\t');
                                        } else {
                                            cmd_batch.push("send-key tab\n".into());
                                        }
                                    }
                                    #[cfg(not(windows))]
                                    { cmd_batch.push("send-key tab\n".into()); }
                                }
                                KeyCode::BackTab => { cmd_batch.push("send-key btab\n".into()); }
                                KeyCode::Backspace => { cmd_batch.push("send-key backspace\n".into()); }
                                KeyCode::Delete => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Delete", key.modifiers))); }
                                KeyCode::Esc => { cmd_batch.push("send-key esc\n".into()); }
                                KeyCode::Left => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Left", key.modifiers))); }
                                KeyCode::Right => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Right", key.modifiers))); }
                                KeyCode::Up => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Up", key.modifiers))); }
                                KeyCode::Down => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Down", key.modifiers))); }
                                KeyCode::PageUp => { cmd_batch.push(format!("send-key {}\n", modified_key_name("PageUp", key.modifiers))); }
                                KeyCode::PageDown => { cmd_batch.push(format!("send-key {}\n", modified_key_name("PageDown", key.modifiers))); }
                                KeyCode::Home => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Home", key.modifiers))); }
                                KeyCode::End => { cmd_batch.push(format!("send-key {}\n", modified_key_name("End", key.modifiers))); }
                                KeyCode::Insert => { cmd_batch.push(format!("send-key {}\n", modified_key_name("Insert", key.modifiers))); }
                                KeyCode::F(n) => { cmd_batch.push(format!("send-key {}\n", modified_key_name(&format!("F{}", n), key.modifiers))); }
                                _ => {}
                            }
                        }
                    }
                    Event::Paste(data) => {
                        let encoded = base64_encode(&data);
                        cmd_batch.push(format!("send-paste {}\n", encoded));
                    }
                    Event::Mouse(me) => {
                        use crossterm::event::{MouseEventKind, MouseButton};
                        match me.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                // Check if server-side copy mode is active
                                let server_copy = if !prev_dump_buf.is_empty() {
                                    serde_json::from_str::<DumpState>(&prev_dump_buf)
                                        .map(|s| active_pane_in_copy_mode(&s.layout))
                                        .unwrap_or(false)
                                } else { false };

                                // Detect if click is on a separator line (for border resize)
                                // Skip when zoomed — no borders to drag (#82)
                                let on_sep = if !prev_dump_buf.is_empty() {
                                    if let Ok(state) = serde_json::from_str::<DumpState>(&prev_dump_buf) {
                                        if state.zoomed { false } else {
                                            let content_area = Rect { x: 0, y: 0, width: last_sent_size.0, height: last_sent_size.1 };
                                            is_on_separator(&state.layout, content_area, me.column, me.row)
                                        }
                                    } else { false }
                                } else { false };

                                // Always forward to server for pane focus, tab clicks, border resize, copy-mode cursor positioning
                                cmd_batch.push(format!("mouse-down {} {}\n", me.column, me.row));

                                if server_copy {
                                    // Server handles copy-mode selection — suppress client-side selection
                                    rsel_start = None;
                                    rsel_end = None;
                                    selection_changed = true;
                                } else if on_sep {
                                    // Border resize mode — server handles drag
                                    border_drag = true;
                                    rsel_start = None;
                                    rsel_end = None;
                                    selection_changed = true;
                                } else {
                                    // Text selection mode
                                    border_drag = false;
                                    rsel_start = Some((me.column, me.row));
                                    rsel_end = Some((me.column, me.row));
                                    rsel_dragged = false;
                                    selection_changed = true;
                                }
                            }
                            MouseEventKind::Down(MouseButton::Right) => {
                                // Check if active pane is running a TUI app (alternate screen).
                                // TUI apps (htop, Claude Code, etc.) expect right-click as a
                                // mouse event, NOT clipboard paste.
                                let tui_active = if !prev_dump_buf.is_empty() {
                                    serde_json::from_str::<DumpState>(&prev_dump_buf)
                                        .map(|s| active_pane_in_alt_screen(&s.layout))
                                        .unwrap_or(false)
                                } else { false };

                                if tui_active {
                                    // Forward right-click to server → inject_mouse_combined
                                    // handles Win32 MOUSE_EVENT injection to ConPTY.
                                    cmd_batch.push(format!("mouse-down-right {} {}\n", me.column, me.row));
                                    rsel_start = None;
                                    rsel_end = None;
                                    selection_changed = true;
                                } else if rsel_start.is_some() && rsel_dragged {
                                    // pwsh-style: right-click with active selection → copy + clear
                                    if let (Some(s), Some(e)) = (rsel_start, rsel_end) {
                                        if let Ok(state) = serde_json::from_str::<DumpState>(&prev_dump_buf) {
                                            let text = extract_selection_text(
                                                &state.layout,
                                                last_sent_size.0,
                                                last_sent_size.1,
                                                s, e,
                                            );
                                            if !text.is_empty() {
                                                copy_to_system_clipboard(&text);
                                                pending_osc52 = Some(text);
                                            }
                                        }
                                    }
                                    rsel_start = None;
                                    rsel_end = None;
                                    rsel_dragged = false;
                                    selection_changed = true;
                                    // Suppress text key events that VS Code's ConPTY
                                    // injects after a right-click copy action.
                                    paste_suppress_until = Some(Instant::now() + Duration::from_secs(2));
                                } else {
                                    // No selection, no TUI — paste from clipboard (pwsh-style)
                                    rsel_start = None;
                                    rsel_end = None;
                                    selection_changed = true;
                                    if let Some(text) = read_from_system_clipboard() {
                                        if !text.is_empty() {
                                            let encoded = base64_encode(&text);
                                            cmd_batch.push(format!("send-paste {}\n", encoded));
                                        }
                                    }
                                }
                            }
                            MouseEventKind::Down(MouseButton::Middle) => { cmd_batch.push(format!("mouse-down-middle {} {}\n", me.column, me.row)); }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                if border_drag {
                                    // Forward drag to server for border resize
                                    cmd_batch.push(format!("mouse-drag {} {}\n", me.column, me.row));
                                } else if rsel_start.is_none() {
                                    // No client selection in progress (copy mode or suppressed)
                                    // — forward to server for copy-mode drag selection
                                    cmd_batch.push(format!("mouse-drag {} {}\n", me.column, me.row));
                                } else {
                                    // Left-drag: extend text selection (pwsh behavior)
                                    if rsel_start.is_some() {
                                        rsel_end = Some((me.column, me.row));
                                        rsel_dragged = true;
                                        selection_changed = true;
                                    }
                                }
                            }
                            MouseEventKind::Drag(MouseButton::Right) => {}
                            MouseEventKind::Up(MouseButton::Left) => {
                                if border_drag {
                                    // Forward mouse-up to server to finalize border resize
                                    cmd_batch.push(format!("mouse-up {} {}\n", me.column, me.row));
                                    border_drag = false;
                                } else if rsel_dragged {
                                    // Left-drag completed — copy selected text to clipboard
                                    rsel_end = Some((me.column, me.row));
                                    if let (Some(s), Some(e)) = (rsel_start, rsel_end) {
                                        if let Ok(state) = serde_json::from_str::<DumpState>(&prev_dump_buf) {
                                            let text = extract_selection_text(
                                                &state.layout,
                                                last_sent_size.0,
                                                last_sent_size.1,
                                                s, e,
                                            );
                                            if !text.is_empty() {
                                                copy_to_system_clipboard(&text);
                                                pending_osc52 = Some(text);
                                            }
                                        }
                                    }
                                    // Keep selection visible (clears on next click or Escape)
                                } else {
                                    // Plain left-click (no drag) — clear any old selection, forward mouse-up
                                    rsel_start = None;
                                    rsel_end = None;
                                    selection_changed = true;
                                    // Always forward to server (finalises copy-mode click)
                                    cmd_batch.push(format!("mouse-up {} {}\n", me.column, me.row));
                                }
                            }
                            MouseEventKind::Up(MouseButton::Right) => {}
                            MouseEventKind::Up(MouseButton::Middle) => {}
                            MouseEventKind::Moved => {
                                // Forward bare mouse motion (hover) to server unconditionally.
                                // SGR button 35 (bare motion) is harmless to shells —
                                // PSReadLine ignores bare motion MOUSE_EVENT records, and
                                // ReadFile-based apps (opencode) receive no data when
                                // ENABLE_VIRTUAL_TERMINAL_INPUT is not set.
                                // Same-coordinate dedup on the server prevents flooding.
                                cmd_batch.push(format!("mouse-move {} {}\n", me.column, me.row));
                            }
                            MouseEventKind::ScrollUp => {
                                // Clear client-side selection when scrolling — server may
                                // enter copy mode, and the blue overlay would hide it.
                                crate::debug_log::client_log("scroll", &format!(
                                    "ScrollUp col={} row={} rsel_start={:?} rsel_end={:?} rsel_dragged={}",
                                    me.column, me.row, rsel_start, rsel_end, rsel_dragged));
                                if rsel_start.is_some() {
                                    rsel_start = None;
                                    rsel_end = None;
                                    rsel_dragged = false;
                                    selection_changed = true;
                                }
                                cmd_batch.push(format!("scroll-up {} {}\n", me.column, me.row));
                            }
                            MouseEventKind::ScrollDown => {
                                if rsel_start.is_some() {
                                    rsel_start = None;
                                    rsel_end = None;
                                    rsel_dragged = false;
                                    selection_changed = true;
                                }
                                cmd_batch.push(format!("scroll-down {} {}\n", me.column, me.row));
                            }
                            _ => {}
                        }
                    }
                    Event::FocusGained => {
                        cmd_batch.push("focus-in\n".into());
                    }
                    Event::FocusLost => {
                        cmd_batch.push("focus-out\n".into());
                    }
                    _ => {}
                }
                if quit { break; }
                _pending_evt = input.try_read()?;
            }
        }
        if quit { break; }

        // ── Windows paste buffer flush (post-event) ────────────────────
        // If Ctrl+V Release was seen in this iteration AND we have pending
        // chars, immediately send as send-paste (don't wait for top-of-loop).
        #[cfg(windows)]
        {
            if paste_confirmed && !paste_pend.is_empty() {
                if input_log_enabled() {
                    input_log("paste", &format!("paste CONFIRMED (post-event), sending {} chars as send-paste: {:?}",
                        paste_pend.len(), &paste_pend.chars().take(200).collect::<String>()));
                }
                let encoded = base64_encode(&paste_pend);
                cmd_batch.push(format!("send-paste {}\n", encoded));
                paste_pend.clear();
                paste_pend_start = None;
                paste_stage2 = false;
                paste_confirmed = false;
            } else if paste_confirmed && paste_pend.is_empty() {
                // Ctrl+V with no buffered chars — read clipboard as fallback
                if let Some(text) = read_from_system_clipboard() {
                    if !text.is_empty() {
                        if input_log_enabled() {
                            input_log("paste", &format!("paste CONFIRMED (no buffer), clipboard read len={}", text.len()));
                        }
                        let encoded = base64_encode(&text);
                        cmd_batch.push(format!("send-paste {}\n", encoded));
                    }
                }
                paste_confirmed = false;
            }
        }

        // ── STEP 2: Send commands immediately, refresh screen at capped rate ──
        // Send client-size if changed
        let mut size_changed = false;
        {
            let ts = terminal.size()?;
            let new_size = (ts.width, ts.height.saturating_sub(last_status_lines));
            if new_size != last_sent_size {
                last_sent_size = new_size;
                size_changed = true;
                if writer.write_all(format!("client-size {} {}\n", new_size.0, new_size.1).as_bytes()).is_err() {
                    break; // Connection lost
                }
                // SSH: re-send mouse-enable on resize — terminal may reset
                // mouse reporting mode after a window size change.
                if is_ssh_mode {
                    crate::ssh_input::send_mouse_enable();
                    last_mouse_enable = Instant::now();
                }
            }
        }

        // Send all batched commands immediately — keys reach the server
        // without waiting for a dump-state round-trip
        let sent_keys_this_iter = !cmd_batch.is_empty();
        if sent_keys_this_iter {
            if input_log_enabled() {
                for cmd in &cmd_batch {
                    input_log("send", &format!("→ {}", cmd.trim()));
                }
            }
            for cmd in &cmd_batch {
                if writer.write_all(cmd.as_bytes()).is_err() {
                    break; // Connection lost
                }
            }
            let _ = writer.flush(); // push keys to server NOW
            last_key_send_time = Some(Instant::now());
            key_send_instant = Some(Instant::now());
            // Force immediate dump-state so we start the echo-detection
            // polling chain right away (eliminates 0-10ms initial wait).
            force_dump = true;
        }

        // ── STEP 2b: Request screen update (non-blocking) ────────────────
        // Rate-limit dump-state requests to avoid flooding the server.
        // dump_in_flight prevents >1 concurrent request; the interval check
        // ensures we don't re-request faster than ~100fps when typing.
        let overlays_active = command_input || renaming || pane_renaming || tree_chooser || session_chooser || keys_viewer || confirm_cmd.is_some() || srv_popup_active || srv_confirm_active || srv_menu_active || srv_display_panes || clock_active;
        let should_dump = if force_dump || size_changed {
            true
        } else if typing_active {
            since_dump >= 10  // ~100fps cap when typing (matches poll_ms)
        } else {
            // Server auto-pushes frames when state changes (PTY output,
            // new window, etc.) — no idle dump-state polling needed.
            // This saves CPU + bandwidth: no 50-100KB JSON roundtrips
            // when the client is just sitting idle.
            false
        };
        if should_dump && !dump_in_flight {
            if writer.write_all(b"dump-state\n").is_err() { break; }
            if writer.flush().is_err() { break; }
            dump_in_flight = true;
            dump_flight_start = Instant::now();
        }

        // ── STEP 3: Render if we have a frame ────────────────────────────
        // Also render if selection changed (for highlight overlay) even without new frame
        // Always render when overlays are active (command prompt, rename, choosers)
        if !got_frame && !selection_changed && !overlays_active {
            continue;
        }

        // Skip parse + render when the raw JSON is identical to the previous
        // frame AND selection hasn't changed AND no overlays are active.
        if dump_buf == prev_dump_buf && !selection_changed && !overlays_active {
            last_dump_time = Instant::now();
            continue;
        }

        // Parse the frame (use prev_dump_buf for selection-only redraws)
        let frame_to_parse = if got_frame && dump_buf != prev_dump_buf { &dump_buf } else { &prev_dump_buf };
        let _t_parse = Instant::now();
        let state: DumpState = match serde_json::from_str(frame_to_parse) {
            Ok(s) => s,
            Err(_e) => {
                client_log("parse", &format!("JSON parse error: {} (len={})", _e, frame_to_parse.len()));
                force_dump = true;
                selection_changed = false;
                continue;
            }
        };
        let _parse_us = _t_parse.elapsed().as_micros();
        if client_log_enabled() {
            client_log("parse", &format!("OK in {}us, {} windows", _parse_us, state.windows.len()));
        }

        let root = state.layout;
        let windows = state.windows;
        last_tree = state.tree;
        let base_index = state.base_index;
        let dim_preds = state.prediction_dimming;
        clock_active = state.clock_mode;
        let state_cursor_style_code = state.cursor_style_code;
        // Server-side overlay state (update persistent variables)
        srv_popup_active = state.popup_active;
        srv_popup_command = state.popup_command.unwrap_or_default();
        srv_popup_width = state.popup_width.unwrap_or(80);
        srv_popup_height = state.popup_height.unwrap_or(24);
        srv_popup_lines = state.popup_lines;
        srv_confirm_active = state.confirm_active;
        srv_confirm_prompt = state.confirm_prompt.unwrap_or_default();
        srv_menu_active = state.menu_active;
        srv_menu_title = state.menu_title.unwrap_or_default();
        srv_menu_selected = state.menu_selected;
        srv_menu_items = state.menu_items;
        srv_display_panes = state.display_panes;
        srv_pane_base_index = state.pane_base_index;

        // ── Extract active pane's cursor state ──────────────────────
        // We collect cursor info here but DON'T use
        // f.set_cursor_position() inside the draw callback for the
        // normal (non-copy-mode) active pane.  Instead we write
        // cursor show/hide + position + style as ONE atomic write
        // after terminal.draw().  This prevents ratatui's separate
        // execute!(..., Show/Hide) flushes from creating intermediate
        // states visible to Windows Terminal between vsync frames,
        // which causes rapid cursor flicker during high-frequency
        // output (e.g. opencode streaming).
        let mut post_draw_cursor: Option<(u16, u16)> = None; // pane-local (col, row)
        {
            fn active_cursor_info(node: &LayoutJson) -> Option<(bool, u16, u16, bool)> {
                match node {
                    LayoutJson::Leaf { active, hide_cursor, cursor_row, cursor_col, copy_mode, .. } => {
                        if *active { Some((*hide_cursor, *cursor_row, *cursor_col, *copy_mode)) } else { None }
                    }
                    LayoutJson::Split { children, .. } => {
                        children.iter().find_map(active_cursor_info)
                    }
                }
            }
            if let Some((hide, cr, cc, copy)) = active_cursor_info(&root) {
                if !hide && !clock_active && !copy {
                    post_draw_cursor = Some((cc, cr));
                }
            }
        }

        // ── OSC 52: propagate server-side clipboard to local terminal ────
        // When the server copies text (yank_selection / copy mode),
        // it includes a one-shot clipboard_osc52 field in the dump.
        // Buffer for emission after terminal.draw() to avoid corrupting
        // ratatui's output.
        if let Some(ref clip_b64) = state.clipboard_osc52 {
            if let Some(clip_text) = crate::util::base64_decode(clip_b64) {
                // Also set the local Win32 clipboard for non-SSH scenarios
                copy_to_system_clipboard(&clip_text);
                pending_osc52 = Some(clip_text);
            }
        }

        // Update prefix key from server config (if provided)
        if let Some(ref prefix_str) = state.prefix {
            if let Some((kc, km)) = parse_key_string(prefix_str) {
                if (kc, km) != prefix_key {
                    prefix_key = (kc, km);
                    // Compute raw control character for Ctrl+<letter> prefix
                    prefix_raw_char = if km.contains(KeyModifiers::CONTROL) {
                        if let KeyCode::Char(c) = kc {
                            Some((c as u8 & 0x1f) as char)
                        } else { None }
                    } else { None };
                }
            }
        }

        // Update prefix2 key from server config (if provided)
        if let Some(ref prefix2_str) = state.prefix2 {
            if !prefix2_str.is_empty() {
                if let Some((kc, km)) = parse_key_string(prefix2_str) {
                    prefix2_key = Some((kc, km));
                    prefix2_raw_char = if km.contains(KeyModifiers::CONTROL) {
                        if let KeyCode::Char(c) = kc {
                            Some((c as u8 & 0x1f) as char)
                        } else { None }
                    } else { None };
                }
            } else {
                prefix2_key = None;
                prefix2_raw_char = None;
            }
        }

        // Update status-style from server config (if provided)
        if let Some(ref ss) = state.status_style {
            if !ss.is_empty() {
                let (fg, bg, bold) = parse_tmux_style_components(ss);
                status_fg = fg.unwrap_or(Color::Black);
                status_bg = bg.unwrap_or(Color::Green);
                status_bold = bold;
            }
        }

        // Sync key bindings from server
        if !state.bindings.is_empty() || !synced_bindings.is_empty() {
            synced_bindings = state.bindings;
        }
        // Sync repeat-time from server
        repeat_time_ms = state.repeat_time;
        // Update status-left / status-right from server (already format-expanded)
        if let Some(sl) = state.status_left {
            if !sl.is_empty() {
                // Pass full string — visual truncation is handled by ratatui
                // when rendering into the allocated status bar area.
                // Do NOT naively truncate by char count as that can split
                // inside #[...] style directives, causing parse failures.
                custom_status_left = Some(sl);
            }
        }
        if let Some(sr) = state.status_right {
            if !sr.is_empty() {
                custom_status_right = Some(sr);
            }
        }
        let status_lines = if state.status_visible { state.status_lines } else { 0 };
        // If server's status_lines changed, re-send client-size with the
        // correct content-area height so the server's pane rects match the
        // client's render area exactly.
        let new_sl = (status_lines as u16).max(1);
        if new_sl != last_status_lines {
            last_status_lines = new_sl;
            // Force a client-size re-send on the next iteration
            last_sent_size = (0, 0);
        }
        let status_format = state.status_format;
        // Update pane border styles
        if let Some(ref pbs) = state.pane_border_style {
            if !pbs.is_empty() {
                let (fg, _bg, _bold) = parse_tmux_style_components(pbs);
                if let Some(c) = fg { pane_border_fg = c; }
            }
        }
        if let Some(ref pabs) = state.pane_active_border_style {
            if !pabs.is_empty() {
                let (fg, _bg, _bold) = parse_tmux_style_components(pabs);
                if let Some(c) = fg { pane_active_border_fg = c; }
            }
        }
        // Update window-status-format strings
        if let Some(ref f) = state.wsf { if !f.is_empty() { win_status_fmt = f.clone(); } }
        if let Some(ref f) = state.wscf { if !f.is_empty() { win_status_current_fmt = f.clone(); } }
        if let Some(ref s) = state.wss { win_status_sep = s.clone(); }
        // Update window-status styles
        if let Some(ref s) = state.ws_style {
            if !s.is_empty() {
                win_status_style = Some(parse_tmux_style_components(s));
            }
        }
        if let Some(ref s) = state.wsc_style {
            if !s.is_empty() {
                win_status_current_style = Some(parse_tmux_style_components(s));
            }
        }
        // Update mode-style, status-position, status-justify from server
        if let Some(ref ms) = state.mode_style {
            if !ms.is_empty() { mode_style_str = ms.clone(); }
        }
        if let Some(ref sp) = state.status_position {
            if !sp.is_empty() { status_position_str = sp.clone(); }
        }
        if let Some(ref sj) = state.status_justify {
            if !sj.is_empty() { status_justify_str = sj.clone(); }
        }

        // ── STEP 3: Render ───────────────────────────────────────────────
        let sel_s = rsel_start;
        let sel_e = rsel_end;
        let status_at_top = status_position_str == "top";
        if client_log_enabled() {
            let sz = terminal.size().unwrap_or_default();
            client_log("draw", &format!("pre-draw terminal_size={}x{}", sz.width, sz.height));
        }
        terminal.draw(|f| {
            let area = f.area();
            let constraints = if status_at_top {
                vec![Constraint::Length(status_lines as u16), Constraint::Min(1)]
            } else {
                vec![Constraint::Min(1), Constraint::Length(status_lines as u16)]
            };
            let chunks = Layout::default().direction(Direction::Vertical)
                .constraints(constraints).split(area);
            let (content_chunk, status_chunk) = if status_at_top {
                (chunks[1], chunks[0])
            } else {
                (chunks[0], chunks[1])
            };

            /// Render a large ASCII clock overlay (tmux clock-mode)
            fn render_clock_overlay(f: &mut Frame, area: Rect) {
                // Big digit font (5 rows high, 3 cols wide per digit + colon)
                const DIGITS: [&[&str; 5]; 10] = [
                    &["###", "# #", "# #", "# #", "###"],  // 0
                    &["  #", "  #", "  #", "  #", "  #"],  // 1
                    &["###", "  #", "###", "#  ", "###"],  // 2
                    &["###", "  #", "###", "  #", "###"],  // 3
                    &["# #", "# #", "###", "  #", "  #"],  // 4
                    &["###", "#  ", "###", "  #", "###"],  // 5
                    &["###", "#  ", "###", "# #", "###"],  // 6
                    &["###", "  #", "  #", "  #", "  #"],  // 7
                    &["###", "# #", "###", "# #", "###"],  // 8
                    &["###", "# #", "###", "  #", "###"],  // 9
                ];
                const COLON: [&str; 5] = [" ", "#", " ", "#", " "];
                let now = Local::now();
                let time_str = now.format("%H:%M:%S").to_string();
                // Each char is 3 wide + 1 gap, colon is 1 wide + 1 gap
                let total_w: u16 = time_str.chars().map(|c| if c == ':' { 2 } else { 4 }).sum::<u16>() - 1;
                let total_h: u16 = 5;
                if area.width < total_w || area.height < total_h { return; }
                let start_x = area.x + (area.width.saturating_sub(total_w)) / 2;
                let start_y = area.y + (area.height.saturating_sub(total_h)) / 2;
                // Clear the area
                let clock_area = Rect::new(start_x.saturating_sub(1), start_y, total_w + 2, total_h);
                f.render_widget(Clear, clock_area);
                for row in 0..5u16 {
                    let mut x = start_x;
                    for ch in time_str.chars() {
                        if ch == ':' {
                            let cell_area = Rect::new(x, start_y + row, 1, 1);
                            let s = Span::styled(COLON[row as usize], Style::default().fg(Color::Cyan));
                            f.render_widget(Paragraph::new(Line::from(s)), cell_area);
                            x += 2;
                        } else if let Some(d) = ch.to_digit(10) {
                            let pattern = DIGITS[d as usize][row as usize];
                            let cell_area = Rect::new(x, start_y + row, 3, 1);
                            let s = Span::styled(pattern, Style::default().fg(Color::Cyan));
                            f.render_widget(Paragraph::new(Line::from(s)), cell_area);
                            x += 4;
                        }
                    }
                }
            }

            fn render_json(f: &mut Frame, node: &LayoutJson, area: Rect, dim_preds: bool, border_fg: Color, active_border_fg: Color, clock_mode: bool, active_rect: Option<Rect>, mode_style_str: &str, zoomed: bool) {
                match node {
                    LayoutJson::Leaf {
                        id: _,
                        rows: _,
                        cols: _,
                        cursor_row,
                        cursor_col,
                        alternate_screen,
                        hide_cursor: _,
                        cursor_shape: _,
                        active,
                        copy_mode,
                        scroll_offset,
                        sel_start_row,
                        sel_start_col,
                        sel_end_row,
                        sel_end_col,
                        sel_mode,
                        copy_cursor_row,
                        copy_cursor_col,
                        content,
                        rows_v2,
                    } => {
                        // No borders — content fills entire area (tmux-style)
                        let inner = area;
                        let mut lines: Vec<Line> = Vec::new();
                        let use_full_cells = *copy_mode && *active && !content.is_empty();
                        if use_full_cells || rows_v2.is_empty() {
                            for r in 0..inner.height.min(content.len() as u16) {
                                let mut spans: Vec<Span> = Vec::new();
                                let row = &content[r as usize];
                                let max_c = inner.width.min(row.len() as u16);
                                let mut c: u16 = 0;
                                while c < max_c {
                                    let cell = &row[c as usize];
                                    let mut fg = map_color(&cell.fg);
                                    let bg = map_color(&cell.bg);
                                    let in_selection = if *copy_mode && *active {
                                        if let (Some(sr), Some(sc), Some(er), Some(ec)) = (sel_start_row, sel_start_col, sel_end_row, sel_end_col) {
                                            let mode = sel_mode.as_deref().unwrap_or("char");
                                            match mode {
                                                "rect" => r >= *sr && r <= *er && c >= (*sc).min(*ec) && c <= (*sc).max(*ec),
                                                "line" => r >= *sr && r <= *er,
                                                _ /* char */ => {
                                                    if *sr == *er {
                                                        // Single line
                                                        r == *sr && c >= (*sc).min(*ec) && c <= (*sc).max(*ec)
                                                    } else if r == *sr {
                                                        c >= *sc
                                                    } else if r == *er {
                                                        c <= *ec
                                                    } else {
                                                        r > *sr && r < *er
                                                    }
                                                }
                                            }
                                        } else { false }
                                    } else { false };
                                    if *active && dim_preds && !*alternate_screen
                                        && (r > *cursor_row || (r == *cursor_row && c >= *cursor_col))
                                    {
                                        fg = dim_color(fg);
                                    }
                                    let mut style = Style::default().fg(fg).bg(bg);
                                    if in_selection {
                                        // Apply mode-style from theme/config instead of hardcoded colors
                                        let ms = crate::rendering::parse_tmux_style(&mode_style_str);
                                        style = ms;
                                    }
                                    if cell.inverse { style = style.add_modifier(Modifier::REVERSED); }
                                    if cell.dim { style = style.add_modifier(Modifier::DIM); }
                                    if cell.bold { style = style.add_modifier(Modifier::BOLD); }
                                    if cell.italic { style = style.add_modifier(Modifier::ITALIC); }
                                    if cell.underline { style = style.add_modifier(Modifier::UNDERLINED); }
                                    let text: &str = if cell.text.is_empty() { " " } else { &cell.text };
                                    let char_width = unicode_width::UnicodeWidthStr::width(text) as u16;
                                    spans.push(Span::styled(text, style));
                                    if char_width >= 2 {
                                        c += 2;
                                    } else {
                                        c += 1;
                                    }
                                }
                                // Pad remaining columns so the Line fills the
                                // full pane width — prevents a visible gap when
                                // the server's content width differs from the
                                // client's computed rect (e.g. during resize or
                                // when status_lines > 1).
                                if c < inner.width {
                                    let last_bg = if !spans.is_empty() {
                                        spans.last().unwrap().style.bg.unwrap_or(Color::Reset)
                                    } else { Color::Reset };
                                    let pad = " ".repeat((inner.width - c) as usize);
                                    spans.push(Span::styled(pad, Style::default().bg(last_bg)));
                                }
                                lines.push(Line::from(spans));
                            }
                        } else {
                            for r in 0..inner.height.min(rows_v2.len() as u16) {
                                let mut spans: Vec<Span> = Vec::new();
                                let mut c: u16 = 0;
                                let mut last_bg = Color::Reset;
                                for run in &rows_v2[r as usize].runs {
                                    if c >= inner.width { break; }
                                    let mut fg = map_color(&run.fg);
                                    let bg = map_color(&run.bg);
                                    last_bg = bg;
                                    if *active && dim_preds && !*alternate_screen
                                        && (r > *cursor_row || (r == *cursor_row && c >= *cursor_col))
                                    {
                                        fg = dim_color(fg);
                                    }
                                    let mut style = Style::default().fg(fg).bg(bg);
                                    if run.flags & 16 != 0 { style = style.add_modifier(Modifier::REVERSED); }
                                    if run.flags & 1 != 0 { style = style.add_modifier(Modifier::DIM); }
                                    if run.flags & 2 != 0 { style = style.add_modifier(Modifier::BOLD); }
                                    if run.flags & 4 != 0 { style = style.add_modifier(Modifier::ITALIC); }
                                    if run.flags & 8 != 0 { style = style.add_modifier(Modifier::UNDERLINED); }
                                    if run.flags & 32 != 0 { style = style.add_modifier(Modifier::SLOW_BLINK); }
                                    if run.flags & 64 != 0 { style = style.add_modifier(Modifier::HIDDEN); }
                                    let text: &str = if run.text.is_empty() { " " } else { &run.text };
                                    spans.push(Span::styled(text, style));
                                    c = c.saturating_add(run.width.max(1));
                                }
                                // Pad remaining columns with the last run's bg
                                // so every Line fills the full pane width.
                                if c < inner.width {
                                    let pad = " ".repeat((inner.width - c) as usize);
                                    spans.push(Span::styled(pad, Style::default().bg(last_bg)));
                                }
                                lines.push(Line::from(spans));
                            }
                        }
                        f.render_widget(Clear, inner);
                        let para = Paragraph::new(Text::from(lines));
                        f.render_widget(para, inner);

                        // Copy mode indicator (replaces the old block title "[copy mode]")
                        if *copy_mode && *active {
                            let label = "[copy mode]";
                            let lw = label.len() as u16;
                            if area.width >= lw {
                                let lx = area.x + area.width.saturating_sub(lw);
                                let la = Rect::new(lx, area.y, lw, 1);
                                let ls = Span::styled(label, Style::default().fg(Color::Black).bg(Color::Yellow));
                                f.render_widget(Paragraph::new(Line::from(ls)), la);
                            }
                        }

                        if *copy_mode && *active && *scroll_offset > 0 {
                            let indicator = format!("[{}/{}]", scroll_offset, scroll_offset);
                            let indicator_width = indicator.len() as u16;
                            if area.width > indicator_width + 2 {
                                let indicator_x = area.x + area.width - indicator_width - 1;
                                let indicator_y = if *copy_mode { area.y + 1 } else { area.y };
                                let indicator_area = Rect::new(indicator_x, indicator_y, indicator_width, 1);
                                let indicator_span = Span::styled(indicator, Style::default().fg(Color::Black).bg(Color::Yellow));
                                f.render_widget(Paragraph::new(Line::from(indicator_span)), indicator_area);
                            }
                        }

                        if *active && !*copy_mode {
                            // Clock mode overlay
                            if clock_mode {
                                render_clock_overlay(f, inner);
                            }
                            // Cursor visibility is handled entirely outside
                            // the draw callback — see the post-draw atomic
                            // cursor write below.  We intentionally do NOT
                            // call f.set_cursor_position() here so that
                            // ratatui never emits separate ?25h/?25l flushes
                            // that would create intermediate states visible
                            // to WT between vsync frames.
                        }

                        // In copy mode, show cursor at copy_pos with a
                        // highlighted (reverse-video) cell so the user can see
                        // where the cursor is before starting selection.
                        if *copy_mode && *active {
                            if let (Some(cr), Some(cc)) = (copy_cursor_row, copy_cursor_col) {
                                let cr = (*cr).min(inner.height.saturating_sub(1));
                                let cc = (*cc).min(inner.width.saturating_sub(1));
                                let cy = inner.y + cr;
                                let cx = inner.x + cc;
                                f.set_cursor_position((cx, cy));
                                // Highlight the cursor cell with reverse video
                                let buf = f.buffer_mut();
                                let buf_area = buf.area;
                                if cy >= buf_area.y && cy < buf_area.y + buf_area.height
                                    && cx >= buf_area.x && cx < buf_area.x + buf_area.width
                                {
                                    let idx = (cy - buf_area.y) as usize * buf_area.width as usize
                                        + (cx - buf_area.x) as usize;
                                    if idx < buf.content.len() {
                                        let cell = &mut buf.content[idx];
                                        cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                                    }
                                }
                            }
                        }
                    }
                    LayoutJson::Split { kind, sizes, children } => {
                        let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                            sizes.clone()
                        } else {
                            vec![(100 / children.len().max(1)) as u16; children.len()]
                        };
                        let is_horizontal = kind == "Horizontal";
                        let rects = split_with_gaps(is_horizontal, &effective_sizes, area);

                        // Render children first
                        for (i, child) in children.iter().enumerate() {
                            if i < rects.len() { render_json(f, child, rects[i], dim_preds, border_fg, active_border_fg, clock_mode, active_rect, mode_style_str, zoomed); }
                        }

                        // Draw separator lines between children using direct buffer access.
                        // Skip when zoomed — no visible borders (#82)
                        if zoomed { return; }
                        let border_style = Style::default().fg(border_fg);
                        let active_border_style = Style::default().fg(active_border_fg);
                        let buf = f.buffer_mut();
                        for i in 0..children.len().saturating_sub(1) {
                            if i >= rects.len() { break; }

                            // When both neighbours are direct leaves, use the midpoint
                            // half-highlight so the colored half indicates which side
                            // is active.  For nested splits, use adjacency to the
                            // computed active pane rect so only the correct portion of
                            // the separator is highlighted.
                            let both_leaves = matches!(&children[i], LayoutJson::Leaf { .. })
                                && matches!(children.get(i + 1), Some(LayoutJson::Leaf { .. }));

                            if is_horizontal {
                                // Vertical separator line between left/right children.
                                let sep_x = rects[i].x + rects[i].width;
                                if sep_x < buf.area.x + buf.area.width {
                                    if both_leaves {
                                        let left_active = matches!(&children[i], LayoutJson::Leaf { active, .. } if *active);
                                        let right_active = matches!(children.get(i + 1), Some(LayoutJson::Leaf { active, .. }) if *active);
                                        let left_sty = if left_active { active_border_style } else { border_style };
                                        let right_sty = if right_active { active_border_style } else { border_style };
                                        let mid_y = area.y + area.height / 2;
                                        for y in area.y..area.y + area.height {
                                            let sty = if y < mid_y { left_sty } else { right_sty };
                                            let idx = (y - buf.area.y) as usize * buf.area.width as usize
                                                + (sep_x - buf.area.x) as usize;
                                            if idx < buf.content.len() {
                                                buf.content[idx].set_char('│');
                                                buf.content[idx].set_style(sty);
                                            }
                                        }
                                    } else {
                                        for y in area.y..area.y + area.height {
                                            let active = active_rect.map_or(false, |ar| {
                                                y >= ar.y && y < ar.y + ar.height
                                                && (sep_x == ar.x + ar.width || sep_x + 1 == ar.x)
                                            });
                                            let sty = if active { active_border_style } else { border_style };
                                            let idx = (y - buf.area.y) as usize * buf.area.width as usize
                                                + (sep_x - buf.area.x) as usize;
                                            if idx < buf.content.len() {
                                                buf.content[idx].set_char('│');
                                                buf.content[idx].set_style(sty);
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Horizontal separator line between top/bottom children.
                                let sep_y = rects[i].y + rects[i].height;
                                if sep_y < buf.area.y + buf.area.height {
                                    if both_leaves {
                                        let top_active = matches!(&children[i], LayoutJson::Leaf { active, .. } if *active);
                                        let bot_active = matches!(children.get(i + 1), Some(LayoutJson::Leaf { active, .. }) if *active);
                                        let top_sty = if top_active { active_border_style } else { border_style };
                                        let bot_sty = if bot_active { active_border_style } else { border_style };
                                        let mid_x = area.x + area.width / 2;
                                        for x in area.x..area.x + area.width {
                                            let sty = if x < mid_x { top_sty } else { bot_sty };
                                            let idx = (sep_y - buf.area.y) as usize * buf.area.width as usize
                                                + (x - buf.area.x) as usize;
                                            if idx < buf.content.len() {
                                                buf.content[idx].set_char('─');
                                                buf.content[idx].set_style(sty);
                                            }
                                        }
                                    } else {
                                        for x in area.x..area.x + area.width {
                                            let active = active_rect.map_or(false, |ar| {
                                                x >= ar.x && x < ar.x + ar.width
                                                && (sep_y == ar.y + ar.height || sep_y + 1 == ar.y)
                                            });
                                            let sty = if active { active_border_style } else { border_style };
                                            let idx = (sep_y - buf.area.y) as usize * buf.area.width as usize
                                                + (x - buf.area.x) as usize;
                                            if idx < buf.content.len() {
                                                buf.content[idx].set_char('─');
                                                buf.content[idx].set_style(sty);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let active_rect = compute_active_rect_json(&root, content_chunk);
            render_json(f, &root, content_chunk, dim_preds, pane_border_fg, pane_active_border_fg, clock_active, active_rect, &mode_style_str, state.zoomed);
            fix_border_intersections(f.buffer_mut());

            // ── Left-click drag text selection overlay ────────────────
            // Suppress the client-side blue selection overlay when the
            // server is in copy mode – the server draws its own themed
            // selection and the blue overlay would hide everything.
            if let (Some(s), Some(e)) = (sel_s, sel_e) {
            if !active_pane_in_copy_mode(&root) {
                // Normalise so (r0,c0) <= (r1,c1) in reading order
                let (r0, c0, r1, c1) = if (s.1, s.0) <= (e.1, e.0) {
                    (s.1, s.0, e.1, e.0)
                } else {
                    (e.1, e.0, s.1, s.0)
                };
                let buf = f.buffer_mut();
                let buf_area = buf.area;
                for row in r0..=r1 {
                    let col_start = if row == r0 { c0 } else { 0 };
                    let col_end = if row == r1 { c1 } else { area.width.saturating_sub(1) };
                    for col in col_start..=col_end {
                        if row < buf_area.height && col < buf_area.width {
                            let idx = (row - buf_area.y) as usize * buf_area.width as usize
                                + (col - buf_area.x) as usize;
                            if idx < buf.content.len() {
                                buf.content[idx].set_style(Style::default().fg(Color::Black).bg(Color::LightCyan));
                            }
                        }
                    }
                }
            } // !active_pane_in_copy_mode
            } // if let sel_s, sel_e

            if session_chooser {
                let sel_style = crate::rendering::parse_tmux_style(&mode_style_str);
                let overlay = Block::default().borders(Borders::ALL).title("choose-session (enter=switch, x=kill, esc=close)").border_style(sel_style);
                let oa = centered_rect(70, 20, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let mut lines: Vec<Line> = Vec::new();
                for (i, (sname, info)) in session_entries.iter().enumerate() {
                    let marker = if sname == &current_session { "*" } else { " " };
                    let line = if i == session_selected {
                        Line::from(Span::styled(format!("{} {}", marker, info), sel_style))
                    } else {
                        Line::from(format!("{} {}", marker, info))
                    };
                    lines.push(line);
                }
                let para = Paragraph::new(Text::from(lines));
                f.render_widget(para, overlay.inner(oa));
            }
            if tree_chooser {
                let sel_style = crate::rendering::parse_tmux_style(&mode_style_str);
                let overlay = Block::default().borders(Borders::ALL).title("choose-tree").border_style(sel_style);
                // Use dynamic height: content lines + 2 (borders), capped to
                // available space so the overlay never exceeds the terminal.
                let tree_h = ((tree_entries.len() as u16).saturating_add(2))
                    .max(5)
                    .min(content_chunk.height.saturating_sub(2));
                let oa = centered_rect(60, tree_h, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let inner = overlay.inner(oa);
                let visible_h = inner.height as usize;
                // Keep tree_selected in view
                if tree_selected >= tree_scroll + visible_h {
                    tree_scroll = tree_selected.saturating_sub(visible_h - 1);
                }
                if tree_selected < tree_scroll {
                    tree_scroll = tree_selected;
                }
                let mut lines: Vec<Line> = Vec::new();
                for (i, (is_win, wid, _pid, label, _sess)) in tree_entries.iter().enumerate().skip(tree_scroll).take(visible_h) {
                    let line = if i == tree_selected {
                        Line::from(Span::styled(label.clone(), sel_style))
                    } else if *is_win && *wid == usize::MAX {
                        // Session header — bold
                        Line::from(Span::styled(label.clone(), Style::default().add_modifier(Modifier::BOLD)))
                    } else {
                        Line::from(label.clone())
                    };
                    lines.push(line);
                }
                let para = Paragraph::new(Text::from(lines));
                f.render_widget(para, inner);
            }
            if keys_viewer {
                // Proportional overlay: 90% width, up to 80% height
                let avail_h = content_chunk.height;
                let overlay_h = (avail_h * 80 / 100).max(5).min(avail_h.saturating_sub(2));
                let overlay = Block::default().borders(Borders::ALL)
                    .title(" list-keys (q/Esc=close, Up/Down/PgUp/PgDn=scroll) ");
                let oa = centered_rect(90, overlay_h, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let inner = overlay.inner(oa);
                let visible_h = inner.height as usize;
                // Clamp scroll so we don't scroll past the end
                let max_scroll = keys_viewer_lines.len().saturating_sub(visible_h);
                if keys_viewer_scroll > max_scroll { keys_viewer_scroll = max_scroll; }
                let mut lines: Vec<Line> = Vec::new();
                for (_i, entry) in keys_viewer_lines.iter().enumerate().skip(keys_viewer_scroll).take(visible_h) {
                    // Highlight section headers, "bind-key" keyword, and plain text differently
                    if entry.starts_with("──") || entry.starts_with("── ") {
                        lines.push(Line::from(Span::styled(entry.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
                    } else if let Some(rest) = entry.strip_prefix("bind-key") {
                        lines.push(Line::from(vec![
                            Span::styled("bind-key", Style::default().fg(Color::Green)),
                            Span::raw(rest.to_string()),
                        ]));
                    } else {
                        lines.push(Line::from(entry.clone()));
                    }
                }
                // Show scroll indicator in bottom-right
                let para = Paragraph::new(Text::from(lines));
                f.render_widget(para, inner);
                // Scroll position indicator
                if keys_viewer_lines.len() > visible_h {
                    let pct = if max_scroll == 0 { 100 } else { keys_viewer_scroll * 100 / max_scroll };
                    let indicator = if keys_viewer_scroll == 0 {
                        "Top".to_string()
                    } else if keys_viewer_scroll >= max_scroll {
                        "Bot".to_string()
                    } else {
                        format!("{}%", pct)
                    };
                    let ind_len = indicator.len() as u16;
                    if oa.width > ind_len + 2 {
                        let ind_x = oa.x + oa.width - ind_len - 2;
                        let ind_y = oa.y + oa.height - 1;
                        let ind_rect = Rect::new(ind_x, ind_y, ind_len, 1);
                        let ind_para = Paragraph::new(Span::styled(indicator, Style::default().fg(Color::DarkGray)));
                        f.render_widget(ind_para, ind_rect);
                    }
                }
            }
            let sb_fg = status_fg;
            let sb_bg = status_bg;
            let sb_base = if status_bold {
                Style::default().fg(sb_fg).bg(sb_bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(sb_fg).bg(sb_bg)
            };
            // ── Build three separate span groups: left, tabs, right ──
            use unicode_width::UnicodeWidthStr;
            // Left portion: custom status_left or default [session] prefix
            let left_prefix = match custom_status_left {
                Some(ref sl) => sl.clone(),
                None => format!("[{}] ", name),
            };
            if client_log_enabled() {
                client_log("status", &format!("parsing left_prefix ({} chars): [{}]",
                    left_prefix.len(), left_prefix.chars().take(100).collect::<String>()));
            }
            let left_spans: Vec<Span> = crate::rendering::parse_inline_styles(&left_prefix, sb_base);

            // Window tabs (the window list)
            let mut tab_spans_all: Vec<Span> = Vec::new();
            for (i, w) in windows.iter().enumerate() {
                let tab_text = if !w.tab_text.is_empty() {
                    w.tab_text.clone()
                } else {
                    let display_idx = i + base_index;
                    let fmt = if w.active { &win_status_current_fmt } else { &win_status_fmt };
                    fmt.replace("#I", &display_idx.to_string())
                       .replace("#W", &w.name)
                       .replace("#F", if w.active { "*" } else { "" })
                };
                if i > 0 {
                    // Parse inline styles in separator (e.g. "#[fg=#44475a]|")
                    let sep_spans = crate::rendering::parse_inline_styles(&win_status_sep, sb_base);
                    tab_spans_all.extend(sep_spans);
                }
                let fallback_style = if w.active {
                    if let Some((fg, bg, bold)) = win_status_current_style {
                        let mut s = Style::default();
                        if let Some(c) = fg { s = s.fg(c); }
                        if let Some(c) = bg { s = s.bg(c); }
                        if bold { s = s.add_modifier(Modifier::BOLD); }
                        s
                    } else {
                        sb_base
                    }
                } else if w.activity {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    if let Some((fg, bg, bold)) = win_status_style {
                        let mut s = Style::default();
                        if let Some(c) = fg { s = s.fg(c); }
                        if let Some(c) = bg { s = s.bg(c); }
                        if bold { s = s.add_modifier(Modifier::BOLD); }
                        s
                    } else {
                        sb_base
                    }
                };
                let parsed = crate::rendering::parse_inline_styles(&tab_text, fallback_style);
                tab_spans_all.extend(parsed);
            }

            // Right portion
            let right_text = custom_status_right.as_deref().unwrap_or("").to_string();
            if client_log_enabled() {
                client_log("status", &format!("parsing right_text ({} chars): [{}]",
                    right_text.len(), right_text.chars().take(100).collect::<String>()));
            }
            let right_spans = crate::rendering::parse_inline_styles(&right_text, sb_base);

            // Measure widths using Unicode display width
            let left_w: usize = left_spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
            let tabs_w: usize = tab_spans_all.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
            let right_w: usize = right_spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
            let total_width = status_chunk.width as usize;

            // Assemble final spans based on status-justify
            let mut status_spans: Vec<Span> = Vec::new();
            match status_justify_str.as_str() {
                "centre" | "center" => {
                    // Centre: [left] [pad1] [tabs] [pad2] [right]
                    // Tabs are centred in the space between left and right.
                    let avail = total_width.saturating_sub(left_w).saturating_sub(right_w);
                    let pad_before = avail.saturating_sub(tabs_w) / 2;
                    let pad_after = avail.saturating_sub(tabs_w).saturating_sub(pad_before);
                    status_spans.extend(left_spans);
                    if pad_before > 0 { status_spans.push(Span::styled(" ".repeat(pad_before), sb_base)); }
                    status_spans.extend(tab_spans_all);
                    if pad_after > 0 { status_spans.push(Span::styled(" ".repeat(pad_after), sb_base)); }
                    status_spans.extend(right_spans);
                }
                "absolute-centre" | "absolute-center" => {
                    // Absolute-centre: tabs centred on the total terminal width
                    let tabs_start = total_width.saturating_sub(tabs_w) / 2;
                    status_spans.extend(left_spans);
                    let pad_before = tabs_start.saturating_sub(left_w);
                    if pad_before > 0 { status_spans.push(Span::styled(" ".repeat(pad_before), sb_base)); }
                    status_spans.extend(tab_spans_all);
                    let used = left_w + pad_before + tabs_w;
                    let pad_after = total_width.saturating_sub(used).saturating_sub(right_w);
                    if pad_after > 0 { status_spans.push(Span::styled(" ".repeat(pad_after), sb_base)); }
                    status_spans.extend(right_spans);
                }
                "right" => {
                    // Right: [left] [pad] [tabs] [right]
                    status_spans.extend(left_spans);
                    let used = left_w + tabs_w + right_w;
                    let pad = total_width.saturating_sub(used);
                    if pad > 0 { status_spans.push(Span::styled(" ".repeat(pad), sb_base)); }
                    status_spans.extend(tab_spans_all);
                    status_spans.extend(right_spans);
                }
                _ => {
                    // Left (default): [left] [tabs] [pad] [right]
                    status_spans.extend(left_spans);
                    status_spans.extend(tab_spans_all);
                    let used = left_w + tabs_w + right_w;
                    let pad = total_width.saturating_sub(used);
                    if pad > 0 { status_spans.push(Span::styled(" ".repeat(pad), sb_base)); }
                    status_spans.extend(right_spans);
                }
            }
            // If a display-message is active, show it on the status bar
            // instead of the normal status content (tmux parity).
            // Uses message-style (default: bg=yellow,fg=black) matching tmux.
            let status_bar = if let Some(ref msg) = state.status_message {
                let msg_style = crate::rendering::parse_tmux_style("bg=yellow,fg=black");
                let padded = if msg.len() < status_chunk.width as usize {
                    format!("{}{}", msg, " ".repeat(status_chunk.width as usize - msg.len()))
                } else {
                    msg.chars().take(status_chunk.width as usize).collect()
                };
                Paragraph::new(Line::from(Span::styled(padded, msg_style))).style(msg_style)
            } else {
                Paragraph::new(Line::from(status_spans)).style(sb_base)
            };
            f.render_widget(Clear, status_chunk);
            // Render the first status line (line 0)
            let line0_area = Rect { x: status_chunk.x, y: status_chunk.y, width: status_chunk.width, height: 1.min(status_chunk.height) };
            f.render_widget(status_bar, line0_area);
            // Render additional status lines (index 1+) from status_format
            for line_idx in 1..status_lines {
                let line_y = status_chunk.y + line_idx as u16;
                if line_y >= status_chunk.y + status_chunk.height { break; }
                let line_area = Rect { x: status_chunk.x, y: line_y, width: status_chunk.width, height: 1 };
                let text = if line_idx < status_format.len() && !status_format[line_idx].is_empty() {
                    status_format[line_idx].clone()
                } else {
                    String::new()
                };
                // Pad to full width
                let padded: String = if text.len() < line_area.width as usize {
                    format!("{}{}", text, " ".repeat(line_area.width as usize - text.len()))
                } else {
                    text.chars().take(line_area.width as usize).collect()
                };
                let line_widget = Paragraph::new(Line::from(Span::styled(padded, sb_base))).style(sb_base);
                f.render_widget(line_widget, line_area);
            }
            if renaming {
                let overlay = Block::default().borders(Borders::ALL).title("rename window");
                let oa = centered_rect(60, 3, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let para = Paragraph::new(format!("name: {}", rename_buf));
                f.render_widget(para, overlay.inner(oa));
            }
            if pane_renaming {
                let overlay = Block::default().borders(Borders::ALL).title("set pane title");
                let oa = centered_rect(60, 3, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let para = Paragraph::new(format!("title: {}", pane_title_buf));
                f.render_widget(para, overlay.inner(oa));
            }
            if command_input {
                let overlay = Block::default().borders(Borders::ALL).title("command");
                let oa = centered_rect(60, 3, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let para = Paragraph::new(format!(": {}", command_buf));
                f.render_widget(para, overlay.inner(oa));
            }
            if let Some(ref cmd) = confirm_cmd {
                let overlay = Block::default().borders(Borders::ALL).title("confirm");
                let oa = centered_rect(50, 3, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let para = Paragraph::new(format!("{}? (y/n)", cmd));
                f.render_widget(para, overlay.inner(oa));
            }

            // ── Server-side overlay rendering ────────────────────────
            if srv_popup_active {
                let w = srv_popup_width.min(content_chunk.width.saturating_sub(2));
                let h = srv_popup_height.min(content_chunk.height.saturating_sub(2));
                let popup_area = Rect {
                    x: content_chunk.x + (content_chunk.width.saturating_sub(w)) / 2,
                    y: content_chunk.y + (content_chunk.height.saturating_sub(h)) / 2,
                    width: w,
                    height: h,
                };
                let title = if srv_popup_command.is_empty() { "Popup".to_string() } else { let max_title = (w as usize).saturating_sub(4); if srv_popup_command.len() > max_title { format!("{}...", &srv_popup_command[..max_title.saturating_sub(3)]) } else { srv_popup_command.clone() } };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(title);
                let mut lines: Vec<Line<'static>> = Vec::new();
                for line_str in &srv_popup_lines {
                    lines.push(Line::from(line_str.clone()));
                }
                let para = Paragraph::new(Text::from(lines)).block(block);
                f.render_widget(Clear, popup_area);
                f.render_widget(para, popup_area);
            }
            if srv_confirm_active {
                let overlay = Block::default().borders(Borders::ALL).title("confirm");
                let oa = centered_rect(60, 3, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let para = Paragraph::new(srv_confirm_prompt.clone());
                f.render_widget(para, overlay.inner(oa));
            }
            if srv_menu_active {
                let sel_style = crate::rendering::parse_tmux_style(&mode_style_str);
                let title_str = if srv_menu_title.is_empty() { "Menu".to_string() } else { srv_menu_title.clone() };
                let overlay = Block::default().borders(Borders::ALL).title(title_str).border_style(sel_style);
                let item_count = srv_menu_items.len();
                let menu_h = ((item_count as u16).saturating_add(2)).max(3).min(content_chunk.height.saturating_sub(2));
                let oa = centered_rect(50, menu_h, content_chunk);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let inner = overlay.inner(oa);
                let mut lines: Vec<Line<'static>> = Vec::new();
                for (i, item) in srv_menu_items.iter().enumerate() {
                    if item.sep {
                        lines.push(Line::from("─".repeat(inner.width as usize)));
                    } else {
                        let name = item.name.clone().unwrap_or_default();
                        let key_str = item.key.clone().unwrap_or_default();
                        let label = if key_str.is_empty() { name } else { format!("{} ({})", name, key_str) };
                        if i == srv_menu_selected {
                            lines.push(Line::from(Span::styled(label, sel_style)));
                        } else {
                            lines.push(Line::from(label));
                        }
                    }
                }
                let para = Paragraph::new(Text::from(lines));
                f.render_widget(para, inner);
            }
            if srv_display_panes {
                // Render pane numbers overlay (like tmux display-panes)
                fn collect_leaf_rects(node: &LayoutJson, area: Rect, out: &mut Vec<Rect>) {
                    match node {
                        LayoutJson::Leaf { .. } => { out.push(area); }
                        LayoutJson::Split { kind, sizes, children } => {
                            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                                sizes.clone()
                            } else {
                                vec![(100 / children.len().max(1)) as u16; children.len()]
                            };
                            let is_horizontal = kind == "Horizontal";
                            let rects = crate::tree::split_with_gaps(is_horizontal, &effective_sizes, area);
                            for (i, child) in children.iter().enumerate() {
                                if i < rects.len() { collect_leaf_rects(child, rects[i], out); }
                            }
                        }
                    }
                }
                let mut leaf_rects = Vec::new();
                collect_leaf_rects(&root, content_chunk, &mut leaf_rects);
                for (idx, prect) in leaf_rects.iter().enumerate() {
                    if prect.width >= 7 && prect.height >= 3 {
                        let bw = 7u16; let bh = 3u16;
                        let bx = prect.x + prect.width.saturating_sub(bw) / 2;
                        let by = prect.y + prect.height.saturating_sub(bh) / 2;
                        let b = Rect { x: bx, y: by, width: bw, height: bh };
                        let pane_sel_style = Style::default().fg(Color::Yellow).bg(Color::Black).add_modifier(Modifier::BOLD);
                        let block = Block::default().borders(Borders::ALL).style(pane_sel_style);
                        let inner = block.inner(b);
                        let disp = ((idx + srv_pane_base_index) % 10).to_string();
                        let para = Paragraph::new(Line::from(Span::styled(
                            format!(" {} ", disp),
                            pane_sel_style,
                        ))).alignment(Alignment::Center);
                        f.render_widget(Clear, b);
                        f.render_widget(block, b);
                        f.render_widget(para, inner);
                    }
                }
            }

        })?;
        if client_log_enabled() {
            client_log("draw", &format!("draw OK, render={}us overlays: popup={} confirm={} menu={} display_panes={}",
                _t_parse.elapsed().as_micros().saturating_sub(_parse_us as u128),
                srv_popup_active, srv_confirm_active, srv_menu_active, srv_display_panes
            ));
        }

        // ── Post-draw: emit buffered OSC 52 clipboard ────────────────
        // Written AFTER terminal.draw() so it doesn't interfere with
        // ratatui's VT output buffer.
        if let Some(clip_text) = pending_osc52.take() {
            crate::copy_mode::emit_osc52(&mut std::io::stdout(), &clip_text);
        }

        // ── SSH: periodic mouse-enable refresh ───────────────────────
        // ConPTY or terminal resize can silently disable mouse reporting.
        // Re-send every 30 seconds to keep mouse working reliably.
        if is_ssh_mode && last_mouse_enable.elapsed().as_secs() >= 30 {
            crate::ssh_input::send_mouse_enable();
            last_mouse_enable = Instant::now();
        }

        // ── Post-draw: atomic cursor write ──────────────────────────
        // Write cursor visibility + position + style as ONE batch to
        // avoid the separate execute!() flushes that ratatui's normal
        // show_cursor()/set_cursor_position() would produce.  Multiple
        // separate console writes create intermediate states visible
        // to WT between vsync frames, causing rapid cursor flicker.
        {
            use std::io::Write;
            fn find_active_cursor_shape(node: &LayoutJson) -> Option<u8> {
                match node {
                    LayoutJson::Leaf { active, cursor_shape, .. } => {
                        if *active && *cursor_shape >= 1 && *cursor_shape <= 6 { Some(*cursor_shape) } else { None }
                    }
                    LayoutJson::Split { children, .. } => {
                        children.iter().find_map(find_active_cursor_shape)
                    }
                }
            }
            let effective = find_active_cursor_shape(&root)
                .unwrap_or_else(|| state_cursor_style_code.unwrap_or_else(crate::rendering::configured_cursor_code));
            // Compute the active pane's screen Rect so we can translate
            // pane-local cursor coords to terminal-global coords.
            fn find_active_rect(node: &LayoutJson, area: Rect) -> Option<Rect> {
                match node {
                    LayoutJson::Leaf { active, .. } => {
                        if *active { Some(area) } else { None }
                    }
                    LayoutJson::Split { kind, sizes, children } => {
                        let eff: Vec<u16> = if sizes.len() == children.len() {
                            sizes.clone()
                        } else {
                            vec![(100 / children.len().max(1)) as u16; children.len()]
                        };
                        let rects = crate::tree::split_with_gaps(kind == "Horizontal", &eff, area);
                        for (i, child) in children.iter().enumerate() {
                            if i < rects.len() {
                                if let Some(r) = find_active_rect(child, rects[i]) { return Some(r); }
                            }
                        }
                        None
                    }
                }
            }
            let active_pane_area: Option<Rect> = {
                let sz = terminal.size().unwrap_or_default();
                let constraints = if status_at_top {
                    vec![Constraint::Length(status_lines as u16), Constraint::Min(1)]
                } else {
                    vec![Constraint::Min(1), Constraint::Length(status_lines as u16)]
                };
                let chunks = Layout::default().direction(Direction::Vertical)
                    .constraints(constraints).split(sz.into());
                let content_chunk = if status_at_top { chunks[1] } else { chunks[0] };
                find_active_rect(&root, content_chunk)
            };
            // Compute screen-global cursor position from pane-local coords.
            let cursor_visible = if let (Some((cc, cr)), Some(inner)) = (post_draw_cursor, active_pane_area) {
                let cy = inner.y + cr.min(inner.height.saturating_sub(1));
                let cx = inner.x + cc.min(inner.width.saturating_sub(1));
                Some((cx, cy))
            } else {
                None
            };
            // Build a single VT string with: ?25h + CUP + DECSCUSR
            // ratatui's draw() always emits ?25l (since we never call
            // f.set_cursor_position), so we must re-emit ?25h + CUP
            // every frame when the cursor should be visible.
            let mut buf = String::with_capacity(32);
            if let Some((cx, cy)) = cursor_visible {
                buf.push_str("\x1b[?25h");
                use std::fmt::Write as FmtWrite;
                let _ = write!(buf, "\x1b[{};{}H", cy + 1, cx + 1);
            }
            // DECSCUSR only when style actually changes (avoids blink
            // timer resets in WT).
            if effective != last_cursor_style {
                last_cursor_style = effective;
                use std::fmt::Write as FmtWrite;
                let _ = write!(buf, "\x1b[{} q", effective);
            }
            if !buf.is_empty() {
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(buf.as_bytes());
                let _ = out.flush();
            }

            // Update Win32 system caret for accessibility / speech-to-text
            // tools (e.g. Wispr Flow).  Skip for SSH sessions — no local
            // console window.
            if !is_ssh_mode {
                if let Some((cx, cy)) = cursor_visible {
                    crate::platform::caret::update(cx, cy);
                }
            }
        }

        let _render_us = _t_parse.elapsed().as_micros().saturating_sub(_parse_us as u128);
        last_dump_time = Instant::now();
        // Latency log: measure full cycle from key-send to render-complete
        if let (Some(ref mut log), Some(ks)) = (&mut latency_log, key_send_instant) {
            let elapsed_ms = ks.elapsed().as_millis();
            loop_count += 1;
            use std::io::Write;
            let _ = writeln!(log, "L{}: key->render {}ms  parse={}us  render={}us  json_len={}  since_dump={}",
                loop_count, elapsed_ms, _parse_us, _render_us, dump_buf.len(), since_dump);
            // Only clear after we rendered a DIFFERENT frame (echo arrived)
            if got_frame && dump_buf != prev_dump_buf {
                let _ = writeln!(log, "L{}: ECHO VISIBLE after {}ms  (parse={}us render={}us)",
                    loop_count, elapsed_ms, _parse_us, _render_us);
                key_send_instant = None;
            }
        }
        selection_changed = false;
        // Cache this frame so we can skip identical re-renders.
        // Only update cache when we got a genuinely new frame (not selection-only redraw)
        if got_frame && dump_buf != prev_dump_buf {
            std::mem::swap(&mut prev_dump_buf, &mut dump_buf);
        }
        // DON'T clear last_key_send_time — keep fast-dumping for 100ms
        // after last keystroke so we catch the ConPTY echo promptly.
        // The timer expires naturally in the poll_ms calculation above.
        // Clear key_send_instant once echo arrives (frame differs).
        if got_frame && dump_buf != prev_dump_buf {
            key_send_instant = None;
        }
        force_dump = false;
    }

    // Clean disconnect on persistent connection
    let _ = writer.write_all(b"client-detach\n");
    let _ = writer.flush();
    Ok(())
}

/// Flush the paste-pending buffer as individual send-text / send-key commands.
/// Called when a non-bufferable key (Backspace, Delete, Esc, BackTab) interrupts
/// a potential paste burst, so we emit whatever we had as normal keystrokes.
#[cfg(windows)]
fn flush_paste_pend_as_text(
    paste_pend: &mut String,
    paste_pend_start: &mut Option<Instant>,
    paste_stage2: &mut bool,
    cmd_batch: &mut Vec<String>,
) {
    if paste_pend.is_empty() {
        return;
    }
    // If we accumulated enough ASCII chars that stage2 was entered, this
    // is almost certainly pasted content — send as send-paste so the server
    // wraps it in bracketed paste sequences (fixes nvim autoindent).
    // Non-ASCII buffers (IME input) are always flushed as normal text to
    // avoid the 300ms delay (fixes #91).
    let has_non_ascii = paste_pend.chars().any(|c| !c.is_ascii());
    if (*paste_stage2 || paste_pend.len() >= 3) && !has_non_ascii {
        let encoded = crate::util::base64_encode(paste_pend);
        cmd_batch.push(format!("send-paste {}\n", encoded));
    } else {
        for c in paste_pend.chars() {
            match c {
                '\n' => { cmd_batch.push("send-key enter\n".into()); }
                '\t' => { cmd_batch.push("send-key tab\n".into()); }
                ' '  => { cmd_batch.push("send-key space\n".into()); }
                _ => {
                    let escaped = match c {
                        '"' => "\\\"".to_string(),
                        '\\' => "\\\\".to_string(),
                        _ => c.to_string(),
                    };
                    cmd_batch.push(format!("send-text \"{}\"\n", escaped));
                }
            }
        }
    }
    paste_pend.clear();
    *paste_pend_start = None;
    *paste_stage2 = false;
}

/// Returns true if the buffer contains any non-ASCII characters (IME / CJK input).
/// Used by the paste detection heuristic to skip Stage 2 for IME input (fixes #91).
#[cfg(windows)]
fn paste_buffer_has_non_ascii(buf: &str) -> bool {
    buf.chars().any(|c| !c.is_ascii())
}

#[cfg(test)]
#[path = "../tests-rs/test_client.rs"]
mod tests;
