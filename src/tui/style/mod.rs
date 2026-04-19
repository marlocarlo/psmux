//! Shared color and style parsing utilities.
//!
//! This module consolidates ALL tmux-compatible color/style parsing into a
//! single place, eliminating duplication between rendering.rs and client.rs.
//! Both the server-side renderer and the remote client import from here.

pub(crate) use ratatui::prelude::*;
pub(crate) use ratatui::style::{Style, Modifier};
pub(crate) use crate::debug_log::style_log;

pub(crate) mod map_color;
pub(crate) mod parse_format_segments;
pub(crate) mod extract_span_range;
pub(crate) mod tests;

pub use map_color::*;
pub use parse_format_segments::*;
pub use extract_span_range::*;
pub use tests::*;
