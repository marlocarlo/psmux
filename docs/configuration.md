# Configuration

psmux reads its config on startup from the **first file found** (in order):

1. `~/.psmux.conf`
2. `~/.psmuxrc`
3. `~/.tmux.conf`
4. `~/.config/psmux/psmux.conf`

Config syntax is **tmux-compatible**. Most `.tmux.conf` lines work as-is.

You can also specify a custom config file path with the `-f` flag:

```powershell
# Use a specific config file instead of default search
psmux -f ~/.config/psmux/custom.conf

# Use an empty config (no settings loaded)
psmux -f NUL
```

This sets the `PSMUX_CONFIG_FILE` environment variable internally, which the server checks before searching the default locations.

## Basic Config Example

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

## Choosing a Shell

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

## All Set Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `prefix` | Key | `C-b` | Prefix key |
| `prefix2` | Key | `none` | Secondary prefix key (optional) |
| `base-index` | Int | `0` | First window number |
| `pane-base-index` | Int | `0` | First pane number |
| `escape-time` | Int | `500` | Escape delay (ms) |
| `repeat-time` | Int | `500` | Repeat key timeout (ms) |
| `history-limit` | Int | `2000` | Scrollback lines per pane |
| `display-time` | Int | `750` | Message display time (ms) |
| `display-panes-time` | Int | `1000` | Pane overlay time (ms) |
| `status-interval` | Int | `15` | Status refresh (seconds) |
| `mouse` | Bool | `on` | Mouse support |
| `scroll-enter-copy-mode` | Bool | `on` | Enter copy mode on mouse scroll (set `off` to disable) |
| `pwsh-mouse-selection` | Bool | `off` | Windows 11 PowerShell-style word/line selection (double/triple-click) |
| `status` | Bool/Int | `on` | Show status bar (number = line count) |
| `status-position` | Str | `bottom` | `top` or `bottom` |
| `status-justify` | Str | `left` | `left`, `centre`, `right`, `absolute-centre` |
| `status-left-length` | Int | `10` | Max width of status-left |
| `status-right-length` | Int | `40` | Max width of status-right |
| `focus-events` | Bool | `off` | Pass focus events to apps |
| `mode-keys` | Str | `emacs` | `vi` or `emacs` |
| `renumber-windows` | Bool | `off` | Auto-renumber windows on close |
| `automatic-rename` | Bool | `on` | Rename windows from foreground process |
| `monitor-activity` | Bool | `off` | Flag windows with new output |
| `monitor-silence` | Int | `0` | Seconds before silence flag (0=off) |
| `visual-activity` | Bool | `off` | Visual indicator for activity |
| `synchronize-panes` | Bool | `off` | Send input to all panes |
| `remain-on-exit` | Bool | `off` | Keep panes after process exits |
| `aggressive-resize` | Bool | `off` | Resize to smallest client |
| `window-size` | Str | `latest` | `largest`, `smallest`, `manual`, `latest` |
| `destroy-unattached` | Bool | `off` | Exit server when no clients attached |
| `exit-empty` | Bool | `on` | Exit server when all windows closed |
| `set-titles` | Bool | `off` | Update terminal title |
| `set-titles-string` | Str | | Terminal title format |
| `default-shell` | Str | `pwsh` | Shell to launch |
| `default-command` | Str | | Alias for default-shell |
| `word-separators` | Str | `" -_@"` | Copy-mode word delimiters |
| `activity-action` | Str | `other` | Action on window activity: `any`, `none`, `current`, `other` |
| `silence-action` | Str | `other` | Action on window silence: `any`, `none`, `current`, `other` |
| `bell-action` | Str | `any` | Bell action: controls audible bell forwarding and status bar flag (`any`, `none`, `current`, `other`) |
| `visual-bell` | Bool | `off` | Visual bell indicator |
| `allow-passthrough` | Str | `off` | Allow terminal passthrough sequences (`on`/`off`/`all`) |
| `allow-rename` | Bool | `on` | Allow programs to set window title via escape sequences |
| `allow-set-title` | Bool | `off` | Allow programs to set pane title via OSC 0/2 escape sequences (see [pane-titles.md](pane-titles.md)) |
| `allow-predictions` | Bool | `off` | Preserve PSReadLine prediction settings (see below) |
| `default-terminal` | Str | | Terminal type string (sets `TERM` env var in panes) |
| `update-environment` | Str | *(tmux defaults)* | Space-separated list of env vars to refresh on client attach |
| `warm` | Bool | `on` | Pre-spawn shells for instant window/pane creation (see [warm-sessions.md](warm-sessions.md)) |
| `copy-command` | Str | | Shell command for clipboard pipe |
| `set-clipboard` | Str | `on` | Clipboard interaction (`on`/`off`/`external`) |
| `main-pane-width` | Int | `0` | Main pane width in main-vertical layout |
| `main-pane-height` | Int | `0` | Main pane height in main-horizontal layout |

### Style Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `status-left` | Str | `[#S] ` | Left status bar content |
| `status-right` | Str | | Right status bar content |
| `status-style` | Str | `bg=green,fg=black` | Status bar style |
| `status-left-style` | Str | | Left status style |
| `status-right-style` | Str | | Right status style |
| `message-style` | Str | `bg=yellow,fg=black` | Message style |
| `message-command-style` | Str | `bg=black,fg=yellow` | Command prompt style |
| `mode-style` | Str | `bg=yellow,fg=black` | Copy-mode highlight |
| `pane-border-style` | Str | | Inactive border style |
| `pane-active-border-style` | Str | `fg=green` | Active border style |
| `pane-border-format` | Str | | Pane border format string (e.g. `#{pane_index}: #{pane_title}`) |
| `pane-border-status` | Str | | Pane border status position (`top`/`bottom`/`off`) |
| `window-status-format` | Str | `#I:#W#F` | Inactive tab format |
| `window-status-current-format` | Str | `#I:#W#F` | Active tab format |
| `window-status-separator` | Str | `" "` | Tab separator |
| `window-status-style` | Str | | Inactive tab style |
| `window-status-current-style` | Str | | Active tab style |
| `window-status-activity-style` | Str | `reverse` | Activity tab style |
| `window-status-bell-style` | Str | `reverse` | Bell tab style |
| `window-status-last-style` | Str | | Last-active tab style |

### Multi-line Status Bar (`status-format[]`)

psmux supports a multi-line status bar using the `status-format[]` array. Set the `status` option to a number to control how many lines the status bar displays:

```tmux
# Enable a 2-line status bar
set -g status 2

# Configure each line (0-indexed)
set -g status-format[0] "#[align=left]#S #[align=right]%H:%M"
set -g status-format[1] "#[align=left]#{W:#I:#W }"
```

The first line (`status-format[0]`) replaces the default status bar content. Additional lines stack below (or above, depending on `status-position`).

### Pane Border Labels

Show pane information on the border between panes:

```tmux
# Enable pane border labels at the top of each pane
set -g pane-border-status top

# Customize what the label shows
set -g pane-border-format " #{pane_index}: #{pane_title} [#{pane_current_command}] "

# Disable pane border labels
set -g pane-border-status off
```

Use `select-pane -T "title"` to set a pane title that appears in the border label. Clear a title with `select-pane -T ""`. The default pane title is the hostname, matching tmux convention.

> **Note:** PowerShell 7 automatically sets the pane title to the current working directory on every prompt via OSC escape sequences. If you see a file path in your pane border labels instead of the hostname, see [pane-titles.md](pane-titles.md) for details and options to control this.

### Bell

When a program inside a pane emits BEL (`\x07`), psmux forwards the bell character to your host terminal so you hear the audible beep. The `bell-action` option controls when this happens and when the status bar tab gets a bell flag (`!`):

```tmux
# Forward bell from any window (default)
set -g bell-action any

# Forward bell only from the active window
set -g bell-action current

# Forward bell only from non-active windows
set -g bell-action other

# Mute bell completely (no sound, no status bar flag)
set -g bell-action none
```

The `window-status-bell-style` option controls how the tab looks when flagged:

```tmux
set -g window-status-bell-style "fg=red,bold"
```

PowerShell example to test:

```powershell
# These should all produce an audible beep inside psmux:
Write-Host "`a"
[Console]::Beep()
[char]7
```

### Mouse Configuration

Mouse support is enabled by default. You can customize how the mouse interacts with psmux:

```tmux
# Disable mouse entirely (no click, scroll, or drag)
set -g mouse off

# Disable entering copy mode on mouse scroll
set -g scroll-enter-copy-mode off

# Enable Windows 11 PowerShell-style word/line selection
# Double-click selects a word, triple-click selects a line
set -g pwsh-mouse-selection on
```

When `scroll-enter-copy-mode` is `off`, scrolling in a pane does not enter copy mode and instead passes scroll events directly to the running application.

### Command Chaining

psmux supports tmux-style command chaining with the `;` operator. Multiple commands on a single line are executed sequentially:

```tmux
# Split and move focus in one binding
bind-key M-s split-window -h \; select-pane -L

# Create a development layout
bind-key D split-window -v -p 30 \; split-window -h \; select-pane -t 0
```

In config files, escape the semicolon with `\;` so it is not treated as a comment delimiter.

### Case-Sensitive Key Bindings

psmux distinguishes between lowercase and uppercase letters in key bindings, matching tmux behavior:

```tmux
# These are two different bindings:
bind-key t clock-mode           # Prefix + t (lowercase)
bind-key T choose-tree          # Prefix + Shift+T (uppercase)

# Uppercase bindings for plugin managers
bind-key I run-shell '~/.psmux/plugins/ppm/scripts/install_plugins.ps1'
bind-key U run-shell '~/.psmux/plugins/ppm/scripts/update_plugins.ps1'
```

### Ctrl+Space as Prefix

Multi-character key names like `Space`, `Enter`, `Tab`, and `Escape` are fully supported in prefix configuration:

```tmux
set -g prefix C-Space
unbind-key C-b
bind-key C-Space send-prefix
```

### psmux Extensions (Windows-specific)

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `prediction-dimming` | Bool | `off` | Dim predictive/speculative text |
| `cursor-style` | Str | | Cursor shape: `block`, `underline`, or `bar` |
| `cursor-blink` | Bool | `off` | Cursor blinking |
| `env-shim` | Bool | `on` | Inject Unix-compatible `env` function in PowerShell panes |
| `claude-code-fix-tty` | Bool | `on` | Patch Node.js process.stdout.isTTY for Claude Code |
| `claude-code-force-interactive` | Bool | `on` | Set CLAUDE_CODE_FORCE_INTERACTIVE=1 in panes |

Style format: `"fg=colour,bg=colour,bold,dim,underscore,italics,reverse,strikethrough"`

Colours: `default`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `colour0`–`colour255`, `#RRGGBB`

## Environment Variables

```powershell
# Default session name used when not explicitly provided
$env:PSMUX_DEFAULT_SESSION = "work"

# Enable prediction dimming (off by default; dims predictive/speculative text)
$env:PSMUX_DIM_PREDICTIONS = "1"

# Disable warm pane pre-spawning (same as set -g warm off)
$env:PSMUX_NO_WARM = "1"

# Override the config file path (same effect as -f flag)
$env:PSMUX_CONFIG_FILE = "C:\Users\me\.psmux-alt.conf"

# These are set INSIDE psmux panes (tmux-compatible):
# TMUX       - socket path and server info
# TMUX_PANE  - current pane ID (%0, %1, etc.)
```

## Managing Environment Variables

Use `set-environment` to set env vars that are inherited by newly created panes:

```powershell
# Set a global env var (inherited by all new panes)
psmux set-environment -g EDITOR vim

# Set a session-scoped env var
psmux set-environment MY_VAR value

# Unset an env var
psmux set-environment -gu MY_VAR

# Show all environment variables
psmux show-environment
psmux show-environment -g
```

Environment variables set this way are injected at the process level when new panes spawn, so they are completely invisible (no commands echoed in the shell).

## PSReadLine Predictions (Intellisense / Autocompletion)

By default, psmux disables PSReadLine inline predictions (the grayed-out autocompletion/intellisense suggestions that appear as you type) to avoid additional unexpected bugs caused by the interaction between predictions and ConPTY. This means `PredictionSource` defaults to `None` inside psmux, even if your profile sets it to `HistoryAndPlugin` ([#150](https://github.com/psmux/psmux/issues/150)).

If enough people test predictions and the community supports enabling them by default, this will be changed in a future release.

To preserve your prediction/autocompletion settings, enable `allow-predictions`:

```tmux
set -g allow-predictions on
```

With this enabled:
- If your profile sets `PredictionSource`, psmux respects your choice
- If your profile does not set it, psmux restores the system default (typically `HistoryAndPlugin`)

## Prediction Dimming

Prediction dimming is off by default. If you want psmux to dim predictive/speculative text (e.g. shell autosuggestions), you can enable it in `~/.psmux.conf`:

```tmux
set -g prediction-dimming on
```

You can also enable it for the current shell only:

```powershell
$env:PSMUX_DIM_PREDICTIONS = "1"
psmux
```

To make it persistent for new shells:

```powershell
setx PSMUX_DIM_PREDICTIONS 1
```

## Reloading Configuration at Runtime

You can reload your config file without restarting psmux. From the command prompt (`Prefix + :`), run:

```tmux
source-file ~/.psmux.conf
```

Or from outside psmux:

```powershell
psmux source-file ~/.psmux.conf
```

This re-executes every line in the config file, applying any changes to options, key bindings, hooks, and styles immediately.

## Window and Pane Numbering

By default, windows and panes are numbered starting from 0. You can change the starting index for both:

```tmux
# Start window numbering at 1
set -g base-index 1

# Start pane numbering at 1
set -g pane-base-index 1
```

The `pane-base-index` setting affects:

- **Display Panes overlay** (`Prefix + q`): The numbers shown on each pane start from your configured base index
- **Pane targets**: When referencing panes by number (e.g. `select-pane -t 1`), numbering follows your base index
- **Format variables**: `#{pane_index}` reflects the base index setting
- **Status bar and border labels**: Pane numbers in format strings use the configured base

A common setup for both windows and panes to start at 1:

```tmux
set -g base-index 1
set -g pane-base-index 1
```

## Display Panes Overlay

Press `Prefix + q` to show numbered overlays on each pane. While the overlay is visible, press any displayed number key to jump to that pane. The overlay auto-dismisses after `display-panes-time` milliseconds (default: 1000ms).

```tmux
# Show pane numbers for 3 seconds
set -g display-panes-time 3000
```

The numbers shown respect your `pane-base-index` setting. For example, with `pane-base-index 1`, three panes show as 1, 2, 3 instead of 0, 1, 2.

You can also trigger this overlay from the command line:

```powershell
psmux display-panes
```

## Split Window Options

When splitting panes, you can control the size and starting directory of the new pane:

```tmux
# Split vertically, new pane takes 30% of the space
split-window -v -p 30

# Split horizontally, new pane takes 70% of the space
split-window -h -p 70

# Split and start in a specific directory
split-window -v -c "C:\Projects\myapp"

# Split and start in the current pane's directory
split-window -h -c "#{pane_current_path}"

# Split and run a specific command
split-window -v -- python
```

These flags also work when creating new windows:

```tmux
# New window with a specific name
new-window -n "logs"

# New window in a specific directory
new-window -c "C:\Projects"

# New window running a specific command with a name
new-window -n "build" -- cargo build --watch
```

When you set a window name with `-n`, the `automatic-rename` flag is turned off for that window so psmux does not overwrite your chosen name with the foreground process name. To re-enable automatic renaming for that window:

```tmux
set-option -w automatic-rename on
```

## Detach and Exit Policies

Control what happens when clients disconnect or all windows close:

```tmux
# Exit the server when no clients are attached (default: off)
set -g destroy-unattached on

# Exit the server when the last window/session closes (default: on)
set -g exit-empty on
```

With `destroy-unattached on`, the server process terminates as soon as the last client detaches. This is useful for single-use sessions.

With `exit-empty off`, the server stays alive even after all sessions are closed, allowing new sessions to be created without restarting.

## Dead Panes and Respawn

When a process inside a pane exits, the pane normally closes. To keep the pane visible after its process exits:

```tmux
set -g remain-on-exit on
```

A pane with a dead process shows its last output and can be respawned:

```powershell
# Restart the default shell in the pane
psmux respawn-pane

# Kill any remaining process and restart
psmux respawn-pane -k

# Respawn in a different directory
psmux respawn-pane -c "C:\Projects"

# Respawn with a specific command
psmux respawn-pane -- python app.py
```

This is useful for monitoring: if a long-running process crashes, you can see its final output and restart it without losing the pane layout.

## Session Environment Variables

You can set environment variables at the session or global level that get inherited by all new panes:

```powershell
# Set a global env var (all new panes in all sessions inherit this)
psmux set-environment -g EDITOR vim

# Set a session-scoped env var
psmux set-environment MY_VAR value

# Unset a global env var
psmux set-environment -gu MY_VAR

# View all environment variables
psmux show-environment
psmux show-environment -g
```

You can also pass environment variables when creating a new session:

```powershell
# Create a session with custom environment
psmux new-session -s work -e "PROJECT=myapp" -e "ENV=production"
```

## Status Bar Time Updates

The status bar supports time format variables that update in real time:

```tmux
# Show current time in the status bar (updates every second)
set -g status-right "%H:%M:%S %d-%b-%y"

# Common time format variables:
#   %H   Hour (24-hour, 00-23)
#   %I   Hour (12-hour, 01-12)
#   %M   Minute (00-59)
#   %S   Second (00-59)
#   %p   AM/PM
#   %r   Full time in 12-hour format (e.g. 02:30:45 PM)
#   %R   Hour:Minute in 24-hour format (e.g. 14:30)
#   %d   Day of month (01-31)
#   %b   Abbreviated month name (Jan, Feb, ...)
#   %Y   Full year (2025)
#   %a   Abbreviated weekday (Mon, Tue, ...)
```

Time variables refresh based on the `status-interval` option (default: 15 seconds). For second-level precision, reduce the interval:

```tmux
# Update status bar every second (for live clock)
set -g status-interval 1
```

## PSReadLine ListView

psmux supports PSReadLine's ListView prediction style, which shows a dropdown list of suggestions:

```powershell
# In your PowerShell profile ($PROFILE)
Set-PSReadLineOption -PredictionSource HistoryAndPlugin
Set-PSReadLineOption -PredictionViewStyle ListView
```

For this to work inside psmux, enable `allow-predictions` in your psmux config:

```tmux
set -g allow-predictions on
```

Without `allow-predictions on`, psmux resets PSReadLine's prediction settings during initialization, which disables ListView mode.
