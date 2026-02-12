```
╔═══════════════════════════════════════════════════════════╗
║   ██████╗ ███████╗███╗   ███╗██╗   ██╗██╗  ██╗            ║
║   ██╔══██╗██╔════╝████╗ ████║██║   ██║╚██╗██╔╝            ║
║   ██████╔╝███████╗██╔████╔██║██║   ██║ ╚███╔╝             ║
║   ██╔═══╝ ╚════██║██║╚██╔╝██║██║   ██║ ██╔██╗             ║
║   ██║     ███████║██║ ╚═╝ ██║╚██████╔╝██╔╝ ██╗            ║
║   ╚═╝     ╚══════╝╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝            ║
║          Terminal Multiplexer for Windows                 ║
╚═══════════════════════════════════════════════════════════╝
```

# psmux

**A terminal multiplexer for Windows** — the tmux alternative you've been waiting for.

psmux brings tmux-style terminal multiplexing to Windows natively. No WSL, no Cygwin, no compromises. Built in Rust for Windows Terminal, PowerShell, and cmd.exe.

> 💡 **Tip:** psmux includes `tmux` and `pmux` aliases, so you can use your muscle memory!

## Why psmux?

If you've used tmux on Linux/macOS and wished you had something similar on Windows — this is it.

- **Windows-native** — Built specifically for Windows 10/11
- **Works everywhere** — Windows Terminal, PowerShell, cmd.exe, ConEmu, etc.
- **No dependencies** — Single binary, just works
- **tmux-compatible** — Same commands, same keybindings, zero learning curve
- **Aliases included** — Use `psmux`, `pmux`, or `tmux` command, your choice

![psmux in action - monitoring system info](psmux_sysinfo.gif)

## Features

- Split panes horizontally and vertically
- Multiple windows with tabs
- Session management (attach/detach)
- Mouse support for resizing panes and clicking tabs
- Copy mode with vim-like keybindings
- **Scrollback history** (configurable, default 2000 lines)
- Synchronized input to multiple panes
- **Automatic window rename** from foreground process (like tmux)
- **Status bar** with full tmux format variable support
- **Theming** — full style customization (fg, bg, bold, dim, italics, etc.)
- **Hooks** — run commands on events (after-new-window, etc.)
- **Monitor activity/silence** — flag windows with new output
- **Layouts** — even-horizontal, even-vertical, main-horizontal, main-vertical, tiled
- **Format engine** — 100+ tmux-compatible variables and modifiers
- **Config file** — drop-in compatible with `.tmux.conf`

![psmux windows and panes](psmux_windows.gif)

## Requirements

- Windows 10 or Windows 11
- **PowerShell 7+** (recommended) or cmd.exe
  - Download PowerShell: `winget install --id Microsoft.PowerShell`
  - Or visit: https://aka.ms/powershell

## Installation

### Quick Install (Recommended)

```powershell
irm https://raw.githubusercontent.com/marlocarlo/psmux/master/scripts/install.ps1 | iex
```

This downloads the latest release and adds it to your PATH. After installation, `psmux`, `pmux`, and `tmux` commands are all available.

To uninstall:
```powershell
irm https://raw.githubusercontent.com/marlocarlo/psmux/master/scripts/uninstall.ps1 | iex
```

### Using Scoop

```powershell
scoop install https://raw.githubusercontent.com/marlocarlo/psmux/master/scoop/psmux.json
```

### Using Chocolatey

```powershell
choco install psmux
```

### Using Cargo

```powershell
cargo install psmux
```

### From GitHub Releases

Download the latest `.zip` from [GitHub Releases](https://github.com/marlocarlo/psmux/releases) and add to your PATH.

### From Source

```powershell
git clone https://github.com/marlocarlo/psmux.git
cd psmux
cargo build --release
```

Built binaries:

```text
target\release\psmux.exe
target\release\pmux.exe
target\release\tmux.exe
```

Optional (install from local source into Cargo bin path):

```powershell
cargo install --path .
```

## Usage

Use `psmux`, `pmux`, or `tmux` — they're identical:

```powershell
# Start a new session
psmux
pmux
tmux

# Start a named session
psmux new-session -s work
tmux new-session -s work

# List sessions
psmux ls
tmux ls

# Attach to a session
psmux attach -t work
tmux attach -t work

# Show help
psmux --help
tmux --help
```

## Key Bindings

Default prefix: `Ctrl+b` (same as tmux)

| Key | Action |
|-----|--------|
| `Prefix + c` | Create new window |
| `Prefix + %` | Split pane left/right |
| `Prefix + "` | Split pane top/bottom |
| `Prefix + x` | Kill current pane |
| `Prefix + &` | Kill current window |
| `Prefix + z` | Toggle pane zoom |
| `Prefix + n` | Next window |
| `Prefix + p` | Previous window |
| `Prefix + 0-9` | Select window by number |
| `Prefix + d` | Detach from session |
| `Prefix + ,` | Rename current window |
| `Prefix + t` | Rename active pane title |
| `Prefix + s` | Session chooser/switcher |
| `Prefix + o` | Select next pane |
| `Prefix + w` | Window/pane chooser |
| `Prefix + [` | Enter copy/scroll mode |
| `Prefix + {` | Enter copy/scroll mode (alternate) |
| `Prefix + ]` | Paste from buffer |
| `Prefix + q` | Display pane numbers |
| `Prefix + Arrow` | Navigate between panes |
| `Ctrl+q` | Quit |

### Copy/Scroll Mode

Enter copy mode with `Prefix + [` or `Prefix + {` to scroll through terminal history:

| Key | Action |
|-----|--------|
| `↑` / `k` | Scroll up 1 line |
| `↓` / `j` | Scroll down 1 line |
| `PageUp` / `b` | Scroll up 10 lines |
| `PageDown` / `f` | Scroll down 10 lines |
| `g` | Jump to top of scrollback |
| `G` | Jump to bottom |
| `←` / `h` | Move cursor left |
| `→` / `l` | Move cursor right |
| `v` | Start selection |
| `y` | Yank (copy) selection |
| `Mouse drag + release` | Select text and copy to clipboard |
| `Esc` / `q` | Exit copy mode |

When in copy mode:
- The pane border turns **yellow** 
- `[copy mode]` appears in the title
- A scroll position indicator shows in the top-right corner
- Mouse selection in copy mode is copied to the Windows clipboard on release

## Scripting & Automation

psmux supports tmux-compatible commands for scripting and automation:

### Window & Pane Control

```powershell
# Create a new window
psmux new-window

# Split panes
psmux split-window -v          # Split vertically (top/bottom)
psmux split-window -h          # Split horizontally (side by side)

# Navigate panes
psmux select-pane -U           # Select pane above
psmux select-pane -D           # Select pane below
psmux select-pane -L           # Select pane to the left
psmux select-pane -R           # Select pane to the right

# Navigate windows
psmux select-window -t 1       # Select window by index (default base-index is 1)
psmux next-window              # Go to next window
psmux previous-window          # Go to previous window
psmux last-window              # Go to last active window

# Kill panes and windows
psmux kill-pane
psmux kill-window
psmux kill-session
```

### Sending Keys

```powershell
# Send text directly
psmux send-keys "ls -la" Enter

# Send keys literally (no parsing)
psmux send-keys -l "literal text"

# Special keys supported:
# Enter, Tab, Escape, Space, Backspace
# Up, Down, Left, Right, Home, End
# PageUp, PageDown, Delete, Insert
# F1-F12, C-a through C-z (Ctrl+key)
```

### Pane Information

```powershell
# List all panes in current window
psmux list-panes

# List all windows
psmux list-windows

# Capture pane content
psmux capture-pane

# Display formatted message with variables
psmux display-message "#S:#I:#W"   # Session:Window Index:Window Name
```

### Paste Buffers

```powershell
# Set paste buffer content
psmux set-buffer "text to paste"

# Paste buffer to active pane
psmux paste-buffer

# List all buffers
psmux list-buffers

# Show buffer content
psmux show-buffer

# Delete buffer
psmux delete-buffer
```

### Pane Layout

```powershell
# Resize panes
psmux resize-pane -U 5         # Resize up by 5
psmux resize-pane -D 5         # Resize down by 5
psmux resize-pane -L 10        # Resize left by 10
psmux resize-pane -R 10        # Resize right by 10

# Swap panes
psmux swap-pane -U             # Swap with pane above
psmux swap-pane -D             # Swap with pane below

# Rotate panes in window
psmux rotate-window

# Toggle pane zoom
psmux zoom-pane
```

### Session Management

```powershell
# Check if session exists (exit code 0 = exists)
psmux has-session -t mysession

# Rename session
psmux rename-session newname

# Respawn pane (restart shell)
psmux respawn-pane
```

### Format Variables

The `display-message` command supports these variables:

| Variable | Description |
|----------|-------------|
| `#S` | Session name |
| `#I` | Window index |
| `#W` | Window name |
| `#P` | Pane ID |
| `#T` | Pane title |
| `#H` | Hostname |

### Advanced Commands

```powershell
# Discover supported commands
psmux list-commands

# Server/session management
psmux kill-server
psmux list-clients
psmux switch-client -t other-session

# Config at runtime
psmux source-file ~/.psmux.conf
psmux show-options
psmux set-option -g status-left "[#S]"

# Layout/history/stream control
psmux next-layout
psmux previous-layout
psmux clear-history
psmux pipe-pane -o "cat > pane.log"

# Hooks
psmux set-hook -g after-new-window "display-message created"
psmux show-hooks
```

### Target Syntax (`-t`)

psmux supports tmux-style targets:

```powershell
# window by index in session
psmux select-window -t work:2

# specific pane by index
psmux send-keys -t work:2.1 "echo hi" Enter

# pane by pane id
psmux send-keys -t %3 "pwd" Enter

# window by window id
psmux select-window -t @4
```

## Configuration

psmux reads its config on startup from the **first file found** (in order):

1. `~/.psmux.conf`
2. `~/.psmuxrc`
3. `~/.tmux.conf`
4. `~/.config/psmux/psmux.conf`

Config syntax is **tmux-compatible** — most `.tmux.conf` lines work as-is.

### Basic Config Example

Create `~/.psmux.conf`:

```tmux
# Change prefix key to Ctrl+a
set -g prefix C-a

# Enable mouse
set -g mouse on

# Window numbering base (default is 1)
set -g base-index 1

# Customize status bar
set -g status-left "[#S] "
set -g status-right "%H:%M %d-%b-%y"
set -g status-style "bg=green,fg=black"

# Cursor style: block, underline, or bar
set -g cursor-style bar
set -g cursor-blink on

# Scrollback history
set -g history-limit 5000

# Prediction dimming (disable for apps like Neovim)
set -g prediction-dimming off

# Key bindings
bind-key -T prefix h split-window -h
bind-key -T prefix v split-window -v
```

### Choosing a Shell

psmux launches **PowerShell 7 (pwsh)** by default. You can change this:

```tmux
# Use cmd.exe
set -g default-shell cmd

# Use PowerShell 5 (Windows built-in)
set -g default-shell powershell

# Use PowerShell 7 (explicit path)
set -g default-shell "C:/Program Files/PowerShell/7/pwsh.exe"

# Use Git Bash
set -g default-shell "C:/Program Files/Git/bin/bash.exe"

# Use Nushell
set -g default-shell nu

# Use Windows Subsystem for Linux (via wsl.exe)
set -g default-shell wsl
```

You can also launch a window with a specific command without changing the default:

```powershell
psmux new-window -- cmd /K echo hello
psmux new-session -s py -- python
psmux split-window -- "C:/Program Files/Git/bin/bash.exe"
```

### All Set Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `prefix` | Key | `C-b` | Prefix key |
| `base-index` | Int | `1` | First window number |
| `pane-base-index` | Int | `0` | First pane number |
| `escape-time` | Int | `500` | Escape delay (ms) |
| `repeat-time` | Int | `500` | Repeat key timeout (ms) |
| `history-limit` | Int | `2000` | Scrollback lines per pane |
| `display-time` | Int | `750` | Message display time (ms) |
| `display-panes-time` | Int | `1000` | Pane overlay time (ms) |
| `status-interval` | Int | `15` | Status refresh (seconds) |
| `mouse` | Bool | `on` | Mouse support |
| `status` | Bool | `on` | Show status bar |
| `status-position` | Str | `bottom` | `top` or `bottom` |
| `focus-events` | Bool | `off` | Pass focus events to apps |
| `mode-keys` | Str | `emacs` | `vi` or `emacs` |
| `renumber-windows` | Bool | `off` | Auto-renumber windows on close |
| `automatic-rename` | Bool | `on` | Rename windows from foreground process |
| `monitor-activity` | Bool | `off` | Flag windows with new output |
| `monitor-silence` | Int | `0` | Seconds before silence flag (0=off) |
| `synchronize-panes` | Bool | `off` | Send input to all panes |
| `remain-on-exit` | Bool | `off` | Keep panes after process exits |
| `aggressive-resize` | Bool | `off` | Resize to smallest client |
| `set-titles` | Bool | `off` | Update terminal title |
| `set-titles-string` | Str | | Terminal title format |
| `default-shell` | Str | `pwsh` | Shell to launch |
| `default-command` | Str | | Alias for default-shell |
| `word-separators` | Str | `" -_@"` | Copy-mode word delimiters |
| `prediction-dimming` | Bool | `on` | Dim predictive text |
| `cursor-style` | Str | | `block`, `underline`, or `bar` |
| `cursor-blink` | Bool | `off` | Cursor blinking |
| `bell-action` | Str | `any` | `any`, `none`, `current`, `other` |
| `visual-bell` | Bool | `off` | Visual bell indicator |
| `status-left` | Str | `[#S] ` | Left status bar content |
| `status-right` | Str | | Right status bar content |
| `status-style` | Str | `bg=green,fg=black` | Status bar style |
| `status-left-style` | Str | | Left status style |
| `status-right-style` | Str | | Right status style |
| `status-justify` | Str | `left` | Tab alignment: `left`, `centre`, `right` |
| `message-style` | Str | `bg=yellow,fg=black` | Message style |
| `message-command-style` | Str | `bg=black,fg=yellow` | Command prompt style |
| `mode-style` | Str | `bg=yellow,fg=black` | Copy-mode highlight |
| `pane-border-style` | Str | | Inactive border style |
| `pane-active-border-style` | Str | `fg=green` | Active border style |
| `window-status-format` | Str | `#I:#W#F` | Inactive tab format |
| `window-status-current-format` | Str | `#I:#W#F` | Active tab format |
| `window-status-separator` | Str | `" "` | Tab separator |
| `window-status-style` | Str | | Inactive tab style |
| `window-status-current-style` | Str | | Active tab style |
| `window-status-activity-style` | Str | `reverse` | Activity tab style |
| `window-status-bell-style` | Str | `reverse` | Bell tab style |
| `window-status-last-style` | Str | | Last-active tab style |

Style format: `"fg=colour,bg=colour,bold,dim,underscore,italics,reverse"`

Colours: `default`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `colour0`–`colour255`, `#RRGGBB`

### Environment Variables

```powershell
# Default session name used when not explicitly provided
$env:PSMUX_DEFAULT_SESSION = "work"

# Disable prediction dimming (useful for apps like Neovim)
$env:PSMUX_DIM_PREDICTIONS = "0"

# These are set INSIDE psmux panes (tmux-compatible):
# TMUX       — socket path and server info
# TMUX_PANE  — current pane ID (%0, %1, etc.)
```

### Neovim Rendering Workaround

If Neovim looks slow inside psmux or shows a "shadow" effect until you move the cursor, disable psmux prediction dimming in `~/.psmux.conf`:

```tmux
set -g prediction-dimming off
```

You can also disable it for the current shell only:

```powershell
$env:PSMUX_DIM_PREDICTIONS = "0"
psmux
```

To make it persistent for new shells:

```powershell
setx PSMUX_DIM_PREDICTIONS 0
```

## License

MIT

---

## About psmux

**psmux** (PowerShell Multiplexer) is a terminal multiplexer built specifically for Windows. It is an alternative to tmux for Windows users who want terminal multiplexing without WSL or Cygwin.

### Keywords

terminal multiplexer, tmux for windows, tmux alternative, tmux windows, windows terminal multiplexer, powershell multiplexer, split terminal windows, multiple terminals, terminal tabs, pane splitting, session management, windows terminal, powershell terminal, cmd terminal, rust terminal, console multiplexer, terminal emulator, windows console, cli tool, command line, devtools, developer tools, productivity, windows 10, windows 11, psmux, pmux

### Related Projects

- [tmux](https://github.com/tmux/tmux) — The original terminal multiplexer for Unix/Linux/macOS
- [Windows Terminal](https://github.com/microsoft/terminal) — Microsoft's modern terminal for Windows
- [PowerShell](https://github.com/PowerShell/PowerShell) — Cross-platform PowerShell

### FAQ

**Q: Is psmux cross-platform?**  
A: No. psmux is built exclusively for Windows. For Linux/macOS, use tmux.

**Q: Does psmux work with Windows Terminal?**  
A: Yes! psmux works great with Windows Terminal, PowerShell, cmd.exe, ConEmu, and other Windows terminal emulators.

**Q: Why use psmux instead of Windows Terminal tabs?**  
A: psmux offers session persistence (detach/reattach), synchronized input to multiple panes, and tmux-compatible keybindings.

**Q: Can I use tmux commands with psmux?**  
A: Yes! psmux includes `tmux` and `pmux` aliases. Commands like `tmux new-session`, `tmux attach`, `tmux ls` all work.
