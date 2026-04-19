use super::*;

/// Mutable loop-local state passed to handler functions so they can modify
/// dirty flags, caches, and other per-iteration bookkeeping without needing
/// closure captures or global state.
pub(crate) struct LoopCtx {
    pub pty_system: Box<dyn portable_pty::PtySystem + Send>,
    pub state_dirty: bool,
    pub meta_dirty: bool,
    pub echo_pending_until: Option<Instant>,
    pub temp_focus_restore: Option<(usize, usize)>,
    pub shared_aliases: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
    // ── DumpState / push caches ──
    pub cached_dump_state: String,
    pub cached_data_version: u64,
    pub cached_windows_json: String,
    pub cached_tree_json: String,
    pub cached_prefix_str: String,
    pub cached_prefix2_str: String,
    pub cached_base_index: usize,
    pub cached_pred_dim: bool,
    pub cached_status_style: String,
    pub cached_bindings_json: String,
    pub combined_buf: String,
}

impl LoopCtx {
    pub fn new(
        pty_system: Box<dyn portable_pty::PtySystem + Send>,
        shared_aliases: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
    ) -> Self {
        Self {
            pty_system,
            state_dirty: true,
            meta_dirty: true,
            echo_pending_until: None,
            temp_focus_restore: None,
            shared_aliases,
            cached_dump_state: String::new(),
            cached_data_version: 0,
            cached_windows_json: String::new(),
            cached_tree_json: String::new(),
            cached_prefix_str: String::new(),
            cached_prefix2_str: String::new(),
            cached_base_index: 0,
            cached_pred_dim: false,
            cached_status_style: String::new(),
            cached_bindings_json: String::from("[]"),
            combined_buf: String::with_capacity(32768),
        }
    }

    /// Rebuild metadata caches (windows JSON, tree, prefix, bindings).
    pub fn rebuild_meta_cache(&mut self, app: &AppState) -> io::Result<()> {
        self.cached_windows_json = list_windows_json_with_tabs(app)?;
        self.cached_tree_json = list_tree_json(app)?;
        self.cached_prefix_str = format_key_binding(&app.prefix_key);
        self.cached_prefix2_str = app.prefix2_key.as_ref().map(|k| format_key_binding(k)).unwrap_or_default();
        self.cached_base_index = app.window_base_index;
        self.cached_pred_dim = app.prediction_dimming;
        self.cached_status_style = app.status_style.clone();
        self.cached_bindings_json = serialize_bindings_json(app);
        self.meta_dirty = false;
        Ok(())
    }
}
