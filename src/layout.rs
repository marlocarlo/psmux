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
pub fn serialize_screen_rows(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<RowRunsJson> {
    const FLAG_DIM: u8 = 1;
    const FLAG_BOLD: u8 = 2;
    const FLAG_ITALIC: u8 = 4;
    const FLAG_UNDERLINE: u8 = 8;
    const FLAG_INVERSE: u8 = 16;
    const FLAG_BLINK: u8 = 32;
    const FLAG_HIDDEN: u8 = 64;
    const FLAG_STRIKETHROUGH: u8 = 128;

    let mut result: Vec<RowRunsJson> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut runs: Vec<CellRunJson> = Vec::new();
        let mut c: u16 = 0;
        let mut prev_fg_raw: Option<vt100::Color> = None;
        let mut prev_bg_raw: Option<vt100::Color> = None;
        let mut prev_flags: u8 = 0;
        while c < cols {
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

                (w, cell_fg, cell_bg, fl)
            } else {
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
                (1u16, vt100::Color::Default, vt100::Color::Default, 0u8)
            };
            prev_fg_raw = Some(cell_fg_raw);
            prev_bg_raw = Some(cell_bg_raw);
            prev_flags = flags;
            c = c.saturating_add(width.max(1));
        }
        result.push(RowRunsJson { runs });
    }
    result
}

pub fn cycle_top_layout(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    // toggle parent of active path, else toggle root
    if !win.active_path.is_empty() {
        let parent_path = &win.active_path[..win.active_path.len()-1].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path.to_vec()) {
            *kind = match *kind { LayoutKind::Horizontal => LayoutKind::Vertical, LayoutKind::Vertical => LayoutKind::Horizontal };
            *sizes = vec![50,50];
        }
    } else {
        if let Node::Split { kind, sizes, .. } = &mut win.root { *kind = match *kind { LayoutKind::Horizontal => LayoutKind::Vertical, LayoutKind::Vertical => LayoutKind::Horizontal }; *sizes = vec![50,50]; }
    }
}

#[derive(Serialize, Deserialize)]
pub struct CellJson { pub text: String, pub fg: String, pub bg: String, pub bold: bool, pub italic: bool, pub underline: bool, pub inverse: bool, pub dim: bool, pub blink: bool, pub hidden: bool, pub strikethrough: bool }

#[derive(Serialize, Deserialize)]
pub struct CellRunJson {
    pub text: String,
    pub fg: String,
    pub bg: String,
    pub flags: u8,
    pub width: u16,
}

#[derive(Serialize, Deserialize)]
pub struct RowRunsJson {
    pub runs: Vec<CellRunJson>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LayoutJson {
    #[serde(rename = "split")]
    Split { kind: String, sizes: Vec<u16>, children: Vec<LayoutJson> },
    #[serde(rename = "leaf")]
    Leaf {
        id: usize,
        rows: u16,
        cols: u16,
        cursor_row: u16,
        cursor_col: u16,
        #[serde(default)]
        alternate_screen: bool,
        #[serde(default)]
        hide_cursor: bool,
        #[serde(default)]
        cursor_shape: u8,
        active: bool,
        copy_mode: bool,
        scroll_offset: usize,
        sel_start_row: Option<u16>,
        sel_start_col: Option<u16>,
        sel_end_row: Option<u16>,
        sel_end_col: Option<u16>,
        #[serde(default)]
        sel_mode: Option<String>,
        #[serde(default)]
        copy_cursor_row: Option<u16>,
        #[serde(default)]
        copy_cursor_col: Option<u16>,
        #[serde(default)]
        content: Vec<Vec<CellJson>>,
        #[serde(default)]
        rows_v2: Vec<RowRunsJson>,
        /// Pane title for border label expansion
        #[serde(default)]
        title: Option<String>,
    },
}

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

    // ── tiny helpers (no captures needed, so plain `fn` items) ───────

    /// Append the JSON-escaped form of `s` into `out`.
    fn json_esc(s: &str, out: &mut String) {
        // Fast path – most cell text needs no escaping.
        if !s.bytes().any(|b| b == b'"' || b == b'\\' || b < 0x20) {
            out.push_str(s);
            return;
        }
        for ch in s.chars() {
            match ch {
                '"'  => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 => {
                    let _ = std::fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
    }

    /// Append a `vt100::Color` as its JSON string value (**no** surrounding quotes).
    fn push_color(c: vt100::Color, out: &mut String) {
        match c {
            vt100::Color::Default => out.push_str("default"),
            vt100::Color::Idx(i) => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("idx:{}", i));
            }
            vt100::Color::Rgb(r, g, b) => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("rgb:{},{},{}", r, g, b));
            }
        }
    }

    /// Close the currently-open run: closing `"` for text, then fg/bg/flags/width, then `}`.
    fn close_run(fg: vt100::Color, bg: vt100::Color, fl: u8, w: u16, out: &mut String) {
        out.push_str("\",\"fg\":\"");
        push_color(fg, out);
        out.push_str("\",\"bg\":\"");
        push_color(bg, out);
        let _ = std::fmt::Write::write_fmt(out, format_args!("\",\"flags\":{},\"width\":{}}}", fl, w));
    }

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

/// Apply a named layout to the current window.
/// Collects ALL leaf panes and rebuilds the tree structure from scratch.
pub fn apply_layout(app: &mut AppState, layout: &str) {
    let win = &mut app.windows[app.active_idx];
    
    // Collect all leaf panes from the current tree
    let old_root = std::mem::replace(&mut win.root, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
    let mut leaves = crate::tree::collect_leaves(old_root);
    let pane_count = leaves.len();
    if pane_count < 2 {
        // Put back the single leaf (or empty)
        if let Some(leaf) = leaves.into_iter().next() {
            win.root = leaf;
        }
        return;
    }

    // Helper: compute equal sizes summing to 100
    fn equal_sizes(n: usize) -> Vec<u16> {
        if n == 0 { return vec![]; }
        let base = 100 / n as u16;
        let mut sizes = vec![base; n];
        let rem = 100 - base * n as u16;
        if let Some(last) = sizes.last_mut() { *last += rem; }
        sizes
    }

    // Determine main-pane percentage
    let main_h_pct = if app.main_pane_height > 0 { app.main_pane_height.min(95) } else { 60 };
    let main_v_pct = if app.main_pane_width > 0 { app.main_pane_width.min(95) } else { 60 };

    match layout.to_lowercase().as_str() {
        "even-horizontal" | "even-h" => {
            // Single horizontal split with N equal children
            let sizes = equal_sizes(pane_count);
            win.root = Node::Split { kind: LayoutKind::Horizontal, sizes, children: leaves };
        }
        "even-vertical" | "even-v" => {
            // Single vertical split with N equal children
            let sizes = equal_sizes(pane_count);
            win.root = Node::Split { kind: LayoutKind::Vertical, sizes, children: leaves };
        }
        "main-horizontal" | "main-h" => {
            // Vertical split: top pane (main) + bottom horizontal split of remaining
            let main_pane = leaves.remove(0);
            if leaves.len() == 1 {
                let other = leaves.remove(0);
                win.root = Node::Split {
                    kind: LayoutKind::Vertical,
                    sizes: vec![main_h_pct, 100 - main_h_pct],
                    children: vec![main_pane, other],
                };
            } else {
                let bottom_sizes = equal_sizes(leaves.len());
                let bottom = Node::Split { kind: LayoutKind::Horizontal, sizes: bottom_sizes, children: leaves };
                win.root = Node::Split {
                    kind: LayoutKind::Vertical,
                    sizes: vec![main_h_pct, 100 - main_h_pct],
                    children: vec![main_pane, bottom],
                };
            }
        }
        "main-vertical" | "main-v" => {
            // Horizontal split: left pane (main) + right vertical split of remaining
            let main_pane = leaves.remove(0);
            if leaves.len() == 1 {
                let other = leaves.remove(0);
                win.root = Node::Split {
                    kind: LayoutKind::Horizontal,
                    sizes: vec![main_v_pct, 100 - main_v_pct],
                    children: vec![main_pane, other],
                };
            } else {
                let right_sizes = equal_sizes(leaves.len());
                let right = Node::Split { kind: LayoutKind::Vertical, sizes: right_sizes, children: leaves };
                win.root = Node::Split {
                    kind: LayoutKind::Horizontal,
                    sizes: vec![main_v_pct, 100 - main_v_pct],
                    children: vec![main_pane, right],
                };
            }
        }
        "tiled" => {
            // Balanced binary tree of splits
            fn build_tiled(mut panes: Vec<Node>) -> Node {
                if panes.len() == 1 { return panes.remove(0); }
                if panes.len() == 2 {
                    return Node::Split {
                        kind: LayoutKind::Horizontal,
                        sizes: vec![50, 50],
                        children: panes,
                    };
                }
                let mid = panes.len() / 2;
                let right_panes = panes.split_off(mid);
                let left = build_tiled(panes);
                let right = build_tiled(right_panes);
                // Alternate between vertical and horizontal at each level
                Node::Split {
                    kind: LayoutKind::Vertical,
                    sizes: vec![50, 50],
                    children: vec![left, right],
                }
            }
            win.root = build_tiled(leaves);
        }
        _ => {
            // Unknown layout name — try to parse as tmux layout string
            let new_root = parse_tmux_layout_string(layout, &mut leaves);
            if let Some(root) = new_root {
                win.root = root;
            } else {
                // Parsing failed; put panes back as even-horizontal fallback
                let sizes = equal_sizes(pane_count);
                win.root = Node::Split { kind: LayoutKind::Horizontal, sizes, children: leaves };
            }
        }
    }
    // Reset active_path to first leaf
    win.active_path = crate::tree::first_leaf_path(&win.root);
}

const LAYOUT_NAMES: [&str; 5] = ["even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled"];

/// Cycle through available layouts (forward)
pub fn cycle_layout(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if matches!(win.root, Node::Leaf(_)) { return; }
    let next_idx = (win.layout_index + 1) % LAYOUT_NAMES.len();
    win.layout_index = next_idx;
    apply_layout(app, LAYOUT_NAMES[next_idx]);
}

/// Cycle through available layouts (reverse)
pub fn cycle_layout_reverse(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if matches!(win.root, Node::Leaf(_)) { return; }
    let prev_idx = (win.layout_index + LAYOUT_NAMES.len() - 1) % LAYOUT_NAMES.len();
    win.layout_index = prev_idx;
    apply_layout(app, LAYOUT_NAMES[prev_idx]);
}

/// Parse a tmux layout string into a Node tree.
///
/// Format: `checksum,WxH,X,Y{child1,child2,...}` or `checksum,WxH,X,Y[child1,child2,...]`
/// - `{...}` = horizontal split (children side-by-side)
/// - `[...]` = vertical split (children stacked)
/// - Each child is either a leaf `WxH,X,Y,pane_id` or a nested split `WxH,X,Y{...}` / `WxH,X,Y[...]`
///
/// The `panes` vec provides existing pane nodes to fill the tree leaves.
/// Returns `None` if parsing fails.
/// Parsed layout node from a tmux layout string.
/// This is a layout descriptor that can be inspected, counted, and applied
/// to existing panes without requiring pane objects during parsing.
#[derive(Debug, Clone)]
pub enum LayoutNode {
    Leaf { width: u16, height: u16, x: u16, y: u16, pane_id: Option<usize> },
    Split { kind: LayoutKind, width: u16, height: u16, x: u16, y: u16, children: Vec<LayoutNode> },
}

impl LayoutNode {
    /// Count the number of leaf panes in this layout tree.
    pub fn count_leaves(&self) -> usize {
        match self {
            LayoutNode::Leaf { .. } => 1,
            LayoutNode::Split { children, .. } => children.iter().map(|c| c.count_leaves()).sum(),
        }
    }

    fn width(&self) -> u16 {
        match self { LayoutNode::Leaf { width, .. } | LayoutNode::Split { width, .. } => *width }
    }

    fn height(&self) -> u16 {
        match self { LayoutNode::Leaf { height, .. } | LayoutNode::Split { height, .. } => *height }
    }
}

/// Parse a tmux layout string into a `LayoutNode` descriptor tree.
///
/// Layout string format: `CHECKSUM,WxH,X,Y{...}` or `[...]` or `,PANE_ID`
/// The 4-hex-digit checksum prefix is skipped.
pub fn parse_layout_string(layout_str: &str) -> Option<LayoutNode> {
    let s = layout_str.trim();
    if s.len() < 5 { return None; }
    // Validate and skip the 4-hex-char checksum prefix followed by comma.
    // tmux checksums are exactly 4 hex digits (e.g. "5e08,").
    let bytes = s.as_bytes();
    if bytes.len() < 5 || bytes[4] != b',' { return None; }
    for &b in &bytes[..4] {
        if !b.is_ascii_hexdigit() { return None; }
    }
    let body = &s[5..];
    let (node, _) = parse_layout_node(body)?;
    Some(node)
}

/// Parse a tmux layout string into a Node tree using existing panes.
///
/// Parses the layout string into a LayoutNode descriptor, then converts
/// it to a Node tree by assigning panes from the provided vec in leaf order.
/// Returns `None` if parsing fails or there aren't enough panes.
pub fn parse_tmux_layout_string(layout_str: &str, panes: &mut Vec<Node>) -> Option<Node> {
    let layout = parse_layout_string(layout_str)?;
    layout_node_to_node(&layout, panes)
}

/// Convert a LayoutNode descriptor tree into a Node tree,
/// consuming panes from the vec in left-to-right leaf order.
fn layout_node_to_node(layout: &LayoutNode, panes: &mut Vec<Node>) -> Option<Node> {
    match layout {
        LayoutNode::Leaf { .. } => {
            if panes.is_empty() { return None; }
            Some(panes.remove(0))
        }
        LayoutNode::Split { kind, children, .. } => {
            let total_size: u32 = match kind {
                LayoutKind::Horizontal => children.iter().map(|c| c.width() as u32).sum(),
                LayoutKind::Vertical => children.iter().map(|c| c.height() as u32).sum(),
            };
            let sizes: Vec<u16> = if total_size == 0 {
                let n = children.len().max(1) as u16;
                vec![100 / n; children.len()]
            } else {
                let mut szs: Vec<u16> = children.iter().map(|c| {
                    let dim = match kind {
                        LayoutKind::Horizontal => c.width() as u32,
                        LayoutKind::Vertical => c.height() as u32,
                    };
                    (dim * 100 / total_size) as u16
                }).collect();
                let sum: u16 = szs.iter().sum();
                if sum < 100 { if let Some(last) = szs.last_mut() { *last += 100 - sum; } }
                szs
            };
            let mut nodes = Vec::with_capacity(children.len());
            for child in children {
                nodes.push(layout_node_to_node(child, panes)?);
            }
            Some(Node::Split { kind: *kind, sizes, children: nodes })
        }
    }
}

/// Parse a single layout node from position in the string, returns (LayoutNode, chars_consumed).
fn parse_layout_node(s: &str) -> Option<(LayoutNode, usize)> {
    let (w, h, x, y, consumed_dims) = parse_dimensions(s)?;
    let rest = &s[consumed_dims..];

    if rest.starts_with('{') {
        // Horizontal split (children side-by-side)
        let (children, consumed_bracket) = parse_layout_children(&rest[1..], '}')?;
        Some((
            LayoutNode::Split { kind: LayoutKind::Horizontal, width: w, height: h, x, y, children },
            consumed_dims + 1 + consumed_bracket,
        ))
    } else if rest.starts_with('[') {
        // Vertical split (children stacked top/bottom)
        let (children, consumed_bracket) = parse_layout_children(&rest[1..], ']')?;
        Some((
            LayoutNode::Split { kind: LayoutKind::Vertical, width: w, height: h, x, y, children },
            consumed_dims + 1 + consumed_bracket,
        ))
    } else {
        // Leaf node; may have ,pane_id suffix
        let mut extra = 0;
        let mut pane_id = None;
        if rest.starts_with(',') {
            let id_str = &rest[1..];
            let end = id_str.find(|c: char| c == ',' || c == '{' || c == '[' || c == '}' || c == ']')
                .unwrap_or(id_str.len());
            pane_id = id_str[..end].parse::<usize>().ok();
            extra = 1 + end;
        }
        Some((
            LayoutNode::Leaf { width: w, height: h, x, y, pane_id },
            consumed_dims + extra,
        ))
    }
}

/// Parse WxH,X,Y returning (width, height, x, y, chars_consumed).
fn parse_dimensions(s: &str) -> Option<(u16, u16, u16, u16, usize)> {
    let x_pos = s.find('x')?;
    let w: u16 = s[..x_pos].parse().ok()?;
    let after_x = &s[x_pos + 1..];
    let comma1 = after_x.find(',')?;
    let h: u16 = after_x[..comma1].parse().ok()?;
    let after_h = &after_x[comma1 + 1..];
    let comma2 = after_h.find(',')?;
    let xc: u16 = after_h[..comma2].parse().ok()?;
    let after_xcoord = &after_h[comma2 + 1..];
    let y_end = after_xcoord.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_xcoord.len());
    let yc: u16 = after_xcoord[..y_end].parse().ok()?;
    let total = x_pos + 1 + comma1 + 1 + comma2 + 1 + y_end;
    Some((w, h, xc, yc, total))
}

/// Parse comma-separated layout children inside brackets.
/// Returns vec of LayoutNode and total chars consumed including closing bracket.
fn parse_layout_children(s: &str, closing: char) -> Option<(Vec<LayoutNode>, usize)> {
    let mut children = Vec::new();
    let mut pos = 0;

    loop {
        if pos >= s.len() { return None; }
        if s.as_bytes()[pos] == closing as u8 {
            pos += 1;
            break;
        }
        if !children.is_empty() {
            if s.as_bytes().get(pos).copied() == Some(b',') {
                pos += 1;
            }
        }
        let child_str = &s[pos..];
        let (node, consumed) = parse_layout_node(child_str)?;
        children.push(node);
        pos += consumed;
    }

    Some((children, pos))
}

#[cfg(test)]
#[path = "../tests-rs/test_layout.rs"]
mod test_layout;
