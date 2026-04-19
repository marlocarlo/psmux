# tmux Compatibility

psmux is the most tmux-compatible terminal multiplexer on Windows.

## Overview

| Feature | Support |
|---------|---------|
| Commands | **83** tmux commands implemented |
| Format variables | **140+** variables with full modifier support |
| Config file | Reads `~/.tmux.conf` directly |
| Key bindings | `bind-key`/`unbind-key` with key tables, case-sensitive |
| Hooks | 15+ event hooks (`after-new-window`, etc.) with `set-hook`/`show-hooks` |
| Status bar | Full format engine with conditionals, loops, and multi-line support |
| Themes | 14 style options, 24-bit color, text attributes |
| Layouts | 5 layouts (even-h, even-v, main-h, main-v, tiled) |
| Copy mode | 53 vim keybindings, search, registers, rectangle select |
| Targets | `session:window.pane`, `session:window_name`, `%id`, `@id` syntax |
| `if-shell` / `run-shell` | ✅ Conditional config logic |
| Paste buffers | ✅ Full buffer management |
| Control mode | ✅ `-C` / `-CC` programmatic protocol |
| Popups and menus | ✅ `display-popup`, `display-menu` |
| Interactive choosers | ✅ `choose-tree`, `choose-buffer`, `choose-client` |
| Server namespaces | ✅ `-L` for isolated instances |
| Command chaining | ✅ Sequential `;` operator |
| Nesting prevention | ✅ Blocks psmux inside psmux |
| Session environment | ✅ `set-environment` / `show-environment` |

**Your existing `.tmux.conf` works.** psmux reads it automatically. Just install and go.

## Comparison

| | psmux | Windows Terminal tabs | WSL + tmux |
|---|:---:|:---:|:---:|
| Session persist (detach/reattach) | ✅ | ❌ | ⚠️ WSL only |
| Synchronized panes | ✅ | ❌ | ✅ |
| tmux keybindings | ✅ | ❌ | ✅ |
| Reads `.tmux.conf` | ✅ | ❌ | ✅ |
| tmux theme support | ✅ | ❌ | ✅ |
| Native Windows shells | ✅ | ✅ | ❌ |
| Full mouse support | ✅ | ✅ | ⚠️ Partial |
| Zero dependencies | ✅ | ✅ | ❌ (needs WSL) |
| Scriptable (83 commands) | ✅ | ❌ | ✅ |
| Claude Code agent teams | ✅ | ❌ | ✅ |
| CJK/IME text input | ✅ | ✅ | ✅ |
| Warm session pre-spawn | ✅ | N/A | ❌ |

## Supported Commands

For the full list of supported tmux commands and arguments, see [tmux_args_reference.md](tmux_args_reference.md).

## Recent Parity Improvements

This section covers tmux features that were recently brought to full parity.

### Case-sensitive Key Bindings

Key bindings now distinguish between lowercase and uppercase letters exactly like tmux. `bind-key T` binds to `Shift+T`, while `bind-key t` binds to lowercase `t`. This is critical for plugins like PPM (`Prefix+I` to install) and psmux-sensible (`Prefix+R` to reload).

### Ctrl+Space as Prefix

`set -g prefix C-Space` now works correctly. Previously, multi-character key names like `Space` were parsed as single character fallbacks.

### Wrapped Directional Pane Navigation

Directional pane navigation (`select-pane -U/-D/-L/-R`) now wraps at layout edges, matching tmux behavior. Navigating past the rightmost pane wraps to the leftmost, and so on. Wrap is also correctly suppressed while zoomed.

### Prefix Repeat Chaining

After pressing the prefix key, successive keypresses within the `repeat-time` window (default 500ms) each trigger the bound action without needing to re-enter the prefix. This matches tmux's repeat behavior for pane navigation and resize bindings.

### Switch Client

`switch-client` is fully functional with all standard flags (`-t`, `-n`, `-p`, `-l`). Use it to programmatically switch between sessions.

### Window Name Resolution in Targets

Target syntax now resolves window names, not just indices. `send-keys -t mysession:mywindow` correctly finds the window named "mywindow" in session "mysession".

### Manual Rename Flag

`new-window -n NAME` now sets the `manual_rename` flag, preventing `automatic-rename` from overwriting the explicitly specified window name with the foreground process name.

### List Commands from Within Session

Commands like `list-panes`, `list-windows`, `list-clients`, `list-commands`, and `show-hooks` now work when run from within a psmux session (via `Prefix + :`). Output is displayed in a temporary overlay.

### Source File from Within Session

`source-file` works from within a live session via `Prefix + :`. Previously, config changes only took effect after detaching and reattaching or killing the server.

### Display Panes Overlay

`display-panes` (and `Prefix + q`) now shows pane numbers briefly and auto-dismisses after `display-panes-time` (default 1s). Type a number during the overlay to switch to that pane.

### Hook Deduplication

`set-hook -g` now replaces existing hooks on reload instead of stacking duplicates. `set-hook -gu` correctly removes hooks.

### Command Chaining with Semicolons

Multiple commands can be chained with `;` on a single line, matching tmux behavior:

```tmux
bind-key M-s split-window -h \; select-pane -L
```

### Run Shell Output

`run-shell` now displays output in the status bar, matching tmux behavior. Background mode with `-b` runs fire and forget.

### Session Server Persistence

The psmux session server now survives SSH disconnects. On reconnect, sessions are intact and `psmux attach` reattaches normally.

### Bell and Alert Support

BEL characters (`\x07`) from programs are forwarded to your host terminal for audible beep. The `bell-action` option controls when bells are forwarded and when the status bar tab gets a bell flag.

### Pane Border Labels with Truncation

`pane-border-format` labels that exceed the pane width are now truncated with ellipsis instead of overflowing or clipping mid-character.

### Pane Title Management

`select-pane -T ""` correctly clears a pane title. The default pane title is the hostname, matching tmux convention. Programs can update the pane title via OSC 0/2 escape sequences (controlled by the `allow-set-title` option). See [pane-titles.md](pane-titles.md) for details on how this interacts with PowerShell and other shells.

### Multi-line Status Bar

`set -g status 2` enables a multi-line status bar with `status-format[0]` and `status-format[1]` fully rendering style directives like `#[fg=red]`, `#[align=left]`, and `#[fill=blue]`.

### Status Bar Style Directives

The following inline style directives are now rendered correctly in status-format lines:

- `#[list]` for the window list region
- `#[fill=colour]` for background fill
- `#[align=left|centre|right]` for text alignment
- `#[range=...]` for click regions

### Format Variable Expansion in Bindings

The `-F` flag on `bind-key` now properly expands format variables, enabling plugins like smart-splits.nvim to query pane dimensions.

### Set Environment

`set-environment` and `show-environment` are fully functional. Environment variables set with `set-environment -g` are inherited by all new panes at the process level (no shell commands echoed). The `new-session -e VAR=val` flag also sets session environment correctly.

### Unbind All Keys

`unbind-key -a` correctly removes all key bindings across all key tables. You can also target specific tables: `unbind-key -a -T prefix`, `unbind-key -a -T root`, `unbind-key -a -T copy-mode`.

### Client Prefix Format Variable

The `#{client_prefix}` format variable is correctly set when the prefix key is pressed. This enables status bar indicators like:

```tmux
set -g status-right "#{?client_prefix,#[bg=red] PREFIX ,}"
```

### Window Zoomed Flag

The `#{window_zoomed_flag}` format variable is correctly maintained during zoom/unzoom operations.

### Capture Pane

`capture-pane -p` correctly outputs pane content to stdout, enabling scripts and integrations (including Claude Code agent team coordination) to read pane state.

### Split Window Percentage

`split-window -p <percent>` correctly creates splits at the specified percentage instead of defaulting to 50/50.

### Split Window Working Directory

`split-window -c "#{pane_current_path}"` correctly resolves the format variable and opens the new pane in the current pane's working directory.

### UTF-8 and CJK Support

Multi-byte UTF-8 characters (box-drawing, emoji, CJK text) render correctly in panes. Pasting CJK text no longer crashes the session. Japanese and Korean IME input is handled with minimal latency (the paste-detection heuristic was tuned to avoid misidentifying rapid IME bursts).

## Format Variables

psmux supports 140+ format variables with full modifier support, including:

- Session/window/pane variables (`#S`, `#W`, `#P`, `#{pane_current_path}`, etc.)
- Style and color modifiers
- Conditional expressions (`#{?condition,true,false}`)
- Comparison operators (`#{==:a,b}`, `#{!=:a,b}`, `#{<:a,b}`)
- Logical operators (`#{||:a,b}`, `#{&&:a,b}`)
- Regex substitution (`#{s/pat/rep/:var}`)
- String operations: basename (`#{b:}`), dirname (`#{d:}`), lowercase (`#{l:}`), shell quote (`#{q:}`)
- Truncation and padding (`#{=N:var}`, `#{pN:var}`)
- Loop iteration over windows (`#{W:fmt}`), panes (`#{P:fmt}`), and sessions (`#{S:fmt}`)
