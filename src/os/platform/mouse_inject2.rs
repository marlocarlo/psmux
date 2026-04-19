#[allow(unused_imports)]
// ---------------------------------------------------------------------------
// CREATE_NO_WINDOW for background subprocesses
// ---------------------------------------------------------------------------

/// Windows `CREATE_NO_WINDOW` flag (0x08000000).
///
/// When set on `CreateProcess`, the child process does not get a console
/// window allocated by conhost.  This is the correct flag for *helper*
/// subprocesses (format `#()` expansion, `run-shell`, `if-shell`, clipboard
/// pipes, plugin scripts) that only need stdin/stdout/stderr pipes.
///
/// **Important:** PTY/ConPTY child processes and psmux server processes must
/// NOT use this flag because they need a real console session.  Those use
/// `spawn_server_hidden()` (with `CREATE_NEW_CONSOLE` + `SW_HIDE`) instead.
///
/// On non-Windows platforms this is a no-op.
#[cfg(windows)]
use super::*;

#[cfg(not(windows))]
pub mod mouse_inject {
    pub fn get_child_pid(_child: &dyn portable_pty::Child) -> Option<u32> { None }
    pub fn send_mouse_event(_pid: u32, _col: i16, _row: i16, _btn: u32, _flags: u32, _reattach: bool) -> bool { false }
    pub fn send_vt_sequence(_pid: u32, _sequence: &[u8]) -> bool { false }
    pub fn query_vti_enabled(_pid: u32) -> Option<bool> { None }
    pub fn send_ctrl_c_event(_pid: u32, _reattach: bool) -> bool { false }
    pub fn query_mouse_input_enabled(_pid: u32) -> Option<bool> { None }
    pub fn send_bracketed_paste(_pid: u32, _text: &str, _bracket: bool) -> bool { false }
    pub fn send_modified_key_event(_pid: u32, _ch: char, _ctrl: bool, _alt: bool, _shift: bool) -> bool { false }
    pub fn send_alt_key_event(_pid: u32, _ch: char) -> bool { false }
    pub fn send_modified_enter_event(_pid: u32, _ctrl: bool, _alt: bool, _shift: bool) -> bool { false }
}

// ---------------------------------------------------------------------------
// Process tree killing — ensures all descendant processes are terminated
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod process_kill {
    const TH32CS_SNAPPROCESS: u32 = 0x00000002;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
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

    /// Collect all descendant PIDs of `root_pid` (children, grandchildren, etc.).
    /// Uses a breadth-first traversal of the process tree snapshot.
    fn collect_descendants(root_pid: u32) -> Vec<u32> {
        let mut descendants = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE || snap == 0 { return descendants; }

            // Build full process table from snapshot
            let mut entries: Vec<(u32, u32)> = Vec::with_capacity(256); // (pid, parent_pid)
            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe) != 0 {
                entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                while Process32NextW(snap, &mut pe) != 0 {
                    entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                }
            }
            CloseHandle(snap);

            // BFS from root_pid
            let mut queue: Vec<u32> = vec![root_pid];
            let mut head = 0;
            while head < queue.len() {
                let parent = queue[head];
                head += 1;
                for &(pid, ppid) in &entries {
                    if ppid == parent && pid != root_pid && !queue.contains(&pid) {
                        queue.push(pid);
                        descendants.push(pid);
                    }
                }
            }
        }
        descendants
    }

    /// Force-terminate a single process by PID.
    fn terminate_pid(pid: u32) {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION, 0, pid);
            if h != 0 && h != INVALID_HANDLE {
                let _ = TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }

    /// Kill an entire process tree: all descendants first (leaves → root order),
    /// then the root process itself.  Calls `child.kill()` via portable_pty as a
    /// fallback.  Does NOT call `child.wait()` so `try_wait()` still works for
    /// the reaper (`prune_exited`), which will detect the dead process and clean
    /// up the tree node.
    ///
    /// This mirrors how tmux on Linux sends SIGKILL to the pane's process group.
    pub fn kill_process_tree(child: &mut Box<dyn portable_pty::Child>) {
        // Try to get the PID
        let pid = super::mouse_inject::get_child_pid(child.as_ref());

        if let Some(root_pid) = pid {
            // Collect all descendants, kill them leaf-first (reverse order)
            let mut descs = collect_descendants(root_pid);
            descs.reverse();
            for &dpid in &descs {
                terminate_pid(dpid);
            }
            // Kill the root process
            terminate_pid(root_pid);
        }

        // Fallback: tell portable_pty to kill the direct child process.
        // Do NOT call child.wait() here — the reaper (prune_exited) needs
        // try_wait() to detect the dead process and remove the tree node.
        let _ = child.kill();
    }

    /// Kill multiple process trees using a SINGLE process snapshot.
    /// Much faster than calling `kill_process_tree` N times when
    /// killing an entire session (avoids N separate system snapshots).
    pub fn kill_process_trees_batch(children: &mut [&mut Box<dyn portable_pty::Child>]) {
        // Collect all root PIDs
        let root_pids: Vec<Option<u32>> = children.iter()
            .map(|c| super::mouse_inject::get_child_pid(c.as_ref()))
            .collect();

        // Take ONE process snapshot for all trees
        let entries = snapshot_process_table();

        // For each root PID, find descendants using the shared snapshot
        for (i, root_pid_opt) in root_pids.iter().enumerate() {
            if let Some(root_pid) = root_pid_opt {
                let mut descs = collect_descendants_from_table(&entries, *root_pid);
                descs.reverse();
                for &dpid in &descs {
                    terminate_pid(dpid);
                }
                terminate_pid(*root_pid);
            }
            let _ = children[i].kill();
        }
    }

    /// Take a system-wide process snapshot and return the process table.
    fn snapshot_process_table() -> Vec<(u32, u32)> {
        let mut entries: Vec<(u32, u32)> = Vec::with_capacity(256);
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE || snap == 0 { return entries; }

            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snap, &mut pe) != 0 {
                entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                while Process32NextW(snap, &mut pe) != 0 {
                    entries.push((pe.th32_process_id, pe.th32_parent_process_id));
                }
            }
            CloseHandle(snap);
        }
        entries
    }

    /// BFS from root_pid using a pre-built process table.
    fn collect_descendants_from_table(entries: &[(u32, u32)], root_pid: u32) -> Vec<u32> {
        let mut descendants = Vec::new();
        let mut queue: Vec<u32> = vec![root_pid];
        let mut head = 0;
        while head < queue.len() {
            let parent = queue[head];
            head += 1;
            for &(pid, ppid) in entries {
                if ppid == parent && pid != root_pid && !queue.contains(&pid) {
                    queue.push(pid);
                    descendants.push(pid);
                }
            }
        }
        descendants
    }
}

#[cfg(not(windows))]
pub mod process_kill {
    /// On non-Windows, fall back to simple kill (no wait — let the reaper handle it).
    pub fn kill_process_tree(child: &mut Box<dyn portable_pty::Child>) {
        let _ = child.kill();
    }

    /// Batch kill — on non-Windows, just kill each child individually.
    pub fn kill_process_trees_batch(children: &mut [&mut Box<dyn portable_pty::Child>]) {
        for child in children.iter_mut() {
            let _ = child.kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Process info queries — get CWD and process name from PID (for format vars)
// ---------------------------------------------------------------------------
