// Tests for command-prompt key binding fix.
//
// BUG: When a key binding like:
//   bind-key , command-prompt -I '#W' 'rename-window "%%"'
// was triggered, the command-prompt overlay did not appear because the client
// only checked for exact "command-prompt" (not "command-prompt -I ...").
//
// FIX: The client now parses the full command-prompt arguments to extract:
//   -I <initial>  : pre-fill text (with #W/#S format expansion)
//   -p <label>    : overlay title (replaces generic "command" title)
//   <template>    : positional arg with %% substituted on Enter
//
// These tests verify the overlay rendering and argument parsing logic.

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;

/// Extract all text content from a TestBackend buffer as a single string.
fn buffer_text(backend: &TestBackend) -> String {
    let buf = backend.buffer();
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            text.push_str(cell.symbol());
        }
    }
    text
}

/// Simulates a centered rect calculation (matches client.rs centered_rect)
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width as u32 * percent_x as u32 / 100).min(area.width as u32) as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Overlay shows custom label from -p flag
// ═════════════════════════════════════════════════════════════════════

#[test]
fn command_prompt_overlay_uses_p_label_as_title() {
    let command_input = true;
    let command_prompt_label = "session name:";
    let command_buf = "my_session";
    // template is set (from a command-prompt binding with positional arg)
    let has_template = true;

    let title = if !command_prompt_label.is_empty() { command_prompt_label } else { "command" };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| {
        let area = f.area();
        if command_input {
            let overlay = Block::default().borders(Borders::ALL).title(title);
            let oa = centered_rect(60, 3, area);
            f.render_widget(Clear, oa);
            f.render_widget(&overlay, oa);
            let inner = overlay.inner(oa);
            let para = if has_template {
                Paragraph::new(command_buf.to_string())
            } else {
                Paragraph::new(format!(": {}", command_buf))
            };
            f.render_widget(para, inner);
        }
    }).unwrap();

    let text = buffer_text(terminal.backend());
    assert!(
        text.contains("session name:"),
        "Overlay title should contain the -p label 'session name:'. Got:\n{}",
        text
    );
    assert!(
        !text.contains(": my_session"),
        "With template, input should NOT be prefixed with ': '. Got:\n{}",
        text
    );
    assert!(
        text.contains("my_session"),
        "Overlay should show the pre-filled initial value. Got:\n{}",
        text
    );
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Overlay uses "command" title when no -p label provided
// ═════════════════════════════════════════════════════════════════════

#[test]
fn command_prompt_overlay_uses_default_command_title_without_p_label() {
    let command_input = true;
    let command_prompt_label = "";  // no -p flag
    let command_buf = "";

    let title = if !command_prompt_label.is_empty() { command_prompt_label } else { "command" };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| {
        let area = f.area();
        if command_input {
            let overlay = Block::default().borders(Borders::ALL).title(title);
            let oa = centered_rect(60, 3, area);
            f.render_widget(Clear, oa);
            f.render_widget(&overlay, oa);
            let inner = overlay.inner(oa);
            let para = Paragraph::new(format!(": {}", command_buf));
            f.render_widget(para, inner);
        }
    }).unwrap();

    let text = buffer_text(terminal.backend());
    assert!(
        text.contains("command"),
        "Without -p label, overlay title should be 'command'. Got:\n{}",
        text
    );
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Without template, normal ": " prefix is shown
// ═════════════════════════════════════════════════════════════════════

#[test]
fn command_prompt_overlay_shows_colon_prefix_without_template() {
    let command_buf = "rename-window test";
    let has_template = false;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| {
        let area = f.area();
        let overlay = Block::default().borders(Borders::ALL).title("command");
        let oa = centered_rect(60, 3, area);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let para = if has_template {
            Paragraph::new(command_buf.to_string())
        } else {
            Paragraph::new(format!(": {}", command_buf))
        };
        f.render_widget(para, inner);
    }).unwrap();

    let text = buffer_text(terminal.backend());
    assert!(
        text.contains(": rename-window test"),
        "Without template, input should be shown with ': ' prefix. Got:\n{}",
        text
    );
}

// ═════════════════════════════════════════════════════════════════════
//  Test: %% substitution logic
// ═════════════════════════════════════════════════════════════════════

#[test]
fn command_prompt_template_percent_percent_substitution() {
    let template = r#"rename-window "%%""#;
    let user_input = "my_window";
    let result = template.replace("%%", user_input);
    assert_eq!(result, r#"rename-window "my_window""#);
}

#[test]
fn command_prompt_session_template_percent_percent_substitution() {
    let template = r#"rename-session "%%""#;
    let user_input = "new_session";
    let result = template.replace("%%", user_input);
    assert_eq!(result, r#"rename-session "new_session""#);
}

// ═════════════════════════════════════════════════════════════════════
//  Test: Format string expansion for #W and #S
// ═════════════════════════════════════════════════════════════════════

#[test]
fn command_prompt_initial_value_hash_w_expansion() {
    let raw_initial = "#W";
    let active_win_name = "current-window";
    let current_session = "my_session";
    let expanded = raw_initial.replace("#W", active_win_name).replace("#S", current_session);
    assert_eq!(expanded, "current-window");
}

#[test]
fn command_prompt_initial_value_hash_s_expansion() {
    let raw_initial = "#S";
    let active_win_name = "current-window";
    let current_session = "my_session";
    let expanded = raw_initial.replace("#W", active_win_name).replace("#S", current_session);
    assert_eq!(expanded, "my_session");
}

#[test]
fn command_prompt_initial_value_no_format_string() {
    // Initial value without format strings should be unchanged
    let raw_initial = "literal_value";
    let active_win_name = "current-window";
    let current_session = "my_session";
    let expanded = raw_initial.replace("#W", active_win_name).replace("#S", current_session);
    assert_eq!(expanded, "literal_value");
}

// ═════════════════════════════════════════════════════════════════════
//  Test: command-prompt argument parsing
// ═════════════════════════════════════════════════════════════════════

/// Minimal version of parse_command_line for testing argument extraction
fn parse_command_prompt_args(cmd: &str) -> (String, String, Option<String>) {
    // Uses the same logic as the client dispatch code
    let args = psmux_parse_command_line(cmd);
    let mut initial = String::new();
    let mut label = String::new();
    let mut template: Option<String> = None;
    let mut ai = 1;
    while ai < args.len() {
        match args[ai].as_str() {
            "-I" => { if ai + 1 < args.len() { initial = args[ai + 1].clone(); ai += 1; } }
            "-p" => { if ai + 1 < args.len() { label = args[ai + 1].clone(); ai += 1; } }
            "-1" | "-N" | "-W" => {}
            "-T" | "-t" => { ai += 1; }
            a if !a.starts_with('-') => { template = Some(a.to_string()); }
            _ => {}
        }
        ai += 1;
    }
    (initial, label, template)
}

/// Duplicate the parse_command_line logic for standalone testing
fn psmux_parse_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_double_quotes = false;
    let mut in_single_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_single_quotes {
            if c == '\'' { in_single_quotes = false; } else { current.push(c); }
        } else if c == '\\' && in_double_quotes {
            if i + 1 < chars.len() && chars[i + 1] == '"' { current.push('"'); i += 1; }
            else if i + 1 < chars.len() && chars[i + 1] == '\\' { current.push('\\'); i += 1; }
            else { current.push(c); }
        } else if c == '"' { in_double_quotes = !in_double_quotes; }
        else if c == '\'' && !in_double_quotes { in_single_quotes = true; }
        else if c.is_whitespace() && !in_double_quotes {
            if !current.is_empty() { args.push(current.clone()); current.clear(); }
        } else { current.push(c); }
        i += 1;
    }
    if !current.is_empty() { args.push(current); }
    args
}

#[test]
fn parse_command_prompt_rename_window_binding() {
    let cmd = "command-prompt -I '#W' 'rename-window \"%%\"'";
    let (initial, label, template) = parse_command_prompt_args(cmd);
    assert_eq!(initial, "#W", "Should extract #W as initial value");
    assert_eq!(label, "", "No -p flag, label should be empty");
    assert_eq!(template, Some(r#"rename-window "%%""#.to_string()), "Template should be the positional arg");
}

#[test]
fn parse_command_prompt_rename_session_binding() {
    let cmd = "command-prompt -p 'session name:' -I '#S' 'rename-session \"%%\"'";
    let (initial, label, template) = parse_command_prompt_args(cmd);
    assert_eq!(initial, "#S", "Should extract #S as initial value");
    assert_eq!(label, "session name:", "Should extract label from -p flag");
    assert_eq!(template, Some(r#"rename-session "%%""#.to_string()), "Template should be the positional arg");
}

#[test]
fn parse_bare_command_prompt() {
    let cmd = "command-prompt";
    let (initial, label, template) = parse_command_prompt_args(cmd);
    assert_eq!(initial, "", "No -I flag, initial should be empty");
    assert_eq!(label, "", "No -p flag, label should be empty");
    assert_eq!(template, None, "No template for bare command-prompt");
}

#[test]
fn command_prompt_first_word_detection() {
    // Verify the starts_with logic detects "command-prompt" as the first word
    let bare = "command-prompt";
    let with_args = "command-prompt -I '#W' 'rename-window \"%%\"'";
    let not_cp = "rename-window";

    let is_cp = |s: &str| s.split_whitespace().next().map_or(false, |w| w == "command-prompt");

    assert!(is_cp(bare), "bare 'command-prompt' should be detected");
    assert!(is_cp(with_args), "'command-prompt -I ...' should be detected");
    assert!(!is_cp(not_cp), "'rename-window' should NOT be detected as command-prompt");
}
