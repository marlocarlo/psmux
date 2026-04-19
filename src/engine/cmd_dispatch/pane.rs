use std::io;

use crate::session::{send_control, send_control_with_response};

pub(crate) fn handle_select_pane(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "select-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            "-T" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -T \"{}\"", t));
                    i += 1;
                }
            }
            "-P" => {
                if let Some(s) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -P \"{}\"", s));
                    i += 1;
                }
            }
            "-D" => { cmd.push_str(" -D"); }
            "-U" => { cmd.push_str(" -U"); }
            "-L" => { cmd.push_str(" -L"); }
            "-R" => { cmd.push_str(" -R"); }
            "-l" => { cmd.push_str(" -l"); }
            "-Z" => { cmd.push_str(" -Z"); }
            "-m" => { cmd.push_str(" -m"); }
            "-M" => { cmd.push_str(" -M"); }
            "-e" => { cmd.push_str(" -e"); }
            "-d" => { cmd.push_str(" -d"); }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_list_panes(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "list-panes".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-a" => { cmd.push_str(" -a"); }
            "-s" => { cmd.push_str(" -s"); }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            "-F" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -F \"{}\"", f.trim_matches('"').replace("\"", "\\\"")));
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

pub(crate) fn handle_capture_pane(cmd_args: &[&String]) -> io::Result<()> {
    // Parse optional flags - cmd_args[0] is command, start from 1
    let mut cmd = "capture-pane".to_string();
    let mut print_stdout = false;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(target) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", target));
                    i += 1;
                }
            }
            "-S" => {
                if let Some(start) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -S {}", start));
                    i += 1;
                }
            }
            "-E" => {
                if let Some(end) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -E {}", end));
                    i += 1;
                }
            }
            "-p" => { cmd.push_str(" -p"); print_stdout = true; }
            "-e" => { cmd.push_str(" -e"); }
            "-J" => { cmd.push_str(" -J"); }
            "-b" => {
                if let Some(buf) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -b {}", buf));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    if print_stdout {
        let resp = send_control_with_response(cmd)?;
        print!("{}", resp);
    } else {
        send_control(cmd)?;
    }
    Ok(())
}

pub(crate) fn handle_swap_pane(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "swap-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-D" => { cmd.push_str(" -D"); }
            "-U" => { cmd.push_str(" -U"); }
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

pub(crate) fn handle_resize_pane(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "resize-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-D" => { cmd.push_str(" -D"); }
            "-U" => { cmd.push_str(" -U"); }
            "-L" => { cmd.push_str(" -L"); }
            "-R" => { cmd.push_str(" -R"); }
            "-Z" => { cmd.push_str(" -Z"); }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            "-x" => {
                if let Some(v) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -x {}", v));
                    i += 1;
                }
            }
            "-y" => {
                if let Some(v) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -y {}", v));
                    i += 1;
                }
            }
            s if s.parse::<i32>().is_ok() => {
                cmd.push_str(&format!(" {}", s));
            }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_join_pane(cmd_args: &[&String]) -> io::Result<()> {
    // Parse args to detect cross-session scenario
    let mut source_spec = String::new();
    let mut horizontal = false;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-h" => horizontal = true,
            "-v" => {} // vertical is default
            "-d" => {} // detach (ignored at CLI level)
            "-s" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    source_spec = t.to_string();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    // Get -t from the saved env var (global handler stripped it from cmd_args)
    let target_spec = std::env::var("PSMUX_TARGET_FULL").unwrap_or_default();
    // Check if source and target reference different sessions
    let src_session = if source_spec.contains(':') {
        source_spec.split(':').next().unwrap_or("").to_string()
    } else {
        String::new()
    };
    let tgt_session = if target_spec.contains(':') {
        target_spec.split(':').next().unwrap_or("").to_string()
    } else {
        String::new()
    };
    let current_session = std::env::var("PSMUX_TARGET_SESSION")
        .or_else(|_| std::env::var("PSMUX_SESSION"))
        .unwrap_or_default();
    let effective_src = if src_session.is_empty() { current_session.clone() } else { src_session.clone() };
    let effective_tgt = if tgt_session.is_empty() { current_session.clone() } else { tgt_session.clone() };
    if !effective_src.is_empty() && !effective_tgt.is_empty() && effective_src != effective_tgt {
        // Cross-session join-pane: orchestrate via TCP
        let src_after_colon = if source_spec.contains(':') {
            source_spec.split(':').nth(1).unwrap_or("0.0")
        } else if !source_spec.is_empty() {
            &source_spec
        } else {
            "0.0"
        };
        let tgt_after_colon = if target_spec.contains(':') {
            target_spec.split(':').nth(1).unwrap_or("")
        } else if !target_spec.is_empty() {
            &target_spec
        } else {
            ""
        };
        let sp = crate::cli::parse_target(src_after_colon);
        let tp = crate::cli::parse_target(tgt_after_colon);
        match crate::cross_session::orchestrate_cross_session_join(
            &effective_src,
            sp.window.unwrap_or(0),
            sp.pane.unwrap_or(0),
            &effective_tgt,
            tp.window,
            tp.pane,
            horizontal,
        ) {
            Ok(()) => {}
            Err(e) => eprintln!("psmux: cross-session join-pane failed: {}", e),
        }
    } else {
        // Same-session join-pane: forward to server as before
        let mut cmd = "join-pane".to_string();
        if horizontal { cmd.push_str(" -h"); }
        if !source_spec.is_empty() { cmd.push_str(&format!(" -s {}", source_spec)); }
        if !target_spec.is_empty() { cmd.push_str(&format!(" -t {}", target_spec)); }
        cmd.push('\n');
        send_control(cmd)?;
    }
    Ok(())
}

pub(crate) fn handle_respawn_pane(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "respawn-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-k" => { cmd.push_str(" -k"); }
            "-c" => {
                if let Some(d) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -c {}", d));
                    i += 1;
                }
            }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            _ => { cmd.push_str(&format!(" {}", cmd_args[i])); }
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}
