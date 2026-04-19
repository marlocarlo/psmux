use super::*;
use super::srv_loop_ctx::LoopCtx;

/// Handle CtrlReq::DumpState: serialize full app state as JSON.
pub(crate) fn handle_dump_state(
    app: &mut AppState, ctx: &mut LoopCtx,
    resp: std::sync::mpsc::Sender<String>, allow_nc: bool,
) -> io::Result<()> {
    let alert_hooks = super::helpers::check_window_activity(app);
    for event in &alert_hooks {
        if let Some(cmds) = app.hooks.get(*event) { let cmds = cmds.clone(); for cmd in &cmds { let _ = execute_command_string(app, cmd); } }
    }
    if super::helpers::propagate_osc_titles(app) { ctx.state_dirty = true; }
    resolve_window_names(app, &mut ctx.state_dirty, &mut ctx.meta_dirty);

    let has_squelch = app.windows.get(app.active_idx)
        .and_then(|w| crate::tree::active_pane(&w.root, &w.active_path))
        .map_or(false, |p| p.squelch_until.is_some());
    if allow_nc && !ctx.state_dirty && !app.bell_forward && !has_squelch
       && !ctx.cached_dump_state.is_empty()
       && ctx.cached_data_version == combined_data_version(app)
    {
        let _ = resp.send("NC".to_string());
        return Ok(());
    }

    if ctx.meta_dirty { ctx.rebuild_meta_cache(app)?; }

    let _t_layout = std::time::Instant::now();
    let layout_json = dump_layout_json_fast(app)?;
    let _layout_ms = _t_layout.elapsed().as_micros();

    build_combined_json(app, ctx, &layout_json);

    ctx.cached_dump_state.clear();
    ctx.cached_dump_state.push_str(&ctx.combined_buf);

    // Inject one-shot clipboard data
    if let Some(clip_text) = app.clipboard_osc52.take() {
        let clip_b64 = base64_encode(&clip_text);
        if ctx.combined_buf.ends_with('}') {
            ctx.combined_buf.pop();
            ctx.combined_buf.push_str(",\"clipboard_osc52\":\"");
            ctx.combined_buf.push_str(&clip_b64);
            ctx.combined_buf.push_str("\"}");
        }
    }
    if app.bell_forward {
        app.bell_forward = false;
        if ctx.combined_buf.ends_with('}') {
            ctx.combined_buf.pop();
            ctx.combined_buf.push_str(",\"bell\":true}");
        }
    }
    ctx.cached_data_version = combined_data_version(app);
    ctx.state_dirty = false;

    // Timing log
    if std::env::var("PSMUX_LATENCY_LOG").unwrap_or_default() == "1" {
        let total_us = _t_layout.elapsed().as_micros();
        use std::io::Write as _;
        static SRV_LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();
        let log = SRV_LOG.get_or_init(|| {
            let p = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\gj".into())).join("psmux_server_latency.log");
            std::sync::Mutex::new(std::fs::File::create(p).expect("create latency log"))
        });
        if let Ok(mut f) = log.lock() {
            let _ = writeln!(f, "[SRV] dump: layout={}us total={}us json_len={}", _layout_ms, total_us, ctx.combined_buf.len());
        }
    }

    crate::types::push_frame(&ctx.combined_buf);
    let _ = resp.send(ctx.combined_buf.clone());
    Ok(())
}

/// Resolve automatic/allow-rename window names.
pub(crate) fn resolve_window_names(app: &mut AppState, state_dirty: &mut bool, meta_dirty: &mut bool) {
    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
    let auto_rename = app.automatic_rename;
    let allow_rename = app.allow_rename;
    if (auto_rename || allow_rename) && !in_copy {
        for win in app.windows.iter_mut() {
            if win.manual_rename { continue; }
            if let Some(p) = crate::tree::active_pane_mut(&mut win.root, &win.active_path) {
                if p.dead { continue; }
                if p.last_title_check.elapsed().as_millis() < 1000 { continue; }
                p.last_title_check = std::time::Instant::now();
                if p.child_pid.is_none() {
                    p.child_pid = crate::platform::mouse_inject::get_child_pid(&*p.child);
                }
                let new_name = if auto_rename {
                    if let Some(pid) = p.child_pid {
                        match crate::platform::process_info::get_foreground_process_name(pid) {
                            Some(name) => name,
                            None => continue,
                        }
                    } else if allow_rename && !p.title.is_empty() {
                        p.title.clone()
                    } else {
                        continue;
                    }
                } else if allow_rename {
                    if let Ok(parser) = p.term.lock() {
                        let title = parser.screen().title();
                        if !title.is_empty() { title.to_string() } else { continue; }
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                if !new_name.is_empty() && win.name != new_name {
                    win.name = new_name;
                    *meta_dirty = true;
                    *state_dirty = true;
                }
            }
        }
    }
}

/// Build the combined JSON envelope into ctx.combined_buf.
pub(crate) fn build_combined_json(app: &AppState, ctx: &mut LoopCtx, layout_json: &str) {
    ctx.combined_buf.clear();
    let ss_escaped = json_escape_string(&ctx.cached_status_style);
    let sl_expanded = json_escape_string(&expand_format(&app.status_left, app));
    let sr_expanded = json_escape_string(&expand_format(&app.status_right, app));
    let pbs_escaped = json_escape_string(&app.pane_border_style);
    let pabs_escaped = json_escape_string(&app.pane_active_border_style);
    let pbhs_escaped = json_escape_string(&app.pane_border_hover_style);
    let wsf_escaped = json_escape_string(&app.window_status_format);
    let wscf_escaped = json_escape_string(&app.window_status_current_format);
    let wss_escaped = json_escape_string(&app.window_status_separator);
    let ws_style_escaped = json_escape_string(&app.window_status_style);
    let wsc_style_escaped = json_escape_string(&app.window_status_current_style);
    let mode_style_escaped = json_escape_string(&app.mode_style);
    let status_position_escaped = json_escape_string(&app.status_position);
    let status_justify_escaped = json_escape_string(&app.status_justify);
    let status_format_json = {
        let mut sf = String::from("[");
        for (i, fmt_str) in app.status_format.iter().enumerate() {
            if i > 0 { sf.push(','); }
            sf.push('"');
            sf.push_str(&json_escape_string(&expand_format(fmt_str, app)));
            sf.push('"');
        }
        sf.push(']');
        sf
    };
    let cursor_style_code = crate::rendering::configured_cursor_code();
    let _ = std::fmt::Write::write_fmt(&mut ctx.combined_buf, format_args!(
        "{{\"layout\":{},\"windows\":{},\"prefix\":\"{}\",\"prefix2\":\"{}\",\"tree\":{},\"base_index\":{},\"pane_base_index\":{},\"prediction_dimming\":{},\"status_style\":\"{}\",\"status_left\":\"{}\",\"status_right\":\"{}\",\"pane_border_style\":\"{}\",\"pane_active_border_style\":\"{}\",\"pane_border_hover_style\":\"{}\",\"wsf\":\"{}\",\"wscf\":\"{}\",\"wss\":\"{}\",\"ws_style\":\"{}\",\"wsc_style\":\"{}\",\"clock_mode\":{},\"bindings\":{},\"status_left_length\":{},\"status_right_length\":{},\"status_lines\":{},\"status_format\":{},\"mode_style\":\"{}\",\"status_position\":\"{}\",\"status_justify\":\"{}\",\"cursor_style_code\":{},\"status_visible\":{},\"repeat_time\":{},\"zoomed\":{},\"defaults_suppressed\":{},\"pwsh_mouse_selection\":{}}}",
        layout_json, ctx.cached_windows_json, ctx.cached_prefix_str, ctx.cached_prefix2_str, ctx.cached_tree_json, ctx.cached_base_index, app.pane_base_index, ctx.cached_pred_dim, ss_escaped, sl_expanded, sr_expanded, pbs_escaped, pabs_escaped, pbhs_escaped, wsf_escaped, wscf_escaped, wss_escaped, ws_style_escaped, wsc_style_escaped,
        matches!(app.mode, Mode::ClockMode), ctx.cached_bindings_json,
        app.status_left_length, app.status_right_length, app.status_lines, status_format_json,
        mode_style_escaped, status_position_escaped, status_justify_escaped,
        cursor_style_code, app.status_visible, app.repeat_time_ms,
        app.windows.get(app.active_idx).map_or(false, |w| w.zoom_saved.is_some()),
        app.defaults_suppressed,
        app.pwsh_mouse_selection,
    ));
    // Inject overlay extras
    inject_overlay_extras(app, &mut ctx.combined_buf);
}

/// Inject clock_colour, pane-border-status/format, and overlay JSON into combined_buf.
fn inject_overlay_extras(app: &AppState, buf: &mut String) {
    if let Some(cc) = app.user_options.get("clock-mode-colour") {
        if buf.ends_with('}') {
            buf.pop();
            buf.push_str(",\"clock_colour\":\"");
            buf.push_str(&json_escape_string(cc));
            buf.push_str("\"}");
        }
    }
    if let Some(pbs) = app.user_options.get("pane-border-status") {
        if buf.ends_with('}') {
            buf.pop();
            buf.push_str(",\"pane_border_status\":\"");
            buf.push_str(&json_escape_string(pbs));
            buf.push('"');
            if let Some(pbf) = app.user_options.get("pane-border-format") {
                buf.push_str(",\"pane_border_format\":\"");
                buf.push_str(&json_escape_string(pbf));
                buf.push('"');
            }
            buf.push('}');
        }
    }
    let overlay_json = serialize_overlay_json(app);
    if !overlay_json.is_empty() && buf.ends_with('}') {
        buf.pop();
        buf.push_str(&overlay_json);
        buf.push('}');
    }
}

/// Server-push: rebuild JSON and push to all persistent clients.
pub(crate) fn push_frame_if_dirty(app: &mut AppState, ctx: &mut LoopCtx) -> io::Result<()> {
    if !(ctx.state_dirty || ctx.meta_dirty) || !crate::types::has_frame_receivers() {
        return Ok(());
    }
    let push_alert_hooks = super::helpers::check_window_activity(app);
    for event in &push_alert_hooks { crate::commands::fire_hooks(app, event); }
    if ctx.meta_dirty { ctx.rebuild_meta_cache(app)?; }
    let layout_json = dump_layout_json_fast(app)?;
    build_combined_json(app, ctx, &layout_json);
    // Inject clipboard
    if let Some(clip_text) = app.clipboard_osc52.take() {
        let clip_b64 = base64_encode(&clip_text);
        if ctx.combined_buf.ends_with('}') {
            ctx.combined_buf.pop();
            ctx.combined_buf.push_str(",\"clipboard_osc52\":\"");
            ctx.combined_buf.push_str(&clip_b64);
            ctx.combined_buf.push_str("\"}");
        }
    }
    ctx.cached_dump_state.clear();
    ctx.cached_dump_state.push_str(&ctx.combined_buf);
    if app.bell_forward {
        app.bell_forward = false;
        if ctx.combined_buf.ends_with('}') {
            ctx.combined_buf.pop();
            ctx.combined_buf.push_str(",\"bell\":true}");
        }
    }
    ctx.cached_data_version = combined_data_version(app);
    ctx.state_dirty = false;
    crate::types::push_frame(&ctx.combined_buf);
    Ok(())
}
