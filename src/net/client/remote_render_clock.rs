use super::*;

/// Render a large ASCII clock overlay (tmux clock-mode).
pub(crate) fn render_clock_overlay(f: &mut Frame, area: Rect, colour: Color) {
    const DIGITS: [&[&str; 5]; 10] = [
        &["###", "# #", "# #", "# #", "###"],
        &["  #", "  #", "  #", "  #", "  #"],
        &["###", "  #", "###", "#  ", "###"],
        &["###", "  #", "###", "  #", "###"],
        &["# #", "# #", "###", "  #", "  #"],
        &["###", "#  ", "###", "  #", "###"],
        &["###", "#  ", "###", "# #", "###"],
        &["###", "  #", "  #", "  #", "  #"],
        &["###", "# #", "###", "# #", "###"],
        &["###", "# #", "###", "  #", "###"],
    ];
    const COLON: [&str; 5] = [" ", "#", " ", "#", " "];
    let now = Local::now();
    let time_str = now.format("%H:%M:%S").to_string();
    let total_w: u16 = time_str.chars().map(|c| if c == ':' { 2 } else { 4 }).sum::<u16>() - 1;
    let total_h: u16 = 5;
    if area.width < total_w || area.height < total_h { return; }
    let start_x = area.x + (area.width.saturating_sub(total_w)) / 2;
    let start_y = area.y + (area.height.saturating_sub(total_h)) / 2;
    let clock_area = Rect::new(start_x.saturating_sub(1), start_y, total_w + 2, total_h);
    f.render_widget(Clear, clock_area);
    for row in 0..5u16 {
        let mut x = start_x;
        for ch in time_str.chars() {
            if ch == ':' {
                let cell_area = Rect::new(x, start_y + row, 1, 1);
                let s = Span::styled(COLON[row as usize], Style::default().fg(colour));
                f.render_widget(Paragraph::new(Line::from(s)), cell_area);
                x += 2;
            } else if let Some(d) = ch.to_digit(10) {
                let pattern = DIGITS[d as usize][row as usize];
                let cell_area = Rect::new(x, start_y + row, 3, 1);
                let s = Span::styled(pattern, Style::default().fg(colour));
                f.render_widget(Paragraph::new(Line::from(s)), cell_area);
                x += 4;
            }
        }
    }
}
