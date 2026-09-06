//! Coherent session registration. Only the server holding the session-name
//! mutex may publish; cleanup additionally checks the immutable generation.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RegistryManifest {
    pub pid: u32,
    pub creation_time: u64,
    pub generation: String,
    pub port: u16,
    pub key: String,
    pub sid: u64,
}

#[derive(Clone, Debug)]
pub struct RegistryOwner {
    pid: u32,
    creation_time: u64,
    generation: String,
}

pub fn manifest_path(base: &str) -> PathBuf {
    PathBuf::from(crate::paths::psmux_dir_file(format!("{}.registry.json", base)))
}

pub fn read_manifest(base: &str) -> io::Result<Option<RegistryManifest>> {
    read_manifest_at(&manifest_path(base))
}

fn read_manifest_at(path: &Path) -> io::Result<Option<RegistryManifest>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 { return Err(io::Error::other("oversized session manifest")); }
    let manifest: RegistryManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if manifest.pid == 0 || manifest.creation_time == 0 || manifest.port == 0
        || manifest.key.is_empty() || manifest.generation.is_empty() {
        return Err(io::Error::other("invalid session manifest"));
    }
    Ok(Some(manifest))
}

/// Same-directory replace: readers see either the complete old file or the
/// complete new file. In particular Windows rename must replace an existing
/// destination without deleting it first.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("tmp-{}-{}", std::process::id(), n));
    // A leftover file may belong to an earlier process with the same PID. Do
    // not remove it when create_new fails; cleanup is only for our own file.
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let ok = unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(), to.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                    | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            ) };
            if ok == 0 { return Err(io::Error::last_os_error()); }
        }
        #[cfg(not(windows))]
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temp); }
    result
}

impl RegistryOwner {
    pub fn current() -> io::Result<Self> {
        let pid = std::process::id();
        let creation_time = crate::platform::process_kill::process_creation_time(pid)
            .filter(|t| *t != 0).ok_or_else(|| io::Error::other("cannot establish server process identity"))?;
        let generation = format!("{}-{}-{}", pid, creation_time,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        Ok(Self { pid, creation_time, generation })
    }

    fn owns(&self, m: &RegistryManifest) -> bool {
        self.pid == m.pid && self.creation_time == m.creation_time && self.generation == m.generation
    }

    pub fn publish(&self, base: &str, port: u16, key: &str, sid: u64) -> io::Result<()> {
        self.publish_at(&PathBuf::from(crate::paths::psmux_dir()), base, port, key, sid)
    }

    fn publish_at(&self, dir: &Path, base: &str, port: u16, key: &str, sid: u64) -> io::Result<()> {
        self.publish_at_with(dir, base, port, key, sid, atomic_write)
    }

    fn publish_at_with(&self, dir: &Path, base: &str, port: u16, key: &str, sid: u64,
        mut write_record: impl FnMut(&Path, &[u8]) -> io::Result<()>) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let manifest = RegistryManifest { pid: self.pid, creation_time: self.creation_time,
            generation: self.generation.clone(), port, key: key.to_string(), sid };
        let mp = dir.join(format!("{}.registry.json", base));
        if let Some(previous) = read_manifest_at(&mp)? {
            if !self.owns(&previous)
                && !crate::platform::process_kill::verified_process_dead(previous.pid, previous.creation_time) {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "session registry belongs to another server"));
            }
        }
        if read_manifest_at(&mp)?.is_none() {
            let pid_path = dir.join(format!("{}.pid", base));
            match std::fs::read_to_string(pid_path) {
                Ok(value) => {
                    let Some((pid, Some(creation))) = crate::session::parse_pid_file_contents(&value) else {
                        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "legacy session identity is uncertain"));
                    };
                    if (pid != self.pid || creation != self.creation_time)
                        && !crate::platform::process_kill::verified_process_dead(pid, creation) {
                        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "legacy session belongs to a live server"));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if dir.join(format!("{}.port", base)).exists() {
                        return Err(io::Error::new(io::ErrorKind::AlreadyExists,
                            "existing session endpoint has no verifiable process identity"));
                    }
                },
                Err(error) => return Err(error),
            }
        }
        // Snapshot first: a failed destination publication must leave the old
        // records intact, including stale legacy records that are not ours.
        let files = vec![
            (dir.join(format!("{}.pid", base)), format!("{}:{}", self.pid, self.creation_time).into_bytes()),
            (dir.join(format!("{}.key", base)), key.as_bytes().to_vec()),
            (dir.join(format!("{}.sid", base)), sid.to_string().into_bytes()),
            (mp, serde_json::to_vec(&manifest).map_err(io::Error::other)?),
            (dir.join(format!("{}.port", base)), port.to_string().into_bytes()),
        ];
        let mut previous = Vec::new();
        for (path, _) in &files {
            previous.push(match std::fs::read(path) {
                Ok(value) => Some(value),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            });
        }
        for (i, (path, contents)) in files.iter().enumerate() {
            if previous[i].as_deref() == Some(contents.as_slice()) { continue; }
            if let Err(error) = write_record(path, contents) {
                for j in (0..i).rev() {
                    // Never roll back data that changed after our write.
                    if std::fs::read(&files[j].0).ok().as_deref() != Some(files[j].1.as_slice()) { continue; }
                    if let Some(old) = &previous[j] { let _ = atomic_write(&files[j].0, old); }
                    else { let _ = std::fs::remove_file(&files[j].0); }
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn remove_owned(&self, base: &str) -> io::Result<bool> {
        self.remove_owned_at(&PathBuf::from(crate::paths::psmux_dir()), base)
    }

    fn remove_owned_at(&self, dir: &Path, base: &str) -> io::Result<bool> {
        let mp = dir.join(format!("{}.registry.json", base));
        let Some(m) = read_manifest_at(&mp)? else { return Ok(false); };
        if !self.owns(&m) { return Ok(false); }
        // An older binary may ignore the manifest. Do not delete records it
        // replaced even when it left this generation's manifest behind.
        let expected = [
            ("port", m.port.to_string()), ("key", m.key),
            ("sid", m.sid.to_string()), ("pid", format!("{}:{}", m.pid, m.creation_time)),
        ];
        for (extension, value) in &expected {
            match std::fs::read_to_string(dir.join(format!("{}.{}", base, extension))) {
                Ok(contents) if contents.trim() != value => return Ok(false),
                Ok(_) => {},
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
        }
        // Remove readiness first. Failure here leaves the old name fully
        // discoverable and allows a rename to roll its destination back.
        match std::fs::remove_file(dir.join(format!("{}.port", base))) {
            Ok(()) => {},
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Err(e) => return Err(e),
        }
        // New clients discover the coherent manifest itself. Remove it before
        // treating cleanup as committed. If that fails, restore the legacy
        // beacon so a rename can safely withdraw its tentative destination.
        if let Err(error) = std::fs::remove_file(&mp) {
            if error.kind() != io::ErrorKind::NotFound {
                let _ = atomic_write(&dir.join(format!("{}.port", base)), m.port.to_string().as_bytes());
                return Err(error);
            }
        }
        // Satellites no longer advertise an endpoint; failures are harmless
        // debris and cannot justify rolling back the successful rename.
        for extension in ["key", "sid", "pid", "act"] {
            match std::fs::remove_file(dir.join(format!("{}.{}", base, extension))) {
                Ok(()) => {},
                Err(e) if e.kind() == io::ErrorKind::NotFound => {},
                Err(_) => {},
            }
        }
        Ok(true)
    }
}

/// Cleans startup/normal-return failures, but never reacts to an unrelated
/// worker panic. Explicit process::exit sites also call remove_owned.
pub struct RegistrationLease {
    pub owner: RegistryOwner,
    pub base: String,
}
impl Drop for RegistrationLease {
    fn drop(&mut self) { let _ = self.owner.remove_owned(&self.base); }
}

#[cfg(test)]
mod reliability_registry_tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("psmux-registry-unit-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&p).unwrap(); p
    }
    fn owner(generation: &str) -> RegistryOwner { RegistryOwner { pid: std::process::id(),
        creation_time: crate::platform::process_kill::process_creation_time(std::process::id()).unwrap(), generation: generation.into() } }
    #[test]
    fn ownership_prevents_old_generation_cleanup() {
        let d = temp(); let first = owner("first"); let second = owner("second");
        first.publish_at(&d, "s", 1234, "key", 1).unwrap();
        assert!(!second.remove_owned_at(&d, "s").unwrap());
        assert!(d.join("s.port").exists());
        assert!(second.publish_at(&d, "s", 2345, "replacement", 2).is_err());
        assert_eq!(read_manifest_at(&d.join("s.registry.json")).unwrap().unwrap().port, 1234);
        assert!(first.remove_owned_at(&d, "s").unwrap()); std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn failed_publication_rolls_back_only_written_records() {
        let d = temp(); let o = owner("first");
        std::fs::write(d.join("s.key"), b"old-key").unwrap();
        std::fs::create_dir(d.join("s.port")).unwrap();
        assert!(o.publish_at(&d, "s", 1234, "key", 1).is_err());
        assert_eq!(std::fs::read(d.join("s.key")).unwrap(), b"old-key");
        assert!(!d.join("s.pid").exists()); std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn atomic_replace_never_truncates_existing_file() {
        let d = temp(); let p = d.join("record");
        atomic_write(&p, b"first").unwrap(); atomic_write(&p, b"second").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"second"); std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn late_write_failure_restores_entire_generation() {
        let d = temp(); let o = owner("first");
        o.publish_at(&d, "s", 1234, "old-key", 1).unwrap();
        let result = o.publish_at_with(&d, "s", 2345, "new-key", 2, |path, data| {
            if path.extension().and_then(|ext| ext.to_str()) == Some("port") {
                Err(io::Error::other("injected beacon write failure"))
            } else { atomic_write(path, data) }
        });
        assert!(result.is_err());
        let m = read_manifest_at(&d.join("s.registry.json")).unwrap().unwrap();
        assert_eq!((m.port, m.key.as_str(), m.sid), (1234, "old-key", 1));
        assert_eq!(std::fs::read_to_string(d.join("s.key")).unwrap(), "old-key");
        assert_eq!(std::fs::read_to_string(d.join("s.sid")).unwrap(), "1");
        assert_eq!(std::fs::read_to_string(d.join("s.port")).unwrap(), "1234");
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn older_binary_replacement_is_not_removed() {
        let d = temp(); let o = owner("first");
        o.publish_at(&d, "s", 1234, "old-key", 1).unwrap();
        std::fs::write(d.join("s.key"), "foreign-key").unwrap();
        assert!(!o.remove_owned_at(&d, "s").unwrap());
        assert!(d.join("s.port").exists());
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn unreadable_legacy_record_prevents_cleanup() {
        let d = temp(); let o = owner("first");
        o.publish_at(&d, "s", 1234, "key", 1).unwrap();
        std::fs::remove_file(d.join("s.key")).unwrap();
        std::fs::create_dir(d.join("s.key")).unwrap();
        assert!(o.remove_owned_at(&d, "s").is_err());
        assert!(d.join("s.port").exists());
        assert!(d.join("s.registry.json").exists());
        std::fs::remove_dir_all(d).unwrap();
    }
}
