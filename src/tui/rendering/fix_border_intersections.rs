use super::*;

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
