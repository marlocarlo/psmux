use super::*;

// ── Issue #165: PredictionViewStyle ListView not working ────────────

#[test]
fn psrl_init_allow_predictions_on_does_not_touch_view_style() {
    // When allow-predictions is ON, the init string must NOT contain
    // PredictionViewStyle so the user's profile setting is preserved (#165).
    let init = build_psrl_init(false, true);
    assert!(
        !init.contains("PredictionViewStyle"),
        "allow-predictions ON: init string must not override PredictionViewStyle, got: {init}"
    );
}

#[test]
fn psrl_init_allow_predictions_on_does_not_remove_f2() {
    // When allow-predictions is ON, the init string must NOT remove the
    // F2 key handler so the user's bindings survive (#165).
    let init = build_psrl_init(false, true);
    assert!(
        !init.contains("Remove-PSReadLineKeyHandler"),
        "allow-predictions ON: init string must not remove F2, got: {init}"
    );
}

#[test]
fn psrl_init_allow_predictions_on_restores_prediction_source() {
    // When allow-predictions is ON, the init string must contain the
    // restore logic that checks PredictionSource after the profile.
    let init = build_psrl_init(false, true);
    assert!(
        init.contains("__psmux_origPred"),
        "allow-predictions ON: init string must save/restore PredictionSource, got: {init}"
    );
}

#[test]
fn psrl_init_allow_predictions_off_forces_inline_view() {
    // When allow-predictions is OFF (default), PSRL_FIX forces
    // PredictionViewStyle InlineView both pre and post profile.
    let init = build_psrl_init(false, false);
    assert!(
        init.contains("PredictionViewStyle InlineView"),
        "allow-predictions OFF: init string must force InlineView, got: {init}"
    );
}

#[test]
fn psrl_crash_guard_does_not_contain_view_style() {
    // PSRL_CRASH_GUARD must only save/disable PredictionSource.
    // It must NOT touch PredictionViewStyle.
    let guard = PSRL_CRASH_GUARD;
    assert!(
        !guard.contains("PredictionViewStyle"),
        "PSRL_CRASH_GUARD must not touch PredictionViewStyle, got: {guard}"
    );
}

#[test]
fn psrl_pred_restore_does_not_contain_view_style() {
    // PSRL_PRED_RESTORE must only restore PredictionSource.
    // It must NOT touch PredictionViewStyle.
    let restore = PSRL_PRED_RESTORE;
    assert!(
        !restore.contains("PredictionViewStyle"),
        "PSRL_PRED_RESTORE must not touch PredictionViewStyle, got: {restore}"
    );
}
