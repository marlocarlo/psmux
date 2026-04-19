use super::*;

/// Append a line to ~/.psmux/autorename.log (first 100 entries only).
pub(crate) fn autorename_log(msg: &str) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    if n > 100 { return; }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let path = format!("{}/.psmux/autorename.log", home);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S%.3f"), msg);
    }
}
