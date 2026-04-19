#[allow(unused_imports)]
use std::io;
use std::time::Instant;
#[cfg(windows)]
use std::path::PathBuf;

use std::io::Write;
use crate::types::{AppState, Mode, Action, FocusDir, LayoutKind, MenuItem, Menu, Node};
use crate::tree::{compute_rects, kill_all_children, get_active_pane_id};
use crate::pane::{create_window, split_active, kill_active_pane};
use crate::copy_mode::{enter_copy_mode, switch_with_copy_save, paste_latest,
    capture_active_pane, save_latest_buffer};
use crate::session::{send_control_to_port, list_all_sessions_tree};
use crate::window_ops::toggle_zoom;

/// Parse a popup dimension spec: "80" (absolute) or "95%" (percentage of term_dim).
use super::*;

#[cfg(test)]
#[path = "../../../tests-rs/test_commands.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests-rs/test_commands_new.rs"]
mod tests_new_commands;

#[cfg(test)]
#[path = "../../../tests-rs/test_commands_audit.rs"]
mod tests_commands_audit;

#[cfg(test)]
#[path = "../../../tests-rs/test_parity.rs"]
mod tests_parity;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue179_bind_key_uppercase.rs"]
mod tests_issue179_bind_key_uppercase;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue192_command_chaining.rs"]
mod tests_issue192_command_chaining;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue200_new_session.rs"]
mod tests_issue200_new_session;

#[cfg(test)]
#[path = "../../../tests-rs/test_run_shell_resolve.rs"]
mod tests_run_shell_resolve;

#[cfg(test)]
#[path = "../../../tests-rs/test_hide_window.rs"]
mod tests_hide_window;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue209_tmux_compat.rs"]
mod tests_issue209_tmux_compat;

#[cfg(test)]
#[path = "../../../tests-rs/test_gastown_scenarios.rs"]
mod tests_gastown_scenarios;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue210_gastown_fixes.rs"]
mod tests_issue210_gastown_fixes;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue210_gastown_captures.rs"]
mod tests_issue210_gastown_captures;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue215_session_persistence.rs"]
mod tests_issue215_session_persistence;

#[cfg(test)]
#[path = "../../../tests-rs/test_mega_unit_coverage.rs"]
mod tests_mega_unit_coverage;

#[cfg(test)]
#[path = "../../../tests-rs/test_flag_parity.rs"]
mod tests_flag_parity;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue227_remain_on_exit_hooks.rs"]
mod tests_issue227_remain_on_exit_hooks;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue235_display_panes_base_index.rs"]
mod tests_issue235_display_panes_base_index;
