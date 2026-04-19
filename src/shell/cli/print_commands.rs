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

pub fn print_commands() {
    println!(r#"Available commands:
  attach-session (attach)   - Attach to a session
  bind-key (bind)           - Bind a key to a command
  break-pane                - Break a pane into a new window
  capture-pane              - Capture the contents of a pane
  choose-buffer (chooseb)   - Choose a paste buffer interactively
  choose-tree               - Choose a session, window or pane from a tree
  clear-history (clearhist) - Clear pane scrollback history
  clock-mode                - Display a large clock in current pane
  confirm-before (confirm)  - Run command after confirmation
  copy-mode                 - Enter copy mode
  delete-buffer             - Delete a paste buffer
  detach-client (detach)    - Detach from the current session
  display-menu (menu)       - Display a menu
  display-message           - Display a message in the status line
  display-panes             - Display pane numbers
  display-popup (popup)     - Display a popup window
  find-window (findw)       - Search for a window by name
  has-session               - Check if a session exists
  if-shell (if)             - Conditional command execution
  join-pane                 - Join a pane to a window
  kill-pane                 - Kill a pane
  kill-server               - Kill the psmux server
  kill-session              - Kill a session
  kill-window               - Kill a window
  last-pane                 - Select the previously active pane
  last-window               - Select the previously active window
  link-window (linkw)       - Link a window to another session
  list-buffers (lsb)        - List paste buffers
  list-clients (lsc)        - List connected clients
  list-commands (lscm)      - List commands
  list-keys (lsk)           - List key bindings
  list-panes (lsp)          - List panes in a window
  list-sessions (ls)        - List sessions
  list-windows (lsw)        - List windows in a session
  load-buffer (loadb)       - Load buffer from file
  lock-client (lockc)       - Lock the client
  move-pane (movep)         - Move a pane to another window
  move-window (movew)       - Move a window to a different index
  new-session (new)         - Create a new session
  new-window (neww)         - Create a new window
  next-layout (nextl)       - Cycle to next layout
  next-window (next)        - Move to the next window
  paste-buffer              - Paste from a buffer
  pipe-pane (pipep)         - Pipe pane output to a command
  previous-window (prev)    - Move to the previous window
  refresh-client (refresh)  - Refresh client display
  rename-session            - Rename a session
  rename-window (renamew)   - Rename a window
  resize-pane (resizep)     - Resize a pane
  respawn-pane              - Respawn a pane
  rotate-window (rotatew)   - Rotate panes in a window
  run-shell (run)           - Run a shell command
  save-buffer (saveb)       - Save buffer to file
  select-layout (selectl)   - Apply a layout preset
  select-pane (selectp)     - Select a pane
  select-window (selectw)   - Select a window
  send-keys                 - Send keys to a pane
  set-buffer (setb)         - Set a paste buffer
  set-environment (setenv)  - Set an environment variable
  set-hook                  - Set a hook command
  set-option (set)          - Set a session or window option
  show-buffer (showb)       - Display the contents of a paste buffer
  show-environment (showenv)- Show environment variables
  show-hooks                - Show defined hooks
  show-options (show)       - Show session or window options
  show-window-options (showw)- Show window options
  source-file (source)      - Execute commands from a file
  split-window (splitw)     - Split a window into panes
  start-server (warmup)     - Pre-spawn a warm server for instant session creation
  suspend-client (suspendc) - Suspend the client
  swap-pane (swapp)         - Swap two panes
  swap-window (swapw)       - Swap two windows
  switch-client (switchc)   - Switch to another session
  unbind-key (unbind)       - Unbind a key
  unlink-window (unlinkw)   - Unlink a window
  wait-for (wait)           - Wait for a signal
  zoom-pane (zoom)          - Toggle pane zoom
"#);
}

/// Parse a tmux-style target specification
pub fn parse_target(target: &str) -> ParsedTarget {
    let mut result = ParsedTarget::default();
    
    if target.starts_with('%') {
        if let Ok(pid) = target[1..].parse::<usize>() {
            result.pane = Some(pid);
            result.pane_is_id = true;
        }
        return result;
    }
    if target.starts_with('@') {
        if let Ok(wid) = target[1..].parse::<usize>() {
            result.window = Some(wid);
            result.window_is_id = true;
        }
        return result;
    }
    
    let (session_part, window_pane_part) = if let Some(colon_pos) = target.find(':') {
        let session = if colon_pos == 0 { None } else { Some(target[..colon_pos].to_string()) };
        (session, Some(&target[colon_pos + 1..]))
    } else if target.starts_with('.') {
        (None, Some(target))
    } else if let Some(dot_pos) = target.find('.') {
        // Handle tmux-style session.pane syntax (e.g., "default.1")
        // Only treat as session.pane if the part after the dot is numeric
        let after_dot = &target[dot_pos + 1..];
        if after_dot.parse::<usize>().is_ok() {
            let session = target[..dot_pos].to_string();
            // Construct ".pane" so the window_pane_part parser handles it
            (Some(session), Some(&target[dot_pos..]))
        } else {
            // Dot is part of the session name (e.g., "my.session")
            (Some(target.to_string()), None)
        }
    } else {
        // A bare string without ':' or '.' is always a session name, even if numeric.
        // Window/pane specifiers require explicit syntax like ":0" or ".1"
        (Some(target.to_string()), None)
    };
    
    result.session = session_part;
    
    if let Some(wp) = window_pane_part {
        if wp.starts_with('%') {
            if let Ok(pid) = wp[1..].parse::<usize>() {
                result.pane = Some(pid);
                result.pane_is_id = true;
            }
        } else if wp.starts_with('@') {
            if let Ok(wid) = wp[1..].parse::<usize>() {
                result.window = Some(wid);
                result.window_is_id = true;
            }
        } else if let Some(dot_pos) = wp.find('.') {
            if dot_pos > 0 {
                let win_part = &wp[..dot_pos];
                if let Ok(w) = win_part.parse::<usize>() {
                    result.window = Some(w);
                } else if !win_part.is_empty() {
                    result.window_name = Some(win_part.to_string());
                }
            }
            if let Ok(p) = wp[dot_pos + 1..].parse::<usize>() {
                result.pane = Some(p);
            }
        } else {
            if let Ok(w) = wp.parse::<usize>() {
                result.window = Some(w);
            } else if !wp.is_empty() {
                result.window_name = Some(wp.to_string());
            }
        }
    }
    
    result
}

/// Extract the session name from a target string (for port file lookup)
pub fn extract_session_from_target(target: &str) -> String {
    let parsed = parse_target(target);
    parsed.session.unwrap_or_else(|| "default".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_window_name() {
        let pt = parse_target("mysession:mywindow");
        assert_eq!(pt.session, Some("mysession".to_string()));
        assert_eq!(pt.window, None);
        assert_eq!(pt.window_name, Some("mywindow".to_string()));
    }

    #[test]
    fn parse_target_window_index() {
        let pt = parse_target("mysession:2");
        assert_eq!(pt.session, Some("mysession".to_string()));
        assert_eq!(pt.window, Some(2));
        assert_eq!(pt.window_name, None);
    }

    #[test]
    fn parse_target_window_name_with_pane() {
        let pt = parse_target("mysession:mywindow.1");
        assert_eq!(pt.session, Some("mysession".to_string()));
        assert_eq!(pt.window, None);
        assert_eq!(pt.window_name, Some("mywindow".to_string()));
        assert_eq!(pt.pane, Some(1));
    }

    #[test]
    fn parse_target_bare_window_name() {
        // :mywindow (no session)
        let pt = parse_target(":mywindow");
        assert_eq!(pt.session, None);
        assert_eq!(pt.window, None);
        assert_eq!(pt.window_name, Some("mywindow".to_string()));
    }

    #[test]
    fn parse_target_bare_window_index() {
        let pt = parse_target(":3");
        assert_eq!(pt.session, None);
        assert_eq!(pt.window, Some(3));
        assert_eq!(pt.window_name, None);
    }

    #[test]
    fn parse_target_session_only() {
        let pt = parse_target("mysession");
        assert_eq!(pt.session, Some("mysession".to_string()));
        assert_eq!(pt.window, None);
        assert_eq!(pt.window_name, None);
    }
}

#[cfg(test)]
#[path = "../../../tests-rs/test_issue196_flag_equals.rs"]
mod tests_issue196_flag_equals;
