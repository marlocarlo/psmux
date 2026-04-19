use super::*;

/// Issue #151: CWD hook guard must survive Set-StrictMode -Version Latest.
///
/// When a user has `Set-StrictMode -Version 2` (or `Latest`) in their
/// PowerShell profile, reading an unset variable like
/// `$Global:__psmux_cwd_hook` throws an InvalidOperation error.
///
/// The fix uses `Test-Path variable:Global:__psmux_cwd_hook` instead,
/// which is strict-mode-safe.

#[test]
fn cwd_sync_uses_test_path_guard() {
    // The CWD_SYNC constant must use `Test-Path variable:` for the guard
    // instead of directly reading the variable.
    let init = build_psrl_init(false, false);
    assert!(
        init.contains("Test-Path variable:Global:__psmux_cwd_hook"),
        "CWD_SYNC guard must use Test-Path to be strict-mode-safe, got: {}",
        init
    );
    assert!(
        !init.contains("if (-not $Global:__psmux_cwd_hook)"),
        "CWD_SYNC must NOT directly read $Global:__psmux_cwd_hook (breaks under Set-StrictMode)"
    );
}

#[test]
fn cwd_sync_guard_present_with_predictions_allowed() {
    let init = build_psrl_init(false, true);
    assert!(
        init.contains("Test-Path variable:Global:__psmux_cwd_hook"),
        "CWD_SYNC guard must use Test-Path even with allow_predictions=true"
    );
}

#[test]
fn cwd_sync_guard_present_with_env_shim() {
    let init = build_psrl_init(true, false);
    assert!(
        init.contains("Test-Path variable:Global:__psmux_cwd_hook"),
        "CWD_SYNC guard must use Test-Path even with env_shim=true"
    );
}

#[test]
fn cwd_sync_sets_guard_variable_after_check() {
    let init = build_psrl_init(false, false);
    // The guard should set the variable to $true after the Test-Path check
    let test_path_pos = init.find("Test-Path variable:Global:__psmux_cwd_hook")
        .expect("Test-Path guard not found in init string");
    let set_pos = init.find("$Global:__psmux_cwd_hook = $true")
        .expect("Guard variable assignment not found in init string");
    assert!(
        set_pos > test_path_pos,
        "Guard variable must be set AFTER the Test-Path check"
    );
}

#[test]
fn cwd_sync_wraps_set_push_pop_location() {
    let init = build_psrl_init(false, false);
    assert!(init.contains("function Global:Set-Location"), "Must wrap Set-Location");
    assert!(init.contains("function Global:Push-Location"), "Must wrap Push-Location");
    assert!(init.contains("function Global:Pop-Location"), "Must wrap Pop-Location");
}

#[test]
fn cwd_sync_calls_set_current_directory() {
    let init = build_psrl_init(false, false);
    let count = init.matches("SetCurrentDirectory").count();
    // Once for initial sync + once in each of the three wrappers = 4
    assert!(
        count >= 4,
        "Expected at least 4 SetCurrentDirectory calls (initial + 3 wrappers), got {}",
        count
    );
}
