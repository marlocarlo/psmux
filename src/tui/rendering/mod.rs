//! TUI rendering — pane tree rendering, separator drawing, cursor positioning.
//!
//! Style/color parsing is in `style.rs`; this module re-exports it for
//! backward compatibility so `use crate::rendering::*` still works.

pub use crate::style::{
    map_color, parse_tmux_style, parse_inline_styles,
};
pub(crate) use std::io::{self, Write};
pub(crate) use std::env;
pub(crate) use ratatui::prelude::*;
pub(crate) use ratatui::widgets::*;
pub(crate) use ratatui::style::{Style, Modifier};
pub(crate) use unicode_width::UnicodeWidthStr;
pub(crate) use crossterm::style::Print;
pub(crate) use crossterm::execute;
pub(crate) use portable_pty::PtySize;
pub(crate) use crate::types::{AppState, Mode, Node, LayoutKind};
pub(crate) use crate::tree::split_with_gaps;

pub(crate) mod vt_to_color;
pub(crate) mod compute_active_rect_pub;
pub(crate) mod fix_border_intersections;

pub use vt_to_color::*;
pub use compute_active_rect_pub::*;
pub use fix_border_intersections::*;
