//! The render path must expand `#()` asynchronously.
//!
//! tests-rs/test_issue272_format_shell_cache.rs already proves the async
//! contract of `run_shell_command` itself — but every one of those tests opts
//! in by constructing an `AsyncFormatGuard` by hand. That is precisely the
//! thing that broke: the guard was correct, the contract was correct, and the
//! server's auto-push block simply did not use it. A `#(cmd /c ...)` in
//! status-right therefore ran `Command::output()` synchronously on the single
//! server event loop — the same thread that delivers keystrokes to ConPTY — on
//! every pane output burst, which is up to ~1000/s while a shell is drawing.
//! Measured cost of the real-world offender: 88ms per call. The result was
//! severe, permanent keyboard lag that only appeared inside psmux.
//!
//! So these tests deliberately do NOT construct a guard. They call the render
//! helpers exactly as the server loop calls them and assert the observable
//! consequence: the call returns promptly, and N calls inside one TTL produce
//! one subprocess spawn rather than N.
//!
//! Spawn accounting is the same trick as the #272 suite: the inner command
//! appends a line to a unique file each time it really runs, so line count is
//! ground truth independent of what the expansion returns.

use super::*;
use std::time::{Duration, Instant};

fn counter_path(tag: &str) -> std::path::PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("psmux_renderpath_{}_{}_{}.count", tag, pid, nanos))
}

/// `#()` inner command that takes ~1s and then records that it ran. Slow on
/// purpose: a synchronous spawn is then unmistakable in the wall clock instead
/// of being lost in noise.
fn slow_tracer(counter: &std::path::Path) -> String {
    let p = counter.display().to_string().replace('\\', "/");
    if cfg!(windows) {
        format!("ping -n 2 127.0.0.1 >nul & echo x>>{}", p)
    } else {
        format!("sleep 1; echo x>>{}", p)
    }
}

fn fast_tracer(counter: &std::path::Path) -> String {
    let p = counter.display().to_string().replace('\\', "/");
    format!("echo x>>{}", p)
}

fn line_count(p: &std::path::Path) -> usize {
    std::fs::read_to_string(p).map(|s| s.lines().count()).unwrap_or(0)
}

fn cleanup(p: &std::path::Path) {
    let _ = std::fs::remove_file(p);
}

fn wait_for_spawns(counter: &std::path::Path, expected: usize, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let c = line_count(counter);
        if c >= expected || Instant::now() >= deadline {
            return c;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn mock_app(interval_secs: u64) -> AppState {
    let mut app = AppState::new("renderpath".to_string());
    app.window_base_index = 0;
    app.status_interval = interval_secs;
    let (tx, rx) = std::sync::mpsc::channel();
    app.format_job_tx = Some(tx);
    app.format_job_rx = Some(rx);
    app
}

/// A synchronous spawn of the ~1s helper would park the event loop for ~1s.
/// Anything under this is comfortably "did not block".
const NONBLOCKING: Duration = Duration::from_millis(300);

// ───────────────── expand_status_formats: the main render call ─────────────────

#[test]
fn status_right_hash_paren_does_not_block_the_render_path() {
    let counter = counter_path("sr_block");
    cleanup(&counter);
    let mut app = mock_app(15);
    app.status_right = format!("#({})", slow_tracer(&counter));

    // No AsyncFormatGuard here on purpose — expand_status_formats owns it.
    let t0 = Instant::now();
    let out = expand_status_formats(&app, "");
    let elapsed = t0.elapsed();

    assert!(
        elapsed < NONBLOCKING,
        "expand_status_formats blocked {:?} on a ~1s status-right helper — the \
         render path is spawning synchronously on the server event loop again, \
         which is the keyboard-lag bug",
        elapsed
    );
    assert_eq!(
        out.status_right, "",
        "first render shows empty #() until the worker completes; got {:?}",
        out.status_right
    );

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    cleanup(&counter);
}

#[test]
fn repeated_renders_spawn_once_per_ttl() {
    let counter = counter_path("sr_once");
    cleanup(&counter);
    let mut app = mock_app(60);
    app.status_right = format!("#({})", fast_tracer(&counter));

    // 50 pushes inside one TTL. Synchronously that is 50 processes.
    for _ in 0..50 {
        let _ = expand_status_formats(&app, "");
    }

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(200));
    let spawns = line_count(&counter);
    cleanup(&counter);

    assert_eq!(
        spawns, 1,
        "50 renders inside one TTL must share one spawn; got {} — the TTL cache \
         is being bypassed, which means the render path lost its async guard",
        spawns
    );
}

#[test]
fn status_left_is_guarded_too() {
    let counter = counter_path("sl_block");
    cleanup(&counter);
    let mut app = mock_app(15);
    app.status_left = format!("#({})", slow_tracer(&counter));

    let t0 = Instant::now();
    let _ = expand_status_formats(&app, "");
    assert!(t0.elapsed() < NONBLOCKING, "status-left blocked the render path");

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    cleanup(&counter);
}

/// `set-titles-string` was expanded OUTSIDE the guard on both render paths, so
/// it blocked even where the rest of the bar did not. It is now part of
/// `expand_status_formats`.
#[test]
fn set_titles_string_is_guarded() {
    let counter = counter_path("title_block");
    cleanup(&counter);
    let mut app = mock_app(15);
    app.set_titles = true;
    app.set_titles_string = format!("#({})", slow_tracer(&counter));

    let t0 = Instant::now();
    let out = expand_status_formats(&app, "");
    let elapsed = t0.elapsed();

    assert!(
        elapsed < NONBLOCKING,
        "set-titles-string blocked the render path for {:?}",
        elapsed
    );
    assert!(out.host_title.is_some(), "set-titles on should produce a host_title");

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    cleanup(&counter);
}

#[test]
fn host_title_is_none_when_set_titles_is_off() {
    let app = mock_app(15);
    assert!(
        expand_status_formats(&app, "").host_title.is_none(),
        "set-titles off must not emit a host_title"
    );
}

// ───────────────── the other two guarded render helpers ─────────────────

#[test]
fn extra_style_json_does_not_block_the_render_path() {
    let counter = counter_path("extra_block");
    cleanup(&counter);
    let mut app = mock_app(15);
    app.status_left_style = format!("#({})", slow_tracer(&counter));

    let mut buf = String::from("{}");
    let t0 = Instant::now();
    append_extra_style_json(&mut buf, &app);
    let elapsed = t0.elapsed();

    assert!(
        elapsed < NONBLOCKING,
        "append_extra_style_json blocked {:?} — it runs on every frame too",
        elapsed
    );

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    cleanup(&counter);
}

#[test]
fn window_status_format_does_not_block_the_render_path() {
    let counter = counter_path("wsf_block");
    cleanup(&counter);
    let mut app = mock_app(15);
    app.window_status_format = format!("#({})", slow_tracer(&counter));
    app.window_status_current_format = app.window_status_format.clone();

    let t0 = Instant::now();
    let _ = list_windows_json_with_tabs(&app);
    let elapsed = t0.elapsed();

    assert!(
        elapsed < NONBLOCKING,
        "list_windows_json_with_tabs blocked {:?} — and it expands once PER \
         WINDOW, so a synchronous #() here multiplies by the window count",
        elapsed
    );

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    cleanup(&counter);
}

// ───────────────── the other half of the contract ─────────────────

/// The guard must not leak out of the render helpers. One-shot callers
/// (`display-message -p '#(cmd)'`) have no later repaint to pick up an async
/// result, so they must still block and return real output. This is the
/// invariant that d981d94 restored, and it is what makes the guard's placement
/// load-bearing rather than cosmetic.
#[test]
fn expansion_outside_the_render_helpers_is_still_synchronous() {
    let counter = counter_path("sync_after");
    cleanup(&counter);
    let app = mock_app(15);

    // Run a render first, so any leaked thread-local state would be active.
    let _ = expand_status_formats(&app, "");

    let p = counter.display().to_string().replace('\\', "/");
    let cmd = if cfg!(windows) {
        format!("echo ONESHOT & echo x>>{}", p)
    } else {
        format!("echo ONESHOT; echo x>>{}", p)
    };
    let out = crate::format::expand_format(&format!("#({})", cmd), &app);
    cleanup(&counter);

    assert!(
        out.contains("ONESHOT"),
        "a one-shot #() must expand synchronously and return real stdout on the \
         FIRST call; got {:?} (async mode leaked out of the render helpers)",
        out
    );
}

/// Nested guards must not drop the outer region back to synchronous. The guard
/// saves and restores the previous flag value rather than clearing it, so this
/// stays true even if the render helpers are ever composed.
#[test]
fn nested_guards_keep_the_outer_region_async() {
    let counter = counter_path("nested");
    cleanup(&counter);
    let app = mock_app(15);

    let outer = crate::format::AsyncFormatGuard::new();
    {
        // Inner guarded helper; its Drop must not clear the outer guard.
        let _ = expand_status_formats(&app, "");
    }
    let t0 = Instant::now();
    let _ = crate::format::expand_format(&format!("#({})", slow_tracer(&counter)), &app);
    let elapsed = t0.elapsed();
    drop(outer);

    assert!(
        elapsed < NONBLOCKING,
        "after an inner guard was dropped the outer region went synchronous \
         ({:?}) — AsyncFormatGuard::drop is clearing instead of restoring",
        elapsed
    );

    wait_for_spawns(&counter, 1, Duration::from_secs(5));
    cleanup(&counter);
}
