use super::*;

// ── Issue #155: OutputRendering not forced ─────────────────────────

#[test]
fn psrl_init_does_not_force_output_rendering() {
    // Verify that the PSRL_FIX, PSRL_CRASH_GUARD, and PSRL_PRED_RESTORE
    // constants no longer contain "$PSStyle.OutputRendering"
    let psrl_fix = PSRL_FIX;
    let crash_guard = PSRL_CRASH_GUARD;
    let pred_restore = PSRL_PRED_RESTORE;
    assert!(
        !psrl_fix.contains("OutputRendering"),
        "PSRL_FIX should not force OutputRendering, got: {psrl_fix}"
    );
    assert!(
        !crash_guard.contains("OutputRendering"),
        "PSRL_CRASH_GUARD should not force OutputRendering, got: {crash_guard}"
    );
    assert!(
        !pred_restore.contains("OutputRendering"),
        "PSRL_PRED_RESTORE should not force OutputRendering, got: {pred_restore}"
    );
}
