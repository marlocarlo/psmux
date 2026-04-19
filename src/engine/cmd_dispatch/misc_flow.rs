use std::io;
use std::time::Duration;
use std::env;

use crate::session::{send_control, send_control_with_response};

pub(crate) fn handle_if_shell(cmd_args: &[&String]) -> io::Result<()> {
    let mut background = false;
    let mut condition: Option<String> = None;
    let mut cmd_true: Option<String> = None;
    let mut cmd_false: Option<String> = None;
    let mut format_mode = false;
    let mut i = 1;

    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-b" => { background = true; }
            "-F" => { format_mode = true; }
            "-t" => { i += 1; } // Skip target
            s if !s.starts_with('-') => {
                if condition.is_none() {
                    condition = Some(s.to_string());
                } else if cmd_true.is_none() {
                    cmd_true = Some(s.to_string());
                } else if cmd_false.is_none() {
                    cmd_false = Some(s.to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }

    if let (Some(cond), Some(true_cmd)) = (condition, cmd_true) {
        if background && !format_mode {
            // -b flag: run the condition check in a background thread
            let cmd_false_bg = cmd_false.clone();
            std::thread::spawn(move || {
                let success = {
                    let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
                    let mut c = std::process::Command::new(&shell_prog);
                    for a in &shell_args { c.arg(a); }
                    c.arg(&cond);
                    c.stdout(std::process::Stdio::null());
                    c.stderr(std::process::Stdio::null());
                    { use crate::platform::HideWindowCommandExt; c.hide_window(); }
                    c.status().map(|s| s.success()).unwrap_or(false)
                };
                let cmd_to_run = if success { Some(true_cmd) } else { cmd_false_bg };
                if let Some(cmd) = cmd_to_run {
                    let tcp_cmd = format!("{}\n", cmd);
                    let _ = send_control_with_response(tcp_cmd);
                }
            });
            // Return immediately — condition runs in background
            return Ok(());
        }

        let success = if format_mode {
            // Expand format string via server before evaluating
            let fmt_cmd = format!("display-message -p {}\n", crate::util::quote_arg(&cond));
            let expanded = send_control_with_response(fmt_cmd).unwrap_or_default();
            let expanded = expanded.trim_end_matches('\n');
            !expanded.is_empty() && expanded != "0"
        } else if cond == "true" || cond == "1" {
            true
        } else if cond == "false" || cond == "0" {
            false
        } else {
            // Run shell command - suppress stdout/stderr so it doesn't leak to terminal
            {
                let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
                let mut c = std::process::Command::new(&shell_prog);
                for a in &shell_args { c.arg(a); }
                c.arg(&cond);
                c.stdout(std::process::Stdio::null());
                c.stderr(std::process::Stdio::null());
                { use crate::platform::HideWindowCommandExt; c.hide_window(); }
                c.status().map(|s| s.success()).unwrap_or(false)
            }
        };

        let cmd_to_run = if success { Some(true_cmd) } else { cmd_false };

        if let Some(cmd) = cmd_to_run {
            let tcp_cmd = format!("{}\n", cmd);
            let resp = send_control_with_response(tcp_cmd)?;
            if !resp.is_empty() {
                print!("{}", resp);
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_wait_for(cmd_args: &[&String]) -> io::Result<()> {
    let mut lock = false;
    let mut signal = false;
    let mut unlock = false;
    let mut channel: Option<String> = None;
    let mut i = 1;

    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-L" => { lock = true; }
            "-S" => { signal = true; }
            "-U" => { unlock = true; }
            s if !s.starts_with('-') => { channel = Some(s.to_string()); }
            _ => {}
        }
        i += 1;
    }

    if let Some(ch) = channel {
        if signal {
            send_control(format!("wait-for -S {}\n", ch))?;
        } else if lock {
            send_control(format!("wait-for -L {}\n", ch))?;
        } else if unlock {
            send_control(format!("wait-for -U {}\n", ch))?;
        } else {
            // Wait for channel - this blocks
            let resp = send_control_with_response(format!("wait-for {}\n", ch))?;
            if !resp.is_empty() {
                print!("{}", resp);
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_set_environment(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "set-environment".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-g" => { cmd.push_str(" -g"); }
            "-r" => { cmd.push_str(" -r"); }
            "-u" => { cmd.push_str(" -u"); }
            "-h" => { cmd.push_str(" -h"); }
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

pub(crate) fn handle_show_environment(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "show-environment".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-g" => { cmd.push_str(" -g"); }
            "-s" => { cmd.push_str(" -s"); }
            "-h" => { cmd.push_str(" -h"); }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            s if !s.starts_with('-') => { cmd.push_str(&format!(" {}", s)); }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    let resp = send_control_with_response(cmd)?;
    print!("{}", resp);
    Ok(())
}

pub(crate) fn handle_start_server(l_socket_name: &Option<String>) -> io::Result<()> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let warm_base = if let Some(ref l) = l_socket_name {
        format!("{}____warm__", l)
    } else {
        "__warm__".to_string()
    };
    let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
    // Check if warm server is already running
    let already_running = if std::path::Path::new(&warm_port_path).exists() {
        if let Ok(port_str) = std::fs::read_to_string(&warm_port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                std::net::TcpStream::connect_timeout(
                    &format!("127.0.0.1:{}", port).parse().unwrap(),
                    Duration::from_millis(100),
                ).is_ok()
            } else { false }
        } else { false }
    } else { false };
    if already_running {
        return Ok(());
    }
    // Clean up stale port file if any
    let _ = std::fs::remove_file(&warm_port_path);
    // Spawn the warm server
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut server_args: Vec<String> = vec!["server".into(), "-s".into(), "__warm__".into()];
    if let Some(ref l) = l_socket_name {
        server_args.push("-L".into());
        server_args.push(l.clone());
    }
    // Detect terminal size for the warm server
    if let Ok((tw, th)) = crossterm::terminal::size() {
        let h = th.saturating_sub(1);
        if tw > 0 && h > 0 {
            server_args.push("-x".into());
            server_args.push(tw.to_string());
            server_args.push("-y".into());
            server_args.push(h.to_string());
        }
    }
    #[cfg(windows)]
    crate::platform::spawn_server_hidden(&exe, &server_args)?;
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new(&exe);
        for a in &server_args { cmd.arg(a); }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn warm server: {e}")))?;
    }
    Ok(())
}
