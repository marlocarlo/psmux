use super::*;
use super::srv_loop_ctx::LoopCtx;

pub(crate) fn handle_new_window(
    app: &mut AppState, ctx: &mut LoopCtx,
    cmd: Option<String>, name: Option<String>, detached: bool, start_dir: Option<String>,
) -> io::Result<()> {
    if let Some(cmds) = app.hooks.get("before-new-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    let prev_idx = app.active_idx;
    let start_dir = start_dir.map(|d| expand_format(&d, app)).filter(|d| !d.is_empty());
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
    if let Err(e) = create_window(&*ctx.pty_system, app, cmd.as_deref(), start_dir.as_deref()) {
        eprintln!("psmux: new-window error: {e}");
    }
    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    if let Some(n) = name { app.windows.last_mut().map(|w| { w.name = n; w.manual_rename = true; }); }
    if detached { app.active_idx = prev_idx; }
    if app.warm_pane.is_none() {
        if let Ok(wp) = spawn_warm_pane(&*ctx.pty_system, app) { app.warm_pane = Some(wp); }
    }
    resize_all_panes(app);
    Ok(())
}

pub(crate) fn handle_new_window_print(
    app: &mut AppState, ctx: &mut LoopCtx,
    cmd: Option<String>, name: Option<String>, detached: bool, start_dir: Option<String>,
    format_str: Option<String>, resp: std::sync::mpsc::Sender<String>,
) -> io::Result<()> {
    if let Some(cmds) = app.hooks.get("before-new-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    let prev_idx = app.active_idx;
    let start_dir = start_dir.map(|d| expand_format(&d, app)).filter(|d| !d.is_empty());
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
    if let Err(e) = create_window(&*ctx.pty_system, app, cmd.as_deref(), start_dir.as_deref()) {
        eprintln!("psmux: new-window error: {e}");
    }
    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    if let Some(n) = name { app.windows.last_mut().map(|w| { w.name = n; w.manual_rename = true; }); }
    let new_win_idx = app.windows.len() - 1;
    let fmt = format_str.as_deref().unwrap_or("#{session_name}:#{window_index}");
    let pane_info = crate::format::expand_format_for_window(fmt, app, new_win_idx);
    if detached { app.active_idx = prev_idx; }
    let _ = resp.send(pane_info);
    if app.warm_pane.is_none() {
        if let Ok(wp) = spawn_warm_pane(&*ctx.pty_system, app) { app.warm_pane = Some(wp); }
    }
    resize_all_panes(app);
    Ok(())
}

pub(crate) fn handle_split_window(
    app: &mut AppState, ctx: &mut LoopCtx,
    k: LayoutKind, cmd: Option<String>, detached: bool, start_dir: Option<String>,
    split_size: Option<(u16, bool)>, resp: std::sync::mpsc::Sender<String>,
) -> io::Result<()> {
    if let Some(cmds) = app.hooks.get("before-split-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    unzoom_if_zoomed(app);
    let start_dir = start_dir.map(|d| expand_format(&d, app)).filter(|d| !d.is_empty());
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
    let prev_path = app.windows[app.active_idx].active_path.clone();
    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
    if let Err(e) = split_active_with_command(app, k, cmd.as_deref(), Some(&*ctx.pty_system), start_dir.as_deref()) {
        let _ = resp.send(format!("psmux: split-window: {e}"));
    } else {
        let _ = resp.send(String::new());
    }
    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
    apply_split_size(app, &prev_path, k, split_size);
    if detached {
        let new_pane_id = crate::tree::get_active_pane_id(&app.windows[app.active_idx].root, &app.windows[app.active_idx].active_path);
        let mut revert_path = prev_path;
        revert_path.push(0);
        app.windows[app.active_idx].active_path = revert_path;
        if let Some(nid) = new_pane_id { app.windows[app.active_idx].pane_mru.retain(|&id| id != nid); }
    } else {
        ctx.temp_focus_restore = None;
    }
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    if app.warm_pane.is_none() {
        if let Ok(wp) = spawn_warm_pane(&*ctx.pty_system, app) { app.warm_pane = Some(wp); }
    }
    resize_all_panes(app);
    Ok(())
}

pub(crate) fn handle_split_window_print(
    app: &mut AppState, ctx: &mut LoopCtx,
    k: LayoutKind, cmd: Option<String>, detached: bool, start_dir: Option<String>,
    split_size: Option<(u16, bool)>, format_str: Option<String>, resp: std::sync::mpsc::Sender<String>,
) -> io::Result<()> {
    if let Some(cmds) = app.hooks.get("before-split-window") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    unzoom_if_zoomed(app);
    let start_dir = start_dir.map(|d| expand_format(&d, app)).filter(|d| !d.is_empty());
    let saved_dir = if start_dir.is_some() { env::current_dir().ok() } else { None };
    if let Some(dir) = &start_dir { env::set_current_dir(dir).ok(); }
    let prev_path = app.windows[app.active_idx].active_path.clone();
    let stashed_warm = if start_dir.is_some() { app.warm_pane.take() } else { None };
    if let Err(e) = split_active_with_command(app, k, cmd.as_deref(), Some(&*ctx.pty_system), start_dir.as_deref()) {
        eprintln!("psmux: split-window error: {e}");
    }
    if let Some(wp) = stashed_warm { app.warm_pane = Some(wp); }
    apply_split_size(app, &prev_path, k, split_size);
    let fmt = format_str.as_deref().unwrap_or("#{session_name}:#{window_index}.#{pane_index}");
    let pane_info = crate::format::expand_format_for_window(fmt, app, app.active_idx);
    if detached {
        let new_pane_id = crate::tree::get_active_pane_id(&app.windows[app.active_idx].root, &app.windows[app.active_idx].active_path);
        let mut revert_path = prev_path;
        revert_path.push(0);
        app.windows[app.active_idx].active_path = revert_path;
        if let Some(nid) = new_pane_id { app.windows[app.active_idx].pane_mru.retain(|&id| id != nid); }
    } else {
        ctx.temp_focus_restore = None;
    }
    let _ = resp.send(pane_info);
    if let Some(prev) = saved_dir { env::set_current_dir(prev).ok(); }
    if app.warm_pane.is_none() {
        if let Ok(wp) = spawn_warm_pane(&*ctx.pty_system, app) { app.warm_pane = Some(wp); }
    }
    resize_all_panes(app);
    Ok(())
}

fn apply_split_size(app: &mut AppState, prev_path: &[usize], k: LayoutKind, split_size: Option<(u16, bool)>) {
    if let Some((val, is_pct)) = split_size {
        let pct = if is_pct {
            val.clamp(1, 99)
        } else {
            let area = app.last_window_area;
            let total = if k == LayoutKind::Horizontal { area.width } else { area.height };
            if total > 0 { ((val as u32 * 100) / total as u32).clamp(1, 99) as u16 } else { 50 }
        };
        let win = &mut app.windows[app.active_idx];
        if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &prev_path.to_vec()) {
            sizes[0] = 100 - pct;
            sizes[1] = pct;
        }
    }
}

pub(crate) fn handle_join_pane(
    app: &mut AppState,
    src_win: Option<usize>, src_pane: Option<usize>,
    target_win: Option<usize>, target_pane: Option<usize>, horizontal: bool,
) -> io::Result<Option<&'static str>> {
    unzoom_if_zoomed(app);
    let src_idx = src_win.unwrap_or(app.active_idx);
    let raw_target_win = target_win.unwrap_or(app.active_idx);
    if src_idx < app.windows.len() && raw_target_win < app.windows.len() && src_idx != raw_target_win {
        let src_path = if let Some(pidx) = src_pane {
            let mut leaves = Vec::new();
            tree::collect_leaf_paths_pub(&app.windows[src_idx].root, &mut Vec::new(), &mut leaves);
            if let Some((_, p)) = leaves.get(pidx) { p.clone() } else { app.windows[src_idx].active_path.clone() }
        } else {
            app.windows[src_idx].active_path.clone()
        };
        if let Some(saved) = app.windows[src_idx].zoom_saved.take() {
            let win = &mut app.windows[src_idx];
            for (p, sz) in saved.into_iter() {
                if let Some(Node::Split { sizes, .. }) = crate::tree::get_split_mut(&mut win.root, &p) { *sizes = sz; }
            }
        }
        let src_root = std::mem::replace(&mut app.windows[src_idx].root,
            Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
        let (remaining, extracted) = tree::extract_node(src_root, &src_path);
        if let Some(pane_node) = extracted {
            let src_empty = remaining.is_none();
            if let Some(rem) = remaining {
                app.windows[src_idx].root = rem;
                app.windows[src_idx].active_path = tree::first_leaf_path(&app.windows[src_idx].root);
            }
            let tgt = if src_empty && raw_target_win > src_idx { raw_target_win - 1 } else { raw_target_win };
            if src_empty {
                app.windows.remove(src_idx);
                if app.active_idx >= app.windows.len() { app.active_idx = app.windows.len().saturating_sub(1); }
            }
            if tgt < app.windows.len() {
                let tgt_path = if let Some(tpidx) = target_pane {
                    let mut leaves = Vec::new();
                    tree::collect_leaf_paths_pub(&app.windows[tgt].root, &mut Vec::new(), &mut leaves);
                    if let Some((_, p)) = leaves.get(tpidx) { p.clone() } else { app.windows[tgt].active_path.clone() }
                } else {
                    app.windows[tgt].active_path.clone()
                };
                let split_kind = if horizontal { LayoutKind::Horizontal } else { LayoutKind::Vertical };
                tree::replace_leaf_with_split(&mut app.windows[tgt].root, &tgt_path, split_kind, pane_node);
                app.active_idx = tgt;
            }
            resize_all_panes(app);
            return Ok(Some("after-join-pane"));
        } else {
            if let Some(rem) = remaining { app.windows[src_idx].root = rem; }
        }
    }
    Ok(None)
}

pub(crate) fn handle_link_window(
    app: &mut AppState, ctx: &mut LoopCtx,
    src_idx_opt: Option<usize>, dst_idx_opt: Option<usize>,
) -> io::Result<Option<&'static str>> {
    let src = src_idx_opt.unwrap_or(app.active_idx);
    if src < app.windows.len() {
        let src_id = app.windows[src].id;
        let src_name = app.windows[src].name.clone();
        let dst = dst_idx_opt.unwrap_or(app.windows.len());
        let pty_system = portable_pty::native_pty_system();
        match crate::pane::create_window(&*pty_system, app, None, None) {
            Ok(()) => {
                let new_idx = app.windows.len() - 1;
                app.windows[new_idx].linked_from = Some(src_id);
                app.windows[new_idx].name = src_name;
                if dst < new_idx {
                    let win = app.windows.remove(new_idx);
                    app.windows.insert(dst, win);
                    if app.active_idx > dst && app.active_idx <= new_idx {
                        app.active_idx = app.active_idx.saturating_sub(1);
                    }
                }
                resize_all_panes(app);
                ctx.meta_dirty = true;
                return Ok(Some("window-linked"));
            }
            Err(_e) => {
                app.status_message = Some(("link-window: failed to create linked window".to_string(), std::time::Instant::now(), None));
            }
        }
    } else {
        app.status_message = Some(("link-window: source window not found".to_string(), std::time::Instant::now(), None));
    }
    ctx.state_dirty = true;
    Ok(None)
}

pub(crate) fn handle_claim_session(
    app: &mut AppState, ctx: &mut LoopCtx,
    name: String, client_cwd: Option<String>, resp: std::sync::mpsc::Sender<String>,
) -> io::Result<()> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let old_path = format!("{}\\.psmux\\{}.port", home, app.port_file_base());
    let old_keypath = format!("{}\\.psmux\\{}.key", home, app.port_file_base());
    let new_base = if let Some(ref sn) = app.socket_name { format!("{}__{}" , sn, name) } else { name.clone() };
    let new_path = format!("{}\\.psmux\\{}.port", home, new_base);
    let new_keypath = format!("{}\\.psmux\\{}.key", home, new_base);
    if let Some(port) = app.control_port {
        let _ = std::fs::remove_file(&old_path);
        let _ = std::fs::write(&new_path, port.to_string());
        if let Ok(key) = std::fs::read_to_string(&old_keypath) {
            let _ = std::fs::remove_file(&old_keypath);
            let _ = std::fs::write(&new_keypath, key);
        }
    }
    app.session_name = name;
    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());
    if let Some(ref cwd) = client_cwd {
        let cwd_path = std::path::Path::new(cwd);
        if cwd_path.is_dir() {
            let server_cwd_differs = env::current_dir().map(|cur| cur != cwd_path).unwrap_or(true);
            if server_cwd_differs {
                env::set_current_dir(cwd_path).ok();
                if let Some(win) = app.windows.last_mut() {
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                        use std::io::Write as _;
                        let escaped = cwd.replace('\'', "''");
                        let clear = if cfg!(windows) { "cls" } else { "clear" };
                        let cd_cmd = format!(" cd '{}'; {}\r", escaped, clear);
                        if let Ok(mut parser) = p.term.lock() { parser.screen_mut().set_squelch_clear_pending(true); }
                        p.squelch_until = Some(Instant::now() + Duration::from_millis(500));
                        let _ = p.writer.write_all(cd_cmd.as_bytes());
                        let _ = p.writer.flush();
                    }
                }
            }
        }
    }
    env::set_var("PSMUX_TARGET_SESSION", app.port_file_base());
    app.key_tables.clear();
    app.defaults_suppressed = false;
    crate::config::populate_default_bindings(app);
    load_config(app);
    if let Ok(mut w) = ctx.shared_aliases.write() { *w = app.command_aliases.clone(); }
    if let Some(cmds) = app.hooks.get("client-session-changed") { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    ctx.meta_dirty = true;
    ctx.state_dirty = true;
    let _ = resp.send("OK\n".to_string());
    spawn_warm_server(app);
    Ok(())
}
