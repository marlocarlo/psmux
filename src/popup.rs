//! Popup overlay module.
//!
//! A popup is a **Pane rendered as a floating overlay**, not part of the
//! window tree.  By storing an actual `Pane` inside `PopupMode`, the popup
//! inherits all pane infrastructure: vt100 parsing, PTY I/O, run-length
//! encoded screen serialization, color rendering, and (in the future)
//! copy-mode, scrollback, etc.
//!
//! This module centralises popup-specific logic:
//!  - PTY-backed pane creation  (`create_popup_pane`)
//!  - Server-side JSON serialization (`serialize_popup_overlay`)
//!  - In-process TUI rendering   (`render_popup_overlay`)

use std::sync::{Arc, Mutex};

use crate::layout::serialize_screen_rows;
use crate::types::{Pane, AppState, Mode};

// ── Popup pane creation ─────────────────────────────────────────────

/// Spawn a PTY-backed `Pane` for use inside a popup overlay.
///
/// This reuses the same PTY infrastructure as regular panes (ConPTY,
/// vt100 parser, reader thread) but does NOT add the pane to any window
/// tree.  The returned `Pane` is stored inside `Mode::PopupMode`.
pub fn create_popup_pane(
    command: &str,
    start_dir: Option<&str>,
    rows: u16,
    cols: u16,
    pane_id: usize,
    session_name: &str,
    environment: &std::collections::HashMap<String, String>,
) -> Option<Pane> {
    let pty_sys = portable_pty::native_pty_system();
    let pty_size = portable_pty::PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = pty_sys.openpty(pty_size).ok()?;

    let mut cmd_builder = portable_pty::CommandBuilder::new(
        if cfg!(windows) { "pwsh" } else { "sh" },
    );
    if let Some(dir) = start_dir {
        cmd_builder.cwd(dir);
    } else if let Ok(dir) = std::env::current_dir() {
        cmd_builder.cwd(dir);
    }
    // Color support env vars (#154)
    cmd_builder.env("TERM", "xterm-256color");
    cmd_builder.env("COLORTERM", "truecolor");
    cmd_builder.env("PSMUX_SESSION", session_name);
    crate::pane::apply_user_environment(&mut cmd_builder, environment);
    if cfg!(windows) {
        cmd_builder.args(["-NoProfile", "-Command", command]);
    } else {
        cmd_builder.args(["-c", command]);
    }

    let child = pair.slave.spawn_command(cmd_builder).ok()?;
    drop(pair.slave); // required for ConPTY

    let term: Arc<Mutex<vt100::Parser>> =
        Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
    let term_reader = term.clone();

    // Reader thread (same as regular pane reader)
    if let Ok(mut reader) = pair.master.try_clone_reader() {
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(n) if n > 0 => {
                        if let Ok(mut p) = term_reader.lock() {
                            p.process(&buf[..n]);
                        }
                    }
                    _ => break,
                }
            }
        });
    }

    let mut pty_writer = pair.master.take_writer().ok()?;
    crate::pane::conpty_preemptive_dsr_response(&mut *pty_writer);

    // Brief delay so the reader thread processes initial output before the
    // first frame is serialized to clients.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let child_pid = crate::platform::mouse_inject::get_child_pid(&*child);
    let epoch = std::time::Instant::now() - std::time::Duration::from_secs(2);
    let data_version = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let cursor_shape = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(
        crate::pane::CURSOR_SHAPE_UNSET,
    ));
    let bell_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    Some(Pane {
        master: pair.master,
        writer: pty_writer,
        child,
        term,
        last_rows: rows,
        last_cols: cols,
        id: pane_id,
        title: String::new(),
        title_locked: false,
        child_pid,
        data_version,
        last_title_check: epoch,
        last_infer_title: epoch,
        dead: false,
        vt_bridge_cache: None,
        vti_mode_cache: None,
        mouse_input_cache: None,
        cursor_shape,
        bell_pending,
        copy_state: None,
        pane_style: None,
        squelch_until: None,
        output_ring: std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
    })
}

// ── Server-side popup serialization ─────────────────────────────────

/// Build a JSON fragment with overlay state for the current popup.
///
/// Returns a string like `,"popup_active":true,"popup_rows":[...]`
/// that the server injects into the dump-state JSON.  Reuses the same
/// `rows_v2` serialization format as regular panes via
/// `serialize_screen_rows()`.
pub fn serialize_popup_overlay(app: &AppState) -> String {
    use crate::server::helpers::json_escape_string;

    let mut out = String::new();
    match &app.mode {
        Mode::PopupMode {
            command,
            output,
            width,
            height,
            popup_pane,
            ..
        } => {
            out.push_str(",\"popup_active\":true");
            out.push_str(",\"popup_command\":\"");
            out.push_str(&json_escape_string(command));
            out.push('"');
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(",\"popup_width\":{},\"popup_height\":{}", width, height),
            );
            let inner_h = height.saturating_sub(2);
            let inner_w = width.saturating_sub(2);

            if let Some(pane) = popup_pane {
                // PTY popup: serialize using the shared pane screen serializer
                out.push_str(",\"popup_rows\":[");
                if let Ok(parser) = pane.term.lock() {
                    let screen = parser.screen();
                    let rows_data = serialize_screen_rows(screen, inner_h, inner_w);
                    for (i, row) in rows_data.iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        // Serialize RowRunsJson inline (avoids serde for perf)
                        out.push_str("{\"runs\":[");
                        for (j, run) in row.runs.iter().enumerate() {
                            if j > 0 {
                                out.push(',');
                            }
                            out.push_str("{\"text\":\"");
                            json_esc_inline(&run.text, &mut out);
                            out.push_str("\",\"fg\":\"");
                            out.push_str(&run.fg);
                            out.push_str("\",\"bg\":\"");
                            out.push_str(&run.bg);
                            let _ = std::fmt::Write::write_fmt(
                                &mut out,
                                format_args!("\",\"flags\":{},\"width\":{}}}", run.flags, run.width),
                            );
                        }
                        out.push_str("]}");
                    }
                }
                out.push(']');
                out.push_str(",\"popup_lines\":[]");
                out.push_str(",\"popup_has_pty\":true");
            } else {
                // Static (non-PTY) popup: plain text lines
                out.push_str(",\"popup_rows\":[]");
                out.push_str(",\"popup_lines\":[");
                for (i, line) in output.lines().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    out.push_str(&json_escape_string(line));
                    out.push('"');
                }
                out.push(']');
                out.push_str(",\"popup_has_pty\":false");
            }
        }
        Mode::MenuMode { menu } => {
            out.push_str(",\"popup_active\":false,\"popup_rows\":[],\"popup_lines\":[],\"popup_has_pty\":false");
            out.push_str(",\"menu_active\":true");
            out.push_str(",\"menu_title\":\"");
            out.push_str(&json_escape_string(&menu.title));
            out.push('"');
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(",\"menu_selected\":{}", menu.selected),
            );
            out.push_str(",\"menu_items\":[");
            for (i, item) in menu.items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str("{\"name\":\"");
                out.push_str(&json_escape_string(&item.name));
                out.push_str("\",\"key\":");
                if let Some(k) = item.key {
                    out.push('"');
                    out.push(k);
                    out.push('"');
                } else {
                    out.push_str("null");
                }
                out.push_str(",\"command\":\"");
                out.push_str(&json_escape_string(&item.command));
                out.push_str("\",\"is_separator\":");
                out.push_str(if item.is_separator { "true" } else { "false" });
                out.push('}');
            }
            out.push(']');
        }
        Mode::ConfirmMode { prompt, .. } => {
            out.push_str(",\"popup_active\":false,\"popup_rows\":[],\"popup_lines\":[],\"popup_has_pty\":false");
            out.push_str(",\"menu_active\":false,\"menu_title\":\"\",\"menu_selected\":0,\"menu_items\":[]");
            out.push_str(",\"confirm_active\":true,\"confirm_prompt\":\"");
            out.push_str(&json_escape_string(prompt));
            out.push('"');
        }
        Mode::PaneChooser { .. } => {
            out.push_str(",\"popup_active\":false,\"popup_rows\":[],\"popup_lines\":[],\"popup_has_pty\":false");
            out.push_str(",\"menu_active\":false,\"menu_title\":\"\",\"menu_selected\":0,\"menu_items\":[]");
            out.push_str(",\"confirm_active\":false,\"confirm_prompt\":\"\"");
            out.push_str(",\"display_panes\":true");
        }
        Mode::CustomizeMode { ref options, selected, scroll_offset, editing, ref edit_buffer, edit_cursor, ref filter } => {
            out.push_str(",\"popup_active\":false,\"popup_rows\":[],\"popup_lines\":[],\"popup_has_pty\":false");
            out.push_str(",\"menu_active\":false,\"menu_title\":\"\",\"menu_selected\":0,\"menu_items\":[]");
            out.push_str(",\"confirm_active\":false,\"confirm_prompt\":\"\"");
            out.push_str(",\"display_panes\":false");
            out.push_str(",\"customize_active\":true");
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(",\"customize_selected\":{},\"customize_scroll\":{},\"customize_editing\":{},\"customize_cursor\":{}",
                    selected, scroll_offset, editing, edit_cursor),
            );
            out.push_str(",\"customize_edit_buf\":\"");
            out.push_str(&json_escape_string(edit_buffer));
            out.push('"');
            out.push_str(",\"customize_filter\":\"");
            out.push_str(&json_escape_string(filter));
            out.push('"');
            // Serialize visible option rows
            out.push_str(",\"customize_options\":[");
            let filter_lower = filter.to_lowercase();
            let mut first = true;
            for (i, (name, value, scope)) in options.iter().enumerate() {
                if !filter.is_empty() && !name.to_lowercase().contains(&filter_lower) {
                    continue;
                }
                if !first { out.push(','); }
                first = false;
                out.push_str("{\"i\":");
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{}", i));
                out.push_str(",\"n\":\"");
                out.push_str(&json_escape_string(name));
                out.push_str("\",\"v\":\"");
                out.push_str(&json_escape_string(value));
                out.push_str("\",\"s\":\"");
                out.push_str(&json_escape_string(scope));
                out.push_str("\"}");
            }
            out.push(']');
        }
        _ => {
            out.push_str(",\"popup_active\":false,\"popup_rows\":[],\"popup_lines\":[],\"popup_has_pty\":false");
            out.push_str(",\"menu_active\":false,\"menu_title\":\"\",\"menu_selected\":0,\"menu_items\":[]");
            out.push_str(",\"confirm_active\":false,\"confirm_prompt\":\"\"");
            out.push_str(",\"display_panes\":false");
        }
    }
    out
}

/// JSON-escape a string inline (for popup run serialization).
fn json_esc_inline(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

// ── In-process popup rendering (app.rs TUI) ─────────────────────────

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
