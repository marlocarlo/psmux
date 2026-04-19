//! TUI rendering — pane tree rendering, separator drawing, cursor positioning.
//!
//! Style/color parsing is in `style.rs`; this module re-exports it for
//! backward compatibility so `use crate::rendering::*` still works.

use std::io::{self, Write};
use std::env;
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::style::{Style, Modifier};
use unicode_width::UnicodeWidthStr;
use crossterm::style::Print;
use crossterm::execute;
use portable_pty::PtySize;

use crate::types::{AppState, Mode, Node, LayoutKind};
use crate::tree::split_with_gaps;

// Re-export style utilities so existing `use crate::rendering::*` still works.
pub use crate::style::{
    map_color, parse_tmux_style, parse_inline_styles,
};

// ─── VT color helpers ───────────────────────────────────────────────────────

pub fn vt_to_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        // Map the 16 standard colors to ratatui named variants so that
        // dim_color() can distinguish individual hues when dimming
        // prediction text.  Note: crossterm 0.29 serialises ALL named
        // colors as 38;5;N (256-color indexed), so the outer terminal
        // sees the same bytes as Color::Indexed(n).
        vt100::Color::Idx(0) => Color::Black,
        vt100::Color::Idx(1) => Color::Red,
        vt100::Color::Idx(2) => Color::Green,
        vt100::Color::Idx(3) => Color::Yellow,
        vt100::Color::Idx(4) => Color::Blue,
        vt100::Color::Idx(5) => Color::Magenta,
        vt100::Color::Idx(6) => Color::Cyan,
        vt100::Color::Idx(7) => Color::Gray,       // index 7 = light gray (SGR 37)
        vt100::Color::Idx(8) => Color::DarkGray,
        vt100::Color::Idx(9) => Color::LightRed,
        vt100::Color::Idx(10) => Color::LightGreen,
        vt100::Color::Idx(11) => Color::LightYellow,
        vt100::Color::Idx(12) => Color::LightBlue,
        vt100::Color::Idx(13) => Color::LightMagenta,
        vt100::Color::Idx(14) => Color::LightCyan,
        vt100::Color::Idx(15) => Color::White,     // index 15 = bright white (SGR 97)
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn dim_color(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb((r as u16 * 2 / 5) as u8, (g as u16 * 2 / 5) as u8, (b as u16 * 2 / 5) as u8),
        Color::Black => Color::Rgb(40, 40, 40),
        Color::White | Color::Gray | Color::DarkGray => Color::Rgb(100, 100, 100),
        Color::LightRed => Color::Rgb(150, 80, 80),
        Color::LightGreen => Color::Rgb(80, 150, 80),
        Color::LightYellow => Color::Rgb(150, 150, 80),
        Color::LightBlue => Color::Rgb(80, 120, 180),
        Color::LightMagenta => Color::Rgb(150, 80, 150),
        Color::LightCyan => Color::Rgb(80, 150, 150),
        _ => Color::Rgb(80, 80, 80),
    }
}

pub fn dim_predictions_enabled() -> bool {
    std::env::var("PSMUX_DIM_PREDICTIONS").map(|v| v == "1" || v.to_lowercase() == "true").unwrap_or(false)
}

// ─── Cursor ─────────────────────────────────────────────────────────────────

/// Returns `true` when ConPTY passthrough mode is available (Windows 11 22H2+,
/// build ≥ 22621).  Cached after the first call.
///
/// On Windows 10 (classic ConPTY without passthrough), the child's CSI ?25h
/// (show cursor) is often lost or delayed by the translation layer, which
/// makes the vt100 parser's `hide_cursor` flag unreliable — it gets stuck on
/// `true`.  We only trust `hide_cursor` when passthrough mode is active.
pub fn has_conpty_passthrough() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        crate::ssh_input::windows_build_number()
            .map(|b| b >= 22621)
            .unwrap_or(false)
    })
}

/// Resolve the DECSCUSR code (0-6) from the PSMUX_CURSOR_STYLE / PSMUX_CURSOR_BLINK
/// configuration.  Returns 0 ("default") when no explicit style is configured.
///
/// Used as the fallback cursor shape when ConPTY doesn't forward DECSCUSR from
/// the child process (Windows 10 without passthrough mode).
pub fn configured_cursor_code() -> u8 {
    let style = env::var("PSMUX_CURSOR_STYLE").unwrap_or_else(|_| "bar".to_string());
    let blink = env::var("PSMUX_CURSOR_BLINK").unwrap_or_else(|_| "1".to_string()) != "0";
    match style.as_str() {
        "block" => if blink { 1 } else { 2 },
        "underline" => if blink { 3 } else { 4 },
        "bar" | "beam" => if blink { 5 } else { 6 },
        "default" => 0,
        _ => 0,
    }
}

pub fn apply_cursor_style<W: Write>(out: &mut W) -> io::Result<()> {
    let code = configured_cursor_code();
    execute!(out, Print(format!("\x1b[{} q", code)))?;
    Ok(())
}

// ─── Pane tree rendering ────────────────────────────────────────────────────

pub fn render_window(f: &mut Frame, app: &mut AppState, area: Rect) {
    let dim_preds = app.prediction_dimming;
    let border_style = parse_tmux_style(&app.pane_border_style);
    let active_border_style = parse_tmux_style(&app.pane_active_border_style);
    let copy_cursor = if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) { app.copy_pos } else { None };
    let window_style = app.user_options.get("window-style").map(|s| parse_tmux_style(s));
    let window_active_style = app.user_options.get("window-active-style").map(|s| parse_tmux_style(s));
    let border_status = app.user_options.get("pane-border-status").cloned().unwrap_or_else(|| "off".to_string());
    let border_format = app.user_options.get("pane-border-format").cloned().unwrap_or_default();
    let win = &mut app.windows[app.active_idx];
    let active_rect = compute_active_rect(&win.root, &win.active_path, area);
    render_node(f, &mut win.root, &win.active_path, &mut Vec::new(), area, dim_preds, border_style, active_border_style, copy_cursor, active_rect, window_style, window_active_style, &border_status, &border_format, &mut 0);
    fix_border_intersections(f.buffer_mut());
}

/// Post-pass: fix border intersection characters where horizontal and vertical
/// separator lines meet. Converts plain '│' and '─' to proper junction
/// characters ('┼', '├', '┤', '┬', '┴') at intersection points.
pub fn fix_border_intersections(buf: &mut Buffer) {
    let w = buf.area.width as usize;
    let h = buf.area.height as usize;
    if w == 0 || h == 0 { return; }

    // Collect fixes first so detection sees only original characters.
    let mut fixes: Vec<(usize, char)> = Vec::new();

    for row in 0..h {
        for col in 0..w {
            let idx = row * w + col;
            if idx >= buf.content.len() { continue; }
            let ch = buf.content[idx].symbol().chars().next().unwrap_or(' ');

            match ch {
                '│' => {
                    // Cell already has vertical (up+down). Check for horizontal neighbours.
                    let has_left = col > 0 && {
                        let li = row * w + (col - 1);
                        li < buf.content.len() && buf.content[li].symbol().chars().next() == Some('─')
                    };
                    let has_right = col + 1 < w && {
                        let ri = row * w + (col + 1);
                        ri < buf.content.len() && buf.content[ri].symbol().chars().next() == Some('─')
                    };
                    match (has_left, has_right) {
                        (true, true)  => fixes.push((idx, '┼')),
                        (true, false) => fixes.push((idx, '┤')),
                        (false, true) => fixes.push((idx, '├')),
                        _ => {}
                    }
                }
                '─' => {
                    // Cell already has horizontal (left+right). Check for vertical neighbours.
                    let has_up = row > 0 && {
                        let ui = (row - 1) * w + col;
                        ui < buf.content.len() && buf.content[ui].symbol().chars().next() == Some('│')
                    };
                    let has_down = row + 1 < h && {
                        let di = (row + 1) * w + col;
                        di < buf.content.len() && buf.content[di].symbol().chars().next() == Some('│')
                    };
                    match (has_up, has_down) {
                        (true, true)  => fixes.push((idx, '┼')),
                        (true, false) => fixes.push((idx, '┴')),
                        (false, true) => fixes.push((idx, '┬')),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    for (idx, ch) in fixes {
        buf.content[idx].set_char(ch);
    }
}

pub fn render_node(
    f: &mut Frame,
    node: &mut Node,
    active_path: &Vec<usize>,
    cur_path: &mut Vec<usize>,
    area: Rect,
    dim_preds: bool,
    border_style: Style,
    active_border_style: Style,
    copy_cursor: Option<(u16, u16)>,
    active_rect: Option<Rect>,
    window_style: Option<Style>,
    window_active_style: Option<Style>,
    border_status: &str,
    border_format: &str,
    pane_idx: &mut usize,
) {
    match node {
        Node::Leaf(pane) => {
            let is_active = *cur_path == *active_path;
            let inner = area;
            let target_rows = inner.height.max(1);
            let target_cols = inner.width.max(1);
            if pane.last_rows != target_rows || pane.last_cols != target_cols {
                let _ = pane.master.resize(PtySize { rows: target_rows, cols: target_cols, pixel_width: 0, pixel_height: 0 });
                if let Ok(mut parser) = pane.term.lock() {
                    parser.screen_mut().set_size(target_rows, target_cols);
                }
                pane.last_rows = target_rows;
                pane.last_cols = target_cols;
            }
            let parser_guard = pane.term.lock();
            let Ok(parser) = parser_guard else { return; };
            let screen = parser.screen();
            let (cur_r, cur_c) = screen.cursor_position();
            let mut lines: Vec<Line> = Vec::with_capacity(target_rows as usize);
            for r in 0..target_rows {
                let mut spans: Vec<Span> = Vec::with_capacity(target_cols as usize);
                let mut c = 0;
                while c < target_cols {
                    if let Some(cell) = screen.cell(r, c) {
                        let mut fg = vt_to_color(cell.fgcolor());
                        let mut bg = vt_to_color(cell.bgcolor());
                        // Apply window-style / window-active-style defaults for unset colors
                        let ws = if is_active { window_active_style } else { window_style };
                        if let Some(ws) = ws {
                            if fg == Color::Reset { if let Some(wfg) = ws.fg { fg = wfg; } }
                            if bg == Color::Reset { if let Some(wbg) = ws.bg { bg = wbg; } }
                        }
                        if dim_preds && !screen.alternate_screen()
                            && (r > cur_r || (r == cur_r && c >= cur_c))
                        {
                            fg = dim_color(fg);
                        }
                        let mut style = Style::default().fg(fg).bg(bg);
                        if cell.dim() { style = style.add_modifier(Modifier::DIM); }
                        if cell.bold() { style = style.add_modifier(Modifier::BOLD); }
                        if cell.italic() { style = style.add_modifier(Modifier::ITALIC); }
                        if cell.underline() { style = style.add_modifier(Modifier::UNDERLINED); }
                        if cell.inverse() { style = style.add_modifier(Modifier::REVERSED); }
                        if cell.blink() { style = style.add_modifier(Modifier::SLOW_BLINK); }
                        if cell.strikethrough() { style = style.add_modifier(Modifier::CROSSED_OUT); }
                        // ratatui-crossterm 0.1.0 omits SGR 8 from
                        // ModifierDiff, so Modifier::HIDDEN never
                        // reaches the terminal.  Render hidden cells
                        // as spaces instead.
                        let text = if cell.hidden() {
                            " ".to_string()
                        } else {
                            cell.contents().to_string()
                        };
                        let w = UnicodeWidthStr::width(text.as_str()) as u16;
                        if w == 0 {
                            spans.push(Span::styled(" ", style));
                            c += 1;
                        } else if w >= 2 {
                            // Wide char at the last column would overflow the pane boundary
                            if c + w > target_cols {
                                spans.push(Span::styled(" ", style));
                                c += 1;
                            } else {
                                spans.push(Span::styled(text, style));
                                c += 2;
                            }
                        } else {
                            spans.push(Span::styled(text, style));
                            c += 1;
                        }
                    } else {
                        spans.push(Span::raw(" "));
                        c += 1;
                    }
                }
                lines.push(Line::from(spans));
            }
            f.render_widget(Clear, inner);
            let para = Paragraph::new(Text::from(lines));
            f.render_widget(para, inner);
            if is_active {
                let (cr, cc) = copy_cursor.unwrap_or_else(|| screen.cursor_position());
                let cr = cr.min(target_rows.saturating_sub(1));
                let cc = cc.min(target_cols.saturating_sub(1));
                let cx = inner.x + cc;
                let cy = inner.y + cr;
                // Respect the child's cursor-visibility state.
                // TUI apps like Claude draw their own cursor via cell
                // inverse-video and hide the real terminal cursor —
                // honour that so we don't place a stray cursor at
                // ConPTY's parking position.
                if !screen.hide_cursor() {
                    f.set_cursor_position((cx, cy));
                }
            }
            // Pane border format/status overlay
            if border_status != "off" && !border_format.is_empty() && area.height > 1 {
                let pane_label = border_format.replace("#{pane_index}", &pane_idx.to_string())
                    .replace("#P", &pane_idx.to_string())
                    .replace("#{pane_title}", &pane.title);
                let label_width = UnicodeWidthStr::width(pane_label.as_str()) as u16;
                if label_width > 0 && area.width >= label_width {
                    let label_y = if border_status == "bottom" { area.y + area.height.saturating_sub(1) } else { area.y };
                    let label_area = Rect::new(area.x, label_y, label_width.min(area.width), 1);
                    let label_style = if is_active { active_border_style } else { border_style };
                    f.render_widget(Paragraph::new(Line::from(Span::styled(pane_label, label_style))), label_area);
                }
            }
            *pane_idx += 1;
        }
        Node::Split { kind, sizes, children } => {
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else { vec![100 / children.len().max(1) as u16; children.len()] };
            let is_horizontal = *kind == LayoutKind::Horizontal;
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
            for (i, child) in children.iter_mut().enumerate() {
                cur_path.push(i);
                if i < rects.len() {
                    render_node(f, child, active_path, cur_path, rects[i], dim_preds, border_style, active_border_style, copy_cursor, active_rect, window_style, window_active_style, border_status, border_format, pane_idx);
                }
                cur_path.pop();
            }
            // Draw separator lines
            let buf = f.buffer_mut();
            for i in 0..children.len().saturating_sub(1) {
                if i >= rects.len() { break; }
                let both_leaves = matches!(&children[i], Node::Leaf(_))
                    && matches!(children.get(i + 1), Some(Node::Leaf(_)));

                if is_horizontal {
                    let sep_x = rects[i].x + rects[i].width;
                    if sep_x < buf.area.x + buf.area.width {
                        if both_leaves {
                            let left_active = cur_path.len() < active_path.len()
                                && active_path[..cur_path.len()] == cur_path[..]
                                && active_path[cur_path.len()] == i;
                            let right_active = cur_path.len() < active_path.len()
                                && active_path[..cur_path.len()] == cur_path[..]
                                && active_path[cur_path.len()] == i + 1;
                            let left_sty = if left_active { active_border_style } else { border_style };
                            let right_sty = if right_active { active_border_style } else { border_style };
                            let mid_y = area.y + area.height / 2;
                            for y in area.y..area.y + area.height {
                                let sty = if y < mid_y { left_sty } else { right_sty };
                                let idx = (y - buf.area.y) as usize * buf.area.width as usize + (sep_x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('│');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        } else {
                            for y in area.y..area.y + area.height {
                                let active = active_rect.map_or(false, |ar| {
                                    y >= ar.y && y < ar.y + ar.height
                                    && (sep_x == ar.x + ar.width || sep_x + 1 == ar.x)
                                });
                                let sty = if active { active_border_style } else { border_style };
                                let idx = (y - buf.area.y) as usize * buf.area.width as usize + (sep_x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('│');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        }
                    }
                } else {
                    let sep_y = rects[i].y + rects[i].height;
                    if sep_y < buf.area.y + buf.area.height {
                        if both_leaves {
                            let top_active = cur_path.len() < active_path.len()
                                && active_path[..cur_path.len()] == cur_path[..]
                                && active_path[cur_path.len()] == i;
                            let bot_active = cur_path.len() < active_path.len()
                                && active_path[..cur_path.len()] == cur_path[..]
                                && active_path[cur_path.len()] == i + 1;
                            let top_sty = if top_active { active_border_style } else { border_style };
                            let bot_sty = if bot_active { active_border_style } else { border_style };
                            let mid_x = area.x + area.width / 2;
                            for x in area.x..area.x + area.width {
                                let sty = if x < mid_x { top_sty } else { bot_sty };
                                let idx = (sep_y - buf.area.y) as usize * buf.area.width as usize + (x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('─');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        } else {
                            for x in area.x..area.x + area.width {
                                let active = active_rect.map_or(false, |ar| {
                                    x >= ar.x && x < ar.x + ar.width
                                    && (sep_y == ar.y + ar.height || sep_y + 1 == ar.y)
                                });
                                let sty = if active { active_border_style } else { border_style };
                                let idx = (sep_y - buf.area.y) as usize * buf.area.width as usize + (x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('─');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ─── Layout helpers ─────────────────────────────────────────────────────────

/// Compute the rectangle of the active pane by following the active_path through the tree.
fn compute_active_rect(node: &Node, active_path: &[usize], area: Rect) -> Option<Rect> {
    compute_active_rect_pub(node, active_path, area)
}

/// Public version of `compute_active_rect` for use outside the rendering module
/// (e.g. accessibility caret updates).
pub fn compute_active_rect_pub(node: &Node, active_path: &[usize], area: Rect) -> Option<Rect> {
    match node {
        Node::Leaf(_) => Some(area),
        Node::Split { kind, sizes, children } => {
            if active_path.is_empty() || children.is_empty() { return None; }
            let idx = active_path[0];
            if idx >= children.len() { return None; }
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![100 / children.len().max(1) as u16; children.len()]
            };
            let is_horizontal = *kind == LayoutKind::Horizontal;
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
            if idx < rects.len() {
                compute_active_rect(&children[idx], &active_path[1..], rects[idx])
            } else {
                None
            }
        }
    }
}

// ─── Status bar convenience wrappers (delegate to style.rs) ─────────────────

/// Expand simple status variables using AppState context.
pub fn expand_status(fmt: &str, app: &AppState, time_str: &str) -> String {
    let window = &app.windows[app.active_idx];
    let win_idx = app.active_idx + app.window_base_index;
    crate::style::expand_status(fmt, &app.session_name, &window.name, win_idx, time_str)
}

/// Parse a status format string with AppState context into styled spans.
pub fn parse_status(fmt: &str, app: &AppState, time_str: &str) -> Vec<Span<'static>> {
    let window = &app.windows[app.active_idx];
    let win_idx = app.active_idx + app.window_base_index;
    crate::style::parse_status(fmt, &app.session_name, &window.name, win_idx, time_str)
}

// ─── UI layout helpers ──────────────────────────────────────────────────────

pub fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    // Clamp requested height to the available area so we never
    // produce a Rect that extends beyond the buffer.
    let clamped_h = height.min(r.height);
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(clamped_h),
            Constraint::Percentage(50),
        ])
        .split(r);
    let middle = popup_layout[1];
    let width = (middle.width * percent_x) / 100;
    let x = middle.x + (middle.width - width) / 2;
    // Use the Layout-allocated height, not the raw parameter,
    // to guarantee the rect stays within the parent area.
    let final_h = middle.height.min(clamped_h);
    Rect { x, y: middle.y, width, height: final_h }
}
