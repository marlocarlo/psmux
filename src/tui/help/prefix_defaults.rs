#[allow(unused_imports)]
// ── src/help.rs ───────────────────────────────────────────────────────
// Comprehensive help / reference data for the C-b ? overlay and
// `list-keys` CLI command.  Kept as a standalone module so it does not
// bloat existing source files.
// ─────────────────────────────────────────────────────────────────────

/// Default prefix-table keybindings.
/// Each entry is `(key_string, command_string)`.
/// The overlay and `list-keys` both use this as the canonical source
/// of truth, so there is exactly *one* place to update.
use super::*;

pub const PREFIX_DEFAULTS: &[(&str, &str)] = &[
    // ── Window management ──
    ("c",       "new-window"),
    ("n",       "next-window"),
    ("p",       "previous-window"),
    ("l",       "last-window"),
    ("w",       "choose-tree"),
    ("&",       "kill-window"),
    (",",       "rename-window"),
    ("'",       "select-window-index"),
    ("0",       "select-window -t :0"),
    ("1",       "select-window -t :1"),
    ("2",       "select-window -t :2"),
    ("3",       "select-window -t :3"),
    ("4",       "select-window -t :4"),
    ("5",       "select-window -t :5"),
    ("6",       "select-window -t :6"),
    ("7",       "select-window -t :7"),
    ("8",       "select-window -t :8"),
    ("9",       "select-window -t :9"),

    // ── Pane splitting ──
    ("%",       "split-window -h"),
    ("\"",      "split-window -v"),

    // ── Pane navigation ──
    ("Up",      "select-pane -U"),
    ("Down",    "select-pane -D"),
    ("Left",    "select-pane -L"),
    ("Right",   "select-pane -R"),
    ("o",       "select-pane -t +"),
    (";",       "last-pane"),
    ("q",       "display-panes"),

    // ── Pane management ──
    ("x",       "kill-pane"),
    ("z",       "resize-pane -Z"),
    ("{",       "swap-pane -U"),
    ("}",       "swap-pane -D"),
    ("!",       "break-pane"),

    // ── Pane resize (Ctrl+Arrow = 1 cell) ──
    ("C-Up",    "resize-pane -U"),
    ("C-Down",  "resize-pane -D"),
    ("C-Left",  "resize-pane -L"),
    ("C-Right", "resize-pane -R"),

    // ── Pane resize (Alt+Arrow = 5 cells) ──
    ("M-Up",    "resize-pane -U 5"),
    ("M-Down",  "resize-pane -D 5"),
    ("M-Left",  "resize-pane -L 5"),
    ("M-Right", "resize-pane -R 5"),

    // ── Layout ──
    ("Space",   "next-layout"),
    ("M-1",     "select-layout even-horizontal"),
    ("M-2",     "select-layout even-vertical"),
    ("M-3",     "select-layout main-horizontal"),
    ("M-4",     "select-layout main-vertical"),
    ("M-5",     "select-layout tiled"),

    // ── Session ──
    ("d",       "detach-client"),
    ("$",       "rename-session"),

    // ── Copy / Paste ──
    ("[",       "copy-mode"),
    ("]",       "paste-buffer"),
    ("=",       "choose-buffer"),
    ("#",       "list-buffers"),

    // ── Misc ──
    (":",       "command-prompt"),
    ("?",       "list-keys"),
    ("i",       "display-message"),
    ("t",       "clock-mode"),
    ("s",       "choose-session"),
    ("(",       "switch-client -p"),
    (")",       "switch-client -n"),
    ("v",       "rectangle-toggle"),
    ("y",       "copy-yank"),
];

// ─────────────────────────────────────────────────────────────────────
// Sections below are used *only* by the overlay — they don't affect
// key dispatching at all (that lives in input.rs).
// ─────────────────────────────────────────────────────────────────────

/// Section header + lines for copy-mode (vi) keybindings shown in the
/// overlay.
pub fn copy_mode_vi_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── copy-mode-vi ──────────────────────────────────────────".into());
    for (k, desc) in COPY_MODE_VI {
        v.push(format!("bind-key -T copy-mode-vi {} {}", k, desc));
    }
    v
}

pub(crate) const COPY_MODE_VI: &[(&str, &str)] = &[
    // Exit
    ("Escape",    "cancel (exit copy mode)"),
    ("q",         "cancel (exit copy mode)"),
    // Cursor movement
    ("h",         "cursor-left"),
    ("j",         "cursor-down"),
    ("k",         "cursor-up"),
    ("l",         "cursor-right"),
    ("Left",      "cursor-left"),
    ("Down",      "cursor-down"),
    ("Up",        "cursor-up"),
    ("Right",     "cursor-right"),
    // Words
    ("w",         "next-word"),
    ("b",         "previous-word"),
    ("e",         "next-word-end"),
    ("W",         "next-space"),
    ("B",         "previous-space"),
    ("E",         "next-space-end"),
    // Line
    ("0",         "start-of-line"),
    ("$",         "end-of-line"),
    ("^",         "back-to-indentation"),
    ("Home",      "start-of-line"),
    ("End",       "end-of-line"),
    // Scrolling
    ("C-u",       "halfpage-up"),
    ("C-d",       "halfpage-down"),
    ("C-b",       "page-up"),
    ("C-f",       "page-down"),
    ("PageUp",    "page-up"),
    ("PageDown",  "page-down"),
    // Document
    ("g",         "history-top"),
    ("G",         "history-bottom"),
    // Screen position
    ("H",         "top-line"),
    ("M",         "middle-line"),
    ("L",         "bottom-line"),
    // Find char
    ("f{char}",   "jump-forward"),
    ("F{char}",   "jump-backward"),
    ("t{char}",   "jump-to-forward"),
    ("T{char}",   "jump-to-backward"),
    // Bracket / paragraph
    ("%",         "next-matching-bracket"),
    ("{",         "previous-paragraph"),
    ("}",         "next-paragraph"),
    // Selection
    ("v",         "rectangle-toggle"),
    ("V",         "select-line"),
    ("C-v",       "rectangle-toggle"),
    ("Space",     "begin-selection"),
    ("o",         "other-end (swap cursor/anchor)"),
    // Yank
    ("y",         "copy-selection-and-cancel"),
    ("Enter",     "copy-selection-and-cancel"),
    ("D",         "copy-end-of-line-and-cancel"),
    ("A",         "append-selection-and-cancel"),
    // Search
    ("/",         "search-forward"),
    ("?",         "search-backward"),
    ("n",         "search-again"),
    ("N",         "search-reverse"),
    // Registers / text objects
    ("\"{a-z}",   "set register for next yank"),
    ("aw",        "select-word (a word)"),
    ("iw",        "select-word (inner word)"),
    // Count prefix
    ("1-9",       "numeric prefix for motions"),
];

/// Section for copy-mode search bindings.
pub fn copy_search_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── copy-mode search ──────────────────────────────────────".into());
    for (k, desc) in COPY_SEARCH {
        v.push(format!("bind-key -T copy-mode-search {} {}", k, desc));
    }
    v
}

pub(crate) const COPY_SEARCH: &[(&str, &str)] = &[
    ("Escape",    "cancel search"),
    ("Enter",     "accept search / jump to match"),
    ("Backspace", "delete character"),
    ("{char}",    "append character to search pattern"),
];

/// Section for command-prompt bindings.
pub fn command_prompt_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── command-prompt ─────────────────────────────────────────".into());
    for (k, desc) in COMMAND_PROMPT {
        v.push(format!("  {} {}", k, desc));
    }
    v
}

pub(crate) const COMMAND_PROMPT: &[(&str, &str)] = &[
    ("Escape",    "cancel"),
    ("Enter",     "execute command (saved to history)"),
    ("Backspace", "delete char before cursor"),
    ("Delete",    "delete char at cursor"),
    ("Left",      "move cursor left"),
    ("Right",     "move cursor right"),
    ("Home",      "move cursor to start"),
    ("End",       "move cursor to end"),
    ("Up",        "history: older command"),
    ("Down",      "history: newer command"),
    ("C-a",       "move cursor to start"),
    ("C-e",       "move cursor to end"),
    ("C-u",       "kill line (clear to start)"),
    ("C-k",       "kill to end of line"),
    ("C-w",       "delete word backwards"),
];

/// Section: CLI command quick-reference (user-facing commands only).
pub fn cli_command_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── commands ───────────────────────────────────────────────".into());
    v.push("  (alias)               description".into());
    for (name, alias, desc) in CLI_COMMANDS {
        if alias.is_empty() {
            v.push(format!("  {:<24}{}", name, desc));
        } else {
            v.push(format!("  {:<13}({:<9}) {}", name, alias, desc));
        }
    }
    v
}

/// `(command_name, alias, description)` — only user-facing commands.
pub(crate) const CLI_COMMANDS: &[(&str, &str, &str)] = &[
    // Session
    ("attach-session",    "attach",   "Attach to an existing session"),
    ("detach-client",     "detach",   "Detach from the current session"),
    ("has-session",       "has",      "Check if a session exists"),
    ("kill-server",       "",         "Kill the server and all sessions"),
    ("kill-session",      "",         "Destroy a session"),
    ("list-sessions",     "ls",       "List sessions"),
    ("new-session",       "new",      "Create a new session"),
    ("rename-session",    "rename",   "Rename the current session"),
    ("switch-client",     "switchc",  "Switch to another session"),
    // Window
    ("choose-tree",       "",         "Interactive session/window chooser"),
    ("find-window",       "findw",    "Search for a window by name"),
    ("kill-window",       "killw",    "Destroy the current window"),
    ("last-window",       "last",     "Select the previous window"),
    ("link-window",       "linkw",    "Link window into another session"),
    ("list-windows",      "lsw",      "List windows"),
    ("move-window",       "movew",    "Move window to another index"),
    ("new-window",        "neww",     "Create a new window"),
    ("next-window",       "next",     "Move to the next window"),
    ("previous-window",   "prev",     "Move to the previous window"),
    ("rename-window",     "renamew",  "Rename the current window"),
    ("resize-window",     "resizew",  "Resize a window"),
    ("respawn-window",    "respawnw", "Restart the process in a window"),
    ("rotate-window",     "rotatew",  "Rotate pane positions"),
    ("select-window",     "selectw",  "Select a window by index"),
    ("swap-window",       "swapw",    "Swap two windows"),
    ("unlink-window",     "unlinkw",  "Unlink a window from the session"),
    // Pane
    ("break-pane",        "breakp",   "Break pane out to a new window"),
    ("capture-pane",      "capturep", "Capture pane contents to buffer"),
    ("display-panes",     "displayp", "Show pane numbers"),
    ("join-pane",         "joinp",    "Move a pane into another window"),
    ("kill-pane",         "killp",    "Kill the active pane"),
    ("last-pane",         "lastp",    "Select the previously active pane"),
    ("move-pane",         "movep",    "Move a pane to another window"),
    ("pipe-pane",         "pipep",    "Pipe pane output to a command"),
    ("resize-pane",       "resizep",  "Resize a pane (-Z to zoom)"),
    ("respawn-pane",      "respawnp", "Restart the process in a pane"),
    ("select-pane",       "selectp",  "Select/focus a pane"),
    ("split-window",      "splitw",   "Split current pane"),
    ("swap-pane",         "swapp",    "Swap two panes"),
    // Layout
    ("next-layout",       "nextl",    "Cycle to the next layout"),
    ("previous-layout",   "prevl",    "Cycle to the previous layout"),
    ("select-layout",     "selectl",  "Apply a layout preset"),
    // Copy / Paste
    ("choose-buffer",     "chooseb",  "Interactive buffer chooser"),
    ("clear-history",     "clearhist","Clear pane scrollback"),
    ("copy-mode",         "",         "Enter copy mode"),
    ("delete-buffer",     "deleteb",  "Delete a paste buffer"),
    ("list-buffers",      "lsb",      "List paste buffers"),
    ("load-buffer",       "loadb",    "Load buffer from file"),
    ("paste-buffer",      "pasteb",   "Paste buffer into pane"),
    ("save-buffer",       "saveb",    "Save buffer to file"),
    ("set-buffer",        "setb",     "Set a buffer's contents"),
    ("show-buffer",       "showb",    "Show buffer contents"),
    // Key binding
    ("bind-key",          "bind",     "Bind a key to a command"),
    ("list-keys",         "lsk",      "List key bindings"),
    ("unbind-key",        "unbind",   "Unbind a key"),
    // Configuration
    ("set-option",        "set",      "Set a session/server option"),
    ("set-window-option", "setw",     "Set a window option"),
    ("show-options",      "show",     "Show options"),
    ("show-window-options","showw",   "Show window options"),
    ("source-file",       "source",   "Load config file"),
    // Display / Info
    ("clock-mode",        "",         "Show a large clock"),
    ("command-prompt",    "",         "Open the command prompt"),
    ("display-menu",      "menu",     "Display an interactive menu"),
    ("display-message",   "display",  "Display a message / pane info"),
    ("display-popup",     "popup",    "Display a popup window"),
    ("list-commands",     "lscm",     "List available commands"),
    ("server-info",       "info",     "Show server information"),
    // Misc
    ("confirm-before",    "confirm",  "Confirm before running command"),
    ("if-shell",          "if",       "Conditional command execution"),
    ("list-clients",      "lsc",      "List connected clients"),
    ("refresh-client",    "refresh",  "Refresh the client display"),
    ("run-shell",         "run",      "Run a shell command"),
    ("send-keys",         "send",     "Send keys/text to a pane"),
    ("set-environment",   "setenv",   "Set an environment variable"),
    ("set-hook",          "",         "Set a hook on an event"),
    ("show-environment",  "showenv",  "Show environment variables"),
    ("show-hooks",        "",         "Show defined hooks"),
    ("show-messages",     "showmsgs", "Show server message log"),
    ("wait-for",          "wait",     "Wait/signal a named channel"),
];

/// Section: configurable options quick-reference.
pub fn options_lines() -> Vec<String> {
    let mut v = Vec::new();
    v.push(String::new());
    v.push("── options (set-option / set) ──────────────────────────────".into());
    v.push("  option                      default".into());
    for (name, default) in OPTIONS_REF {
        v.push(format!("  {:<30}{}", name, default));
    }
    v
}
