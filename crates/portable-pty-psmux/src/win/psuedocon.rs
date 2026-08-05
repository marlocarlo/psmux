use super::WinChild;
use crate::cmdbuilder::CommandBuilder;
use crate::win::procthreadattr::ProcThreadAttributeList;
use anyhow::{bail, ensure, Error};
use filedescriptor::{FileDescriptor, OwnedHandle};
use lazy_static::lazy_static;
use shared_library::shared_library;
use std::ffi::OsString;
use std::io::Error as IoError;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::sync::Mutex;
use std::{mem, ptr};
use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::{HRESULT, S_OK};
use winapi::um::handleapi::*;
use winapi::um::processthreadsapi::*;
use winapi::um::winbase::{
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, STARTUPINFOEXW,
};
use winapi::um::wincon::COORD;
use winapi::um::winnt::HANDLE;

pub type HPCON = HANDLE;

/// Deliberately absent from base_flags() (see the doc comment there): only the
/// regression test asserting that absence references it, hence test-gated.
#[cfg(test)]
pub const PSUEDOCONSOLE_INHERIT_CURSOR: DWORD = 0x1;
pub const PSEUDOCONSOLE_RESIZE_QUIRK: DWORD = 0x2;
pub const PSEUDOCONSOLE_WIN32_INPUT_MODE: DWORD = 0x4;
pub const PSEUDOCONSOLE_PASSTHROUGH_MODE: DWORD = 0x8;

shared_library!(ConPtyFuncs,
    pub fn CreatePseudoConsole(
        size: COORD,
        hInput: HANDLE,
        hOutput: HANDLE,
        flags: DWORD,
        hpc: *mut HPCON
    ) -> HRESULT,
    pub fn ResizePseudoConsole(hpc: HPCON, size: COORD) -> HRESULT,
    pub fn ClosePseudoConsole(hpc: HPCON),
);

fn load_conpty() -> ConPtyFuncs {
    // Sideloading rules:
    //
    // - Never resolve "conpty.dll" through the default DLL search order:
    //   terminal emulators like WezTerm bundle their own conpty.dll +
    //   OpenConsole.exe and the search order can pick those up when psmux
    //   runs inside such a terminal, yielding blank panes / broken I/O with
    //   our flag set (PASSTHROUGH_MODE, WIN32_INPUT_MODE, etc.).
    //
    // - Absolute paths we control are fine, and are the standard way
    //   (Windows Terminal / VS Code / WezTerm all ship their own conpty) to
    //   escape bugs in the in-box conhost.exe, which only updates with the
    //   OS.  Precedence:
    //     1. PSMUX_CONPTY_DLL=<absolute path> (diagnostic override)
    //     2. conpty.dll next to the psmux executable, only when the matching
    //        OpenConsole.exe is present beside it (conpty.dll spawns
    //        OpenConsole.exe from its own directory)
    //     3. the system kernel32.dll implementation
    if let Ok(path) = std::env::var("PSMUX_CONPTY_DLL") {
        if !path.is_empty() {
            match ConPtyFuncs::open(Path::new(&path)) {
                Ok(funcs) => {
                    log::info!("using ConPTY implementation from {path}");
                    return funcs;
                }
                Err(err) => {
                    log::warn!("PSMUX_CONPTY_DLL={path} failed to load ({err:?}), falling back");
                }
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let dll = dir.join("conpty.dll");
            let open_console = dir.join("OpenConsole.exe");
            if dll.is_file() && open_console.is_file() {
                match ConPtyFuncs::open(&dll) {
                    Ok(funcs) => {
                        log::info!("using bundled ConPTY implementation from {}", dll.display());
                        return funcs;
                    }
                    Err(err) => {
                        log::warn!(
                            "bundled {} failed to load ({err:?}), falling back to kernel32",
                            dll.display()
                        );
                    }
                }
            }
        }
    }
    ConPtyFuncs::open(Path::new("kernel32.dll")).expect(
        "this system does not support conpty.  Windows 10 October 2018 or newer is required",
    )
}

lazy_static! {
    static ref CONPTY: ConPtyFuncs = load_conpty();
}

pub struct PsuedoCon {
    con: HPCON,
    /// Whether this ConPTY was created with PSEUDOCONSOLE_PASSTHROUGH_MODE.
    /// Used by the retry logic in ConPtySlavePty::spawn_command to decide
    /// whether a fallback without passthrough is worth attempting.
    pub used_passthrough: bool,
}

unsafe impl Send for PsuedoCon {}
unsafe impl Sync for PsuedoCon {}

impl Drop for PsuedoCon {
    fn drop(&mut self) {
        unsafe { (CONPTY.ClosePseudoConsole)(self.con) };
    }
}

/// Returns true if the current Windows build supports ConPTY passthrough mode.
/// PSEUDOCONSOLE_PASSTHROUGH_MODE requires Windows 11 22H2 (build 22621+).
/// On older Windows versions, the flag may be silently accepted but produce
/// broken ConPTY output (no Win32 Console API translation).
///
/// Respects `PSMUX_NO_PASSTHROUGH=1` environment variable to let users
/// force-disable passthrough mode on builds where it causes CreateProcessW
/// to fail with ERROR_INVALID_PARAMETER (87).
fn supports_passthrough_mode() -> bool {
    if std::env::var("PSMUX_NO_PASSTHROUGH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        log::info!("ConPTY passthrough mode disabled via PSMUX_NO_PASSTHROUGH");
        return false;
    }
    let ver = unsafe {
        let mut info: winapi::um::winnt::OSVERSIONINFOW = mem::zeroed();
        info.dwOSVersionInfoSize = mem::size_of::<winapi::um::winnt::OSVERSIONINFOW>() as u32;
        // RtlGetVersion is used because GetVersionEx lies on Windows 10+
        // unless the application has a compatibility manifest.
        type RtlGetVersionFn = unsafe extern "system" fn(*mut winapi::um::winnt::OSVERSIONINFOW) -> i32;
        let ntdll = winapi::um::libloaderapi::GetModuleHandleW(
            ['n' as u16, 't' as u16, 'd' as u16, 'l' as u16, 'l' as u16, '.' as u16,
             'd' as u16, 'l' as u16, 'l' as u16, 0].as_ptr()
        );
        if ntdll.is_null() {
            return false;
        }
        let func = winapi::um::libloaderapi::GetProcAddress(
            ntdll,
            b"RtlGetVersion\0".as_ptr() as *const i8,
        );
        if func.is_null() {
            return false;
        }
        let rtl_get_version: RtlGetVersionFn = mem::transmute(func);
        rtl_get_version(&mut info);
        info
    };
    // Windows 11 22H2 = build 22621
    ver.dwBuildNumber >= 22621
}

/// The flag set passed to every CreatePseudoConsole call (passthrough is
/// OR'ed on separately where supported).
///
/// PSUEDOCONSOLE_INHERIT_CURSOR is deliberately NOT set. With it, conhost emits
/// an ESC[6n cursor-position request at startup and will not service a child's
/// console connection until the host answers it. So if that reply is sent later
/// than the child's connect attempt, the child blocks in
/// ConsoleCreateConnectionObject during process initialization (a single
/// thread, before any user code runs) until the reply arrives: a temporary
/// stall if it is merely late, indefinite if it never comes. A multiplexer pane
/// always starts on a fresh screen, so inheriting the host cursor row buys
/// nothing here.
fn base_flags() -> DWORD {
    PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE
}

impl PsuedoCon {
    pub fn new(size: COORD, input: FileDescriptor, output: FileDescriptor) -> Result<Self, Error> {
        let mut con: HPCON = INVALID_HANDLE_VALUE;
        let base_flags = base_flags();

        // Use PSEUDOCONSOLE_PASSTHROUGH_MODE on Windows 11 22H2+ to relay
        // VT sequences (including DECSCUSR cursor shapes) from child processes
        // directly through the output pipe.  On older Windows, this flag is
        // silently accepted but breaks Win32 Console API translation, so we
        // only attempt it on known-good builds.
        if supports_passthrough_mode() {
            let result = unsafe {
                (CONPTY.CreatePseudoConsole)(
                    size,
                    input.as_raw_handle() as _,
                    output.as_raw_handle() as _,
                    base_flags | PSEUDOCONSOLE_PASSTHROUGH_MODE,
                    &mut con,
                )
            };

            if result == S_OK {
                return Ok(Self { con, used_passthrough: true });
            }
            // If the API call failed despite being on a supported build,
            // fall through to the standard path.
            con = INVALID_HANDLE_VALUE;
        }

        let result = unsafe {
            (CONPTY.CreatePseudoConsole)(
                size,
                input.as_raw_handle() as _,
                output.as_raw_handle() as _,
                base_flags,
                &mut con,
            )
        };
        ensure!(
            result == S_OK,
            "failed to create psuedo console: HRESULT {}",
            result
        );
        Ok(Self { con, used_passthrough: false })
    }

    /// Create a ConPTY explicitly without passthrough mode, regardless of
    /// Windows build version.  Used by the retry logic when CreateProcessW
    /// rejects the passthrough ConPTY handle.
    pub fn new_without_passthrough(size: COORD, input: FileDescriptor, output: FileDescriptor) -> Result<Self, Error> {
        let mut con: HPCON = INVALID_HANDLE_VALUE;
        let base_flags = base_flags();

        let result = unsafe {
            (CONPTY.CreatePseudoConsole)(
                size,
                input.as_raw_handle() as _,
                output.as_raw_handle() as _,
                base_flags,
                &mut con,
            )
        };
        ensure!(
            result == S_OK,
            "failed to create psuedo console (no passthrough): HRESULT {}",
            result
        );
        Ok(Self { con, used_passthrough: false })
    }

    pub fn resize(&self, size: COORD) -> Result<(), Error> {
        let result = unsafe { (CONPTY.ResizePseudoConsole)(self.con, size) };
        ensure!(
            result == S_OK,
            "failed to resize console to {}x{}: HRESULT: {}",
            size.X,
            size.Y,
            result
        );
        Ok(())
    }

    pub fn spawn_command(&self, cmd: CommandBuilder) -> anyhow::Result<WinChild> {
        let mut si: STARTUPINFOEXW = unsafe { mem::zeroed() };
        si.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        // Note: we deliberately do NOT set STARTF_USESTDHANDLES with
        // INVALID_HANDLE_VALUE for stdio.  MSDN explicitly requires
        // STARTF_USESTDHANDLES to be paired with bInheritHandles=TRUE,
        // and we use bInheritHandles=FALSE below.  Most Windows builds
        // tolerate the combination silently (because INVALID_HANDLE_VALUE
        // is a sentinel rather than a real handle), but newer/restricted
        // configurations — Win 11 26200, Microsoft-account profiles with
        // tighter token policies, certain WDAC/AppLocker rule sets — now
        // enforce the contract strictly and reject the call with
        // ERROR_INVALID_PARAMETER (87).  See psmux issue #167.
        //
        // ConPTY routes stdio through the PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE
        // attribute on the attribute list, so the child gets correct stdio
        // regardless of dwFlags.  bInheritHandles=FALSE prevents leaking
        // any other inheritable handles.

        let mut attrs = ProcThreadAttributeList::with_capacity(1)?;
        attrs.set_pty(self.con)?;
        si.lpAttributeList = attrs.as_mut_ptr();

        let mut pi: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        let (mut exe, mut cmdline) = cmd.cmdline()?;
        let cmd_os = OsString::from_wide(&cmdline);

        let cwd = cmd.current_directory();

        // The child's ProcessParameters std handles are stamped from this
        // process's std handle slots at CreateProcessW time.  Hold the console
        // state lock so no FreeConsole/AttachConsole dance (Ctrl+C delivery,
        // mouse/VT injection) is mid-flight on another thread, and park the
        // slots on NULL for the duration of the call: after any dance the
        // slots of a headless server dangle on freed, recycled handle values,
        // and a child born from them dies at its first console read
        // (issue #450).  A NULL-std parent is the GUI-parent case, for which
        // conhost always hands the child fresh handles to its own console.
        let _console_guard = crate::console_state_lock();
        let saved_std = unsafe {
            use winapi::um::processenv::{GetStdHandle, SetStdHandle};
            use winapi::um::winbase::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
            let saved = (
                GetStdHandle(STD_INPUT_HANDLE),
                GetStdHandle(STD_OUTPUT_HANDLE),
                GetStdHandle(STD_ERROR_HANDLE),
            );
            SetStdHandle(STD_INPUT_HANDLE, ptr::null_mut());
            SetStdHandle(STD_OUTPUT_HANDLE, ptr::null_mut());
            SetStdHandle(STD_ERROR_HANDLE, ptr::null_mut());
            saved
        };

        let res = unsafe {
            CreateProcessW(
                exe.as_mut_slice().as_mut_ptr(),
                cmdline.as_mut_slice().as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                cmd.environment_block().as_mut_slice().as_mut_ptr() as *mut _,
                cwd.as_ref()
                    .map(|c| c.as_slice().as_ptr())
                    .unwrap_or(ptr::null()),
                &mut si.StartupInfo,
                &mut pi,
            )
        };
        let create_err = IoError::last_os_error();
        unsafe {
            use winapi::um::processenv::SetStdHandle;
            use winapi::um::winbase::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
            SetStdHandle(STD_INPUT_HANDLE, saved_std.0);
            SetStdHandle(STD_OUTPUT_HANDLE, saved_std.1);
            SetStdHandle(STD_ERROR_HANDLE, saved_std.2);
        }
        if res == 0 {
            let err = create_err;
            let msg = format!(
                "CreateProcessW `{:?}` in cwd `{:?}` failed: {}",
                cmd_os,
                cwd.as_ref().map(|c| OsString::from_wide(c)),
                err
            );
            log::error!("{}", msg);
            bail!("{}", msg);
        }

        // Make sure we close out the thread handle so we don't leak it;
        // we do this simply by making it owned
        let _main_thread = unsafe { OwnedHandle::from_raw_handle(pi.hThread as _) };
        let proc = unsafe { OwnedHandle::from_raw_handle(pi.hProcess as _) };

        Ok(WinChild {
            proc: Mutex::new(proc),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conpty_flags_do_not_include_inherit_cursor() {
        // PSUEDOCONSOLE_INHERIT_CURSOR makes the new conhost emit ESC[6n at
        // startup and block its own initialization until the host replies; an
        // unanswered query leaves conhost unable to service the child's
        // console connect, so the child hangs inside process initialization
        // (single thread parked in ConsoleCreateConnectionObject). A
        // multiplexer pane always starts on a fresh screen, so inheriting the
        // host cursor row has no value here.
        assert_eq!(
            base_flags() & PSUEDOCONSOLE_INHERIT_CURSOR,
            0,
            "INHERIT_CURSOR must not be set: it makes conhost block startup waiting for an ESC[6n reply"
        );
        // The other flags are load-bearing and must stay.
        assert_ne!(base_flags() & PSEUDOCONSOLE_RESIZE_QUIRK, 0);
        assert_ne!(base_flags() & PSEUDOCONSOLE_WIN32_INPUT_MODE, 0);
    }
}
