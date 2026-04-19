#[allow(unused_imports)]
use std::io;

use serde::{Serialize, Deserialize};
use unicode_width::UnicodeWidthStr;

use crate::types::{AppState, Node, LayoutKind, Mode};
use crate::tree::get_split_mut;

/// Serialize a vt100 screen region into run-length-encoded rows (rows_v2 format).
///
/// This is the shared serialization used by both pane layout rendering and popup
/// overlay rendering.  Extracts cells from [0..rows) x [0..cols), merges
/// adjacent cells with identical styling into runs, and returns the result
/// as a `Vec<RowRunsJson>`.
use super::*;

/// Direct JSON serialisation of the layout tree – writes JSON straight into
/// a pre-allocated `String`, avoiding the intermediate `LayoutJson` / `CellRunJson`
/// allocations **and** the `serde_json::to_string` traversal.  Produces the
/// identical JSON format that the client deserialises into `LayoutJson`.
pub fn dump_layout_json_fast(app: &mut AppState) -> io::Result<String> {
    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
    let scroll_off = app.copy_scroll_offset;
    let anchor = app.copy_anchor;
    let anchor_scroll = app.copy_anchor_scroll_offset;
    let cpos = app.copy_pos;
    let sel_mode = app.copy_selection_mode;

    // ── tiny helpers are in json_helpers.rs (module-level) ─────────

    // ── recursive tree walker ────────────────────────────────────────

    fn write_node(
        node: &mut Node,
        cur_path: &mut Vec<usize>,
        active_path: &[usize],
        in_copy: bool,
        scroll_off: usize,
        anchor: Option<(u16, u16)>,
        anchor_scroll: usize,
        cpos: Option<(u16, u16)>,
        sel_mode: crate::types::SelectionMode,
        out: &mut String,
    ) {
        match node {
            Node::Split { kind, sizes, children } => {
                out.push_str("{\"type\":\"split\",\"kind\":\"");
                match kind {
                    LayoutKind::Horizontal => out.push_str("Horizontal"),
                    LayoutKind::Vertical   => out.push_str("Vertical"),
                }
                out.push_str("\",\"sizes\":[");
                for (i, s) in sizes.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", s));
                }
                out.push_str("],\"children\":[");
                for (i, c) in children.iter_mut().enumerate() {
                    if i > 0 { out.push(','); }
                    cur_path.push(i);
                    write_node(c, cur_path, active_path, in_copy, scroll_off, anchor, anchor_scroll, cpos, sel_mode, out);
                    cur_path.pop();
                }
                out.push_str("]}");
            }

            Node::Leaf(p) => {
                const FLAG_DIM: u8      = 1;
                const FLAG_BOLD: u8     = 2;
                const FLAG_ITALIC: u8   = 4;
                const FLAG_UNDERLINE: u8 = 8;
                const FLAG_INVERSE: u8  = 16;
                const FLAG_BLINK: u8    = 32;
                const FLAG_HIDDEN: u8   = 64;
                const FLAG_STRIKETHROUGH: u8 = 128;

                // If the pane is squelched, emit a blank leaf.
                if p.squelch_until.is_some() {
                    let sentinel_arrived = p.term.lock()
                        .map(|mut parser| parser.screen_mut().take_squelch_cleared())
                        .unwrap_or(false);
                    if sentinel_arrived {
                        p.squelch_until = None;
                    } else if p.squelch_until.map_or(false, |d| std::time::Instant::now() < d) {
                        let is_active = cur_path.as_slice() == active_path;
                        let _ = std::fmt::Write::write_fmt(out, format_args!(
                            concat!(
                                "{{\"type\":\"leaf\",\"id\":{},",
                                "\"rows\":{},\"cols\":{},",
                                "\"cursor_row\":0,\"cursor_col\":0,",
                                "\"alternate_screen\":false,",
                                "\"hide_cursor\":true,",
                                "\"cursor_shape\":0,",
                                "\"active\":{},\"copy_mode\":false,",
                                "\"scroll_offset\":0,",
                                "\"rows_v2\":[],\"content\":[],\"title\":null}}"),
                            p.id, p.last_rows, p.last_cols, is_active,
                        ));
                        return;
                    } else {
                        p.squelch_until = None;
                    }
                }

                let is_active    = cur_path.as_slice() == active_path;
                let need_content = in_copy && is_active;

                // ── Snapshot cell data under the mutex, then release ──
                // This minimises the time we block the reader thread (which
                // also holds p.term's mutex while processing ConPTY output).
                // Without this, WSL echo gets starved because its output sits
                // in the ConPTY pipe while we build the JSON string.
                struct Run { text: String, fg: vt100::Color, bg: vt100::Color, flags: u8, width: u16 }
                struct RowSnap { runs: Vec<Run> }
                struct CopyCell { text: String, fg: vt100::Color, bg: vt100::Color, bold: bool, italic: bool, underline: bool, inverse: bool, dim: bool, blink: bool, hidden: bool, strikethrough: bool, width: u16 }
                struct LeafSnap {
                    cr: u16, cc: u16, alt: bool,
                    hide_cursor: bool,
                    rows_v2: Vec<RowSnap>,
                    content: Vec<Vec<CopyCell>>,
                }

                let snap = 'snap: {
                    let parser = match p.term.lock() {
                        Ok(g) => g,
                        Err(_) => break 'snap LeafSnap { cr: 0, cc: 0, alt: false, hide_cursor: false, rows_v2: vec![], content: vec![] },
                    };
                    let screen = parser.screen();
                    let (cr, cc) = screen.cursor_position();
                    let hide_cursor = screen.hide_cursor();

                    // Alternate-screen heuristic
                    let alt = screen.alternate_screen() || {
                        let lr = p.last_rows.saturating_sub(1);
                        (0..p.last_cols).any(|col| {
                            screen.cell(lr, col).map_or(false, |c| {
                                let t = c.contents();
                                !t.is_empty() && t != " "
                            })
                        })
                    };

                    // Snapshot rows_v2 (run-merged)
                    let mut snap_rows: Vec<RowSnap> = Vec::with_capacity(p.last_rows as usize);
                    for r in 0..p.last_rows {
                        let mut runs: Vec<Run> = Vec::new();
                        let mut c = 0u16;
                        let mut prev_fg: Option<vt100::Color> = None;
                        let mut prev_bg: Option<vt100::Color> = None;
                        let mut prev_fl: u8 = 0;

                        while c < p.last_cols {
                            if let Some(cell) = screen.cell(r, c) {
                                let t = cell.contents();
                                let t = if t.is_empty() { " " } else { t };
                                let cfg = cell.fgcolor();
                                let cbg = cell.bgcolor();
                                let mut w = UnicodeWidthStr::width(t) as u16;
                                if w == 0 { w = 1; }
                                let mut fl = 0u8;
                                if cell.dim()   { fl |= FLAG_DIM; }
                                if cell.bold()  { fl |= FLAG_BOLD; }
                                if cell.italic(){ fl |= FLAG_ITALIC; }
                                if cell.underline() { fl |= FLAG_UNDERLINE; }
                                if cell.inverse()   { fl |= FLAG_INVERSE; }
                                if cell.blink()     { fl |= FLAG_BLINK; }
                                if cell.hidden()    { fl |= FLAG_HIDDEN; }
                                if cell.strikethrough() { fl |= FLAG_STRIKETHROUGH; }

                                if prev_fg == Some(cfg) && prev_bg == Some(cbg) && prev_fl == fl {
                                    if let Some(last) = runs.last_mut() {
                                        last.text.push_str(t);
                                        last.width += w;
                                    }
                                } else {
                                    runs.push(Run { text: t.to_string(), fg: cfg, bg: cbg, flags: fl, width: w });
                                }
                                prev_fg = Some(cfg);
                                prev_bg = Some(cbg);
                                prev_fl = fl;
                                c += w.max(1);
                            } else {
                                let cfg = vt100::Color::Default;
                                let cbg = vt100::Color::Default;
                                let fl  = 0u8;
                                if prev_fg == Some(cfg) && prev_bg == Some(cbg) && prev_fl == fl {
                                    if let Some(last) = runs.last_mut() {
                                        last.text.push(' ');
                                        last.width += 1;
                                    }
                                } else {
                                    runs.push(Run { text: " ".to_string(), fg: cfg, bg: cbg, flags: fl, width: 1 });
                                }
                                prev_fg = Some(cfg);
                                prev_bg = Some(cbg);
                                prev_fl = fl;
                                c += 1;
                            }
                        }
                        snap_rows.push(RowSnap { runs });
                    }

                    // Snapshot content (copy-mode only)
                    let mut snap_content: Vec<Vec<CopyCell>> = Vec::new();
                    if need_content {
                        for r in 0..p.last_rows {
                            let mut row_cells: Vec<CopyCell> = Vec::new();
                            let mut c = 0u16;
                            while c < p.last_cols {
                                if let Some(cell) = screen.cell(r, c) {
                                    let t = cell.contents();
                                    let t = if t.is_empty() { " " } else { t };
                                    let w = UnicodeWidthStr::width(t).max(1) as u16;
                                    row_cells.push(CopyCell {
                                        text: t.to_string(), fg: cell.fgcolor(), bg: cell.bgcolor(),
                                        bold: cell.bold(), italic: cell.italic(), underline: cell.underline(),
                                        inverse: cell.inverse(), dim: cell.dim(), blink: cell.blink(), hidden: cell.hidden(), strikethrough: cell.strikethrough(), width: w,
                                    });
                                    c += w;
                                } else {
                                    row_cells.push(CopyCell {
                                        text: " ".to_string(), fg: vt100::Color::Default, bg: vt100::Color::Default,
                                        bold: false, italic: false, underline: false, inverse: false, dim: false, blink: false, hidden: false, strikethrough: false, width: 1,
                                    });
                                    c += 1;
                                }
                            }
                            snap_content.push(row_cells);
                        }
                    }

                    LeafSnap { cr, cc, alt, hide_cursor, rows_v2: snap_rows, content: snap_content }
                };
                // ── Parser mutex is now RELEASED ──
                // All JSON string building below happens without holding the lock,
                // so the reader thread can process ConPTY output concurrently.

                // ── leaf header ──────────────────────────────────────
                let so = if is_active && in_copy { scroll_off } else { 0 };
                let cs = p.cursor_shape.load(std::sync::atomic::Ordering::Relaxed);
                let _ = std::fmt::Write::write_fmt(out, format_args!(
                    concat!(
                        "{{\"type\":\"leaf\",\"id\":{},",
                        "\"rows\":{},\"cols\":{},",
                        "\"cursor_row\":{},\"cursor_col\":{},",
                        "\"alternate_screen\":{},",
                        "\"hide_cursor\":{},",
                        "\"cursor_shape\":{},",
                        "\"active\":{},\"copy_mode\":{},",
                        "\"scroll_offset\":{},"),
                    p.id, p.last_rows, p.last_cols,
                    snap.cr, snap.cc, snap.alt, snap.hide_cursor,
                    cs,
                    is_active, need_content, so,
                ));

                // selection bounds + copy cursor position
                if is_active && in_copy {
                    if let (Some((ar, ac)), Some((pr, pc))) = (anchor, cpos) {
                        // Compute display position of anchor accounting for
                        // scrollback changes since the anchor was set.  Clamp
                        // to the visible row range [0, last_rows-1].
                        let display_ar = (ar as i32 + scroll_off as i32 - anchor_scroll as i32)
                            .max(0)
                            .min(p.last_rows as i32 - 1) as u16;
                        // For char mode: send directional start/end so the
                        // client can render flow selection (first line from
                        // start_col to EOL, middle full, last line to end_col).
                        // For rect mode: send min/max columns.
                        // For line mode: columns are irrelevant.
                        let (sr, sc, er, ec) = match sel_mode {
                            crate::types::SelectionMode::Char => {
                                let top = display_ar.min(pr);
                                let bot = display_ar.max(pr);
                                let (tc, bc) = if display_ar <= pr {
                                    (ac, pc) // anchor is top, cursor is bottom
                                } else {
                                    (pc, ac) // cursor is top, anchor is bottom
                                };
                                (top, tc, bot, bc)
                            }
                            crate::types::SelectionMode::Rect => {
                                (display_ar.min(pr), ac.min(pc), display_ar.max(pr), ac.max(pc))
                            }
                            crate::types::SelectionMode::Line => {
                                (display_ar.min(pr), 0u16, display_ar.max(pr), p.last_cols.saturating_sub(1))
                            }
                        };
                        let mode_str = match sel_mode {
                            crate::types::SelectionMode::Char => "char",
                            crate::types::SelectionMode::Line => "line",
                            crate::types::SelectionMode::Rect => "rect",
                        };
                        let _ = std::fmt::Write::write_fmt(out, format_args!(
                            "\"sel_start_row\":{},\"sel_start_col\":{},\"sel_end_row\":{},\"sel_end_col\":{},\"sel_mode\":\"{}\",",
                            sr, sc, er, ec, mode_str,
                        ));
                    } else {
                        out.push_str("\"sel_start_row\":null,\"sel_start_col\":null,\"sel_end_row\":null,\"sel_end_col\":null,\"sel_mode\":null,");
                    }
                    if let Some((pr, pc)) = cpos {
                        let _ = std::fmt::Write::write_fmt(out, format_args!(
                            "\"copy_cursor_row\":{},\"copy_cursor_col\":{},",
                            pr, pc,
                        ));
                    } else {
                        out.push_str("\"copy_cursor_row\":null,\"copy_cursor_col\":null,");
                    }
                } else {
                    out.push_str("\"sel_start_row\":null,\"sel_start_col\":null,\"sel_end_row\":null,\"sel_end_col\":null,\"sel_mode\":null,");
                    out.push_str("\"copy_cursor_row\":null,\"copy_cursor_col\":null,");
                }

                // ── content (per-cell, only in copy-mode active pane) ──
                if need_content && !snap.content.is_empty() {
                    out.push_str("\"content\":[");
                    for (ri, row) in snap.content.iter().enumerate() {
                        if ri > 0 { out.push(','); }
                        out.push('[');
                        for (ci, cell) in row.iter().enumerate() {
                            if ci > 0 { out.push(','); }
                            out.push_str("{\"text\":\"");
                            json_esc(&cell.text, out);
                            out.push_str("\",\"fg\":\"");
                            push_color(cell.fg, out);
                            out.push_str("\",\"bg\":\"");
                            push_color(cell.bg, out);
                            let _ = std::fmt::Write::write_fmt(out, format_args!(
                                "\",\"bold\":{},\"italic\":{},\"underline\":{},\"inverse\":{},\"dim\":{},\"blink\":{},\"hidden\":{},\"strikethrough\":{}}}",
                                cell.bold, cell.italic, cell.underline, cell.inverse, cell.dim, cell.blink, cell.hidden, cell.strikethrough,
                            ));
                            // Emit width-2 filler cells
                            for _ in 1..cell.width {
                                out.push_str(",{\"text\":\"\",\"fg\":\"");
                                push_color(cell.fg, out);
                                out.push_str("\",\"bg\":\"");
                                push_color(cell.bg, out);
                                let _ = std::fmt::Write::write_fmt(out, format_args!(
                                    "\",\"bold\":{},\"italic\":{},\"underline\":{},\"inverse\":{},\"dim\":{},\"blink\":{},\"hidden\":{},\"strikethrough\":{}}}",
                                    cell.bold, cell.italic, cell.underline, cell.inverse, cell.dim, cell.blink, cell.hidden, cell.strikethrough,
                                ));
                            }
                        }
                        // pad to full column width
                        let total_w: u16 = row.iter().map(|c| c.width).sum();
                        for _ in total_w..p.last_cols {
                            out.push_str(",{\"text\":\" \",\"fg\":\"default\",\"bg\":\"default\",\"bold\":false,\"italic\":false,\"underline\":false,\"inverse\":false,\"dim\":false,\"blink\":false,\"hidden\":false,\"strikethrough\":false}");
                        }
                        out.push(']');
                    }
                    out.push_str("],");
                } else {
                    out.push_str("\"content\":[],");
                }

                // ── rows_v2 (from snapshot, no mutex held) ───────────
                out.push_str("\"rows_v2\":[");
                for (ri, row) in snap.rows_v2.iter().enumerate() {
                    if ri > 0 { out.push(','); }
                    out.push_str("{\"runs\":[");
                    for (i, run) in row.runs.iter().enumerate() {
                        if i > 0 { out.push(','); }
                        out.push_str("{\"text\":\"");
                        json_esc(&run.text, out);
                        close_run(run.fg, run.bg, run.flags, run.width, out);
                    }
                    out.push_str("]}");
                }
                out.push_str("]");
                // Append pane title if set
                if !p.title.is_empty() {
                    out.push_str(",\"title\":\"");
                    json_esc(&p.title, out);
                    out.push('"');
                }
                out.push('}');
            }
        }
    }

    let win = &mut app.windows[app.active_idx];
    let active_path = win.active_path.clone();
    let mut path = Vec::new();
    let mut out = String::with_capacity(32768);
    write_node(
        &mut win.root, &mut path, &active_path,
        in_copy, scroll_off, anchor, anchor_scroll, cpos, sel_mode, &mut out,
    );
    Ok(out)
}
