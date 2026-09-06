// Root cause of the flaky `session_id_is_unique` test: `allocate_session_id`
// did a non-atomic read-modify-write on the `.psmux/next_session_id` counter
// file with no synchronization. Two concurrent callers (parallel test threads,
// or two sessions starting at once across processes) both read the same
// `current`, both return it, and both write `current + 1` -> duplicate ids.
//
// The fix serializes the read-modify-write with a process-global mutex plus a
// cross-process advisory lock file. This test hammers `allocate_session_id`
// from many threads at once and asserts every id is distinct, which reliably
// failed before the fix.

use super::*;

#[test]
fn allocate_session_id_is_unique_under_concurrency() {
    let _env_lock = crate::util::lock_test_env();
    let scratch = std::env::temp_dir().join(format!("psmux-counter-test-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&scratch).unwrap();
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) { match &self.0 { Some(value) => std::env::set_var("PSMUX_DATA_DIR", value), None => std::env::remove_var("PSMUX_DATA_DIR") } }
    }
    let _restore = Restore(std::env::var_os("PSMUX_DATA_DIR"));
    std::env::set_var("PSMUX_DATA_DIR", &scratch);
    const THREADS: usize = 16;
    const PER_THREAD: usize = 32;

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
    let mut handles = Vec::new();
    for _ in 0..THREADS {
        let b = barrier.clone();
        handles.push(std::thread::spawn(move || {
            // Release all threads simultaneously to maximize the race window.
            b.wait();
            let mut ids = Vec::with_capacity(PER_THREAD);
            for _ in 0..PER_THREAD {
                ids.push(allocate_session_id().unwrap());
            }
            ids
        }));
    }

    let mut all = Vec::new();
    for h in handles {
        all.extend(h.join().expect("thread panicked"));
    }

    let total = all.len();
    let mut sorted = all.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        total,
        "allocate_session_id handed out duplicate ids under concurrency: {} unique of {} allocated",
        sorted.len(),
        total
    );
    std::fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn counter_failure_does_not_reset_or_wrap_ids() {
    let scratch = std::env::temp_dir().join(format!("psmux-counter-invalid-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("counter");
    assert_eq!(increment_session_counter(&path).unwrap(), 0);
    assert_eq!(increment_session_counter(&path).unwrap(), 1);
    for value in ["broken".to_string(), usize::MAX.to_string()] {
        std::fs::write(&path, &value).unwrap();
        assert!(increment_session_counter(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), value);
    }
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn counter_drop_does_not_delete_replacement_lock() {
    let scratch = std::env::temp_dir().join(format!("psmux-counter-lock-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("counter.lock");
    let guard = CounterLock::acquire(path.to_string_lossy().into_owned()).unwrap();
    std::fs::write(&path, b"replacement").unwrap();
    drop(guard);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
    std::fs::remove_dir_all(scratch).unwrap();
}
