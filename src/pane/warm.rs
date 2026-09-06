//! A single asynchronous warm-pane refill, with a configuration snapshot.
//! No worker holds AppState and no stale result is installed after settings,
//! identity, environment, or terminal dimensions have changed.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc;

use super::{AppState, MIN_PANE_DIM};
use crate::types::{HostColors, WarmPane};

#[derive(Clone, PartialEq)]
pub(super) struct WarmPaneConfig {
    pub rows: u16,
    pub cols: u16,
    pub shell: String,
    pub env_shim: bool,
    pub allow_predictions: bool,
    pub control_port: Option<u16>,
    pub socket_name: Option<String>,
    pub session_name: String,
    pub fix_tty: bool,
    pub force_interactive: bool,
    pub host_colors: Option<HostColors>,
    pub environment: HashMap<String, String>,
    pub history_limit: usize,
    pub allow_alternate_screen: bool,
}

impl WarmPaneConfig {
    pub(super) fn capture(app: &AppState) -> Self {
        let area = app.client_area;
        Self {
            rows: if area.height > 1 { area.height } else { 30 }.max(MIN_PANE_DIM),
            cols: if area.width > 1 { area.width } else { 120 }.max(MIN_PANE_DIM),
            shell: crate::format::expand_format(&app.default_shell, app),
            env_shim: app.env_shim,
            allow_predictions: app.allow_predictions,
            control_port: app.control_port,
            socket_name: app.socket_name.clone(),
            session_name: app.session_name.clone(),
            fix_tty: app.claude_code_fix_tty,
            force_interactive: app.claude_code_force_interactive,
            host_colors: app.host_colors.clone(),
            environment: app.environment.clone(),
            history_limit: app.history_limit,
            allow_alternate_screen: app.allow_alternate_screen,
        }
    }
}

struct OwnedWarmPane(Option<WarmPane>);
impl Drop for OwnedWarmPane {
    fn drop(&mut self) {
        if let Some(pane) = self.0.take() {
            // Receiver abandonment/stale results must kill the owned shell,
            // not leave an unreferenced prewarmed child process running.
            retire_warm_pane(pane);
        }
    }
}

/// Closing ConPTY can wait for its output drain, so retire outside the control
/// loop. If the OS refuses a worker thread, still kill the owned child rather
/// than dropping a process handle and silently leaving the child alive.
pub fn retire_warm_pane(pane: WarmPane) {
    let owned = std::sync::Arc::new(std::sync::Mutex::new(Some(pane)));
    let worker_owned = owned.clone();
    let spawned = std::thread::Builder::new().name("warm-pane-retire".into()).spawn(move || {
        if let Some(mut pane) = worker_owned.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = pane.child.kill();
            drop(pane);
        }
    });
    if spawned.is_err() {
        if let Some(mut pane) = owned.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = pane.child.kill();
            drop(pane);
        }
    }
}

pub struct WarmPaneTask {
    config: WarmPaneConfig,
    result: mpsc::Receiver<io::Result<OwnedWarmPane>>,
    invalidated: bool,
}

impl WarmPaneTask {
    pub fn start(app: &mut AppState) -> io::Result<Self> {
        if !app.warm_enabled {
            return Err(io::Error::other("warm panes disabled"));
        }
        let config = WarmPaneConfig::capture(app);
        let worker_config = config.clone();
        let pane_id = app.next_pane_id;
        app.next_pane_id = app.next_pane_id.checked_add(1)
            .ok_or_else(|| io::Error::other("pane ID space exhausted"))?;
        let (tx, result) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("warm-pane-spawn".into()).spawn(move || {
            let pty_system = portable_pty::native_pty_system();
            let spawned = super::spawn_warm_pane_config(&*pty_system, worker_config, pane_id)
                .map(|pane| OwnedWarmPane(Some(pane)));
            // Failed delivery drops the ownership guard and retires the shell.
            let _ = tx.send(spawned);
        })?;
        Ok(Self { config, result, invalidated: false })
    }

    pub fn invalidate(&mut self) { self.invalidated = true; }

    /// true means the task is terminal and its caller can clear the slot.
    pub fn poll(&mut self, app: &mut AppState) -> bool {
        match self.result.try_recv() {
            Ok(Ok(mut owned)) => {
                if !self.invalidated && app.warm_enabled && app.warm_pane.is_none()
                    && self.config == WarmPaneConfig::capture(app) {
                    app.warm_pane = owned.0.take();
                }
                true
            }
            Ok(Err(error)) => {
                crate::debug_log::server_log("warm", &format!("warm pane spawn failed: {}", error));
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => true,
        }
    }
}
