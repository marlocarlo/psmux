use std::io::{self, Write};

use crate::session::{send_control, send_control_with_response};

pub(crate) fn handle_display_message(cmd_args: &[&String]) -> io::Result<()> {
    let mut message: Vec<String> = Vec::new();
    let mut target: Option<String> = None;
    let mut print_to_stdout = false;
    let mut duration_ms: Option<u64> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    target = Some(t.to_string());
                    i += 1;
                }
            }
            "-p" => { print_to_stdout = true; }
            "-d" => {
                if let Some(val) = cmd_args.get(i + 1) {
                    duration_ms = val.parse::<u64>().ok();
                }
                i += 1;
            }
            "-I" => { i += 1; } // consume -I <input>, skip value
            s => { message.push(s.to_string()); }
        }
        i += 1;
    }
    let msg = message.join(" ");
    let mut cmd = "display-message".to_string();
    if let Some(t) = target { cmd.push_str(&format!(" -t {}", t)); }
    if print_to_stdout { cmd.push_str(" -p"); }
    if let Some(d) = duration_ms { cmd.push_str(&format!(" -d {}", d)); }
    cmd.push_str(&format!(" {}", msg));
    cmd.push('\n');
    if print_to_stdout {
        let resp = send_control_with_response(cmd)?;
        print!("{}", resp);
    } else {
        send_control(cmd)?;
    }
    Ok(())
}

pub(crate) fn handle_run_shell(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd_to_run: Vec<String> = Vec::new();
    let mut background = false;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-b" => { background = true; }
            s => { cmd_to_run.push(s.to_string()); }
        }
        i += 1;
    }
    let shell_cmd_str = cmd_to_run.join(" ");
    if shell_cmd_str.trim().is_empty() {
        eprintln!("usage: run-shell [-b] shell-command");
        std::process::exit(1);
    }
    let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd_str);
    // Run the command using the resolved shell
    if background {
        let mut c = crate::commands::build_run_shell_command(&shell_cmd);
        let _ = c.spawn();
    } else {
        let mut c = crate::commands::build_run_shell_command(&shell_cmd);
        let output = c.output()?;
        io::stdout().write_all(&output.stdout)?;
        io::stderr().write_all(&output.stderr)?;
        std::process::exit(output.status.code().unwrap_or(0));
    }
    Ok(())
}

pub(crate) fn handle_command_prompt(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "command-prompt".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-I" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -I {}", t));
                    i += 1;
                }
            }
            "-p" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -p {}", t));
                    i += 1;
                }
            }
            "-1" => { cmd.push_str(" -1"); }
            "-N" => { cmd.push_str(" -N"); }
            "-W" => { cmd.push_str(" -W"); }
            "-T" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -T {}", t));
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

pub(crate) fn handle_refresh_client(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "refresh-client".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-S" => { cmd.push_str(" -S"); }
            "-l" => { cmd.push_str(" -l"); }
            "-C" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -C {}", t));
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

pub(crate) fn handle_switch_client(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "switch-client".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-l" => { cmd.push_str(" -l"); }
            "-n" => { cmd.push_str(" -n"); }
            "-p" => { cmd.push_str(" -p"); }
            "-r" => { cmd.push_str(" -r"); }
            "-c" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -c {}", t));
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

pub(crate) fn handle_select_layout(cmd_args: &[&String]) -> io::Result<()> {
    let mut layout: Option<String> = None;
    let mut next = false;
    let mut prev = false;
    let mut i = 1;

    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-n" => { next = true; }
            "-p" => { prev = true; }
            "-o" => { /* last layout */ }
            "-E" => { /* spread evenly */ }
            "-t" => { i += 1; } // Skip target
            s if !s.starts_with('-') => { layout = Some(s.to_string()); }
            _ => {}
        }
        i += 1;
    }

    if next {
        send_control("next-layout\n".to_string())?;
    } else if prev {
        send_control("previous-layout\n".to_string())?;
    } else if let Some(l) = layout {
        send_control(format!("select-layout {}\n", l))?;
    } else {
        send_control("select-layout\n".to_string())?;
    }
    Ok(())
}

pub(crate) fn handle_pipe_pane(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "pipe-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-I" => { cmd.push_str(" -I"); }
            "-O" => { cmd.push_str(" -O"); }
            "-o" => { cmd.push_str(" -o"); }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            s => { cmd.push_str(&format!(" {}", s)); }
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_find_window(cmd_args: &[&String]) -> io::Result<()> {
    let mut pattern: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-C" | "-N" | "-T" | "-i" | "-r" | "-Z" => {}
            "-t" => { i += 1; }
            s if !s.starts_with('-') => { pattern = Some(s.to_string()); }
            _ => {}
        }
        i += 1;
    }
    if let Some(p) = pattern {
        let resp = send_control_with_response(format!("find-window {}\n", p))?;
        print!("{}", resp);
    }
    Ok(())
}
