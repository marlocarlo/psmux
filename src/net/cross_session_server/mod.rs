//! Server-side handlers for cross-session pane forwarding.
//!
//! Extracted into its own module to keep server/mod.rs small.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc};
use std::sync::atomic::AtomicBool;

use crate::types::{AppState, ForwardedPane, Node, LayoutKind};
use crate::tree;

/// Handle `PaneForwardExtract`: extract a pane from the window tree, keep
/// its real ConPTY alive, start a TCP forwarding listener, and reply with
/// connection info so the target session can connect.
pub fn handle_pane_forward_extract(
    app: &mut AppState,
    win_idx: usize,
    pane_idx: usize,
    resp: mpsc::Sender<String>,
) {
    if win_idx >= app.windows.len() {
        let _ = resp.send("ERR window out of range".to_string());
        return;
    }
    // Resolve pane path by DFS index
    let src_path = {
        let mut leaves = Vec::new();
        tree::collect_leaf_paths_pub(&app.windows[win_idx].root, &mut Vec::new(), &mut leaves);
        if let Some((_, p)) = leaves.get(pane_idx) {
            p.clone()
        } else {
            app.windows[win_idx].active_path.clone()
        }
    };
    // Unzoom if needed
    if let Some(saved) = app.windows[win_idx].zoom_saved.take() {
        let win = &mut app.windows[win_idx];
        for (p, sz) in saved.into_iter() {
            if let Some(Node::Split { sizes, .. }) = tree::get_split_mut(&mut win.root, &p) {
                *sizes = sz;
            }
        }
    }
    // Extract the pane node from the tree
    let src_root = std::mem::replace(
        &mut app.windows[win_idx].root,
        Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
    );
    let (remaining, extracted) = tree::extract_node(src_root, &src_path);
    let pane_node = match extracted {
        Some(n) => n,
        None => {
            if let Some(rem) = remaining {
                app.windows[win_idx].root = rem;
            }
            let _ = resp.send("ERR pane not found".to_string());
            return;
        }
    };
    // Restore remaining tree (or remove empty window)
    let src_empty = remaining.is_none();
    if let Some(rem) = remaining {
        app.windows[win_idx].root = rem;
        app.windows[win_idx].active_path = tree::first_leaf_path(&app.windows[win_idx].root);
    }
    if src_empty {
        app.windows.remove(win_idx);
        if app.active_idx >= app.windows.len() {
            app.active_idx = app.windows.len().saturating_sub(1);
        }
    }
    // Unwrap the leaf into its Pane
    let pane = match pane_node {
        Node::Leaf(p) => p,
        _ => {
            let _ = resp.send("ERR extracted non-leaf".to_string());
            return;
        }
    };
    // Capture metadata before consuming the pane
    let pid = pane.child_pid;
    let title = pane.title.clone();
    let rows = pane.last_rows;
    let cols = pane.last_cols;
    // Capture full screen state (with colors, attributes, cursor) as VT escape codes
    let screen_b64 = {
        if let Ok(parser) = pane.term.lock() {
            let buf = parser.screen().state_formatted();
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&buf)
        } else {
            String::new()
        }
    };
    // Start TCP forwarding listener
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            let _ = resp.send(format!("ERR bind: {}", e));
            return;
        }
    };
    let listen_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let shutdown = Arc::new(AtomicBool::new(false));
    let fwd_id = app.next_forward_id;
    app.next_forward_id += 1;
    // Get reader from MasterPty and use the already-taken writer from the Pane
    let pty_reader = match pane.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = resp.send(format!("ERR reader: {}", e));
            return;
        }
    };
    // The writer was already taken during pane creation and stored in pane.writer
    let pty_writer = pane.writer;
    // Start forwarding threads
    let sd_clone = shutdown.clone();
    std::thread::spawn(move || {
        // Accept one connection for I/O forwarding
        if let Ok((stream, _)) = listener.accept() {
            let _ = stream.set_nodelay(true);
            let sd = sd_clone;
            // Spawn reader: PTY output -> TCP
            let mut tcp_writer = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => return,
            };
            let mut pty_reader = pty_reader;
            let sd2 = sd.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    if sd2.load(std::sync::atomic::Ordering::Relaxed) { break; }
                    match pty_reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tcp_writer.write_all(&buf[..n]).is_err() { break; }
                            let _ = tcp_writer.flush();
                        }
                        Err(_) => break,
                    }
                }
            });
            // Writer: TCP -> PTY input (same 64K buffer as reader for symmetric throughput)
            let mut tcp_reader = stream;
            let mut pty_writer = pty_writer;
            let mut buf = [0u8; 65536];
            loop {
                if sd.load(std::sync::atomic::Ordering::Relaxed) { break; }
                match tcp_reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if pty_writer.write_all(&buf[..n]).is_err() { break; }
                        let _ = pty_writer.flush();
                    }
                    Err(_) => break,
                }
            }
        }
    });
    // Store forwarded pane state
    app.forwarded_panes.insert(fwd_id, ForwardedPane {
        master: pane.master,
        child: pane.child,
        listener_port: listen_port,
        pid,
        title: title.clone(),
        rows,
        cols,
        shutdown,
    });
    // Send response
    let title_wire = title.replace(' ', "\x01");
    let response = format!(
        "FORWARD {} {} {} {} {} {} {}",
        fwd_id, listen_port, pid.unwrap_or(0), title_wire, rows, cols, screen_b64.len(),
    );
    if screen_b64.is_empty() {
        let _ = resp.send(response);
    } else {
        let _ = resp.send(format!("{}\n{}", response, screen_b64));
    }
}

/// Handle `PaneForwardInject`: create a proxy pane that tunnels I/O to a
/// source session's forwarded pane, then graft it into the window tree.
pub fn handle_pane_forward_inject(
    app: &mut AppState,
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
) {
    // Connect to the forwarding listener on the source session
    let fwd_addr = format!("127.0.0.1:{}", fwd_port);
    let stream = match TcpStream::connect(&fwd_addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("psmux: cross-session inject connect failed: {}", e);
            return;
        }
    };
    let _ = stream.set_nodelay(true);
    let reader_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("psmux: cross-session inject clone failed: {}", e);
            return;
        }
    };
    let writer_stream = stream;
    // Decode screen snapshot
    let screen_snapshot = if !screen_b64.is_empty() {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&screen_b64).ok()
    } else {
        None
    };
    // Generate a unique pane ID
    let pane_id = {
        let mut max_id = 0usize;
        for win in &app.windows {
            let mut leaves = Vec::new();
            tree::collect_leaf_paths_pub(&win.root, &mut Vec::new(), &mut leaves);
            for (id, _) in &leaves {
                if *id > max_id { max_id = *id; }
            }
        }
        max_id + 1
    };
    // Create the proxy pane
    let proxy_pane = match crate::proxy_pane::create_proxy_pane(
        reader_stream,
        writer_stream,
        source_addr.clone(),
        source_key,
        source_session,
        forward_id,
        if pid > 0 { Some(pid) } else { None },
        title,
        rows,
        cols,
        pane_id,
        screen_snapshot,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("psmux: create_proxy_pane failed: {}", e);
            return;
        }
    };
    // Start the reader thread (same as normal panes)
    let reader = match proxy_pane.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("psmux: proxy reader clone failed: {}", e);
            return;
        }
    };
    crate::pane::spawn_reader_thread(
        reader,
        proxy_pane.term.clone(),
        proxy_pane.data_version.clone(),
        proxy_pane.cursor_shape.clone(),
        proxy_pane.bell_pending.clone(),
        proxy_pane.output_ring.clone(),
    );
    // Graft into the target window tree
    let tgt_idx = target_win.unwrap_or(app.active_idx);
    if tgt_idx < app.windows.len() {
        let tgt_path = if let Some(tp) = target_pane {
            let mut leaves = Vec::new();
            tree::collect_leaf_paths_pub(&app.windows[tgt_idx].root, &mut Vec::new(), &mut leaves);
            if let Some((_, p)) = leaves.get(tp) {
                p.clone()
            } else {
                app.windows[tgt_idx].active_path.clone()
            }
        } else {
            app.windows[tgt_idx].active_path.clone()
        };
        let split_kind = if horizontal { LayoutKind::Horizontal } else { LayoutKind::Vertical };
        tree::replace_leaf_with_split(
            &mut app.windows[tgt_idx].root,
            &tgt_path,
            split_kind,
            Node::Leaf(proxy_pane),
        );
        app.active_idx = tgt_idx;
    }
}
