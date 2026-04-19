# Warm Sessions

psmux uses a background **warm session** (`__warm__`) to make new session creation nearly instant. This page explains how it works and how to interact with it if needed.

## What is a Warm Session?

When you create a session, psmux pre-spawns a hidden standby server called `__warm__`. This server loads your config, initializes a shell, and waits. When you run `psmux new-session` next time, psmux **claims** this warm server (renames it to your requested session name) instead of cold-starting a new process. This skips the entire server startup + config load + shell spawn cycle.

**Result:** New session creation drops from ~400-1000ms (shell startup) to near-instant.

## Why You Don't See It

The `__warm__` session is an internal implementation detail. It is hidden from:

- `psmux ls` / `psmux list-sessions`
- `prefix + s` (choose-session)
- `prefix + w` (choose-tree)
- `prefix + (` / `)` (session navigation)
- The `last_session` tracking file

Users should never need to interact with it directly.

## When It's Not Spawned

The warm server is **not** created when:

- The current session has `destroy-unattached on` — keeping a hidden warm server alive would break the expectation that sessions die when you detach
- The current session **is** the warm session (no recursive warm spawning)
- Warm panes are explicitly disabled (see below)

## Disabling Warm Sessions

If you prefer every session, window, and pane to start with a completely fresh shell invocation (no pre-spawned state), you can disable warm entirely.

### Via config file

Add this to your `.psmux.conf`, `.tmux.conf`, or `~/.config/psmux/psmux.conf`:

```
set -g warm off
```

### Via environment variable

```powershell
$env:PSMUX_NO_WARM = "1"
```

When warm is disabled:
- No `__warm__` background server is spawned
- No warm panes are pre-spawned inside sessions
- Every `new-session`, `new-window`, and `split-window` cold-starts a fresh shell
- Startup latency increases slightly (shell profile load is not parallelized)

You can re-enable warm at runtime with `set -g warm on`.

## Accessing the Warm Session (Advanced)

If you need to inspect or manage the warm session directly (debugging, development):

```powershell
# Check if a warm session is running
Test-Path "$HOME\.psmux\__warm__.port"

# List all sessions including warm (raw port files)
Get-ChildItem "$HOME\.psmux\*.port" | Select-Object Name

# Send a command to the warm server
psmux -t __warm__ list-windows

# Kill just the warm session
psmux -t __warm__ kill-session

# With -L namespace: warm session is stored as "<namespace>____warm__"
Test-Path "$HOME\.psmux\myns____warm__.port"
```

## File Layout

| File | Purpose |
|------|---------|
| `~\.psmux\__warm__.port` | TCP port of the warm server |
| `~\.psmux\__warm__.key` | Auth key for the warm server |
| `~\.psmux\<ns>____warm__.port` | Warm server under `-L <ns>` namespace |
