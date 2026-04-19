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

#[derive(Clone, Serialize, Deserialize)]
pub struct CellRunJson {
    pub text: String,
    pub fg: String,
    pub bg: String,
    pub flags: u8,
    pub width: u16,
}

#[derive(Clone, Serialize, Deserialize)]
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
