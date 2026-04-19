# Performance

psmux is built for speed. The Rust release binary is compiled with **opt-level 3**, **full LTO**, and **single codegen unit**. Every cycle counts.

| Metric | psmux | Notes |
|--------|-------|-------|
| **Session creation** | **< 100ms** | Time for `new-session -d` to return |
| **New window** | **< 80ms** | Overhead on top of shell startup |
| **New pane (split)** | **< 80ms** | Same as window, cached shell resolution |
| **Startup to prompt** | **~shell launch time** | psmux adds near-zero overhead; bottleneck is your shell |
| **15+ windows** | ✅ Stable | Stress-tested with 15+ rapid windows, 18+ panes, 5 concurrent sessions |
| **Rapid fire creates** | ✅ No hangs | Burst-create windows/panes without delays or orphaned processes |

## How It's Fast

- **Lazy pane resize**: only the active window's panes are resized. Background windows resize on-demand when switched to, avoiding O(n) ConPTY syscalls
- **Cached shell resolution**: `which` PATH lookups are cached with `OnceLock`, not repeated per spawn
- **10ms polling**: client-server discovery uses tight 10ms polling for sub-100ms session attach
- **Early port-file write**: server writes its discovery file *before* spawning the first shell, so the client connects instantly
- **8KB reader buffers**: small buffer size minimizes mutex contention across pane reader threads

> **Note:** The primary startup bottleneck is your shell (PowerShell 7 takes ~400-1000ms to display a prompt). psmux itself adds < 100ms of overhead. For faster shells like `cmd.exe` or `nushell`, total startup is near-instant.
