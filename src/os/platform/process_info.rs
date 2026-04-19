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

#[cfg(windows)]
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_VM_READ: u32 = 0x0010;
const MAX_PATH: usize = 260;
const TH32CS_SNAPPROCESS: u32 = 0x00000002;
const INVALID_HANDLE: isize = -1;

#[allow(non_snake_case)]
#[repr(C)]
struct PROCESS_BASIC_INFORMATION {
    Reserved1: isize,
    PebBaseAddress: isize, // pointer to PEB
    Reserved2: [isize; 2],
    UniqueProcessId: isize,
    Reserved3: isize,
}

#[allow(non_snake_case)]
#[repr(C)]
struct UNICODE_STRING {
    Length: u16,
    MaximumLength: u16,
    Buffer: isize, // pointer to wide string
}

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
    fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn QueryFullProcessImageNameW(h: isize, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn ReadProcessMemory(
        h_process: isize,
        base_address: isize,
        buffer: *mut u8,
        size: usize,
        bytes_read: *mut usize,
    ) -> i32;
    fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> isize;
    fn Process32FirstW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
    fn Process32NextW(h_snapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(
        process_handle: isize,
        process_information_class: u32,
        process_information: *mut u8,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

/// Get the executable name of a process by PID (e.g. "pwsh" or "vim").
pub fn get_process_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 || h == -1 { return None; }
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size);
        CloseHandle(h);
        if ok == 0 { return None; }
        let full_path = OsString::from_wide(&buf[..size as usize])
            .to_string_lossy()
            .into_owned();
        let name = std::path::Path::new(&full_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())?;
        Some(name)
    }
}

/// Get the current working directory of a process by PID.
/// Reads the PEB → ProcessParameters → CurrentDirectory from the target process.
pub fn get_process_cwd(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if h == 0 || h == -1 { return None; }
        let result = read_process_cwd(h);
        CloseHandle(h);
        result
    }
}

/// Read CWD from a process handle via NtQueryInformationProcess + ReadProcessMemory.
unsafe fn read_process_cwd(h: isize) -> Option<String> {
    // Step 1: Get PEB address
    let mut pbi: PROCESS_BASIC_INFORMATION = std::mem::zeroed();
    let mut ret_len: u32 = 0;
    let status = NtQueryInformationProcess(
        h,
        0, // ProcessBasicInformation
        &mut pbi as *mut _ as *mut u8,
        std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
        &mut ret_len,
    );
    if status != 0 { return None; }
    let peb_addr = pbi.PebBaseAddress;
    if peb_addr == 0 { return None; }

    // Step 2: Read ProcessParameters pointer from PEB.
    // PEB layout (x64): offset 0x20 = ProcessParameters pointer
    // PEB layout (x86): offset 0x10 = ProcessParameters pointer
    let params_ptr_offset = if std::mem::size_of::<usize>() == 8 { 0x20 } else { 0x10 };
    let mut process_params_ptr: isize = 0;
    let mut bytes_read: usize = 0;
    let ok = ReadProcessMemory(
        h,
        peb_addr + params_ptr_offset,
        &mut process_params_ptr as *mut isize as *mut u8,
        std::mem::size_of::<isize>(),
        &mut bytes_read,
    );
    if ok == 0 || process_params_ptr == 0 { return None; }

    // Step 3: Read CurrentDirectory.DosPath (UNICODE_STRING) from RTL_USER_PROCESS_PARAMETERS.
    // x64 offset: 0x38 = CurrentDirectory.DosPath
    // x86 offset: 0x24 = CurrentDirectory.DosPath
    let cwd_offset = if std::mem::size_of::<usize>() == 8 { 0x38 } else { 0x24 };
    let mut cwd_ustr: UNICODE_STRING = std::mem::zeroed();
    let ok = ReadProcessMemory(
        h,
        process_params_ptr + cwd_offset,
        &mut cwd_ustr as *mut UNICODE_STRING as *mut u8,
        std::mem::size_of::<UNICODE_STRING>(),
        &mut bytes_read,
    );
    if ok == 0 || cwd_ustr.Length == 0 || cwd_ustr.Buffer == 0 { return None; }

    // Step 4: Read the actual CWD wide string
    let char_count = (cwd_ustr.Length / 2) as usize;
    let mut wchars: Vec<u16> = vec![0u16; char_count];
    let ok = ReadProcessMemory(
        h,
        cwd_ustr.Buffer,
        wchars.as_mut_ptr() as *mut u8,
        cwd_ustr.Length as usize,
        &mut bytes_read,
    );
    if ok == 0 { return None; }

    let path = OsString::from_wide(&wchars)
        .to_string_lossy()
        .into_owned();
    // Remove trailing backslash (tmux convention)
    Some(path.trim_end_matches('\\').to_string())
}

/// Get the name of the foreground process in the pane.
/// Walks the process tree from the shell PID to find the deepest
/// non-system descendant (the user's foreground command).
pub fn get_foreground_process_name(pid: u32) -> Option<String> {
    // Walk the process tree to find the foreground child.
    let result = find_foreground_child_pid(pid);
    match result {
        Some(target) if target != pid => {
            let name = get_process_name(target);
            autorename_log(&format!("pid={} fg_child={} name={:?}", pid, target, name));
            if let Some(n) = name {
                return Some(n);
            }
        }
        Some(_) => {
            autorename_log(&format!("pid={} fg_child=self (no children)", pid));
        }
        None => {
            autorename_log(&format!("pid={} fg_child=None (BFS found nothing)", pid));
        }
    }
    // No foreground child found.  Return None so the caller can
    // preserve the current window name instead of briefly flashing
    // to the shell name before the child process has spawned
    // (issue #229).
    autorename_log(&format!("pid={} no_foreground_child", pid));
    None
}

/// Get the CWD of the foreground process in the pane.
pub fn get_foreground_cwd(pid: u32) -> Option<String> {
    if let Some(target) = find_foreground_child_pid(pid) {
        if target != pid {
            if let Some(cwd) = get_process_cwd(target) {
                return Some(cwd);
            }
        }
    }
    get_process_cwd(pid)
}

/// Known system/infrastructure processes that should be skipped when
/// walking the process tree to find the user's foreground command.
fn is_system_exe(name: &str) -> bool {
    matches!(name,
        "conhost.exe" | "csrss.exe" | "dwm.exe" | "services.exe"
        | "svchost.exe" | "wininit.exe" | "winlogon.exe"
        | "openconsole.exe" | "runtimebroker.exe"
    )
}

/// Walk the process tree from `root_pid` downward and return the PID of
/// the process most likely to be the user's foreground command.
///
/// Strategy: BFS all descendants, then pick the deepest non-system leaf.
/// When multiple candidates exist at the same depth, prefer the largest
/// PID (heuristic for "most recently created").
fn find_foreground_child_pid(root_pid: u32) -> Option<u32> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE || snap == 0 {
            autorename_log(&format!("root={} SNAPSHOT FAILED", root_pid));
            return None;
        }

        // Collect (pid, ppid, exe_name_lower) for every process.
        let mut entries: Vec<(u32, u32, String)> = Vec::with_capacity(512);
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut pe) != 0 {
            let name = exe_name_from_entry(&pe);
            entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
            while Process32NextW(snap, &mut pe) != 0 {
                let name = exe_name_from_entry(&pe);
                entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
            }
        }
        CloseHandle(snap);

        autorename_log(&format!("root={} snapshot_entries={}", root_pid, entries.len()));

        // Log direct children of root_pid
        let direct: Vec<_> = entries.iter()
            .filter(|(_, ppid, _)| *ppid == root_pid)
            .collect();
        for (pid, _, name) in &direct {
            autorename_log(&format!("  direct_child: pid={} name={}", pid, name));
        }

        // BFS: collect all descendants with their depth.
        // Each entry is (pid, exe_name, depth).
        let mut descendants: Vec<(u32, String, u32)> = Vec::new();
        let mut queue: Vec<(u32, u32)> = vec![(root_pid, 0)]; // (pid, depth)
        let mut head = 0;
        while head < queue.len() {
            let (parent, depth) = queue[head];
            head += 1;
            for (pid, ppid, name) in &entries {
                if *ppid == parent && *pid != root_pid
                    && !descendants.iter().any(|(p, _, _)| p == pid)
                {
                    descendants.push((*pid, name.clone(), depth + 1));
                    queue.push((*pid, depth + 1));
                }
            }
        }

        autorename_log(&format!("root={} descendants={}", root_pid, descendants.len()));
        for (pid, name, depth) in &descendants {
            autorename_log(&format!("  desc: pid={} name={} depth={}", pid, name, depth));
        }

        if descendants.is_empty() {
            return None;
        }

        // A "leaf" is a descendant that has no children in our descendant set.
        let desc_pids: std::collections::HashSet<u32> =
            descendants.iter().map(|(p, _, _)| *p).collect();
        let leaves: Vec<(u32, &str, u32)> = descendants.iter()
            .filter(|(pid, _, _)| {
                // No entry in the process table has this pid as parent
                // while also being in our descendant set.
                !entries.iter().any(|(ep, eppid, _)| *eppid == *pid && desc_pids.contains(ep))
            })
            .map(|(pid, name, depth)| (*pid, name.as_str(), *depth))
            .collect();

        // Choose from leaves if available, otherwise from all descendants.
        let pool: Vec<(u32, &str, u32)> = if !leaves.is_empty() {
            leaves
        } else {
            descendants.iter().map(|(p, n, d)| (*p, n.as_str(), *d)).collect()
        };

        // Prefer non-system candidates.
        let user_pool: Vec<&(u32, &str, u32)> = pool.iter()
            .filter(|(_, name, _)| !is_system_exe(name))
            .collect();

        let selection = if !user_pool.is_empty() { user_pool } else { pool.iter().collect() };

        // Deepest first, then largest PID as tiebreaker.
        let result = selection.iter()
            .max_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)))
            .map(|(pid, _, _)| *pid);

        autorename_log(&format!("root={} selected={:?}", root_pid, result));
        result
    }
}

/// Extract the lowercased executable name from a PROCESSENTRY32W.
fn exe_name_from_entry(pe: &PROCESSENTRY32W) -> String {
    let nul = pe.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(pe.sz_exe_file.len());
    String::from_utf16_lossy(&pe.sz_exe_file[..nul]).to_lowercase()
}

/// Check if an executable name is a VT bridge process (WSL, SSH, etc.)
/// that requires VT mouse injection instead of Win32 console injection.
fn is_vt_bridge_exe(name: &str) -> bool {
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    matches!(stem, "wsl" | "ssh" | "ubuntu" | "debian" | "kali"
                  | "fedoraremix" | "opensuse-leap" | "sles" | "arch")
        || stem.starts_with("wsl")
}

/// Walk the process tree from `root_pid` and check if any descendant
/// is a VT bridge process (wsl.exe, ssh.exe, etc.).
/// This is used for mouse injection: VT bridge processes need VT mouse
/// sequences written to the PTY master, not Win32 MOUSE_EVENT records.
pub fn has_vt_bridge_descendant(root_pid: u32) -> bool {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE || snap == 0 { return false; }

        let mut entries: Vec<(u32, u32, String)> = Vec::with_capacity(256);
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut pe) != 0 {
            let name = exe_name_from_entry(&pe);
            entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
            while Process32NextW(snap, &mut pe) != 0 {
                let name = exe_name_from_entry(&pe);
                entries.push((pe.th32_process_id, pe.th32_parent_process_id, name));
            }
        }
        CloseHandle(snap);

        // BFS from root_pid to check all descendants
        let mut queue: Vec<u32> = vec![root_pid];
        let mut head = 0;
        while head < queue.len() {
            let parent = queue[head];
            head += 1;
            for (pid, ppid, name) in &entries {
                if *ppid == parent && *pid != root_pid
                    && !queue.contains(pid)
                {
                    if is_vt_bridge_exe(name) {
                        return true;
                    }
                    queue.push(*pid);
                }
            }
        }
        false
    }
}