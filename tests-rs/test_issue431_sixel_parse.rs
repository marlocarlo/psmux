// Issue #431 (M2 parser DCS handling): the vte DCS hook/put/unhook path must
// capture a sixel graphics DCS (`ESC P ... q ... ST`) into one SixelImage on the
// Screen, leave the surrounding text intact, advance the cursor below the image,
// and — critically — ignore every NON-sixel DCS (e.g. DECRQSS) so the historical
// no-op behaviour is preserved.
use crate::Parser;

// A real, self-contained sixel: raster `"1;1;24;12` (24x12 px) then two bands.
const SIXEL: &[u8] =
    b"\x1bP0;0;0q\"1;1;24;12#0;2;100;0;0#0!24~$-!24~$-\x1b\\";

#[test]
fn sixel_dcs_becomes_one_image_and_advances_cursor() {
    let mut p: Parser = Parser::new(24, 80, 100);

    // Text before the image on its own line.
    p.process(b"BEFORE\n");
    let (row_before, _col_before) = p.screen().cursor_position();

    // Feed the sixel; it must materialise exactly one image.
    p.process(SIXEL);
    assert_eq!(p.screen().images().len(), 1, "sixel DCS -> one image");

    let img = &p.screen().images()[0];
    // Raw blob round-trips as ESC P ... ST.
    assert!(img.raw.starts_with(b"\x1bP"), "raw starts with ESC P");
    assert!(img.raw.ends_with(b"\x1b\\"), "raw ends with ST");
    // Raster `"1;1;24;12` => 24x12 px, cells ceil(24/10)=3 x ceil(12/20)=1.
    assert_eq!(img.px_width, 24);
    assert_eq!(img.px_height, 12);
    assert_eq!(img.cell_width, 3);
    assert_eq!(img.cell_height, 1);

    // Cursor advanced by exactly cell_height rows (to the left margin below).
    let (row_after, col_after) = p.screen().cursor_position();
    assert_eq!(
        row_after,
        row_before + u16::from(img.cell_height),
        "cursor moved down cell_height rows",
    );
    assert_eq!(col_after, 0, "cursor returned to the left margin");

    // Text after the image still lands and the screen keeps both labels.
    p.process(b"AFTER");
    let text = p.screen().contents();
    assert!(text.contains("BEFORE"), "BEFORE survived the sixel");
    assert!(text.contains("AFTER"), "AFTER printed below the sixel");

    // Still exactly one image after the trailing text.
    assert_eq!(p.screen().images().len(), 1);
}

#[test]
fn decrqss_dcs_produces_no_image_and_no_panic() {
    // DECRQSS: ESC P $ q m ESC \  — final byte 'q' but with the '$' intermediate.
    // Must be ignored entirely (no image, no state change, no panic).
    let mut p: Parser = Parser::new(24, 80, 100);
    p.process(b"HELLO");
    p.process(b"\x1bP$qm\x1b\\");
    assert_eq!(
        p.screen().images().len(),
        0,
        "DECRQSS (non-sixel DCS) must not create an image",
    );
    // Surrounding text is unaffected.
    p.process(b"WORLD");
    let text = p.screen().contents();
    assert!(text.contains("HELLO"));
    assert!(text.contains("WORLD"));
}

#[test]
fn sixel_without_raster_falls_back_to_scanned_dims() {
    // No `"` raster command: dimensions come from the data scan. Two bands of
    // 4 sixel columns each => width 4, height 12 (2 bands x 6).
    let mut p: Parser = Parser::new(24, 80, 100);
    p.process(b"\x1bP0;0;0q#0~~~~$-~~~~\x1b\\");
    assert_eq!(p.screen().images().len(), 1);
    let img = &p.screen().images()[0];
    assert_eq!(img.px_width, 4, "widest band = 4 sixel columns");
    assert_eq!(img.px_height, 12, "2 bands x 6 px");
    // cells: ceil(4/10)=1 x ceil(12/20)=1.
    assert_eq!(img.cell_width, 1);
    assert_eq!(img.cell_height, 1);
}
