//! In-memory model for sixel graphics captured from a pane's DCS stream.
//!
//! The parser reconstructs each sixel DCS sequence into a [`SixelImage`] and
//! hands it to [`crate::Screen::add_image`].  The image blob is stored verbatim
//! so the client can re-emit it to a sixel-capable outer terminal; the cell and
//! anchor fields let the server compute a viewport-relative draw position that
//! survives scrolling.  See the sixel design (issue #431) for the full pipeline.

/// Default outer-terminal cell size in pixels, `(width, height)`.
///
/// psmux does not know the host terminal's true cell size today, so v1 uses the
/// xterm / tmux fallback of 10x20.  This only affects cursor-advance and
/// clipping math (how many cells the image occupies), NOT the rasterized pixels:
/// the outer terminal draws the raw bytes at its own real cell size.  A wrong
/// constant costs at most a slightly-off cursor-below position and a
/// conservative clip.
pub const DEFAULT_CELL_PX: (u32, u32) = (10, 20);

/// Computes an image's cell footprint from its pixel size using ceil-division
/// against [`DEFAULT_CELL_PX`].  A non-empty image always occupies at least one
/// cell in each axis; the result is clamped into `u16`.
#[must_use]
pub fn cell_dims_from_px(px_width: u32, px_height: u32) -> (u16, u16) {
    let (cell_px_w, cell_px_h) = DEFAULT_CELL_PX;
    let cells_w = px_width.div_ceil(cell_px_w.max(1)).max(1);
    let cells_h = px_height.div_ceil(cell_px_h.max(1)).max(1);
    (
        u16::try_from(cells_w).unwrap_or(u16::MAX),
        u16::try_from(cells_h).unwrap_or(u16::MAX),
    )
}

/// A single sixel image anchored at a logical position on a pane's grid.
#[derive(Clone, Debug)]
pub struct SixelImage {
    /// Stable per-`Screen` monotonic id.  The blob is shipped to the client
    /// once and referenced by id thereafter.
    pub id: u64,
    /// Exact re-emit bytes: `ESC P` + params + intermediates + `q` + payload +
    /// `ST`.  Writable verbatim to a sixel-capable terminal after a cursor move.
    pub raw: Vec<u8>,
    /// Image width in pixels (from the raster attributes, else scanned).
    pub px_width: u32,
    /// Image height in pixels (from the raster attributes, else scanned).
    pub px_height: u32,
    /// Cell width = `ceil(px_width / cell_px_w)` (see [`DEFAULT_CELL_PX`]).
    pub cell_width: u16,
    /// Cell height = `ceil(px_height / cell_px_h)` (see [`DEFAULT_CELL_PX`]).
    pub cell_height: u16,
    /// Absolute logical line of the top-left cell; survives scrolling and is
    /// pruned when it falls off retained scrollback.
    pub anchor_line: u64,
    /// Grid column of the top-left cell at emit time.
    pub anchor_col: u16,
    /// Grid ownership: images tagged `on_alt` belong to the alternate screen
    /// and are dropped when it exits.
    pub on_alt: bool,
}
