//! SSH VT Input — transparent mouse + keyboard support over SSH on Windows.
//!
//! ## Problem
//!
//! ConPTY does **not** translate VT mouse escape sequences (SGR `\x1b[<…M`,
//! X10 `\x1b[M…`) into native `MOUSE_EVENT` `INPUT_RECORD`s.  When psmux
//! runs over SSH, the remote terminal sends SGR mouse bytes through:
//!
//! ```text
//!   remote terminal → SSH client → sshd → ConPTY input pipe
//!     → ConPTY does NOT convert to MOUSE_EVENT
//!       → crossterm's ReadConsoleInputW never sees mouse events
//! ```
//!
//! ## Solution
//!
//! When an SSH session is detected, this module:
//!
//! 1. Configures the console stdin for raw input (no echo, no line edit,
//!    no Quick Edit) with `ENABLE_MOUSE_INPUT` and
//!    `ENABLE_VIRTUAL_TERMINAL_INPUT` (VTI).  VTI is **critical** — without
//!    it, ConPTY's input parser intercepts CSI sequences from the SSH data
//!    stream (including SGR mouse `\x1b[<…M`) and discards those it doesn't
//!    recognise.  With VTI, ConPTY passes raw bytes through as `KEY_EVENT`
//!    records with `u_char` set, which our VT parser reassembles.
//! 2. Spawns a dedicated reader thread that calls `ReadConsoleInputW` in a
//!    tight loop.
//! 3. Handles **two kinds** of `KEY_EVENT` records:
//!    - `u_char != 0` — character data (ConPTY passed unrecognised VT bytes
//!      through as individual characters).  Fed into a fast VT state-machine
//!      parser that decodes SGR/X10 mouse, CSI keyboard, SS3 function keys,
//!      bracketed paste, Alt+key, and plain characters.
//!    - `u_char == 0` — virtual-key events (ConPTY recognised the VT
//!      sequence and translated it, e.g. VK_UP for `\x1b[A`).  Mapped
//!      directly to `crossterm::event::Event` via VK-code lookup.
//! 4. Delivers events through a bounded `mpsc::sync_channel` — the client
//!    event loop reads via [`InputSource::read_timeout`] /
//!    [`InputSource::try_read`].
//!
//! Resize events (`WINDOW_BUFFER_SIZE_EVENT`) and native `MOUSE_EVENT`
//! records are forwarded directly.
//!
//! On non-Windows platforms (or when not under SSH), [`InputSource`] simply
//! delegates to `crossterm::event`.
//!
//! ## Debugging
//!
//! Set `PSMUX_SSH_DEBUG=1` to write a detailed trace of every INPUT_RECORD
//! and emitted event to `~/.psmux/ssh_input.log`.

pub(crate) use std::io;
pub(crate) use std::time::Duration;
pub(crate) use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};

pub(crate) mod send_mouse_enable;
pub(crate) mod impl_vtparser;
pub(crate) mod impl_vtparser_dispatch;
pub(crate) mod vk_to_keycode;
pub(crate) mod start_ssh_reader;
pub(crate) mod ssh_reader_helpers;
pub(crate) mod tests;

pub use send_mouse_enable::*;
pub use impl_vtparser::*;
pub use impl_vtparser_dispatch::*;
pub use vk_to_keycode::*;
pub use start_ssh_reader::*;
pub use ssh_reader_helpers::*;
pub use tests::*;
