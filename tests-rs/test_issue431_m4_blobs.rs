// Issue #431 - Sixel M4: server blob shipping (image_blobs) + resync.
//
// The full end-to-end path (real sixel bytes through a pane) cannot be
// exercised on this machine: the pane runs under inbox conhost, which STRIPS
// the sixel DCS before psmux's vt100 parser ever sees it (design section 7 -
// "conhost strips sixel even with PASSTHROUGH_MODE").  Rasterised pixels and a
// live-pane sixel therefore need Windows Terminal (deferred to M8).
//
// This test proves the M4 TRANSPORT half headlessly by injecting the sixel
// bytes straight into a pane's vt100 parser via `create_proxy_pane`'s
// screen_snapshot (the SAME parser a live pane would use, minus conhost), then
// driving the REAL server code:
//   * helpers::visible_image_blobs gathers the visible (id, raw) pairs.
//   * build_image_blobs_json base64-ships each unshipped visible blob ONCE,
//     records the id in AppState.shipped_image_ids, and emits "{}" thereafter.
//   * clearing shipped_image_ids (what the ClientAttach / RefreshClient resync
//     hooks do) makes the blob re-ship.

use super::build_image_blobs_json;
use crate::server::helpers::visible_image_blobs;
use crate::types::{AppState, Node, Window};
use ratatui::layout::Rect;

use std::net::{TcpListener, TcpStream};

fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let accept = std::thread::spawn(move || listener.accept().expect("accept").0);
    let client = TcpStream::connect(addr).expect("connect");
    let server = accept.join().expect("join accept");
    (client, server)
}

/// Exact re-emit bytes of a tiny sixel with the distinctive marker "13;57;91":
///   ESC P q "1;1;24;12 #7;2;13;57;91 #7 !24~ $ - !24~ $ - ESC \
fn sixel_bytes() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1bP");
    v.extend_from_slice(b"q\"1;1;24;12#7;2;13;57;91#7!24~$-!24~$-");
    v.extend_from_slice(b"\x1b\\");
    v
}

/// Build a one-window, one-pane AppState with the sixel pre-loaded into the
/// pane's screen (fed through the real vt100 parser, bypassing conhost).
fn app_with_sixel_pane() -> AppState {
    let (reader, _peer_r) = tcp_pair();
    let (writer, _peer_w) = tcp_pair();
    let pane = crate::proxy_pane::create_proxy_pane(
        reader,
        writer,
        "127.0.0.1:1".to_string(),
        "test-key".to_string(),
        "test-session".to_string(),
        1,
        None,
        "pane-1".to_string(),
        24,
        80,
        1,
        Some(sixel_bytes()), // screen_snapshot -> processed by vt100::Parser
    )
    .expect("create proxy pane");

    let win = Window {
        root: Node::Leaf(pane),
        active_path: vec![], // single leaf root -> empty path is the active pane
        name: "w0".to_string(),
        id: 0,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: std::time::Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: vec![1],
        zoom_saved: None,
        linked_from: None,
    };

    let mut app = AppState::new("m4-blobs".to_string());
    app.last_window_area = Rect { x: 0, y: 0, width: 80, height: 24 };
    app.windows.push(win);
    app.active_idx = 0;
    app
}

/// Pull the single `"<id>":"<base64>"` pair out of an image_blobs object.
fn first_pair(json: &str) -> Option<(String, String)> {
    let inner = json.trim();
    let inner = inner.strip_prefix('{')?.strip_suffix('}')?;
    if inner.is_empty() {
        return None;
    }
    let (k, v) = inner.split_once(':')?;
    let k = k.trim().trim_matches('"').to_string();
    let v = v.trim().trim_matches('"').to_string();
    Some((k, v))
}

#[test]
fn sixel_snapshot_stores_exactly_one_image() {
    let app = app_with_sixel_pane();
    let win = &app.windows[0];
    if let Node::Leaf(p) = &win.root {
        let parser = p.term.lock().unwrap();
        let imgs = parser.screen().images();
        assert_eq!(imgs.len(), 1, "expected exactly one stored sixel image");
        let raw = &imgs[0].raw;
        assert!(raw.starts_with(b"\x1bP"), "raw must start with ESC P");
        assert!(raw.ends_with(b"\x1b\\"), "raw must end with ST (ESC \\)");
        assert!(
            String::from_utf8_lossy(raw).contains("13;57;91"),
            "raw must contain the sixel marker"
        );
    } else {
        panic!("root is not a leaf");
    }
}

#[test]
fn visible_image_blobs_returns_the_pane_image() {
    let app = app_with_sixel_pane();
    let blobs = visible_image_blobs(&app);
    assert_eq!(blobs.len(), 1, "one visible image expected");
    let (id, raw) = &blobs[0];
    assert!(*id >= 1, "id assigned by add_image");
    assert!(raw.starts_with(b"\x1bP") && raw.ends_with(b"\x1b\\"));
}

#[test]
fn blob_shipped_once_then_empty_then_reships_on_resync() {
    use base64::Engine as _;
    let mut app = app_with_sixel_pane();

    // FRAME 1: blob ships, keyed by the image id, base64 of the raw bytes.
    let f1 = build_image_blobs_json(&mut app);
    let (id, b64) = first_pair(&f1).expect("frame 1 must ship a blob");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .expect("value must be valid base64");
    assert!(
        decoded.starts_with(b"\x1bP") && decoded.ends_with(b"\x1b\\"),
        "decoded blob must be the raw sixel (ESC P ... ST)"
    );
    assert!(
        String::from_utf8_lossy(&decoded).contains("13;57;91"),
        "decoded blob must carry the marker"
    );
    let id_num: u64 = id.parse().expect("blob key is a numeric id");
    assert!(
        app.shipped_image_ids.contains(&id_num),
        "id must be recorded as shipped"
    );

    // The leaf descriptor id (M3) must be the SAME id as the blob key so the
    // client can pair them.
    let descs = {
        let win = &app.windows[0];
        if let Node::Leaf(p) = &win.root {
            let parser = p.term.lock().unwrap();
            crate::server::helpers::visible_pane_images(parser.screen(), p.last_rows, 0)
        } else {
            panic!("not a leaf");
        }
    };
    assert_eq!(descs.len(), 1);
    assert_eq!(descs[0].id, id_num, "descriptor id must match blob key");

    // FRAME 2: same image still visible but already shipped -> empty object.
    let f2 = build_image_blobs_json(&mut app);
    assert_eq!(f2, "{}", "steady-state frame must emit empty image_blobs");

    // RESYNC: ClientAttach / RefreshClient clear shipped_image_ids; the blob
    // must re-ship on the next frame.
    app.shipped_image_ids.clear();
    let f3 = build_image_blobs_json(&mut app);
    let (id3, _b64_3) = first_pair(&f3).expect("post-resync frame must re-ship the blob");
    assert_eq!(id3, id, "same id re-ships after resync");
}
