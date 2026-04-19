#[allow(unused_imports)]
use std::io::{self, Write};
use std::time::Duration;
use std::env;

/// Returns true if this port-file base name belongs to a warm (standby) server.
/// Warm sessions should be hidden from user-facing lists and never auto-attached.
use super::*;

/// List all running sessions and their windows for choose-tree display.
/// Queries each running server via its TCP port for window list info.
pub fn list_all_sessions_tree(current_session: &str, current_windows: &[(String, usize, String, bool)]) -> Vec<TreeEntry> {
    let home = match env::var("USERPROFILE").or_else(|_| env::var("HOME")) {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let psmux_dir = format!("{}\\.psmux", home);
    let mut sessions: Vec<(String, u16, std::time::SystemTime)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "port").unwrap_or(false) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    // Hide warm (standby) sessions from choose-tree
                    if is_warm_session(stem) { continue; }
                    if let Ok(port_str) = std::fs::read_to_string(&path) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let mtime = entry.metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                            sessions.push((stem.to_string(), port, mtime));
                        }
                    }
                }
            }
        }
    }

    sessions.sort_by_key(|(name, _, _)| name.clone());

    let mut tree = Vec::new();
    for (name, port, _) in &sessions {
        let is_current = name == current_session;
        // Session header
        tree.push(TreeEntry {
            session_name: name.clone(),
            session_port: *port,
            is_session_header: true,
            window_index: None,
            window_name: String::new(),
            window_panes: 0,
            window_size: String::new(),
            is_current_session: is_current,
            is_active_window: false,
        });

        if is_current {
            // Use local data for the current session (fast, no IPC)
            for (i, (wname, panes, size, is_active)) in current_windows.iter().enumerate() {
                tree.push(TreeEntry {
                    session_name: name.clone(),
                    session_port: *port,
                    is_session_header: false,
                    window_index: Some(i),
                    window_name: wname.clone(),
                    window_panes: *panes,
                    window_size: size.clone(),
                    is_current_session: true,
                    is_active_window: *is_active,
                });
            }
        } else {
            // Query remote session for its window list
            let key = read_session_key(name).unwrap_or_default();
            let addr = format!("127.0.0.1:{}", port);
            if let Ok(resp) = send_auth_cmd_response(&addr, &key, b"list-windows -F \"#{window_index}:#{window_name}:#{window_panes}:#{window_width}x#{window_height}:#{window_active}\"\n") {
                for line in resp.lines() {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let parts: Vec<&str> = line.splitn(5, ':').collect();
                    if parts.len() >= 5 {
                        let wi = parts[0].parse::<usize>().unwrap_or(0);
                        let wn = parts[1].to_string();
                        let wp = parts[2].parse::<usize>().unwrap_or(1);
                        let ws = parts[3].to_string();
                        let wa = parts[4] == "1";
                        tree.push(TreeEntry {
                            session_name: name.clone(),
                            session_port: *port,
                            is_session_header: false,
                            window_index: Some(wi),
                            window_name: wn,
                            window_panes: wp,
                            window_size: ws,
                            is_current_session: false,
                            is_active_window: wa,
                        });
                    }
                }
            }
        }
    }
    tree
}

/// Force-kill any remaining psmux/pmux/tmux server processes that didn't
/// exit via the TCP kill-server command.  This is the nuclear fallback that
/// guarantees kill-server always succeeds.
///
/// On Windows, uses CreateToolhelp32Snapshot to enumerate processes and
/// TerminateProcess to kill them.  Skips the current process.
#[cfg(windows)]
pub fn kill_remaining_server_processes() {
    const TH32CS_SNAPPROCESS: u32 = 0x00000002;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const INVALID_HANDLE: isize = -1;

    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> isize;
        fn Process32FirstW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> isize;
        fn TerminateProcess(h_process: isize, exit_code: u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    let my_pid = std::process::id();

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE || snap == 0 { return; }

        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let target_names: &[&str] = &["psmux.exe", "pmux.exe", "tmux.exe"];
        let mut pids_to_kill: Vec<u32> = Vec::new();

        if Process32FirstW(snap, &mut pe) != 0 {
            loop {
                let pid = pe.th32_process_id;
                if pid != my_pid {
                    // Extract exe name from wide string
                    let len = pe.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(260);
                    let name = String::from_utf16_lossy(&pe.sz_exe_file[..len]);
                    let name_lower = name.to_lowercase();
                    for target in target_names {
                        if name_lower == *target || name_lower.ends_with(&format!("\\{}", target)) {
                            pids_to_kill.push(pid);
                            break;
                        }
                    }
                }
                if Process32NextW(snap, &mut pe) == 0 { break; }
            }
        }
        CloseHandle(snap);

        for pid in &pids_to_kill {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, *pid);
            if h != 0 && h != INVALID_HANDLE {
                let _ = TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn kill_remaining_server_processes() {
    // On non-Windows, use signal-based killing
    let _ = std::process::Command::new("pkill")
        .args(&["-f", "psmux|pmux"])
        .status();
}
