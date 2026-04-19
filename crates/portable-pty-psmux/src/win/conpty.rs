use crate::cmdbuilder::CommandBuilder;
use crate::win::psuedocon::PsuedoCon;
use crate::{Child, MasterPty, PtyPair, PtySize, PtySystem, SlavePty};
use anyhow::Error;
use filedescriptor::FileDescriptor;
use std::sync::{Arc, Mutex};
use winapi::um::wincon::COORD;

/// Create a pipe pair with an explicit buffer size.
///
/// Windows Terminal uses 128 KB pipe buffers for ConPTY I/O.  The default
/// `CreatePipe(..., 0)` typically gets 4 KB, which forces more frequent
/// kernel transitions during high-throughput output (e.g. `cat large_file`).
/// Using 64 KB matches Windows Terminal's approach and reduces syscall
/// overhead for both input (mouse/keyboard) and output.
fn create_pipe_with_buffer(size: u32) -> anyhow::Result<(FileDescriptor, FileDescriptor)> {
    use std::os::windows::io::FromRawHandle;
    use std::ptr;
    use winapi::shared::minwindef::TRUE;
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
    use winapi::um::namedpipeapi::CreatePipe;
    use winapi::um::winnt::HANDLE;

    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: TRUE as _,
    };
    let mut read: HANDLE = INVALID_HANDLE_VALUE;
    let mut write: HANDLE = INVALID_HANDLE_VALUE;
    if unsafe { CreatePipe(&mut read, &mut write, &mut sa, size) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe {(
        FileDescriptor::from_raw_handle(read as _),
        FileDescriptor::from_raw_handle(write as _),
    )})
}

#[derive(Default)]
pub struct ConPtySystem {}

impl PtySystem for ConPtySystem {
    fn openpty(&self, size: PtySize) -> anyhow::Result<PtyPair> {
        // Use 64KB pipe buffers (Windows Terminal uses 128KB).
        // Default CreatePipe(..., 0) = ~4KB, causing frequent kernel round-trips.
        const PIPE_BUF: u32 = 64 * 1024;
        let (stdin_read, stdin_write) = create_pipe_with_buffer(PIPE_BUF)?;
        let (stdout_read, stdout_write) = create_pipe_with_buffer(PIPE_BUF)?;

        let con = PsuedoCon::new(
            COORD {
                X: size.cols as i16,
                Y: size.rows as i16,
            },
            stdin_read,
            stdout_write,
        )?;

        let master = ConPtyMasterPty {
            inner: Arc::new(Mutex::new(Inner {
                con,
                readable: stdout_read,
                writable: Some(stdin_write),
                size,
            })),
        };

        let slave = ConPtySlavePty {
            inner: master.inner.clone(),
        };

        Ok(PtyPair {
            master: Box::new(master),
            slave: Box::new(slave),
        })
    }
}

struct Inner {
    con: PsuedoCon,
    readable: FileDescriptor,
    writable: Option<FileDescriptor>,
    size: PtySize,
}

impl Inner {
    pub fn resize(
        &mut self,
        num_rows: u16,
        num_cols: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), Error> {
        self.con.resize(COORD {
            X: num_cols as i16,
            Y: num_rows as i16,
        })?;
        self.size = PtySize {
            rows: num_rows,
            cols: num_cols,
            pixel_width,
            pixel_height,
        };
        Ok(())
    }
}

#[derive(Clone)]
pub struct ConPtyMasterPty {
    inner: Arc<Mutex<Inner>>,
}

pub struct ConPtySlavePty {
    inner: Arc<Mutex<Inner>>,
}

impl MasterPty for ConPtyMasterPty {
    fn resize(&self, size: PtySize) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.resize(size.rows, size.cols, size.pixel_width, size.pixel_height)
    }

    fn get_size(&self) -> Result<PtySize, Error> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.size.clone())
    }

    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn std::io::Read + Send>> {
        Ok(Box::new(self.inner.lock().unwrap().readable.try_clone()?))
    }

    fn take_writer(&self) -> anyhow::Result<Box<dyn std::io::Write + Send>> {
        Ok(Box::new(
            self.inner
                .lock()
                .unwrap()
                .writable
                .take()
                .ok_or_else(|| anyhow::anyhow!("writer already taken"))?,
        ))
    }
}

impl SlavePty for ConPtySlavePty {
    fn spawn_command(&self, cmd: CommandBuilder) -> anyhow::Result<Box<dyn Child + Send + Sync>> {
        let mut inner = self.inner.lock().unwrap();
        match inner.con.spawn_command(cmd.clone()) {
            Ok(child) => Ok(Box::new(child)),
            Err(e) if inner.con.used_passthrough && is_invalid_parameter(&e) => {
                // CreateProcessW rejected the ConPTY handle that was created
                // with PSEUDOCONSOLE_PASSTHROUGH_MODE.  Some Windows 11 builds
                // (notably Insider/Canary builds like 26200) accept the flag
                // during CreatePseudoConsole but later fail in CreateProcessW
                // with ERROR_INVALID_PARAMETER (87).
                //
                // Recovery: recreate the ConPTY without passthrough mode and
                // create fresh pipe pairs for the new pseudo-console.
                log::warn!(
                    "CreateProcessW failed with ERROR_INVALID_PARAMETER while using \
                     ConPTY passthrough mode; retrying without passthrough"
                );
                const PIPE_BUF: u32 = 64 * 1024;
                let (stdin_read, stdin_write) = create_pipe_with_buffer(PIPE_BUF)?;
                let (stdout_read, stdout_write) = create_pipe_with_buffer(PIPE_BUF)?;

                let new_con = PsuedoCon::new_without_passthrough(
                    COORD {
                        X: inner.size.cols as i16,
                        Y: inner.size.rows as i16,
                    },
                    stdin_read,
                    stdout_write,
                )?;

                // Replace the ConPTY and pipe endpoints inside Inner.
                // At this point nobody has cloned the reader or taken the
                // writer yet (pane.rs acquires them after spawn_command),
                // so the old FileDescriptors are dropped cleanly.
                inner.con = new_con;
                inner.readable = stdout_read;
                inner.writable = Some(stdin_write);

                let child = inner.con.spawn_command(cmd)?;
                Ok(Box::new(child))
            }
            Err(e) => Err(e),
        }
    }
}

/// Check if an error chain contains Windows ERROR_INVALID_PARAMETER (87).
/// The OS error number is locale-independent; the textual message varies
/// (e.g. "Falscher Parameter" in German).
fn is_invalid_parameter(e: &anyhow::Error) -> bool {
    let msg = format!("{}", e);
    msg.contains("os error 87")
}
