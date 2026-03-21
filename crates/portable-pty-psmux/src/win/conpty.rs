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
        let inner = self.inner.lock().unwrap();
        let child = inner.con.spawn_command(cmd)?;
        Ok(Box::new(child))
    }
}
