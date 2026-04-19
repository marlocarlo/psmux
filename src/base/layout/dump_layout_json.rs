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

pub fn dump_layout_json(app: &mut AppState) -> io::Result<String> {
    let in_copy_mode = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
    let scroll_offset = app.copy_scroll_offset;
    
    fn build(node: &mut Node, cur_path: &mut Vec<usize>, active_path: &[usize], include_full_content: bool) -> LayoutJson {
        match node {
            Node::Split { kind, sizes, children } => {
                let k = match *kind { LayoutKind::Horizontal => "Horizontal".to_string(), LayoutKind::Vertical => "Vertical".to_string() };
                let mut ch: Vec<LayoutJson> = Vec::new();
                for (i, c) in children.iter_mut().enumerate() {
                    cur_path.push(i);
                    ch.push(build(c, cur_path, active_path, include_full_content));
                    cur_path.pop();
                }
                LayoutJson::Split { kind: k, sizes: sizes.clone(), children: ch }
            }
            Node::Leaf(p) => {
                const FLAG_DIM: u8 = 1;
                const FLAG_BOLD: u8 = 2;
                const FLAG_ITALIC: u8 = 4;
                const FLAG_UNDERLINE: u8 = 8;
                const FLAG_INVERSE: u8 = 16;
                const FLAG_BLINK: u8 = 32;
                const FLAG_HIDDEN: u8 = 64;
                const FLAG_STRIKETHROUGH: u8 = 128;

                // If the pane is squelched (hiding injected commands),
                // return a blank leaf so the client never sees the flash.
                // Squelch is lifted when the vt100 parser detects CSI 2J
                // (screen clear from cls/clear), or when the safety
                // timeout expires (fallback for unusual shells).
                if p.squelch_until.is_some() {
                    // Check if the sentinel has arrived in the parser.
                    let sentinel_arrived = p.term.lock()
                        .map(|mut parser| parser.screen_mut().take_squelch_cleared())
                        .unwrap_or(false);
                    if sentinel_arrived {
                        // Sentinel received: cd+cls finished, show the pane.
                        p.squelch_until = None;
                    } else if p.squelch_until.map_or(false, |d| std::time::Instant::now() < d) {
                        // Still waiting: return blank frame.
                        return LayoutJson::Leaf {
                            id: p.id, rows: p.last_rows, cols: p.last_cols,
                            cursor_row: 0, cursor_col: 0, alternate_screen: false,
                            hide_cursor: true,
                            cursor_shape: 0,
                            active: *cur_path == active_path, copy_mode: false,
                            scroll_offset: 0,
                            sel_start_row: None, sel_start_col: None,
                            sel_end_row: None, sel_end_col: None,
                            sel_mode: None,
                            copy_cursor_row: None, copy_cursor_col: None,
                            content: vec![], rows_v2: vec![], title: None,
                        };
                    } else {
                        // Safety timeout expired without sentinel; unsquelch anyway.
                        p.squelch_until = None;
                    }
                }

                let Ok(parser) = p.term.lock() else {
                    return LayoutJson::Leaf {
                        id: p.id, rows: p.last_rows, cols: p.last_cols,
                        cursor_row: 0, cursor_col: 0, alternate_screen: false,
                        hide_cursor: false,
                        cursor_shape: p.cursor_shape.load(std::sync::atomic::Ordering::Relaxed),
                        active: *cur_path == active_path, copy_mode: false,
                        scroll_offset: 0,
                        sel_start_row: None, sel_start_col: None,
                        sel_end_row: None, sel_end_col: None,
                        sel_mode: None,
                        copy_cursor_row: None, copy_cursor_col: None,
                        content: vec![], rows_v2: vec![], title: None,
                    };
                };
                let screen = parser.screen();
                let (cr, cc) = screen.cursor_position();
                let hide_cursor_flag = screen.hide_cursor();
                // ConPTY never passes through ESC[?1049h, so alternate_screen()
                // is always false.  Use a heuristic instead: if the last row of
                // the screen has non-blank content, this is a fullscreen TUI app.
                let alternate_screen = screen.alternate_screen() || {
                    let last_row = p.last_rows.saturating_sub(1);
                    let mut has_content = false;
                    for col in 0..p.last_cols {
                        if let Some(cell) = screen.cell(last_row, col) {
                            let t = cell.contents();
                            if !t.is_empty() && t != " " {
                                has_content = true;
                                break;
                            }
                        }
                    }
                    has_content
                };
                let need_full_content = include_full_content && *cur_path == active_path;
                let mut lines: Vec<Vec<CellJson>> = if need_full_content {
                    Vec::with_capacity(p.last_rows as usize)
                } else {
                    Vec::new()
                };
                let mut rows_v2: Vec<RowRunsJson> = Vec::with_capacity(p.last_rows as usize);
                for r in 0..p.last_rows {
                    let mut row: Vec<CellJson> = if need_full_content {
                        Vec::with_capacity(p.last_cols as usize)
                    } else {
                        Vec::new()
                    };
                    let mut runs: Vec<CellRunJson> = Vec::new();
                    let mut c = 0;
                    // Track previous cell's raw color enums for run-merging
                    // without allocating strings on every cell.
                    let mut prev_fg_raw: Option<vt100::Color> = None;
                    let mut prev_bg_raw: Option<vt100::Color> = None;
                    let mut prev_flags: u8 = 0;
                    while c < p.last_cols {
                        // Process each cell inline to avoid per-cell String allocation.
                        // The &str from cell.contents() can only be used inside the
                        // if-let block (borrows from parser), so run-merging happens
                        // here too — push_str(&str) avoids allocation for merged cells.
                        let (width, cell_fg_raw, cell_bg_raw, flags) = if let Some(cell) = screen.cell(r, c) {
                            let t = cell.contents();
                            let t = if t.is_empty() { " " } else { t };
                            let cell_fg = cell.fgcolor();
                            let cell_bg = cell.bgcolor();
                            let mut w = UnicodeWidthStr::width(t) as u16;
                            if w == 0 { w = 1; }
                            let mut fl = 0u8;
                            if cell.dim() { fl |= FLAG_DIM; }
                            if cell.bold() { fl |= FLAG_BOLD; }
                            if cell.italic() { fl |= FLAG_ITALIC; }
                            if cell.underline() { fl |= FLAG_UNDERLINE; }
                            if cell.inverse() { fl |= FLAG_INVERSE; }
                            if cell.blink() { fl |= FLAG_BLINK; }
                            if cell.hidden() { fl |= FLAG_HIDDEN; }
                            if cell.strikethrough() { fl |= FLAG_STRIKETHROUGH; }

                            // Run merging — push &str directly, no String allocation
                            let merged = if let Some(last) = runs.last_mut() {
                                if prev_fg_raw == Some(cell_fg) && prev_bg_raw == Some(cell_bg) && prev_flags == fl {
                                    last.text.push_str(t);
                                    last.width = last.width.saturating_add(w);
                                    true
                                } else { false }
                            } else { false };
                            if !merged {
                                let fg = crate::util::color_to_name(cell_fg);
                                let bg = crate::util::color_to_name(cell_bg);
                                runs.push(CellRunJson { text: t.to_string(), fg: fg.into_owned(), bg: bg.into_owned(), flags: fl, width: w });
                            }

                            if need_full_content {
                                let fg_str = crate::util::color_to_name(cell_fg).into_owned();
                                let bg_str = crate::util::color_to_name(cell_bg).into_owned();
                                row.push(CellJson {
                                    text: t.to_string(), fg: fg_str.clone(), bg: bg_str.clone(),
                                    bold: cell.bold(), italic: cell.italic(),
                                    underline: cell.underline(), inverse: cell.inverse(), dim: cell.dim(),
                                    blink: cell.blink(), hidden: cell.hidden(), strikethrough: cell.strikethrough(),
                                });
                                for _ in 1..w {
                                    row.push(CellJson {
                                        text: String::new(), fg: fg_str.clone(), bg: bg_str.clone(),
                                        bold: cell.bold(), italic: cell.italic(),
                                        underline: cell.underline(), inverse: cell.inverse(), dim: cell.dim(),
                                        blink: cell.blink(), hidden: cell.hidden(), strikethrough: cell.strikethrough(),
                                    });
                                }
                            }

                            (w, cell_fg, cell_bg, fl)
                        } else {
                            // No cell — default space
                            let merged = if let Some(last) = runs.last_mut() {
                                if prev_fg_raw == Some(vt100::Color::Default) && prev_bg_raw == Some(vt100::Color::Default) && prev_flags == 0 {
                                    last.text.push(' ');
                                    last.width = last.width.saturating_add(1);
                                    true
                                } else { false }
                            } else { false };
                            if !merged {
                                runs.push(CellRunJson { text: " ".to_string(), fg: "default".to_string(), bg: "default".to_string(), flags: 0, width: 1 });
                            }
                            if need_full_content {
                                row.push(CellJson {
                                    text: " ".to_string(), fg: "default".to_string(), bg: "default".to_string(),
                                    bold: false, italic: false, underline: false, inverse: false, dim: false,
                                    blink: false, hidden: false, strikethrough: false,
                                });
                            }
                            (1u16, vt100::Color::Default, vt100::Color::Default, 0u8)
                        };
                        prev_fg_raw = Some(cell_fg_raw);
                        prev_bg_raw = Some(cell_bg_raw);
                        prev_flags = flags;
                        c = c.saturating_add(width.max(1));
                    }
                    if need_full_content {
                        while row.len() < p.last_cols as usize {
                            row.push(CellJson {
                                text: " ".to_string(),
                                fg: "default".to_string(),
                                bg: "default".to_string(),
                                bold: false,
                                italic: false,
                                underline: false,
                                inverse: false,
                                dim: false,
                                blink: false,
                                hidden: false,
                                strikethrough: false,
                            });
                        }
                        lines.push(row);
                    }
                    rows_v2.push(RowRunsJson { runs });
                }
                LayoutJson::Leaf {
                    id: p.id,
                    rows: p.last_rows,
                    cols: p.last_cols,
                    cursor_row: cr,
                    cursor_col: cc,
                    alternate_screen,
                    hide_cursor: hide_cursor_flag,
                    cursor_shape: p.cursor_shape.load(std::sync::atomic::Ordering::Relaxed),
                    active: false,
                    copy_mode: false,
                    scroll_offset: 0,
                    sel_start_row: None,
                    sel_start_col: None,
                    sel_end_row: None,
                    sel_end_col: None,
                    sel_mode: None,
                    copy_cursor_row: None,
                    copy_cursor_col: None,
                    content: lines,
                    rows_v2,
                    title: if p.title.is_empty() { None } else { Some(p.title.clone()) },
                }
            }
        }
    }
    let win = &mut app.windows[app.active_idx];
    let mut path = Vec::new();
    let mut root = build(&mut win.root, &mut path, &win.active_path, in_copy_mode);
    // Mark the active pane and set copy mode info
    fn mark_active(
        node: &mut LayoutJson,
        path: &[usize],
        idx: usize,
        in_copy_mode: bool,
        scroll_offset: usize,
        copy_anchor: Option<(u16, u16)>,
        copy_pos: Option<(u16, u16)>,
    ) {
        match node {
            LayoutJson::Leaf {
                active,
                copy_mode,
                scroll_offset: so,
                sel_start_row,
                sel_start_col,
                sel_end_row,
                sel_end_col,
                copy_cursor_row,
                copy_cursor_col,
                ..
            } => {
                let is_active = idx >= path.len();
                *active = is_active;
                if is_active {
                    *copy_mode = in_copy_mode;
                    *so = scroll_offset;
                    if in_copy_mode {
                        if let Some((pr, pc)) = copy_pos {
                            *copy_cursor_row = Some(pr);
                            *copy_cursor_col = Some(pc);
                        } else {
                            *copy_cursor_row = None;
                            *copy_cursor_col = None;
                        }
                        if let (Some((ar, ac)), Some((pr, pc))) = (copy_anchor, copy_pos) {
                            *sel_start_row = Some(ar.min(pr));
                            *sel_start_col = Some(ac.min(pc));
                            *sel_end_row = Some(ar.max(pr));
                            *sel_end_col = Some(ac.max(pc));
                        } else {
                            *sel_start_row = None;
                            *sel_start_col = None;
                            *sel_end_row = None;
                            *sel_end_col = None;
                        }
                    } else {
                        *sel_start_row = None;
                        *sel_start_col = None;
                        *sel_end_row = None;
                        *sel_end_col = None;
                        *copy_cursor_row = None;
                        *copy_cursor_col = None;
                    }
                }
            }
            LayoutJson::Split { children, .. } => {
                if idx < path.len() {
                    if let Some(child) = children.get_mut(path[idx]) {
                        mark_active(child, path, idx + 1, in_copy_mode, scroll_offset, copy_anchor, copy_pos);
                    }
                }
            }
        }
    }
    mark_active(
        &mut root,
        &win.active_path,
        0,
        in_copy_mode,
        scroll_offset,
        app.copy_anchor,
        app.copy_pos,
    );
    let s = serde_json::to_string(&root).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("json error: {e}")))?;
    Ok(s)
}
