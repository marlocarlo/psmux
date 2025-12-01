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

## Features

- Split panes horizontally and vertically
- Multiple windows with tabs
- Session management (attach/detach)
- Mouse support for resizing panes
- Copy mode with vim-like keybindings
- Synchronized input to multiple panes

## Requirements

- Windows 10 or Windows 11
- PowerShell 5.1+, PowerShell Core 7+, or cmd.exe

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

### Using Cargo

```powershell
cargo install psmux
```

### Using Chocolatey

```powershell
choco install psmux
```

### From GitHub Releases

Download the latest `.zip` from [GitHub Releases](https://github.com/marlocarlo/psmux/releases) and add to your PATH.

### From Source

```powershell
git clone https://github.com/marlocarlo/psmux.git
cd psmux
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
| `Prefix + z` | Toggle pane zoom |
| `Prefix + n` | Next window |
| `Prefix + p` | Previous window |
| `Prefix + 0-9` | Select window by number |
| `Prefix + d` | Detach from session |
| `Prefix + ,` | Rename current window |
| `Prefix + w` | Window/pane chooser |
| `Prefix + [` | Enter copy mode |
| `Prefix + ]` | Paste from buffer |
| `Prefix + q` | Display pane numbers |
| `Prefix + Arrow` | Navigate between panes |
| `Ctrl+q` | Quit |

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
psmux select-window -t 0       # Select window by index
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

## Configuration

Create `~/.psmux.conf`:

```
# Change prefix key to Ctrl+a
set -g prefix C-a

# Enable mouse
set -g mouse on

# Customize status bar
set -g status-left "[#S]"
set -g status-right "%H:%M"

# Cursor style: block, underline, or bar
set -g cursor-style bar
set -g cursor-blink on
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
