use std::io::{self, Write, Read as _};

use crate::session::{send_control, send_control_with_response};

pub(crate) fn handle_paste_buffer(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "paste-buffer".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            "-b" => {
                if let Some(b) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -b {}", b));
                    i += 1;
                }
            }
            "-d" => { cmd.push_str(" -d"); }
            "-p" => { cmd.push_str(" -p"); }
            _ => {}
        }
        i += 1;
    }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_set_buffer(cmd_args: &[&String]) -> io::Result<()> {
    let mut buffer_name: Option<String> = None;
    let mut data: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-b" => {
                if let Some(b) = cmd_args.get(i + 1) {
                    buffer_name = Some(b.to_string());
                    i += 1;
                }
            }
            s if !s.starts_with('-') => {
                data = Some(s.to_string());
            }
            _ => {}
        }
        i += 1;
    }
    let mut cmd = "set-buffer".to_string();
    if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
    if let Some(d) = data { cmd.push_str(&format!(" {}", d)); }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_list_buffers(cmd_args: &[&String]) -> io::Result<()> {
    let mut format_str: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-F" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    format_str = Some(f.to_string());
                    i += 1;
                }
            }
            "-t" => { i += 1; } // skip target
            _ => {}
        }
        i += 1;
    }
    let cmd = if let Some(fmt) = format_str {
        format!("list-buffers -F {}\n", fmt)
    } else {
        "list-buffers\n".to_string()
    };
    let resp = send_control_with_response(cmd)?;
    print!("{}", resp);
    Ok(())
}

pub(crate) fn handle_show_buffer(cmd_args: &[&String]) -> io::Result<()> {
    let mut buffer_name: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-b" => {
                if let Some(b) = cmd_args.get(i + 1) {
                    buffer_name = Some(b.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let mut cmd = "show-buffer".to_string();
    if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
    cmd.push('\n');
    let resp = send_control_with_response(cmd)?;
    print!("{}", resp);
    Ok(())
}

pub(crate) fn handle_delete_buffer(cmd_args: &[&String]) -> io::Result<()> {
    let mut buffer_name: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-b" => {
                if let Some(b) = cmd_args.get(i + 1) {
                    buffer_name = Some(b.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let mut cmd = "delete-buffer".to_string();
    if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
    cmd.push('\n');
    send_control(cmd)?;
    Ok(())
}

pub(crate) fn handle_load_buffer(cmd_args: &[&String]) -> io::Result<()> {
    let mut buffer_name: Option<String> = None;
    let mut file_path: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-b" => {
                if let Some(b) = cmd_args.get(i + 1) {
                    buffer_name = Some(b.to_string());
                    i += 1;
                }
            }
            "-w" => {} // tmux 3.2+ clipboard propagation flag, silently accept
            "-" => { file_path = Some("-".to_string()); }
            s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
            _ => {}
        }
        i += 1;
    }
    if let Some(path) = file_path {
        let content = if path == "-" {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            input
        } else {
            std::fs::read_to_string(&path)?
        };
        let mut cmd = "set-buffer".to_string();
        if let Some(b) = buffer_name {
            cmd.push_str(&format!(" -b {}", b));
        }
        // Escape the content for transmission
        let escaped = content.replace('\n', "\\n").replace('\r', "\\r");
        cmd.push_str(&format!(" {}", escaped));
        cmd.push('\n');
        send_control(cmd)?;
    }
    Ok(())
}

pub(crate) fn handle_save_buffer(cmd_args: &[&String]) -> io::Result<()> {
    let mut buffer_name: Option<String> = None;
    let mut file_path: Option<String> = None;
    let mut append = false;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-a" => { append = true; }
            "-b" => {
                if let Some(b) = cmd_args.get(i + 1) {
                    buffer_name = Some(b.to_string());
                    i += 1;
                }
            }
            "-" => { file_path = Some("-".to_string()); }
            s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
            _ => {}
        }
        i += 1;
    }
    if let Some(path) = file_path {
        let mut cmd = "show-buffer".to_string();
        if let Some(b) = buffer_name {
            cmd.push_str(&format!(" -b {}", b));
        }
        cmd.push('\n');
        let content = send_control_with_response(cmd)?;
        if path == "-" {
            print!("{}", content);
        } else if append {
            use std::fs::OpenOptions;
            let mut file = OpenOptions::new().append(true).create(true).open(&path)?;
            file.write_all(content.as_bytes())?;
        } else {
            std::fs::write(&path, &content)?;
        }
    }
    Ok(())
}

pub(crate) fn handle_clear_history(cmd_args: &[&String]) -> io::Result<()> {
    let mut cmd = "clear-history".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-H" => { cmd.push_str(" -H"); }
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
