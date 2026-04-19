#[allow(unused_imports)]
use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::types::{AppState, Pane, Node, LayoutKind, Window};
use crate::tree::{replace_leaf_with_split, active_pane_mut, kill_leaf};
use crate::format::hostname_cached;

/// Sentinel value for cursor_shape: means "no DECSCUSR received from child yet".
/// When ConPTY passthrough mode is unavailable, DECSCUSR sequences from child
/// processes are consumed by ConPTY and never forwarded.  Using this sentinel
/// lets the rendering code skip emitting any cursor-shape override, so the
/// real terminal keeps its user-configured default cursor.
use super::*;

pub fn split_active_with_command(app: &mut AppState, kind: LayoutKind, command: Option<&str>, pty_system_ref: Option<&dyn portable_pty::PtySystem>, start_dir: Option<&str>) -> io::Result<()> {
    // ── Guard: refuse split if the active pane is too small ──────────
    // After splitting, each half gets roughly (dim / 2) - 1 (for the divider).
    // If that would be below MIN_PANE_DIM, deny the split to avoid crashing
    // the child process (ConPTY cannot function below ~2 rows or cols).
    {
        let win = &app.windows[app.active_idx];
        if let Some(p) = crate::tree::active_pane(&win.root, &win.active_path) {
            let (cur_rows, cur_cols) = (p.last_rows, p.last_cols);
            match kind {
                LayoutKind::Vertical => {
                    // Splitting vertically divides height; need room for 2 panes + 1 divider
                    if cur_rows < MIN_SPLIT_ROWS * 2 + 1 {
                        return Err(io::Error::new(io::ErrorKind::Other,
                            format!("pane too small to split vertically ({cur_rows} rows, need {})", MIN_SPLIT_ROWS * 2 + 1)));
                    }
                }
                LayoutKind::Horizontal => {
                    // Splitting horizontally divides width; need room for 2 panes + 1 divider
                    if cur_cols < MIN_SPLIT_COLS * 2 + 1 {
                        return Err(io::Error::new(io::ErrorKind::Other,
                            format!("pane too small to split horizontally ({cur_cols} cols, need {})", MIN_SPLIT_COLS * 2 + 1)));
                    }
                }
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
    // Compute target pane size from the *active pane's* actual dimensions,
    // not the full window area — ensures we don't over-estimate and then
    // immediately resize to a tiny rect.
    let (pane_rows, pane_cols) = {
        let win = &app.windows[app.active_idx];
        if let Some(p) = crate::tree::active_pane(&win.root, &win.active_path) {
            (p.last_rows, p.last_cols)
        } else {
            let area = app.last_window_area;
            (if area.height > 1 { area.height } else { 30 }, if area.width > 1 { area.width } else { 120 })
        }
    };
    let (rows, cols) = match kind {
        LayoutKind::Vertical => {
            let half = (pane_rows.saturating_sub(1)) / 2; // subtract 1 for divider
            (half.max(MIN_PANE_DIM), pane_cols.max(MIN_PANE_DIM))
        }
        LayoutKind::Horizontal => {
            let half = (pane_cols.saturating_sub(1)) / 2;
            (pane_rows.max(MIN_PANE_DIM), half.max(MIN_PANE_DIM))
        }
    };
    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };

    // ── Fast path: transplant warm pane for default-shell splits ─────
    // The warm pane has its shell already loaded (~470ms for pwsh).  Even
    // though its ConPTY was created at full-window size, resizing to the
    // split dimensions only costs a ConPTY repaint (~10-50ms) vs a full
    // cold spawn (~500ms).  Net result: split feels nearly instant.
    // Skip warm pane when start_dir is set — the warm pane was spawned
    // in the server's CWD, not the requested directory (#107).
    if command.is_none() && start_dir.is_none() && app.warm_pane.is_some() {
        let wp = app.warm_pane.take().unwrap();
        // Resize ConPTY + parser to the split dimensions
        if rows != wp.rows || cols != wp.cols {
            let sz = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
            wp.master.resize(sz).ok();
            if let Ok(mut parser) = wp.term.lock() {
                parser.screen_mut().set_size(rows, cols);
            }
        }
        let epoch = std::time::Instant::now() - Duration::from_secs(2);
        let new_pane_id = wp.pane_id;
        let new_leaf = Node::Leaf(Pane { master: wp.master, writer: wp.writer, child: wp.child, term: wp.term, last_rows: rows, last_cols: cols, id: new_pane_id, title: hostname_cached(), title_locked: false, child_pid: wp.child_pid, data_version: wp.data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape: wp.cursor_shape, bell_pending: wp.bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring: wp.output_ring });
        let win = &mut app.windows[app.active_idx];
        replace_leaf_with_split(&mut win.root, &win.active_path, kind, new_leaf);
        let mut new_path = win.active_path.clone();
        new_path.push(1);
        win.active_path = new_path;
        // Add new pane to MRU (most recent)
        crate::tree::touch_mru(&mut win.pane_mru, new_pane_id);
        return Ok(());
    }

    // ── Normal path: cold-spawn a new ConPTY + shell ────────────────
    let pair = pty_system.openpty(size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    // When no explicit command is given, use the configured default-shell.
    // Expand format variables like #{pane_current_path} at spawn time (#111).
    let expanded_shell = crate::format::expand_format(&app.default_shell, app);
    let mut shell_cmd = if command.is_some() {
        build_command(command, app.env_shim, app.allow_predictions)
    } else if !expanded_shell.is_empty() {
        build_default_shell(&expanded_shell, app.env_shim, app.allow_predictions)
    } else {
        build_command(None, app.env_shim, app.allow_predictions)
    };
    // Override CWD if -c start_dir was specified
    if let Some(dir) = start_dir {
        shell_cmd.cwd(std::path::Path::new(dir));
    }
    set_tmux_env(&mut shell_cmd, app.next_pane_id, app.control_port, app.socket_name.as_deref(), &app.session_name, app.claude_code_fix_tty, app.claude_code_force_interactive);
    apply_user_environment(&mut shell_cmd, &app.environment);
    let child = pair.slave.spawn_command(shell_cmd).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    // Close the slave handle immediately – see create_window() comment.
    drop(pair.slave);
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, app.history_limit)));
    let term_reader = term.clone();
    let reader = pair.master.try_clone_reader().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dv_writer = data_version.clone();
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(CURSOR_SHAPE_UNSET));
    let cs_writer = cursor_shape.clone();
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bell_writer = bell_pending.clone();
    let output_ring = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<u8>::new()));
    spawn_reader_thread(reader, term_reader, dv_writer, cs_writer, bell_writer, output_ring.clone());
    let child_pid = crate::platform::mouse_inject::get_child_pid(&*child);
    let mut pty_writer = pair.master.take_writer()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("take writer error: {e}")))?;
    conpty_preemptive_dsr_response(&mut *pty_writer);
    let epoch = std::time::Instant::now() - Duration::from_secs(2);
    let split_pane_id = app.next_pane_id;
    let new_leaf = Node::Leaf(Pane { master: pair.master, writer: pty_writer, child, term, last_rows: size.rows, last_cols: size.cols, id: split_pane_id, title: hostname_cached(), title_locked: false, child_pid, data_version, last_title_check: epoch, last_infer_title: epoch, dead: false, vt_bridge_cache: None, vti_mode_cache: None, mouse_input_cache: None, cursor_shape, bell_pending, copy_state: None, pane_style: None, squelch_until: None, output_ring });
    app.next_pane_id += 1;
    let win = &mut app.windows[app.active_idx];
    replace_leaf_with_split(&mut win.root, &win.active_path, kind, new_leaf);
    let mut new_path = win.active_path.clone();
    new_path.push(1);
    win.active_path = new_path;
    // Add new pane to MRU (most recent)
    crate::tree::touch_mru(&mut win.pane_mru, split_pane_id);
    Ok(())
}

pub(crate) fn kill_pane_at_path(win: &mut Window, path: &Vec<usize>) {
    // Get the ID of the pane being killed (for MRU removal)
    let killed_id = crate::tree::get_active_pane_id(&win.root, path);
    // Collect ordered pane IDs before kill for prev-by-index fallback (#71).
    let ordered_ids_before = crate::tree::collect_pane_ids(&win.root);
    // Explicitly kill the target pane's process tree FIRST.
    // remove_node() doesn't call kill_node() when the root is a single Leaf,
    // so we must do it here to ensure no orphaned processes.
    if let Some(p) = active_pane_mut(&mut win.root, path) {
        crate::platform::process_kill::kill_process_tree(&mut p.child);
    }
    kill_leaf(&mut win.root, path);
    // Remove killed pane from MRU
    if let Some(kid) = killed_id {
        crate::tree::remove_from_mru(&mut win.pane_mru, kid);
    }
    // Focus the most recently used remaining pane (tmux parity #71).
    // Walk the MRU list and pick the first pane that still exists.
    let mru_target = win.pane_mru.iter()
        .find_map(|&id| crate::tree::find_path_by_id(&win.root, id));
    // Fallback when MRU is empty (all remaining panes unvisited):
    // tmux picks previous pane by pane_index, or next if no previous.
    let fallback = || {
        if let Some(kid) = killed_id {
            let pos = ordered_ids_before.iter().position(|&id| id == kid);
            if let Some(pos) = pos {
                // Try previous by index first, then next
                let prev_id = if pos > 0 { Some(ordered_ids_before[pos - 1]) } else { None };
                let next_id = ordered_ids_before.get(pos + 1).copied();
                let candidate = prev_id.or(next_id);
                if let Some(cid) = candidate {
                    if let Some(path) = crate::tree::find_path_by_id(&win.root, cid) {
                        return path;
                    }
                }
            }
        }
        crate::tree::first_leaf_path(&win.root)
    };
    win.active_path = mru_target.unwrap_or_else(fallback);
}

pub fn kill_active_pane(app: &mut AppState) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    let active_path = win.active_path.clone();
    kill_pane_at_path(win, &active_path);
    Ok(())
}

pub fn kill_pane_by_id(app: &mut AppState, pane_id: usize) -> io::Result<()> {
    let restore_idx = app.active_idx;
    let restore_path = app.windows[restore_idx].active_path.clone();
    let restore_pane_id = crate::tree::get_active_pane_id(&app.windows[restore_idx].root, &restore_path);

    let target = app.windows.iter().enumerate().find_map(|(wi, win)| {
        crate::tree::find_path_by_id(&win.root, pane_id).map(|path| (wi, path))
    });

    let Some((target_idx, target_path)) = target else {
        return Ok(());
    };

    {
        let win = &mut app.windows[target_idx];
        kill_pane_at_path(win, &target_path);
    }

    // Only restore focus when the killed pane was in a DIFFERENT window.
    // For same-window kills, kill_pane_at_path already set the correct
    // MRU-based focus.  The old restore logic used path_exists() which
    // can succeed on stale indices that now point to a different pane
    // after tree restructuring (issue #140).
    if restore_idx < app.windows.len() && target_idx != restore_idx {
        app.active_idx = restore_idx;
        let restore_win = &mut app.windows[restore_idx];
        let resolved_restore_path = restore_pane_id
            .and_then(|id| crate::tree::find_path_by_id(&restore_win.root, id))
            .unwrap_or_else(|| crate::tree::first_leaf_path(&restore_win.root));
        restore_win.active_path = resolved_restore_path;
    }

    Ok(())
}

/// Set TMUX, TMUX_PANE, and PSMUX_SESSION environment variables on a CommandBuilder.
/// TMUX format: /tmp/psmux-{server_pid}/{socket_name},{port},0
/// TMUX_PANE format: %{pane_id}
/// PSMUX_SESSION: actual session name (for Claude Code / tool detection)
/// The socket_name component encodes the -L namespace for child process resolution.
pub fn set_tmux_env(builder: &mut CommandBuilder, pane_id: usize, control_port: Option<u16>, socket_name: Option<&str>, session_name: &str, fix_tty: bool, _force_interactive: bool) {
    let server_pid = std::process::id();
    let port = control_port.unwrap_or(0);
    let sn = socket_name.unwrap_or("default");
    // Format compatible with tmux: <socket_path>,<pid>,<session_idx>
    // We encode the socket name in the path component for -L namespace resolution
    builder.env("TMUX", format!("/tmp/psmux-{}/{},{},0", server_pid, sn, port));
    builder.env("TMUX_PANE", format!("%{}", pane_id));
    // Override the placeholder "1" from build_command/build_default_shell with the
    // real session name.  Tools like Claude Code can use PSMUX_SESSION for explicit
    // psmux detection (e.g. `if (process.env.PSMUX_SESSION) return 'psmux'`).
    builder.env("PSMUX_SESSION", session_name);
    // Prevent MSYS2/Git-Bash from path-mangling the TMUX value (which starts
    // with /tmp/ and would be rewritten to a Windows path otherwise).
    builder.env("MSYS2_ENV_CONV_EXCL", "TMUX");
    // Enable Claude Code agent teams feature.  The standalone binary gates
    // the entire teammate tool-set (spawnTeam, spawnTeammate, …) behind
    //   T8(): LA(process.env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS) || --agent-teams
    // Without this env var the team tools are never registered and Claude
    // always falls back to the in-process "Agent" tool.
    builder.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");

    // ── Claude Code workarounds (removable once upstream fixes land) ──
    //
    // claude-code-fix-tty (set -g claude-code-fix-tty on/off):
    //   Claude Code v2.1.71 standalone binary ignores `teammateMode` from
    //   settings.json (config schema strips the field).  The `--teammate-mode
    //   tmux` CLI flag DOES work.  We set PSMUX_CLAUDE_TEAMMATE_MODE=tmux so
    //   the PowerShell env-shim `claude` wrapper function injects the flag
    //   automatically.  Disable with: set -g claude-code-fix-tty off
    if fix_tty {
        builder.env("PSMUX_CLAUDE_TEAMMATE_MODE", "tmux");
    }

}

/// PowerShell env shim snippet — defines a `Global:env` function that translates
/// POSIX `env VAR=val ... command args` invocations into PowerShell equivalents.
///
/// Key design decisions for Windows + Claude Code agent teams compatibility:
///   1. POSIX backslash-escape removal uses `\\([^\w\\])` so that escapes like
///      `\@` and `\:` (produced by shell-quote) are stripped, while Windows
///      path separators (`\U` in `C:\Users`) are preserved (letter after `\`
///      is a `\w` character, so the regex does NOT match).
///   2. Escape stripping is applied to ALL arguments (env var values, the
///      command itself, and every trailing arg), not just env-var values.
///   3. `.js` / `.mjs` files are detected and automatically executed via
///      `node` because Windows associates `.js` with WScript.exe (WSH),
///      which cannot run Node.js code and instead shows error dialogs.
///   4. The shim is **always** installed (even when a native env.exe exists
///      on PATH) because Claude Code's shell-quote library produces POSIX
///      escapes (`\@`, `\:`) that native env.exe does not strip, causing
///      agent ID mismatches and spawn failures (psmux#172, #173, #180).
///      Users who need the raw env.exe can invoke it as `env.exe` explicitly.
pub(crate) const ENV_SHIM_PS: &str = concat!(
    "function Global:env { ",
    // _pu: POSIX-unescape helper — strips `\` before non-word, non-backslash
    // chars (e.g. \@ → @, \: → :) produced by npm shell-quote.
    // SKIPS Windows absolute paths (C:\...) where `\` is a directory
    // separator, not a POSIX escape.  On Linux paths use `/` so
    // there's never a collision; on Windows `\@` in a path like
    // `node_modules\@anthropic-ai` must be preserved.
    "function _pu($s){if($s -match '^[A-Za-z]:\\\\'){return $s}; $s -replace '\\\\([^\\w\\\\])','$1'}; ",
    // _shebang: reads the first line of a script file and extracts the
    // interpreter, mimicking Linux kernel shebang execution.
    // Handles #!/usr/bin/env node, #!/usr/bin/node, #!/usr/bin/env deno, etc.
    "function _shebang($f){ ",
    "try{ $l=(Get-Content $f -TotalCount 1 -EA Stop); ",
    "if($l -match '^#!\\s*(.+)$'){ ",
    "$p=$Matches[1].Trim(); ",
    "if($p -match '/env\\s+(.+)$'){return ($Matches[1].Trim()-split'\\s+')[0]}; ",
    "return ($p-split'/')[-1] } }catch{}; $null }; ",
    "$v=@{}; $i=0; ",
    "while($i -lt $args.Count){ ",
    "if([string]$args[$i] -match '^([A-Za-z_]\\w*)=(.*)$'){ ",
    "$v[$Matches[1]]=(_pu $Matches[2]); $i++ ",
    "} else { break } }; ",
    "if($i -lt $args.Count){ ",
    "foreach($e in $v.GetEnumerator()){[Environment]::SetEnvironmentVariable($e.Key,$e.Value,'Process')}; ",
    "$cmd=(_pu ([string]$args[$i])); $rest=@(); ",
    "if($i+1 -lt $args.Count){$rest=@($args[($i+1)..($args.Count-1)]|ForEach-Object{_pu ([string]$_)})}; ",
    // For script files (.js/.mjs/.ts/.sh/.py/etc), read the shebang line
    // to determine the interpreter — exactly like Linux kernel does.
    // Falls back to node for .js/.mjs only if no shebang is found
    // (since Windows associates .js with WScript.exe, not node).
    "$interp=$null; ",
    "$resolved=$cmd; if($cmd -match '^''(.+)''$'){$resolved=$Matches[1]}; ",
    "if(Test-Path $resolved -EA 0){$interp=(_shebang $resolved)}; ",
    "if($interp){& $interp $cmd @rest} ",
    "elseif($cmd -match '\\.m?js$'){& node $cmd @rest} ",
    "else{& $cmd @rest} ",
    "} elseif($v.Count -gt 0){ ",
    "foreach($e in $v.GetEnumerator()){[Environment]::SetEnvironmentVariable($e.Key,$e.Value,'Process')} ",
    "} else { Get-ChildItem Env:|ForEach-Object{$_.Name+'='+$_.Value} } }; ",
    // Claude Code teammate-mode wrapper (claude-code#26244):
    // The standalone (Bun SFE) binary ignores `teammateMode` from settings.json
    // but honours the `--teammate-mode tmux` CLI flag.  The agent teams tool-set
    // is separately gated by CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS env var (set
    // above in set_tmux_env).  This wrapper auto-injects --teammate-mode when
    // PSMUX_CLAUDE_TEAMMATE_MODE is set (via `set -g claude-code-fix-tty on`).
    // Disable with: set -g claude-code-fix-tty off
    "if($env:PSMUX_CLAUDE_TEAMMATE_MODE){ ",
    "function Global:claude { ",
    "if($args -contains '--teammate-mode'){ & claude.exe @args } ",
    "else{ & claude.exe --teammate-mode $env:PSMUX_CLAUDE_TEAMMATE_MODE @args } } }",
);

/// PSReadLine prediction fix — disables predictions that crash with
/// NullReferenceException in GetHistoryItems() during ConPTY startup.
/// See https://github.com/psmux/psmux/issues/109
pub(crate) const PSRL_FIX: &str = concat!(
    "try { Set-PSReadLineOption -PredictionSource None -ErrorAction Stop } catch {}; ",
    "try { Set-PSReadLineOption -PredictionViewStyle InlineView -ErrorAction Stop } catch {}; ",
    "try { Remove-PSReadLineKeyHandler -Chord 'F2' -ErrorAction Stop } catch {}",
);

/// Minimal crash guard: saves the user's original PredictionSource, then
/// disables predictions to prevent the #109 NullReferenceException during
/// ConPTY startup.  Does NOT touch PredictionViewStyle or F2 so those stay
/// at whatever the system default is.  Used pre-profile when allow-predictions
/// is on (#150).
pub(crate) const PSRL_CRASH_GUARD: &str = concat!(
    "$Global:__psmux_origPred = try { (Get-PSReadLineOption).PredictionSource } catch { 'History' }; ",
    "try { Set-PSReadLineOption -PredictionSource None -ErrorAction Stop } catch {}",
);

/// Post-profile prediction restore: if PredictionSource is still None (meaning
/// the user's profile did not explicitly set it), restore the saved original.
/// If the profile DID set a value, we leave it alone.
/// Used post-profile when allow-predictions is on (#150).
pub(crate) const PSRL_PRED_RESTORE: &str = concat!(
    "if ((Get-PSReadLineOption).PredictionSource -eq 'None' -and $Global:__psmux_origPred -ne 'None') { ",
    "try { Set-PSReadLineOption -PredictionSource $Global:__psmux_origPred -ErrorAction Stop } catch {} ",
    "}",
);

/// Source all four PowerShell profile scripts in the standard order.
/// Used with -NoProfile to give us control over execution order — we disable
/// PSReadLine predictions BEFORE the profile loads (preventing the
/// GetHistoryItems NullReferenceException), then re-disable after the profile
/// in case the user's profile re-enables predictions.
pub(crate) const PROFILE_SOURCE: &str = concat!(
    "foreach ($__p in @(",
    "$PROFILE.AllUsersAllHosts,",
    "$PROFILE.AllUsersCurrentHost,",
    "$PROFILE.CurrentUserAllHosts,",
    "$PROFILE.CurrentUserCurrentHost",
    ")) { if ($__p -and (Test-Path $__p)) { try { . $__p } catch { Write-Warning \"psmux: profile error in ${__p}: $_\" } } }",
);
