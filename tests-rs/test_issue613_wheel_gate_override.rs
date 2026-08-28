//! Issue #613: a pane's wheel goes permanently silent once any child enters
//! raw mode.
//!
//! The #598 gate forwards a wheel report only into a pane whose application
//! asked for the mouse, and it accepts two answers: a DECSET attributed to a
//! confirmed foreground, or `ENABLE_MOUSE_INPUT` on the child console right
//! now. The second is not psmux's to keep. Console input mode belongs to the
//! CONSOLE, not to the process that set it, and libuv's raw mode overwrites
//! the whole mode word with `ENABLE_WINDOW_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT`
//! without restoring it, so one `node` child anywhere in the pane's process
//! tree strips the bit for good. Because the console outlives the process,
//! restarting the pane's application does not earn it back; only a new window
//! does. Measured on Claude Code, which emits no DECSET either, so the pane
//! cannot satisfy the other answer at any point in its life.
//!
//! The gate itself is deliberately left alone: it is what stops htop reading a
//! raw report as keystrokes and filling its search prompt with the digits.
//! These tests pin the escape hatch's semantics, and in particular that it
//! stays independent of #573's `PSMUX_FORCE_MOUSE`, which governs the opposite
//! direction.

use super::*;

/// Restores every env seam on drop, so a failing assertion inside the closure
/// still cleans up instead of leaking an override into the #457/#573 suites.
struct EnvRestore {
    wheel: Option<String>,
    mouse: Option<String>,
    build: Option<String>,
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        set_or_clear(FORCE_WHEEL_ENV, self.wheel.as_deref());
        set_or_clear(FORCE_MOUSE_ENV, self.mouse.as_deref());
        set_or_clear("PSMUX_FAKE_WIN_BUILD", self.build.as_deref());
    }
}

fn set_or_clear(key: &str, value: Option<&str>) {
    match value {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

/// Set the env seams for the duration of the closure and restore them after.
/// Uses the shared env lock so this never races other env-touching tests.
fn with_env<T>(
    wheel: Option<&str>,
    mouse: Option<&str>,
    build: Option<&str>,
    f: impl FnOnce() -> T,
) -> T {
    let _lock = crate::util::lock_test_env();
    let _restore = EnvRestore {
        wheel: std::env::var(FORCE_WHEEL_ENV).ok(),
        mouse: std::env::var(FORCE_MOUSE_ENV).ok(),
        build: std::env::var("PSMUX_FAKE_WIN_BUILD").ok(),
    };
    set_or_clear(FORCE_WHEEL_ENV, wheel);
    set_or_clear(FORCE_MOUSE_ENV, mouse);
    set_or_clear("PSMUX_FAKE_WIN_BUILD", build);
    f()
}

#[test]
fn unset_keeps_the_gate() {
    with_env(None, None, None, || {
        assert!(
            !wheel_gate_forced(),
            "the #598 gate must stay in force for anyone who did not opt in"
        );
    });
}

#[test]
fn affirmative_spellings_open_the_gate() {
    // Same vocabulary as PSMUX_FORCE_MOUSE, including the trim and the case
    // fold, so a user who learned one env var can spell the other.
    for raw in ["1", "on", "true", "yes", "  ON  ", "True"] {
        with_env(Some(raw), None, None, || {
            assert!(
                wheel_gate_forced(),
                "BUG #613: PSMUX_FORCE_WHEEL={raw:?} must authorize the wheel"
            );
        });
    }
}

#[test]
fn everything_else_keeps_the_gate() {
    // Unlike forced_mouse_setting there is no third state to express: the gate
    // is the safe side, so an unrecognised value must not be read as consent.
    for raw in ["0", "off", "false", "no", "", "   ", "maybe", "2"] {
        with_env(Some(raw), None, None, || {
            assert!(
                !wheel_gate_forced(),
                "PSMUX_FORCE_WHEEL={raw:?} is not consent and must keep the gate"
            );
        });
    }
}

#[test]
fn force_mouse_does_not_open_the_wheel_gate() {
    // The two overrides travel in opposite directions: PSMUX_FORCE_MOUSE is
    // about whether psmux may write mouse DECSET out to the terminal, this one
    // is about whether a report psmux already holds may be delivered into a
    // pane. A host that needed the first says nothing about the second.
    with_env(None, Some("1"), None, || {
        assert!(
            !wheel_gate_forced(),
            "PSMUX_FORCE_MOUSE must not silently widen the pane-direction gate"
        );
    });
}

#[test]
fn force_wheel_does_not_open_the_client_direction_gate() {
    // And the reverse, which is the direction that can kill a pane: #457's
    // build gate exists because an SGR report fast-fails Win10-era conhost.
    // Opting into the wheel must not reach it.
    with_env(Some("1"), None, Some("20348"), || {
        assert!(
            !conpty_mouse_supported(),
            "PSMUX_FORCE_WHEEL must not relax the #457 build gate"
        );
    });
}
