//! `psmux` binary — thin shim over the shared crate entry point.
//!
//! All program logic lives in the `psmux` library (`src/lib.rs`); this binary,
//! like its `pmux` / `tmux` aliases, simply forwards to [`psmux::run`].

fn main() {
    psmux::run();
}
