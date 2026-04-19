#[allow(unused_imports)]

use std::io;
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};

/// Explicitly (re-)send the VT mouse-enable escape sequences to stdout.
///
/// Over SSH, ConPTY may consume DECSET 1000/1002/1003/1006 from the output
/// stream and NOT forward them to sshd.  This tries several approaches:
///  1. `WriteFile` on the raw console output handle (may bypass ConPTY VT
///     processing in some Windows builds).
///  2. A regular `write_all` to stdout (belt-and-suspenders).
///
/// Call this **after** crossterm's `EnableMouseCapture` and `InputSource::new`.
#[cfg(windows)]
use super::*;

#[cfg(test)]
#[path = "../../../tests-rs/test_ssh_vt_paste.rs"]
mod tests;
