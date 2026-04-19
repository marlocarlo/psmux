use super::*;

/// Forward a non-left mouse button press/release to the child.
pub fn remote_mouse_button(app: &mut AppState, x: u16, y: u16, button: u8, press: bool) {
    let (x, y) = map_client_coords(app, x, y);
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
        let (col, row) = pane_inner_cell_0based(area, x, y);
        let win_name = win.name.clone();
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if pane_wants_mouse(active) {
                let sgr_btn = match button {
                    1 => 1u8, // middle
                    2 => 2u8, // right
                    _ => 0u8,
                };
                let button_state = if press {
                    match button {
                        1 => mouse_inject::FROM_LEFT_2ND_BUTTON_PRESSED,
                        2 => mouse_inject::RIGHTMOST_BUTTON_PRESSED,
                        _ => 0,
                    }
                } else {
                    0
                };
                inject_mouse_combined(active, col, row, sgr_btn, press,
                    button_state, 0, &win_name);
            }
        }
    }
}

/// Forward bare mouse motion (hover) to the child PTY.
///
/// Only forwarded when the active pane explicitly wants mouse input
/// (`pane_wants_mouse`).  Shell prompts and ClaudeCode-style inputs are
/// excluded because they do not enable mouse tracking, and sending raw SGR
/// motion bytes (ESC[<35;...) would appear as visible garbage.
///
/// SGR button 35 = bare motion with no button held (WT parity).
/// Windows Terminal encodes hover as WM_MOUSEMOVE -> button 3 + 0x20 = 35.
///
/// Same-coordinate events are suppressed (Windows Terminal parity: the
/// terminal only sends motion when coordinates actually change).
pub fn remote_mouse_motion(app: &mut AppState, x: u16, y: u16) {
    let (x, y) = map_client_coords(app, x, y);
    // WT parity: suppress same-coordinate duplicates
    if app.last_hover_pos == Some((x, y)) {
        return;
    }
    app.last_hover_pos = Some((x, y));

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);

    // Forward hover only when the active pane explicitly wants mouse input.
    // This avoids leaking raw SGR motion bytes (ESC[<35;...) into shell-style
    // prompts such as claudecode input boxes.
    mouse_log(&format!("remote_mouse_motion: x={} y={}", x, y));

    if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
        let (col, row) = pane_inner_cell_0based(area, x, y);
        let win_name = win.name.clone();
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if pane_wants_mouse(active) {
                inject_mouse_combined(active, col, row, 35, true,
                    0, mouse_inject::MOUSE_MOVED, &win_name);
            }
        }
    }
}
