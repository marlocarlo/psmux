//! Popup overlay module.
//!
//! A popup is a **Pane rendered as a floating overlay**, not part of the
//! window tree.  By storing an actual `Pane` inside `PopupMode`, the popup
//! inherits all pane infrastructure: vt100 parsing, PTY I/O, run-length
//! encoded screen serialization, color rendering, and (in the future)
//! copy-mode, scrollback, etc.
//!
//! This module centralises popup-specific logic:
//!  - PTY-backed pane creation  (`create_popup_pane`)
//!  - Server-side JSON serialization (`serialize_popup_overlay`)
//!  - In-process TUI rendering   (`render_popup_overlay`)

pub(crate) use std::sync::{Arc, Mutex};
pub(crate) use crate::layout::serialize_screen_rows;
pub(crate) use crate::types::{Pane, AppState, Mode};

pub(crate) mod create_popup_pane;
pub(crate) mod render_popup_overlay;

pub use create_popup_pane::*;
pub use render_popup_overlay::*;
