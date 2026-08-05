use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;
use std::net::TcpStream;

use crate::types::{CtrlReq, LayoutKind, WaitForOp, ControlNotification, ControlOutput};
use crate::cli::{parse_target, extract_flag_value};
use crate::util::base64_decode;
use crate::control;

/// Append-only AUTH diagnostics, gated by PSMUX_AUTH_DEBUG=1. Written to
/// %TEMP%\psmux_auth_debug.log so concurrent processes never truncate each
/// other (issue #496 forensics).
fn auth_debug(msg: &str) {
    if std::env::var("PSMUX_AUTH_DEBUG").map(|v| v == "1").unwrap_or(false) {
        let tmp = std::env::var("TEMP")
            .or_else(|_| std::env::var("TMP"))
            .unwrap_or_else(|_| ".".to_string());
        let path = format!("{}\\psmux_auth_debug.log", tmp);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = std::io::Write::write_all(
                &mut f,
                format!("[{} pid={}] {}\n", ts, std::process::id(), msg).as_bytes(),
            );
        }
    }
}
use crate::commands::parse_command_line;
use super::helpers::TMUX_COMMANDS;


#[derive(Debug, PartialEq)]
enum ControlCommandResponse {
    Success(String),
    Error(String),
}

impl ControlCommandResponse {
    fn empty() -> Self {
        Self::Success(String::new())
    }

    fn success(body: impl Into<String>) -> Self {
        Self::Success(body.into())
    }

    fn error(body: impl std::fmt::Display) -> Self {
        Self::Error(body.to_string())
    }
}

struct ControlConnectionGuard {
    client_id: u64,
    request_tx: mpsc::Sender<CtrlReq>,
}

impl Drop for ControlConnectionGuard {
    fn drop(&mut self) {
        // Deregistration drops the server's output sender. The writer then
        // drains queued command blocks before it closes the shared socket.
        let _ = self.request_tx.send(CtrlReq::ControlDeregister {
            client_id: self.client_id,
        });
    }
}

struct ControlWriterGuard {
    client_id: u64,
    shutdown: TcpStream,
}

impl Drop for ControlWriterGuard {
    fn drop(&mut self) {
        crate::types::deregister_persistent_stream(self.client_id);
        let _ = self.shutdown.shutdown(std::net::Shutdown::Both);
    }
}

fn write_control_output(stream: &mut TcpStream, output: ControlOutput) -> io::Result<bool> {
    let (text, exit) = match output {
        ControlOutput::Notification(notification) => {
            let exit = matches!(notification, ControlNotification::Exit { .. });
            (format!("{}\n", control::format_notification(&notification)), exit)
        }
        ControlOutput::CommandBlock(block) => (block, false),
    };
    stream.write_all(text.as_bytes())?;
    stream.flush()?;
    Ok(exit)
}

fn format_control_command_block(
    echo: bool,
    command: &str,
    timestamp: i64,
    command_number: u64,
    response: Option<Result<ControlCommandResponse, mpsc::RecvError>>,
) -> String {
    let mut block = String::new();
    if echo {
        block.push_str(command);
        block.push('\n');
    }
    block.push_str(&control::format_begin(timestamp, command_number));
    block.push('\n');
    match response {
        Some(Ok(response)) => {
            let (is_error, body) = match response {
                ControlCommandResponse::Success(body) => (false, body),
                ControlCommandResponse::Error(body) => (true, body),
            };
            if !body.is_empty() {
                block.push_str(&body);
                if !body.ends_with('\n') {
                    block.push('\n');
                }
            }
            block.push_str(&if is_error {
                control::format_error(timestamp, command_number)
            } else {
                control::format_end(timestamp, command_number)
            });
        }
        Some(Err(_)) => {
            block.push_str("command response unavailable\n");
            block.push_str(&control::format_error(timestamp, command_number));
        }
        None => block.push_str(&control::format_end(timestamp, command_number)),
    }
    block.push('\n');
    block
}

/// Split a command line on top-level `;` separators, respecting single and
/// double quotes and `\` escapes. Real tmux's parser treats `;` as a command
/// separator on the same line; iTerm2's `sendCommandList` joins many commands
/// with "; " into one wire line and expects one %begin/%end pair per command.
fn split_top_level_semicolons(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && !in_single {
            // Escape: copy the backslash and the next char (if any) verbatim.
            cur.push(c);
            if let Some(nc) = chars.next() { cur.push(nc); }
            continue;
        }
        match c {
            '\'' if !in_double => { in_single = !in_single; cur.push(c); }
            '"'  if !in_single => { in_double = !in_double; cur.push(c); }
            ';'  if !in_single && !in_double => {
                let trimmed = cur.trim().to_string();
                if !trimmed.is_empty() { out.push(trimmed); }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let trimmed = cur.trim().to_string();
    if !trimmed.is_empty() { out.push(trimmed); }
    out
}

/// Try to decode a single `send`/`send-keys` command into the literal byte
/// payload it would inject and the pane target.  Returns `None` if the
/// command uses features we don't safely coalesce (e.g. `-X`, `-p`, `-N`,
/// or named keys like `Up`/`Tab`) — in that case the caller falls back to
/// normal per-command dispatch.
///
/// This is used to merge consecutive `send` sub-commands within one input
/// line into a single PTY write.  iTerm2 sends arrow keys as
/// `send -t %1 0x1b 0x5b; send -lt %1 A` — two separate sub-commands.  If
/// each becomes its own PTY write, pwsh's PSReadLine times out between the
/// ESC byte and the `[A` and emits them as literal characters.  Coalescing
/// guarantees the whole VT sequence reaches the shell in one read().
fn decode_send_command(line: &str) -> Option<(String, Vec<u8>)> {
    let toks = parse_command_line(line);
    if toks.is_empty() { return None; }
    let cmd = toks[0].as_str();
    if cmd != "send" && cmd != "send-keys" { return None; }
    let args: Vec<&str> = toks[1..].iter().map(|s| s.as_str()).collect();

    // Bail on modes that require special semantics.
    let any_short = |c: char| {
        args.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.chars().skip(1).any(|fc| fc == c))
    };
    if any_short('X') || any_short('p') || any_short('N') || any_short('R') { return None; }

    let prev_consumes_operand = |i: usize| -> bool {
        if i == 0 { return false; }
        if let Some(prev) = args.get(i - 1) {
            if prev.starts_with('-') && !prev.starts_with("--") && prev.len() >= 2 {
                if let Some(last) = prev.chars().last() {
                    return matches!(last, 't' | 'T' | 'N' | 'R' | 'c');
                }
            }
        }
        false
    };

    // Find target (-t / -lt /...).  Default to %active if absent.
    let mut target: Option<String> = None;
    for (i, a) in args.iter().enumerate() {
        if a.starts_with('-') && !a.starts_with("--") && a.ends_with('t') {
            if let Some(t) = args.get(i + 1) { target = Some((*t).to_string()); break; }
        }
    }

    let literal = any_short('l');
    // -H marks every operand as one raw hexadecimal byte (tmux KEYC_LITERAL).
    let literal_byte = any_short('H');
    let mut bytes: Vec<u8> = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a.starts_with('-') { continue; }
        if prev_consumes_operand(i) { continue; }
        // Hex codepoint?
        let s = *a;
        if literal_byte {
            match u8::from_str_radix(s, 16) {
                Ok(byte) => { bytes.push(byte); continue; }
                // Malformed operand: refuse to coalesce and let the send-keys
                // handler decide what to do with the whole command.
                Err(_) => return None,
            }
        }
        if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit()) {
                // `0xNN` is a codepoint (iTerm2's encoding for typed
                // characters), so it is UTF-8 encoded here.  Raw bytes only
                // ever arrive through -H above.  Encoding every value keeps
                // 0x80-0xFF correct: they are Latin-1 codepoints, not bytes.
                if let Ok(n) = u32::from_str_radix(rest, 16) {
                    if let Some(c) = char::from_u32(n) {
                        let mut buf = [0u8; 4];
                        bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        continue;
                    }
                }
            }
        }
        // Non-literal mode + non-hex token = could be a named key (Up, Tab,
        // BSpace, C-a, ...).  We can't safely turn that into raw bytes here,
        // so refuse to coalesce.
        if !literal { return None; }
        bytes.extend_from_slice(s.as_bytes());
    }

    Some((target.unwrap_or_else(|| String::new()), bytes))
}

/// Re-join already-tokenized command args into a single command string,
/// re-quoting any token that held whitespace or quote characters so the
/// grouping survives a later re-parse by `parse_command_line` (#476).
/// `parse_command_line` consumes the quotes during tokenization, so a plain
/// `join(" ")` flattens `if-shell -F '1' 'set -g @r A' 'set -g @r B'` into
/// ungrouped words and the stored binding silently dispatches garbage.
/// Tokens with embedded single quotes use double quotes (single-quoted
/// content is fully literal in `parse_command_line`, so `'\''` cannot work);
/// everything else uses single quotes. Chain separators (`;`, `\;`) are left
/// bare so command chaining still splits.
fn requote_command_tail(args: &[&str]) -> String {
    args.iter().map(|t| {
        if t.is_empty() {
            "''".to_string()
        } else if *t == ";" || *t == "\\;" {
            t.to_string()
        } else if t.contains('\'') {
            format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
        } else if t.chars().any(|c| c.is_whitespace() || c == '"') {
            format!("'{}'", t)
        } else {
            t.to_string()
        }
    }).collect::<Vec<String>>().join(" ")
}

/// Walk the sub-commands produced by `split_top_level_semicolons` and merge
/// any consecutive run of `send`/`send-keys` commands targeting the same
/// pane into a single synthesized `send -lt <target> <bytes>` command.
/// This keeps multi-byte VT sequences (arrows, function keys, etc.) atomic
/// when they reach the shell PTY.
fn coalesce_send_commands(parts: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(parts.len());
    let mut acc: Vec<u8> = Vec::new();
    let mut acc_target: Option<String> = None;

    fn flush(out: &mut Vec<String>, acc: &mut Vec<u8>, target: &mut Option<String>) {
        if acc.is_empty() { return; }
        // Re-emit as `send -H`: a byte-exact, pure-ASCII command line.  The
        // previous `send -l '<latin1>'` form ran every byte through a latin-1
        // char round trip, which double-encoded anything the decoder had
        // already turned into UTF-8 (a `send 0x4e2d` arrived as "ä¸­").
        let hex: Vec<String> = acc.iter().map(|b| format!("{:02x}", b)).collect();
        let line = match target.as_deref() {
            Some(t) if !t.is_empty() => format!("send -H -t {} {}", t, hex.join(" ")),
            _ => format!("send -H {}", hex.join(" ")),
        };
        out.push(line);
        acc.clear();
        *target = None;
    }

    for part in parts {
        match decode_send_command(&part) {
            Some((tgt, bytes)) => {
                let target_match = acc.is_empty()
                    || acc_target.as_deref() == Some(tgt.as_str());
                if !target_match {
                    flush(&mut out, &mut acc, &mut acc_target);
                }
                if acc.is_empty() { acc_target = Some(tgt); }
                acc.extend_from_slice(&bytes);
            }
            None => {
                flush(&mut out, &mut acc, &mut acc_target);
                out.push(part);
            }
        }
    }
    flush(&mut out, &mut acc, &mut acc_target);
    out
}

/// Parsed `new-pane` flags. Semantics match tmux `cmd-split-window.c`:
/// `-x`=width, `-y`=height, `-X`=x-position, `-Y`=y-position, `-B`=border-lines,
/// `-T`=title, `-c`=start-directory, `-d`=detached, `-P`=print pane id.
struct ParsedNewPane {
    command: String,
    x: Option<u16>,     // -X x-position
    y: Option<u16>,     // -Y y-position
    w: Option<u16>,     // -x width
    h: Option<u16>,     // -y height
    border: String,     // -B
    title: Option<String>, // -T
    start_dir: Option<String>, // -c
    detached: bool,     // -d
    print: bool,        // -P
    empty: bool,        // -E (empty pane, no command)
}

fn parse_new_pane_args(args: &[&str]) -> ParsedNewPane {
    let mut detached = false;
    let mut print = false;
    let mut empty = false;
    let mut border = String::new();
    let mut title: Option<String> = None;
    let mut start_dir: Option<String> = None;
    // x/y = POSITION (from -X/-Y); w/h = SIZE (from -x/-y). tmux ordering.
    let (mut x, mut y, mut w, mut h): (Option<u16>, Option<u16>, Option<u16>, Option<u16>) = (None, None, None, None);
    let mut skip = std::collections::HashSet::new();
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-d" => { skip.insert(i); detached = true; }
            "-P" => { skip.insert(i); print = true; }
            "-E" => { skip.insert(i); empty = true; }
            "-B" => { if let Some(v) = args.get(i+1) { border = v.trim_matches('"').to_string(); skip.insert(i); skip.insert(i+1); i += 1; } }
            "-T" => { if let Some(v) = args.get(i+1) { title = Some(v.trim_matches('"').to_string()); skip.insert(i); skip.insert(i+1); i += 1; } }
            "-c" => { if let Some(v) = args.get(i+1) { start_dir = Some(v.trim_matches('"').to_string()); skip.insert(i); skip.insert(i+1); i += 1; } }
            "-x" => { if let Some(v) = args.get(i+1) { w = v.parse().ok(); skip.insert(i); skip.insert(i+1); i += 1; } }
            "-y" => { if let Some(v) = args.get(i+1) { h = v.parse().ok(); skip.insert(i); skip.insert(i+1); i += 1; } }
            "-X" => { if let Some(v) = args.get(i+1) { x = v.parse().ok(); skip.insert(i); skip.insert(i+1); i += 1; } }
            "-Y" => { if let Some(v) = args.get(i+1) { y = v.parse().ok(); skip.insert(i); skip.insert(i+1); i += 1; } }
            _ => {}
        }
        i += 1;
    }
    let command = args.iter().enumerate()
        .filter(|(idx, _)| !skip.contains(idx))
        .map(|(_, a)| *a)
        .collect::<Vec<&str>>()
        .join(" ");
    ParsedNewPane { command, x, y, w, h, border, title, start_dir, detached, print, empty }
}

/// Handle a single TCP connection from a client.
/// Parses auth, optional TARGET/PERSISTENT flags, then dispatches commands
/// to the main server event loop via the `tx` channel.
pub(crate) fn handle_connection(
    stream: TcpStream,
    tx: mpsc::Sender<CtrlReq>,
    session_key: &str,
    aliases: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
) {
let client_id = crate::types::next_client_id();
// Enable TCP_NODELAY for low-latency responses
let _ = stream.set_nodelay(true);
// Clone stream for writing, original goes into BufReader for reading
let mut write_stream = match stream.try_clone() {
    Ok(s) => s,
    Err(_) => return,
};

// Set initial timeout for auth (reduced from 5s - client sends immediately)
let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
let mut r = io::BufReader::new(stream);

// Read the authentication line
let mut auth_line = String::new();
if r.read_line(&mut auth_line).is_err() {
    auth_debug(&format!("client_id={} reject: auth read_line error/timeout", client_id));
    return;
}

// Verify session key
let auth_line = auth_line.trim();
if !auth_line.starts_with("AUTH ") {
    auth_debug(&format!("client_id={} reject: no AUTH prefix, line={:?}", client_id, auth_line));
    // Legacy client without auth - reject for security
    let _ = write_stream.write_all(b"ERROR: Authentication required\n");
    let _ = write_stream.flush();
    return;
}
let provided_key = auth_line.strip_prefix("AUTH ").unwrap_or("");
if provided_key != session_key {
    auth_debug(&format!(
        "client_id={} reject: key mismatch provided={:?} expected={:?}",
        client_id, provided_key, session_key
    ));
    let _ = write_stream.write_all(b"ERROR: Invalid session key\n");
    let _ = write_stream.flush();
    return;
}
// Auth successful - send OK and flush immediately
let _ = write_stream.write_all(b"OK\n");
let _ = write_stream.flush();

// Use a reasonable timeout for the first command after AUTH.
// Clients may have a small delay between AUTH and the actual command.
let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(2000)));

// Check for PERSISTENT flag and optional TARGET line
let mut persistent = false;
let mut resp_tx_opt: Option<mpsc::Sender<mpsc::Receiver<String>>> = None;
let mut global_target_win: Option<usize> = None;
let mut global_target_win_is_id = false;
let mut global_target_win_name: Option<String> = None;
let mut global_target_pane: Option<usize> = None;
let mut global_pane_is_id = false;
let mut line = String::new();
if r.read_line(&mut line).is_err() {
    return;
}

// Check if client requests persistent connection mode
if line.trim() == "PERSISTENT" {
    persistent = true;
    // Enable TCP_NODELAY for low-latency persistent connections
    let _ = r.get_ref().set_nodelay(true);
    let _ = write_stream.set_nodelay(true);
    // Use longer read timeout for persistent mode - client controls pacing
    let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(5000)));

    // Track this stream so the server can explicitly shut it down before
    // process::exit(0).  Without this, the client never gets EOF on
    // Windows loopback sockets.
    crate::types::register_persistent_stream(client_id, &write_stream);
    
    // Spawn a dedicated writer thread so the read loop never blocks
    // waiting for dump-state responses.  The read loop sends oneshot
    // receivers here; the writer thread waits for each response and
    // writes it to TCP in order.
    let mut ws_bg = write_stream.try_clone().unwrap();
    // Prevent the writer from blocking indefinitely when the client's TCP
    // receive buffer fills up (e.g. during a slow render). Without a write
    // timeout, a full socket causes write() to block forever, silently
    // freezing frame delivery. 5 s matches the command-response timeout.
    let _ = ws_bg.set_write_timeout(Some(Duration::from_secs(5)));
    let (resp_tx, resp_rx) = mpsc::channel::<mpsc::Receiver<String>>();

    // Register a frame slot for server-pushed frames (event-driven rendering).
    // Slot holds at most one pending frame; push_frame() overwrites any
    // unconsumed frame because only the latest snapshot is worth rendering.
    let frame_slot = crate::types::register_frame_channel(client_id);

    // Register a directive channel for queued directives (e.g. SWITCH).
    // Directives use a separate mpsc channel so they are never affected
    // by frame slot contention.
    let directive_rx = crate::types::register_directive_channel(client_id);

    // Clone the write socket so the Guard can shut down the connection when
    // the writer exits. shutdown(Both) on any clone affects the underlying
    // socket, causing the client's reader thread to receive EOF and reconnect
    // instead of hanging indefinitely with a frozen last frame.
    //
    // We use write_stream (not ws_bg) as the source so that even under fd
    // pressure the clone chain stays shallow.  If the clone fails here we
    // return early — the client immediately sees a closed connection and
    // reconnects, which is far better than hanging with no shutdown signal.
    let ws_shutdown = match write_stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let tx_writer = tx.clone();
    std::thread::spawn(move || {
        // Deregister the frame channel and shut down the TCP connection when
        // this thread exits for any reason (write timeout, resp_rx disconnect,
        // etc.). The shutdown causes the client's reader thread to see EOF,
        // which triggers reconnect rather than leaving the client frozen.
        //
        // Also enqueue ClientDetach so a teardown observed *only* by the writer
        // path (write timeout / broken pipe / resp_rx disconnect, before the
        // reader loop reaches its EOF branch, or when the reader never set
        // `attached_sent`) still reaps the `client_registry` entry. The reaper
        // is idempotent, so if the reader path also fires ClientDetach for the
        // same `client_id` the second one is a harmless no-op. The client
        // reconnects under a *new* `client_id`, so reaping the old id here does
        // not remove the live reconnected client.
        struct Guard { client_id: u64, shutdown: std::net::TcpStream, tx: mpsc::Sender<CtrlReq> }
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = self.shutdown.shutdown(std::net::Shutdown::Both);
                crate::types::deregister_frame_channel(self.client_id);
                crate::types::remove_directive_channel(self.client_id);
                crate::types::deregister_persistent_stream(self.client_id);
                let _ = self.tx.send(CtrlReq::ClientDetach(self.client_id));
            }
        }
        let _guard = Guard { client_id, shutdown: ws_shutdown, tx: tx_writer };

        loop {
            // Each iteration drains all three sources in priority order
            // (directives, command responses, frame slot). The frame slot
            // is always checked, so command-response activity cannot
            // starve frame delivery.

            // 0. Drain queued directives (non-blocking).
            while let Ok(directive) = directive_rx.try_recv() {
                if write!(ws_bg, "{}\n", directive).is_err() { return; }
                if ws_bg.flush().is_err() { return; }
            }
            // 1. Drain pending command responses.
            match resp_rx.recv_timeout(Duration::from_millis(5)) {
                Ok(rrx) => {
                    // Use a timeout matching the TCP write timeout (5 s) so the
                    // writer thread cannot block indefinitely if the command
                    // handler is slow or panics without sending a response.
                    // A timeout (or disconnected sender) is treated as fatal:
                    // break so Guard::drop fires, the client receives EOF, and
                    // reconnects cleanly rather than stalling on a silent drop.
                    match rrx.recv_timeout(Duration::from_secs(5)) {
                        Ok(text) => {
                            if write!(ws_bg, "{}\n", text).is_err() { return; }
                            if ws_bg.flush().is_err() { return; }
                        }
                        Err(_) => return,
                    }
                    while let Ok(rrx) = resp_rx.try_recv() {
                        match rrx.recv_timeout(Duration::from_secs(5)) {
                            Ok(text) => {
                                if write!(ws_bg, "{}\n", text).is_err() { return; }
                                if ws_bg.flush().is_err() { return; }
                            }
                            Err(_) => return,
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            // 2. Take the latest pushed frame from the slot.
            let frame = frame_slot.lock().ok().and_then(|mut slot| slot.take());
            if let Some(text) = frame {
                if write!(ws_bg, "{}\n", text).is_err() { return; }
                if ws_bg.flush().is_err() { return; }
            }
        }
    });
    resp_tx_opt = Some(resp_tx);
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

// Check for CONTROL or CONTROL_NOECHO (control mode)
let control_echo = line.trim() == "CONTROL";
let control_noecho = line.trim() == "CONTROL_NOECHO";
if control_echo || control_noecho {
    let _ = r.get_ref().set_nodelay(true);
    let _ = write_stream.set_nodelay(true);
    let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(5000)));

    let ctrl_client_id = crate::types::next_client_id();
    let writer_guard = ControlWriterGuard {
        client_id: ctrl_client_id,
        shutdown: match write_stream.try_clone() {
            Ok(stream) => stream,
            Err(_) => return,
        },
    };
    // No write timeout here: a slow control reader must be buffered, not
    // disconnected (tmux keeps an unbounded evbuffer per control client).
    // Backpressure is handled by backlog accounting — %output frames are
    // shed past a threshold while structural notifications always queue —
    // and a dead peer surfaces as a hard write error via TCP reset.
    let _connection_guard = ControlConnectionGuard {
        client_id: ctrl_client_id,
        request_tx: tx.clone(),
    };

    let (output_tx, output_rx) = std::sync::mpsc::channel::<ControlOutput>();
    let output_backlog = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // For -CC (no-echo) mode, emit the DCS opening sequence "\033P1000p"
    // before anything else. Real tmux writes exactly 7 bytes with NO
    // trailing newline (tmux/control.c control_start()). The next bytes
    // on the wire are the first %begin line, so iTerm2 sees:
    //   \x1bP1000p%begin <time> 1 0\n%end <time> 1 0\n
    // which enters DCS mode and delivers "%begin ..." as the first
    // DCS data line.
    //
    // After the DCS, we emit a synthetic %begin/%end pair representing
    // the response to the implicit attach-session that bare `tmux -CC` runs.
    // Real tmux uses flags=0 (server-originated) here. iTerm2's parseBegin:
    //   - flag=1 (client-originated) requires a queued command in
    //     commandQueue_, otherwise aborts with "%begin with empty command
    //     queue" → tmuxHostDisconnected → "Detached".
    //   - flag=0 (server-originated) creates a synthetic currentCommand_
    //     and the matching %end fires tmuxInitialCommandDidCompleteSuccessfully
    //     which kicks off iTerm's tmux integration (phony-command, ping, etc.).
    let init_result = (|| -> io::Result<()> {
        if control_noecho {
            let init_ts = chrono::Utc::now().timestamp();
            write_stream.write_all(b"\x1bP1000p")?;
            writeln!(write_stream, "%begin {} 1 0", init_ts)?;
            writeln!(write_stream, "%end {} 1 0", init_ts)?;
        } else {
            writeln!(write_stream)?;
        }
        write_stream.flush()
    })();
    if init_result.is_err() {
        return;
    }

    crate::types::register_persistent_stream(ctrl_client_id, &write_stream);
    let writer_backlog = output_backlog.clone();
    std::thread::spawn(move || {
        let _writer_guard = writer_guard;
        while let Ok(output) = output_rx.recv() {
            let cost = crate::control::output_cost(&output);
            let result = write_control_output(&mut write_stream, output);
            writer_backlog.fetch_sub(cost, std::sync::atomic::Ordering::Relaxed);
            match result {
                Ok(false) => {}
                Ok(true) | Err(_) => return,
            }
        }
    });

    // NOW register with the server. This triggers emit_initial_state()
    // which sends %session-changed and other notifications through the
    // notification channel. Because we flushed the DCS + %begin/%end
    // above, those bytes are already in the kernel send buffer and will
    // arrive at the client before any notifications.
    if tx.send(CtrlReq::ControlRegister {
        client_id: ctrl_client_id,
        echo: control_echo,
        output_tx: output_tx.clone(),
        backlog: output_backlog.clone(),
    }).is_err() {
        return;
    }

    // Control mode command loop: read lines, dispatch, wrap in %begin/%end/%error
    let mut cmd_counter: u64 = 0;
    let tx_ctrl = tx.clone();
    let aliases_ctrl = aliases.clone();
    // Queue of pending sub-command strings produced by splitting a single input
    // line on top-level `;` (real tmux does this in its command parser). iTerm2's
    // sendCommandList joins many commands with "; " into one wire line and
    // expects one %begin/%end pair per sub-command.
    let mut pending: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    loop {
        let trimmed_owned: String = if let Some(s) = pending.pop_front() {
            s
        } else {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) => break, // EOF
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
                        continue;
                    }
                    break;
                }
                Ok(_) => {}
            }

            // Strip leading ASCII control characters (e.g., \x03 Ctrl-C) that
            // iTerm2 sends when entering tmux gateway mode. Real tmux's command
            // parser silently ignores these; without this strip they get glued
            // onto the first command name (e.g. "\x03phony-command") and are
            // rejected as "unknown command", causing iTerm2 to detach.
            let trimmed_raw = line.trim();
            let stripped = trimmed_raw.trim_start_matches(|c: char| (c as u32) < 0x20 && c != '\t');
            if stripped.is_empty() { continue; }

            // Split on top-level `;` (respecting single/double quotes and `\`
            // escapes). If the line splits into multiple sub-commands, queue
            // the rest and process the first; this mirrors real tmux's parser
            // and is required for iTerm2's multi-command kickoff lines like
            // `show -v -q -t $0 @x; refresh-client -C 80,25; show ...`.
            let parts = split_top_level_semicolons(stripped);
            let parts = coalesce_send_commands(parts);
            if parts.is_empty() { continue; }
            let mut iter = parts.into_iter();
            let first = iter.next().unwrap();
            for rest in iter { pending.push_back(rest); }
            first
        };
        let trimmed: &str = trimmed_owned.trim();
        if trimmed.is_empty() { continue; }

        cmd_counter += 1;
        let ts = chrono::Utc::now().timestamp();

        // Dispatch the command before handing its completed block to the
        // single control writer.
        let parsed = crate::cli::normalize_flag_equals(parse_command_line(trimmed));
        let raw_cmd = parsed.first().map(|s| s.as_str()).unwrap_or("");

        if raw_cmd.is_empty() {
            let block = format_control_command_block(
                control_echo,
                trimmed,
                ts,
                cmd_counter,
                Some(Ok(ControlCommandResponse::empty())),
            );
            let block_output = ControlOutput::CommandBlock(block);
            output_backlog.fetch_add(crate::control::output_cost(&block_output), std::sync::atomic::Ordering::Relaxed);
            if output_tx.send(block_output).is_err() {
                break;
            }
            continue;
        }

        // Check aliases
        let alias_expanded = if let Ok(map) = aliases_ctrl.read() {
            map.get(raw_cmd).cloned()
        } else { None };

        let (cmd_name, cmd_args): (&str, Vec<&str>) = if let Some(ref expanded) = alias_expanded {
            let parts: Vec<&str> = expanded.split_whitespace().collect();
            let mut all: Vec<&str> = parts[1..].to_vec();
            all.extend(parsed.iter().skip(1).map(|s| s.as_str()));
            (parts.first().copied().unwrap_or(raw_cmd), all)
        } else {
            (raw_cmd, parsed.iter().skip(1).map(|s| s.as_str()).collect())
        };

        // Parse -t from command args
        let mut ctrl_target_win: Option<usize> = None;
        let mut ctrl_target_win_is_id = false;
        let mut ctrl_target_win_name: Option<String> = None;
        let mut ctrl_target_pane: Option<usize> = None;
        let mut ctrl_pane_is_id = false;
        let mut ctrl_raw_target: Option<String> = None;
        {
            let mut i = 0;
            while i < cmd_args.len() {
                if cmd_args[i] == "-t" {
                    if let Some(v) = cmd_args.get(i+1) {
                        ctrl_raw_target = Some(v.to_string());
                        let pt = parse_target(v);
                        if pt.window.is_some() { ctrl_target_win = pt.window; ctrl_target_win_is_id = pt.window_is_id; ctrl_target_win_name = None; }
                        else if pt.window_name.is_some() { ctrl_target_win_name = pt.window_name; ctrl_target_win = None; ctrl_target_win_is_id = false; }
                        if pt.pane.is_some() {
                            ctrl_target_pane = pt.pane;
                            ctrl_pane_is_id = pt.pane_is_id;
                        }
                    }
                    i += 2; continue;
                }
                i += 1;
            }
        }

        // Build filtered args (without -t)
        let filtered_args: Vec<&str> = {
            let mut filtered = Vec::new();
            let mut i = 0;
            while i < cmd_args.len() {
                if cmd_args[i] == "-t" { i += 2; continue; }
                filtered.push(cmd_args[i]);
                i += 1;
            }
            filtered
        };

        // Apply target focus
        let is_focus_cmd = matches!(cmd_name, "select-window" | "selectw" | "select-pane" | "selectp");
        let skip_target_focus = matches!(cmd_name, "resize-window" | "resizew" | "kill-window" | "killw");
        if let Some(wid) = ctrl_target_win {
            if is_focus_cmd {
                if ctrl_target_win_is_id {
                    let _ = tx_ctrl.send(CtrlReq::FocusWindowById(wid));
                } else {
                    let _ = tx_ctrl.send(CtrlReq::FocusWindow(wid));
                }
            } else if !skip_target_focus {
                if ctrl_target_win_is_id {
                    let _ = tx_ctrl.send(CtrlReq::FocusWindowByIdTemp(wid));
                } else {
                    let _ = tx_ctrl.send(CtrlReq::FocusWindowTemp(wid));
                }
            }
        } else if let Some(ref wname) = ctrl_target_win_name {
            if is_focus_cmd {
                let _ = tx_ctrl.send(CtrlReq::FocusWindowByName(wname.clone()));
            } else if !skip_target_focus {
                let _ = tx_ctrl.send(CtrlReq::FocusWindowByNameTemp(wname.clone()));
            }
        }
        if let Some(pid) = ctrl_target_pane {
            if is_focus_cmd {
                if ctrl_pane_is_id {
                    let _ = tx_ctrl.send(CtrlReq::FocusPane(pid));
                } else {
                    let _ = tx_ctrl.send(CtrlReq::FocusPaneByIndex(pid));
                }
            } else if !matches!(cmd_name, "swap-pane" | "swapp" | "resize-window" | "resizew") {
                // swap-pane resolves its own target and swaps it with the *current*
                // active pane.  Temporarily focusing the target here would make
                // active == target, turning the swap into a no-op (so skip it).
                if ctrl_pane_is_id {
                    let _ = tx_ctrl.send(CtrlReq::FocusPaneTemp(pid));
                } else {
                    let _ = tx_ctrl.send(CtrlReq::FocusPaneByIndexTemp(pid));
                }
            }
        }

        // Dispatch command (use a oneshot for the response)
        let (resp_s, resp_r) = mpsc::channel::<ControlCommandResponse>();
        let dispatched = dispatch_control_command(
            cmd_name, &filtered_args, &tx_ctrl, resp_s,
            ctrl_target_pane, ctrl_pane_is_id, ctrl_raw_target.as_deref(),
            ctrl_client_id,
        );

        // The dispatcher runs synchronously and each handled command must
        // complete its oneshot exactly once. The writer remains independent,
        // so waiting here never blocks control-socket output.
        let response_result = if dispatched {
            Some(resp_r.recv())
        } else {
            None
        };
        let block = format_control_command_block(
            control_echo,
            trimmed,
            ts,
            cmd_counter,
            response_result,
        );
        let block_output = ControlOutput::CommandBlock(block);
        output_backlog.fetch_add(crate::control::output_cost(&block_output), std::sync::atomic::Ordering::Relaxed);
        if output_tx.send(block_output).is_err() {
            break;
        }
    }

    return;
}

// Check if this line is a TARGET specification
// Save raw target for relative pane specifiers like :.+ and :.-
let mut global_raw_target: Option<String> = None;
if line.trim().starts_with("TARGET ") {
    let target_spec = line.trim().strip_prefix("TARGET ").unwrap_or("");
    global_raw_target = Some(target_spec.to_string());
    let parsed = parse_target(target_spec);
    global_target_win = parsed.window;
    global_target_win_is_id = parsed.window_is_id;
    global_target_win_name = parsed.window_name;
    global_target_pane = parsed.pane;
    global_pane_is_id = parsed.pane_is_id;
    // Now read the actual command line
    line.clear();
    if r.read_line(&mut line).is_err() {
        return;
    }
}

// Set short read timeout for batched command processing
let _ = r.get_ref().set_read_timeout(Some(Duration::from_millis(10)));

// Process commands in a loop to handle batching
let mut attached_sent = false;
let mut pending_chain: Vec<String> = Vec::new();
loop {
    // Check pending chained commands before reading from socket
    if !pending_chain.is_empty() {
        line = pending_chain.remove(0);
    } else if line.trim().is_empty() {
        // Try to read another command with timeout
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => {
                // EOF - client disconnected
                if attached_sent {
                    let _ = tx.send(CtrlReq::ClientDetach(client_id));
                }
                break;
            }
            Err(e) => {
                // In persistent mode, timeouts are expected - keep waiting
                if persistent && (e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut) {
                    line.clear(); // Clear any partial data from interrupted read
                    continue;
                }
                if attached_sent {
                    let _ = tx.send(CtrlReq::ClientDetach(client_id));
                }
                break; // Real error or non-persistent timeout
            }
            Ok(_) => continue, // Process the new line
        }
    }
    
    // Use quote-aware parser to preserve arguments with spaces
    // Handle command chaining (\; or ;) by splitting into sub-commands
    let sub_cmds = crate::config::split_chained_commands_pub(line.trim());
    let effective_line: String;
    if sub_cmds.len() > 1 {
        effective_line = sub_cmds[0].clone();
        pending_chain.extend(sub_cmds.into_iter().skip(1));
    } else {
        effective_line = line.trim().to_string();
    }
    let parsed = crate::cli::normalize_flag_equals(parse_command_line(&effective_line));
    let raw_cmd = parsed.get(0).map(|s| s.as_str()).unwrap_or("");
    // Check command aliases before normal dispatch
    let alias_expanded = if let Ok(map) = aliases.read() {
        map.get(raw_cmd).cloned()
    } else { None };
    let (cmd, args): (&str, Vec<&str>) = if let Some(ref expanded) = alias_expanded {
        // Alias expansion: replace command name, keep original args
        let expanded_parts: Vec<&str> = expanded.split_whitespace().collect();
        let mut all_args: Vec<&str> = expanded_parts[1..].to_vec();
        all_args.extend(parsed.iter().skip(1).map(|s| s.as_str()));
        (expanded_parts.first().copied().unwrap_or(raw_cmd), all_args)
    } else {
        (raw_cmd, parsed.iter().skip(1).map(|s| s.as_str()).collect())
    };

// Parse -t argument from command line (takes precedence over global TARGET)
let mut target_win: Option<usize> = global_target_win;
let mut target_win_is_id: bool = global_target_win_is_id;
let mut target_win_name: Option<String> = global_target_win_name.clone();
let mut target_pane: Option<usize> = global_target_pane;
let mut pane_is_id = global_pane_is_id;
// Save raw -t value for relative pane targets like :.+ or :.-
// Falls back to global_raw_target from TARGET protocol line
let mut raw_target: Option<String> = global_raw_target.clone();
let mut i = 0;
while i < args.len() {
    if args[i] == "-t" {
        if let Some(v) = args.get(i+1) {
            raw_target = Some(v.to_string());
            // Parse the -t value using parse_target for consistent handling
            let pt = parse_target(v);
            if pt.window.is_some() { target_win = pt.window; target_win_is_id = pt.window_is_id; target_win_name = None; }
            else if pt.window_name.is_some() { target_win_name = pt.window_name; target_win = None; target_win_is_id = false; }
            if pt.pane.is_some() { 
                target_pane = pt.pane;
                pane_is_id = pt.pane_is_id;
            }
        }
        i += 2; continue;
    }
    i += 1;
}
// Build args without -t and its value so command handlers get clean positional args
let args: Vec<&str> = {
    let mut filtered = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            i += 2; // skip -t and its value
            continue;
        }
        filtered.push(args[i]);
        i += 1;
    }
    filtered
};
// Commands that should permanently change focus when used with -t
let is_focus_cmd = matches!(cmd, "select-window" | "selectw" | "select-pane" | "selectp");
// Commands that handle -t internally and should NOT get FocusWindowTemp.
// switch-client resolves the window/pane target itself (#483) and makes the
// change PERMANENT; letting the generic block issue a temporary focus here
// would restore the old focus after the batch and silently undo the switch.
let skip_target_focus = matches!(cmd, "join-pane" | "joinp" | "move-pane" | "movep"
    | "move-window" | "movew" | "swap-window" | "swapw"
    | "switch-client" | "switchc" | "resize-window" | "resizew"
    | "kill-window" | "killw");
if let Some(wid) = target_win {
    if is_focus_cmd {
        if target_win_is_id {
            let _ = tx.send(CtrlReq::FocusWindowById(wid));
        } else {
            let _ = tx.send(CtrlReq::FocusWindow(wid));
        }
    } else if !skip_target_focus {
        if target_win_is_id {
            let _ = tx.send(CtrlReq::FocusWindowByIdTemp(wid));
        } else {
            let _ = tx.send(CtrlReq::FocusWindowTemp(wid));
        }
    }
} else if let Some(ref wname) = target_win_name {
    if is_focus_cmd {
        let _ = tx.send(CtrlReq::FocusWindowByName(wname.clone()));
    } else if !skip_target_focus {
        let _ = tx.send(CtrlReq::FocusWindowByNameTemp(wname.clone()));
    }
}
let targeted_kill_pane_id = if matches!(cmd, "kill-pane" | "killp") && pane_is_id {
    target_pane
} else {
    None
};
// swap-pane swaps the target with the *current* active pane; focusing the
// target first would make active == target and turn the swap into a no-op.
let skip_pane_focus = matches!(cmd, "display-message" | "display" | "swap-pane" | "swapp") || skip_target_focus;
if !skip_pane_focus && targeted_kill_pane_id.is_none() {
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
    "new-window" | "neww" => {
        let name: Option<String> = args.windows(2).find(|w| w[0] == "-n").map(|w| w[1].trim_matches('"').to_string());
        let start_dir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
        let detached = args.iter().any(|a| *a == "-d");
        let print_info = args.iter().any(|a| *a == "-P");
        let format_str: Option<String> = extract_flag_value(&args, "-F").map(|s| s.trim_matches('"').to_string());
        let title: Option<String> = extract_flag_value(&args, "-T").map(|s| s.trim_matches('"').to_string());
        let empty = args.iter().any(|a| *a == "-E");
        // -e KEY=VALUE (repeatable, tmux parity, #489): collect environment
        // for the new pane. The values must also be excluded from the
        // shell-command extraction below or they get spawned as the command.
        let env_sets: Vec<(String, String)> = args.windows(2)
            .filter(|w| w[0] == "-e")
            .filter_map(|w| w[1].trim_matches('"').split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let cmd_str: Option<String> = args.iter()
            .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-n" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-T" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-e" && w[1] == **a)) && !args.iter().any(|f| f.starts_with("-F") && f.len() > 2 && &f[2..] == **a))
            .map(|s| s.trim_matches('"').to_string());
        if print_info {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::NewWindowPrint(cmd_str, name, detached, start_dir, format_str, rtx, title, empty, env_sets));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        } else {
            let _ = tx.send(CtrlReq::NewWindow(cmd_str, name, detached, start_dir, title, empty, env_sets));
        }
    }
    "split-window" | "splitw" | "split-pane" | "splitp" => {
        let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
        let detached = args.iter().any(|a| *a == "-d");
        let print_info = args.iter().any(|a| *a == "-P");
        let format_str: Option<String> = extract_flag_value(&args, "-F").map(|s| s.trim_matches('"').to_string());
        let title: Option<String> = extract_flag_value(&args, "-T").map(|s| s.trim_matches('"').to_string());
        let start_dir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
        // -p N = percentage, -l N = cell count, -l N% = percentage (tmux semantics)
        let split_size: Option<(u16, bool)> = args.windows(2).find(|w| w[0] == "-p")
            .and_then(|w| w[1].trim_matches('%').parse::<u16>().ok())
            .map(|v| (v, true))
            .or_else(|| args.windows(2).find(|w| w[0] == "-l")
                .and_then(|w| {
                    let raw = &w[1];
                    let is_pct = raw.ends_with('%');
                    raw.trim_end_matches('%').parse::<u16>().ok().map(|v| (v, is_pct))
                }));
        // -e KEY=VALUE (repeatable, tmux parity, #489): environment for the
        // new pane. Excluded from shell-command extraction below — before
        // this fix the -e value itself was spawned as the pane command,
        // which flashed a red error and closed the pane instantly.
        let env_sets: Vec<(String, String)> = args.windows(2)
            .filter(|w| w[0] == "-e")
            .filter_map(|w| w[1].trim_matches('"').split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let cmd_str: Option<String> = args.iter()
            .find(|a| !a.starts_with('-') && args.windows(2).all(|w| !(w[0] == "-c" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-p" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-l" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-T" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-F" && w[1] == **a)) && args.windows(2).all(|w| !(w[0] == "-e" && w[1] == **a)))
            .map(|s| s.trim_matches('"').to_string());
        if print_info {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, split_size, format_str, rtx, title, env_sets));
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, split_size, rtx, title, env_sets));
            if let Ok(err_msg) = rrx.recv_timeout(Duration::from_millis(2000)) {
                if !err_msg.is_empty() {
                    let _ = write!(write_stream, "{}\n", err_msg);
                    let _ = write_stream.flush();
                }
            }
        }
    }
    "kill-pane" | "killp" => {
        if let Some(pid) = targeted_kill_pane_id {
            let _ = tx.send(CtrlReq::KillPaneById(pid));
        } else {
            let _ = tx.send(CtrlReq::KillPane);
        }
    }
    "capture-pane" | "capturep" => {
        let print_stdout = crate::cli::has_short_flag(&args, 'p');
        let join_lines = crate::cli::has_short_flag(&args, 'J');
        let escape_seqs = crate::cli::has_short_flag(&args, 'e');
        // Parse -S start and -E end (negative = scrollback offset, - = entire scrollback)
        let s_arg = args.windows(2).find(|w| w[0] == "-S").map(|w| w[1]);
        let e_arg = args.windows(2).find(|w| w[0] == "-E").map(|w| w[1]);
        let start: Option<i32> = match s_arg {
            Some("-") => Some(i32::MIN), // entire scrollback start
            Some(v) => v.parse::<i32>().ok(),
            None => None,
        };
        let end: Option<i32> = match e_arg {
            Some("-") => None, // to end of visible
            Some(v) => v.parse::<i32>().ok(),
            None => None,
        };
        let (rtx, rrx) = mpsc::channel::<String>();
        if escape_seqs {
            let _ = tx.send(CtrlReq::CapturePaneStyled(rtx, start, end));
        } else if s_arg.is_some() || e_arg.is_some() {
            let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start, end));
        } else {
            let _ = tx.send(CtrlReq::CapturePane(rtx));
        }
        // Bounded wait, mirroring the control-mode capture path: if the server
        // loop is wedged the client must fail with an error instead of
        // blocking forever on the response channel.
        match rrx.recv_timeout(Duration::from_secs(5)) {
            Ok(mut text) => {
                if join_lines {
                    // Remove trailing whitespace from each line (join wrapped lines)
                    text = text.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
                }
                if print_stdout {
                    // Write text directly — it already ends with \n from capture
                    if persistent {
                        let _ = tx.send(CtrlReq::ShowTextPopup("capture-pane".to_string(), text));
                    } else {
                        let _ = write_stream.write_all(text.as_bytes());
                        let _ = write_stream.flush();
                    }
                    if !persistent { break; }
                } else {
                    let _ = tx.send(CtrlReq::SetBuffer(text));
                }
            }
            Err(_) => {
                if !persistent {
                    let _ = writeln!(write_stream, "ERROR: capture-pane timed out");
                    let _ = write_stream.flush();
                    break;
                }
            }
        }
    }
    "dump-layout" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::DumpLayout(rtx));
        if let Ok(text) = rrx.recv() { 
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("dump-layout".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); 
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "dump-state" | "dump" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::DumpState(rtx, persistent, client_id));
        if let Some(ref rtx_bg) = resp_tx_opt {
            // Persistent mode: hand off to writer thread (non-blocking).
            // This lets the read loop keep processing keys immediately.
            let _ = rtx_bg.send(rrx);
        } else {
            // One-shot mode: block and respond inline
            if let Ok(text) = rrx.recv() { 
                let _ = write!(write_stream, "{}\n", text); 
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        }
    }
    "send-text" => {
        if let Some(payload) = args.get(0) { let _ = tx.send(CtrlReq::SendText(payload.to_string())); }
    }
    "send-paste" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = tx.send(CtrlReq::SendPaste(decoded));
            }
        }
    }
    "send-key" => {
        if let Some(payload) = args.get(0) { let _ = tx.send(CtrlReq::SendKey(payload.to_string())); }
    }
    "zoom-pane" | "resize-pane" | "resizep" if args.iter().any(|a| *a == "-Z") => { let _ = tx.send(CtrlReq::ZoomPane); }
    "zoom-pane" => { let _ = tx.send(CtrlReq::ZoomPane); }
    "prefix-begin" => { let _ = tx.send(CtrlReq::PrefixBegin); }
    "prefix-end" => { let _ = tx.send(CtrlReq::PrefixEnd); }
    "copy-enter" => { let _ = tx.send(CtrlReq::CopyEnter); }
    "copy-move" => {
        if args.len() >= 2 { if let (Ok(dx), Ok(dy)) = (args[0].parse::<i16>(), args[1].parse::<i16>()) { let _ = tx.send(CtrlReq::CopyMove(dx, dy)); } }
    }
    "copy-anchor" => { let _ = tx.send(CtrlReq::CopyAnchor); }
    "rectangle-toggle" => { let _ = tx.send(CtrlReq::CopyRectToggle); }
    "copy-yank" => { let _ = tx.send(CtrlReq::CopyYank); }
    "client-size" => {
        if args.len() >= 2 { if let (Ok(w), Ok(h)) = (args[0].parse::<u16>(), args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::ClientSize(client_id, w, h)); } }
    }
    "host-colors" => {
        // Issue #473: client reports its host terminal's colors (queried at
        // attach time) so the server can answer pane color queries.
        if let Some(spec) = args.get(0) { let _ = tx.send(CtrlReq::HostColors(spec.to_string())); }
    }
    "focus-pane" => {
        if let Some(pid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = tx.send(CtrlReq::FocusPaneCmd(pid)); }
    }
    "focus-window" => {
        if let Some(wid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = tx.send(CtrlReq::FocusWindowCmd(wid)); }
    }
    "mouse-down" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDown(client_id,x,y)); } }
    }
    "mouse-down-right" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDownRight(client_id,x,y)); } }
    }
    "mouse-down-middle" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDownMiddle(client_id,x,y)); } }
    }
    "mouse-drag" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDrag(client_id,x,y)); } }
    }
    "mouse-up" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUp(client_id,x,y)); } }
    }
    "mouse-up-right" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUpRight(client_id,x,y)); } }
    }
    "mouse-up-middle" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUpMiddle(client_id,x,y)); } }
    }
    "mouse-move" => {
        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseMove(client_id,x,y)); } }
    }
    "scroll-up" => {
        let x = args.get(0).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let y = args.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::ScrollUp(client_id, x, y));
    }
    "scroll-down" => {
        let x = args.get(0).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let y = args.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::ScrollDown(client_id, x, y));
    }
    "pane-mouse" => {
        // pane-mouse PANE_ID BUTTON COL ROW M|m
        if args.len() >= 5 {
            if let (Ok(pane_id), Ok(button), Ok(col), Ok(row)) = (
                args[0].parse::<usize>(), args[1].parse::<u8>(),
                args[2].parse::<i16>(), args[3].parse::<i16>()
            ) {
                let press = args[4] != "m";
                let _ = tx.send(CtrlReq::PaneMouse(client_id, pane_id, button, col, row, press));
            }
        }
    }
    "pane-scroll" => {
        // pane-scroll PANE_ID up|down
        if args.len() >= 2 {
            if let Ok(pane_id) = args[0].parse::<usize>() {
                let up = args[1] == "up";
                let _ = tx.send(CtrlReq::PaneScroll(client_id, pane_id, up));
            }
        }
    }
    "split-sizes" => {
        // split-sizes PATH SIZE1,SIZE2,...  (PATH is "_" for root, or dot-separated indices)
        if args.len() >= 2 {
            let path: Vec<usize> = if args[0] == "_" {
                Vec::new()
            } else {
                args[0].split('.').filter_map(|s| s.parse().ok()).collect()
            };
            let sizes: Vec<u16> = args[1].split(',').filter_map(|s| s.parse().ok()).collect();
            if sizes.len() >= 2 {
                let _ = tx.send(CtrlReq::SplitSetSizes(client_id, path, sizes));
            }
        }
    }
    "split-resize-done" => {
        let _ = tx.send(CtrlReq::SplitResizeDone(client_id));
    }
    "next-window" | "next" => { let _ = tx.send(CtrlReq::NextWindow); }
    "previous-window" | "prev" => { let _ = tx.send(CtrlReq::PrevWindow); }
    "rename-window" | "renamew" => { if let Some(name) = args.get(0) { let _ = tx.send(CtrlReq::RenameWindow((*name).to_string())); } }
    "list-windows" | "lsw" => {
        // Extract -F format if provided (supports -F val and -Fval)
        let fmt = extract_flag_value(&args, "-F");
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindowsFormat(rtx, fmt_str));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        } else if args.iter().any(|a| *a == "-J") {
            // JSON output for programmatic use
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindows(rtx));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        } else {
            // tmux-compatible text output (default)
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-windows".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        }
        if !persistent { break; }
    }
    "list-tree" => { let (rtx, rrx) = mpsc::channel::<String>(); let _ = tx.send(CtrlReq::ListTree(rtx)); if let Ok(text) = rrx.recv() { if persistent { let _ = tx.send(CtrlReq::ShowTextPopup("list-tree".to_string(), text)); } else { let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush(); } } if !persistent { break; } }
    "window-layout" => {
        // Issue #257: return simplified layout JSON for a given window id.
        // Usage: window-layout <window_id>
        let wid: Option<usize> = args.get(0).and_then(|a| a.trim_start_matches('@').parse::<usize>().ok());
        if let Some(wid) = wid {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::WindowLayout(wid, rtx));
            if let Ok(text) = rrx.recv() {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
        } else {
            let _ = write!(write_stream, "{{}}\n");
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "window-dump" => {
        // Issue #257: return full styled `LayoutJson` (with rows_v2 cell
        // runs, titles, sizes) for a specific window id. The client uses
        // this for cross-session previews so every pane is rendered with
        // its own content via the same code path as the main viewport.
        // Usage: window-dump <window_id>
        let wid: Option<usize> = args.get(0).and_then(|a| a.trim_start_matches('@').parse::<usize>().ok());
        if let Some(wid) = wid {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::WindowDump(wid, rtx));
            if let Ok(text) = rrx.recv() {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
        } else {
            let _ = write!(write_stream, "{{}}\n");
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "toggle-sync" => { let _ = tx.send(CtrlReq::ToggleSync); }
    "set-pane-title" => { let title = args.join(" "); let _ = tx.send(CtrlReq::SetPaneTitle(title)); }
    "send-keys" | "send" => {
        // tmux short-flag clusters (e.g. iTerm2's `send -lt %1 l`): inspect
        // each `-xyz` arg and check whether any of x/y/z is a known flag.
        let flag_has = |c: char| -> bool {
            args.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.chars().skip(1).any(|fc| fc == c))
        };
        // Returns true if the previous arg is a short-flag cluster whose
        // *trailing* character takes an operand (e.g. -t, -lt, -N).
        let prev_consumes_operand = |i: usize| -> bool {
            if i == 0 { return false; }
            if let Some(prev) = args.get(i - 1) {
                if prev.starts_with('-') && !prev.starts_with("--") && prev.len() >= 2 {
                    if let Some(last) = prev.chars().last() {
                        return matches!(last, 't' | 'T' | 'N' | 'R' | 'c');
                    }
                }
            }
            false
        };
        let literal = flag_has('l');
        let paste_mode = flag_has('p');
        let has_x = flag_has('X');
        let hex_mode = flag_has('H');
        // Parse -N <count> for repeat (look for any cluster ending in 'N')
        let mut repeat_count: usize = 1;
        if let Some(n_pos) = args.iter().position(|a| a.starts_with('-') && !a.starts_with("--") && a.ends_with('N')) {
            if let Some(count_str) = args.get(n_pos + 1) {
                repeat_count = count_str.parse::<usize>().unwrap_or(1).max(1);
            }
        }
        if hex_mode {
            // tmux `send-keys -H`: every operand is one hexadecimal BYTE value.
            // tmux tags these keys KEYC_LITERAL and writes them to the pty as
            // single bytes ("can't be more than 8 bits"), so this is a byte
            // channel, not a codepoint channel.  Decode here and pass the raw
            // bytes through untouched so UTF-8 sequences and escape sequences
            // survive verbatim.
            let mut bytes: Vec<u8> = Vec::new();
            for (i, a) in args.iter().enumerate() {
                if a.starts_with('-') || prev_consumes_operand(i) { continue; }
                // A malformed operand is dropped rather than leaked into the
                // pane as its literal token text.
                if let Ok(byte) = u8::from_str_radix(a, 16) { bytes.push(byte); }
            }
            if !bytes.is_empty() {
                for _ in 0..repeat_count {
                    let _ = tx.send(CtrlReq::SendBytes(bytes.clone()));
                }
            }
        } else if has_x {
            // send-keys -X copy-mode-command
            let cmd_parts: Vec<&str> = args.iter().enumerate()
                .filter(|(i, a)| !a.starts_with('-') && !prev_consumes_operand(*i))
                .map(|(_, a)| *a).collect();
            for _ in 0..repeat_count {
                let _ = tx.send(CtrlReq::SendKeysX(cmd_parts.join(" ")));
            }
        } else {
            let keys: Vec<String> = args.iter()
                .enumerate()
                .filter(|(i, a)| !a.starts_with('-') && !prev_consumes_operand(*i))
                .map(|(_, a)| {
                    // Convert real-tmux 0xNN hex codepoint syntax (sent by
                    // iTerm2's gateway: e.g. `send -t %1 0xd` for Enter) into
                    // the literal character so SendKeys forwards the right
                    // byte to the PTY instead of the string "0xd".
                    let s = *a;
                    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit()) {
                            if let Ok(n) = u32::from_str_radix(rest, 16) {
                                if let Some(c) = char::from_u32(n) {
                                    return c.to_string();
                                }
                            }
                        }
                    }
                    s.to_string()
                })
                .collect();
            // If any key was a hex-converted single byte, force literal mode so
            // the byte is written verbatim and not parsed as a key name.
            let any_hex = args.iter().any(|a| {
                let s = *a;
                if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit());
                }
                false
            });
            let effective_literal = literal || any_hex;
            for _ in 0..repeat_count {
                if paste_mode {
                    let _ = tx.send(CtrlReq::SendPaste(keys.join("")));
                } else {
                    // #490: hand the tokens over UNJOINED so quoted
                    // arguments keep their exact whitespace end to end.
                    let _ = tx.send(CtrlReq::SendKeys(keys.clone(), effective_literal));
                }
            }
        }
    }
    "select-pane" | "selectp" => {
        // Detect relative pane targets: -t :.+  or  -t :.-
        let is_next_pane = raw_target.as_deref().map_or(false, |t| t.contains(".+") || t == "+" || t == ":.+");
        let is_prev_pane = raw_target.as_deref().map_or(false, |t| t.contains(".-") || t == "-" || t == ":.-");
        let dir = if is_next_pane { "next" }
            else if is_prev_pane { "prev" }
            else if args.iter().any(|a| *a == "-U") { "U" }
            else if args.iter().any(|a| *a == "-D") { "D" }
            else if args.iter().any(|a| *a == "-L") { "L" }
            else if args.iter().any(|a| *a == "-R") { "R" }
            else if args.iter().any(|a| *a == "-l") { "last" }
            else if args.iter().any(|a| *a == "-m") { "mark" }
            else if args.iter().any(|a| *a == "-M") { "unmark" }
            else if args.iter().any(|a| *a == "-e") { "enable-input" }
            else if args.iter().any(|a| *a == "-d") { "disable-input" }
            else { "" };
        // Check for -T title
        let title = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string());
        if let Some(t) = title {
            let _ = tx.send(CtrlReq::SetPaneTitle(t));
        }
        // Handle -P style (per-pane style, e.g. "bg=default,fg=blue")
        // Claude Code uses this for agent pane coloring. Store silently
        // even if rendering doesn't support it yet.
        let pane_style = args.windows(2).find(|w| w[0] == "-P").map(|w| w[1].to_string());
        if let Some(style) = pane_style {
            let _ = tx.send(CtrlReq::SetPaneStyle(style));
        }
        if !dir.is_empty() {
            let keep_zoom = args.iter().any(|a| *a == "-Z");
            let _ = tx.send(CtrlReq::SelectPane(dir.to_string(), keep_zoom));
        }
    }
    "select-window" | "selectw" => {
        // An @id target was already focused permanently by the generic target
        // focus block above (FocusWindowById). Re-sending it here as an INDEX
        // via SelectWindow would override that with the wrong window (#497).
        let idx = args.iter().find(|a| !a.starts_with('-')).and_then(|s| s.parse::<usize>().ok())
            .or(if target_win_is_id { None } else { target_win });
        if let Some(idx) = idx {
            let _ = tx.send(CtrlReq::SelectWindow(idx));
        }
        if args.iter().any(|a| *a == "-l") {
            let _ = tx.send(CtrlReq::LastWindow);
        }
        if args.iter().any(|a| *a == "-n") {
            let _ = tx.send(CtrlReq::NextWindow);
        }
        if args.iter().any(|a| *a == "-p") {
            let _ = tx.send(CtrlReq::PrevWindow);
        }
    }
    "list-panes" | "lsp" => {
        let fmt = extract_flag_value(&args, "-F");
        // tmux: -a = all panes across all sessions, -s = all panes in target session
        // psmux uses per-session servers, so -s is equivalent to listing the current
        // session's panes (same as no flag). -a lists all panes in this server too
        // since there's only one session per server.
        let all = args.iter().any(|a| *a == "-a");
        let session_scope = args.iter().any(|a| *a == "-s");
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            if all || session_scope {
                let _ = tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt_str));
            } else {
                let _ = tx.send(CtrlReq::ListPanesFormat(rtx, fmt_str));
            }
        } else {
            if all {
                let _ = tx.send(CtrlReq::ListAllPanes(rtx));
            } else if session_scope {
                // -s: list all panes in the targeted session (all windows)
                let _ = tx.send(CtrlReq::ListAllPanes(rtx));
            } else {
                let _ = tx.send(CtrlReq::ListPanes(rtx));
            }
        }
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-panes".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "kill-window" | "killw" => {
        // Resolve the -t target server-side. The generic temp-focus block is
        // skipped for kill-window (see skip_target_focus): focusing a target
        // that fails to resolve silently left the PREVIOUS window focused and
        // the bare KillWindow then killed it — `kill-window -t sess:typo`
        // destroyed whatever was active (tmux: "can't find window", no kill).
        if target_win.is_some() || target_win_name.is_some() {
            let (resp_s, resp_r) = mpsc::channel();
            let _ = tx.send(CtrlReq::KillWindowTarget {
                win: target_win,
                win_is_id: target_win_is_id,
                name: target_win_name.clone(),
                resp: resp_s,
            });
            if let Ok(Err(e)) = resp_r.recv_timeout(Duration::from_secs(5)) {
                if !persistent {
                    let _ = writeln!(write_stream, "ERROR: {}", e);
                    let _ = write_stream.flush();
                }
                // persistent clients see it in the status bar (set server-side)
            }
        } else {
            let _ = tx.send(CtrlReq::KillWindow);
        }
    }
    "kill-session" | "kill-ses" => {
        // If -t <target> is given, kill that session instead of self.
        // The target may be specified without the -L socket-name namespace
        // prefix (e.g. "worker1" instead of "ns1__worker1"), so if the raw
        // path is missing we ask our own server for its session name and
        // fall through to KillSession when raw_target matches us.
        if let Some(ref tgt) = raw_target {
            let port_path = crate::paths::port_file(tgt);
            let mut handled = false;
            if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    let key = crate::session::read_session_key(tgt).unwrap_or_default();
                    let _ = crate::session::send_control_to_port(port, "kill-session\n", &key);
                    handled = true;
                }
            }
            if !handled {
                // Query our own session name. If it matches the target
                // (in-namespace name), kill self. Otherwise the target
                // simply does not exist on this server.
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::SessionInfo(rtx));
                if let Ok(line) = rrx.recv() {
                    let self_name = line.split(':').next().unwrap_or("").trim();
                    if !self_name.is_empty() && self_name == tgt {
                        let _ = tx.send(CtrlReq::KillSession);
                    }
                }
            }
        } else {
            let _ = tx.send(CtrlReq::KillSession);
        }
    }
    "has-session" => {
        let (rtx, rrx) = mpsc::channel::<bool>();
        let _ = tx.send(CtrlReq::HasSession(rtx));
        if let Ok(exists) = rrx.recv() {
            if !exists { std::process::exit(1); }
        }
    }
    "rename-session" | "rename" => {
        if let Some(name) = args.iter().find(|a| !a.starts_with('-')) {
            let _ = tx.send(CtrlReq::RenameSession((*name).to_string()));
        }
    }
    "claim-session" => {
        // Warm-server claim: rename + synchronous response so CLI knows it's done.
        // Usage: claim-session <name> [<client-cwd>]
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(|s| &**s).collect();
        if let Some(name) = non_flag.first().copied() {
            let client_cwd = non_flag.get(1).map(|s| s.to_string());
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ClaimSession(name.to_string(), client_cwd, rtx));
            if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(5)) {
                let _ = write!(write_stream, "{}", resp);
                let _ = write_stream.flush();
            }
        }
    }
    "swap-pane" | "swapp" => {
        let detach = args.iter().any(|a| *a == "-d");
        let raw_s = args.iter().position(|a| *a == "-s")
            .and_then(|i| args.get(i + 1).copied());
        let raw_t = args.iter().position(|a| *a == "-t")
            .and_then(|i| args.get(i + 1).copied())
            .or_else(|| raw_target.as_deref());
        // -s <src> -t <dst>: swap two explicit panes (#442). Only when BOTH
        // resolve to a concrete pane (id or index); a `{position}` token is not
        // a valid source. Otherwise fall through to the -t / directional forms.
        let src = raw_s.map(parse_target).and_then(|pt| pt.pane.map(|p| (p, pt.pane_is_id)));
        let dst = raw_t.filter(|t| !t.starts_with('{')).map(parse_target)
            .and_then(|pt| pt.pane.map(|p| (p, pt.pane_is_id)));
        if let (Some((sv, sid)), Some((dv, did))) = (src, dst) {
            let _ = tx.send(CtrlReq::SwapPaneSrcDst { src: sv, src_is_id: sid, dst: dv, dst_is_id: did, detach });
        } else if let Some(tok) = raw_t.filter(|t| t.starts_with('{')) {
            // Layout position token like {top-right} — layout-independent.
            let _ = tx.send(CtrlReq::SwapPanePosition(tok.to_string()));
        } else if let Some((p, is_id)) = raw_t.map(parse_target).and_then(|pt| pt.pane.map(|p| (p, pt.pane_is_id))) {
            let _ = tx.send(CtrlReq::SwapPaneTarget(p, is_id));
        } else {
            let dir = if args.iter().any(|a| *a == "-U") { "U" }
                else if args.iter().any(|a| *a == "-L") { "L" }
                else if args.iter().any(|a| *a == "-R") { "R" }
                else { "D" };
            let _ = tx.send(CtrlReq::SwapPane(dir.to_string()));
        }
    }
    "resize-pane" | "resizep" => {
        // Check for zoom toggle first (issue #35)
        if args.iter().any(|a| *a == "-Z") {
            let _ = tx.send(CtrlReq::ZoomPane);
        } else
        // Check for absolute resize (-x N or -y N), supporting both
        // absolute values (e.g. "60") and percentage strings (e.g. "30%").
        if let Some(xval) = args.windows(2).find(|w| w[0] == "-x").map(|w| w[1]) {
            if let Some(pct) = xval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                let _ = tx.send(CtrlReq::ResizePanePercent("x".to_string(), pct));
            } else if let Ok(abs) = xval.parse::<u16>() {
                let _ = tx.send(CtrlReq::ResizePaneAbsolute("x".to_string(), abs));
            }
        } else if let Some(yval) = args.windows(2).find(|w| w[0] == "-y").map(|w| w[1]) {
            if let Some(pct) = yval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                let _ = tx.send(CtrlReq::ResizePanePercent("y".to_string(), pct));
            } else if let Ok(abs) = yval.parse::<u16>() {
                let _ = tx.send(CtrlReq::ResizePaneAbsolute("y".to_string(), abs));
            }
        } else {
            let amount = args.iter().find(|a| a.parse::<u16>().is_ok()).and_then(|s| s.parse::<u16>().ok()).unwrap_or(1);
            let dir = if args.iter().any(|a| *a == "-U") { "U" }
                else if args.iter().any(|a| *a == "-D") { "D" }
                else if args.iter().any(|a| *a == "-L") { "L" }
                else if args.iter().any(|a| *a == "-R") { "R" }
                else { "D" };
            let _ = tx.send(CtrlReq::ResizePane(dir.to_string(), amount));
        }
    }
    "set-buffer" => {
        // Parse -b name, -w (clipboard propagation), and content
        let mut buf_name: Option<String> = None;
        let mut propagate_to_clipboard = false;
        let mut i = 0;
        let mut content_parts: Vec<&str> = Vec::new();
        while i < args.len() {
            if args[i] == "-b" {
                if let Some(name) = args.get(i + 1) {
                    buf_name = Some(name.to_string());
                }
                i += 2; // skip -b and its value (buffer name)
            } else if args[i] == "-w" {
                propagate_to_clipboard = true;
                i += 1;
            } else if args[i].starts_with('-') {
                i += 1; // skip unknown flags
            } else {
                content_parts.extend_from_slice(&args[i..]);
                break;
            }
        }
        let content = content_parts.join(" ");
        if propagate_to_clipboard {
            crate::clipboard::copy_to_system_clipboard(&content);
        }
        if let Some(name) = buf_name {
            let _ = tx.send(CtrlReq::SetNamedBuffer(name, content));
        } else {
            let _ = tx.send(CtrlReq::SetBuffer(content));
        }
    }
    "paste-buffer" | "pasteb" => {
        let buf_name: Option<String> = args.windows(2).find(|w| w[0] == "-b").map(|w| w[1].to_string());
        let paste_mode = args.iter().any(|a| *a == "-p");
        if let Some(ref name) = buf_name {
            // Issue #264: an explicitly-named/-indexed buffer that does not
            // exist must error (matching real tmux's "no buffer <name>"),
            // not silently no-op. ShowBufferAt/ShowNamedBuffer return `None`
            // when the buffer is missing, distinct from an empty-but-present
            // buffer's `Some(String::new())`.
            let (rtx, rrx) = mpsc::channel::<Option<String>>();
            if let Ok(idx) = name.parse::<usize>() {
                let _ = tx.send(CtrlReq::ShowBufferAt(rtx, idx));
            } else {
                let _ = tx.send(CtrlReq::ShowNamedBuffer(rtx, name.clone()));
            }
            match rrx.recv() {
                Ok(Some(text)) => {
                    if !text.is_empty() {
                        if paste_mode {
                            let _ = tx.send(CtrlReq::SendPaste(text));
                        } else {
                            let _ = tx.send(CtrlReq::SendText(text));
                        }
                    }
                }
                _ => {
                    let err = format!("no buffer {}\n", name);
                    if persistent {
                        let _ = tx.send(CtrlReq::ShowTextPopup("paste-buffer".to_string(), format!("ERROR: {}", err.trim_end())));
                    } else {
                        let _ = write!(write_stream, "ERROR: {}", err);
                        let _ = write_stream.flush();
                    }
                }
            }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowBuffer(rtx));
            if let Ok(mut text) = rrx.recv() {
                // Issue #428: when no explicit buffer is named and the internal
                // paste-buffer stack is empty, fall back to the OS clipboard so
                // prefix+] pastes externally-copied text (matching Ctrl+Shift+V).
                if text.is_empty() {
                    if let Some(clip) = crate::clipboard::read_from_system_clipboard() {
                        text = clip;
                    }
                }
                if !text.is_empty() {
                    if paste_mode {
                        let _ = tx.send(CtrlReq::SendPaste(text));
                    } else {
                        let _ = tx.send(CtrlReq::SendText(text));
                    }
                }
            }
        }
    }
    "list-buffers" | "lsb" => {
        let fmt = extract_flag_value(&args, "-F");
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            let _ = tx.send(CtrlReq::ListBuffersFormat(rtx, fmt_str));
        } else {
            let _ = tx.send(CtrlReq::ListBuffers(rtx));
        }
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-buffers".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "show-buffer" | "showb" => {
        let buf_name: Option<String> = args.windows(2).find(|w| w[0] == "-b").map(|w| w[1].to_string());
        let text: String = if let Some(name) = buf_name {
            let (rtx, rrx) = mpsc::channel::<Option<String>>();
            if let Ok(idx) = name.parse::<usize>() {
                let _ = tx.send(CtrlReq::ShowBufferAt(rtx, idx));
            } else {
                let _ = tx.send(CtrlReq::ShowNamedBuffer(rtx, name));
            }
            rrx.recv().ok().flatten().unwrap_or_default()
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowBuffer(rtx));
            rrx.recv().unwrap_or_default()
        };
        if persistent {
            let _ = tx.send(CtrlReq::ShowTextPopup("show-buffer".to_string(), text));
        } else {
            let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "delete-buffer" => {
        let buf_name: Option<String> = args.windows(2).find(|w| w[0] == "-b").map(|w| w[1].to_string());
        if let Some(name) = buf_name {
            if let Ok(idx) = name.parse::<usize>() {
                let _ = tx.send(CtrlReq::DeleteBufferAt(idx));
            } else {
                let _ = tx.send(CtrlReq::DeleteNamedBuffer(name));
            }
        } else {
            let _ = tx.send(CtrlReq::DeleteBuffer);
        }
    }
    "delete-buffer-at" => {
        if let Some(idx_str) = args.get(0) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                let _ = tx.send(CtrlReq::DeleteBufferAt(idx));
            }
        }
    }
    "paste-buffer-at" => {
        if let Some(idx_str) = args.get(0) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                let _ = tx.send(CtrlReq::PasteBufferAt(idx));
            }
        }
    }
    "choose-buffer" | "chooseb" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ChooseBuffer(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("choose-buffer".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "display-message" | "display" => {
        // Parse tmux-like display-message flags without dropping message text.
        let mut print_stdout = false;
        let mut parts: Vec<&str> = Vec::new();
        let mut end_of_opts = false;
        let mut duration_ms: Option<u64> = None;
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if end_of_opts {
                parts.push(a);
                i += 1;
                continue;
            }
            match a {
                "--" => { end_of_opts = true; }
                "-p" => { print_stdout = true; }
                "-F" => { /* format mode */ }
                "-d" => {
                    if i + 1 < args.len() {
                        duration_ms = args[i + 1].parse::<u64>().ok();
                    }
                    i += 1;
                }
                "-I" => { i += 1; }
                _ if a.starts_with('-') => { parts.push(a); }
                _ => parts.push(a),
            }
            i += 1;
        }

        let fmt = if parts.is_empty() {
            crate::commands::DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
        } else {
            parts.join(" ")
        };
        // Pass target pane index for PANE_POS_OVERRIDE (#113).
        // Bare %N (pane_is_id=true) goes through DisplayMessageById which
        // resolves the pane ID globally across windows (#332).
        let (rtx, rrx) = mpsc::channel::<String>();
        if pane_is_id {
            if let Some(pid) = target_pane {
                let _ = tx.send(CtrlReq::DisplayMessageById(rtx, fmt, pid, !print_stdout, duration_ms));
            } else {
                let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, None, !print_stdout, duration_ms));
            }
        } else {
            let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane, !print_stdout, duration_ms));
        }
        if let Ok(text) = rrx.recv() {
            if print_stdout {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("display-message".to_string(), text));
                } else {
                    let _ = writeln!(write_stream, "{}", text);
                    let _ = write_stream.flush();
                }
            }
        }
        if !persistent { break; }
    }
    "last-window" | "last" => { let _ = tx.send(CtrlReq::LastWindow); }
    "last-pane" | "lastp" => { let _ = tx.send(CtrlReq::LastPane); }
    "rotate-window" | "rotatew" => {
        let reverse = args.iter().any(|a| *a == "-D");
        let _ = tx.send(CtrlReq::RotateWindow(reverse));
    }
    "display-panes" | "displayp" => { let _ = tx.send(CtrlReq::DisplayPanes); }
    "break-pane" | "breakp" => { let _ = tx.send(CtrlReq::BreakPane); }
    "join-pane" | "joinp" | "move-pane" | "movep" => {
        // Parse -s source and -h/-v direction.
        // -t target is already parsed by the global -t handler above into target_win / target_pane.
        let horizontal = args.iter().any(|a| *a == "-h");
        // Parse -s source (session:window.pane format)
        let mut src_win: Option<usize> = None;
        let mut src_pane: Option<usize> = None;
        {
            let mut si = 0;
            while si < args.len() {
                if args[si] == "-s" {
                    if let Some(sv) = args.get(si + 1) {
                        let pt = parse_target(sv);
                        src_win = pt.window;
                        src_pane = pt.pane;
                    }
                    si += 2; continue;
                }
                si += 1;
            }
        }
        // If no -s given, try bare integer as target window (legacy compat)
        let tgt_win = target_win.or_else(|| {
            args.iter()
                .find(|a| a.parse::<usize>().is_ok())
                .and_then(|s| s.parse::<usize>().ok())
        });
        // Always send the request (server will use defaults for None fields)
        let _ = tx.send(CtrlReq::JoinPane {
            src_win,
            src_pane,
            target_win: tgt_win,
            target_pane: target_pane,
            horizontal,
        });
    }
    "respawn-pane" | "respawnp" => {
        let workdir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
        let empty = args.iter().any(|a| *a == "-E");
        // -E implies replacing the running pane, so it also kills it.
        let kill = args.iter().any(|a| *a == "-k") || empty;
        // Honor `-- <shell-command>` (issue #399): Claude Code agent-teams
        // delivers the teammate launch via `respawn-pane -k -t %N -- "<cmd>"`.
        // Without this the pane is respawned with the default shell and the
        // teammate never boots (mailbox stays unread, task never runs).
        let command = args.iter().position(|a| *a == "--")
            .map(|i| args[i + 1..].join(" "))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let _ = tx.send(CtrlReq::RespawnPane(workdir, kill, command, empty));
    }
    // ── Cross-session pane forwarding commands ──────────────────────
    "pane-forward-extract" => {
        // Usage: pane-forward-extract <win>.<pane>
        let spec = args.first().copied().unwrap_or("0.0");
        let pt = parse_target(spec);
        let win = pt.window.unwrap_or(0);
        let pane = pt.pane.unwrap_or(0);
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::PaneForwardExtract(win, pane, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_millis(5000)) {
            let _ = write!(write_stream, "{}\n", resp);
            let _ = write_stream.flush();
        } else {
            let _ = write!(write_stream, "ERR timeout\n");
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "pane-forward-inject" => {
        // Usage: pane-forward-inject <src_session> <src_addr> <src_key> <fwd_id> <fwd_port>
        //        <pid> <title> <rows> <cols> <screen_b64_len> [-h] [-t win.pane]
        // Followed by optional screen base64 data on next line.
        if args.len() >= 10 {
            let source_session = args[0].to_string();
            let source_addr = args[1].to_string();
            let source_key = args[2].to_string();
            let forward_id: u64 = args[3].parse().unwrap_or(0);
            let fwd_port: u16 = args[4].parse().unwrap_or(0);
            let pid: u32 = args[5].parse().unwrap_or(0);
            let title = args[6].replace('\x01', " ");
            let rows: u16 = args[7].parse().unwrap_or(24);
            let cols: u16 = args[8].parse().unwrap_or(80);
            let screen_b64_len: usize = args[9].parse().unwrap_or(0);
            let horizontal = args.iter().any(|a| *a == "-h");
            // Read screen base64 data from remaining args/payload
            let screen_b64 = if screen_b64_len > 0 {
                // The base64 data may be appended after the args as a separate read
                let payload: String = args[10..].iter()
                    .filter(|a| **a != "-h" && !a.starts_with("-t"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                if payload.len() >= screen_b64_len {
                    payload[..screen_b64_len].to_string()
                } else {
                    payload
                }
            } else {
                String::new()
            };
            let _ = tx.send(CtrlReq::PaneForwardInject {
                source_session, source_addr, source_key,
                forward_id, fwd_port, pid, title, rows, cols, screen_b64,
                target_win: target_win, target_pane: target_pane, horizontal,
            });
            let _ = write!(write_stream, "OK\n");
            let _ = write_stream.flush();
        } else {
            let _ = write!(write_stream, "ERR not enough args\n");
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "pane-forward-resize" => {
        // Usage: pane-forward-resize <forward_id> <rows> <cols>
        if args.len() >= 3 {
            let fwd_id: u64 = args[0].parse().unwrap_or(0);
            let rows: u16 = args[1].parse().unwrap_or(24);
            let cols: u16 = args[2].parse().unwrap_or(80);
            let _ = tx.send(CtrlReq::PaneForwardResize(fwd_id, rows, cols));
            let _ = write!(write_stream, "OK\n");
        }
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "pane-forward-status" => {
        // Usage: pane-forward-status <forward_id>
        let fwd_id: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::PaneForwardStatus(fwd_id, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_millis(2000)) {
            let _ = write!(write_stream, "{}\n", resp);
        } else {
            let _ = write!(write_stream, "exited\n");
        }
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "pane-forward-kill" => {
        // Usage: pane-forward-kill <forward_id>
        let fwd_id: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::PaneForwardKill(fwd_id));
        let _ = write!(write_stream, "OK\n");
        let _ = write_stream.flush();
        if !persistent { break; }
    }
    "session-info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::SessionInfo(rtx));
        // Bounded wait: session-info doubles as the Tier 2 execution barrier for
        // send_control, so it must never block the connection thread forever if
        // the event loop is momentarily wedged — that would leak the thread and
        // hold the socket open. 3s is far longer than a healthy loop cycle.
        if let Ok(line) = rrx.recv_timeout(Duration::from_secs(3)) {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("session-info".to_string(), line));
            } else {
                let _ = write!(write_stream, "{}\n", line); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "client-attach" => {
        if !attached_sent {
            let _ = tx.send(CtrlReq::ClientAttach(client_id));
            attached_sent = true;
        }
        if !persistent { let _ = write!(write_stream, "ok\n"); }
    }
    "client-detach" => {
        let _ = tx.send(CtrlReq::ClientDetach(client_id));
        attached_sent = false;
        if !persistent { let _ = write!(write_stream, "ok\n"); }
    }
    "bind-key" | "bind" => {
        let mut table = "prefix".to_string();
        let mut repeatable = false;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-T" if i + 1 < args.len() => {
                    table = args[i + 1].to_string();
                    i += 2; continue;
                }
                "-n" => { table = "root".to_string(); i += 1; continue; }
                "-r" => { repeatable = true; i += 1; continue; }
                _ => break,
            }
        }
        if i < args.len() && i + 1 < args.len() {
            let key = args[i].to_string();
            let command = requote_command_tail(&args[i + 1..]);
            let _ = tx.send(CtrlReq::BindKey(table, key, command, repeatable));
        }
    }
    "unbind-key" | "unbind" => {
        if args.iter().any(|a| *a == "-a" || (a.starts_with('-') && a.contains('a'))) {
            // Check if -T or -n was explicitly specified
            let mut has_table = false;
            let mut table = String::new();
            for (j, a) in args.iter().enumerate() {
                if *a == "-T" { if let Some(t) = args.get(j + 1) { table = t.to_string(); has_table = true; } }
                if *a == "-n" { table = "root".to_string(); has_table = true; }
            }
            if has_table {
                let _ = tx.send(CtrlReq::UnbindAllInTable(table));
            } else {
                let _ = tx.send(CtrlReq::UnbindAll);
            }
        } else {
            // Parse -n / -T flags for table-specific individual unbind
            let mut table: Option<String> = None;
            let mut t_value_idx: Option<usize> = None;
            let mut target_session_idx: Option<usize> = None;
            for (j, a) in args.iter().enumerate() {
                if *a == "-T" {
                    if let Some(t) = args.get(j + 1) {
                        table = Some(t.to_string());
                        t_value_idx = Some(j + 1);
                    }
                }
                if *a == "-n" { table = Some("root".to_string()); }
                // -t <session> is the target flag; skip its value
                if *a == "-t" { target_session_idx = Some(j + 1); }
            }
            // Find the key argument: first non-flag arg that isn't the -T table value
            // or the -t session target value
            let key_arg = args.iter().enumerate()
                .filter(|(i, a)| !a.starts_with('-') && Some(*i) != t_value_idx && Some(*i) != target_session_idx)
                .map(|(_, a)| *a)
                .next();
            if let Some(key) = key_arg {
                let _ = tx.send(CtrlReq::UnbindKey(key.to_string(), table));
            }
        }
    }
    "list-keys" | "lsk" => {
        // Parse -T <table> for filtering by key table
        let table_filter = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string());
        // Remaining non-flag args are optional key filter
        let key_filter: Option<String> = args.iter()
            .enumerate()
            .filter(|(i, a)| {
                !a.starts_with('-')
                && !(i > &0 && args.get(i - 1).map_or(false, |prev| *prev == "-T"))
            })
            .map(|(_, a)| a.to_string())
            .next();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListKeys(rtx));
        if let Ok(text) = rrx.recv() {
            let filtered = if table_filter.is_some() || key_filter.is_some() {
                text.lines().filter(|line| {
                    if let Some(ref tbl) = table_filter {
                        // list-keys output format: "bind-key -T <table> <key> <command>"
                        let parts: Vec<&str> = line.splitn(5, ' ').collect();
                        if parts.len() >= 3 {
                            if parts[2] != tbl.as_str() {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    if let Some(ref key) = key_filter {
                        // Filter by key name (4th field)
                        let parts: Vec<&str> = line.splitn(5, ' ').collect();
                        if parts.len() >= 4 {
                            if parts[3] != key.as_str() {
                                return false;
                            }
                        }
                    }
                    true
                }).collect::<Vec<&str>>().join("\n")
            } else {
                text
            };
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-keys".to_string(), filtered));
            } else {
                let _ = write!(write_stream, "{}\n", filtered); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "set-option" | "set" | "set-window-option" | "setw" => {
        // Support combined flag tokens like -ga, -gu, -gq (tmux compat)
        let combined_has_set = |ch: char| -> bool {
            args.iter().any(|a| {
                if *a == format!("-{}", ch) { return true; }
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
            })
        };
        let has_u = combined_has_set('u');
        let has_a = combined_has_set('a');
        let has_q = combined_has_set('q');
        let has_o = combined_has_set('o');
        let global = combined_has_set('g');
        // Skip -t TARGET / -p PANE values (TARGET is not a positional option/value).
        // Note: -w is a scope flag (window), not a target flag — it does NOT
        // consume the next argument.
        let t_targets: std::collections::HashSet<&str> = args.windows(2)
            .filter(|w| w[0] == "-t" || w[0] == "-p")
            .map(|w| w[1]).collect();
        let non_flag_args: Vec<&str> = args.iter()
            .filter(|a| (!a.starts_with('-') || a.starts_with('@')) && !t_targets.contains(*a))
            .copied().collect();
        if has_u {
            if let Some(option) = non_flag_args.first() {
                if *option == "window-size" && !global {
                    let _ = tx.send(CtrlReq::SetWindowSize(None));
                } else {
                    let _ = tx.send(CtrlReq::SetOptionUnset(option.to_string()));
                }
            }
        } else if non_flag_args.len() >= 2 {
            let option = non_flag_args[0].to_string();
            let value = non_flag_args[1..].join(" ");
            if option == "window-size" && !global {
                let _ = tx.send(CtrlReq::SetWindowSize(Some(value)));
            } else if has_a {
                let _ = tx.send(CtrlReq::SetOptionAppend(option, value));
            } else if has_o {
                let _ = tx.send(CtrlReq::SetOptionOnlyIfUnset(option, value));
            } else {
                let _ = tx.send(CtrlReq::SetOptionQuiet(option, value, has_q));
            }
        } else if non_flag_args.len() == 1 {
            // An option name with no value (#535). Previously this fell off the
            // end of the chain and the command vanished: no option set, no
            // warning, exit 0. tmux 3.4 splits it two ways, and so do we.
            let option = non_flag_args[0];
            if crate::server::options::missing_value_toggles(option) {
                // Boolean flag: `set -g mouse` toggles it (tmux parity, #278).
                // The config-file parser already did this; the CLI/TCP path
                // dropped it, so `psmux set -g mouse` was a no-op.
                let _ = tx.send(CtrlReq::SetOptionToggle(option.to_string()));
            }
            // Otherwise it is an error ("empty value"). The CLI reports it on
            // stderr with exit 1 before it ever reaches us (main.rs), which is
            // the only place an exit code exists; nothing to apply here.
        }
    }
    "show-options" | "show" | "show-window-options" | "showw" => {
        // Support combined flag tokens like -gv, -wv, -Av (tmux compat)
        let combined_has = |ch: char| -> bool {
            args.iter().any(|a| {
                if *a == format!("-{}", ch) { return true; }
                // Check combined tokens like -gv, -wvs, etc.
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
            })
        };
        let has_a = combined_has('A');
        let _has_s = combined_has('s');
        let has_w = combined_has('w');
        let window_scope = matches!(cmd, "show-window-options" | "showw") || has_w;
        let has_v = combined_has('v');
        let has_q = combined_has('q');
        let opt_name: Option<&str> = args.iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .last();
        // Extract window index from -t target (issue #266 — needed so
        // per-window options like automatic-rename can return the right
        // value for explicitly-targeted windows).
        let target_window: Option<usize> = extract_flag_value(&args, "-t")
            .as_deref()
            .map(parse_target)
            .and_then(|pt| pt.window);
        if has_v && opt_name.is_some() || (opt_name.is_some() && !has_q) {
            // Single-option query: show-options -v <name> or show <name>
            if let Some(name) = opt_name {
                let (rtx, rrx) = mpsc::channel::<String>();
                if window_scope {
                    let _ = tx.send(CtrlReq::ShowWindowOptionValue(rtx, name.to_string(), target_window));
                } else {
                    let _ = tx.send(CtrlReq::ShowOptionValue(rtx, name.to_string()));
                }
                if let Ok(text) = rrx.recv() {
                    // Options with no real per-window meaning that libtmux/tmuxp
                    // nonetheless probe via `-w` (issue #321); always resolve these
                    // to the global value even without `-A`.
                    const ALWAYS_GLOBAL_FALLBACK: &[&str] = &["pane-base-index", "base-index", "mouse"];
                    let should_fallback = window_scope
                        && (has_a || ALWAYS_GLOBAL_FALLBACK.contains(&name));
                    let resolved = if text.is_empty() && should_fallback {
                        // Fall back to global/session options. Without -A this only
                        // applies to the allowlist above; with -A it applies to any
                        // option so `show-window-options -A -v prefix` still works
                        // (session-only options must stay empty without -A so they
                        // don't leak through show-window-options, see #showw_sendkeys_p).
                        let (frtx, frrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowOptionValue(frtx, name.to_string()));
                        frrx.recv().unwrap_or_default()
                    } else {
                        text
                    };
                    if !(has_q && resolved.is_empty()) {
                        let output = if has_v {
                            format!("{}\n", resolved)
                        } else {
                            format!("{} {}\n", name, resolved)
                        };
                        if persistent {
                            let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), output));
                        } else {
                            let _ = write_stream.write_all(output.as_bytes());
                            let _ = write_stream.flush();
                        }
                    }
                }
            }
        } else if has_v && opt_name.is_none() {
            // -v without option name: list all options, values only
            let (rtx, rrx) = mpsc::channel::<String>();
            if window_scope {
                let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
            } else {
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
            }
            if let Ok(text) = rrx.recv() {
                // Extract values only (each line is "option_name value")
                let values_only: String = text.lines()
                    .filter_map(|line| {
                        let trimmed = line.trim();
                        if trimmed.is_empty() { return None; }
                        // Split at first space: name value
                        if let Some(pos) = trimmed.find(' ') {
                            Some(&trimmed[pos + 1..])
                        } else {
                            Some(trimmed) // option with no value
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let output = if values_only.is_empty() { String::new() } else { format!("{}\n", values_only) };
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), output));
                } else {
                    let _ = write_stream.write_all(output.as_bytes());
                    let _ = write_stream.flush();
                }
            }
        } else {
            if window_scope {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
                if let Ok(mut text) = rrx.recv() {
                    if has_a {
                        let (srtx, srrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowOptions(srtx));
                        if let Ok(session_text) = srrx.recv() {
                            if !text.ends_with('\n') && !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&session_text);
                        }
                    }
                    if persistent {
                        let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), text));
                    } else {
                        let _ = write!(write_stream, "{}\n", text);
                        let _ = write_stream.flush();
                    }
                }
            } else {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
                if let Ok(text) = rrx.recv() {
                    if persistent {
                        let _ = tx.send(CtrlReq::ShowTextPopup("show-options".to_string(), text));
                    } else {
                        let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                    }
                }
            }
        }
        if !persistent { break; }
    }
    "source-file" | "source" => {
        let format_expand = args.iter().any(|a| *a == "-F");
        let parse_only = args.iter().any(|a| *a == "-n");
        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if !parse_only {
            if let Some(path) = non_flag_args.first() {
                let source_spec = if format_expand {
                    format!("-F {}", path)
                } else {
                    path.to_string()
                };
                let _ = tx.send(CtrlReq::SourceFile(source_spec));
            }
        }
    }
    "move-window" | "movew" => {
        // Destination display index: the parsed -t window target (arrives via the
        // TARGET line / `-t :N`), else a bare positional (`move-window N`). The
        // -t value was stripped from `args`, so scan positionals for the fallback
        // (skipping the `-s` source value). Server handler honors gapped indices.
        let target = target_win.or_else(|| {
            args.iter().enumerate()
                .filter(|(i, a)| !a.starts_with('-') && (*i == 0 || args[*i - 1] != "-s"))
                .find_map(|(_, a)| a.trim_start_matches(':').parse::<usize>().ok())
        });
        let _ = tx.send(CtrlReq::MoveWindow(target));
    }
    "swap-window" | "swapw" => {
        // Source: `-s <win>` (bare or ':'-prefixed); None = active window.
        let src = args.windows(2).find(|w| w[0] == "-s")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        // Destination: parsed -t window target, else bare positional.
        let target = target_win.or_else(|| {
            args.iter().enumerate()
                .filter(|(i, a)| !a.starts_with('-') && (*i == 0 || args[*i - 1] != "-s"))
                .find_map(|(_, a)| a.trim_start_matches(':').parse::<usize>().ok())
        });
        if let Some(t) = target {
            let _ = tx.send(CtrlReq::SwapWindow(src, t));
        }
    }
    "link-window" | "linkw" => {
        // Parse -s source_window and -t target_index
        let src_idx = args.windows(2).find(|w| w[0] == "-s")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        let dst_idx = args.windows(2).find(|w| w[0] == "-t")
            .and_then(|w| w[1].trim_start_matches(':').parse::<usize>().ok());
        let _ = tx.send(CtrlReq::LinkWindow(src_idx, dst_idx));
    }
    "unlink-window" | "unlinkw" => {
        let _ = tx.send(CtrlReq::UnlinkWindow);
    }
    "find-window" | "findw" => {
        let pattern = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::FindWindow(rtx, pattern));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("find-window".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "pipe-pane" | "pipep" => {
        let stdin_flag = args.iter().any(|a| *a == "-I");
        let stdout_flag = args.iter().any(|a| *a == "-O");
        let toggle = args.iter().any(|a| *a == "-o");
        // #482: everything after pipe-pane's own options (-I/-O/-o; the -t target
        // was already stripped by the global -t parser) is an opaque shell
        // command. Skip only the leading option flags and keep the rest verbatim
        // so the command's OWN dash-flags survive (e.g.
        // `pwsh -NoProfile -EncodedCommand <b64>`). Previously every dash-token
        // was filtered out, silently mangling the piped command.
        let mut start = 0;
        while start < args.len() && matches!(args[start], "-I" | "-O" | "-o") {
            start += 1;
        }
        let cmd = args[start..].join(" ");
        let (stdin, stdout) = if !stdin_flag && !stdout_flag {
            (false, true)
        } else {
            (stdin_flag, stdout_flag)
        };
        let _ = tx.send(CtrlReq::PipePane(cmd, stdin, stdout, toggle));
    }
    "select-layout" | "selectl" => {
        let layout = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"tiled").to_string();
        let _ = tx.send(CtrlReq::SelectLayout(layout));
    }
    "next-layout" | "nextl" => {
        let _ = tx.send(CtrlReq::NextLayout);
    }
    "list-clients" | "lsc" => {
        let fmt = extract_flag_value(&args, "-F");
        let (rtx, rrx) = mpsc::channel::<String>();
        if let Some(fmt_str) = fmt {
            let _ = tx.send(CtrlReq::ListClientsFormat(rtx, fmt_str));
        } else {
            let _ = tx.send(CtrlReq::ListClients(rtx));
        }
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("list-clients".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "switch-client" | "switchc" => {
        let has_big_t = args.windows(2).any(|w| w[0] == "-T");
        if has_big_t {
            let table = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].to_string()).unwrap_or_default();
            let _ = tx.send(CtrlReq::SwitchClientTable(table));
        } else if args.contains(&"-n") {
            let _ = tx.send(CtrlReq::SwitchClient(String::new(), 'n'));
        } else if args.contains(&"-p") {
            let _ = tx.send(CtrlReq::SwitchClient(String::new(), 'p'));
        } else if args.contains(&"-l") {
            let _ = tx.send(CtrlReq::SwitchClient(String::new(), 'l'));
        } else {
            // -t <target> was already extracted into raw_target by the global -t
            // parser. Pass the FULL target (session:window.pane / @window / %pane)
            // to the server loop so it switches the session AND selects the
            // addressed window/pane, and validates existence (#483). Previously
            // the window/pane suffix was stripped and silently ignored.
            let target = raw_target.clone().unwrap_or_default();
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SwitchClientTarget(target, rtx));
            let resp = rrx.recv_timeout(Duration::from_millis(2000))
                .unwrap_or_else(|_| "OK".to_string());
            if !persistent {
                let _ = write!(write_stream, "{}\n", resp);
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "lock-client" | "lockc" => {
        let _ = tx.send(CtrlReq::LockClient);
    }
    "refresh-client" | "refresh" => {
        let _ = tx.send(CtrlReq::RefreshClient);
    }
    "suspend-client" | "suspendc" => {
        let _ = tx.send(CtrlReq::SuspendClient);
    }
    "copy-mode-page-up" => {
        let _ = tx.send(CtrlReq::CopyModePageUp);
    }
    "clear-history" | "clearhist" => {
        let _ = tx.send(CtrlReq::ClearHistory);
    }
    "save-buffer" | "saveb" => {
        let path = args.iter().find(|a| **a == "-" || !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::SaveBuffer(path));
    }
    "load-buffer" | "loadb" => {
        let path = args.iter().find(|a| **a == "-" || !a.starts_with('-')).unwrap_or(&"").to_string();
        let _ = tx.send(CtrlReq::LoadBuffer(path));
    }
    "set-environment" | "setenv" => {
        let has_u = args.iter().any(|a| {
            if *a == "-u" { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('u')
        });
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_u {
            if let Some(key) = non_flag.first() {
                let _ = tx.send(CtrlReq::UnsetEnvironment(key.to_string()));
            }
        } else if non_flag.len() >= 2 {
            let _ = tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), non_flag[1].to_string()));
        } else if non_flag.len() == 1 {
            let _ = tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), String::new()));
        }
    }
    "show-environment" | "showenv" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowEnvironment(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-environment".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "set-hook" => {
        let has_unset = args.iter().any(|a| *a == "-u" || *a == "-gu" || *a == "-ug");
        let has_append = args.iter().any(|a| *a == "-a" || *a == "-ga" || *a == "-ag");
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if has_unset {
            // set-hook -gu <hook-name>  →  remove the hook
            if let Some(name) = non_flag.first() {
                let _ = tx.send(CtrlReq::RemoveHook(name.to_string()));
            }
        } else if non_flag.len() >= 2 {
            // Extract hook command from raw line to preserve quoting
            // (join of parsed tokens loses quotes around paths with spaces)
            let hook_name = non_flag[0];
            let hook_cmd = if let Some(pos) = line.find(hook_name) {
                line[pos + hook_name.len()..].trim().to_string()
            } else {
                non_flag[1..].join(" ")
            };
            if has_append {
                let _ = tx.send(CtrlReq::AppendHook(hook_name.to_string(), hook_cmd));
            } else {
                let _ = tx.send(CtrlReq::SetHook(hook_name.to_string(), hook_cmd));
            }
        }
    }
    "show-hooks" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowHooks(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-hooks".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "wait-for" => {
        let lock = args.iter().any(|a| *a == "-L");
        let signal = args.iter().any(|a| *a == "-S");
        let unlock = args.iter().any(|a| *a == "-U");
        let channel = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
        let op = if lock { WaitForOp::Lock }
            else if signal { WaitForOp::Signal }
            else if unlock { WaitForOp::Unlock }
            else { WaitForOp::Wait };
        let _ = tx.send(CtrlReq::WaitFor(channel, op));
    }
    "display-menu" | "menu" => {
        let mut x_pos: Option<i16> = None;
        let mut y_pos: Option<i16> = None;
        let mut title = String::new();
        let mut skip_indices = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-x" => { if let Some(v) = args.get(i+1) { x_pos = v.parse().ok(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-y" => { if let Some(v) = args.get(i+1) { y_pos = v.parse().ok(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-T" => { if let Some(v) = args.get(i+1) { title = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                _ => {}
            }
            i += 1;
        }
        // Collect remaining positional args (name, key, command triplets)
        let positional: Vec<&str> = args.iter().enumerate()
            .filter(|(idx, a)| !skip_indices.contains(idx) && !a.starts_with('-'))
            .map(|(_, a)| *a).collect();
        // Build menu from triplets
        let mut menu = crate::types::Menu { title, items: Vec::new(), selected: 0, x: x_pos, y: y_pos };
        let mut pi = 0;
        while pi < positional.len() {
            let name = positional[pi];
            if name.is_empty() || name == "-" {
                menu.items.push(crate::types::MenuItem { name: String::new(), key: None, command: String::new(), is_separator: true });
                pi += 1;
            } else {
                let key = positional.get(pi + 1).and_then(|k| k.chars().next());
                let command = positional.get(pi + 2).map(|c| c.to_string()).unwrap_or_default();
                menu.items.push(crate::types::MenuItem { name: name.to_string(), key, command, is_separator: false });
                pi += 3;
            }
        }
        if !menu.items.is_empty() {
            let _ = tx.send(CtrlReq::DisplayMenuDirect(menu));
        }
    }
    "new-pane" | "newp" => {
        let p = parse_new_pane_args(&args);
        if p.print {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::NewFloat { command: p.command, x: p.x, y: p.y, w: p.w, h: p.h, border: p.border, title: p.title, start_dir: p.start_dir, detached: p.detached, empty: p.empty, resp: Some(rtx) });
            if let Ok(text) = rrx.recv_timeout(Duration::from_millis(2000)) {
                let _ = write!(write_stream, "{}\n", text);
                let _ = write_stream.flush();
            }
            if !persistent { break; }
        } else {
            let _ = tx.send(CtrlReq::NewFloat { command: p.command, x: p.x, y: p.y, w: p.w, h: p.h, border: p.border, title: p.title, start_dir: p.start_dir, detached: p.detached, empty: p.empty, resp: None });
        }
    }
    "display-popup" | "popup" => {
        // Default close-on-exit = true (tmux parity: popup closes when command finishes)
        let close_on_exit = !args.iter().any(|a| *a == "-K");
        let mut width_spec = "80".to_string();
        let mut height_spec = "24".to_string();
        let mut start_dir: Option<String> = None;
        let mut skip_indices = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" => { if let Some(v) = args.get(i+1) { width_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-h" => { if let Some(v) = args.get(i+1) { height_spec = v.to_string(); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-d" | "-c" => { if let Some(v) = args.get(i+1) { start_dir = Some(v.to_string()); skip_indices.insert(i); skip_indices.insert(i+1); i += 1; } }
                "-E" | "-K" => { skip_indices.insert(i); }
                _ => {}
            }
            i += 1;
        }
        let content = args.iter().enumerate().filter(|(idx, _)| !skip_indices.contains(idx)).map(|(_, a)| *a).collect::<Vec<&str>>().join(" ");
        let _ = tx.send(CtrlReq::DisplayPopup(content, width_spec, height_spec, close_on_exit, start_dir));
    }
    "confirm-before" | "confirm" => {
        let mut prompt: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-p" {
                if let Some(p) = args.get(i+1) { prompt = Some(p.to_string()); i += 1; }
            }
            i += 1;
        }
        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-') && Some(&a.to_string()) != prompt.as_ref()).copied().collect();
        let command = non_flag.join(" ");
        let prompt_str = prompt.unwrap_or_else(|| format!("Run '{}'", command));
        let _ = tx.send(CtrlReq::ConfirmBefore(prompt_str, command));
    }
    // tmux standard aliases (issue #275: full -a/-s/-t/-P parity)
    "detach-client" | "detach" => {
        let kill_parent = args.iter().any(|a| *a == "-P");
        let detach_all_others = args.iter().any(|a| *a == "-a");
        // -s <session> targets a specific session.  We're already routed to this
        // server (one server per session), so -s anything is honored by detaching
        // every client of this session.
        let detach_session = args.windows(2).any(|w| w[0] == "-s");
        // -t <target>: numeric ID, %ID, or tty_name like "/dev/pts/2"
        let target_str = raw_target.clone();
        let target_cid_numeric: Option<u64> = target_str.as_ref()
            .and_then(|t| t.trim_start_matches('%').parse::<u64>().ok());

        if detach_session {
            let _ = tx.send(CtrlReq::DetachAllClients(kill_parent));
            // This client is part of the session, so it will be detached too.
            attached_sent = false;
        } else if detach_all_others {
            let _ = tx.send(CtrlReq::DetachAllOtherClients(client_id, kill_parent));
            // Current client stays attached.
        } else if let Some(cid) = target_cid_numeric {
            if cid == client_id {
                if kill_parent {
                    let _ = crate::types::send_directive_to_client(client_id, "DETACH-KILL-PARENT");
                }
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
                attached_sent = false;
            } else {
                if kill_parent {
                    let _ = crate::types::send_directive_to_client(cid, "DETACH-KILL-PARENT");
                }
                let _ = tx.send(CtrlReq::ForceDetachClient(cid));
            }
        } else if let Some(tty) = target_str {
            // Non-numeric -t value: treat as a tty_name lookup.
            let _ = tx.send(CtrlReq::ForceDetachClientByTty(tty, kill_parent));
        } else {
            // No flags, no -t: detach THIS client.
            if kill_parent {
                let _ = crate::types::send_directive_to_client(client_id, "DETACH-KILL-PARENT");
            }
            let _ = tx.send(CtrlReq::ClientDetach(client_id));
            attached_sent = false;
        }
    }
    "attach-session" | "attach" => {
        if !attached_sent {
            let _ = tx.send(CtrlReq::ClientAttach(client_id));
            attached_sent = true;
        }
    }
    "kill-server" => { let _ = tx.send(CtrlReq::KillServer); }
    "choose-tree" | "choose-window" | "choose-session" => {
        // These are interactive choosers — send a dump that client handles
        // For now, map to listing which the client renders as a chooser
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListTree(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("choose-tree".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "copy-mode" => {
        if args.iter().any(|a| *a == "-u") {
            let _ = tx.send(CtrlReq::CopyEnterPageUp);
        } else {
            let _ = tx.send(CtrlReq::CopyEnter);
        }
    }
    "clock-mode" => { let _ = tx.send(CtrlReq::ClockMode); }
    // Overlay interaction commands (sent by client during active overlays)
    "popup-input" => {
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = tx.send(CtrlReq::PopupInput(decoded.into_bytes()));
            }
        }
    }
    "popup-input-raw" => {
        // Raw bytes (not base64) for single-byte key sequences
        if let Some(encoded) = args.get(0) {
            if let Some(decoded) = base64_decode(encoded) {
                let _ = tx.send(CtrlReq::PopupInput(decoded.into_bytes()));
            }
        }
    }
    "overlay-close" => { let _ = tx.send(CtrlReq::OverlayClose); }
    "display-panes-select" => {
        if let Some(idx) = args.get(0).and_then(|s| s.parse::<usize>().ok()) {
            let _ = tx.send(CtrlReq::DisplayPaneSelect(idx));
        }
    }
    "confirm-respond" => {
        let yes = args.get(0).map(|a| *a == "y" || *a == "yes").unwrap_or(false);
        let _ = tx.send(CtrlReq::ConfirmRespond(yes));
    }
    "menu-select" => {
        if let Some(idx) = args.get(0).and_then(|s| s.parse::<usize>().ok()) {
            let _ = tx.send(CtrlReq::MenuSelect(idx));
        }
    }
    "menu-navigate" => {
        let delta = args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::MenuNavigate(delta));
    }
    "customize-navigate" => {
        let delta = args.get(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let _ = tx.send(CtrlReq::CustomizeNavigate(delta));
    }
    "customize-edit" => {
        let _ = tx.send(CtrlReq::CustomizeEdit);
    }
    "customize-edit-update" => {
        let text = args.join(" ");
        let _ = tx.send(CtrlReq::CustomizeEditUpdate(text));
    }
    "customize-edit-confirm" => {
        let _ = tx.send(CtrlReq::CustomizeEditConfirm);
    }
    "customize-edit-cancel" => {
        let _ = tx.send(CtrlReq::CustomizeEditCancel);
    }
    "customize-reset-default" => {
        let _ = tx.send(CtrlReq::CustomizeResetDefault);
    }
    "customize-filter" => {
        let text = args.join(" ");
        let _ = tx.send(CtrlReq::CustomizeFilter(text));
    }
    "show-messages" | "showmsgs" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ShowMessages(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("show-messages".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "command-prompt" => {
        let initial = args.windows(2).find(|w| w[0] == "-I").map(|w| w[1].to_string()).unwrap_or_default();
        let _ = tx.send(CtrlReq::CommandPrompt(initial));
    }
    "run-shell" | "run" => {
        let background = args.iter().any(|a| *a == "-b");
        // Only strip run-shell's OWN "-b" flag. A blind `!a.starts_with('-')`
        // filter here also deleted flags belonging to the wrapped shell
        // command itself (e.g. `run-shell psmux new-window -n X -c 'dir'`
        // lost both -n and -c, `run-shell psmux set-option -g @o v` lost
        // -g), silently mangling the command into positional garbage. This
        // was the root cause of #402's "[A] command-prompt single-quoted -c
        // FAILS" case: the interactive command-prompt/TCP path reaches this
        // arm directly, while the stored-bind path (fixed in eb5bee1) goes
        // through commands.rs::execute_command_string_single, which already
        // does an exact "-b" match instead of a starts_with prefix filter.
        let cmd_parts: Vec<&str> = args.iter().filter(|a| **a != "-b").copied().collect();
        let shell_cmd = cmd_parts.join(" ");
        // Strip only a BALANCED pair of outer wrapping quotes, e.g.
        // run-shell "'~/plugins/foo.tmux'" -> ~/plugins/foo.tmux.
        // A blind trim_matches here was the root cause of #402: it also removed
        // a lone TRAILING quote when the command's last argument was legitimately
        // quoted (e.g. `psmux new-window -c 'C:\path'` or `pwsh -Command "..."`),
        // producing an unterminated-string parse error in the spawned shell so the
        // command silently never ran. Only unwrap when both ends match the same quote.
        let trimmed = shell_cmd.trim();
        let shell_cmd = if trimmed.len() >= 2
            && ((trimmed.starts_with('\'') && trimmed.ends_with('\''))
                || (trimmed.starts_with('"') && trimmed.ends_with('"')))
        {
            trimmed[1..trimmed.len() - 1].to_string()
        } else {
            trimmed.to_string()
        };
        // Expand #{...} against live server state (tmux parity). This thread has
        // no &AppState — it belongs to the server loop — so the expansion makes
        // a round trip over the control channel. Only pay for it when the
        // command actually contains a format reference.
        //
        // Without this, `run-shell "helper '#{pane_id}'"` passed the helper the
        // literal text `#{pane_id}`; with `-b` swallowing the spawn result, the
        // bind then failed completely silently.
        let shell_cmd = if shell_cmd.contains("#{") {
            let (rtx, rrx) = mpsc::channel::<String>();
            if tx.send(CtrlReq::ExpandFormat(shell_cmd.clone(), rtx)).is_ok() {
                // On timeout fall back to the unexpanded string: running the
                // command with a literal #{...} is no worse than the old
                // behaviour, and better than dropping it silently.
                rrx.recv_timeout(Duration::from_secs(5)).unwrap_or(shell_cmd)
            } else {
                shell_cmd
            }
        } else {
            shell_cmd
        };
        // Expand ~ to home directory + XDG fallback for plugin paths
        let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd);
        if shell_cmd.is_empty() {
            if !persistent {
                let _ = write!(write_stream, "usage: run-shell [-b] shell-command\n");
                let _ = write_stream.flush();
            }
        } else {
            if background {
                let mut c = crate::commands::build_run_shell_command(&shell_cmd);
                // `-b` means "don't wait for it", not "don't tell me it never
                // started". Swallowing this made a broken background bind
                // indistinguishable from an unbound key.
                if let Err(e) = c.spawn() {
                    // No trailing newline in the message itself: StatusMessage is
                    // stored verbatim in app.status_message and rendered in the
                    // status bar, where a stray newline corrupts the line. The
                    // newline belongs only on the stream write.
                    let err_msg = format!("run-shell: {}: {}", shell_cmd, e);
                    if persistent {
                        let _ = tx.send(CtrlReq::StatusMessage(err_msg));
                    } else {
                        let _ = write!(write_stream, "{}\n", err_msg);
                        let _ = write_stream.flush();
                    }
                }
            } else {
                let mut c = crate::commands::build_run_shell_command(&shell_cmd);
                let result = c.output();
                match result {
                    Ok(out) => {
                        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                        let stderr_text = String::from_utf8_lossy(&out.stderr);
                        if !stderr_text.is_empty() {
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            text.push_str(&stderr_text);
                        }
                        if !text.is_empty() {
                            if persistent {
                                let _ = tx.send(CtrlReq::ShowTextPopup("run-shell".to_string(), text));
                            } else {
                                let _ = write!(write_stream, "{}", text);
                                let _ = write_stream.flush();
                            }
                        }
                    }
                    Err(e) => {
                        // Same newline handling as the -b path above (this one
                        // predates the branch; fixed alongside for consistency).
                        let err_msg = format!("run-shell: {}", e);
                        if persistent {
                            let _ = tx.send(CtrlReq::StatusMessage(err_msg));
                        } else {
                            let _ = write!(write_stream, "{}\n", err_msg);
                            let _ = write_stream.flush();
                        }
                    }
                }
            }
        }
    }
    "if-shell" | "if" => {
        let format_mode = args.iter().any(|a| *a == "-F" || *a == "-bF" || *a == "-Fb");
        // Collect positional args (skip flags like -b, -F, -bF),
        // collapsing brace blocks { ... } into single tokens.
        let mut positional: Vec<String> = Vec::new();
        {
            let non_flags: Vec<&str> = args.iter()
                .filter(|a| !a.starts_with('-'))
                .copied()
                .collect();
            let mut j = 0;
            while j < non_flags.len() {
                if non_flags[j] == "{" {
                    // Collect everything between { and } as a single command
                    let mut depth = 1;
                    let mut block = Vec::new();
                    j += 1;
                    while j < non_flags.len() && depth > 0 {
                        if non_flags[j] == "{" { depth += 1; }
                        else if non_flags[j] == "}" { depth -= 1; if depth == 0 { break; } }
                        block.push(non_flags[j]);
                        j += 1;
                    }
                    positional.push(block.join(" "));
                } else {
                    positional.push(non_flags[j].to_string());
                }
                j += 1;
            }
        }
        if positional.len() >= 2 {
            let condition = &positional[0];
            let true_cmd = &positional[1];
            let false_cmd = positional.get(2);
            let success = if format_mode {
                let (rtx, rrx) = std::sync::mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::DisplayMessage(rtx, condition.to_string(), None, false, None));
                let expanded = rrx.recv().unwrap_or_default();
                !expanded.is_empty() && expanded != "0"
            } else if condition == "true" || condition == "1" {
                true
            } else if condition == "false" || condition == "0" {
                false
            } else {
                // Use resolve_run_shell for consistent shell fallback
                let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
                let mut c = std::process::Command::new(&shell_prog);
                for a in &shell_args { c.arg(a); }
                c.arg(condition.as_str());
                c.stdout(std::process::Stdio::null());
                c.stderr(std::process::Stdio::null());
                { use crate::platform::HideWindowCommandExt; c.hide_window(); }
                c.status().map(|s| s.success()).unwrap_or(false)
            };
            let cmd_to_run = if success { Some(true_cmd) } else { false_cmd };
            if let Some(chosen) = cmd_to_run {
                // Feed the chosen command back into the line buffer so the
                // main dispatch loop processes it as a regular command.
                line.clear();
                line.push_str(chosen);
                line.push('\n');
                continue;  // re-enter the dispatch loop with the new command
            }
        }
    }
    "list-sessions" | "ls" => {
        let fmt = extract_flag_value(&args, "-F");
        if let Some(fmt_str) = fmt {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt_str, None, false, None));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-sessions".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        } else {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::SessionInfo(rtx));
            if let Ok(text) = rrx.recv() {
                if persistent {
                    let _ = tx.send(CtrlReq::ShowTextPopup("list-sessions".to_string(), text));
                } else {
                    let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
                }
            }
        }
        if !persistent { break; }
    }
    "new-session" | "new" => {
        // new-session -t target: set session group on this server
        if let Some(target) = args.windows(2).find(|w| w[0] == "-t").map(|w| w[1].to_string()) {
            let _ = tx.send(CtrlReq::SetSessionGroup(target));
        } else {
            // Issue #200: spawn a new session from inside a running session.
            // Parse flags
            let mut sess_name: Option<String> = None;
            let mut detached = false;
            let mut window_name: Option<String> = None;
            let mut start_dir: Option<String> = None;
            let mut init_width: Option<String> = None;
            let mut init_height: Option<String> = None;
            let mut env_vars: Vec<(String, String)> = Vec::new();
            let mut env_parse_err: Option<String> = None;
            let mut initial_command: Option<String> = None;
            {
                let mut i = 0;
                while i < args.len() {
                    match args[i] {
                        "-s" => { i += 1; if i < args.len() { sess_name = Some(args[i].trim_matches('"').to_string()); } }
                        "-n" => { i += 1; if i < args.len() { window_name = Some(args[i].trim_matches('"').to_string()); } }
                        "-c" => { i += 1; if i < args.len() { start_dir = Some(args[i].trim_matches('"').to_string()); } }
                        "-x" => { i += 1; if i < args.len() { init_width = Some(args[i].to_string()); } }
                        "-y" => { i += 1; if i < args.len() { init_height = Some(args[i].to_string()); } }
                        "-e" => {
                            i += 1;
                            match crate::util::parse_new_session_e_value_token(args.get(i).copied()) {
                                Ok(p) => env_vars.push(p),
                                Err(e) => {
                                    env_parse_err = Some(e);
                                    break;
                                }
                            }
                        }
                        "-d" => { detached = true; }
                        "-t" => { i += 1; /* already handled above */ }
                        "-F" | "-f" => { i += 1; /* skip value */ }
                        other => {
                            // Positional arg: initial shell command (issue #229)
                            if !other.starts_with('-') {
                                initial_command = Some(args[i..].iter().map(|s| s.trim_matches('"').to_string()).collect::<Vec<_>>().join(" "));
                                break;
                            }
                        }
                    }
                    i += 1;
                }
            }

            if let Some(ref err) = env_parse_err {
                let msg = format!("psmux: {}\n", err);
                if persistent {
                    let _ = tx.send(CtrlReq::StatusMessage(msg.trim().to_string()));
                } else {
                    let _ = write!(write_stream, "{}", msg);
                    let _ = write_stream.flush();
                }
                if !persistent { break; }
            } else {

            // Note: socket_name (from -L flag) is not directly available here;
            // the client-side handler in commands.rs has it via app.socket_name.
            let name = sess_name.unwrap_or_else(|| crate::session::next_session_name(None));

            let port_file_base = name.clone();

            let port_path = crate::paths::port_file(&port_file_base);

            // Check if session already exists
            let already_exists = if std::path::Path::new(&port_path).exists() {
                if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                    if let Ok(port) = port_str.trim().parse::<u16>() {
                        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
                        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(100)).is_ok()
                    } else { false }
                } else { false }
            } else { false };

            if already_exists {
                if persistent {
                    let _ = tx.send(CtrlReq::StatusMessage(format!("session '{}' already exists", name)));
                } else {
                    let _ = write!(write_stream, "session '{}' already exists\n", name);
                    let _ = write_stream.flush();
                    break;
                }
            } else {
                // Spawn new server
                let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
                let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), name.clone()];

                if let Some(ref dir) = start_dir {
                    server_args.push("-d".into());
                    server_args.push(dir.clone());
                }
                if let Some(ref wn) = window_name {
                    server_args.push("-n".into());
                    server_args.push(wn.clone());
                }
                // Pass initial command to server (issue #229)
                if let Some(ref cmd) = initial_command {
                    server_args.push("-c".into());
                    server_args.push(cmd.clone());
                }
                // Pass -x/-y initial dimensions to server
                if let Some(ref w) = init_width {
                    server_args.push("-x".into());
                    server_args.push(w.clone());
                }
                if let Some(ref h) = init_height {
                    server_args.push("-y".into());
                    server_args.push(h.clone());
                }
                // Pass -e environment variables to server
                for (k, v) in &env_vars {
                    server_args.push("-e".into());
                    server_args.push(format!("{}={}", k, v));
                }
                #[cfg(windows)]
                { let _ = crate::platform::spawn_server_hidden(&exe, &server_args); }
                #[cfg(not(windows))]
                {
                    let mut cmd_proc = std::process::Command::new(&exe);
                    for a in &server_args { cmd_proc.arg(a); }
                    cmd_proc.stdin(std::process::Stdio::null());
                    cmd_proc.stdout(std::process::Stdio::null());
                    cmd_proc.stderr(std::process::Stdio::null());
                    let _ = cmd_proc.spawn();
                }

                // Wait for port file
                for _ in 0..500 {
                    if std::path::Path::new(&port_path).exists() { break; }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }

                if std::path::Path::new(&port_path).exists() {
                    if !detached {
                        let _ = tx.send(CtrlReq::SwitchClient(name.clone(), 't'));
                    }
                    if persistent {
                        let _ = tx.send(CtrlReq::StatusMessage(format!("created session '{}'", name)));
                    } else {
                        let _ = write!(write_stream, "OK\n");
                        let _ = write_stream.flush();
                    }
                } else {
                    if persistent {
                        let _ = tx.send(CtrlReq::StatusMessage(format!("failed to create session '{}'", name)));
                    } else {
                        let _ = write!(write_stream, "failed to create session '{}'\n", name);
                        let _ = write_stream.flush();
                    }
                }
            }
            } // env_parse_err else
        }
    }
    "list-commands" | "lscm" => {
        let cmds = TMUX_COMMANDS.join("\n");
        if persistent {
            let _ = tx.send(CtrlReq::ShowTextPopup("list-commands".to_string(), cmds));
        } else {
            let _ = write!(write_stream, "{}\n", cmds);
            let _ = write_stream.flush();
        }
        if !persistent { break; }
    }
    "server-info" | "info" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ServerInfo(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("server-info".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}\n", text); let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "start-server" => {
        // Server is already running if we're here, no-op
        if !persistent { break; }
    }
    "send-prefix" => {
        let _ = tx.send(CtrlReq::SendPrefix);
    }
    "previous-layout" | "prevl" => {
        let _ = tx.send(CtrlReq::PrevLayout);
    }
    "resize-window" | "resizew" => {
        match crate::resize_window::parse_resize_window(&args, raw_target.as_deref()) {
            Ok(request) => {
                let (resize_tx, resize_rx) = mpsc::channel();
                if tx.send(CtrlReq::ResizeWindow(request, resize_tx)).is_ok() {
                    match resize_rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            if persistent {
                                let _ = tx.send(CtrlReq::StatusMessage(error));
                            } else {
                                let _ = writeln!(write_stream, "ERROR: {}", error);
                                let _ = write_stream.flush();
                            }
                        }
                        Err(_) if !persistent => {
                            let _ = writeln!(write_stream, "ERROR: resize-window timed out");
                            let _ = write_stream.flush();
                        }
                        Err(_) => {}
                    }
                }
            }
            Err(error) => {
                if persistent {
                    let _ = tx.send(CtrlReq::StatusMessage(error));
                } else {
                    let _ = writeln!(write_stream, "ERROR: {}", error);
                    let _ = write_stream.flush();
                }
            }
        }
    }
    "respawn-window" | "respawnw" => {
        let _ = tx.send(CtrlReq::RespawnWindow);
    }
    "lock-server" | "lock-session" | "lock" | "locks" => {
        // Lock is a no-op on Windows (no terminal locking concept)
        // Stub for compatibility
    }
    "focus-in" => { let _ = tx.send(CtrlReq::FocusIn); }
    "focus-out" => { let _ = tx.send(CtrlReq::FocusOut); }
    "choose-client" => {
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::ListClients(rtx));
        if let Ok(text) = rrx.recv() {
            if persistent {
                let _ = tx.send(CtrlReq::ShowTextPopup("choose-client".to_string(), text));
            } else {
                let _ = write!(write_stream, "{}", text);
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    "customize-mode" => {
        // tmux 3.2+ customize-mode: interactive options editor
        let _ = tx.send(CtrlReq::CustomizeMode);
    }
    "clear-prompt-history" | "clearphist" => {
        let _ = tx.send(CtrlReq::ClearPromptHistory);
    }
    "show-prompt-history" | "showphist" => {
        let _ = tx.send(CtrlReq::ShowPromptHistory(persistent));
    }
    "server-access" => {
        // Multi-user server access — not applicable to psmux
    }
    "run-command" | "runcmd" => {
        // Route command through the server-side execute_command_string path
        // (same code path as keybindings and command prompt).
        let full_cmd = args.join(" ");
        let (rtx, rrx) = mpsc::channel::<String>();
        let _ = tx.send(CtrlReq::RunCommand(full_cmd, rtx));
        if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(15)) {
            if persistent {
                let _ = tx.send(CtrlReq::StatusMessage(resp));
            } else {
                let _ = write!(write_stream, "{}\n", resp);
                let _ = write_stream.flush();
            }
        }
        if !persistent { break; }
    }
    _ => {}
}
    // Process pending chained commands before reading from socket
    if !pending_chain.is_empty() {
        line = pending_chain.remove(0);
        continue;
    }
    // Try to read next command for batching (with timeout)
    line.clear();
    match r.read_line(&mut line) {
        Ok(0) => {
            // EOF - client disconnected
            if attached_sent {
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
            }
            break;
        }
        Err(e) => {
            if persistent && (e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut) {
                line.clear(); // Clear any partial data from interrupted read
                continue; // Persistent mode - keep waiting
            }
            if attached_sent {
                let _ = tx.send(CtrlReq::ClientDetach(client_id));
            }
            break; // Non-persistent timeout or real error
        }
        Ok(_) => {} // Continue processing
    }
} // end command loop
}

fn recv_control_response<T>(
    command: &str,
    receiver: mpsc::Receiver<T>,
    timeout: Duration,
) -> Result<T, String> {
    receiver.recv_timeout(timeout).map_err(|error| match error {
        mpsc::RecvTimeoutError::Timeout => format!("{} timed out", command),
        mpsc::RecvTimeoutError::Disconnected => {
            format!("{} response channel disconnected", command)
        }
    })
}

fn forward_control_response(
    command: &str,
    receiver: mpsc::Receiver<String>,
    response_tx: &mpsc::Sender<ControlCommandResponse>,
    timeout: Duration,
) {
    let response = match recv_control_response(command, receiver, timeout) {
        Ok(body) => ControlCommandResponse::success(body),
        Err(error) => ControlCommandResponse::error(error),
    };
    let _ = response_tx.send(response);
}

/// Dispatch a command from a control mode client.
/// Returns true when the caller must wait for exactly one response through
/// `resp_tx`, or false for a fire-and-forget command.
fn dispatch_control_command(
    cmd: &str,
    args: &[&str],
    tx: &mpsc::Sender<CtrlReq>,
    resp_tx: mpsc::Sender<ControlCommandResponse>,
    target_pane: Option<usize>,
    pane_is_id: bool,
    raw_target: Option<&str>,
    client_id: u64,
) -> bool {
    match cmd {
        "list-windows" | "lsw" => {
            let format_str = extract_flag_value(&args, "-F");
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt) = format_str {
                let _ = tx.send(CtrlReq::ListWindowsFormat(rtx, fmt));
            } else {
                let _ = tx.send(CtrlReq::ListWindowsTmux(rtx));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "list-panes" | "lsp" => {
            let all = args.iter().any(|a| *a == "-a");
            let session_scope = args.iter().any(|a| *a == "-s");
            let format_str = extract_flag_value(&args, "-F");
            let (rtx, rrx) = mpsc::channel::<String>();
            if all || session_scope {
                if let Some(fmt) = format_str {
                    let _ = tx.send(CtrlReq::ListAllPanesFormat(rtx, fmt));
                } else {
                    let _ = tx.send(CtrlReq::ListAllPanes(rtx));
                }
            } else {
                if let Some(fmt) = format_str {
                    let _ = tx.send(CtrlReq::ListPanesFormat(rtx, fmt));
                } else {
                    let _ = tx.send(CtrlReq::ListPanes(rtx));
                }
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "display-message" | "display" => {
            let print_mode = args.iter().any(|a| *a == "-p");
            let raw_fmt = args.last().map(|s| s.trim_matches('"').to_string()).unwrap_or_default();
            let fmt = if raw_fmt.is_empty() {
                crate::commands::DISPLAY_MESSAGE_DEFAULT_FMT.to_string()
            } else {
                raw_fmt
            };
            let target_pane_idx = if pane_is_id { None } else { target_pane };
            let (rtx, rrx) = mpsc::channel::<String>();
            if pane_is_id {
                if let Some(pid) = target_pane {
                    let _ = tx.send(CtrlReq::DisplayMessageById(rtx, fmt, pid, !print_mode, None));
                } else {
                    let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_mode, None));
                }
            } else {
                let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt, target_pane_idx, !print_mode, None));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "new-pane" | "newp" => {
            let p = parse_new_pane_args(args);
            if p.print {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::NewFloat { command: p.command, x: p.x, y: p.y, w: p.w, h: p.h, border: p.border, title: p.title, start_dir: p.start_dir, detached: p.detached, empty: p.empty, resp: Some(rtx) });
                forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(2));
                true
            } else {
                let _ = tx.send(CtrlReq::NewFloat { command: p.command, x: p.x, y: p.y, w: p.w, h: p.h, border: p.border, title: p.title, start_dir: p.start_dir, detached: p.detached, empty: p.empty, resp: None });
                false
            }
        }
        "new-window" | "neww" => {
            let name = args.windows(2).find(|w| w[0] == "-n").map(|w| w[1].trim_matches('"').to_string());
            let start_dir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
            let detached = crate::cli::has_short_flag(&args, 'd');
            let print_info = crate::cli::has_short_flag(&args, 'P');
            let format_str = extract_flag_value(&args, "-F").map(|s| s.trim_matches('"').to_string());
            let title = extract_flag_value(&args, "-T").map(|s| s.trim_matches('"').to_string());
            let empty = args.iter().any(|a| *a == "-E");
            // Skip arg if it's a flag, the value of a flag, or a flag-cluster
            // value (e.g. the format string after `-PF`).
            let mut skip: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for (i, a) in args.iter().enumerate() {
                if a.starts_with('-') && !a.starts_with("--") {
                    skip.insert(i);
                    // Two-token forms: next arg is the value
                    if matches!(*a, "-n" | "-c" | "-F" | "-t" | "-x" | "-y" | "-e" | "-T") {
                        skip.insert(i + 1);
                    } else if a.len() > 2
                        && a.chars().skip(1).all(|c| c.is_ascii_alphabetic())
                        && matches!(a.chars().last(), Some('n') | Some('c') | Some('F') | Some('t') | Some('x') | Some('y') | Some('e') | Some('T'))
                    {
                        // Cluster ending in value-taking flag: -PF <value>
                        skip.insert(i + 1);
                    }
                }
            }
            let cmd_str: Option<String> = args.iter().enumerate()
                .find(|(i, _)| !skip.contains(i))
                .map(|(_, s)| s.trim_matches('"').to_string());
            // -e KEY=VALUE environment for the new pane (tmux parity, #489).
            let env_sets: Vec<(String, String)> = args.windows(2)
                .filter(|w| w[0] == "-e")
                .filter_map(|w| w[1].trim_matches('"').split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            if print_info {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::NewWindowPrint(cmd_str, name, detached, start_dir, format_str, rtx, title, empty, env_sets));
                forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
                true
            } else {
                let _ = tx.send(CtrlReq::NewWindow(cmd_str, name, detached, start_dir, title, empty, env_sets));
                let _ = resp_tx.send(ControlCommandResponse::empty());
                true
            }
        }
        "split-window" | "splitw" | "split-pane" | "splitp" => {
            let kind = if crate::cli::has_short_flag(&args, 'h') {
                LayoutKind::Horizontal
            } else {
                LayoutKind::Vertical
            };
            let cmd_str = args.windows(2).find(|w| w[0] == "-c").map(|_| ()).and(None);
            let start_dir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].trim_matches('"').to_string());
            let detached = crate::cli::has_short_flag(&args, 'd');
            let print_info = crate::cli::has_short_flag(&args, 'P');
            let format_str = extract_flag_value(&args, "-F").map(|s| s.trim_matches('"').to_string());
            let title = extract_flag_value(&args, "-T").map(|s| s.trim_matches('"').to_string());
            // -p N = percentage, -l N = cell count, -l N% = percentage (tmux semantics)
            let split_size: Option<(u16, bool)> = args.windows(2).find(|w| w[0] == "-p")
                .and_then(|w| w[1].trim_end_matches('%').parse::<u16>().ok())
                .map(|v| (v, true))
                .or_else(|| args.windows(2).find(|w| w[0] == "-l")
                    .and_then(|w| {
                        let raw = &w[1];
                        let is_pct = raw.ends_with('%');
                        raw.trim_end_matches('%').parse::<u16>().ok().map(|v| (v, is_pct))
                    }));
            // -e KEY=VALUE environment for the new pane (tmux parity, #489).
            let env_sets: Vec<(String, String)> = args.windows(2)
                .filter(|w| w[0] == "-e")
                .filter_map(|w| w[1].trim_matches('"').split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            let (rtx, rrx) = mpsc::channel::<String>();
            if print_info {
                let _ = tx.send(CtrlReq::SplitWindowPrint(kind, cmd_str, detached, start_dir, split_size, format_str, rtx, title, env_sets));
            } else {
                let _ = tx.send(CtrlReq::SplitWindow(kind, cmd_str, detached, start_dir, split_size, rtx, title, env_sets));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "send-keys" | "send" => {
            let flag_has = |c: char| -> bool {
                args.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.chars().skip(1).any(|fc| fc == c))
            };
            let prev_consumes_operand = |i: usize| -> bool {
                if i == 0 { return false; }
                if let Some(prev) = args.get(i - 1) {
                    if prev.starts_with('-') && !prev.starts_with("--") && prev.len() >= 2 {
                        if let Some(last) = prev.chars().last() {
                            return matches!(last, 't' | 'T' | 'N' | 'R' | 'c');
                        }
                    }
                }
                false
            };
            let literal = flag_has('l');
            if flag_has('H') {
                // tmux `send-keys -H`: every operand is one hexadecimal BYTE value.
                // tmux tags these keys KEYC_LITERAL and writes them to the pty as
                // single bytes ("can't be more than 8 bits"), so this is a byte
                // channel, not a codepoint channel.  Decode here and pass the raw
                // bytes through untouched so UTF-8 sequences and escape sequences
                // survive verbatim.
                let mut bytes: Vec<u8> = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    if a.starts_with('-') || prev_consumes_operand(i) { continue; }
                    // A malformed operand is dropped rather than leaked into the
                    // pane as its literal token text.
                    if let Ok(byte) = u8::from_str_radix(a, 16) { bytes.push(byte); }
                }
                if !bytes.is_empty() {
                    let _ = tx.send(CtrlReq::SendBytes(bytes));
                }
                let _ = resp_tx.send(ControlCommandResponse::empty());
                return true;
            }
            // Convert real-tmux 0xNN hex codepoint syntax (used by iTerm2 for
            // every keystroke: `send -t %1 0xd` etc.) into literal characters.
            let keys: Vec<String> = args.iter().enumerate().filter(|(i, a)| {
                !a.starts_with('-') && !prev_consumes_operand(*i)
            }).map(|(_, a)| {
                let s = *a;
                if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit()) {
                        if let Ok(n) = u32::from_str_radix(rest, 16) {
                            if let Some(c) = char::from_u32(n) {
                                return c.to_string();
                            }
                        }
                    }
                }
                s.to_string()
            }).collect();
            let any_hex = args.iter().any(|a| {
                let s = *a;
                if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit());
                }
                false
            });
            let effective_literal = literal || any_hex;
            // #490: hand the tokens over UNJOINED so quoted arguments keep
            // their exact whitespace end to end.
            let _ = tx.send(CtrlReq::SendKeys(keys, effective_literal));
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "capture-pane" | "capturep" => {
            let start = args.windows(2).find(|w| w[0] == "-S").and_then(|w| if w[1] == "-" { Some(i32::MIN) } else { w[1].parse::<i32>().ok() });
            let end = args.windows(2).find(|w| w[0] == "-E").and_then(|w| w[1].parse::<i32>().ok());
            let styled = crate::cli::has_short_flag(&args, 'e');
            let (rtx, rrx) = mpsc::channel::<String>();
            if styled {
                let _ = tx.send(CtrlReq::CapturePaneStyled(rtx, start, end));
            } else if start.is_some() || end.is_some() {
                let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start, end));
            } else {
                let _ = tx.send(CtrlReq::CapturePane(rtx));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "kill-pane" | "killp" => {
            if pane_is_id {
                if let Some(pid) = target_pane {
                    let _ = tx.send(CtrlReq::KillPaneById(pid));
                }
            } else {
                let _ = tx.send(CtrlReq::KillPane);
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "kill-window" | "killw" => {
            // Resolve any -t target server-side and error on a miss instead
            // of killing the active window (same fix as the one-shot path).
            let parsed = raw_target.map(parse_target);
            let (t_win, t_win_is_id, t_name) = match parsed {
                Some(pt) => (pt.window, pt.window_is_id, pt.window_name),
                None => (None, false, None),
            };
            if t_win.is_some() || t_name.is_some() {
                let (resp_s, resp_r) = mpsc::channel();
                let _ = tx.send(CtrlReq::KillWindowTarget {
                    win: t_win,
                    win_is_id: t_win_is_id,
                    name: t_name,
                    resp: resp_s,
                });
                match resp_r.recv_timeout(Duration::from_secs(5)) {
                    Ok(Err(e)) => { let _ = resp_tx.send(ControlCommandResponse::error(e)); }
                    _ => { let _ = resp_tx.send(ControlCommandResponse::empty()); }
                }
            } else {
                let _ = tx.send(CtrlReq::KillWindow);
                let _ = resp_tx.send(ControlCommandResponse::empty());
            }
            true
        }
        "unlink-window" | "unlinkw" => {
            let _ = tx.send(CtrlReq::UnlinkWindow);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "select-window" | "selectw" => {
            // Already handled by target focus above
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "select-pane" | "selectp" => {
            // Handle -T title setting
            if let Some(t) = args.windows(2).find(|w| w[0] == "-T").map(|w| w[1].trim_matches('"').to_string()) {
                let _ = tx.send(CtrlReq::SetPaneTitle(t));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "rename-window" | "renamew" => {
            if let Some(name) = args.last() {
                let _ = tx.send(CtrlReq::RenameWindow(name.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "rename-session" | "rename" => {
            if let Some(name) = args.last() {
                let _ = tx.send(CtrlReq::RenameSession(name.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "set-option" | "set" | "set-window-option" | "setw" => {
            // Support combined flag tokens like -ga, -gu, -gq (tmux compat)
            let combined_has_set2 = |ch: char| -> bool {
                args.iter().any(|a| {
                    if *a == format!("-{}", ch) { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
                })
            };
            let quiet = combined_has_set2('q');
            let unset = combined_has_set2('u');
            let append = combined_has_set2('a');
            let global = combined_has_set2('g');
            let only_if_unset = combined_has_set2('o');
            // Skip values that follow target flags. `-w` selects window scope
            // and does not consume the next argument.
            let t_vals2: std::collections::HashSet<&str> = args.windows(2)
                .filter(|w| w[0] == "-t" || w[0] == "-p")
                .map(|w| w[1]).collect();
            let positional: Vec<&str> = args.iter()
                .filter(|a| (!a.starts_with('-') || a.starts_with('@')) && !t_vals2.contains(*a))
                .copied().collect();
            if unset && !positional.is_empty() {
                if positional[0] == "window-size" && !global {
                    let _ = tx.send(CtrlReq::SetWindowSize(None));
                } else {
                    let _ = tx.send(CtrlReq::SetOptionUnset(positional[0].to_string()));
                }
            } else if positional.len() >= 2 {
                let key = positional[0].to_string();
                let val = positional[1].trim_matches('"').to_string();
                if key == "window-size" && !global {
                    let _ = tx.send(CtrlReq::SetWindowSize(Some(val)));
                } else if append {
                    let _ = tx.send(CtrlReq::SetOptionAppend(key, val));
                } else if only_if_unset {
                    let _ = tx.send(CtrlReq::SetOptionOnlyIfUnset(key, val));
                } else if quiet || global {
                    let _ = tx.send(CtrlReq::SetOptionQuiet(key, val, quiet));
                } else {
                    let _ = tx.send(CtrlReq::SetOption(key, val));
                }
            } else if positional.len() == 1 && !unset && !append {
                // Option name, no value (#535), see the matching arm above.
                // Boolean options toggle; anything else is an "empty value"
                // error the CLI has already reported with exit 1.
                if crate::server::options::missing_value_toggles(positional[0]) {
                    let _ = tx.send(CtrlReq::SetOptionToggle(positional[0].to_string()));
                }
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "show-options" | "show" | "show-window-options" | "showw"
        | "show-option" | "show-window-option" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let combined_has2 = |ch: char| -> bool {
                args.iter().any(|a| {
                    if *a == format!("-{}", ch) { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
                })
            };
            let value_only = combined_has2('v');
            let window_scope2 = matches!(cmd, "show-window-options" | "showw" | "show-window-option") || combined_has2('w');
            let opt_name = args.iter().filter(|a| !a.starts_with('-')).next().map(|s| s.to_string());
            let has_opt_name = opt_name.is_some();
            // See issue #266 — same -t window-index extraction as the
            // primary handler above.
            let target_window2: Option<usize> = extract_flag_value(&args, "-t")
                .as_deref()
                .map(parse_target)
                .and_then(|pt| pt.window);
            if let Some(name) = opt_name {
                if value_only {
                    let _ = tx.send(CtrlReq::ShowOptionValue(rtx, name));
                } else if window_scope2 {
                    let _ = tx.send(CtrlReq::ShowWindowOptionValue(rtx, name, target_window2));
                } else {
                    let _ = tx.send(CtrlReq::ShowOptionValue(rtx, name));
                }
            } else if value_only {
                // -v/-gv without option name: list all, values only
                if window_scope2 {
                    let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
                } else {
                    let _ = tx.send(CtrlReq::ShowOptions(rtx));
                }
            } else if window_scope2 {
                let _ = tx.send(CtrlReq::ShowWindowOptions(rtx));
            } else {
                let _ = tx.send(CtrlReq::ShowOptions(rtx));
            }
            match recv_control_response(cmd, rrx, Duration::from_secs(5)) {
                Ok(text) if value_only && !has_opt_name => {
                    // Strip option names, keep values only
                    let values_only: String = text.lines()
                        .filter_map(|line| {
                            let t = line.trim();
                            if t.is_empty() { return None; }
                            if let Some(pos) = t.find(' ') { Some(&t[pos + 1..]) } else { Some(t) }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = resp_tx.send(ControlCommandResponse::success(values_only));
                }
                Ok(text) => {
                    let _ = resp_tx.send(ControlCommandResponse::success(text));
                }
                Err(error) => {
                    let _ = resp_tx.send(ControlCommandResponse::error(error));
                }
            }
            true
        }
        "list-keys" | "lsk" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListKeys(rtx));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "list-sessions" | "ls" => {
            let format_str = extract_flag_value(&args, "-F");
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt) = format_str {
                let _ = tx.send(CtrlReq::SessionInfoFormat(rtx, fmt));
            } else {
                let _ = tx.send(CtrlReq::SessionInfo(rtx));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "list-buffers" | "lsb" => {
            let format_str = extract_flag_value(&args, "-F");
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt) = format_str {
                let _ = tx.send(CtrlReq::ListBuffersFormat(rtx, fmt));
            } else {
                let _ = tx.send(CtrlReq::ListBuffers(rtx));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "show-buffer" | "showb" => {
            let buf_name: Option<String> = args.windows(2).find(|w| w[0] == "-b").map(|w| w[1].to_string());
            let text = if let Some(name) = buf_name {
                let (rtx, rrx) = mpsc::channel::<Option<String>>();
                if let Ok(idx) = name.parse::<usize>() {
                    let _ = tx.send(CtrlReq::ShowBufferAt(rtx, idx));
                } else {
                    let _ = tx.send(CtrlReq::ShowNamedBuffer(rtx, name));
                }
                recv_control_response(cmd, rrx, Duration::from_secs(5))
                    .and_then(|text| text.ok_or_else(|| "buffer not found".to_string()))
            } else {
                let (rtx, rrx) = mpsc::channel::<String>();
                let _ = tx.send(CtrlReq::ShowBuffer(rtx));
                recv_control_response(cmd, rrx, Duration::from_secs(5))
            };
            let response = match text {
                Ok(body) => ControlCommandResponse::success(body),
                Err(error) => ControlCommandResponse::error(error),
            };
            let _ = resp_tx.send(response);
            true
        }
        "has-session" | "has" => {
            let (rtx, rrx) = mpsc::channel::<bool>();
            let _ = tx.send(CtrlReq::HasSession(rtx));
            let response = recv_control_response(cmd, rrx, Duration::from_secs(5))
                .and_then(|exists| {
                    if exists {
                        Ok(String::new())
                    } else {
                        Err("session not found".to_string())
                    }
                })
                .map(ControlCommandResponse::success)
                .unwrap_or_else(ControlCommandResponse::error);
            let _ = resp_tx.send(response);
            true
        }
        "list-clients" | "lsc" => {
            let fmt = extract_flag_value(&args, "-F");
            let (rtx, rrx) = mpsc::channel::<String>();
            if let Some(fmt_str) = fmt {
                let _ = tx.send(CtrlReq::ListClientsFormat(rtx, fmt_str));
            } else {
                let _ = tx.send(CtrlReq::ListClients(rtx));
            }
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "detach-client" | "detach" => {
            // One-shot CLI dispatch path (issue #275).  No "current client" since
            // the caller is a short-lived `psmux detach-client` process, not an
            // attached TUI client.  Default behavior: detach EVERY attached client
            // of this session.
            let kill_parent = args.iter().any(|a| *a == "-P");
            let detach_all = args.iter().any(|a| *a == "-a");
            let detach_session = args.windows(2).any(|w| w[0] == "-s");
            let target_str: Option<String> = extract_flag_value(&args, "-t").map(|s| s.to_string());
            let target_cid_numeric: Option<u64> = target_str.as_ref()
                .and_then(|t| t.trim_start_matches('%').parse::<u64>().ok());

            if let Some(cid) = target_cid_numeric {
                if kill_parent {
                    let _ = crate::types::send_directive_to_client(cid, "DETACH-KILL-PARENT");
                }
                let _ = tx.send(CtrlReq::ForceDetachClient(cid));
            } else if let Some(tty) = target_str {
                let _ = tx.send(CtrlReq::ForceDetachClientByTty(tty, kill_parent));
            } else if detach_all || detach_session {
                // -a from CLI = no current to exclude → detach all.
                let _ = tx.send(CtrlReq::DetachAllClients(kill_parent));
            } else {
                // No flags from CLI: detach all clients of this session.
                let _ = tx.send(CtrlReq::DetachAllClients(kill_parent));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "kill-session" => {
            let _ = tx.send(CtrlReq::KillSession);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "kill-server" => {
            let _ = tx.send(CtrlReq::KillServer);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "select-layout" | "selectl" => {
            if let Some(layout) = args.first() {
                let _ = tx.send(CtrlReq::SelectLayout(layout.to_string()));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "next-layout" | "nextl" => {
            let _ = tx.send(CtrlReq::NextLayout);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "resize-pane" | "resizep" => {
            if args.iter().any(|a| *a == "-Z") {
                let _ = tx.send(CtrlReq::ZoomPane);
            } else if let Some(xval) = args.windows(2).find(|w| w[0] == "-x").map(|w| w[1]) {
                if let Some(pct) = xval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                    let _ = tx.send(CtrlReq::ResizePanePercent("x".to_string(), pct));
                } else if let Ok(abs) = xval.parse::<u16>() {
                    let _ = tx.send(CtrlReq::ResizePaneAbsolute("x".to_string(), abs));
                }
            } else if let Some(yval) = args.windows(2).find(|w| w[0] == "-y").map(|w| w[1]) {
                if let Some(pct) = yval.strip_suffix('%').and_then(|n| n.parse::<u8>().ok()) {
                    let _ = tx.send(CtrlReq::ResizePanePercent("y".to_string(), pct));
                } else if let Ok(abs) = yval.parse::<u16>() {
                    let _ = tx.send(CtrlReq::ResizePaneAbsolute("y".to_string(), abs));
                }
            } else {
                let amount = args.iter().filter(|a| !a.starts_with('-')).next()
                    .and_then(|s| s.parse::<u16>().ok()).unwrap_or(1);
                let dir = if args.iter().any(|a| *a == "-U") { "U" }
                    else if args.iter().any(|a| *a == "-D") { "D" }
                    else if args.iter().any(|a| *a == "-L") { "L" }
                    else if args.iter().any(|a| *a == "-R") { "R" }
                    else { "D" };
                let _ = tx.send(CtrlReq::ResizePane(dir.to_string(), amount));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "swap-pane" | "swapp" => {
            // A `{...}` position token (e.g. {top-right}) resolves to whatever
            // pane currently sits there — layout-independent.  Otherwise resolve
            // an inline `-t <target>` / pre-parsed control target; else directional.
            // -s <src> -t <dst>: swap two explicit panes (#442). Only when both
            // resolve to a concrete pane; otherwise fall through.
            let detach = args.iter().any(|a| *a == "-d");
            let src = args.iter().position(|a| *a == "-s")
                .and_then(|i| args.get(i + 1).copied())
                .map(parse_target)
                .and_then(|pt| pt.pane.map(|p| (p, pt.pane_is_id)));
            let dst_inline = args.iter().position(|a| *a == "-t")
                .and_then(|i| args.get(i + 1).copied())
                .map(parse_target)
                .and_then(|pt| pt.pane.map(|p| (p, pt.pane_is_id)));
            let dst = dst_inline.or_else(|| target_pane.map(|p| (p, pane_is_id)));
            if let (Some((sv, sid)), Some((dv, did))) = (src, dst) {
                let _ = tx.send(CtrlReq::SwapPaneSrcDst { src: sv, src_is_id: sid, dst: dv, dst_is_id: did, detach });
            } else if let Some(tok) = raw_target.filter(|t| t.starts_with('{')) {
                let _ = tx.send(CtrlReq::SwapPanePosition(tok.to_string()));
            } else {
                let resolved = dst;
                if let Some((p, is_id)) = resolved {
                    let _ = tx.send(CtrlReq::SwapPaneTarget(p, is_id));
                } else {
                    let direction = if args.iter().any(|a| *a == "-U") { "U".to_string() }
                                   else if args.iter().any(|a| *a == "-L") { "L".to_string() }
                                   else if args.iter().any(|a| *a == "-R") { "R".to_string() }
                                   else { "D".to_string() };
                    let _ = tx.send(CtrlReq::SwapPane(direction));
                }
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "bind-key" | "bind" => {
            // Parse bind-key's own flags, then treat everything after
            // the key name as the verbatim command (preserving flags like -c).
            let mut table_name = "prefix".to_string();
            let mut repeat = false;
            let mut i = 0;
            while i < args.len() {
                match args[i] {
                    "-T" if i + 1 < args.len() => {
                        table_name = args[i + 1].to_string();
                        i += 2; continue;
                    }
                    "-n" => { table_name = "root".to_string(); i += 1; continue; }
                    "-r" => { repeat = true; i += 1; continue; }
                    _ => break,
                }
            }
            if i < args.len() && i + 1 < args.len() {
                let key = args[i].to_string();
                let command = requote_command_tail(&args[i + 1..]);
                let _ = tx.send(CtrlReq::BindKey(table_name, key, command, repeat));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "unbind-key" | "unbind" => {
            if args.iter().any(|a| *a == "-a" || (a.starts_with('-') && a.contains('a'))) {
                let mut has_table = false;
                let mut table = String::new();
                for (j, a) in args.iter().enumerate() {
                    if *a == "-T" { if let Some(t) = args.get(j + 1) { table = t.to_string(); has_table = true; } }
                    if *a == "-n" { table = "root".to_string(); has_table = true; }
                }
                if has_table {
                    let _ = tx.send(CtrlReq::UnbindAllInTable(table));
                } else {
                    let _ = tx.send(CtrlReq::UnbindAll);
                }
            } else {
                // Parse -n / -T flags for table-specific individual unbind
                let mut table: Option<String> = None;
                let mut t_value_idx: Option<usize> = None;
                let mut target_session_idx: Option<usize> = None;
                for (j, a) in args.iter().enumerate() {
                    if *a == "-T" {
                        if let Some(t) = args.get(j + 1) {
                            table = Some(t.to_string());
                            t_value_idx = Some(j + 1);
                        }
                    }
                    if *a == "-n" { table = Some("root".to_string()); }
                    // -t <session> is the target flag; skip its value
                    if *a == "-t" { target_session_idx = Some(j + 1); }
                }
                // Find the key argument: first non-flag arg that isn't the -T table value
                // or the -t session target value
                let key_arg = args.iter().enumerate()
                    .filter(|(i, a)| !a.starts_with('-') && Some(*i) != t_value_idx && Some(*i) != target_session_idx)
                    .map(|(_, a)| *a)
                    .next();
                if let Some(key) = key_arg {
                    let _ = tx.send(CtrlReq::UnbindKey(key.to_string(), table));
                }
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "source-file" | "source" => {
            if let Some(path) = args.first() {
                let _ = tx.send(CtrlReq::SourceFile(path.trim_matches('"').to_string()));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "set-environment" | "setenv" => {
            let unset = args.iter().any(|a| {
                if *a == "-u" { return true; }
                a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('u')
            });
            let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            if unset && !positional.is_empty() {
                let _ = tx.send(CtrlReq::UnsetEnvironment(positional[0].to_string()));
            } else if positional.len() >= 2 {
                let _ = tx.send(CtrlReq::SetEnvironment(positional[0].to_string(), positional[1].to_string()));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "show-environment" | "showenv" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowEnvironment(rtx));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "set-hook" => {
            let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
            if positional.len() >= 2 {
                let name = positional[0].to_string();
                // Re-quote tokens that contain spaces to preserve paths like "Psmux Plugins"
                let command = positional[1..].iter().map(|s| {
                    if s.contains(' ') { format!("'{}'", s) } else { s.to_string() }
                }).collect::<Vec<_>>().join(" ");
                let has_append = args.iter().any(|a| {
                    if *a == "-a" { return true; }
                    a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains('a')
                });
                if has_append {
                    let _ = tx.send(CtrlReq::AppendHook(name, command));
                } else {
                    let _ = tx.send(CtrlReq::SetHook(name, command));
                }
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "show-hooks" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ShowHooks(rtx));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "server-info" | "info" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ServerInfo(rtx));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "list-commands" | "lscm" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::ListCommands(rtx));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "dump-state" | "dump" => {
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::DumpState(rtx, false, client_id));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(5));
            true
        }
        "zoom-pane" | "resizep -Z" => {
            let _ = tx.send(CtrlReq::ZoomPane);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "last-window" | "last" => {
            let _ = tx.send(CtrlReq::LastWindow);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "last-pane" | "lastp" => {
            let _ = tx.send(CtrlReq::LastPane);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "next-window" | "next" => {
            let _ = tx.send(CtrlReq::NextWindow);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "previous-window" | "prev" => {
            let _ = tx.send(CtrlReq::PrevWindow);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "rotate-window" | "rotatew" => {
            let upward = args.iter().any(|a| *a == "-U");
            let _ = tx.send(CtrlReq::RotateWindow(upward));
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "break-pane" | "breakp" => {
            let _ = tx.send(CtrlReq::BreakPane);
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "respawn-pane" | "respawnp" => {
            let workdir = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
            let empty = args.iter().any(|a| *a == "-E");
            let kill = args.iter().any(|a| *a == "-k") || empty;
            // Honor `-- <shell-command>` (issue #399): teammate launch delivery.
            let command = args.iter().position(|a| *a == "--")
                .map(|i| args[i + 1..].join(" "))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let _ = tx.send(CtrlReq::RespawnPane(workdir, kill, command, empty));
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "wait-for" | "wait" => {
            let op = if args.iter().any(|a| *a == "-L") { WaitForOp::Lock }
                     else if args.iter().any(|a| *a == "-U") { WaitForOp::Unlock }
                     else if args.iter().any(|a| *a == "-S") { WaitForOp::Signal }
                     else { WaitForOp::Wait };
            if let Some(channel) = args.iter().find(|a| !a.starts_with('-')) {
                let _ = tx.send(CtrlReq::WaitFor(channel.to_string(), op));
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "refresh-client" | "refresh" => {
            // Parse -B name:what:format (subscription management)
            let mut i = 0;
            while i < args.len() {
                if args[i] == "-B" {
                    if let Some(spec) = args.get(i + 1) {
                        // Format: "name:what:format" or "name:" (remove)
                        let spec = spec.trim_matches('"');
                        if let Some(colon1) = spec.find(':') {
                            let name = spec[..colon1].to_string();
                            let rest = &spec[colon1 + 1..];
                            if rest.is_empty() {
                                // Remove subscription: "name:"
                                let _ = tx.send(CtrlReq::ControlUnsubscribe {
                                    client_id,
                                    name,
                                });
                            } else if let Some(colon2) = rest.find(':') {
                                let target = rest[..colon2].to_string();
                                let format = rest[colon2 + 1..].to_string();
                                let _ = tx.send(CtrlReq::ControlSubscribe {
                                    client_id,
                                    name,
                                    target,
                                    format,
                                });
                            }
                        }
                    }
                    i += 2;
                    continue;
                }
                // Parse -f flags (e.g. pause-after=N)
                if args[i] == "-f" {
                    if let Some(flag_val) = args.get(i + 1) {
                        let flag_val = flag_val.trim_matches('"');
                        if let Some(stripped) = flag_val.strip_prefix("pause-after=") {
                            let secs = stripped.parse::<u64>().ok();
                            let _ = tx.send(CtrlReq::ControlSetPauseAfter {
                                client_id,
                                pause_after_secs: secs,
                            });
                        } else if flag_val == "no-pause" {
                            let _ = tx.send(CtrlReq::ControlSetPauseAfter {
                                client_id,
                                pause_after_secs: None,
                            });
                        }
                    }
                    i += 2;
                    continue;
                }
                // Parse -A '%N:continue' (resume paused pane)
                if args[i] == "-A" {
                    if let Some(spec) = args.get(i + 1) {
                        let spec = spec.trim_matches('"').trim_matches('\'');
                        // Format: %N:continue or %N:pause
                        if let Some(colon) = spec.find(':') {
                            let pane_spec = &spec[..colon];
                            let action = &spec[colon + 1..];
                            if action == "continue" {
                                if let Some(pid_str) = pane_spec.strip_prefix('%') {
                                    if let Ok(pid) = pid_str.parse::<usize>() {
                                        let _ = tx.send(CtrlReq::ControlContinuePane {
                                            client_id,
                                            pane_id: pid,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    i += 2;
                    continue;
                }
                // Parse control client viewport sizes. tmux accepts a default
                // `WxH`/`W,H`, a per-window `@id:WxH`, and `@id:` to clear it.
                if args[i] == "-C" {
                    match args.get(i + 1) {
                        Some(spec) => match crate::resize_window::parse_control_client_size(spec) {
                            Ok((window_id, size)) => {
                                let _ = tx.send(CtrlReq::ControlClientResize {
                                    client_id,
                                    window_id,
                                    size,
                                });
                            }
                            Err(error) => {
                                let _ = resp_tx.send(ControlCommandResponse::error(error));
                                return true;
                            }
                        },
                        None => {
                            let _ = resp_tx.send(ControlCommandResponse::error(
                                "missing size argument",
                            ));
                            return true;
                        }
                    }
                    i += 2;
                    continue;
                }
                i += 1;
            }
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "run-command" | "runcmd" => {
            let full_cmd = args.join(" ");
            let (rtx, rrx) = mpsc::channel::<String>();
            let _ = tx.send(CtrlReq::RunCommand(full_cmd, rtx));
            forward_control_response(cmd, rrx, &resp_tx, Duration::from_secs(15));
            true
        }
        // iTerm2 sends "phony-command" as a tmux ping/keepalive on entering
        // gateway mode (see iTerm2 TmuxController.m kickOffTmuxForRestoration).
        // Real tmux returns success with no output; we mimic that.
        "phony-command" => {
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        // Copy mode in tmux control sessions is a no-op for iTerm2 — iTerm
        // implements its own copy mode locally on captured pane content.
        // Returning success keeps iTerm's command pipeline alive.
        "copy-mode" => {
            let _ = resp_tx.send(ControlCommandResponse::empty());
            true
        }
        "resize-window" | "resizew" => {
            let response = match crate::resize_window::parse_resize_window(args, raw_target) {
                Ok(request) => {
                    let (resize_tx, resize_rx) = mpsc::channel();
                    if tx.send(CtrlReq::ResizeWindow(request, resize_tx)).is_err() {
                        Err("server unavailable".to_string())
                    } else {
                        resize_rx
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap_or_else(|_| Err("resize-window timed out".to_string()))
                    }
                }
                Err(error) => Err(error),
            };
            match response {
                Ok(()) => {
                    let _ = resp_tx.send(ControlCommandResponse::empty());
                }
                Err(error) => {
                    let _ = resp_tx.send(ControlCommandResponse::error(error));
                }
            }
            true
        }
        _ => {
            let _ = resp_tx.send(ControlCommandResponse::error(format!(
                "unknown command: {}",
                cmd
            )));
            true
        }
    }
}

#[cfg(test)]
#[path = "../../tests-rs/test_issue476_bindkey_quoting.rs"]
mod tests_issue476_bindkey_quoting;

#[cfg(test)]
#[path = "../../tests-rs/test_send_keys_literal_byte.rs"]
mod tests_send_keys_literal_byte;

#[cfg(test)]
#[path = "../../tests-rs/test_control_mode_liveness.rs"]
mod tests_control_mode_liveness;
