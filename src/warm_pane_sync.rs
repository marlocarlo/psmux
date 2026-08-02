//! Centralised lifecycle for the warm pane.
//!
//! The warm pane is a snapshot of server state at spawn time: shell
//! binary, environment, terminal dimensions, vt100 scrollback cap.
//! When any of those change, the snapshot becomes stale.  Without a
//! single owner of "what does the warm pane need now?", each
//! invalidation site grew its own ad-hoc kill+respawn — which led to
//! gaps:
//!
//!   * `set-option default-shell` killed the warm pane but never
//!     respawned (only handled in one of two SetOption paths).
//!   * `set-option allow-predictions` was reconciled at boot only,
//!     never at runtime.
//!   * `set-option default-terminal` updated `app.environment["TERM"]`
//!     but the warm pane kept the old TERM forever.
//!   * `set-option history-limit` was not propagated at all (#271).
//!
//! This module is the only place that decides what to do, and the
//! only place that mutates `app.warm_pane`.
//!
//! Three response kinds, in increasing cost:
//!
//!   `Noop`             — change does not affect the warm pane.
//!   `Patch(...)`       — mutate the running pane in place (cheap,
//!                         keeps the shell warm).  Used for state
//!                         that only lives in the vt100 parser.
//!   `Respawn(reason)`  — kill the child shell and pre-spawn a new
//!                         one with current `AppState`.  Required for
//!                         anything that affects the child process
//!                         (env vars, shell binary, predictions).
//!
//! `apply` honours `app.warm_enabled`: if warm panes are disabled,
//! `Respawn` degrades to a kill with no respawn.

use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::types::{AppState, WarmPane};

/// Decision returned by the `for_*` helpers. Apply it with [`apply`] during
/// startup or [`apply_runtime`] once the server loop is running.
#[derive(Debug)]
pub enum WarmPaneSync {
    Noop,
    Patch(WarmPanePatch),
    Respawn(&'static str),
}

/// In-place mutations safe to perform on a running warm pane.
#[derive(Clone, Debug)]
pub enum WarmPanePatch {
    /// Resize the vt100 parser's scrollback cap.  Trims oldest rows
    /// if shrinking.  See `vt100::Screen::set_scrollback_len`.
    HistoryLimit(usize),
    /// Toggle whether DEC 47/1049 alt-screen mode switches are honoured.
    /// Off → TUI app output lands in main scrollback (#88).  Cheap to
    /// apply: a single field flip on the parser, no shell restart.
    AllowAlternateScreen(bool),
}

/// Decide what the warm pane needs given that a server option changed.
/// The caller has already mutated `app` so this reads the new value
/// straight off `AppState`, not from the raw `value` string.
///
/// Adding a new option that affects the warm pane?  Add it here.
/// Forgetting to do so leaves the warm pane stale until the next
/// kill-everything event (server restart, env-var change, resize),
/// which is exactly the class of bug this module exists to prevent.
pub fn for_option_change(name: &str, app: &AppState) -> WarmPaneSync {
    match name {
        // Parser-only: patch in place, no shell restart needed.
        // Kept cheap because users may set this in the prompt and we
        // do not want to throw away ~470ms of shell init for it.
        "history-limit" => WarmPaneSync::Patch(WarmPanePatch::HistoryLimit(app.history_limit)),

        // Parser-only flag — cheap to flip on a running pane.
        // Drives whether TUI apps render to alt grid (default) or
        // straight to main grid + scrollback (#88).
        "alternate-screen" => {
            WarmPaneSync::Patch(WarmPanePatch::AllowAlternateScreen(app.allow_alternate_screen))
        }

        // The shell binary itself differs — must respawn.
        "default-shell" => WarmPaneSync::Respawn("default-shell changed"),

        // These options change how the child is constructed, or whether a
        // spare should exist at all.
        "env-shim" => WarmPaneSync::Respawn("env-shim changed"),
        "warm" => WarmPaneSync::Respawn("warm setting changed"),

        // We send a different PSReadLine init script depending on this
        // option (PSRL_FIX vs PSRL_CRASH_GUARD).  The script runs once
        // at shell startup, so a running shell is stuck with whichever
        // it got — only a fresh spawn picks up the new value (#165).
        "allow-predictions" => WarmPaneSync::Respawn("allow-predictions changed"),

        // default-terminal feeds `TERM` into the child env at spawn
        // time.  An already-running child has the old TERM baked in.
        "default-terminal" => WarmPaneSync::Respawn("default-terminal changed"),

        // Claude Code TTY-shim flags are read by `set_tmux_env` at
        // spawn time — running children miss the change.
        "claude-code-fix-tty" | "claude-code-force-interactive" => {
            WarmPaneSync::Respawn("claude-code option changed")
        }

        // Everything else either does not affect the warm pane (status
        // styles, key tables, hooks, etc.) or is read live (mouse,
        // status-visible) so no warm-pane action is required.
        _ => WarmPaneSync::Noop,
    }
}

/// `set-environment` / `unset-environment` always require a respawn:
/// you cannot mutate a running process's environment block from
/// outside (kernel-level constraint), so the child must be re-execed.
/// Already-handled in the codebase prior to this module (#137); now
/// consolidated through one entry point.
pub fn for_env_change() -> WarmPaneSync {
    WarmPaneSync::Respawn("environment changed")
}

/// When the client terminal resizes, the warm pane's parser grid is
/// at the old dimensions.  Respawn at the new size so the next
/// transplant lands pixel-perfect on the first frame with no reflow.
pub fn for_resize(app: &AppState, new_rows: u16, new_cols: u16) -> WarmPaneSync {
    match app.warm_pane.as_ref() {
        Some(wp) if wp.rows == new_rows && wp.cols == new_cols => WarmPaneSync::Noop,
        Some(_) => WarmPaneSync::Respawn("client resized"),
        None if app.client_area.height == new_rows && app.client_area.width == new_cols => {
            WarmPaneSync::Noop
        }
        None => WarmPaneSync::Respawn("client resized"),
    }
}

pub fn for_config_reload() -> WarmPaneSync {
    WarmPaneSync::Respawn("configuration reloaded")
}

/// After the user's config has been parsed at server boot, the
/// early-warm pane (born with all defaults) needs to be reconciled
/// with whatever the config actually set.  Returns the cheapest
/// action that gets the warm pane to a state consistent with `app`.
///
/// Order matters: respawn-class triggers are checked first because a
/// respawn implicitly applies all patch-class state to the new pane.
pub fn for_post_config(app: &AppState) -> WarmPaneSync {
    // Config disabled warm panes — `apply` handles this by killing
    // without respawning when `warm_enabled` is false.
    if !app.warm_enabled {
        return WarmPaneSync::Respawn("warm panes disabled by config");
    }

    // Custom default-shell set: the early warm pane has the wrong
    // shell binary.  Respawn to get the right one.  Doing this at
    // post-config time (rather than killing-and-deferring) keeps
    // create_window's fast path warm.
    if !app.default_shell.is_empty() {
        return WarmPaneSync::Respawn("post-config: custom default-shell");
    }

    // Config injected env vars (e.g. via set -g default-terminal,
    // set-environment in the config, or update-environment passing
    // through client env): the early child has them missing.
    let needs_env = app.environment.iter().any(|(k, _)| {
        !k.starts_with("PSMUX_TARGET_SESSION") && k != "TMUX" && k != "TMUX_PANE"
    });
    if needs_env {
        return WarmPaneSync::Respawn("post-config: env vars set");
    }

    // Config flipped allow-predictions on — the early pwsh got the
    // wrong PSReadLine init.
    if app.allow_predictions {
        return WarmPaneSync::Respawn("post-config: predictions enabled");
    }

    // history-limit only differs in the parser cap, no respawn.
    // alternate-screen only differs in a parser flag, also no respawn.
    // If both differ from defaults, the consume-time helper will
    // reconcile both via `reconcile_consumed_parser`; here we only
    // need to tell the policy module that *something* parser-level
    // wants patching.  We bias to history-limit because it is the
    // more common config knob in real-world setups.
    if app.history_limit != 2000 {
        return WarmPaneSync::Patch(WarmPanePatch::HistoryLimit(app.history_limit));
    }
    if !app.allow_alternate_screen {
        return WarmPaneSync::Patch(WarmPanePatch::AllowAlternateScreen(false));
    }

    WarmPaneSync::Noop
}

/// Synchronous startup reconciliation. Runtime call sites must use
/// [`apply_runtime`] so ConPTY creation never blocks the server loop.
pub fn apply(
    app: &mut AppState,
    pty_system: &dyn portable_pty::PtySystem,
    sync: WarmPaneSync,
) {
    match sync {
        WarmPaneSync::Noop => {}
        WarmPaneSync::Patch(patch) => apply_patch(app, patch),
        WarmPaneSync::Respawn(_reason) => respawn(app, pty_system),
    }
}

struct SpawnCompletion {
    generation: u64,
    result: io::Result<WarmPane>,
}

/// Owns the one permitted runtime warm-pane worker. A generation number
/// rejects results built from stale size/config snapshots, while backoff
/// prevents a broken PTY environment from spawning in a tight loop.
pub struct WarmPaneSpawner {
    generation: u64,
    in_flight: bool,
    completion_tx: mpsc::Sender<SpawnCompletion>,
    completion_rx: mpsc::Receiver<SpawnCompletion>,
    consecutive_failures: u32,
    retry_not_before: Option<Instant>,
}

impl WarmPaneSpawner {
    pub fn new() -> Self {
        let (completion_tx, completion_rx) = mpsc::channel();
        Self {
            generation: 0,
            in_flight: false,
            completion_tx,
            completion_rx,
            consecutive_failures: 0,
            retry_not_before: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.retry_not_before = None;
    }

    pub fn poll(&mut self, app: &mut AppState) {
        while let Ok(mut completion) = self.completion_rx.try_recv() {
            self.in_flight = false;
            let current = completion.generation == self.generation
                && app.warm_enabled
                && app.warm_pane.is_none();
            if !current {
                if let Ok(ref mut pane) = completion.result {
                    pane.child.kill().ok();
                }
                continue;
            }
            match completion.result {
                Ok(pane) => {
                    app.warm_pane = Some(pane);
                    self.consecutive_failures = 0;
                    self.retry_not_before = None;
                }
                Err(_) => self.record_failure(),
            }
        }
    }

    pub fn ensure(&mut self, app: &mut AppState) {
        if !app.warm_enabled || app.warm_pane.is_some() || self.in_flight {
            return;
        }
        if self.retry_not_before.is_some_and(|deadline| Instant::now() < deadline) {
            return;
        }
        let spec = match crate::pane::prepare_warm_pane_spawn(app) {
            Ok(spec) => spec,
            Err(_) => {
                self.record_failure();
                return;
            }
        };
        let generation = self.generation;
        let completion_tx = self.completion_tx.clone();
        self.in_flight = true;
        std::thread::spawn(move || {
            #[cfg(debug_assertions)]
            if let Ok(delay) = std::env::var("PSMUX_TEST_WARM_PANE_DELAY_MS") {
                if let Ok(delay) = delay.parse::<u64>() {
                    if let Ok(path) =
                        std::env::var("PSMUX_TEST_WARM_PANE_DELAY_STARTED_FILE")
                    {
                        std::fs::write(path, b"started").ok();
                    }
                    std::thread::sleep(Duration::from_millis(delay));
                }
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let pty_system = portable_pty::native_pty_system();
                crate::pane::spawn_warm_pane_from_spec(&*pty_system, spec)
            }))
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    "warm pane worker panicked",
                ))
            });
            let completion = SpawnCompletion { generation, result };
            if let Err(mpsc::SendError(mut completion)) = completion_tx.send(completion) {
                if let Ok(ref mut pane) = completion.result {
                    pane.child.kill().ok();
                }
            }
        });
    }

    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let delay = retry_delay(self.consecutive_failures);
        self.retry_not_before = Some(Instant::now() + delay);
    }
}

impl Default for WarmPaneSpawner {
    fn default() -> Self {
        Self::new()
    }
}

fn retry_delay(consecutive_failures: u32) -> Duration {
    match consecutive_failures {
        0 | 1 => Duration::from_millis(100),
        2 => Duration::from_millis(500),
        3 => Duration::from_secs(2),
        _ => Duration::from_secs(5),
    }
}

/// Apply runtime policy without performing PTY creation on the caller.
pub fn apply_runtime(
    app: &mut AppState,
    spawner: &mut WarmPaneSpawner,
    sync: WarmPaneSync,
) {
    match sync {
        WarmPaneSync::Noop => {}
        WarmPaneSync::Patch(patch) => {
            apply_patch(app, patch);
            if app.warm_pane.is_none() {
                spawner.invalidate();
                spawner.ensure(app);
            }
        }
        WarmPaneSync::Respawn(_reason) => {
            if let Some(mut old) = app.warm_pane.take() {
                old.child.kill().ok();
            }
            spawner.invalidate();
            spawner.ensure(app);
        }
    }
}

fn apply_patch(app: &mut AppState, patch: WarmPanePatch) {
    // Both patch kinds also need to be applied to *every existing
    // pane*, not just the warm pane — otherwise the user's
    // `set -g alternate-screen off` would only affect the next pane
    // they open, surprising anyone who issued the change with a TUI
    // already running.  These are O(panes × O(1)) parser flag flips,
    // bounded and cheap.
    apply_patch_to_existing_panes(app, &patch);

    let wp = match app.warm_pane.as_ref() {
        Some(wp) => wp,
        None => return,
    };
    match patch {
        WarmPanePatch::HistoryLimit(n) => {
            if let Ok(mut parser) = wp.term.lock() {
                if parser.screen().scrollback_len() != n {
                    parser.screen_mut().set_scrollback_len(n);
                }
            }
        }
        WarmPanePatch::AllowAlternateScreen(allowed) => {
            if let Ok(mut parser) = wp.term.lock() {
                if parser.screen().allow_alternate_screen() != allowed {
                    parser.screen_mut().set_allow_alternate_screen(allowed);
                }
            }
        }
    }
}

/// Walk every live pane and apply the patch.  Critical for options
/// that change perceived behaviour from the user's point of view —
/// `alternate-screen off` would be useless if it only affected
/// future panes.  history-limit propagation matches tmux semantics:
/// existing buffers grow / shrink to the new cap.
fn apply_patch_to_existing_panes(app: &mut AppState, patch: &WarmPanePatch) {
    use crate::types::Node;
    fn walk(node: &mut Node, patch: &WarmPanePatch) {
        match node {
            Node::Leaf(p) => {
                if let Ok(mut parser) = p.term.lock() {
                    match patch {
                        WarmPanePatch::HistoryLimit(n) => {
                            if parser.screen().scrollback_len() != *n {
                                parser.screen_mut().set_scrollback_len(*n);
                            }
                        }
                        WarmPanePatch::AllowAlternateScreen(allowed) => {
                            if parser.screen().allow_alternate_screen() != *allowed {
                                parser.screen_mut().set_allow_alternate_screen(*allowed);
                            }
                        }
                    }
                }
            }
            Node::Split { children, .. } => {
                for child in children.iter_mut() {
                    walk(child, patch);
                }
            }
        }
    }
    for win in app.windows.iter_mut() {
        walk(&mut win.root, patch);
    }
}

fn respawn(app: &mut AppState, pty_system: &dyn portable_pty::PtySystem) {
    // Always kill any existing warm pane first — there is no in-place
    // way to swap shell binaries or environment blocks.
    if let Some(mut old) = app.warm_pane.take() {
        old.child.kill().ok();
    }
    // Honour warm_enabled: a config-disabled warm pane must not come
    // back to life after a Respawn — the user opted out.
    if !app.warm_enabled {
        return;
    }
    match crate::pane::spawn_warm_pane(pty_system, app) {
        Ok(wp) => {
            app.warm_pane = Some(wp);
        }
        Err(_) => {
            // Best-effort: if a respawn fails (e.g. transient PTY
            // creation error) we leave warm_pane = None and the next
            // consume path falls back to a synchronous cold spawn.
        }
    }
}

/// Helper for warm-pane consume sites in `pane.rs`.  When a warm
/// pane is transplanted into a real session, its parser may still
/// hold stale flags (history-limit raised, alt-screen toggled) after
/// the pane was born.  This is the safety net that guarantees
/// consume-time consistency even if a future caller forgets to
/// invoke `apply` on a state change.
pub fn reconcile_consumed_parser(parser: &mut vt100::Parser, app: &AppState) {
    let screen = parser.screen();
    let need_history = screen.scrollback_len() != app.history_limit;
    let need_alt = screen.allow_alternate_screen() != app.allow_alternate_screen;
    if need_history || need_alt {
        let s = parser.screen_mut();
        if need_history {
            s.set_scrollback_len(app.history_limit);
        }
        if need_alt {
            s.set_allow_alternate_screen(app.allow_alternate_screen);
        }
    }
}

#[cfg(test)]
#[path = "../tests-rs/test_warm_pane_sync.rs"]
mod test_warm_pane_sync;
