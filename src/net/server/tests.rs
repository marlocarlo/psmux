#[allow(unused_imports)]

use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::env;
use std::net::TcpListener;

use portable_pty::native_pty_system;
use ratatui::prelude::Rect;

use crate::types::{AppState, CtrlReq, Mode, FocusDir, LayoutKind, PipePaneState, VERSION,
    WaitChannel, WaitForOp, Node, Action, Bind};
use crate::platform::install_console_ctrl_handler;
use crate::pane::{create_window, create_window_raw, split_active_with_command, kill_active_pane, kill_pane_by_id, spawn_warm_pane};
use crate::tree::{self, active_pane, active_pane_mut, resize_all_panes, kill_all_children,
    find_window_index_by_id, focus_pane_by_id, focus_pane_by_id_no_mru, focus_pane_by_index, get_active_pane_id,
    get_split_mut, path_exists};

use super::helpers::{collect_pane_paths_server, serialize_bindings_json, json_escape_string,
    list_windows_json_with_tabs, combined_data_version, TMUX_COMMANDS};
use super::options::{get_option_value, apply_set_option};
use super::window_options::{get_window_option_value, render_window_options};

use crate::input::{send_text_to_active, send_key_to_active, send_paste_to_active, move_focus, find_best_pane_in_direction, find_wrap_target};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, move_copy_cursor, current_prompt_pos,
    yank_selection, scroll_copy_up, scroll_copy_down, switch_with_copy_save,
    capture_active_pane_text, capture_active_pane_range, capture_active_pane_styled};
use crate::layout::{dump_layout_json, dump_layout_json_fast, apply_layout, cycle_layout,
    cycle_layout_reverse};
use crate::window_ops::{toggle_zoom, remote_mouse_down, remote_mouse_drag, remote_mouse_up,
    remote_mouse_button, remote_mouse_motion, remote_scroll_up, remote_scroll_down,
    swap_pane, break_pane_to_window, unzoom_if_zoomed, resize_pane_vertical,
    resize_pane_horizontal, resize_pane_absolute, rotate_panes, respawn_active_pane,
    handle_pane_mouse, handle_pane_scroll, handle_split_set_sizes, handle_split_resize_done};
use crate::config::{load_config, parse_key_string, format_key_binding, normalize_key_for_binding,
    parse_config_content};
use crate::commands::{parse_command_to_action, format_action, parse_menu_definition, execute_command_string};
use crate::util::{list_windows_json, list_tree_json, list_windows_tmux, base64_encode};
use crate::control;
use crate::format::{expand_format, format_list_windows, format_list_panes, set_buffer_idx_override};
use crate::help;

/// Build a JSON fragment with overlay state (popup, menu, confirm, display_panes).
/// Delegates popup-specific serialization to the popup module.
use super::*;

#[cfg(test)]
#[path = "../../../tests-rs/test_server.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue169_manual_rename.rs"]
mod test_issue169;

#[cfg(test)]
#[path = "../../../tests-rs/test_pane_title.rs"]
mod test_pane_title;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue202_switch_client.rs"]
mod test_issue202;

#[cfg(test)]
#[path = "../../../tests-rs/test_new_session_env.rs"]
mod test_new_session_env;
