#[cfg(windows)]
use super::*;

// ── Native MOUSE_EVENT → crossterm Event conversion ──────────────────

#[cfg(windows)]
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct SshMouseEventRecord {
    pub mouse_x: i16,
    pub mouse_y: i16,
    pub button_state: u32,
    pub control_key_state: u32,
    pub event_flags: u32,
}

#[cfg(windows)]
const FROM_LEFT_1ST: u32 = 0x0001;
#[cfg(windows)]
const RIGHTMOST: u32     = 0x0002;
#[cfg(windows)]
const FROM_LEFT_2ND: u32 = 0x0004;
#[cfg(windows)]
const ME_MOVED: u32      = 0x0001;
#[cfg(windows)]
const ME_WHEELED: u32    = 0x0004;

#[cfg(windows)]
pub(crate) fn convert_native_mouse(rec: &SshMouseEventRecord) -> Option<Event> {
    let col = rec.mouse_x.max(0) as u16;
    let row = rec.mouse_y.max(0) as u16;
    let mods = {
        let s = rec.control_key_state;
        let mut m = KeyModifiers::empty();
        if s & 0x0010 != 0 { m |= KeyModifiers::SHIFT; } // SHIFT_PRESSED
        if s & (0x0001 | 0x0002) != 0 { m |= KeyModifiers::ALT; } // LEFT/RIGHT_ALT
        if s & (0x0004 | 0x0008) != 0 { m |= KeyModifiers::CONTROL; } // LEFT/RIGHT_CTRL
        m
    };

    if rec.event_flags & ME_WHEELED != 0 {
        let delta = (rec.button_state >> 16) as i16;
        let kind = if delta > 0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown };
        return Some(Event::Mouse(MouseEvent { kind, column: col, row, modifiers: mods }));
    }

    if rec.event_flags & ME_MOVED != 0 {
        if rec.button_state & FROM_LEFT_1ST != 0 {
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: col, row, modifiers: mods }));
        }
        if rec.button_state & RIGHTMOST != 0 {
            return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Right), column: col, row, modifiers: mods }));
        }
        return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Moved, column: col, row, modifiers: mods }));
    }

    if rec.button_state & FROM_LEFT_1ST != 0 {
        return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: col, row, modifiers: mods }));
    }
    if rec.button_state & RIGHTMOST != 0 {
        return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: col, row, modifiers: mods }));
    }
    if rec.button_state & FROM_LEFT_2ND != 0 {
        return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Middle), column: col, row, modifiers: mods }));
    }

    // button_state == 0  → all buttons released
    if rec.button_state == 0 && rec.event_flags == 0 {
        return Some(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: col, row, modifiers: mods }));
    }

    None
}

/// Startup diagnostics: log Windows version and SSH environment variables.
#[cfg(windows)]
pub(crate) fn log_ssh_startup() {
    ssh_debug_log("=== psmux SSH input module starting ===");
    // Log Windows version
    {
        #[repr(C)]
        struct OSVERSIONINFOW {
            os_version_info_size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform_id: u32,
            sz_csd_version: [u16; 128],
        }
        #[link(name = "ntdll")]
        extern "system" {
            fn RtlGetVersion(info: *mut OSVERSIONINFOW) -> i32;
        }
        let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
        info.os_version_info_size = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        unsafe { RtlGetVersion(&mut info) };
        ssh_debug_log(&format!(
            "Windows {}.{} build {}",
            info.major, info.minor, info.build,
        ));
        // ConPTY mouse support requires Windows 11 build 22523+.
        // On older builds, ConPTY's VT parser discards SGR mouse input
        // sequences and does not forward DECSET to the SSH client.
        if info.build < 22523 {
            ssh_debug_log(&format!(
                "WARNING: Windows build {} < 22523 — ConPTY does NOT support \
                 mouse over SSH. Mouse clicks will not work. \
                 Upgrade to Windows 11 22H2+ for SSH mouse support.",
                info.build,
            ));
        } else {
            ssh_debug_log("ConPTY build >= 22523 — mouse over SSH should be supported");
        }
    }
    // Log SSH env vars
    for var in &["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
        if let Ok(val) = std::env::var(var) {
            ssh_debug_log(&format!("  {}={}", var, val));
        }
    }
}
