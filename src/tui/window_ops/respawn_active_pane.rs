#[allow(unused_imports)]
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{PtySize, native_pty_system};
use ratatui::prelude::*;

use crate::types::{AppState, Mode, Pane, Node, LayoutKind, DragState, Window, FocusDir};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, get_split_mut, resize_all_panes};
use crate::pane::{detect_shell, build_default_shell, set_tmux_env};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, yank_selection};
use crate::platform::mouse_inject;

/// Mouse debug logger — writes to ~/.psmux/mouse_debug.log when
/// PSMUX_MOUSE_DEBUG=1 is set.
use super::*;

pub fn respawn_active_pane(app: &mut AppState, pty_system_ref: Option<&dyn portable_pty::PtySystem>, workdir: Option<&str>, kill: bool) -> io::Result<()> {
    // tmux semantics: without -k, respawn only works on dead panes.
    // With -k, kill the running process first and respawn.
    {
        let win = &app.windows[app.active_idx];
        if let Some(pane) = crate::tree::active_pane(&win.root, &win.active_path) {
            if !pane.dead && !kill {
                return Err(io::Error::new(io::ErrorKind::Other, "pane still active"));
            }
        }
    }
    // If -k and pane is alive, kill the child process first
    if kill {
        let win = &mut app.windows[app.active_idx];
        if let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) {
            if !pane.dead {
                crate::platform::process_kill::kill_process_tree(&mut pane.child);
                pane.dead = true;
            }
        }
    }

    // Reuse provided PTY system or create one as fallback
    let owned_pty;
    let pty_system: &dyn portable_pty::PtySystem = if let Some(ps) = pty_system_ref {
        ps
    } else {
        owned_pty = native_pty_system();
        &*owned_pty
    };
    // Expand format variables like #{pane_current_path} at spawn time (#111).
    // Must happen before the mutable borrow of app.windows below.
    let expanded_shell = crate::format::expand_format(&app.default_shell, &app);

    let win = &mut app.windows[app.active_idx];
    let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) else { return Ok(()); };
    let pane_id = pane.id;
    
    let size = PtySize { rows: pane.last_rows, cols: pane.last_cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    let mut shell_cmd = if !expanded_shell.is_empty() {
        build_default_shell(&expanded_shell, app.env_shim, app.allow_predictions)
    } else {
        detect_shell()
    };
    set_tmux_env(&mut shell_cmd, pane_id, app.control_port, app.socket_name.as_deref(), &app.session_name, app.claude_code_fix_tty, app.claude_code_force_interactive);
    crate::pane::apply_user_environment(&mut shell_cmd, &app.environment);
    if let Some(dir) = workdir {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        let expanded = dir.replace("~/", &format!("{}/", home))
            .replace("~\\", &format!("{}\\", home));
        shell_cmd.cwd(std::path::Path::new(&expanded));
    }
    let child = pair.slave.spawn_command(shell_cmd).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    // Close the slave handle immediately – required for ConPTY.
    drop(pair.slave);
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, app.history_limit)));
    let term_reader = term.clone();
    let reader = pair.master.try_clone_reader().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(crate::pane::CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    
    let output_ring = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    crate::pane::spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer, output_ring.clone());
    pane.output_ring = output_ring;
    
    let mut pty_writer = pair.master.take_writer().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    crate::pane::conpty_preemptive_dsr_response(&mut *pty_writer);
    
    pane.master = pair.master;
    pane.writer = pty_writer;
    pane.child = child;
    pane.term = term;
    pane.data_version = data_version;
    pane.cursor_shape = cursor_shape;
    pane.bell_pending = bell_pending;
    pane.child_pid = None;
    pane.vt_bridge_cache = None;
    pane.vti_mode_cache = None;
    pane.mouse_input_cache = None;
    pane.dead = false;
    
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests-rs/test_issue81_resize_direction.rs"]
mod test_issue81_resize_direction;
