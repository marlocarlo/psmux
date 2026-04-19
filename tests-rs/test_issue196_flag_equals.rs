// Issue #196: Argument parser silently drops -x=VALUE form across all commands
// (has-session -t=NAME always exits 0)
//
// Tests that normalize_flag_equals correctly splits -x=VALUE into ["-x", "VALUE"]
// while preserving all edge cases (long flags, positional args, bare dashes, etc.)

use super::*;

// ---- normalize_flag_equals (owned) ----

fn nfe(args: &[&str]) -> Vec<String> {
    normalize_flag_equals(args.iter().map(|s| s.to_string()).collect())
}

#[test]
fn split_short_flag_equals_t() {
    assert_eq!(nfe(&["-t=mysession"]), vec!["-t", "mysession"]);
}

#[test]
fn split_short_flag_equals_s() {
    assert_eq!(nfe(&["-s=foo"]), vec!["-s", "foo"]);
}

#[test]
fn split_short_flag_equals_n() {
    assert_eq!(nfe(&["-n=mywin"]), vec!["-n", "mywin"]);
}

#[test]
fn split_short_flag_equals_x() {
    assert_eq!(nfe(&["-x=80"]), vec!["-x", "80"]);
}

#[test]
fn split_short_flag_equals_y() {
    assert_eq!(nfe(&["-y=24"]), vec!["-y", "24"]);
}

#[test]
fn split_preserves_value_with_equals() {
    // Value itself may contain = signs (e.g. set-option value)
    assert_eq!(nfe(&["-t=a=b"]), vec!["-t", "a=b"]);
}

#[test]
fn split_preserves_value_with_colon() {
    // session:window.pane targets
    assert_eq!(nfe(&["-t=dev:0.1"]), vec!["-t", "dev:0.1"]);
}

#[test]
fn no_split_space_form() {
    // Already correct: -t value as separate args
    assert_eq!(nfe(&["-t", "mysession"]), vec!["-t", "mysession"]);
}

#[test]
fn no_split_long_flag() {
    // Long flags (--name=value) pass through unchanged
    assert_eq!(nfe(&["--target=foo"]), vec!["--target=foo"]);
}

#[test]
fn no_split_positional_with_equals() {
    // Positional args like FOO=bar (no dash prefix) pass through
    assert_eq!(nfe(&["FOO=bar"]), vec!["FOO=bar"]);
}

#[test]
fn no_split_bare_dash() {
    // Bare dash (stdin marker) passes through
    assert_eq!(nfe(&["-"]), vec!["-"]);
}

#[test]
fn no_split_degenerate_dash_equals() {
    // -=value: single dash then equals, no letter: pass through
    assert_eq!(nfe(&["-=value"]), vec!["-=value"]);
}

#[test]
fn no_split_flag_without_value() {
    // -h, -v etc. (boolean flags) pass through
    assert_eq!(nfe(&["-h"]), vec!["-h"]);
    assert_eq!(nfe(&["-v"]), vec!["-v"]);
}

#[test]
fn no_split_numeric_flag() {
    // -1=bar: digit after dash, not a letter, pass through
    assert_eq!(nfe(&["-1=bar"]), vec!["-1=bar"]);
}

#[test]
fn mixed_args_normalize_correctly() {
    let input = &["psmux", "has-session", "-t=mysession", "-v"];
    let expected = vec!["psmux", "has-session", "-t", "mysession", "-v"];
    assert_eq!(nfe(input), expected);
}

#[test]
fn multiple_flags_with_equals() {
    let input = &["capture-pane", "-t=dev:0.1", "-S=0", "-E=100", "-p"];
    let expected = vec!["capture-pane", "-t", "dev:0.1", "-S", "0", "-E", "100", "-p"];
    assert_eq!(nfe(input), expected);
}

#[test]
fn has_session_garbage_regression() {
    // The original bug: -t=literally_any_garbage_xyzzy should split so the
    // downstream parser sees -t followed by the garbage name, which should NOT
    // match any real session.
    let input = &["has-session", "-t=literally_any_garbage_xyzzy"];
    let expected = vec!["has-session", "-t", "literally_any_garbage_xyzzy"];
    assert_eq!(nfe(input), expected);
}

// ---- normalize_flag_equals_borrowed ----

#[test]
fn borrowed_split_short_flag() {
    let args: Vec<&str> = vec!["-t=foo", "-p"];
    let result = normalize_flag_equals_borrowed(&args);
    assert_eq!(result, vec!["-t", "foo", "-p"]);
}

#[test]
fn borrowed_no_split_long_flag() {
    let args: Vec<&str> = vec!["--target=bar"];
    let result = normalize_flag_equals_borrowed(&args);
    assert_eq!(result, vec!["--target=bar"]);
}

#[test]
fn borrowed_mixed_args() {
    let args: Vec<&str> = vec!["-t=dev:0", "-S=10", "positional", "--long=v"];
    let result = normalize_flag_equals_borrowed(&args);
    assert_eq!(result, vec!["-t", "dev:0", "-S", "10", "positional", "--long=v"]);
}
