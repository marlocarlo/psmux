#[allow(unused_imports)]

use std::sync::{Arc, Mutex};

use crate::layout::serialize_screen_rows;
use crate::types::{Pane, AppState, Mode};

// ── Popup pane creation ─────────────────────────────────────────────

/// Spawn a PTY-backed `Pane` for use inside a popup overlay.
///
/// This reuses the same PTY infrastructure as regular panes (ConPTY,
/// vt100 parser, reader thread) but does NOT add the pane to any window
/// tree.  The returned `Pane` is stored inside `Mode::PopupMode`.
use super::*;

/// Render a popup overlay inside the TUI frame.
///
/// Used by the in-process (non-server) rendering path in `app.rs`.
/// Reads the popup pane's vt100 screen directly and renders with full
/// color/style support.
pub fn render_popup_overlay(
    f: &mut ratatui::Frame,
    area: ratatui::prelude::Rect,
    app: &AppState,
) {
    use ratatui::prelude::*;
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

    if let Mode::PopupMode {
        command,
        output,
        width,
        height,
        ref popup_pane,
        scroll_offset,
        ..
    } = &app.mode
    {
        let w = (*width).min(area.width.saturating_sub(4));
        let h = (*height).min(area.height.saturating_sub(4));
        let popup_area = Rect {
            x: (area.width.saturating_sub(w)) / 2,
            y: (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };

        let title = if command.is_empty() {
            "Popup"
        } else {
            command
        };
        let border_style = if let Some(style_str) = app.user_options.get("popup-border-style") {
            crate::style::parse_tmux_style(style_str)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let border_type = match app.user_options.get("popup-border-lines").map(|s| s.as_str()) {
            Some("double") => ratatui::widgets::BorderType::Double,
            Some("heavy") => ratatui::widgets::BorderType::Thick,
            Some("rounded") => ratatui::widgets::BorderType::Rounded,
            Some("none") | Some("simple") => ratatui::widgets::BorderType::Plain,
            _ => ratatui::widgets::BorderType::Plain,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(border_type)
            .title(title);

        let content = if let Some(pane) = popup_pane {
            if let Ok(parser) = pane.term.lock() {
                let screen = parser.screen();
                let inner_h = h.saturating_sub(2);
                let inner_w = w.saturating_sub(2);
                let mut lines: Vec<Line<'static>> = Vec::new();
                for row in 0..inner_h {
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    let mut current_text = String::new();
                    let mut current_style = Style::default();
                    for col in 0..inner_w {
                        if let Some(cell) = screen.cell(row, col) {
                            let mut style = Style::default();
                            style = style.fg(crate::rendering::vt_to_color(cell.fgcolor()));
                            style = style.bg(crate::rendering::vt_to_color(cell.bgcolor()));
                            if cell.dim() {
                                style = style.add_modifier(Modifier::DIM);
                            }
                            if cell.bold() {
                                style = style.add_modifier(Modifier::BOLD);
                            }
                            if cell.italic() {
                                style = style.add_modifier(Modifier::ITALIC);
                            }
                            if cell.underline() {
                                style = style.add_modifier(Modifier::UNDERLINED);
                            }
                            if cell.inverse() {
                                style = style.add_modifier(Modifier::REVERSED);
                            }
                            if cell.blink() {
                                style = style.add_modifier(Modifier::SLOW_BLINK);
                            }
                            if cell.strikethrough() {
                                style = style.add_modifier(Modifier::CROSSED_OUT);
                            }
                            // ratatui-crossterm 0.1.0 omits SGR 8, so
                            // Modifier::HIDDEN won't reach the terminal.
                            // Render hidden cells as spaces instead.
                            let ch = if cell.hidden() {
                                " ".to_string()
                            } else {
                                cell.contents().to_string()
                            };
                            if style != current_style {
                                if !current_text.is_empty() {
                                    spans.push(Span::styled(
                                        std::mem::take(&mut current_text),
                                        current_style,
                                    ));
                                }
                                current_style = style;
                            }
                            if ch.is_empty() {
                                current_text.push(' ');
                            } else {
                                current_text.push_str(&ch);
                            }
                        } else {
                            current_text.push(' ');
                        }
                    }
                    if !current_text.is_empty() {
                        spans.push(Span::styled(current_text, current_style));
                    }
                    lines.push(Line::from(spans));
                }
                Text::from(lines)
            } else {
                Text::from(output.as_str())
            }
        } else {
            Text::from(output.as_str())
        };

        let para = Paragraph::new(content)
            .block(block)
            .scroll((*scroll_offset, 0));

        f.render_widget(Clear, popup_area);
        f.render_widget(para, popup_area);
    }
}
