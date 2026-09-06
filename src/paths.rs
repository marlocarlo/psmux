//! Single source of truth for psmux's home and data directories.
//!
//! Every `.port`/`.key`/`.sid`/warm-pool/log path routes through [`psmux_dir`]
//! (or [`psmux_dir_opt`]), so client and server can never disagree about where
//! the bookkeeping lives.

/// User home directory: `USERPROFILE`, then the Windows profile API, then
/// `HOMEDRIVE`+`HOMEPATH`, then `HOME`. Empty string when nothing resolves.
///
/// `HOME` is deliberately the LAST resort on Windows (issue #474): MSYS2
/// login shells unset `USERPROFILE` and set `HOME` to a POSIX-style path
/// (`/home/user`), so a psmux invocation from an MSYS2 shell that trusted
/// `HOME` would resolve the data dir somewhere the registry does not live.
/// The startup orphan reaper would then see every live server as untracked
/// and terminate them all. Querying the profile API keeps every invocation
/// of the same user converging on the same data dir regardless of which
/// shell launched it.
pub fn home_dir() -> String {
    if let Ok(v) = std::env::var("USERPROFILE") {
        if !v.is_empty() {
            return v;
        }
    }
    #[cfg(windows)]
    if let Some(p) = windows_profile_dir() {
        return p;
    }
    let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
    let path = std::env::var("HOMEPATH").unwrap_or_default();
    if !drive.is_empty() && !path.is_empty() {
        let p = format!("{}{}", drive, path);
        if std::path::Path::new(&p).is_dir() {
            return p;
        }
    }
    std::env::var("HOME").unwrap_or_default()
}

/// The current user's profile directory straight from the OS
/// (`GetUserProfileDirectoryW` on the process token), independent of any
/// environment variable a shell may have rewritten or unset.
#[cfg(windows)]
fn windows_profile_dir() -> Option<String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        // isize handle to match every other CloseHandle declaration in the
        // crate (clashing_extern_declarations); cast at the call site.
        fn CloseHandle(h: isize) -> i32;
    }
    #[link(name = "userenv")]
    extern "system" {
        fn GetUserProfileDirectoryW(token: *mut c_void, buf: *mut u16, len: *mut u32) -> i32;
    }
    const TOKEN_QUERY: u32 = 0x0008;
    unsafe {
        let mut token: *mut c_void = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = GetUserProfileDirectoryW(token, buf.as_mut_ptr(), &mut len);
        CloseHandle(token as isize);
        if ok == 0 || len == 0 {
            return None;
        }
        let s = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]);
        if s.is_empty() { None } else { Some(s) }
    }
}

/// Validate and normalize a PSMUX_DATA_DIR override value. Panics on a
/// relative or empty value: silently resolving against the CWD would scatter
/// the registry across working directories. Kept separate from the env read so
/// the rejection can be tested without ever placing an invalid value in the
/// process-global environment, which unlocked readers may observe mid-test.
fn normalize_data_dir_override(raw: &std::ffi::OsStr) -> String {
    let path = std::path::PathBuf::from(raw);
    assert!(
        path.is_absolute() && !path.as_os_str().is_empty(),
        "PSMUX_DATA_DIR must be an absolute non-empty path"
    );
    path.to_string_lossy().trim_end_matches(['/', '\\']).to_string()
}

/// psmux data directory, without a trailing separator. Fallible variant for
/// call sites that early-exit when home is missing (`.ok()?` /
/// `Err(_) => return …`). `None` when no home directory is available.
pub fn psmux_dir_opt() -> Option<String> {
    if let Some(raw) = std::env::var_os("PSMUX_DATA_DIR") {
        return Some(normalize_data_dir_override(&raw));
    }
    let home = home_dir();
    if home.is_empty() {
        None
    } else {
        Some(format!("{}\\.psmux", home))
    }
}

/// psmux data directory, without a trailing separator. Infallible variant for
/// call sites that today use `.unwrap_or_default()`.
///
/// When no home directory is set, returns `\.psmux` — byte-identical to the
/// historical `format!("{}\.psmux", "")`. It does not panic: the historical
/// behavior is to proceed and let the filesystem call fail gracefully, and a
/// crash on an unset `USERPROFILE` would be worse.
pub fn psmux_dir() -> String {
    psmux_dir_opt().unwrap_or_else(|| "\\.psmux".to_string())
}

/// The data root reduced to a comparison key: separators unified and case
/// folded, because Windows treats `C:/Foo` and `c:\foo` as one directory and
/// anything keyed on the root must treat them as one too.
fn normalized_data_root() -> String {
    let root = psmux_dir();
    // Resolve junctions and dot components before naming a kernel lock. Two
    // spellings of the same registry must never acquire different locks.
    std::fs::canonicalize(&root).unwrap_or_else(|_| root.into())
        .to_string_lossy().trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase()
}

const ENCODED_BASE: &str = "~psmux~";

fn plain_component(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let reserved_device = matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || (lower.len() == 4 && (lower.starts_with("com") || lower.starts_with("lpt"))
            && matches!(lower.as_bytes()[3], b'1'..=b'9'));
    !value.is_empty() && !value.contains("__") && !value.starts_with(ENCODED_BASE)
        && !value.starts_with('_') && !value.ends_with('_')
        && !reserved_device
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn hex_name(value: &str) -> String {
    value.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex_name(value: &str) -> Option<String> {
    if value.len() % 2 != 0 || !value.is_ascii() { return None; }
    let bytes = (0..value.len()).step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i+2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

/// An injective filename encoding for newly created registry entries. Preserve
/// ordinary legacy names; delimiters and path characters are encoded together
/// with an explicit default/named-namespace tag.
pub fn storage_base(namespace: Option<&str>, name: &str) -> String {
    let ordinary_name = plain_component(name) || name == "__warm__";
    if ordinary_name && namespace.map_or(true, plain_component) {
        return namespace.map_or_else(|| name.to_string(), |ns| format!("{ns}__{name}"));
    }
    match namespace {
        None => format!("{ENCODED_BASE}d~{}", hex_name(name)),
        Some(ns) => format!("{ENCODED_BASE}n~{}~{}", hex_name(ns), hex_name(name)),
    }
}

pub fn session_base(namespace: Option<&str>, name: &str) -> String {
    storage_base(namespace, name)
}

fn decode_base(base: &str) -> Option<(Option<String>, String)> {
    let encoded = base.strip_prefix(ENCODED_BASE)?;
    if let Some(name) = encoded.strip_prefix("d~") {
        return Some((None, unhex_name(name)?));
    }
    let (ns, name) = encoded.strip_prefix("n~")?.split_once('~')?;
    Some((Some(unhex_name(ns)?), unhex_name(name)?))
}

pub fn registry_namespace(base: &str) -> Option<String> {
    if let Some((ns, _)) = decode_base(base) { return ns; }
    if base == "__warm__" { return None; }
    base.split_once("__").map(|(ns, _)| ns.to_string())
}

pub fn registry_session_name(base: &str) -> String {
    if let Some((_, name)) = decode_base(base) { return name; }
    if base == "__warm__" { return base.to_string(); }
    base.split_once("__").map_or(base, |(_, name)| name).to_string()
}

pub fn registry_visible_from(base: &str, namespace: Option<&str>) -> bool {
    registry_namespace(base).as_deref() == namespace
}

/// Keep an encoded registry component within Windows' 255-character limit and
/// prevent protocol delimiters from entering command/session identifiers.
pub fn validate_session_name(name: &str) -> std::io::Result<()> {
    if name.is_empty() || name.chars().any(char::is_control) || name.contains(':') || name.len() > 100 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "session name must be 1–100 UTF-8 bytes, without control characters or ':'"));
    }
    Ok(())
}

pub fn validate_namespace(namespace: &str) -> std::io::Result<()> {
    if namespace.is_empty() || namespace.chars().any(char::is_control) || namespace.len() > 100 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "namespace must be 1–100 UTF-8 bytes without control characters"));
    }
    Ok(())
}

pub fn validate_registry_base(namespace: Option<&str>, name: &str) -> std::io::Result<()> {
    validate_session_name(name)?;
    if let Some(ns) = namespace { validate_namespace(ns)?; }
    if storage_base(namespace, name).len() > 235 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "combined namespace and session name are too long for the registry"));
    }
    Ok(())
}

/// Stable per-data-root tag for names that live in the machine-wide kernel
/// object namespace (issue #599).
///
/// `PSMUX_DATA_DIR` namespaces the registry: every `.port`/`.key`/`.sid`/`.pid`
/// file, and therefore every fact the single-server guard protects, lives under
/// it. That guard's mutex, though, is a machine-wide name. Without the root in
/// the key two independent registries collide on one session name and the
/// second root's server is refused as a duplicate of a server it cannot even
/// see -- including the fixed `__warm__` name, which only the first root then
/// ever publishes. `-L` already reaches the key through `port_file_base()`;
/// this makes psmux's other namespace mechanism agree.
///
/// Derived from the RESOLVED root rather than the raw variable, so pointing
/// `PSMUX_DATA_DIR` at the default `~\.psmux` keys identically to leaving it
/// unset.
pub fn data_root_tag() -> String {
    use std::hash::{Hash, Hasher};
    // A fixed-seed hasher: the name must be identical across processes and
    // runs, so `RandomState` is deliberately NOT used here (same reasoning as
    // `namespace_instance_file`).
    let mut h = std::collections::hash_map::DefaultHasher::new();
    normalized_data_root().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Path to a fixed-name file directly under the data directory (e.g.
/// `latency.log`, `last_session`, `next_session_id`, `crash.log`).
pub fn psmux_dir_file(name: impl AsRef<str>) -> String {
    format!("{}\\{}", psmux_dir(), name.as_ref())
}

/// Path to a session's `.port` file (the TCP port its server listens on).
pub fn port_file(session: impl AsRef<str>) -> String {
    format!("{}\\{}.port", psmux_dir(), session.as_ref())
}

/// Directory holding one ownership marker per server process this data dir
/// started (issue #510).
///
/// The startup reaper enumerates candidate servers machine-wide, but its
/// registry only describes one data dir. Without a per-process record of
/// "this one is mine" it cannot tell an orphan of its own from another
/// instance's healthy server, and killing on that ambiguity destroyed live
/// sessions belonging to a different USERPROFILE/HOME. A server writes its
/// marker into its OWN data dir, so ownership is self-declared and needs no
/// inspection of other processes.
pub fn server_marker_dir() -> String {
    format!("{}\\servers", psmux_dir())
}

/// Path to one server process's ownership marker (see `server_marker_dir`).
pub fn server_marker_file(pid: u32) -> String {
    format!("{}\\{}", server_marker_dir(), pid)
}

/// Fallible variant of [`port_file`]: `None` when no data directory can be
/// determined. Lets a call site early-exit on the same condition without having
/// to know that `port_file` routes through `psmux_dir`.
pub fn port_file_opt(session: impl AsRef<str>) -> Option<String> {
    psmux_dir_opt().map(|dir| format!("{}\\{}.port", dir, session.as_ref()))
}

/// Path to a session's `.key` file (the auth key for its server).
pub fn key_file(session: impl AsRef<str>) -> String {
    format!("{}\\{}.key", psmux_dir(), session.as_ref())
}

/// Path to a session's `.sid` file (its stable session id).
pub fn sid_file(session: impl AsRef<str>) -> String {
    format!("{}\\{}.sid", psmux_dir(), session.as_ref())
}

/// Path to a session's `.pid` file (the liveness anchor: the server pid).
pub fn pid_file(session: impl AsRef<str>) -> String {
    format!("{}\\{}.pid", psmux_dir(), session.as_ref())
}

/// Path to a session's `.spawnlock` file (the warm-pool spawn lock).
pub fn spawnlock_file(session: impl AsRef<str>) -> String {
    format!("{}\\{}.spawnlock", psmux_dir(), session.as_ref())
}

/// Path to a session's `.act` file: the last-activity stamp (Unix epoch
/// microseconds, ASCII) that bare CLI routing ranks candidates by.
///
/// This is psmux's registry-visible equivalent of tmux's in-memory
/// `session.activity_time` (tmux `session.c`). tmux can consult it directly
/// because the command runs inside the server; a psmux CLI process talks to one
/// server per session, so the ranking input has to live in the shared registry
/// where it can be read without connecting to anything (issue #603).
pub fn activity_file(session: impl AsRef<str>) -> String {
    format!("{}\\{}.act", psmux_dir(), session.as_ref())
}

/// Directory holding one namespace-identity file per `-L` namespace (issue #509).
///
/// A subdirectory rather than a `<ns>.instance` sibling: session files are named
/// `<ns>__<session>.<ext>`, and the default namespace uses a bare `<session>`,
/// so any flat naming risks colliding with a legitimately-named session.
pub fn instance_dir_in(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("instances")
}

/// Path to a namespace's identity file. `ns` is the `-L` value, or `None` for
/// the default namespace.
///
/// The file name is a readable prefix plus a hash of the *full* namespace name.
/// `-L` values come from the user and may contain characters that are illegal in
/// a filename (or that would collide once sanitised — `a/b` and `a_b` both
/// become `a_b`), so the hash, not the prefix, is what guarantees isolation.
pub fn namespace_instance_file(dir: &std::path::Path, ns: Option<&str>) -> std::path::PathBuf {
    let name = match ns {
        None => "default-0000000000000000".to_string(),
        Some(n) => {
            use std::hash::{Hash, Hasher};
            // A fixed-seed hasher: the file name must be identical across
            // processes and runs, so `RandomState` (used for the random session
            // key) is deliberately NOT used here.
            let mut h = std::collections::hash_map::DefaultHasher::new();
            n.hash(&mut h);
            let digest = h.finish();
            let prefix: String = n
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(32)
                .collect();
            format!("{}-{:016x}", prefix, digest)
        }
    };
    instance_dir_in(dir).join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests READ the process-global env (USERPROFILE/HOME/
    // PSMUX_DATA_DIR) that sibling modules mutate, so they must hold the
    // same crate-wide lock the mutators serialize on or they observe a
    // half-mutated environment in full parallel runs.
    #[test]
    fn psmux_dir_is_home_relative_dot_psmux() {
        let _lock = crate::util::lock_test_env();
        // The test environment always has a home, so the data dir resolves to
        // {home}\.psmux and the two accessors agree.
        let dir = psmux_dir();
        assert!(dir.ends_with("\\.psmux"), "got {dir:?}");
        assert_eq!(psmux_dir_opt().as_deref(), Some(dir.as_str()));
    }

    #[test]
    fn per_session_helpers_append_name_and_suffix_to_data_dir() {
        let _lock = crate::util::lock_test_env();
        let dir = psmux_dir();
        assert_eq!(port_file("foo"), format!("{}\\foo.port", dir));
        assert_eq!(key_file("foo"), format!("{}\\foo.key", dir));
        assert_eq!(sid_file("foo"), format!("{}\\foo.sid", dir));
        assert_eq!(pid_file("foo"), format!("{}\\foo.pid", dir));
        assert_eq!(spawnlock_file("foo"), format!("{}\\foo.spawnlock", dir));
    }

    #[test]
    fn psmux_dir_file_appends_fixed_name() {
        let _lock = crate::util::lock_test_env();
        assert_eq!(
            psmux_dir_file("latency.log"),
            format!("{}\\latency.log", psmux_dir())
        );
    }
}

#[cfg(test)]
#[path = "../tests-rs/test_issue474_home_resolution.rs"]
mod tests_issue474_home_resolution;

#[cfg(test)]
#[path = "../tests-rs/test_data_dir_override.rs"]
mod tests_data_dir_override;

#[cfg(test)]
mod registry_encoding_tests {
    use super::*;

    #[test]
    fn identities_round_trip_without_delimiter_or_path_collisions() {
        let namespaces = [None, Some("a"), Some("a__b"), Some("a_"), Some("a/b"), Some("a\\b")];
        let names = ["dev", "build__dev", "b__c", "_b", "__warm__", "~psmux~d~61", "a/b", "é", "CON", "nul", "COM1"];
        let mut bases = std::collections::HashSet::new();
        for namespace in namespaces {
            for name in names {
                let base = storage_base(namespace, name);
                assert!(bases.insert(base.clone()), "duplicate encoding: {base}");
                assert_eq!(registry_namespace(&base).as_deref(), namespace);
                assert_eq!(registry_session_name(&base), name);
                assert!(registry_visible_from(&base, namespace));
                assert!(!base.contains(['/', '\\', ':']));
            }
        }
    }

    #[test]
    fn ordinary_legacy_entries_remain_readable() {
        assert_eq!(storage_base(Some("work"), "dev"), "work__dev");
        assert_eq!(storage_base(None, "dev"), "dev");
        assert_eq!(registry_namespace("work____warm__"), Some("work".into()));
        assert_eq!(registry_session_name("work____warm__"), "__warm__");
        assert_eq!(registry_namespace("work__build__dev"), Some("work".into()));
        assert_eq!(registry_session_name("work__build__dev"), "build__dev");
    }

    #[test]
    fn unsafe_or_oversized_protocol_names_fail_explicitly() {
        for name in ["", "bad\nname", "a:b", "a\0b"] { assert!(validate_registry_base(None, name).is_err()); }
        assert!(validate_registry_base(Some(&"n".repeat(100)), &"_".repeat(100)).is_err());
        assert!(validate_registry_base(Some("a__b"), "c__d").is_ok());
    }
}
