// Regression coverage for tmux-compatible window sizing.
//
// tmex sends `resize-window -t @id -x cols -y rows` to its Windows psmux
// backend. psmux used to accept that command while leaving the window and
// ConPTY unchanged. These tests exercise the production parser and state
// transitions behind both normal and control-mode command dispatch without
// starting a psmux server (see AGENTS.md).

use super::*;
use crate::types::{ControlClient, LayoutKind, Node, Window};

fn empty_window(id: usize, name: &str, width: u16, height: u16) -> Window {
    Window {
        root: Node::Split {
            kind: LayoutKind::Horizontal,
            sizes: Vec::new(),
            children: Vec::new(),
        },
        active_path: Vec::new(),
        name: name.to_string(),
        id,
        area: Rect::new(0, 0, width, height),
        window_size: None,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: std::time::Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: Vec::new(),
        zoom_saved: None,
        linked_from: None,
        floating: Vec::new(),
        floating_focus: None,
    }
}

fn app_with_two_windows() -> AppState {
    let mut app = AppState::new("test".to_string());
    app.windows.push(empty_window(3, "first", 80, 24));
    app.windows.push(empty_window(7, "second", 80, 24));
    app.window_indices = vec![0, 1];
    app.client_area = Rect::new(0, 0, 80, 24);
    app.last_window_area = app.client_area;
    app
}

fn control_client(
    client_id: u64,
    size: Option<(u16, u16)>,
    window_sizes: &[(usize, (u16, u16))],
) -> ControlClient {
    let (output_tx, _output_rx) = std::sync::mpsc::sync_channel(1);
    ControlClient {
        client_id,
        cmd_counter: 0,
        echo_enabled: false,
        output_tx,
        paused_panes: std::collections::HashSet::new(),
        subscriptions: std::collections::HashMap::new(),
        subscription_values: std::collections::HashMap::new(),
        subscription_last_check: std::collections::HashMap::new(),
        pause_after_secs: None,
        output_paused_panes: std::collections::HashSet::new(),
        pane_last_output: std::collections::HashMap::new(),
        size,
        window_sizes: window_sizes.iter().copied().collect(),
    }
}

#[test]
fn parses_combined_absolute_dimensions_and_target() {
    let request = parse_resize_window(&["-x120", "-y", "40"], Some("@7")).unwrap();
    assert_eq!(request.target, WindowTarget::Id(7));
    assert_eq!(request.width, Some(120));
    assert_eq!(request.height, Some(40));

    let attached_target = parse_resize_window(&["-t@7", "-x=130"], None).unwrap();
    assert_eq!(attached_target.target, WindowTarget::Id(7));
    assert_eq!(attached_target.width, Some(130));
}

#[test]
fn parses_session_qualified_window_id() {
    let request = parse_resize_window(&["-t", "work:@7", "-x", "120", "-y", "40"], None).unwrap();

    assert_eq!(request.target, WindowTarget::Id(7));
    assert_eq!(request.width, Some(120));
    assert_eq!(request.height, Some(40));
}

#[test]
fn parses_tmux_adjustment_flags_with_priority() {
    let request = parse_resize_window(&["-DR", "5"], None).unwrap();
    assert_eq!(request.direction, Some(ResizeDirection::Right));
    assert_eq!(request.adjustment, 5);
}

#[test]
fn parses_largest_before_smallest_like_tmux() {
    let request = parse_resize_window(&["-aA"], None).unwrap();
    assert_eq!(request.client_size, Some(ClientSizeChoice::Largest));
}

#[test]
fn rejects_dimensions_unsafe_for_conpty() {
    assert_eq!(
        parse_resize_window(&["-x", "1"], None),
        Err("width too small".to_string())
    );
    assert_eq!(
        parse_resize_window(&["-y10001"], None),
        Err("height too large".to_string())
    );
    assert_eq!(
        parse_resize_window(&["-R", "2147483648"], None),
        Err("adjustment too large".to_string())
    );
}

#[test]
fn parses_control_default_and_per_window_sizes() {
    assert_eq!(
        parse_control_client_size("80,24").unwrap(),
        (None, Some((80, 24)))
    );
    assert_eq!(
        parse_control_client_size("@3:120x40").unwrap(),
        (Some(3), Some((120, 40)))
    );
    assert_eq!(parse_control_client_size("@3:").unwrap(), (Some(3), None));
    assert!(parse_control_client_size("@3").is_err());
    assert!(parse_control_client_size("80").is_err());
    assert!(parse_control_client_size("@3:1x24").is_err());
}

#[test]
fn tmex_resize_command_updates_only_the_target_window() {
    let mut app = app_with_two_windows();
    let request = parse_resize_window(&["-t", "@7", "-x", "120", "-y", "40"], None).unwrap();
    let result = apply_resize_window(&mut app, &request).unwrap();

    assert_eq!(result.window_index, 1);
    assert_eq!(app.windows[1].area, Rect::new(0, 0, 120, 40));
    assert_eq!(app.windows[1].window_size.as_deref(), Some("manual"));
    assert_eq!(app.windows[0].area, Rect::new(0, 0, 80, 24));
    assert_eq!(app.client_area, Rect::new(0, 0, 80, 24));
    assert_eq!(app.last_window_area, Rect::new(0, 0, 80, 24));
}

#[test]
fn relative_resize_uses_tmux_direction_priority() {
    let mut app = app_with_two_windows();
    let request = parse_resize_window(&["-DR", "5"], Some("@3")).unwrap();

    apply_resize_window(&mut app, &request).unwrap();

    assert_eq!(app.windows[0].area, Rect::new(0, 0, 85, 24));
    assert_eq!(app.manual_window_sizes.get(&3), Some(&(85, 24)));
}

#[test]
fn client_resize_preserves_manual_window() {
    let mut app = app_with_two_windows();
    let request = parse_resize_window(&["-x", "120", "-y", "40"], Some("@7")).unwrap();
    apply_resize_window(&mut app, &request).unwrap();

    app.client_sizes.insert(11, (100, 32));
    app.latest_client_id = Some(11);
    refresh_dynamic_window_sizes(&mut app);

    assert_eq!(app.windows[0].area, Rect::new(0, 0, 100, 32));
    assert_eq!(app.windows[1].area, Rect::new(0, 0, 120, 40));
    assert_eq!(app.last_window_area, Rect::new(0, 0, 100, 32));
}

#[test]
fn largest_and_smallest_use_all_client_dimensions() {
    let mut app = app_with_two_windows();
    app.client_sizes.insert(1, (90, 50));
    app.client_sizes.insert(2, (120, 30));

    let largest = parse_resize_window(&["-A"], Some("@3")).unwrap();
    apply_resize_window(&mut app, &largest).unwrap();
    assert_eq!(app.windows[0].area, Rect::new(0, 0, 120, 50));

    let smallest = parse_resize_window(&["-a"], Some("@7")).unwrap();
    apply_resize_window(&mut app, &smallest).unwrap();
    assert_eq!(app.windows[1].area, Rect::new(0, 0, 90, 30));
}

#[test]
fn largest_without_clients_uses_the_default_geometry() {
    let mut app = app_with_two_windows();
    let largest = parse_resize_window(&["-A"], Some("@3")).unwrap();

    apply_resize_window(&mut app, &largest).unwrap();

    assert_eq!(app.windows[0].area, Rect::new(0, 0, 80, 24));
    assert_eq!(app.windows[0].window_size.as_deref(), Some("manual"));
}

#[test]
fn per_window_client_size_clamps_manual_geometry_until_cleared() {
    let mut app = app_with_two_windows();
    app.control_clients
        .insert(11, control_client(11, Some((200, 100)), &[(3, (80, 24))]));
    let request = parse_resize_window(&["-x", "120", "-y", "40"], Some("@3")).unwrap();

    apply_resize_window(&mut app, &request).unwrap();
    assert_eq!(app.windows[0].area, Rect::new(0, 0, 80, 24));
    assert_eq!(app.manual_window_sizes.get(&3), Some(&(120, 40)));

    app.control_clients
        .get_mut(&11)
        .unwrap()
        .window_sizes
        .remove(&3);
    refresh_dynamic_window_sizes(&mut app);
    assert_eq!(app.windows[0].area, Rect::new(0, 0, 120, 40));
}

#[test]
fn explicit_window_size_caps_largest_across_other_clients() {
    let mut app = app_with_two_windows();
    app.control_clients
        .insert(11, control_client(11, Some((200, 100)), &[(3, (80, 24))]));
    app.control_clients
        .insert(12, control_client(12, Some((160, 80)), &[]));
    let largest = parse_resize_window(&["-A"], Some("@3")).unwrap();

    apply_resize_window(&mut app, &largest).unwrap();

    assert_eq!(app.windows[0].area, Rect::new(0, 0, 80, 24));
    assert_eq!(app.manual_window_sizes.get(&3), Some(&(160, 80)));
}

#[test]
fn window_size_option_can_leave_manual_mode() {
    let mut app = app_with_two_windows();
    app.client_sizes.insert(1, (100, 32));
    app.latest_client_id = Some(1);
    let request = parse_resize_window(&["-x", "120", "-y", "40"], Some("@3")).unwrap();
    apply_resize_window(&mut app, &request).unwrap();

    set_active_window_size_mode(&mut app, Some("latest".to_string())).unwrap();

    assert_eq!(app.windows[0].window_size.as_deref(), Some("latest"));
    assert_eq!(app.windows[0].area, Rect::new(0, 0, 100, 32));
}

#[test]
fn leaving_manual_mode_without_clients_preserves_geometry() {
    let mut app = app_with_two_windows();
    let request = parse_resize_window(&["-x", "112", "-y", "36"], Some("@3")).unwrap();
    apply_resize_window(&mut app, &request).unwrap();

    set_active_window_size_mode(&mut app, Some("latest".to_string())).unwrap();

    assert_eq!(app.windows[0].window_size.as_deref(), Some("latest"));
    assert_eq!(app.windows[0].area, Rect::new(0, 0, 112, 36));
}
