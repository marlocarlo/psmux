use super::*;

/// Append the JSON-escaped form of `s` into `out`.
pub(crate) fn json_esc(s: &str, out: &mut String) {
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
pub(crate) fn push_color(c: vt100::Color, out: &mut String) {
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
pub(crate) fn close_run(fg: vt100::Color, bg: vt100::Color, fl: u8, w: u16, out: &mut String) {
    out.push_str("\",\"fg\":\"");
    push_color(fg, out);
    out.push_str("\",\"bg\":\"");
    push_color(bg, out);
    let _ = std::fmt::Write::write_fmt(out, format_args!("\",\"flags\":{},\"width\":{}}}", fl, w));
}
