# Scripting & Automation

psmux supports tmux-compatible commands for scripting and automation.

## Window & Pane Control

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

## Sending Keys

```powershell
# Send text directly
psmux send-keys "ls -la" Enter

# Send keys literally (no parsing)
psmux send-keys -l "literal text"

# Paste mode (legacy compatibility)
psmux send-keys -p

# Repeat a key N times
psmux send-keys -N 5 Up

# Send copy mode command
psmux send-keys -X copy-mode-up

# Special keys supported:
# Enter, Tab, Escape, Space, Backspace
# Up, Down, Left, Right, Home, End
# PageUp, PageDown, Delete, Insert
# F1-F12, C-a through C-z (Ctrl+key)
```

## Pane Information

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

## Paste Buffers

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

# Interactive buffer chooser (enter=paste, d=delete, esc=close)
psmux choose-buffer

# Clear command prompt history
psmux clear-prompt-history
```

## Pane Layout

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

## Pane Titles

Programs running inside a pane can set the title via OSC escape sequences. PowerShell 7 does this automatically with the current working directory. See [pane-titles.md](pane-titles.md) for full details on how pane titles work, how to control them, and how different shells behave.

```powershell
# Set a title on the active pane
psmux select-pane -T "my build pane"

# Set pane title on a specific pane
psmux select-pane -t %3 -T "logs"

# Set per-pane style (foreground/background color override)
psmux select-pane -P "bg=default,fg=blue"

# Display pane title using format variables
psmux display-message "#{pane_title}"
```

Enable `pane-border-format` and `pane-border-status` in your config to see titles on pane borders:

```tmux
set -g pane-border-status top
set -g pane-border-format " #{pane_index}: #{pane_title} "
```

## Popups

```powershell
# Open a popup running a command
psmux display-popup "Get-Process"

# Set width and height (absolute or percentage)
psmux display-popup -w 80% -h 50% "htop"

# Set the starting directory
psmux display-popup -d "C:\Projects" -w 100 -h 30

# Close popup on command exit (default behavior, -E inverts it)
psmux display-popup -E "git log --oneline -20"

# Keep popup open after command finishes
psmux display-popup -K "echo done"
```

## Menus

```powershell
# Display an interactive menu
# Format: display-menu [-x x] [-y y] [-T title] name key command ...
psmux display-menu -T "Actions" \
  "New Window" n "new-window" \
  "Split Horizontal" h "split-window -h" \
  "Split Vertical" v "split-window -v" \
  "Close Pane" x "kill-pane"

# Position the menu at specific coordinates
psmux display-menu -x 10 -y 5 -T "Quick" \
  "Zoom" z "resize-pane -Z" \
  "Rename" r "command-prompt -I '#W' 'rename-window %%'"
```

## Session Management

```powershell
# Check if session exists (exit code 0 = exists)
psmux has-session -t mysession

# Rename session
psmux rename-session newname

# Switch to another session
psmux switch-client -t other-session

# Cycle through sessions
psmux switch-client -n          # Next session
psmux switch-client -p          # Previous session
psmux switch-client -l          # Last (most recently used) session

# Create a session with environment variables
psmux new-session -s work -e "MY_VAR=value"

# Respawn pane (restart shell, or restart with a different command)
psmux respawn-pane
psmux respawn-pane -k           # Kill the current process first
psmux respawn-pane -c /tmp      # Restart in a different directory
```

## Pane Reorganization

```powershell
# Break the current pane out into a new window
psmux break-pane

# Break a specific pane, keep it in background
psmux break-pane -d -s %3

# Join a pane from another window into the current window
psmux join-pane -s :2           # Bring pane from window 2

# Join horizontally or vertically
psmux join-pane -h -s :2        # Join side by side
psmux join-pane -v -s :3        # Join top/bottom

# Move a pane (same as join-pane)
psmux move-pane -s %5 -t %3

# Find a window by name or content
psmux find-window "search term"
```

## Environment Variables

```powershell
# Set a global env var (inherited by all new panes)
psmux set-environment -g EDITOR vim

# Set a session-scoped env var
psmux set-environment MY_VAR value

# Unset a global env var
psmux set-environment -gu MY_VAR

# Show all environment variables
psmux show-environment
psmux show-environment -g
```

## Format Variables

The `display-message` command supports 140+ variables. Common ones include:

| Variable | Description |
|----------|-------------|
| `#S` | Session name |
| `#I` | Window index |
| `#W` | Window name |
| `#P` | Pane ID |
| `#T` | Pane title |
| `#H` | Hostname |
| `#{pane_current_path}` | Current working directory of the pane |
| `#{pane_current_command}` | Foreground process name |
| `#{pane_pid}` | PID of the pane's shell |
| `#{pane_width}` | Width of the pane in columns |
| `#{pane_height}` | Height of the pane in rows |
| `#{pane_active}` | `1` if this pane is the active pane |
| `#{pane_index}` | Pane index within the window |
| `#{window_zoomed_flag}` | `1` if the window has a zoomed pane |
| `#{window_panes}` | Number of panes in the window |
| `#{window_active}` | `1` if this is the active window |
| `#{session_windows}` | Number of windows in the session |
| `#{session_attached}` | Number of clients attached to the session |
| `#{client_prefix}` | `1` if the prefix key was pressed |
| `#{client_width}` | Width of the client terminal |
| `#{client_height}` | Height of the client terminal |

### Format Modifiers

```powershell
# Conditional
psmux display-message -p "#{?window_zoomed_flag,ZOOMED,normal}"

# Comparison
psmux display-message -p "#{==:#{pane_index},0}"

# Regex substitution
psmux display-message -p "#{s/old/new/:pane_title}"

# Basename and dirname
psmux display-message -p "#{b:pane_current_path}"
psmux display-message -p "#{d:pane_current_path}"

# Loop over all windows
psmux display-message -p "#{W:#{window_index}:#{window_name} }"

# Loop over all panes
psmux display-message -p "#{P:#{pane_index} }"
```

## Advanced Commands

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
psmux select-layout tiled         # Apply a specific layout
psmux clear-history
psmux pipe-pane -o "cat > pane.log"

# Hooks (event callbacks) - see Hooks section below for full reference
psmux set-hook -g after-new-window "display-message created"
psmux set-hook -g client-attached "run-shell 'echo attached'"
psmux set-hook -gu after-new-window     # Unset (remove) a hook
psmux show-hooks

# Run shell commands
psmux run-shell "echo hello"           # Output shown in status bar
psmux run-shell -b "long-running.ps1"  # Fire-and-forget (background)

# Conditional execution
psmux if-shell "test -f ~/.psmux.conf" "source-file ~/.psmux.conf"
psmux if-shell -F "#{window_zoomed_flag}" "" "resize-pane -Z"

# User confirmation dialogs
psmux confirm-before -p "Kill this pane? (y/n)" kill-pane

# Wait channels for cross-pane synchronization
psmux wait-for -L mychannel             # Lock a channel
psmux wait-for -S mychannel             # Signal (unlock) a channel
psmux wait-for mychannel                # Wait until channel is signaled
```

## Hooks (Event Callbacks)

Hooks let you run commands automatically when events occur. They are one of the most powerful scripting features in psmux.

### Setting Hooks

```powershell
# Global hook (applies to all sessions)
psmux set-hook -g after-new-window "display-message 'New window created'"

# Session-scoped hook
psmux set-hook after-split-window "select-layout tiled"

# Chain multiple commands in a hook
psmux set-hook -g after-new-session "set -g status-left '[#S] ' \; display-message 'Session ready'"
```

### Available Hook Events

| Hook | Fires when... |
|------|---------------|
| `after-new-session` | A new session is created |
| `after-new-window` | A new window is created |
| `after-split-window` | A pane is split |
| `client-attached` | A client attaches to a session |
| `client-detached` | A client detaches from a session |
| `after-select-window` | A different window is selected |
| `after-select-pane` | A different pane is selected |
| `after-resize-pane` | A pane is resized |
| `pane-died` | A pane's process exits |
| `alert-activity` | Activity detected in a monitored window |
| `alert-silence` | Silence detected in a monitored window |
| `alert-bell` | Bell received from a pane |
| `after-kill-pane` | A pane is killed |

### Removing Hooks

```powershell
# Remove a global hook
psmux set-hook -gu after-new-window

# View all active hooks
psmux show-hooks
```

**Important:** If you repeatedly call `set-hook -g` for the same event, psmux appends duplicate entries. Use `set-hook -gu` to clear the old hook before setting a new one, or check `show-hooks` to verify no duplicates.

## Display Panes

Show numbered overlays on all panes, then type a number to jump to that pane:

```powershell
# Show pane number overlay (also: Prefix + q)
psmux display-panes
```

The overlay shows each pane's number according to `pane-base-index`. Press a number key while the overlay is visible to switch to that pane. The overlay auto-dismisses after `display-panes-time` milliseconds.

## Run Shell

Run an external command and display the output:

```powershell
# Output appears in the status bar message area
psmux run-shell "echo hello"

# Run in background (fire-and-forget, no output displayed)
psmux run-shell -b "long-running-script.ps1"

# Use format variables in shell commands
psmux run-shell "echo 'Current pane: #{pane_index}'"
```

## Interactive Choosers

```powershell
# Interactive session/window/pane tree browser
psmux choose-tree

# Show only sessions
psmux choose-tree -s

# Show only windows
psmux choose-tree -w

# Interactive buffer picker (enter=paste, d=delete)
psmux choose-buffer

# Interactive client picker
psmux choose-client

# Interactive options editor
psmux customize-mode
```

## Target Syntax (`-t`)

psmux supports tmux-style targets:

```powershell
# Window by index in session
psmux select-window -t work:2

# Window by name in session
psmux select-window -t work:editor

# Specific pane by index
psmux send-keys -t work:2.1 "echo hi" Enter

# Pane by pane id
psmux send-keys -t %3 "pwd" Enter

# Window by window id
psmux select-window -t @4

# Target a specific session
psmux has-session -t mysession

# Session:window.pane full path
psmux send-keys -t dev:0.2 "make build" Enter
```

## Server Namespaces (`-L`)

Use `-L` to run multiple isolated psmux servers on the same machine:

```powershell
# Start a session in a named server namespace
psmux -L work new-session -s dev

# Attach to a session in that namespace
psmux -L work attach -t dev

# Each namespace gets its own server, sessions, and socket
psmux -L personal new-session -s play
```

## Key Binding Management

```powershell
# Bind a key in the default prefix table
psmux bind-key h split-window -h

# Bind with format variable expansion (-F flag)
psmux bind-key -F M-h "resize-pane -L #{pane_width}"

# Bind with repeat (successive presses within repeat-time don't need prefix)
psmux bind-key -r Left select-pane -L
psmux bind-key -r Right select-pane -R

# Bind in root table (no prefix needed)
psmux bind-key -n M-Left select-pane -L

# Bind in a specific key table
psmux bind-key -T copy-mode-vi y send-keys -X copy-selection

# Unbind a single key
psmux unbind-key h

# Unbind ALL keys (reset to clean slate)
psmux unbind-key -a

# Unbind all keys in a specific key table only
psmux unbind-key -a -T copy-mode-vi
psmux unbind-key -a -T prefix
psmux unbind-key -a -T root
psmux unbind-key -a -T copy-mode
```

## Command Chaining

Chain multiple commands with `\;` in config files:

```tmux
# Split and select in one binding
bind-key M-v split-window -v \; select-pane -U

# Create a 3-pane layout
bind-key M-d split-window -h \; split-window -v \; select-pane -t 0

# Conditional chaining
bind-key M-z if-shell -F "#{window_zoomed_flag}" "resize-pane -Z" ""
```

From the CLI, use `\;` or quote the command:

```powershell
psmux split-window -h `; select-pane -L
```

## Querying Lists with Custom Formats

```powershell
# List all sessions with custom format
psmux list-sessions -F "#{session_name}:#{session_windows}"

# List all windows with custom format
psmux list-windows -F "#{window_index}:#{window_name}:#{window_panes}"

# List all panes across the session (-s flag)
psmux list-panes -s -F "#{window_index}.#{pane_index}: #{pane_current_command} [#{pane_width}x#{pane_height}]"

# List all panes across all sessions (-a flag)
psmux list-panes -a

# Capture pane content to stdout
psmux capture-pane -p -t %0

# Capture with line range (negative = scrollback)
psmux capture-pane -p -S -100 -E -1

# Print a format variable
psmux display-message -p "#{pane_current_path}"
```

## Window and Pane Creation Options

### new-window

```powershell
# Create a window with a name
psmux new-window -n "logs"

# Create a window in the background (don't switch to it)
psmux new-window -d -n "background"

# Create a window in a specific directory
psmux new-window -c "C:\Projects\myapp"

# Create a window running a command
psmux new-window -n "build" -- cargo watch

# Create a window at a specific index
psmux new-window -t 5
```

When you set a window name with `-n`, automatic renaming is disabled for that window so the foreground process name does not overwrite your chosen name.

### split-window

```powershell
# Split with percentage size
psmux split-window -v -p 30            # Bottom pane gets 30%
psmux split-window -h -p 70            # Right pane gets 70%

# Split in the current pane's directory
psmux split-window -h -c "#{pane_current_path}"

# Split with a specific command
psmux split-window -v -- python

# Split a specific target pane
psmux split-window -v -t %3

# Split without switching focus
psmux split-window -d -v
```

### new-session

```powershell
# Create a named session
psmux new-session -s work

# Create in a specific directory
psmux new-session -s project -c "C:\Projects\myapp"

# Create with environment variables
psmux new-session -s dev -e "NODE_ENV=development"

# Create in background (detached)
psmux new-session -d -s background

# Create with an initial command
psmux new-session -s monitor -- htop

# Create a session with a named first window
psmux new-session -s work -n "editor"
```

## Target Syntax

Many commands accept a `-t` flag to specify which session, window, or pane to act on:

```powershell
# Target a session by name
psmux switch-client -t mysession

# Target a window by index (within current session)
psmux select-window -t 3

# Target a window in a specific session
psmux select-window -t mysession:2

# Target a pane by ID (absolute, shown with %)
psmux select-pane -t %5

# Target a pane within a window
psmux select-pane -t :2.1             # Window 2, pane 1

# Special targets
psmux select-pane -t +               # Next pane
psmux select-pane -t -               # Previous pane
psmux select-window -t !             # Last (previous) window
```

## Server Namespaces

Run isolated psmux instances using the `-L` flag. Each namespace gets its own server process with its own sessions:

```powershell
# Start a session in a named namespace
psmux -L work new-session -s dev

# Attach to a session in that namespace
psmux -L work attach

# List sessions in a namespace
psmux -L work list-sessions

# Default namespace is used when -L is not specified
```

This is useful for running completely separate psmux environments, for example one for development and one for monitoring.
