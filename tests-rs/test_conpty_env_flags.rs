// Regression tests for the ConPTY diagnostic escape hatches (42ca018) and
// the sideloading rules (87055f6).
//
//   - PSMUX_NO_WIN32_INPUT=1 / =true drops PSEUDOCONSOLE_WIN32_INPUT_MODE
//     from every CreatePseudoConsole flag set (same shape as
//     PSMUX_NO_PASSTHROUGH), so a conhost build that crashes around that
//     mode can be diagnosed without rebuilding.
//   - PSMUX_NO_PASSTHROUGH=1 short-circuits supports_passthrough_mode()
//     before any OS version probing.
//
// These tests lock the env branches only: no DLL is loaded and no pseudo
// console is created, so they are safe on any Windows CI. The sideload
// precedence inside load_conpty() (explicit PSMUX_CONPTY_DLL, then a
// bundled conpty.dll + OpenConsole.exe beside the exe, then kernel32) is
// not closed-testable without actually loading DLLs, so it is deliberately
// not exercised here.
//
// This file lives under src/win/, which is cfg(windows): macOS never
// compiles it. Env is process-global, so the tests serialize on ENV_LOCK
// and restore the previous value via a Drop guard.

use super::*;

use parking_lot::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Restores a mutated env var on drop, so a failure mid-test cannot leak
/// the value into other tests.
struct EnvGuard {
    var: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(var: &'static str, value: &str) -> EnvGuard {
        let prev = std::env::var_os(var);
        std::env::set_var(var, value);
        EnvGuard { var, prev }
    }

    fn remove(var: &'static str) -> EnvGuard {
        let prev = std::env::var_os(var);
        std::env::remove_var(var);
        EnvGuard { var, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => std::env::set_var(self.var, value),
            None => std::env::remove_var(self.var),
        }
    }
}

/// PSMUX_NO_WIN32_INPUT=1 drops PSEUDOCONSOLE_WIN32_INPUT_MODE entirely:
/// the base flag set collapses to the resize quirk alone.
#[test]
fn no_win32_input_drops_win32_input_mode() {
    let _lock = ENV_LOCK.lock();
    let _g = EnvGuard::set("PSMUX_NO_WIN32_INPUT", "1");
    assert_eq!(
        base_flags(),
        PSEUDOCONSOLE_RESIZE_QUIRK,
        "the escape hatch must remove WIN32_INPUT_MODE"
    );
    assert_eq!(base_flags() & PSEUDOCONSOLE_WIN32_INPUT_MODE, 0);
}

/// The same escape hatch accepts the boolean-true spellings used by the
/// other PSMUX_NO_* switches.
#[test]
fn no_win32_input_true_spellings_also_disable() {
    let _lock = ENV_LOCK.lock();
    for value in ["true", "TRUE", "True"] {
        let _g = EnvGuard::set("PSMUX_NO_WIN32_INPUT", value);
        assert_eq!(
            base_flags() & PSEUDOCONSOLE_WIN32_INPUT_MODE,
            0,
            "PSMUX_NO_WIN32_INPUT={value} must disable WIN32_INPUT_MODE"
        );
    }
}

/// Unset or "0" keeps the standard flag set — the escape hatch must not
/// leak into normal operation.
#[test]
fn no_win32_input_unset_or_zero_keeps_win32_input_mode() {
    let _lock = ENV_LOCK.lock();
    let _unset = EnvGuard::remove("PSMUX_NO_WIN32_INPUT");
    assert_ne!(base_flags() & PSEUDOCONSOLE_WIN32_INPUT_MODE, 0);
    assert_ne!(base_flags() & PSEUDOCONSOLE_RESIZE_QUIRK, 0);

    let _zero = EnvGuard::set("PSMUX_NO_WIN32_INPUT", "0");
    assert_ne!(base_flags() & PSEUDOCONSOLE_WIN32_INPUT_MODE, 0);
}

/// PSMUX_NO_PASSTHROUGH short-circuits the passthrough probe before any OS
/// call, in every accepted spelling.
#[test]
fn no_passthrough_short_circuits_before_os_probing() {
    let _lock = ENV_LOCK.lock();
    for value in ["1", "true", "TRUE", "True"] {
        let _g = EnvGuard::set("PSMUX_NO_PASSTHROUGH", value);
        assert!(
            !supports_passthrough_mode(),
            "PSMUX_NO_PASSTHROUGH={value} must force passthrough off"
        );
    }
}

/// The escape hatch and the default path agree on the resize quirk: even
/// with WIN32_INPUT_MODE dropped, the resize quirk stays.
#[test]
fn resize_quirk_survives_both_escape_hatches() {
    let _lock = ENV_LOCK.lock();
    let _a = EnvGuard::set("PSMUX_NO_WIN32_INPUT", "1");
    let _b = EnvGuard::set("PSMUX_NO_PASSTHROUGH", "1");
    assert_ne!(base_flags() & PSEUDOCONSOLE_RESIZE_QUIRK, 0);
    assert!(!supports_passthrough_mode());
}
