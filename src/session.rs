use std::io::{self, ErrorKind, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};
use std::env;

const STALE_PORT_PROBE_ATTEMPTS: usize = 3;
const STALE_PORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const STALE_PORT_RETRY_DELAY: Duration = Duration::from_millis(25);
/// How long to wait for the server's AUTH ack (`OK` / `ERROR`) when verifying
/// that the listener on a port file's port is actually *our* psmux server.
const STALE_PORT_AUTH_READ_TIMEOUT: Duration = Duration::from_millis(120);
/// Grace window subtracted from the system boot time before treating a
/// registry file as "written before this boot". Absorbs clock jitter and the
/// inherent imprecision of deriving boot wall-time from uptime, so a server
/// that wrote its port file moments after boot is never falsely reaped.
const BOOT_TIME_MARGIN: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortProbeResult {
    Alive,
    Stale,
    Inconclusive,
}

/// Returns true if this port-file base name belongs to a warm (standby) server.
/// Warm sessions should be hidden from user-facing lists and never auto-attached.
pub fn is_warm_session(base: &str) -> bool {
    crate::paths::registry_session_name(base) == "__warm__"
}

pub fn session_namespace(full: &str) -> Option<String> {
    crate::paths::registry_namespace(full)
}

pub fn session_visible_from(base: &str, ns: Option<&str>) -> bool {
    crate::paths::registry_visible_from(base, ns)
}

/// Resolve once for the entire invocation: explicit socket, inherited socket,
/// then the current pane's registered endpoint, otherwise default namespace.
pub fn effective_namespace(explicit: Option<&str>, inherited: Option<&str>, tmux: Option<&str>, dir: &Path) -> Option<String> {
    explicit.or(inherited).map(str::to_string).or_else(|| {
        tmux.and_then(|value| session_base_owning_tmux_port(value, dir))
            .and_then(|base| session_namespace(&base))
    })
}

/// Find the next available numeric session name (tmux-compatible).
/// tmux uses a monotonically incrementing counter, but since psmux has
/// no persistent server state, we scan existing port files and pick
/// the lowest non-negative integer not already in use.
/// When `ns_prefix` is Some("foo"), names are checked as "foo__0", "foo__1", etc.
pub fn next_session_name(ns_prefix: Option<&str>) -> String {
    let Some(psmux_dir) = crate::paths::psmux_dir_opt() else {
        return "0".to_string();
    };
    let mut used: std::collections::HashSet<u32> = std::collections::HashSet::new();
    if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
        for entry in entries.flatten() {
            if let Some(fname) = entry.file_name().to_str() {
                if let Some((base, ext)) = fname.rsplit_once('.') {
                    if ext != "port" { continue; }
                    if is_warm_session(base) { continue; }
                    if !session_visible_from(base, ns_prefix) { continue; }
                    let session_part = crate::paths::registry_session_name(base);
                    if let Ok(n) = session_part.parse::<u32>() {
                        used.insert(n);
                    }
                }
            }
        }
    }
    let mut id = 0u32;
    while used.contains(&id) {
        id += 1;
    }
    id.to_string()
}

/// Serializes session-id allocation within this process. Without it, two
/// threads (e.g. concurrent `new-session` handling, or the test harness running
/// tests in parallel) can both read the same value from the counter file before
/// either writes back, and hand out duplicate ids.
static SESSION_ID_ALLOC: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Compatibility lock for older executables, protected by a strict named
/// mutex between current servers. Age alone never authorizes lock theft.
struct CounterLock { path: String, identity: String }
impl CounterLock {
    fn acquire(path: String) -> io::Result<Self> {
        let creation = crate::platform::process_kill::process_creation_time(std::process::id())
            .ok_or_else(|| io::Error::other("cannot establish counter owner identity"))?;
        let identity = format!("{}:{}", std::process::id(), creation);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(identity.as_bytes()) {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(Self { path, identity });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    if let Ok(record) = std::fs::read_to_string(&path) {
                        if let Some((pid, creation)) = parse_pid_file_contents(&record) {
                            use crate::platform::process_kill::{process_identity, ProcessIdentity};
                            let dead = match process_identity(pid) {
                                ProcessIdentity::Exited => true,
                                ProcessIdentity::Alive(actual) => creation.is_some_and(|old| old != actual),
                                ProcessIdentity::Unknown => false,
                            };
                            if dead && std::fs::read_to_string(&path).ok().as_deref() == Some(record.as_str()) {
                                std::fs::remove_file(&path)?;
                                continue;
                            }
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(io::Error::new(ErrorKind::WouldBlock, "session ID counter lock is busy or its owner is uncertain"));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
    }
}
impl Drop for CounterLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).ok().as_deref() == Some(self.identity.as_str()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn increment_session_counter(path: &Path) -> io::Result<usize> {
    let current = match std::fs::read_to_string(path) {
        Ok(value) => value.trim().parse::<usize>()
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "invalid session ID counter"))?,
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let next = current.checked_add(1).ok_or_else(|| io::Error::other("session ID counter exhausted"))?;
    crate::registry::atomic_write(path, next.to_string().as_bytes())?;
    Ok(current)
}

/// Reserve an ID only when starting a server; failed allocation aborts startup.
/// Neither AppState construction nor read-only clients mutate this counter.
pub fn allocate_session_id() -> io::Result<usize> {
    let _guard = SESSION_ID_ALLOC.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(crate::paths::psmux_dir())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let _named = loop {
        if let Some(guard) = crate::platform::acquire_session_mutex_strict("~psmux~counter~session")? { break guard; }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(ErrorKind::WouldBlock, "session ID allocator is busy"));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let counter_path = crate::paths::psmux_dir_file("next_session_id");
    let _compatibility = CounterLock::acquire(format!("{}.lock", counter_path))?;
    increment_session_counter(Path::new(&counter_path))
}

/// Write a `.sid` file recording the session ID for this session.
pub fn write_session_id_file(port_file_base: &str, session_id: usize) {
    let sid_path = crate::paths::sid_file(port_file_base);
    let _ = std::fs::write(&sid_path, session_id.to_string());
}

/// Remove the `.sid` file for a session. Also removes the twin `.pid` file
/// (issue #448): both are per-session identity sentinels written together by
/// `ensure_session_registry_files`, and every session-teardown site already
/// calls this, so piggybacking `.pid` cleanup here keeps the registry consistent
/// without touching each teardown call site.
pub fn remove_session_id_file(port_file_base: &str) {
    let sid_path = crate::paths::sid_file(port_file_base);
    let _ = std::fs::remove_file(&sid_path);
    remove_session_pid_file(port_file_base);
    // The `.act` activity stamp (issue #603) is per-session registry state
    // exactly like `.sid`/`.pid`, so it leaves with them. Left behind, a dead
    // session's stamp survived kill-server (seen as a stray
    // `<ns>__<name>.act` after `-L <ns> kill-server` in test_mouse_hover) and
    // would keep ranking a session that no longer exists.
    remove_session_activity_file(port_file_base);
}

/// Remove the `.act` activity stamp for a session (issue #603).
pub fn remove_session_activity_file(port_file_base: &str) {
    if let Some(dir) = crate::paths::psmux_dir_opt() {
        remove_session_activity_file_in(std::path::Path::new(&dir), port_file_base);
    }
}

/// Registry-directory-parameterized [`remove_session_activity_file`].
pub fn remove_session_activity_file_in(dir: &std::path::Path, port_file_base: &str) {
    let _ = std::fs::remove_file(dir.join(format!("{}.act", port_file_base)));
}

/// Move a session's `.act` activity stamp from `old_base` to `new_base` on
/// rename (issue #603). tmux keeps `activity_time` on the session struct, so a
/// rename never touches it; psmux's copy is keyed by name and has to follow.
/// Without this a renamed session fell back to its `.port` mtime and lost its
/// place in bare CLI routing. Best effort: a session that was never stamped
/// has nothing to carry.
pub fn carry_session_activity_file(old_base: &str, new_base: &str) {
    if let Some(dir) = crate::paths::psmux_dir_opt() {
        carry_session_activity_file_in(std::path::Path::new(&dir), old_base, new_base);
    }
}

/// Registry-directory-parameterized [`carry_session_activity_file`].
pub fn carry_session_activity_file_in(dir: &std::path::Path, old_base: &str, new_base: &str) {
    if old_base == new_base {
        return;
    }
    let old = dir.join(format!("{}.act", old_base));
    let new = dir.join(format!("{}.act", new_base));
    if old.exists() {
        let _ = std::fs::rename(&old, &new);
    }
}

/// Write a `.pid` file recording the OS process ID of the server that owns this
/// session (issue #448). The stale-port cleanup only knew a server by its TCP
/// port; a wedged server that stopped listening but hasn't exited could not be
/// targeted by identity at all. The PID gives every registry entry a stable
/// process anchor.
pub fn write_session_pid_file(port_file_base: &str, pid: u32) {
    let pid_path = crate::paths::pid_file(port_file_base);
    // `pid:creation_filetime` — same body as ensure_session_registry_files, so a
    // freshly renamed session is force-kill-identifiable before the next re-ensure.
    let creation = crate::platform::process_kill::process_creation_time(pid).unwrap_or(0);
    let _ = std::fs::write(&pid_path, format_pid_file_contents(pid, creation));
}

/// Remove the `.pid` file for a session.
pub fn remove_session_pid_file(port_file_base: &str) {
    let pid_path = crate::paths::pid_file(port_file_base);
    let _ = std::fs::remove_file(&pid_path);
}

/// Record that server process `pid` belongs to THIS data dir (issue #510).
///
/// Keyed by PID rather than by session, and deliberately independent of the
/// session registry lifecycle: the whole point is to still identify a server as
/// ours after its `.port`/`.pid` entries are gone, which is exactly the state a
/// spawn-race duplicate or a registry wipe leaves behind. Body is the same
/// `pid:creation_filetime` as the `.pid` sentinel so a reused PID cannot
/// inherit a dead process's claim.
pub fn write_server_marker(pid: u32) {
    let dir = crate::paths::server_marker_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let creation = crate::platform::process_kill::process_creation_time(pid).unwrap_or(0);
    let _ = std::fs::write(
        crate::paths::server_marker_file(pid),
        format_pid_file_contents(pid, creation),
    );
}

/// Server processes `psmux_dir` claims, as `pid -> recorded creation filetime`.
///
/// The PID is read from the file body rather than its name so a truncated or
/// hand-copied marker cannot assert a claim over an arbitrary PID.
pub fn read_owned_server_pids(psmux_dir: &Path) -> std::collections::HashMap<u32, Option<u64>> {
    let mut owned = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir(psmux_dir.join("servers")) else {
        return owned;
    };
    for entry in entries.flatten() {
        let Ok(body) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some((pid, creation)) = parse_pid_file_contents(&body) {
            owned.insert(pid, creation);
        }
    }
    owned
}

/// Resolve a tmux session ID (`$N`) to the port file base name of the
/// session that owns that ID. Returns `None` if no session has that ID.
pub fn resolve_session_by_id(id: usize) -> Option<String> {
    let psmux_dir = crate::paths::psmux_dir_opt()?;
    if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "sid").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(file_id) = content.trim().parse::<usize>() {
                        if file_id == id {
                            if let Some(base) = path.file_stem().and_then(|s| s.to_str()) {
                                // Verify the session is actually alive
                                let port_path = crate::paths::port_file(base);
                                if std::path::Path::new(&port_path).exists() {
                                    return Some(base.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Clean up any stale port files (where server is not actually running)
pub fn cleanup_stale_port_files() {
    let Some(psmux_dir) = crate::paths::psmux_dir_opt() else {
        return;
    };
    cleanup_stale_port_files_in(Path::new(&psmux_dir));
}

fn cleanup_stale_port_files_in(psmux_dir: &Path) {
    cleanup_stale_port_files_in_with(psmux_dir, probe_session_for_cleanup);
}

/// Registry file extensions that only ever exist as satellites of a `.port`
/// entry. Anything else in the data dir (`next_session_id`, its `.lock`, debug
/// logs, the `instances/` and `servers/` subdirectories) is never touched.
const ORPHAN_REGISTRY_EXTS: &[&str] = &["sid", "key", "pid", "spawnlock", "act"];

/// How long a `.port`-less registry file must sit untouched before it is
/// considered abandoned (issue #530).
///
/// `ensure_session_registry_files` writes `.sid`/`.key`/`.pid` BEFORE the
/// `.port` beacon, so a perfectly healthy server that is still coming up
/// briefly looks exactly like an orphan, and it is only during that window that
/// this bound is load-bearing (the 5s registry self-heal re-writes a file only
/// when its contents changed, so a live server's mtimes do NOT keep advancing).
/// Once the server is up its `.port` is present and the whole set is skipped
/// outright. One minute is therefore far beyond any legitimate window while
/// still clearing the backlog on the next CLI invocation.
const ORPHAN_REGISTRY_GRACE: Duration = Duration::from_secs(60);

/// Most files a single sweep may remove.
///
/// psmux CLI commands are invoked constantly by scripts and prompts, and the
/// backlog this fix targets can run to thousands of files. Deleting all of it
/// inside one arbitrary invocation — `psmux -V`, say — makes a trivial command
/// do a surprising amount of destructive work and stalls it on I/O. The budget
/// keeps any one invocation cheap and bounded; the backlog drains across
/// successive sweeps instead, which is just as effective and far less abrupt.
const ORPHAN_REGISTRY_SWEEP_BUDGET: usize = 256;

/// Minimum interval between sweeps, tracked by [`ORPHAN_REGISTRY_SWEEP_STAMP`].
///
/// Without this, every psmux invocation pays a full directory walk. Orphans are
/// produced only by session teardown, so there is nothing to gain from looking
/// more often than this.
const ORPHAN_REGISTRY_SWEEP_INTERVAL: Duration = Duration::from_secs(300);

/// Marker file recording when the last sweep ran. Leading dot, so it has no
/// extension and can never be mistaken for a registry satellite.
const ORPHAN_REGISTRY_SWEEP_STAMP: &str = ".registry_sweep";

/// Delete per-session registry files whose `.port` entry is already gone
/// (issue #530).
///
/// Every other sweep in this module enumerates `.port` files and deletes the
/// siblings it finds. That makes the `.port` file the sole entry point to the
/// registry, so the moment one is removed while a sibling survives, the
/// survivor becomes permanently unreachable: no code path ever looks at it
/// again, and nothing can ever delete it. Teardown paths that remove
/// `.port`/`.key`/`.pid` but not `.sid` therefore leak one file per session
/// forever.
///
/// The cost is not only disk. `resolve_session_by_id` scans every `.sid` file
/// in the directory to map `$N` to a session, so each leaked file is paid for
/// on every lookup.
pub fn prune_orphaned_registry_files() {
    let Some(psmux_dir) = crate::paths::psmux_dir_opt() else {
        return;
    };
    let psmux_dir = Path::new(&psmux_dir);
    if !registry_sweep_due(psmux_dir, ORPHAN_REGISTRY_SWEEP_INTERVAL) {
        return;
    }
    prune_orphaned_registry_files_in(psmux_dir);
}

/// Whether a sweep is due, re-stamping the marker when it is.
///
/// The stamp is written BEFORE the sweep runs, not after: if the process is
/// killed partway through, the next invocation waits a full interval rather
/// than immediately retrying the same work.
fn registry_sweep_due(psmux_dir: &Path, interval: Duration) -> bool {
    let stamp = psmux_dir.join(ORPHAN_REGISTRY_SWEEP_STAMP);
    let swept_recently = stamp
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|age| age < interval)
        .unwrap_or(false); // missing or unreadable stamp -> sweep
    if swept_recently {
        return false;
    }
    let _ = std::fs::write(&stamp, b"");
    true
}

fn prune_orphaned_registry_files_in(psmux_dir: &Path) -> usize {
    let satellites = prune_orphaned_registry_files_in_with(
        psmux_dir,
        ORPHAN_REGISTRY_GRACE,
        ORPHAN_REGISTRY_SWEEP_BUDGET,
        pid_owns_live_server,
    );
    // Namespace tokens are the same leak in a subdirectory: bounded by distinct
    // namespace NAMES, which is unbounded for disposable `-L` namespaces (#530).
    // Run it after the satellite sweep so it sees the `.port` set that pass left,
    // and give it whatever is left of this sweep's budget so the two passes
    // together stay bounded rather than each costing a full budget.
    satellites
        + prune_orphaned_instance_tokens_in_with(
            psmux_dir,
            ORPHAN_REGISTRY_GRACE,
            ORPHAN_REGISTRY_SWEEP_BUDGET.saturating_sub(satellites),
        )
}

/// Core of [`prune_orphaned_registry_files`], with the grace period, the
/// per-sweep budget and the liveness oracle injected so tests can drive it
/// deterministically.
///
/// Returns the number of files removed.
fn prune_orphaned_registry_files_in_with<F>(
    psmux_dir: &Path,
    grace: Duration,
    budget: usize,
    mut is_live: F,
) -> usize
where
    F: FnMut(u32) -> bool,
{
    if budget == 0 {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(psmux_dir) else {
        return 0;
    };
    let mut pruned = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ORPHAN_REGISTRY_EXTS.contains(&ext) {
            continue;
        }
        // A surviving `.port` means this entry still belongs to the port-driven
        // sweep, which owns the liveness decision for the whole set.
        if path.with_extension("port").exists() {
            continue;
        }
        // Too young to judge: could be a server mid-startup that hasn't
        // published its port yet. Leave it for a later invocation.
        let old_enough = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|age| age >= grace)
            .unwrap_or(false); // unreadable mtime -> keep
        if !old_enough {
            continue;
        }
        // If the set still names a live psmux process, the server exists but is
        // not publishing a port (still binding, or wedged). Reaping that is
        // #448's job, not ours — deleting its identity files would only make it
        // harder to find.
        if let Some(pid) = orphan_anchor_pid(&path, ext) {
            if is_live(pid) {
                continue;
            }
        }
        if std::fs::remove_file(&path).is_ok() {
            pruned += 1;
            if crate::debug_log::session_log_enabled() {
                crate::debug_log::session_log(
                    "cleanup",
                    &format!(
                        "pruned orphaned '{}': no .port sibling and no live owner",
                        path.file_name().and_then(|s| s.to_str()).unwrap_or("?")
                    ),
                );
            }
            // Budget spent: leave the rest for the next sweep so no single
            // invocation does an unbounded amount of destructive work.
            if pruned >= budget {
                break;
            }
        }
    }
    pruned
}

/// PID that owns an orphaned registry file, when one can be determined.
///
/// A `.spawnlock` records its holder's PID directly; every other satellite is
/// anchored by the sibling `.pid` sentinel. `.sid`/`.key` orphans left by a
/// teardown that already removed the `.pid` have no anchor at all, which is
/// precisely the abandoned case.
fn orphan_anchor_pid(path: &Path, ext: &str) -> Option<u32> {
    let body = if ext == "spawnlock" {
        std::fs::read_to_string(path).ok()?
    } else {
        std::fs::read_to_string(path.with_extension("pid")).ok()?
    };
    parse_pid_file_contents(&body)
        .map(|(pid, _creation)| pid)
        .or_else(|| body.trim().parse::<u32>().ok())
}

/// True when `pid` is a live process running a psmux server image.
///
/// The process-table query is Windows-only. Elsewhere this answers "live", so
/// an orphan that still carries a PID anchor is kept rather than deleted on a
/// guess. Orphans with no anchor — the overwhelming majority, and the ones
/// #530 is about — are unaffected by the platform and prune everywhere.
fn pid_owns_live_server(pid: u32) -> bool {
    if !cfg!(windows) {
        return true;
    }
    match crate::platform::process_info::get_process_name(pid) {
        None => false,
        Some(name) => PSMUX_SERVER_IMAGE_NAMES.contains(&name.to_ascii_lowercase().as_str()),
    }
}

/// Instance-token file names (`instances/<prefix>-<hash>`) that a namespace with
/// a live server could be using.
///
/// A token file name is a hash of the namespace, so it cannot be read backwards
/// into one. The mapping is therefore built forwards from the live `.port`
/// files: a registry base is `<ns>__<session>` for a `-L` namespace and a bare
/// `<session>` for the default one, and BOTH halves may themselves contain
/// `__`, so every prefix ending at a `__` is claimed. Over-claiming is the safe
/// direction: it can only keep a token that nothing is using, never delete one
/// that a live namespace depends on.
fn live_instance_token_names(psmux_dir: &Path) -> std::collections::HashSet<std::ffi::OsString> {
    let mut live = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir(psmux_dir) else {
        return live;
    };
    fn claim(
        dir: &Path,
        ns: Option<&str>,
        set: &mut std::collections::HashSet<std::ffi::OsString>,
    ) {
        if let Some(name) = crate::paths::namespace_instance_file(dir, ns).file_name() {
            set.insert(name.to_os_string());
        }
    }
    let mut any_port = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "port").unwrap_or(true) {
            continue;
        }
        any_port = true;
        let Some(base) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let bytes = base.as_bytes();
        for i in 1..bytes.len().saturating_sub(1) {
            // `_` is ASCII, so a match is always a char boundary.
            if bytes[i] == b'_' && bytes[i + 1] == b'_' {
                claim(psmux_dir, Some(&base[..i]), &mut live);
            }
        }
    }
    // Any live server at all may be a default-namespace one: a bare `<session>`
    // base is indistinguishable from a namespaced base whose split we cannot
    // pin down, so the default token is kept whenever anything is running.
    if any_port {
        claim(psmux_dir, None, &mut live);
    }
    live
}

/// Delete namespace identity tokens (issue #509's `instances/`) for namespaces
/// that have no live server left (issue #530).
///
/// #509 argued that tokens need no teardown because the next server in a
/// namespace re-mints one anyway. That holds for a fixed set of namespace
/// names; it does not for callers that mint a throwaway `-L` namespace per run,
/// which is exactly what namespaces are good for. Since a token for a dead
/// namespace is discarded and re-minted the moment that namespace is used again
/// (`ensure_namespace_instance_in` re-decides when it finds no live peer),
/// deleting it early is observationally identical and keeps the directory
/// bounded by LIVE namespaces rather than by every name ever used.
fn prune_orphaned_instance_tokens_in_with(
    psmux_dir: &Path,
    grace: Duration,
    budget: usize,
) -> usize {
    if budget == 0 {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(crate::paths::instance_dir_in(psmux_dir)) else {
        // No instances dir: skip the parent walk `live_instance_token_names`
        // would otherwise pay for nothing.
        return 0;
    };
    let live = live_instance_token_names(psmux_dir);
    let mut pruned = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name() else { continue };
        if live.contains(name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        // Same startup guard as the satellite sweep: a server establishes its
        // namespace identity before it publishes a `.port`, so a young token
        // may belong to a namespace that is still coming up.
        let old_enough = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age >= grace)
            .unwrap_or(false); // unreadable mtime -> keep
        if !old_enough {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            pruned += 1;
            if crate::debug_log::session_log_enabled() {
                crate::debug_log::session_log(
                    "cleanup",
                    &format!(
                        "pruned namespace token '{}': no live server in that namespace",
                        name.to_string_lossy()
                    ),
                );
            }
            // Same budget rule as the satellite sweep: leave the rest for the
            // next one rather than doing unbounded destructive work here.
            if pruned >= budget {
                break;
            }
        }
    }
    pruned
}

/// Image-name stems (lower-case, no extension) that count as a psmux server for
/// the orphan reaper. Only processes whose executable matches one of these are
/// ever candidates for termination — an unrelated app that happens to hold a
/// loopback listener is never touched.
const PSMUX_SERVER_IMAGE_NAMES: &[&str] = &["psmux", "tmux", "pmux"];

/// Grace period before a live server process is eligible for orphan reaping.
/// A server that just bound its socket but hasn't finished writing its `.port`
/// file yet (or a concurrent `new-session` still coming up) would otherwise look
/// untracked; requiring the process to be older than this avoids that race. The
/// spawn-race itself is fixed in #444 — this reaper is only the accumulation
/// backstop, so it can afford to skip very young processes and catch them next
/// startup instead.
const ORPHAN_REAP_MIN_AGE: Duration = Duration::from_secs(10);

/// A live psmux server process discovered by the reaper: its PID, every loopback
/// port it listens on, and its process creation time (FILETIME 100ns ticks).
#[derive(Clone, Debug, PartialEq)]
struct ServerCandidate {
    pid: u32,
    ports: Vec<u16>,
    creation_ft: u64,
}

/// Pure orphan-selection policy (unit-testable, no OS calls).
///
/// A candidate server is an orphan to reap iff ALL hold:
///  - it is not this very process (`self_pid`),
///  - its PID is not recorded in any live registry entry (`tracked_pids`),
///  - NONE of its listening ports is claimed by a registry `.port` file
///    (`tracked_ports`) — i.e. nothing references this server, so it is a
///    duplicate / lost headless server rather than a legitimate session,
///  - it was created at or before `age_cutoff_ft` (older than the grace window),
///    so a just-spawned server still writing its registry files is never reaped.
///
/// The port check is the primary anchor: a legitimate server ALWAYS has a
/// `.port` file pointing at it, so it can never be selected even if its `.pid`
/// file is missing (backward compatibility with servers started before #448).
fn select_orphan_pids(
    _candidates: &[ServerCandidate],
    _tracked_ports: &std::collections::HashSet<u16>,
    _tracked_pids: &std::collections::HashSet<u32>,
    _owned_pids: &std::collections::HashMap<u32, Option<u64>>,
    _self_pid: u32,
    _age_cutoff_ft: u64,
) -> Vec<u32> {
    // Missing metadata cannot establish that a live session is disposable.
    // Process termination is restricted to explicit kill operations.
    Vec::new()
}

/// Read the set of ports referenced by `.port` files and the set of PIDs
/// recorded in `.pid` files whose sibling `.port` still exists. A `.pid` without
/// a live `.port` is ignored so a dead-then-reused PID can't be treated as
/// tracked.
fn read_tracked_registry(psmux_dir: &Path)
    -> (std::collections::HashSet<u16>, std::collections::HashSet<u32>)
{
    let mut tracked_ports = std::collections::HashSet::new();
    let mut tracked_pids = std::collections::HashSet::new();
    if let Ok(entries) = std::fs::read_dir(psmux_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("port") => {
                    if let Ok(s) = std::fs::read_to_string(&path) {
                        if let Ok(p) = s.trim().parse::<u16>() { tracked_ports.insert(p); }
                    }
                }
                Some("pid") => {
                    // Only trust a PID whose session still has a live .port file.
                    let port_sibling = path.with_extension("port");
                    if port_sibling.exists() {
                        if let Ok(s) = std::fs::read_to_string(&path) {
                            // Tolerate both `pid` and `pid:creation_filetime` bodies.
                            if let Some((pid, _)) = parse_pid_file_contents(&s) { tracked_pids.insert(pid); }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (tracked_ports, tracked_pids)
}

// --- kill-server force-kill fallback: data-dir-scoped, identity-checked -------
// tmux's kill-server is socket-scoped. psmux's bare kill-server sends a graceful
// kill-server to every session in scope; a wedged server that ignores it must be
// force-killed. The old fallback scanned every process on the machine by image
// name (psmux/pmux/tmux) and TerminateProcess'd them — machine-wide, reaching
// unrelated servers and other namespaces. These helpers replace that with a
// selection scoped by construction to this data dir's `.pid` files, gated by an
// exact process-creation-time match so a recycled pid is never killed.

/// A force-kill target read from a data dir's registry: the recorded server pid
/// and the creation FILETIME it had when it wrote the pid file. The pair is what
/// defeats pid reuse — a recycled pid will not carry the same creation time.
#[derive(Debug, PartialEq)]
pub struct PidTarget {
    pub pid: u32,
    pub creation_time: u64,
}

/// Parse a `.pid` file body. Two forms are accepted: a bare `pid` (the #448
/// liveness anchor as first written) and `pid:creation_filetime` (extended so
/// kill-server can verify identity). Returns `(pid, Option<creation_time>)`, or
/// `None` when the pid itself is unparseable. One parser so every reader
/// (`force_kill_targets`, the orphan reaper, the pid anchor) stays in step.
pub fn parse_pid_file_contents(s: &str) -> Option<(u32, Option<u64>)> {
    let s = s.trim();
    match s.split_once(':') {
        Some((pid_str, time_str)) => Some((pid_str.trim().parse().ok()?, time_str.trim().parse().ok())),
        None => Some((s.parse().ok()?, None)),
    }
}

/// The `.pid` body the server writes: `pid:creation_filetime`. Kept as one
/// function so the producer and `parse_pid_file_contents` cannot drift apart.
pub fn format_pid_file_contents(pid: u32, creation_time: u64) -> String {
    format!("{pid}:{creation_time}")
}

/// Force-kill candidates for `kill-server`'s fallback, scoped by construction to
/// a single data dir: the `pid:creation_filetime` `.pid` files in `dir`. When
/// `ns_prefix` is `Some`, only files whose base starts with it are considered —
/// mirroring the graceful pass's `-L` filter, so a namespaced kill-server never
/// reaches another namespace. Bare-pid and malformed files are skipped (no
/// recorded creation time means no identity gate, so they are not force-kill
/// candidates). This selects targets; it does not kill.
pub fn force_kill_targets(dir: &std::path::Path, ns_prefix: Option<&str>) -> Vec<PidTarget> {
    let mut targets = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return targets; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "pid").unwrap_or(false) {
            if let Some(pfx) = ns_prefix {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !session_visible_from(stem, pfx.strip_suffix("__")) { continue; }
            }
            if let Ok(contents) = std::fs::read_to_string(&path) {
                // A recorded creation time is required: it is the identity gate.
                // Bare-pid files carry none, so they are not force-kill candidates.
                if let Some((pid, Some(creation_time))) = parse_pid_file_contents(&contents) {
                    targets.push(PidTarget { pid, creation_time });
                }
            }
        }
    }
    targets
}

const SESSION_REGISTRY_EXTENSIONS: &[&str] = &["port", "key", "sid", "pid", "act", "registry.json"];

struct KillRegistrySnapshot {
    base: String,
    dir: std::path::PathBuf,
    identity: Option<PidTarget>,
    files: Vec<Option<Vec<u8>>>,
}

fn snapshot_kill_registry(dir: &Path, base: &str) -> io::Result<KillRegistrySnapshot> {
    use std::io::Read;
    let mut files = Vec::new();
    for ext in SESSION_REGISTRY_EXTENSIONS {
        let path = dir.join(format!("{}.{}", base, ext));
        let bytes = match std::fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(16385).read_to_end(&mut bytes)?;
                if bytes.len() > 16384 { return Err(io::Error::new(ErrorKind::InvalidData, "oversized registry file")); }
                Some(bytes)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        files.push(bytes);
    }
    let identity = if let Some(bytes) = files[5].as_ref() {
        let m: crate::registry::RegistryManifest = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        Some(PidTarget { pid: m.pid, creation_time: m.creation_time })
    } else {
        files[3].as_deref().and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(parse_pid_file_contents).and_then(|(pid, creation)| creation.map(|creation_time| PidTarget { pid, creation_time }))
    };
    Ok(KillRegistrySnapshot { base: base.to_string(), dir: dir.to_path_buf(), identity, files })
}

/// Only explicit kill uses this cleanup. Hold the destination name reservation,
/// prove the snapshotted process generation exited, and compare every sibling
/// before deleting anything. A new registration always wins this race.
fn cleanup_killed_registry(snapshot: &KillRegistrySnapshot) -> io::Result<bool> {
    let Some(identity) = snapshot.identity.as_ref() else { return Ok(false); };
    if !crate::platform::process_kill::verified_process_dead(identity.pid, identity.creation_time) { return Ok(false); }
    let Some(_guard) = crate::platform::acquire_session_mutex_strict(&snapshot.base)? else { return Ok(false); };
    let current = snapshot_kill_registry(&snapshot.dir, &snapshot.base)?;
    if current.files != snapshot.files { return Ok(false); }
    if !crate::platform::process_kill::verified_process_dead(identity.pid, identity.creation_time) { return Ok(false); }
    for (ext, bytes) in SESSION_REGISTRY_EXTENSIONS.iter().zip(&snapshot.files) {
        if bytes.is_some() {
            match std::fs::remove_file(snapshot.dir.join(format!("{}.{}", snapshot.base, ext))) {
                Ok(()) => {},
                Err(e) if e.kind() == ErrorKind::NotFound => {},
                Err(e) => return Err(e),
            }
        }
    }
    Ok(true)
}

/// Explicit kill scoped to the effective namespace; no process-name searches,
/// and no registry deletion on a failed connection or authentication attempt.
pub fn kill_registered_servers(namespace: Option<&str>) -> io::Result<()> {
    let dir = std::path::PathBuf::from(crate::paths::psmux_dir());
    let mut bases = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue; };
            let base = name.strip_suffix(".port").or_else(|| name.strip_suffix(".registry.json"))
                .or_else(|| name.strip_suffix(".pid"));
            if let Some(base) = base {
                if namespace.is_none() || session_visible_from(base, namespace) { bases.insert(base.to_string()); }
            }
        }
    }
    let mut failures = Vec::new();
    for base in bases {
        let snapshot = match snapshot_kill_registry(&dir, &base) {
            Ok(snapshot) => snapshot,
            Err(error) => { failures.push(format!("{}: {}", crate::paths::registry_session_name(&base), error)); continue; }
        };
        if let Ok((port, key)) = read_session_endpoint(&base) {
            if let Ok(mut response) = open_authed(&format!("127.0.0.1:{}", port), &key, b"kill-server\n",
                Duration::from_millis(500), Duration::from_millis(2000)) {
                let _ = response.read_all();
            }
        }
        if let Some(identity) = snapshot.identity.as_ref() {
            // Exact check and termination happen on the same OS handle.
            if !crate::platform::process_kill::verified_process_dead(identity.pid, identity.creation_time) {
                crate::platform::process_kill::terminate_server_pid_exact(identity.pid, identity.creation_time);
            }
            let deadline = std::time::Instant::now() + Duration::from_millis(1000);
            while !crate::platform::process_kill::verified_process_dead(identity.pid, identity.creation_time)
                && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if crate::platform::process_kill::verified_process_dead(identity.pid, identity.creation_time) {
                cleanup_killed_registry(&snapshot)?;
            } else { failures.push(format!("{}: server exit could not be confirmed", crate::paths::registry_session_name(&base))); }
        } else if snapshot.files.iter().any(Option::is_some) {
            // Old servers without a precise anchor can clean themselves. They
            // do not grant permission for a PID-based force kill or file wipe.
            if snapshot_kill_registry(&dir, &base)?.files.iter().any(Option::is_some) {
                failures.push(format!("{}: server identity unavailable; registry preserved", crate::paths::registry_session_name(&base)));
            }
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(io::Error::other(failures.join("; "))) }
}

/// The exact-match identity gate for the force-kill fallback: terminate only when
/// the live process at the pid still carries the creation time recorded in the
/// pid file. `queried` is the process's current creation FILETIME (None if it
/// could not be read); `expected` is the value from the pid file. A recycled pid
/// carries a different creation time and is rejected; an unreadable process is
/// rejected too, so the fallback fails safe and never kills on uncertainty.
pub fn confirms_identity(queried: Option<u64>, expected: u64) -> bool {
    queried == Some(expected)
}

/// `.pid` registry entries belonging to namespace `ns`, excluding `self_pid`
/// (issue #509).
///
/// Membership is decided by the file-name convention: a `-L` namespace writes
/// `<ns>__<session>.pid`, while the default namespace writes a bare
/// `<session>.pid`. The warm helper is included in both cases — it is a real
/// server, so while it lives the namespace has not gone away.
pub fn namespace_peer_pids(dir: &Path, ns: Option<&str>, self_pid: u32) -> Vec<PidTarget> {
    let mut peers = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return peers; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "pid").unwrap_or(true) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let mine = session_visible_from(stem, ns);
        if !mine {
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Some((pid, Some(creation_time))) = parse_pid_file_contents(&contents) {
                if pid != self_pid {
                    peers.push(PidTarget { pid, creation_time });
                }
            }
        }
    }
    peers
}

/// Whether the calling server is the first live server in its namespace, i.e.
/// no peer from the registry is still running under its recorded identity.
///
/// `creation_of` supplies each pid's current creation FILETIME (`None` when the
/// process is gone or unreadable) so the decision is testable without OS process
/// enumeration. A pid whose creation time no longer matches has been recycled
/// and is somebody else's process, not our peer.
pub fn is_first_server_in_namespace(
    peers: &[PidTarget],
    creation_of: impl Fn(u32) -> Option<u64>,
) -> bool {
    !peers
        .iter()
        .any(|p| confirms_identity(creation_of(p.pid), p.creation_time))
}

/// Mint a fresh namespace identity token.
fn mint_instance_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    );
    h.write_u64(std::process::id() as u64);
    format!("{:016x}", h.finish())
}

/// Establish this namespace's stable identity (issue #509), returning the token
/// now in force.
///
/// psmux runs one server process per session, so a per-process value such as
/// `#{pid}` changes every time a session is created and a supervisor reads that
/// as a server restart. Every server in a namespace instead reads one shared
/// token file, so it does not matter which server answers a query.
///
/// The token is minted by the namespace's first server and left alone by every
/// later one. It is re-minted only when no peer is still alive — that is a
/// genuine restart, and a supervisor must be able to see it. Reclamation is
/// therefore self-healing: a token left behind by a namespace that died is
/// replaced by the next server to start, so nothing needs to delete it on exit
/// (a server can exit via exit-empty, kill-session, or a crash).
///
/// `established` is the token this server already holds from a previous call,
/// or `None` on the first call of the process. The first-server decision — and
/// the re-mint it triggers — belongs to server STARTUP only: this function is
/// also re-run every few seconds by the registry self-heal loop, and a lone
/// server (single session, no warm helper) sees no live peers on every tick.
/// Re-deciding there would delete and re-mint its own token every interval,
/// churning the very identity this feature exists to keep stable. Once a token
/// is established, re-ensure only restores the file if it was lost — with the
/// SAME token, so even losing the file does not fake a restart.
pub fn ensure_namespace_instance_in(
    dir: &Path,
    ns: Option<&str>,
    self_pid: u32,
    creation_of: impl Fn(u32) -> Option<u64>,
    established: Option<&str>,
) -> Option<String> {
    let path = crate::paths::namespace_instance_file(dir, ns);

    match established {
        // Steady state: this server already holds the namespace's identity.
        // While it is alive the namespace cannot have restarted, so never
        // delete or re-mint — only put the established token back if the file
        // went missing.
        Some(token) => {
            if !write_token_if_missing(&path, token) {
                return None;
            }
        }
        // Startup: decide whether this is a fresh namespace or a join.
        None => {
            let peers = namespace_peer_pids(dir, ns, self_pid);
            if is_first_server_in_namespace(&peers, creation_of) {
                // The namespace this token described is gone; do not inherit its identity.
                let _ = std::fs::remove_file(&path);
            }
            if !write_token_if_missing(&path, &mint_instance_token()) {
                return None;
            }
        }
    }

    read_namespace_instance_in(dir, ns)
}

/// Create the token file with `token` unless it already exists (`create_new`,
/// so a concurrent first-start cannot clobber the winner's token). Returns
/// false only on an unexpected I/O failure.
fn write_token_if_missing(path: &Path, token: &str) -> bool {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            use std::io::Write as _;
            let _ = write!(f, "{}", token);
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => true,
        Err(_) => false,
    }
}

/// Read a namespace's identity token, or `None` when the namespace has none.
///
/// Reading never mints: only a starting server may establish identity, so a
/// client query against an unknown namespace reports nothing rather than
/// inventing a value.
pub fn read_namespace_instance_in(dir: &Path, ns: Option<&str>) -> Option<String> {
    let path = crate::paths::namespace_instance_file(dir, ns);
    let contents = std::fs::read_to_string(path).ok()?;
    let token = contents.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// The namespace identity this server process established at startup. One
/// server process serves exactly one namespace, so a process-wide cell is the
/// correct scope; it is what makes the periodic registry re-ensure a restore
/// rather than a fresh first-server decision (see
/// [`ensure_namespace_instance_in`]).
static ESTABLISHED_INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Production wrapper for [`ensure_namespace_instance_in`] against the real data
/// directory and live process table.
pub fn ensure_namespace_instance(ns: Option<&str>, self_pid: u32) -> Option<String> {
    let dir = crate::paths::psmux_dir_opt()?;
    let token = ensure_namespace_instance_in(
        Path::new(&dir),
        ns,
        self_pid,
        |pid| crate::platform::process_kill::process_creation_time(pid),
        ESTABLISHED_INSTANCE.get().map(|s| s.as_str()),
    )?;
    let _ = ESTABLISHED_INSTANCE.set(token.clone());
    Some(token)
}

/// Production wrapper for [`read_namespace_instance_in`].
pub fn read_namespace_instance(ns: Option<&str>) -> Option<String> {
    let dir = crate::paths::psmux_dir_opt()?;
    read_namespace_instance_in(Path::new(&dir), ns)
}

/// Terminate live psmux *server* processes that no registry entry accounts for
/// (issue #448). Complements `cleanup_stale_port_files`, which only removes
/// registry files for servers already proven dead: this pass finds a live but
/// orphaned server (a spawn-race duplicate, or a crashed client's headless
/// server) and reaps the process itself, bounding the process count regardless
/// of how the duplicate arose.
pub fn reap_orphaned_servers() {
    // Kept as a compatibility entry point. Live orphan cleanup is unsafe:
    // a busy server can temporarily lose registry files and still own panes.
}

fn reap_orphaned_servers_in(_psmux_dir: &Path) {}

/// Delete ownership markers whose process is gone or whose PID now belongs to
/// something else, so the directory tracks live servers rather than growing
/// without bound. Kept separate from reaping: a marker is only a claim, and
/// dropping a claim never terminates anything.
///
/// This is the sole reclamation path rather than a removal on server shutdown:
/// a server can exit down several routes (exit-empty, kill-session, a crash)
/// and a marker missed by any of them would linger forever, so the invariant is
/// "markers are reconciled against the process table", not "every exit tidies
/// up after itself". A lingering marker is harmless in the meantime — it names
/// either a dead PID or, after reuse, a process whose creation time no longer
/// matches, and neither can authorise a kill.
fn prune_stale_server_markers(
    psmux_dir: &Path,
    owned_pids: &std::collections::HashMap<u32, Option<u64>>,
) {
    for (&pid, &recorded_creation) in owned_pids {
        let live_creation = crate::platform::process_kill::process_creation_time(pid);
        let still_ours = match (live_creation, recorded_creation) {
            // Process is gone.
            (None, _) => false,
            // Same PID, different process: the original exited and the PID was
            // reused, so this claim is stale.
            (Some(live), Some(recorded)) => live == recorded,
            // No recorded creation time to compare against. Keep it: a marker
            // that cannot be identity-checked can never authorise a kill
            // either, so leaving it costs nothing.
            (Some(_), None) => true,
        };
        if !still_ours {
            let _ = std::fs::remove_file(psmux_dir.join("servers").join(pid.to_string()));
        }
    }
}

/// Windows FILETIME ticks (100ns since 1601-01-01) for a `SystemTime`.
/// Used to compare a process creation time against a registry file mtime.
fn system_time_to_filetime_ticks(t: SystemTime) -> Option<u64> {
    const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    let since_unix = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(UNIX_EPOCH_AS_FILETIME.saturating_add((since_unix.as_nanos() / 100) as u64))
}

/// Slack allowed between a `.pid` file's last write and the recorded process's
/// creation time before the PID is considered recycled. A legitimate server is
/// always created BEFORE it writes its `.pid`, so its creation time can never
/// exceed the file mtime; the margin only absorbs filesystem/clock jitter.
const PID_REUSE_MARGIN_TICKS: u64 = 60 * 10_000_000; // 60s in 100ns ticks

/// Definitive liveness verdict from the `.pid` sentinel written next to every
/// `.port` file (issue #448) — no network round-trip.
///
/// This is what keeps CLI startup O(microseconds) per registry entry: a TCP
/// probe of a dead port can burn its full connect timeout on Windows (stealth
/// firewall behavior never sends RST on loopback for some configurations) and
/// then classify as Inconclusive, leaving the stale file to tax EVERY future
/// invocation. The process table answers instantly and definitively.
///
/// Returns:
///   Some(true)  - recorded PID is a live psmux-image process created no later
///                 than the `.pid` file was written -> genuinely our server.
///   Some(false) - PID gone, recycled by a non-psmux image, or recycled by a
///                 psmux process created long after the file -> server is dead.
///   None        - no usable `.pid` anchor (pre-#448 registry) -> caller must
///                 fall back to the network probe.
fn pid_anchor_verdict(port_path: &Path) -> Option<bool> {
    // The process-table queries below are Windows-only; other platforms fall
    // back to the network probe rather than misreading stub returns as "dead".
    if !cfg!(windows) {
        return None;
    }
    let pid_path = port_path.with_extension("pid");
    // Tolerate both `pid` and `pid:creation_filetime` bodies (the latter written
    // so kill-server can verify identity); the anchor only needs the pid.
    let (pid, creation) = parse_pid_file_contents(&std::fs::read_to_string(&pid_path).ok()?)?;
    use crate::platform::process_kill::{process_identity, ProcessIdentity};
    let observed_creation = match process_identity(pid) {
        ProcessIdentity::Alive(actual) => Some(actual),
        ProcessIdentity::Exited => return Some(false),
        ProcessIdentity::Unknown => return None,
    };
    if let Some(recorded) = creation {
        return observed_creation.map(|actual| actual == recorded);
    }
    // PID-reuse guard (same idea as the #447 reaper guard): a psmux process
    // created well AFTER the .pid file was last written cannot be the server
    // that wrote it. When either timestamp is unavailable, err towards alive.
    if let Some(created_ft) = observed_creation {
        if let Some(mtime_ft) = std::fs::metadata(&pid_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(system_time_to_filetime_ticks)
        {
            if created_ft > mtime_ft.saturating_add(PID_REUSE_MARGIN_TICKS) {
                return Some(false);
            }
        }
    }
    // Legacy bare PID records cannot establish ownership. A live process may
    // have reused the PID; query the authenticated endpoint instead.
    None
}

/// Resolve the session key stored alongside a `.port` file (the sibling
/// `.key`). Returns an empty string when the key file is missing, which the
/// identity probe treats as "cannot verify" (Inconclusive) rather than dead.
fn read_key_for_port_path(port_path: &Path) -> String {
    let key_path = port_path.with_extension("key");
    std::fs::read_to_string(&key_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Best-effort wall-clock time the system last booted, derived from uptime.
///
/// Any registry file last written before this instant cannot belong to a
/// server that has been running since the machine started — its owning
/// process died with the previous boot (e.g. an OS-update reboot). This is
/// the reliable signal for cleaning up sessions orphaned by a restart, and
/// it does not depend on the network (the old port may now be free, occupied
/// by an unrelated process, or even reused by a *different* live psmux
/// server — all of which a bare TCP probe would misclassify).
#[cfg(windows)]
fn system_boot_time() -> Option<SystemTime> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount64() -> u64;
    }
    let uptime_ms = unsafe { GetTickCount64() };
    SystemTime::now().checked_sub(Duration::from_millis(uptime_ms))
}

#[cfg(not(windows))]
fn system_boot_time() -> Option<SystemTime> {
    None
}

/// True when `mtime` is old enough (older than `boot - margin`) that the file
/// must have been written by a process from a previous boot.
fn is_pre_boot(mtime: SystemTime, boot: SystemTime, margin: Duration) -> bool {
    match boot.checked_sub(margin) {
        Some(cutoff) => mtime < cutoff,
        None => false,
    }
}

fn cleanup_stale_port_files_in_with<F>(psmux_dir: &Path, mut probe: F)
where
    F: FnMut(&str, u16) -> PortProbeResult,
{
    // Legacy entry point is now observational. Registry reclamation requires
    // an owned generation under the session reservation, not age or TCP state.
    if let Ok(entries) = std::fs::read_dir(psmux_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "port") {
                if let Ok(port) = std::fs::read_to_string(&path).unwrap_or_default().trim().parse::<u16>() {
                    let _ = probe(&read_key_for_port_path(&path), port);
                }
            }
        }
    }
}

/// Display name (file stem) of a registry path, for logging.
fn registry_base(port_path: &Path) -> &str {
    port_path.file_stem().and_then(|s| s.to_str()).unwrap_or("?")
}

/// Remove a session's entire registry set, given its `.port` path.
///
/// Teardown paths must go through this rather than deleting extensions
/// individually: the `.port` file is the only entry point the sweeps know, so
/// any satellite left behind when it disappears is unreachable forever (#530).
pub(crate) fn remove_session_registry_files(port_path: &Path) {
    let _ = std::fs::remove_file(port_path);
    let key_path = port_path.with_extension("key");
    let _ = std::fs::remove_file(&key_path);
    let sid_path = port_path.with_extension("sid");
    let _ = std::fs::remove_file(&sid_path);
    // Also drop the twin .pid sentinel (issue #448) so a dead server's PID
    // never lingers to be mistaken for a live tracked process by the reaper.
    let pid_path = port_path.with_extension("pid");
    let _ = std::fs::remove_file(&pid_path);
    // And the `.act` activity stamp (issue #603): a reaped session must not
    // keep ranking in bare CLI routing.
    let act_path = port_path.with_extension("act");
    let _ = std::fs::remove_file(&act_path);
}

/// Outcome of a single AUTH handshake against the listener on a port.
#[derive(Clone, Copy, PartialEq)]
enum AuthProbe {
    /// Server accepted our session key (`OK`) — this is genuinely our session.
    Authenticated,
    /// Server explicitly rejected the key (`ERROR ...`) — a *different* psmux
    /// server has reused this port; the session this file names is dead.
    Rejected,
    /// Connected but the peer didn't complete our protocol (no reply, garbage,
    /// or an unrelated process). Identity is unverifiable from the network.
    Unknown,
}

/// Connect to `addr` and verify, via the AUTH handshake, that the listener is
/// the psmux server that owns `key`.
///
/// Returns `Err(kind)` when the connection itself fails so the caller can tell
/// "nothing is listening" (refused) from a transient network error.
fn probe_auth_identity(addr: std::net::SocketAddr, key: &str) -> Result<AuthProbe, ErrorKind> {
    let mut s = std::net::TcpStream::connect_timeout(&addr, STALE_PORT_CONNECT_TIMEOUT)
        .map_err(|e| e.kind())?;
    // A successful connect alone proves only that *something* listens. Without
    // a key we cannot prove it is ours, so leave the verdict to the boot guard.
    let key = match validate_auth_key(key) {
        Some(k) => k,
        None => return Ok(AuthProbe::Unknown),
    };
    let _ = s.set_read_timeout(Some(STALE_PORT_AUTH_READ_TIMEOUT));
    let _ = s.set_nodelay(true);
    if write!(s, "AUTH {}\n", key).is_err() {
        return Ok(AuthProbe::Unknown);
    }
    let _ = s.flush();
    let mut br = std::io::BufReader::new(std::io::Read::take(s, 4096));
    let mut line = String::new();
    match std::io::BufRead::read_line(&mut br, &mut line) {
        Ok(0) => Ok(AuthProbe::Unknown),
        Ok(_) => {
            let t = line.trim();
            if t == "OK" {
                Ok(AuthProbe::Authenticated)
            } else if t.starts_with("ERROR") {
                Ok(AuthProbe::Rejected)
            } else {
                Ok(AuthProbe::Unknown)
            }
        }
        Err(_) => Ok(AuthProbe::Unknown),
    }
}

/// Identity-aware liveness probe used by stale-port cleanup.
///
/// A bare TCP connect cannot distinguish our server from any other process
/// that grabbed the same port after a crash or reboot — that false "alive"
/// is exactly what left dead sessions showing `(not responding)` in the
/// picker. This probe instead requires the AUTH key to match:
///   - connection refused (or a full timeout with zero response) on every
///     attempt                                    -> `Stale`
///   - server accepts our key (`OK`)              -> `Alive`
///   - server rejects our key (`ERROR`, reused port) -> `Stale`
///   - anything ambiguous (no reply, slow, foreign process) -> `Inconclusive`
///
/// Only definitive signals delete a file; ambiguous ones are left for the
/// boot-time guard, so a live-but-busy server is never reaped by mistake.
///
/// Issue #7 batch D: some environments (confirmed on this host via a raw
/// `TcpClient.BeginConnect`/`WaitOne` probe against several unbound loopback
/// ports) never return an immediate ECONNREFUSED for a closed loopback
/// port — the SYN is silently dropped and the connect attempt runs the full
/// `STALE_PORT_CONNECT_TIMEOUT` before giving up, surfacing as
/// `ErrorKind::TimedOut` rather than `ConnectionRefused`. The original code
/// treated any `TimedOut` as ambiguous ("maybe a live-but-slow server"),
/// which on such a host means the network probe can NEVER return `Stale` —
/// orphaned `.port`/`.key` files with no `.pid` anchor (the only registry
/// shape old enough to still reach this probe) are kept forever. A real
/// live server on loopback completes the AUTH handshake in well under a
/// millisecond; three full timeouts in a row with no byte of response ever
/// received is just as definitive a "nothing is there" signal as an
/// instant refusal, so treat it the same — but ONLY when every attempt saw
/// refused/timed-out and nothing else (any actual response, however
/// unparseable, still keeps the conservative `Inconclusive` verdict).
fn probe_session_for_cleanup(key: &str, port: u16) -> PortProbeResult {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match probe_auth_identity(addr, key) {
        Ok(AuthProbe::Authenticated) => PortProbeResult::Alive,
        _ => PortProbeResult::Inconclusive,
    }
}

/// Read the session key from the key file
pub fn read_session_key(session: &str) -> io::Result<String> {
    let keypath = crate::paths::key_file(session);
    std::fs::read_to_string(&keypath).map(|s| s.trim().to_string())
}

/// Read one coherent endpoint generation, preferring the atomic manifest.
/// A malformed manifest is a real error; it must not fall back to mixed files.
pub fn read_session_endpoint(base: &str) -> io::Result<(u16, String)> {
    if let Some(manifest) = crate::registry::read_manifest(base)? {
        if validate_auth_key(&manifest.key).is_none() {
            return Err(io::Error::new(ErrorKind::InvalidData, "invalid session credentials"));
        }
        return Ok((manifest.port, manifest.key));
    }
    let port = std::fs::read_to_string(crate::paths::port_file(base))?
        .trim().parse::<u16>()
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "invalid session port"))?;
    if port == 0 { return Err(io::Error::new(ErrorKind::InvalidData, "invalid session port")); }
    let key = read_session_key(base)?;
    if validate_auth_key(&key).is_none() {
        return Err(io::Error::new(ErrorKind::InvalidData, "invalid session credentials"));
    }
    Ok((port, key))
}

pub fn existing_session_state(base: &str) -> SessionLiveness {
    if registry_pid_anchor_alive(base) == Some(false) { return SessionLiveness::Dead; }
    match read_session_endpoint(base) {
        Ok((port, key)) => probe_session_liveness(&format!("127.0.0.1:{}", port), &key,
            Duration::from_millis(500), Duration::from_millis(2000)),
        Err(_) => SessionLiveness::Unresponsive,
    }
}

/// Hard cap on a single response payload read from the server (256 KB).
///
/// The server is trusted, but the client should still bound how much memory
/// a single picker fetch can consume. A buggy or malicious peer that sends
/// an unbounded line with no `\n` would otherwise block until the read
/// timeout while filling the BufReader. 256 KB is comfortably larger than
/// any real `session-info`, `list-tree`, or `choose-buffer` payload.
pub const MAX_AUTHED_RESPONSE_BYTES: u64 = 256 * 1024;

/// Validate that a session key is well-formed for the line-oriented AUTH
/// protocol. Rejects keys containing CR, LF, or NUL — anything that could
/// terminate the AUTH line early or smuggle a second protocol frame.
///
/// Returns the trimmed key on success, `None` on rejection.
///
/// SECURITY: Without this check, a key sourced from a future caller (e.g.
/// env var, IPC, plugin) that contains `\n` could inject a second command
/// onto the AUTH line. All AUTH writers should funnel through this guard.
pub fn validate_auth_key(key: &str) -> Option<&str> {
    let k = key.trim_matches(|c: char| c == '\r' || c == '\n');
    if k.is_empty() {
        return None;
    }
    if k.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return None;
    }
    Some(k)
}

/// Send an authenticated command to a server (fire-and-forget).
///
/// Validates the key against CRLF/NUL injection. Silently no-ops on a
/// malformed key — callers are at the trust boundary already (key file
/// under user's profile), this is defense-in-depth.
pub fn send_auth_cmd(addr: &str, key: &str, cmd: &[u8]) -> io::Result<()> {
    let key = match validate_auth_key(key) {
        Some(k) => k,
        None => return Ok(()),
    };
    let sock_addr: std::net::SocketAddr = addr.parse().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    if let Ok(mut s) = std::net::TcpStream::connect_timeout(&sock_addr, Duration::from_millis(50)) {
        let _ = s.set_nodelay(true);
        let _ = write!(s, "AUTH {}\n", key);
        let _ = std::io::Write::write_all(&mut s, cmd);
        let _ = s.flush();
    }
    Ok(())
}

/// Send an authenticated command and get response.
///
/// Validates the key, caps the response at `MAX_AUTHED_RESPONSE_BYTES`,
/// and returns whatever the server sent after the AUTH ack. The `OK\n`
/// ack is **not** stripped here for backward compatibility with existing
/// callers; new code should prefer `fetch_authed_response` /
/// `fetch_authed_response_multi`.
pub fn send_auth_cmd_response(addr: &str, key: &str, cmd: &[u8]) -> io::Result<String> {
    query_authed_all(addr, key, cmd, Duration::from_millis(500), Duration::from_millis(2000))
}

/// A bounded protocol reader with a total wall-time budget, not merely an
/// idle timeout. A peer sending one byte per timeout cannot keep the CLI hung.
struct AuthedReader {
    reader: std::io::BufReader<std::net::TcpStream>,
    remaining: usize,
    deadline: std::time::Instant,
}

impl AuthedReader {
    fn arm_timeout(&self) -> io::Result<()> {
        let remaining = self.deadline.checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(ErrorKind::TimedOut, "server response deadline exceeded"))?;
        self.reader.get_ref().set_read_timeout(Some(remaining))
    }

    fn read_line(&mut self) -> io::Result<String> {
        use std::io::BufRead;
        let mut line = Vec::new();
        loop {
            self.arm_timeout()?;
            let buf = self.reader.fill_buf()?;
            if buf.is_empty() {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "server closed before a complete response line"));
            }
            let n = buf.iter().position(|b| *b == b'\n').map_or(buf.len(), |p| p + 1);
            if n > self.remaining {
                return Err(io::Error::new(ErrorKind::InvalidData, "server response exceeds size limit"));
            }
            let complete = buf[n - 1] == b'\n';
            line.extend_from_slice(&buf[..n]);
            self.reader.consume(n);
            self.remaining -= n;
            if complete {
                line.pop();
                if line.last() == Some(&b'\r') { line.pop(); }
                return String::from_utf8(line).map_err(|_| io::Error::new(ErrorKind::InvalidData, "server response is not UTF-8"));
            }
        }
    }

    fn read_all(&mut self) -> io::Result<String> {
        use std::io::BufRead;
        let mut output = Vec::new();
        loop {
            self.arm_timeout()?;
            let buf = self.reader.fill_buf()?;
            if buf.is_empty() { break; }
            if buf.len() > self.remaining {
                return Err(io::Error::new(ErrorKind::InvalidData, "server response exceeds size limit"));
            }
            let n = buf.len();
            output.extend_from_slice(buf);
            self.reader.consume(n);
            self.remaining -= n;
        }
        String::from_utf8(output).map_err(|_| io::Error::new(ErrorKind::InvalidData, "server response is not UTF-8"))
    }
}

fn protocol_error(line: &str) -> Option<io::Error> {
    if line.starts_with("ERROR:") || line == "ERR" || line.starts_with("ERR ") {
        let kind = if line.contains("Authentication required") || line.contains("Invalid session key") {
            ErrorKind::PermissionDenied
        } else { ErrorKind::Other };
        Some(io::Error::new(kind, line.to_string()))
    } else { None }
}

fn open_authed(
    addr: &str,
    key: &str,
    cmd: &[u8],
    connect_timeout: Duration,
    read_timeout: Duration,
) -> io::Result<AuthedReader> {
    let key = validate_auth_key(key)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "missing or invalid session key"))?;
    let sock_addr: std::net::SocketAddr = addr.parse()
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid server address"))?;
    let mut s = std::net::TcpStream::connect_timeout(&sock_addr, connect_timeout)?;
    s.set_read_timeout(Some(read_timeout))?;
    s.set_write_timeout(Some(read_timeout))?;
    let _ = s.set_nodelay(true);
    write!(s, "AUTH {}\n", key)?;
    s.write_all(cmd)?;
    if !cmd.ends_with(b"\n") { s.write_all(b"\n")?; }
    s.flush()?;
    // Commands are complete; EOF makes multi-line response completion explicit.
    // A fast peer can finish and close before this half-close reaches Winsock.
    // The response parser still must prove completion; do not discard buffered
    // valid replies merely because shutdown reports an already closed socket.
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut response = AuthedReader {
        reader: std::io::BufReader::new(s),
        remaining: MAX_AUTHED_RESPONSE_BYTES as usize,
        deadline: std::time::Instant::now() + read_timeout,
    };
    let ack = response.read_line()?;
    if let Some(error) = protocol_error(&ack) { return Err(error); }
    if ack != "OK" {
        return Err(io::Error::new(ErrorKind::InvalidData, "invalid authentication acknowledgement"));
    }
    Ok(response)
}

/// Require exact AUTH acknowledgement and a complete newline-framed payload.
/// A valid empty line is a result, whereas EOF, timeout and truncation are errors.
pub fn query_authed_line(
    addr: &str, key: &str, cmd: &[u8], connect_timeout: Duration, read_timeout: Duration,
) -> io::Result<String> {
    let mut response = open_authed(addr, key, cmd, connect_timeout, read_timeout)?;
    let line = response.read_line()?;
    if let Some(error) = protocol_error(&line) { return Err(error); }
    Ok(line)
}

/// Require exact AUTH acknowledgement and EOF for a complete bounded body.
pub fn query_authed_all(
    addr: &str, key: &str, cmd: &[u8], connect_timeout: Duration, read_timeout: Duration,
) -> io::Result<String> {
    let mut response = open_authed(addr, key, cmd, connect_timeout, read_timeout)?;
    let body = response.read_all()?;
    if body.is_empty() {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "server closed without a command response"));
    }
    if let Some(error) = protocol_error(body.trim_end()) { return Err(error); }
    Ok(body)
}

/// Compatibility wrappers for picker callers; transport errors remain unknown.
pub fn fetch_authed_response(
    addr: &str, key: &str, cmd: &[u8], connect_timeout: Duration, read_timeout: Duration,
) -> Option<String> {
    query_authed_line(addr, key, cmd, connect_timeout, read_timeout).ok()
        .filter(|s| !s.is_empty())
}

pub fn fetch_authed_response_multi(
    addr: &str, key: &str, cmd: &[u8], connect_timeout: Duration, read_timeout: Duration,
) -> Option<String> {
    query_authed_all(addr, key, cmd, connect_timeout, read_timeout).ok()
        .map(|s| s.trim_end().to_string()).filter(|s| !s.is_empty())
}

/// Fetch a one-line `session-info` response from a session server.
///
/// Thin wrapper over `fetch_authed_response` retained for the call site
/// in `client.rs` (and the regression tests added in PR #251 for #250).
pub fn fetch_session_info(
    addr: &str,
    key: &str,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Option<String> {
    fetch_authed_response(addr, key, b"session-info\n", connect_timeout, read_timeout)
}

/// Fan out `fetch_session_info` across many sessions in parallel.
///
/// The session picker used to call `fetch_session_info` sequentially, so
/// opening the picker with N sessions was bounded by `N * read_timeout`
/// in the worst case. With this helper, N concurrent threads share that
/// bound: total wall time is roughly `read_timeout`, regardless of N.
///
/// `inputs` is `(label, addr, key)`. Output preserves input order and
/// pairs each label with the fetched info or the supplied `fallback`
/// (typically `"<label>: (not responding)"`).
///
/// Retained for the #250 regression suite; the picker now uses
/// `classify_sessions_parallel`, which both lists and prunes in one pass.
#[allow(dead_code)]
pub fn fetch_session_infos_parallel<F>(
    inputs: Vec<(String, String, String)>, connect_timeout: Duration, read_timeout: Duration, fallback: F,
) -> Vec<(String, String)>
where F: Fn(&str) -> String + Send + Sync,
{
    classify_sessions_parallel(inputs, connect_timeout, read_timeout).into_iter()
        .map(|(label, state)| {
            let info = match state { SessionLiveness::Alive(info) => info, _ => fallback(&label) };
            (label, info)
        }).collect()
}

/// Liveness verdict for one session, produced by a single bounded probe.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionLiveness {
    /// Server authenticated and returned a complete nonempty session-info line.
    Alive(String),
    /// A recorded process generation is positively known to have exited.
    Dead,
    /// Communication or identity could not be verified. Never authorizes cleanup.
    Unresponsive,
    /// Credentials are unavailable. Retained for callers distinguishing this case.
    Unreachable,
}

/// Network failure never proves a process died: a live server can be busy,
/// lose its listener, or have inconsistent registry data. Discovery is read-only.
fn probe_session_liveness(
    addr: &str, key: &str, connect_timeout: Duration, read_timeout: Duration,
) -> SessionLiveness {
    if validate_auth_key(key).is_none() { return SessionLiveness::Unreachable; }
    match query_authed_line(addr, key, b"session-info\n", connect_timeout, read_timeout) {
        Ok(info) if !info.is_empty() => SessionLiveness::Alive(info),
        _ => SessionLiveness::Unresponsive,
    }
}

/// Classify many sessions in parallel with a single bounded probe each.
///
/// Like `fetch_session_infos_parallel`, total wall time is ~one probe window
/// regardless of N (each session runs on its own thread). Returns the liveness
/// verdict per input label, preserving order, so the caller can reap the dead
/// ones and render the rest. This is what keeps the session picker responsive:
/// it replaces a sequential cleanup pass (O(N * timeout)) with one parallel
/// round-trip that both lists and prunes.
pub fn classify_sessions_parallel(
    inputs: Vec<(String, String, String)>, connect_timeout: Duration, read_timeout: Duration,
) -> Vec<(String, SessionLiveness)> {
    // Bound thread/resource use for a registry with many entries. Failed worker
    // creation cannot omit a row or turn unknown liveness into a death verdict.
    const MAX_PROBE_WORKERS: usize = 16;
    let mut results = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(MAX_PROBE_WORKERS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk.iter().map(|(label, addr, key)| {
                let worker = std::thread::Builder::new().spawn_scoped(scope, move || {
                    probe_session_liveness(addr, key, connect_timeout, read_timeout)
                });
                (label, worker)
            }).collect();
            for (label, worker) in handles {
                let state = worker.ok().and_then(|worker| worker.join().ok()).unwrap_or(SessionLiveness::Unresponsive);
                results.push((label.clone(), state));
            }
        });
    }
    results
}

/// PID-anchor liveness for the session registered under `base`, for
/// enumeration paths (e.g. CLI `list-sessions`) that would otherwise pay a
/// TCP connect timeout per dead entry. Some(false) = definitively dead
/// (reap + skip), Some(true) = live, None = no anchor (probe as usual).
pub fn registry_pid_anchor_alive(base: &str) -> Option<bool> {
    match crate::registry::read_manifest(base) {
        Ok(Some(manifest)) => {
            use crate::platform::process_kill::{process_identity, ProcessIdentity};
            match process_identity(manifest.pid) {
                ProcessIdentity::Alive(actual) => Some(actual == manifest.creation_time),
                ProcessIdentity::Exited => Some(false),
                ProcessIdentity::Unknown => None,
            }
        }
        Ok(None) => {
            let port_path = crate::paths::port_file_opt(base)?;
            pid_anchor_verdict(Path::new(&port_path))
        }
        Err(_) => None,
    }
}

/// Reap a single session's registry files (`.port`/`.key`/`.sid`) by base name.
///
/// Used when a probe proves the session is dead. Safe against a live server:
/// it re-creates these files on its next 5s registry tick.
pub fn remove_session_registry(base: &str) {
    let Some(port_path) = crate::paths::port_file_opt(base) else {
        return;
    };
    remove_session_registry_files(Path::new(&port_path));
}

fn control_target() -> io::Result<String> {
    let target = env::var("PSMUX_TARGET_SESSION").unwrap_or_else(|_| "default".to_string());
    if is_warm_session(&target) {
        resolve_last_session_name_ns(session_namespace(&target).as_deref())
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no user session running"))
    } else { Ok(target) }
}

fn targeted_command(line: &str) -> io::Result<Vec<u8>> {
    let mut command = String::new();
    if let Ok(full) = env::var("PSMUX_TARGET_FULL") {
        if full.bytes().any(|b| b == b'\n' || b == b'\r' || b == 0) {
            return Err(io::Error::new(ErrorKind::InvalidInput, "invalid target"));
        }
        command.push_str(&format!("TARGET {}\n", full));
    }
    command.push_str(line);
    if !command.ends_with('\n') { command.push('\n'); }
    Ok(command.into_bytes())
}

pub fn send_control(line: String) -> io::Result<()> {
    let target = control_target()?;
    let (port, key) = read_session_endpoint(&target)?;
    let mut command = targeted_command(&line)?;
    command.extend_from_slice(b"session-info\n");
    let mut response = open_authed(&format!("127.0.0.1:{}", port), &key, &command,
        Duration::from_millis(1000), Duration::from_millis(3000))?;
    let body = response.read_all()?;
    for line in body.lines() {
        if let Some(error) = protocol_error(line) { return Err(error); }
    }
    // A complete session-info reply proves the FIFO event loop passed the
    // command. EOF after AUTH alone must never report a crashed command as OK.
    if !body.lines().any(|line| line.contains(" windows (created ")) {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "server closed before command completion was confirmed"));
    }
    Ok(())
}

pub fn send_control_with_response(line: String) -> io::Result<String> {
    let target = control_target()?;
    let (port, key) = read_session_endpoint(&target)?;
    let command = targeted_command(&line)?;
    let mut response = open_authed(&format!("127.0.0.1:{}", port), &key, &command,
        Duration::from_millis(1000), Duration::from_millis(3000))?;
    // Pane captures and buffers can be much larger than picker information.
    response.remaining = 16 * 1024 * 1024;
    let body = response.read_all()?;
    let verb = line.split_whitespace().next().unwrap_or("");
    // These commands carry arbitrary user text, including literal ERROR lines.
    let arbitrary_text = matches!(verb, "capture-pane" | "capturep" | "show-buffer" | "showb" | "save-buffer" | "saveb");
    if !arbitrary_text {
        if let Some(error) = protocol_error(body.trim_end()) { return Err(error); }
    }
    if body.is_empty() {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "server closed without a command response"));
    }
    Ok(body)
}

const MAX_INTERNAL_COMMAND_BYTES: usize = 128 * 1024;
const MAX_INTERNAL_COMMANDS: usize = 32;

struct QueuedControl {
    port: u16,
    command: String,
    key: String,
    notify: Option<crate::types::ControlSender>,
}

fn queue_control_request(queue: &std::sync::mpsc::SyncSender<QueuedControl>, port: u16, msg: &str,
    key: &str, notify: Option<crate::types::ControlSender>) -> io::Result<()> {
    if msg.len().saturating_add(key.len()) > MAX_INTERNAL_COMMAND_BYTES {
        return Err(io::Error::new(ErrorKind::InvalidInput, "internal command exceeds 128 KiB limit"));
    }
    validate_auth_key(key).ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "invalid session key"))?;
    queue.try_send(QueuedControl { port, command: msg.to_string(), key: key.to_string(), notify })
        .map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => io::Error::new(ErrorKind::WouldBlock, "internal command queue is full"),
            std::sync::mpsc::TrySendError::Disconnected(_) => io::Error::new(ErrorKind::BrokenPipe, "internal command worker stopped"),
        })
}

/// Enqueue key bindings/hooks without connecting or waiting on the server's own
/// event loop. One bounded worker preserves submission order, including across
/// commands that need an execution acknowledgement before the next is sent.
pub fn enqueue_control(app: &crate::types::AppState, port: u16, msg: &str) -> io::Result<()> {
    static QUEUE: std::sync::OnceLock<Option<std::sync::mpsc::SyncSender<QueuedControl>>> = std::sync::OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<QueuedControl>(MAX_INTERNAL_COMMANDS);
        let worker = std::thread::Builder::new().name("psmux-command-worker".into()).spawn(move || {
            while let Ok(request) = rx.recv() {
                let result = (|| {
                    let mut command = request.command.clone();
                    if !command.ends_with('\n') { command.push('\n'); }
                    command.push_str("session-info\n");
                    let mut response = open_authed(&format!("127.0.0.1:{}", request.port), &request.key,
                        command.as_bytes(), Duration::from_millis(500), Duration::from_millis(3000))?;
                    let body = response.read_all()?;
                    for line in body.lines() {
                        if let Some(error) = protocol_error(line) { return Err(error); }
                    }
                    if !body.lines().any(|line| line.contains(" windows (created ")) {
                        return Err(io::Error::new(ErrorKind::UnexpectedEof, "internal command completion not confirmed"));
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let message = format!("command failed: {}", error);
                    crate::debug_log::session_log("command", &message);
                    if let Some(notify) = request.notify {
                        // Retry a bounded number of times from the worker, never
                        // from the event loop, so a busy queue can surface the error.
                        for _ in 0..20 {
                            if notify.send(crate::types::CtrlReq::StatusMessage(message.clone())).is_ok() { break; }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            }
        });
        worker.ok().map(|_| tx)
    }).as_ref().ok_or_else(|| io::Error::other("could not start internal command worker"))?;
    queue_control_request(queue, port, msg, &app.session_key, app.control_tx.clone())
}

/// Send a control message to a specific port with authentication
pub fn send_control_to_port(port: u16, msg: &str, session_key: &str) -> io::Result<()> {
    let key = validate_auth_key(session_key).ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "invalid session key"))?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&address, Duration::from_millis(500))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let _ = stream.set_nodelay(true);
    write!(stream, "AUTH {}\n", key)?;
    stream.write_all(msg.as_bytes())?;
    if !msg.ends_with('\n') { stream.write_all(b"\n")?; }
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut response = AuthedReader { reader: std::io::BufReader::new(stream),
        remaining: 4096, deadline: std::time::Instant::now() + Duration::from_millis(500) };
    let ack = response.read_line()?;
    if let Some(error) = protocol_error(&ack) { return Err(error); }
    if ack != "OK" { return Err(io::Error::new(ErrorKind::InvalidData, "invalid authentication acknowledgement")); }
    Ok(())
}

/// Shortest gap between two `.act` writes for the same session on the
/// per-keystroke path.
///
/// tmux restamps `session.activity_time` on every single key (server-client.c
/// `server_client_handle_key`) because the value is a struct in its own address
/// space. psmux's copy is a file, so a burst of typing would otherwise be a
/// burst of writes; a one second floor keeps the ranking accurate to well
/// within any interval a human notices while costing at most one 16 byte write
/// per second per attached client.
const ACTIVITY_STAMP_MIN_GAP: Duration = Duration::from_millis(1000);

/// Last `.act` write this process performed, as (session, when). Only the
/// throttled path consults it; attach and switch always write.
static LAST_ACTIVITY_STAMP: std::sync::Mutex<Option<(String, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// The stamp written into a `.act` file: microseconds since the Unix epoch.
///
/// Microseconds, not milliseconds, because the value is compared against a
/// `.port` file's mtime, which NTFS keeps to 100ns. A millisecond stamp written
/// just after a port file can truncate to BELOW that file's mtime and lose a
/// comparison it should win. Microseconds is also the resolution tmux keeps
/// `activity_time` at (a `struct timeval`).
fn epoch_micros_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Record `session` as active right now: psmux's `session_update_activity`
/// (tmux session.c). Warm (standby) sessions are internal and never ranked, so
/// they are never stamped.
pub fn touch_session_activity(session: &str) {
    let Some(dir) = crate::paths::psmux_dir_opt() else { return };
    touch_session_activity_in(std::path::Path::new(&dir), session);
}

/// Registry-directory-parameterized variant of [`touch_session_activity`],
/// matching the `_in` convention the routing resolvers use: taking the dir
/// explicitly lets the writer be unit-tested without mutating the process-wide
/// `PSMUX_DATA_DIR`, which other tests read without holding the env lock.
pub fn touch_session_activity_in(dir: &std::path::Path, session: &str) {
    if session.is_empty() || is_warm_session(session) {
        return;
    }
    let path = dir.join(format!("{}.act", session));
    if std::fs::write(path, epoch_micros_now().to_string()).is_ok() {
        // Arm the throttle too: the stamp this just wrote IS current, so the
        // first keystroke after an attach has nothing to add.
        if let Ok(mut guard) = LAST_ACTIVITY_STAMP.lock() {
            *guard = Some((session.to_string(), std::time::Instant::now()));
        }
    }
}

/// [`touch_session_activity`] for the per-keystroke path: a no-op unless
/// `ACTIVITY_STAMP_MIN_GAP` has passed since this process last stamped this
/// same session. A different session always writes, so a client that switches
/// sessions stamps the new one immediately.
pub fn touch_session_activity_throttled(session: &str) {
    if session.is_empty() || is_warm_session(session) {
        return;
    }
    if throttled_out(session) {
        return;
    }
    touch_session_activity(session);
}

/// Registry-directory-parameterized [`touch_session_activity_throttled`].
pub fn touch_session_activity_throttled_in(dir: &std::path::Path, session: &str) {
    if session.is_empty() || is_warm_session(session) {
        return;
    }
    if throttled_out(session) {
        return;
    }
    touch_session_activity_in(dir, session);
}

/// True while this process's last stamp for `session` is still inside
/// `ACTIVITY_STAMP_MIN_GAP`.
fn throttled_out(session: &str) -> bool {
    let Ok(guard) = LAST_ACTIVITY_STAMP.lock() else { return true };
    match *guard {
        Some((ref last, when)) => last == session && when.elapsed() < ACTIVITY_STAMP_MIN_GAP,
        None => false,
    }
}

/// When `base` was last active, as ranked by bare CLI routing.
///
/// The `.act` stamp when one exists, else the `.port` file's mtime. The
/// fallback is the creation time of the session, which is exactly what tmux
/// seeds `activity_time` with for a session nobody has attached to yet
/// (session.c `session_create`: `session_update_activity(s, &s->creation_time)`),
/// and it keeps a registry written by an older psmux ranking sensibly.
fn session_activity_in(
    dir: &std::path::Path,
    base: &str,
    port_meta: Option<std::fs::Metadata>,
) -> std::time::SystemTime {
    if let Ok(text) = std::fs::read_to_string(dir.join(format!("{}.act", base))) {
        if let Ok(us) = text.trim().parse::<u64>() {
            return std::time::UNIX_EPOCH + Duration::from_micros(us);
        }
    }
    port_meta
        .and_then(|m| m.modified().ok())
        .unwrap_or(std::time::UNIX_EPOCH)
}

pub fn resolve_last_session_name() -> Option<String> {
    resolve_last_session_name_ns(None)
}

/// Resolve the most recently modified session, optionally filtered by -L namespace.
/// When `ns` is Some("foo"), only sessions with port files named "foo__*" are considered
/// and the returned name includes the prefix (e.g. "foo__dev").
/// When `ns` is None, only non-namespaced sessions (no "__" in name) are considered.
pub fn resolve_last_session_name_ns(ns: Option<&str>) -> Option<String> {
    let psmux_dir = crate::paths::psmux_dir_opt()?;
    resolve_last_session_name_ns_in(std::path::Path::new(&psmux_dir), ns)
}

/// Registry-directory-parameterized variant of [`resolve_last_session_name_ns`]:
/// the most recently ACTIVE real (non-warm) session base in namespace `ns`.
/// Taking the dir explicitly lets routing be unit-tested without mutating
/// `USERPROFILE`/`HOME`.
///
/// tmux parity (issue #603). tmux picks this session in cmd-find.c
/// `cmd_find_best_session`, whose comparator `cmd_find_session_better` gets no
/// `CMD_FIND_PREFER_UNATTACHED` flag for any ordinary command and so collapses
/// to a single `timercmp` on `activity_time`. The winner
/// is simply the session with the newest activity, and activity is restamped on
/// client attach and on every key a real client sends (server-client.c).
///
/// psmux used to answer this with the `last_session` file alone: whatever name
/// was in it won outright as long as its `.port` still existed. That file is
/// written once per attach and never again, so a session attached long ago and
/// since detached kept beating the session the user is actually sitting in. It
/// is now only a tie-break for candidates whose stamps are identical, which in
/// practice means a registry an older psmux wrote and nobody has attached to
/// since.
pub fn resolve_last_session_name_ns_in(dir: &std::path::Path, ns: Option<&str>) -> Option<String> {
    let hint = std::fs::read_to_string(dir.join("last_session"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // (activity, is_last_session_hint, base): ranked in that order, newest and
    // then hinted first, with the name as a final deterministic tie-break.
    let mut best: Option<(std::time::SystemTime, bool, String)> = None;
    let Ok(rd) = std::fs::read_dir(dir) else { return None };
    for e in rd.flatten() {
        let Some(fname) = e.file_name().to_str().map(|s| s.to_string()) else { continue };
        let Some((base, ext)) = fname.rsplit_once('.') else { continue };
        if ext != "port" || is_warm_session(base) {
            continue;
        }
        if !session_visible_from(base, ns) { continue; }
        let activity = session_activity_in(dir, base, e.metadata().ok());
        let hinted = hint.as_deref() == Some(base);
        let better = match best {
            None => true,
            Some((best_act, best_hinted, ref best_base)) => {
                activity > best_act
                    || (activity == best_act
                        && ((hinted && !best_hinted)
                            || (hinted == best_hinted && base < best_base.as_str())))
            }
        };
        if better {
            best = Some((activity, hinted, base.to_string()));
        }
    }
    best.map(|(_, _, base)| base)
}

/// Resolve the routing target session (the port-file base name) for a CLI
/// command that did not name an explicit `-t session`.
///
/// Socket-selection precedence follows tmux: `-L`/`-S` take precedence over
/// `$TMUX`, which tmux consults only when neither is given (tmux.c `main()`:
/// `if (path == NULL && label == NULL)`). psmux has no single socket, so this
/// maps to: adopt the current server named by `$TMUX` only when its session is
/// in the requested `-L` namespace; otherwise resolve within the namespace (the
/// most-recent session, else the namespaced `X__default`).
///
/// `$TMUX` (set inside every psmux pane, formatted `<socketpath>,<port>,<idx>`)
/// identifies the current server via its control `<port>`; `psmux_dir` is the
/// `.psmux` registry directory of `<base>.port` files; `l_socket_name` is the
/// `-L` namespace, if any. Returns the base to route to, or `None`.
pub fn resolve_routing_target(
    l_socket_name: Option<&str>,
    tmux_env: Option<&str>,
    psmux_dir: &std::path::Path,
) -> Option<String> {
    // Adopt the current server (named by `$TMUX`) only when it is in-namespace.
    if let Some(tmux_val) = tmux_env {
        if let Some(base) = session_base_owning_tmux_port(tmux_val, psmux_dir) {
            let in_namespace = match l_socket_name {
                Some(ns) => session_visible_from(&base, Some(ns)),
                None => true,
            };
            if in_namespace {
                return Some(base);
            }
        }
    }
    // Otherwise the most recent real session in the namespace,
    if let Some(name) = resolve_last_session_name_ns_in(psmux_dir, l_socket_name) {
        return Some(name);
    }
    // else a namespaced `X__default` so `-L X` never leaks to the un-namespaced
    // `default` server. With no `-L`, stay unresolved (the caller keeps "default").
    l_socket_name.map(|ns| crate::paths::storage_base(Some(ns), "default"))
}

/// Find the session port-file base whose control port matches the one encoded
/// in a `$TMUX` value (`<socketpath>,<port>,<idx>`). Warm (standby) sessions are
/// internal-only and never adopted.
fn session_base_owning_tmux_port(tmux_val: &str, psmux_dir: &std::path::Path) -> Option<String> {
    let port: u16 = tmux_val.split(',').nth(1)?.trim().parse().ok()?;
    for entry in std::fs::read_dir(psmux_dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "port").unwrap_or(false) {
            if let Ok(port_str) = std::fs::read_to_string(&path) {
                if port_str.trim().parse::<u16>().ok() == Some(port) {
                    let base = path.file_stem().and_then(|s| s.to_str())?;
                    return if is_warm_session(base) { None } else { Some(base.to_string()) };
                }
            }
        }
    }
    None
}

pub fn resolve_default_session_name() -> Option<String> {
    if let Ok(name) = env::var("PSMUX_DEFAULT_SESSION") {
        let p = crate::paths::port_file(&name);
        if std::path::Path::new(&p).exists() { return Some(name); }
    }
    // `.psmuxrc` is a home-relative config file, not part of the data dir, so it
    // stays under home_dir(); `pmuxrc` and the port files live in the data dir.
    let home = crate::paths::home_dir();
    let candidates = [format!("{}\\.psmuxrc", home), format!("{}\\pmuxrc", crate::paths::psmux_dir())];
    for cfg in candidates.iter() {
        if let Ok(text) = std::fs::read_to_string(cfg) {
            let line = text.lines().find(|l| !l.trim().is_empty())?;
            let name = if let Some(rest) = line.strip_prefix("default-session ") { rest.trim().to_string() } else { line.trim().to_string() };
            let p = crate::paths::port_file(&name);
            if std::path::Path::new(&p).exists() { return Some(name); }
        }
    }
    None
}

pub fn reap_children_placeholder() -> io::Result<bool> { Ok(false) }

/// Return the names of all live sessions by scanning .psmux/*.port files.
pub fn list_session_names() -> Vec<String> {
    list_session_names_ns(None)
}

/// Return session names filtered by namespace (same logic as resolve_last_session_name_ns).
pub fn list_session_names_ns(ns: Option<&str>) -> Vec<String> {
    let dir = crate::paths::psmux_dir();
    list_session_names_ns_in(Path::new(&dir), ns)
}

fn list_session_names_ns_in(dir: &Path, ns: Option<&str>) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else { continue; };
            let Some(base) = name.strip_suffix(".port").or_else(|| name.strip_suffix(".registry.json")) else { continue; };
            if !is_warm_session(base) && session_visible_from(base, ns) {
                names.insert(base.to_string());
            }
        }
    }
    names.into_iter().collect()
}

/// A tree entry used by choose-tree: either a session header or a window under a session.
#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub session_name: String,
    pub session_port: u16,
    pub is_session_header: bool,
    pub window_index: Option<usize>,
    pub window_name: String,
    pub window_panes: usize,
    pub window_size: String,
    pub is_current_session: bool,
    pub is_active_window: bool,
}

/// The sessions the tree chooser offers to a client attached as
/// `current_session`: every live registry entry in the same `-L` namespace,
/// warm helpers excluded, sorted by name. Directory parameterised so it can
/// be tested without touching `USERPROFILE`.
pub fn tree_chooser_sessions_in(
    dir: &std::path::Path,
    current_session: &str,
) -> Vec<(String, u16, std::time::SystemTime)> {
    let ns = session_namespace(current_session);
    let mut sessions: Vec<(String, u16, std::time::SystemTime)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "port").unwrap_or(false) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    // Hide warm (standby) sessions from choose-tree
                    if is_warm_session(stem) { continue; }
                    // Another -L namespace is another server: never listed.
                    if !session_visible_from(stem, ns.as_deref()) { continue; }
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
    sessions
}

/// List all running sessions and their windows for choose-tree display.
/// Queries each running server via its TCP port for window list info.
pub fn list_all_sessions_tree(current_session: &str, current_windows: &[(String, usize, String, bool, usize)]) -> Vec<TreeEntry> {
    let Some(psmux_dir) = crate::paths::psmux_dir_opt() else {
        return vec![];
    };
    let sessions = tree_chooser_sessions_in(std::path::Path::new(&psmux_dir), current_session);

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
            for (wname, panes, size, is_active, disp_idx) in current_windows.iter() {
                tree.push(TreeEntry {
                    session_name: name.clone(),
                    session_port: *port,
                    is_session_header: false,
                    window_index: Some(*disp_idx),
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

#[cfg(test)]
#[path = "../tests-rs/test_session.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests-rs/test_issue250_root_cause.rs"]
mod tests_issue250_root_cause;

#[cfg(test)]
#[path = "../tests-rs/test_session_id_alloc_race.rs"]
mod tests_session_id_alloc_race;

#[cfg(test)]
#[path = "../tests-rs/test_issue448_orphan_reaper.rs"]
mod tests_issue448_orphan_reaper;

#[cfg(test)]
#[path = "../tests-rs/test_startup_stale_port_tax.rs"]
mod tests_startup_stale_port_tax;

#[cfg(test)]
#[path = "../tests-rs/test_l_socket_tmux_precedence.rs"]
mod tests_l_socket_tmux_precedence;

#[cfg(test)]
#[path = "../tests-rs/test_issue509_namespace_instance.rs"]
mod tests_issue509_namespace_instance;

#[cfg(test)]
#[path = "../tests-rs/test_issue510_reaper_attribution.rs"]
mod tests_issue510_reaper_attribution;

#[cfg(test)]
#[path = "../tests-rs/test_issue530_registry_pruning.rs"]
mod tests_issue530_registry_pruning;

#[cfg(test)]
#[path = "../tests-rs/test_issue603_bare_routing.rs"]
mod tests_issue603_bare_routing;

#[cfg(test)]
#[path = "../tests-rs/test_picker_namespace_filter.rs"]
mod tests_picker_namespace_filter;

#[cfg(test)]
#[path = "../tests-rs/test_reliability_discovery.rs"]
mod tests_reliability_discovery;
