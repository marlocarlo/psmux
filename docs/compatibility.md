# tmux Compatibility

psmux is the most tmux-compatible terminal multiplexer on Windows.

## Overview

| Feature | Support |
|---------|---------|
| Commands | **76** tmux commands implemented |
| Format variables | **126+** variables with full modifier support |
| Config file | Reads `~/.tmux.conf` directly |
| Key bindings | `bind-key`/`unbind-key` with key tables |
| Hooks | 15+ event hooks (`after-new-window`, etc.) |
| Status bar | Full format engine with conditionals and loops |
| Themes | 14 style options, 24-bit color, text attributes |
| Layouts | 5 layouts (even-h, even-v, main-h, main-v, tiled) |
| Copy mode | 53 vim keybindings, search, registers |
| Targets | `session:window.pane`, `%id`, `@id` syntax |
| `if-shell` / `run-shell` | ✅ Conditional config logic |
| Paste buffers | ✅ Full buffer management |

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
| Scriptable (76 commands) | ✅ | ❌ | ✅ |

## Supported Commands

For the full list of supported tmux commands and arguments, see [tmux_args_reference.md](tmux_args_reference.md).

## Format Variables

psmux supports 126+ format variables with full modifier support, including:

- Session/window/pane variables (`#S`, `#W`, `#P`, `#{pane_current_path}`, etc.)
- Style and color modifiers
- Conditional expressions (`#{?condition,true,false}`)
- String operations and comparisons
