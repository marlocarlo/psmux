//! `pmux` binary — alias of `psmux`.
//!
//! A thin shim over the shared crate entry point; see [`psmux::run`].

fn main() {
    psmux::run();
}
