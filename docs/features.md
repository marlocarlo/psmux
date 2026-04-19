# Features

## Highlights

- 🦠 **Made in Rust** : opt-level 3, full LTO, single codegen unit. Maximum performance.
- 🖱️ **Full mouse support** : click panes, drag-resize borders, scroll, click tabs, select text, right-click copy
- 🎨 **tmux theme support** : 16 named colors + 256 indexed + 24-bit true color (`#RRGGBB`), 14 style options
- 📋 **Reads your `.tmux.conf`** : drop-in config compatibility, zero learning curve
- ⚡ **Blazing fast startup** : sub-100ms session creation, near-zero overhead over shell startup
- 🔌 **83 tmux-compatible commands** : `bind-key`, `set-option`, `if-shell`, `run-shell`, `display-popup`, `display-menu`, hooks, and more
- 🪟 **Windows-native** : ConPTY, Win32 API, works with PowerShell, cmd, bash, WSL, nushell
- 📦 **Single binary, no dependencies** : install via `cargo`, `winget`, `scoop`, or `choco`
- 🤖 **Claude Code agent teams** : first-class support for teammate pane spawning
- 🌐 **CJK/IME input** : full support for Chinese, Japanese, and Korean input methods

## Terminal Multiplexing

- Split panes horizontally (`Prefix + %`) and vertically (`Prefix + "`)
- Multiple windows with clickable status-bar tabs
- Session management: detach (`Prefix + d`) and reattach from anywhere
- 5 layouts: even-horizontal, even-vertical, main-horizontal, main-vertical, tiled

## Full Mouse Support

- **Click** any pane to focus it, input goes to the right shell
- **Drag** pane borders to resize splits interactively
- **Click** status-bar tabs to switch windows
- **Scroll wheel** in any pane, scrolls that pane's output (configurable via `scroll-enter-copy-mode`)
- **Drag-select** text to copy to clipboard
- **Right-click** to paste or copy selection
- **Windows 11 PowerShell selection** : word and line selection with double/triple-click (`pwsh-mouse-selection on`)
- **VT mouse forwarding** : apps like vim, htop, and midnight commander get full mouse events
- **3-layer mouse injection** : VT protocol, VT bridge (for WSL/SSH), and native Win32 MOUSE_EVENT
- **Mouse over SSH** : works from any OS client when server runs Windows 11 build 22523+
- **Disable mouse** : `set -g mouse off` fully suppresses mouse event handling

## tmux Theme & Style Support

- **14 customizable style options** : status bar, pane borders, messages, copy-mode highlights, popups, menus
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

- **140+ tmux-compatible format variables** across sessions, windows, panes, cursor, client, and server
- Conditionals (`#{?cond,true,false}`), comparisons (`#{==:a,b}`, `#{!=:a,b}`), boolean logic (`#{||:}`, `#{&&:}`)
- Regex substitution (`#{s/pat/rep/:var}`), string manipulation
- Loop iteration (`#{W:fmt}`, `#{P:fmt}`, `#{S:fmt}`) over windows, panes, sessions
- Truncation, padding, basename, dirname, strftime, shell quoting
- Inline style directives: `#[list]`, `#[fill]`, `#[align=left|centre|right]`, `#[range=...]`

## Scripting & Automation

- **83 tmux-compatible commands** : everything you need for automation
- `send-keys`, `capture-pane`, `pipe-pane` for CI/CD and DevOps workflows
- `display-popup` for floating popup windows with custom commands
- `display-menu` for interactive context menus
- `choose-tree` for interactive session/window/pane selection
- `choose-buffer` and `choose-client` for interactive buffer and client picking
- `if-shell` and `run-shell` for conditional config logic
- **15+ event hooks** : `after-new-window`, `after-split-window`, `client-attached`, etc.
- Paste buffers, named registers, `display-message` with format variables
- Server namespaces via `-L` for running isolated psmux instances
- Command chaining with `;` for multi-step bindings
- `switch-client` for programmatic session switching
- `break-pane` and `join-pane` for pane reorganization
- `wait-for` with lock/signal/unlock for cross-pane synchronization
- `confirm-before` for user confirmation dialogs

See [scripting.md](scripting.md) for full command reference and examples.

## Session Persistence

- psmux session servers survive SSH disconnects and terminal crashes
- Detach with `Prefix + d`, reconnect with `psmux attach` from any terminal
- Warm sessions (`set -g warm on`, default) pre-spawn background servers for instant session creation
- Use [psmux-resurrect](https://github.com/psmux/psmux-plugins/tree/main/psmux-resurrect) to save/restore sessions across reboots
- Use [psmux-continuum](https://github.com/psmux/psmux-plugins/tree/main/psmux-continuum) for automatic periodic save/restore

## Session Switching

- **Prefix + s** opens an interactive session/window/pane tree chooser
- **Prefix + (** and **Prefix + )** cycle through sessions
- `switch-client -t sessionname` switches to a named session
- `switch-client -l` returns to the last (most recently used) session
- Create multiple sessions with `new-session -s name` and switch freely between them

## Display Panes Overlay

- **Prefix + q** shows numbered overlays on all panes for quick selection
- Press a number key to jump to that pane instantly
- Numbers respect `pane-base-index` (e.g., starts from 1 if configured)
- Overlay auto-dismisses after `display-panes-time` milliseconds (default: 1000ms)
- Only single-digit pane numbers (0 through 9) can be selected by keypress

## Nesting Prevention

- psmux automatically detects when running inside an existing session
- Prevents accidental creation of nested psmux instances
- To create a new session from inside psmux, use the command prompt (`Prefix + :`)

## Dead Pane Handling

- `set -g remain-on-exit on` keeps panes visible after their process exits
- Dead panes display their final output for inspection
- `respawn-pane` restarts the shell or a new command in a dead pane
- Useful for monitoring long-running processes that may crash

## Command Prompt

- **Prefix + :** opens a command prompt at the bottom of the screen
- Full cursor movement (arrow keys, Home, End) within the command line
- Command history (Up/Down arrows recall previous commands)
- Any psmux/tmux command can be typed and executed interactively
- Supports `source-file`, `set-option`, `split-window`, `list-commands`, and all 83 commands

## Claude Code Agent Teams

- First-class support for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) teammate pane spawning
- Automatically sets `TMUX`, `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`, and teammate mode
- Each agent gets its own visible pane with full terminal output
- No extra configuration needed: start psmux, run `claude`, and ask it to create a team

See [claude-code.md](claude-code.md) for detailed setup and troubleshooting.

## CJK and IME Input

- Full support for Chinese, Japanese, and Korean character input
- IME composition handled with minimal latency (paste-detection heuristic tuned for rapid IME bursts)
- Korean IME input correctly handled without bracketed paste sequence injection
- CJK text pasting works reliably for any length
- UTF-8 multi-byte characters (box-drawing, emoji, CJK) render correctly in ConPTY panes

## Interactive Choosers

- `choose-tree` (`Prefix + w`): browse and select sessions, windows, and panes interactively
- `choose-buffer` (`Prefix + =`): pick from paste buffers with preview
- `choose-client`: view connected clients
- `customize-mode`: interactive options editor

## Nesting Prevention

psmux prevents launching a psmux session inside an existing psmux session. If you attempt to nest sessions, psmux blocks it to avoid UI confusion. This matches tmux behavior where nesting requires explicitly unsetting `$TMUX`.

## Multi-Shell Support

- **PowerShell 7** (default), PowerShell 5, cmd.exe
- **Git Bash**, WSL, nushell, and any Windows executable
- Sets `TERM=xterm-256color`, `COLORTERM=truecolor` automatically
- Sets `TMUX` and `TMUX_PANE` env vars for tmux-aware tool compatibility

See [configuration.md](configuration.md) for `default-shell` and other options.
