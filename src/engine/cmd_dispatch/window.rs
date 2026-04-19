use std::io;

use crate::session::{send_control, send_control_with_response};

pub(crate) fn handle_new_window(cmd_args: &[&String]) -> io::Result<()> {
    // Strict getopt-style parsing for new-window flags.
    // tmux template: "ac:dDe:F:kn:Pt:S:"
    let mut name_arg: Option<String> = None;
    let mut detached = false;
    let mut print_info = false;
    let mut format_str: Option<String> = None;
    let mut start_dir: Option<String> = None;
    let mut nw_positional: Vec<String> = Vec::new();
    {
        let mut i = 1;
        while i < cmd_args.len() {
            let a = cmd_args[i].as_str();
            if a == "--" { nw_positional.extend(cmd_args[i+1..].iter().map(|s| s.to_string())); break; }
            match a {
                "-n" => { i += 1; if i < cmd_args.len() { name_arg = Some(cmd_args[i].trim_matches('"').to_string()); } }
                "-F" => { i += 1; if i < cmd_args.len() { format_str = Some(cmd_args[i].trim_matches('"').to_string()); } }
                "-c" => { i += 1; if i < cmd_args.len() { start_dir = Some(cmd_args[i].trim_matches('"').to_string()); } }
                "-t" | "-e" | "-S" => { i += 1; /* skip value */ }
                "-d" => { detached = true; }
                "-P" => { print_info = true; }
                "-a" | "-D" | "-k" => { /* ignored for compatibility */ }
                _ if a.starts_with('-') => { /* unknown flag, skip */ }
                _ => { nw_positional.extend(cmd_args[i..].iter().map(|s| s.to_string())); break; }
            }
            i += 1;
        }
    }
    let cmd_arg = nw_positional.join(" ");
    let cmd_arg = cmd_arg.as_str();
    let mut cmd_line = "new-window".to_string();
    if detached { cmd_line.push_str(" -d"); }
    if print_info { cmd_line.push_str(" -P"); }
    if let Some(ref fmt) = format_str {
        cmd_line.push_str(&format!(" -F \"{}\"", fmt.replace("\"", "\\\"")));
    }
    if let Some(name) = &name_arg {
        cmd_line.push_str(&format!(" -n \"{}\"", name.replace("\"", "\\\"")));
    }
    if let Some(dir) = &start_dir {
        cmd_line.push_str(&format!(" -c \"{}\"", dir.replace("\"", "\\\"")));
    }
    if !cmd_arg.is_empty() {
        cmd_line.push_str(&format!(" \"{}\"", cmd_arg.replace("\"", "\\\"")));
    }
    cmd_line.push('\n');
    if print_info {
        let resp = send_control_with_response(cmd_line)?;
        print!("{}", resp);
    } else {
        send_control(cmd_line)?;
    }
    Ok(())
}

pub(crate) fn handle_split_window(cmd_args: &[&String]) -> io::Result<()> {
    // Strict getopt-style parsing for split-window flags.
    // tmux template: "bc:de:F:fhIl:p:Pt:vZ"
    let mut flag = "-v";
    let mut detached = false;
    let mut print_info = false;
    let mut format_str: Option<String> = None;
    let mut start_dir: Option<String> = None;
    let mut size_pct: Option<String> = None;
    let mut size_cells: Option<String> = None;
    let mut sw_positional: Vec<String> = Vec::new();
    {
        let mut i = 1;
        while i < cmd_args.len() {
            let a = cmd_args[i].as_str();
            if a == "--" { sw_positional.extend(cmd_args[i+1..].iter().map(|s| s.to_string())); break; }
            match a {
                "-F" => { i += 1; if i < cmd_args.len() { format_str = Some(cmd_args[i].trim_matches('"').to_string()); } }
                "-c" => { i += 1; if i < cmd_args.len() { start_dir = Some(cmd_args[i].trim_matches('"').to_string()); } }
                "-p" => { i += 1; if i < cmd_args.len() { size_pct = Some(cmd_args[i].to_string()); size_cells = None; } }
                "-l" => { i += 1; if i < cmd_args.len() { let v = cmd_args[i].to_string(); if v.ends_with('%') { size_pct = Some(v); size_cells = None; } else { size_cells = Some(v); size_pct = None; } } }
                "-t" | "-e" => { i += 1; /* skip value */ }
                "-h" => { flag = "-h"; }
                "-v" => { flag = "-v"; }
                "-d" => { detached = true; }
                "-P" => { print_info = true; }
                "-b" | "-f" | "-I" | "-Z" => { /* ignored for compatibility */ }
                _ if a.starts_with('-') => { /* unknown flag, skip */ }
                _ => { sw_positional.extend(cmd_args[i..].iter().map(|s| s.to_string())); break; }
            }
            i += 1;
        }
    }
    let cmd_arg = sw_positional.join(" ");
    let cmd_arg = cmd_arg.as_str();
    let mut cmd_line = format!("split-window {}", flag);
    if detached { cmd_line.push_str(" -d"); }
    if print_info { cmd_line.push_str(" -P"); }
    if let Some(ref fmt) = format_str {
        cmd_line.push_str(&format!(" -F \"{}\"", fmt.replace("\"", "\\\"")));
    }
    if let Some(dir) = &start_dir {
        cmd_line.push_str(&format!(" -c \"{}\"", dir.replace("\"", "\\\"")));
    }
    if let Some(pct) = &size_pct {
        cmd_line.push_str(&format!(" -p {}", pct));
    } else if let Some(cells) = &size_cells {
        cmd_line.push_str(&format!(" -l {}", cells));
    }
    if !cmd_arg.is_empty() {
        cmd_line.push_str(&format!(" \"{}\"", cmd_arg.replace("\"", "\\\"")));
    }
    cmd_line.push('\n');
    if print_info {
        let resp = send_control_with_response(cmd_line)?;
        print!("{}", resp);
    } else {
        let resp = send_control_with_response(cmd_line)?;
        if !resp.is_empty() {
            eprint!("{}", resp);
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn handle_select_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "select-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            "-l" => { cmd.push_str(" -l"); }
            "-n" => { cmd.push_str(" -n"); }
            "-p" => { cmd.push_str(" -p"); }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_kill_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "kill-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            "-a" => { cmd.push_str(" -a"); }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_list_windows(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "list-windows".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-a" => { cmd.push_str(" -a"); }
            "-J" => { cmd.push_str(" -J"); }
            "-F" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -F \"{}\"", f.trim_matches('"').replace("\"", "\\\"")));
                    i += 1;
                }
            }
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
    let resp = send_control_with_response(cmd)?;
    print!("{}", resp);
    Ok(())
}

pub(crate) fn handle_rotate_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "rotate-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-D" => { cmd.push_str(" -D"); }
            "-U" => { cmd.push_str(" -U"); }
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

pub(crate) fn handle_break_pane(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "break-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-d" => { cmd.push_str(" -d"); }
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

pub(crate) fn handle_move_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "move-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-a" => { cmd.push_str(" -a"); }
            "-b" => { cmd.push_str(" -b"); }
            "-r" => { cmd.push_str(" -r"); }
            "-d" => { cmd.push_str(" -d"); }
            "-k" => { cmd.push_str(" -k"); }
            "-s" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -s {}", t));
                    i += 1;
                }
            }
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

pub(crate) fn handle_swap_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "swap-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-d" => { cmd.push_str(" -d"); }
            "-s" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -s {}", t));
                    i += 1;
                }
            }
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

pub(crate) fn handle_resize_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "resize-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-x" | "-y" => {
                if let Some(v) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" {} {}", cmd_args[i], v));
                    i += 1;
                }
            }
            "-t" => { i += 1; } // target handled globally
            "-A" | "-D" | "-U" => { cmd.push_str(&format!(" {}", cmd_args[i])); }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}
