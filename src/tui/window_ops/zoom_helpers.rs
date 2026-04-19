use super::*;

/// Temporarily unzoom for an operation, saving the zoom state so it can be
/// restored via `pop_zoom()` afterwards (tmux push/pop semantics).
/// Returns true if zoom was active and was suspended.
pub fn push_zoom(app: &mut AppState) -> bool {
    if app.windows[app.active_idx].zoom_saved.is_some() {
        // Mark that we had zoom active, unzoom, but DON'T clear zoom_saved
        // — we move it to a temp slot so pop_zoom can re-apply it.
        unzoom_if_zoomed(app);
        true
    } else {
        false
    }
}

/// Re-apply zoom after a push_zoom operation (tmux push/pop semantics).
/// Only re-zooms if `was_zoomed` is true.
pub fn pop_zoom(app: &mut AppState, was_zoomed: bool) {
    if was_zoomed && app.windows[app.active_idx].zoom_saved.is_none() {
        toggle_zoom(app);
    }
}

/// If zoom is currently active, unzoom (restore saved sizes) and resize panes.
/// Returns true if zoom was active and was cancelled.
pub fn unzoom_if_zoomed(app: &mut AppState) -> bool {
    if let Some(saved) = app.windows[app.active_idx].zoom_saved.take() {
        let win = &mut app.windows[app.active_idx];
        for (p, sz) in saved.into_iter() {
            if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) { *sizes = sz; }
        }
        resize_all_panes(app);
        true
    } else {
        false
    }
}
