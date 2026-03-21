# portable-pty-patched

Patched version of [portable-pty](https://crates.io/crates/portable-pty) v0.9.0 (originally from [wez/wezterm](https://github.com/wez/wezterm)) with ConPTY flag support required by [psmux](https://github.com/psmux/psmux).

## Why this exists

`portable-pty` is not published as a standalone repo — it lives inside the wezterm monorepo, making a proper GitHub fork impractical (we'd be forking an entire terminal emulator project just for one file change).

The upstream crate does not pass modern ConPTY creation flags that psmux needs for correct terminal behavior on Windows 10/11.

## Patches (`src/win/psuedocon.rs`)

### New ConPTY flags
- **`PSEUDOCONSOLE_RESIZE_QUIRK`** (0x2) — fixes resize artifacts
- **`PSEUDOCONSOLE_WIN32_INPUT_MODE`** (0x4) — enables Win32 input mode for proper key handling
- **`PSEUDOCONSOLE_PASSTHROUGH_MODE`** (0x8) — relays VT sequences directly from child processes (Windows 11 22H2+ only), enabling cursor shape forwarding, DECSCUSR, etc.

### Build detection (`supports_passthrough_mode()`)
Uses `RtlGetVersion` to detect Windows build >= 22621 (Windows 11 22H2). On older builds, passthrough mode is skipped to avoid broken ConPTY output.

### Two-tier `PsuedoCon::new()`
1. Attempts `CreatePseudoConsole` with all flags including `PASSTHROUGH_MODE` on supported builds
2. Falls back to base flags (without passthrough) if the call fails or on older Windows

### Cargo.toml
Added `libloaderapi` and `winnt` features to `winapi` dependency for `GetModuleHandleW`/`GetProcAddress`/`RtlGetVersion`.

## Usage

In your `Cargo.toml`:
```toml
portable-pty = { git = "https://github.com/psmux/portable-pty-patched.git", branch = "main" }
```

## Keeping up to date

This is **not** a GitHub fork (upstream lives inside wezterm monorepo). To sync with a new upstream release:
1. Download the new version from [crates.io](https://crates.io/crates/portable-pty)
2. Re-apply the patches to `src/win/psuedocon.rs` and `Cargo.toml`
