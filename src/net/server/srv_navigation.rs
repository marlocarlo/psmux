use super::*;
use super::srv_loop_ctx::LoopCtx;

pub(crate) fn handle_focus_window(app: &mut AppState, wid: usize) {
    if wid >= app.window_base_index {
        let internal_idx = wid - app.window_base_index;
        if internal_idx < app.windows.len() && internal_idx != app.active_idx {
            switch_with_copy_save(app, |app| {
                app.last_window_idx = app.active_idx;
                app.active_idx = internal_idx;
            });
            if let Some(win) = app.windows.get_mut(internal_idx) {
                win.activity_flag = false; win.bell_flag = false; win.silence_flag = false;
            }
            resize_all_panes(app);
        }
    }
}

pub(crate) fn handle_focus_window_by_name(app: &mut AppState, name: &str) {
    if let Some(internal_idx) = app.windows.iter().position(|w| w.name == *name) {
        if internal_idx != app.active_idx {
            switch_with_copy_save(app, |app| {
                app.last_window_idx = app.active_idx;
                app.active_idx = internal_idx;
            });
            if let Some(win) = app.windows.get_mut(internal_idx) {
                win.activity_flag = false; win.bell_flag = false; win.silence_flag = false;
            }
            resize_all_panes(app);
        }
    }
}

pub(crate) fn handle_focus_pane(app: &mut AppState, pid: usize) {
    let old_path = app.windows[app.active_idx].active_path.clone();
    switch_with_copy_save(app, |app| { focus_pane_by_id(app, pid); });
    if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(app); }
}

pub(crate) fn handle_focus_pane_by_index(app: &mut AppState, idx: usize) {
    let old_path = app.windows[app.active_idx].active_path.clone();
    switch_with_copy_save(app, |app| { focus_pane_by_index(app, idx); });
    if app.windows[app.active_idx].active_path != old_path { unzoom_if_zoomed(app); }
    let win = &mut app.windows[app.active_idx];
    if let Some(pid) = crate::tree::get_active_pane_id(&win.root, &win.active_path) {
        crate::tree::touch_mru(&mut win.pane_mru, pid);
    }
}

pub(crate) fn handle_focus_window_temp(app: &mut AppState, ctx: &mut LoopCtx, wid: usize) {
    if ctx.temp_focus_restore.is_none() {
        let pane_id = crate::tree::get_active_pane_id(&app.windows[app.active_idx].root, &app.windows[app.active_idx].active_path).unwrap_or(usize::MAX);
        ctx.temp_focus_restore = Some((app.active_idx, pane_id));
    }
    if wid >= app.window_base_index {
        let internal_idx = wid - app.window_base_index;
        if internal_idx < app.windows.len() { app.active_idx = internal_idx; }
    }
}

pub(crate) fn handle_focus_window_by_name_temp(app: &mut AppState, ctx: &mut LoopCtx, name: &str) {
    if ctx.temp_focus_restore.is_none() {
        let pane_id = crate::tree::get_active_pane_id(&app.windows[app.active_idx].root, &app.windows[app.active_idx].active_path).unwrap_or(usize::MAX);
        ctx.temp_focus_restore = Some((app.active_idx, pane_id));
    }
    if let Some(internal_idx) = app.windows.iter().position(|w| w.name == *name) {
        app.active_idx = internal_idx;
    }
}

pub(crate) fn handle_focus_pane_temp(app: &mut AppState, ctx: &mut LoopCtx, pid: usize) {
    if ctx.temp_focus_restore.is_none() {
        let pane_id = crate::tree::get_active_pane_id(&app.windows[app.active_idx].root, &app.windows[app.active_idx].active_path).unwrap_or(usize::MAX);
        ctx.temp_focus_restore = Some((app.active_idx, pane_id));
    }
    focus_pane_by_id_no_mru(app, pid);
}

pub(crate) fn handle_focus_pane_by_index_temp(app: &mut AppState, ctx: &mut LoopCtx, idx: usize) {
    if ctx.temp_focus_restore.is_none() {
        let pane_id = crate::tree::get_active_pane_id(&app.windows[app.active_idx].root, &app.windows[app.active_idx].active_path).unwrap_or(usize::MAX);
        ctx.temp_focus_restore = Some((app.active_idx, pane_id));
    }
    focus_pane_by_index(app, idx);
}

pub(crate) fn handle_select_pane(app: &mut AppState, dir: String) {
    if let Some(cmds) = app.hooks.get("before-select-pane") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    match dir.as_str() {
        "U" | "D" | "L" | "R" => {
            let focus_dir = match dir.as_str() {
                "U" => FocusDir::Up, "D" => FocusDir::Down,
                "L" => FocusDir::Left, _ => FocusDir::Right,
            };
            let was_zoomed = unzoom_if_zoomed(app);
            if was_zoomed {
                let win = &app.windows[app.active_idx];
                let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                let active_idx = rects.iter().position(|(path, _)| *path == win.active_path);
                let has_target = if let Some(ai) = active_idx {
                    let (_, arect) = &rects[ai];
                    find_best_pane_in_direction(&rects, ai, arect, focus_dir, &[], &[])
                        .or_else(|| find_wrap_target(&rects, ai, arect, focus_dir, &[], &[]))
                        .is_some()
                } else { false };
                if has_target {
                    let old_path = app.windows[app.active_idx].active_path.clone();
                    switch_with_copy_save(app, |app| { move_focus(app, focus_dir); });
                    app.last_pane_path = old_path;
                } else {
                    toggle_zoom(app);
                }
            } else {
                let old_path = app.windows[app.active_idx].active_path.clone();
                switch_with_copy_save(app, |app| { move_focus(app, focus_dir); });
                if app.windows[app.active_idx].active_path != old_path {
                    app.last_pane_path = old_path;
                }
            }
        }
        "last" => {
            let old_path = app.windows[app.active_idx].active_path.clone();
            switch_with_copy_save(app, |app| {
                let win = &mut app.windows[app.active_idx];
                if !app.last_pane_path.is_empty() {
                    let tmp = win.active_path.clone();
                    win.active_path = app.last_pane_path.clone();
                    app.last_pane_path = tmp;
                }
            });
            if app.windows[app.active_idx].active_path != old_path {
                let win = &mut app.windows[app.active_idx];
                if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                    crate::tree::touch_mru(&mut win.pane_mru, pid);
                }
                unzoom_if_zoomed(app);
            }
        }
        "mark" => {
            let win = &app.windows[app.active_idx];
            if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                app.marked_pane = Some((app.active_idx, pid));
            }
        }
        "next" => {
            let old_path = app.windows[app.active_idx].active_path.clone();
            switch_with_copy_save(app, |app| {
                let win = &app.windows[app.active_idx];
                let mut pane_paths = Vec::new();
                let mut path = Vec::new();
                collect_pane_paths_server(&win.root, &mut path, &mut pane_paths);
                if let Some(cur) = pane_paths.iter().position(|p| *p == win.active_path) {
                    let next = (cur + 1) % pane_paths.len();
                    let new_path = pane_paths[next].clone();
                    let win = &mut app.windows[app.active_idx];
                    app.last_pane_path = win.active_path.clone();
                    win.active_path = new_path;
                }
            });
            if app.windows[app.active_idx].active_path != old_path {
                let win = &mut app.windows[app.active_idx];
                if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                    crate::tree::touch_mru(&mut win.pane_mru, pid);
                }
                unzoom_if_zoomed(app);
            }
        }
        "prev" => {
            let old_path = app.windows[app.active_idx].active_path.clone();
            switch_with_copy_save(app, |app| {
                let win = &app.windows[app.active_idx];
                let mut pane_paths = Vec::new();
                let mut path = Vec::new();
                collect_pane_paths_server(&win.root, &mut path, &mut pane_paths);
                if let Some(cur) = pane_paths.iter().position(|p| *p == win.active_path) {
                    let prev = (cur + pane_paths.len() - 1) % pane_paths.len();
                    let new_path = pane_paths[prev].clone();
                    let win = &mut app.windows[app.active_idx];
                    app.last_pane_path = win.active_path.clone();
                    win.active_path = new_path;
                }
            });
            if app.windows[app.active_idx].active_path != old_path {
                let win = &mut app.windows[app.active_idx];
                if let Some(pid) = get_active_pane_id(&win.root, &win.active_path) {
                    crate::tree::touch_mru(&mut win.pane_mru, pid);
                }
                unzoom_if_zoomed(app);
            }
        }
        "unmark" => { app.marked_pane = None; }
        _ => {}
    }
}

pub(crate) fn handle_select_window(app: &mut AppState, idx: usize) {
    if let Some(cmds) = app.hooks.get("before-select-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    if idx >= app.window_base_index {
        let internal_idx = idx - app.window_base_index;
        if internal_idx < app.windows.len() && internal_idx != app.active_idx {
            switch_with_copy_save(app, |app| {
                app.last_window_idx = app.active_idx;
                app.active_idx = internal_idx;
            });
            resize_all_panes(app);
        }
    }
}
