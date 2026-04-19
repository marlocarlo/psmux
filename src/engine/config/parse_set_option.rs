#[allow(unused_imports)]
use std::env;
use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::types::{AppState, Action, Bind};
use crate::commands::parse_command_to_action;

// Track the current config file being parsed (for #{current_file}, #{d:current_file})
use super::*;

pub(crate) fn parse_set_option(app: &mut AppState, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 { return; }
    
    let mut i = 1;
    let mut is_global = false;
    let mut format_expand = false;  // -F: expand format strings in value
    let mut only_if_unset = false;  // -o: only set if not already set
    let mut append_mode = false;    // -a: append to current value
    let mut unset_mode = false;     // -u: unset (reset to default)
    
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('g') { is_global = true; }
            if p.contains('F') { format_expand = true; }
            if p.contains('o') { only_if_unset = true; }
            if p.contains('a') { append_mode = true; }
            if p.contains('u') { unset_mode = true; }
            // -q (quiet): no-op — we don't produce errors for unknown options
            // -w: window option — treat same as global for our single-server model
            i += 1;
            if p.contains('t') && i < parts.len() { i += 1; }
        } else {
            break;
        }
    }
    
    if i >= parts.len() { return; }

    // Extract key and value
    let key = parts[i];
    let raw_value = if i + 1 < parts.len() {
        parts[i + 1..].join(" ")
    } else {
        String::new()
    };

    // Handle -u (unset): reset option to empty
    if unset_mode {
        parse_option_value(app, &format!("{} ", key), is_global);
        return;
    }

    // Handle -o (only set if not currently set)
    if only_if_unset {
        // For @-prefixed user options, check if key exists
        // For built-in options, check the user_set_options tracker
        let already_set = if key.starts_with('@') {
            app.user_options.contains_key(key)
        } else {
            app.user_set_options.contains(key)
        };
        if already_set { return; }
    }

    // Expand format strings in the value if -F flag is set
    let value = if format_expand && !raw_value.is_empty() {
        let stripped = raw_value.trim_matches('"').trim_matches('\'');
        let expanded = crate::format::expand_format(stripped, app);
        expanded
    } else {
        raw_value
    };

    // Handle -a (append to current value)
    let final_value = if append_mode {
        let current = crate::format::lookup_option_pub(key, app).unwrap_or_default();
        format!("{}{}", current, value.trim_matches('"').trim_matches('\''))
    } else {
        value
    };

    let rest = format!("{} {}", key, final_value);
    parse_option_value(app, &rest, is_global);
    // Track that this option was explicitly set (for -o only-if-unset checks)
    app.user_set_options.insert(key.to_string());
}
