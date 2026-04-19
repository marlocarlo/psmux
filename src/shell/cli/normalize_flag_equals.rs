#[allow(unused_imports)]
use crate::types::{ParsedTarget, VERSION};

/// Normalize `-x=VALUE` short-flag forms into `["-x", "VALUE"]`.
///
/// tmux accepts both `-t VALUE` (space) and `-t=VALUE` (equals) for
/// single-character flags.  psmux's parsers only handled the space form.
/// This function expands the equals form so every downstream comparison
/// (`arg == "-t"`, `args.windows(2)`, etc.) works without changes.
///
/// Rules:
///   - Only tokens starting with a single `-` (not `--`) are split.
///   - The flag letter must be ASCII alphabetic (`-t=foo` yes, `-1=bar` no).
///   - Long flags (`--name=value`) pass through unchanged.
///   - Positional tokens without a leading `-` pass through unchanged.
///   - Bare `-` and degenerate `-=` pass through unchanged.
use super::*;

pub fn normalize_flag_equals(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        // Must start with exactly one dash, followed by a single ASCII letter,
        // then `=`, then at least one character of value.
        if arg.len() >= 4
            && arg.starts_with('-')
            && !arg.starts_with("--")
        {
            let bytes = arg.as_bytes();
            if bytes[1].is_ascii_alphabetic() && bytes[2] == b'=' {
                out.push(format!("-{}", bytes[1] as char));
                out.push(arg[3..].to_string());
                continue;
            }
        }
        out.push(arg);
    }
    out
}

/// Same as [`normalize_flag_equals`] but operates on `Vec<&str>`, returning
/// owned strings (needed where the caller already has borrowed slices).
pub fn normalize_flag_equals_borrowed(args: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if arg.len() >= 4
            && arg.starts_with('-')
            && !arg.starts_with("--")
        {
            let bytes = arg.as_bytes();
            if bytes[1].is_ascii_alphabetic() && bytes[2] == b'=' {
                out.push(format!("-{}", bytes[1] as char));
                out.push(arg[3..].to_string());
                continue;
            }
        }
        out.push(arg.to_string());
    }
    out
}

pub fn get_program_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "psmux".to_string())
        .to_lowercase()
        .replace(".exe", "")
}
