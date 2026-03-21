# Features

## Highlights

- 🦠 **Made in Rust** : opt-level 3, full LTO, single codegen unit. Maximum performance.
- 🖱️ **Full mouse support** : click panes, drag-resize borders, scroll, click tabs, select text, right-click copy
- 🎨 **tmux theme support** : 16 named colors + 256 indexed + 24-bit true color (`#RRGGBB`), 14 style options
- 📋 **Reads your `.tmux.conf`** : drop-in config compatibility, zero learning curve
- ⚡ **Blazing fast startup** : sub-100ms session creation, near-zero overhead over shell startup
- 🔌 **76 tmux-compatible commands** : `bind-key`, `set-option`, `if-shell`, `run-shell`, hooks, and more
- 🪟 **Windows-native** : ConPTY, Win32 API, works with PowerShell, cmd, bash, WSL, nushell
- 📦 **Single binary, no dependencies** : install via `cargo`, `winget`, `scoop`, or `choco`

## Terminal Multiplexing

- Split panes horizontally (`Prefix + %`) and vertically (`Prefix + "`)
- Multiple windows with clickable status-bar tabs
- Session management: detach (`Prefix + d`) and reattach from anywhere
- 5 layouts: even-horizontal, even-vertical, main-horizontal, main-vertical, tiled

## Full Mouse Support

- **Click** any pane to focus it, input goes to the right shell
- **Drag** pane borders to resize splits interactively
- **Click** status-bar tabs to switch windows
- **Scroll wheel** in any pane, scrolls that pane's output
- **Drag-select** text to copy to clipboard
- **Right-click** to paste or copy selection
- **VT mouse forwarding** : apps like vim, htop, and midnight commander get full mouse events
- **3-layer mouse injection** : VT protocol, VT bridge (for WSL/SSH), and native Win32 MOUSE_EVENT
- **Mouse over SSH** : works from any OS client when server runs Windows 11 build 22523+

## tmux Theme & Style Support

- **14 customizable style options** : status bar, pane borders, messages, copy-mode highlights, popups
- **Full color spectrum** : 16 named colors, 256 indexed (`colour0`–`colour255`), 24-bit true color (`#RRGGBB`)
- **Text attributes** : bold, dim, italic, underline, blink, reverse, strikethrough, and more
- **Status bar** : fully customizable left/right content with format variables
- **Window tab styling** : separate styles for active, inactive, activity, bell, and last-used tabs
- Compatible with existing tmux theme configs

## Copy Mode (Vim Keybindings)

- **53 vi-style key bindings** : motions, selections, search, text objects
- Visual, line, and **rectangle selection** modes (`v`, `V`, `Ctrl+v`)
- `/` and `?` search with `n`/`N` navigation
- `f`/`F`/`t`/`T` character find, `%` bracket matching, `{`/`}` paragraph jump
- Named registers (`"a`–`"z`), count prefixes, word/WORD variants
- Mouse drag-select copies to Windows clipboard on release

See [keybindings.md](keybindings.md) for the full copy mode key reference.

## Format Engine

- **126+ tmux-compatible format variables** across sessions, windows, panes, cursor, client, and server
- Conditionals (`#{?cond,true,false}`), comparisons, boolean logic
- Regex substitution (`#{s/pat/rep/:var}`), string manipulation
- Loop iteration (`#{W:fmt}`, `#{P:fmt}`, `#{S:fmt}`) over windows, panes, sessions
- Truncation, padding, basename, dirname, strftime, shell quoting

## Scripting & Automation

- **76 tmux-compatible commands** : everything you need for automation
- `send-keys`, `capture-pane`, `pipe-pane` for CI/CD and DevOps workflows
- `if-shell` and `run-shell` for conditional config logic
- **15+ event hooks** : `after-new-window`, `after-split-window`, `client-attached`, etc.
- Paste buffers, named registers, `display-message` with format variables

See [scripting.md](scripting.md) for full command reference and examples.

## Multi-Shell Support

- **PowerShell 7** (default), PowerShell 5, cmd.exe
- **Git Bash**, WSL, nushell, and any Windows executable
- Sets `TERM=xterm-256color`, `COLORTERM=truecolor` automatically
- Sets `TMUX` and `TMUX_PANE` env vars for tmux-aware tool compatibility

See [configuration.md](configuration.md) for `default-shell` and other options.
