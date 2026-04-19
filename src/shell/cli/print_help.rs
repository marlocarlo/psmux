#[allow(unused_imports)]
use crate::types::{ParsedTarget, VERSION};

/// Normalize `-x=VALUE` short-flag forms into `["-x", "VALUE"]`.
///
/// tmux accepts both `-t VALUE` (space) and `-t=VALUE` (equals) for
/// single-character flags.  psmux's parsers only handled the space form.
/// This function expands the equals form so every downstream comparison
/// (`arg == "-t"`, `args.windows(2)`, etc.) works without changes.
///
/// Rules:
///   - Only tokens starting with a single `-` (not `--`) are split.
///   - The flag letter must be ASCII alphabetic (`-t=foo` yes, `-1=bar` no).
///   - Long flags (`--name=value`) pass through unchanged.
///   - Positional tokens without a leading `-` pass through unchanged.
///   - Bare `-` and degenerate `-=` pass through unchanged.
use super::*;

pub fn print_help() {
    let prog = get_program_name();
    println!(r#"{prog} v{ver} - Terminal multiplexer for Windows (tmux alternative)

USAGE:
    {prog} [COMMAND] [OPTIONS]

SESSION COMMANDS:
    (no command)            Start a new session or attach to existing one
    new-session, new        Create a new session
        -s <name>           Session name (default: "default")
        -d                  Start detached (in background)
        -n <winname>        Name for the initial window
        -- <cmd> [args]     Run a specific command instead of default shell
    a, at, attach, attach-session
                            Attach to an existing session
        -t <name>           Target session name
    ls, list-sessions       List all active sessions
    has-session, has        Check if a session exists (exit code 0 = yes)
        -t <name>           Target session name
    kill-session, kill-ses  Kill a session
        -t <name>           Target session name
    kill-server             Kill all sessions and the server
    rename-session, rename  Rename the current session
    switch-client, switchc  Switch to another session
    list-clients, lsc       List connected clients
    server-info, info       Show server information

WINDOW COMMANDS:
    new-window, neww        Create a new window in current session
        -n <name>           Window name
        -d                  Create but don't switch to it
        -c <dir>            Start directory
    kill-window, killw      Close the current window
    rename-window, renamew  Rename current window
    select-window, selectw  Select a window by index
        -t <index>          Target window index
    next-window, next       Go to next window
    previous-window, prev   Go to previous window
    last-window, last       Go to last active window
    move-window, movew      Move window to a different index
    swap-window, swapw      Swap two windows
    find-window, findw      Search for a window by name
    link-window, linkw      Link a window to another session
    unlink-window, unlinkw  Unlink a window
    list-windows, lsw       List windows in a session

PANE COMMANDS:
    split-window, splitw    Split current pane
        -h                  Split horizontally (side by side)
        -v                  Split vertically (top/bottom, default)
        -p <percent>        Size as percentage
        -c <dir>            Start directory
    kill-pane, killp        Close the current pane
    select-pane, selectp    Select a pane
        -U / -D / -L / -R  Direction (up/down/left/right)
        -t <id>             Target pane (e.g. %3)
        -m / -M             Mark / unmark pane
    resize-pane, resizep    Resize a pane
        -U/-D/-L/-R <n>    Direction and amount
        -Z                  Toggle zoom
        -x <cols> -y <rows> Absolute size
    swap-pane, swapp        Swap two panes
        -U / -D             Direction
    join-pane, joinp        Join a pane to a window
    break-pane, breakp      Break pane into a new window
    rotate-window, rotatew  Rotate panes in a window
    display-panes, displayp Display pane numbers
    zoom-pane               Toggle pane zoom (alias for resizep -Z)
    respawn-pane, respawnp  Restart the pane's shell
    pipe-pane, pipep        Pipe pane output to a command
    list-panes, lsp         List panes in current window
    capture-pane, capturep  Capture pane content to buffer
        -p                  Print to stdout

COPY & PASTE COMMANDS:
    copy-mode               Enter copy/scroll mode
    set-buffer, setb        Set paste buffer content
    paste-buffer, pasteb    Paste from buffer to active pane
    list-buffers, lsb       List paste buffers
    show-buffer, showb      Display paste buffer content
    delete-buffer, deleteb  Delete a paste buffer
    choose-buffer, chooseb  Interactive buffer chooser
    save-buffer, saveb      Save buffer to file
    load-buffer, loadb      Load buffer from file
    clear-history, clearhist Clear pane scrollback history

KEY BINDING COMMANDS:
    bind-key, bind          Bind a key to a command
    unbind-key, unbind      Unbind a key
    list-keys, lsk          List all key bindings
    send-keys, send         Send keys to a pane
        -l                  Send literally (no key parsing)
        -p                  Paste text (legacy compatibility)
        -t <target>         Target pane

CONFIGURATION COMMANDS:
    set-option, set         Set a session/window option
        -g                  Set globally
        -u                  Unset (reset to default)
        -a                  Append to current value
        -q                  Quiet (no error on unknown option)
    show-options, show      Show all options and values
    show-window-options, showw  Show window-scoped options
    source-file, source     Execute commands from a config file
    set-environment, setenv Set an environment variable
    show-environment, showenv Show environment variables
    set-hook                Set a hook command for an event
    show-hooks              Show all defined hooks
    list-commands, lscm     List all available commands

LAYOUT COMMANDS:
    select-layout, selectl  Apply a layout preset
                            Presets: even-horizontal, even-vertical,
                            main-horizontal, main-vertical, tiled
    next-layout             Cycle to next layout
    previous-layout         Cycle to previous layout

DISPLAY COMMANDS:
    display-message, display  Display a message or format variable
    display-menu, menu      Display an interactive menu
    display-popup, popup    Display a popup window
    confirm-before, confirm Run command after y/n confirmation
    clock-mode              Display a big clock
    run-shell, run          Run a shell command
    if-shell, if            Conditional command execution
    wait-for, wait          Wait for / signal a named channel

MISC:
    help                    Show this help message
    version                 Show version information

OPTIONS:
    -h, --help              Show this help message
    -V, --version           Show version information
    -f <file>               Use <file> as the configuration file
    -L <name>               Name the server socket (namespace isolation)
    -S <path>               Specify server socket path
    -t <target>             Target session, window, or pane

TARGET SYNTAX (-t):
    session:window.pane     Full target path
    :2                      Window 2 in current session
    :2.1                    Pane 1 of window 2
    %3                      Pane by pane ID
    @4                      Window by window ID
    work:2                  Window 2 in session "work"

CONFIGURATION:
    psmux reads config on startup from the first file found:
        %USERPROFILE%\.psmux.conf
        %USERPROFILE%\.psmuxrc
        %USERPROFILE%\.tmux.conf
        %USERPROFILE%\.config\psmux\psmux.conf

    Config syntax is tmux-compatible. Example ~/.psmux.conf:

        # Change prefix to Ctrl+a
        set -g prefix C-a

        # Use a different shell
        set -g default-shell "C:/Program Files/PowerShell/7/pwsh.exe"
        # or: set -g default-command pwsh
        # or: set -g default-command cmd

        # Status bar
        set -g status-left "[#S] "
        set -g status-right "%H:%M %d-%b-%y"
        set -g status-style "bg=green,fg=black"

        # Key bindings
        bind-key -T prefix h split-window -h
        bind-key -T prefix v split-window -v

SHELL CONFIGURATION:
    psmux launches PowerShell 7 (pwsh) by default. To change:

    Use cmd.exe:
        set -g default-shell cmd
        set -g default-command "cmd /K"

    Use PowerShell 5 (Windows built-in):
        set -g default-shell powershell

    Use PowerShell 7 (pwsh):
        set -g default-shell pwsh

    Use Git Bash:
        set -g default-shell "C:/Program Files/Git/bin/bash.exe"

    Use Nushell:
        set -g default-shell nu

    Launch a window with a specific command:
        psmux new-window -- cmd /K echo hello
        psmux new-session -- python

SET OPTIONS (use with: set -g <option> <value>):
    prefix              Key  Prefix key (default: C-b)
    base-index          Int  First window number (default: 1)
    pane-base-index     Int  First pane number (default: 0)
    escape-time         Int  Escape delay in ms (default: 500)
    repeat-time         Int  Repeat key timeout in ms (default: 500)
    history-limit       Int  Scrollback lines (default: 2000)
    display-time        Int  Message display time in ms (default: 750)
    display-panes-time  Int  Pane number display time in ms (default: 1000)
    status-interval     Int  Status refresh interval in sec (default: 15)
    mouse               Bool Mouse support (default: on)
    status              Bool Show status bar (default: on)
    status-position     Str  "top" or "bottom" (default: bottom)
    focus-events        Bool Pass focus events to apps (default: off)
    mode-keys           Str  "vi" or "emacs" (default: emacs)
    renumber-windows    Bool Auto-renumber on close (default: off)
    automatic-rename    Bool Auto-rename from foreground process (default: on)
    monitor-activity    Bool Flag windows with new output (default: off)
    monitor-silence     Int  Seconds before silence flag (default: 0)
    synchronize-panes   Bool Send input to all panes (default: off)
    remain-on-exit      Bool Keep panes after process exits (default: off)
    aggressive-resize   Bool Resize to smallest client (default: off)
    set-titles          Bool Update terminal title (default: off)
    set-titles-string   Str  Terminal title format
    default-shell       Str  Shell to launch (default: pwsh)
    default-command     Str  Alias for default-shell
    word-separators     Str  Copy-mode word delimiters (default: " -_@")
    prediction-dimming  Bool Dim predictive text (default: on)
    cursor-style        Str  Cursor shape: block, underline, bar
    cursor-blink        Bool Cursor blinking (default: off)
    bell-action         Str  Bell handling: any, none, current, other
    visual-bell         Bool Visual bell indicator (default: off)

    STATUS / STYLE OPTIONS:
    status-left         Str  Left status content (default: "[#S] ")
    status-right        Str  Right status content
    status-style        Str  Status bar style (default: bg=green,fg=black)
    status-bg           Str  Status background color (deprecated, use status-style)
    status-fg           Str  Status foreground color (deprecated, use status-style)
    status-left-style   Str  Left status area style
    status-right-style  Str  Right status area style
    status-justify      Str  Tab alignment: left, centre, right
    message-style       Str  Message bar style
    message-command-style Str Command prompt style
    mode-style          Str  Copy-mode highlight style
    pane-border-style   Str  Inactive pane border style
    pane-active-border-style Str Active pane border style
    pane-border-hover-style Str Border hover highlight style
    window-status-format        Str  Inactive window tab format
    window-status-current-format Str  Active window tab format
    window-status-separator     Str  Separator between tabs
    window-status-style         Str  Inactive tab style
    window-status-current-style Str  Active tab style
    window-status-activity-style Str Activity tab style
    window-status-bell-style    Str  Bell tab style
    window-status-last-style    Str  Last-active tab style

    Style format: "fg=colour,bg=colour,bold,dim,underscore,italics,reverse"
    Colours: default, black, red, green, yellow, blue, magenta, cyan, white,
             colour0-colour255, #RRGGBB

FORMAT VARIABLES (use in status-left, status-right, display-message, etc.):
    #S  session_name          #I  window_index
    #W  window_name           #F  window_flags
    #P  pane_index            #T  pane_title
    #D  pane_id               #H  hostname
    #h  host_short

    Conditionals:  #{{?window_active,yes,no}}
    Comparison:    #{{==:#I,1}}  #{{!=:#W,bash}}
    Substitution:  #{{s/old/new/:variable}}
    Truncation:    #{{=20:variable}}
    Basename:      #{{b:pane_current_path}}
    Dirname:       #{{d:pane_current_path}}
    Literal:       #{{l:text}}

KEY BINDINGS (default prefix: Ctrl+B):
    prefix + c          Create new window
    prefix + n          Next window
    prefix + p          Previous window
    prefix + l          Last window
    prefix + "          Split pane top/bottom
    prefix + %          Split pane left/right
    prefix + o          Switch to next pane
    prefix + ;          Last pane
    prefix + x          Kill current pane
    prefix + &          Kill current window
    prefix + z          Toggle pane zoom
    prefix + {{          Swap pane up
    prefix + }}          Swap pane down
    prefix + !          Break pane to new window
    prefix + d          Detach from session
    prefix + [          Enter copy/scroll mode
    prefix + ]          Paste from buffer
    prefix + =          Buffer chooser
    prefix + :          Enter command mode
    prefix + ?          List keybindings
    prefix + ,          Rename current window
    prefix + '          Select window by index
    prefix + $          Rename session
    prefix + w          Window/pane chooser
    prefix + s          Session chooser
    prefix + q          Display pane numbers
    prefix + i          Display pane info
    prefix + t          Clock mode
    prefix + Space      Next layout
    prefix + Arrow      Navigate between panes
    prefix + 0-9        Select window by number
    prefix + M-1..5     Preset layouts
    prefix + C-Arrow    Resize pane by 1
    prefix + M-Arrow    Resize pane by 5

COPY MODE KEYS (prefix + [):
    ↑/k  Scroll up         ↓/j  Scroll down
    PgUp/b  Page up        PgDn/f  Page down
    g  Top of scrollback   G  Bottom
    ←/h  Cursor left       →/l  Cursor right
    w/W  Next word          b/B  Previous word
    0  Start of line       $  End of line
    ^  First non-blank     H/M/L  Top/Mid/Bot
    f/F  Find char fwd/bwd t/T  Till char fwd/bwd
    %  Matching bracket    {{/}}  Prev/next paragraph
    /  Search forward      ?  Search backward
    n  Next match          N  Previous match
    v  Rectangle toggle    V  Line selection
    Space  Begin selection y/Enter  Yank (copy)
    D  Copy to end of line "a-z  Named registers
    o  Swap cursor/anchor  1-9  Numeric prefix
    q/Esc  Exit copy mode

ENVIRONMENT VARIABLES:
    PSMUX_SESSION_NAME       Default session name
    PSMUX_DEFAULT_SESSION    Fallback default session name
    PSMUX_CURSOR_STYLE       Cursor style (block, underline, bar)
    PSMUX_CURSOR_BLINK       Cursor blinking (1/0)
    PSMUX_DIM_PREDICTIONS    Prediction dimming (1 to enable)
    TMUX                     Set inside psmux panes (tmux-compatible)
    TMUX_PANE                Current pane ID (e.g. %1)

EXAMPLES:
    {prog}                          Start or attach to default session
    {prog} new -s work              Create session named "work"
    {prog} new -s dev -- cmd /K     Create session running cmd.exe
    {prog} new -s py -- python      Create session running Python REPL
    {prog} attach -t work           Attach to session "work"
    {prog} ls                       List all sessions
    {prog} split-window -h          Split pane side by side
    {prog} send-keys -t %1 "ls" Enter
                                    Send keystrokes to pane %1
    {prog} set -g default-shell cmd Use cmd.exe as default shell
    {prog} source-file ~/.psmux.conf Reload config

NOTE: psmux ships as 'psmux', 'pmux', and 'tmux' - use whichever you prefer!

For more information: https://github.com/psmux/psmux
"#, prog = prog, ver = VERSION);
}

pub fn print_version() {
    let prog = get_program_name();
    println!("{} {}", prog, VERSION);
}
