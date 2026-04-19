use std::io;
use std::env;

use crate::session::{send_control, send_control_with_response};

pub(crate) fn handle_send_keys(cmd_args: &[&String]) -> io::Result<()> {
    let mut literal = false;
    let mut has_x = false;
    let mut keys: Vec<String> = Vec::new();
    // Getopt-style parsing: -t consumes next arg, -l/-R/-X are flags
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-l" => { literal = true; }
            "-R" => { keys.push("__RESET__".to_string()); }
            "-X" => { has_x = true; }
            "-t" => { i += 1; } // consume target value (already handled globally)
            "-N" => { i += 1; } // repeat count, consume value
            _ => { keys.push(cmd_args[i].to_string()); }
        }
        i += 1;
    }
    let mut cmd = "send-keys".to_string();
    if literal { cmd.push_str(" -l"); }
    if has_x { cmd.push_str(" -X"); }
    // Quote arguments that contain spaces to preserve them
    for k in keys { 
        if k.contains(' ') || k.contains('\t') || k.contains('"') {
            // Escape embedded double-quotes and wrap in quotes.
            // Do NOT escape backslashes: the server parser treats
            // them as literal (Windows path separator).
            let escaped = k.replace('"', "\\\"");
            cmd.push_str(&format!(" \"{}\"", escaped));
        } else {
            cmd.push_str(&format!(" {}", k)); 
        }
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_send_paste(cmd_args: &[&String]) -> io::Result<()> {
    let mut payload = String::new();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => { i += 1; } // consume target (handled globally)
            _ => { payload = cmd_args[i].to_string(); }
        }
        i += 1;
    }
    if !payload.is_empty() {
        send_control(format!("send-paste {}\n", payload))?;
    }
    Ok(())
}

pub(crate) fn handle_list_keys(cmd_args: &[&String]) -> io::Result<()> {
    let mut table_filter: Option<String> = None;
    let mut key_filter: Option<String> = None;
    let mut cmd = "list-keys".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-T" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    table_filter = Some(t.to_string());
                    cmd.push_str(&format!(" -T {}", t));
                    i += 1;
                }
            }
            "-t" => { i += 1; } // target handled globally
            arg if !arg.starts_with('-') => {
                // Positional: key name to filter
                if key_filter.is_none() {
                    key_filter = Some(arg.to_string());
                }
                cmd.push_str(&format!(" {}", arg));
            }
            _ => { cmd.push_str(&format!(" {}", cmd_args[i])); }
        }
        i += 1;
    }
    cmd.push('\n');
    match send_control_with_response(cmd) {
        Ok(resp) => { print!("{}", resp); }
        Err(_) => {
            // No running server — emit built-in defaults filtered by -T and key.
            let table = table_filter.as_deref().unwrap_or("prefix");
            for (key, action) in crate::help::PREFIX_DEFAULTS {
                if table != "prefix" { break; }
                if let Some(ref kf) = key_filter {
                    if *key != kf.as_str() { continue; }
                }
                println!("bind-key -T {} {} {}", table, key, action);
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_copy_mode(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "copy-mode".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-u" => { cmd.push_str(" -u"); }
            "-d" => { cmd.push_str(" -d"); }
            "-e" => { cmd.push_str(" -e"); }
            "-H" => { cmd.push_str(" -H"); }
            "-q" => { cmd.push_str(" -q"); }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_set_option(cmd_args: &[&String]) -> io::Result<()> {
    let cmd_str: String = cmd_args.iter().map(|s| {
        let s = s.as_str();
        if s.contains(' ') {
            format!("\"{}\"", s.replace('"', "\\\""))
        } else {
            s.to_string()
        }
    }).collect::<Vec<String>>().join(" ");
    match send_control(format!("{}\n", cmd_str)) {
        Ok(()) => {},
        Err(e) if e.to_string().contains("no session") => {
            eprintln!("warning: no active session; option will take effect when set inside a session or via config file");
        },
        Err(e) => return Err(e),
    }
    Ok(())
}

pub(crate) fn handle_source_file(cmd_args: &[&String]) -> io::Result<()> {
    let mut quiet = false;
    let mut file_path: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-q" => { quiet = true; }
            "-n" => { /* parse only, don't execute */ }
            "-v" => { /* verbose */ }
            s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
            _ => {}
        }
        i += 1;
    }
    if let Some(path) = file_path {
        // Expand ~ to home directory
        let expanded = if path.starts_with('~') {
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            path.replacen('~', &home, 1)
        } else {
            path
        };
        if let Err(e) = std::fs::read_to_string(&expanded) {
            if !quiet {
                eprintln!("psmux: {}: {}", expanded, e);
                std::process::exit(1);
            }
        } else {
            // Send source-file command to server if attached
            send_control(format!("source-file {}\n", crate::util::quote_arg(&expanded)))?;
        }
    }
    Ok(())
}
