use std::io::{self, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use std::net::TcpListener;
use std::io::Read as _;
use std::io::BufRead as _;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use portable_pty::{CommandBuilder, MasterPty, PtySize, PtySystemSelection};
use ratatui::{prelude::*, widgets::*};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use crossterm::terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute};
use crossterm::cursor::{EnableBlinking, DisableBlinking};
use crossterm::event::EnableMouseCapture;
use crossterm::event::DisableMouseCapture;
use ratatui::style::{Style, Modifier};
use unicode_width::UnicodeWidthStr;
use chrono::Local;
use std::env;
use crossterm::style::Print;
use serde::{Serialize, Deserialize};

struct Pane {
    master: Box<dyn MasterPty>,
    child: Box<dyn portable_pty::Child>,
    term: Arc<Mutex<vt100::Parser>>,
    last_rows: u16,
    last_cols: u16,
    id: usize,
    title: String,
}

#[derive(Clone, Copy, PartialEq)]
enum LayoutKind { Horizontal, Vertical }

enum Node {
    Leaf(Pane),
    Split { kind: LayoutKind, sizes: Vec<u16>, children: Vec<Node> },
}

struct Window {
    root: Node,
    active_path: Vec<usize>,
    name: String,
    id: usize,
}

/// A menu item for display-menu
#[derive(Clone)]
struct MenuItem {
    name: String,
    key: Option<char>,
    command: String,
    is_separator: bool,
}

/// A parsed menu structure
#[derive(Clone)]
struct Menu {
    title: String,
    items: Vec<MenuItem>,
    selected: usize,
    x: Option<i16>,
    y: Option<i16>,
}

/// Hook definition - command to run on certain events
#[derive(Clone)]
struct Hook {
    name: String,
    command: String,
}

/// Pipe pane state - process piping pane output
struct PipePaneState {
    pane_id: usize,
    process: Option<std::process::Child>,
    stdin: bool,  // -I: pipe stdout of command to pane
    stdout: bool, // -O: pipe pane output to command stdin
}

/// Wait-for channel state
struct WaitChannel {
    locked: bool,
    waiters: Vec<mpsc::Sender<()>>,
}

enum Mode {
    Passthrough,
    Prefix { armed_at: Instant },
    CommandPrompt { input: String },
    WindowChooser { selected: usize },
    RenamePrompt { input: String },
    CopyMode,
    PaneChooser { opened_at: Instant },
    /// Interactive menu mode
    MenuMode { menu: Menu },
    /// Popup window running a command
    PopupMode { 
        command: String, 
        output: String, 
        process: Option<std::process::Child>,
        width: u16,
        height: u16,
        close_on_exit: bool,
    },
    /// Confirmation prompt before command
    ConfirmMode { 
        prompt: String, 
        command: String,
        input: String,
    },
}

#[derive(Clone, Copy)]
enum FocusDir { Left, Right, Up, Down }

struct AppState {
    windows: Vec<Window>,
    active_idx: usize,
    mode: Mode,
    escape_time_ms: u64,
    prefix_key: (KeyCode, KeyModifiers),
    drag: Option<DragState>,
    last_window_area: Rect,
    mouse_enabled: bool,
    paste_buffers: Vec<String>,
    status_left: String,
    status_right: String,
    copy_anchor: Option<(u16,u16)>,
    copy_pos: Option<(u16,u16)>,
    display_map: Vec<(usize, Vec<usize>)>,
    binds: Vec<Bind>,
    control_rx: Option<mpsc::Receiver<CtrlReq>>,
    control_port: Option<u16>,
    session_name: String,
    attached_clients: usize,
    created_at: chrono::DateTime<Local>,
    next_win_id: usize,
    next_pane_id: usize,
    zoom_saved: Option<Vec<(Vec<usize>, Vec<u16>)>>,
    sync_input: bool,
    /// Hooks: map of hook name to list of commands
    hooks: std::collections::HashMap<String, Vec<String>>,
    /// Wait-for channels: map of channel name to list of waiting senders
    wait_channels: std::collections::HashMap<String, WaitChannel>,
    /// Pipe pane processes
    pipe_panes: Vec<PipePaneState>,
    /// Last active window index (for last-window command)
    last_window_idx: usize,
    /// Last active pane path (for last-pane command)
    last_pane_path: Vec<usize>,
}

struct DragState {
    split_path: Vec<usize>,
    kind: LayoutKind,
    index: usize,
    start_x: u16,
    start_y: u16,
    left_initial: u16,
    _right_initial: u16,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn get_program_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "psmux".to_string())
        .to_lowercase()
        .replace(".exe", "")
}

fn print_help() {
    let prog = get_program_name();
    println!(r#"{prog} - Terminal multiplexer for Windows (tmux alternative)

USAGE:
    {prog} [COMMAND] [OPTIONS]

COMMANDS:
    (no command)        Start a new session or attach to existing one
    new-session         Create a new session
        -s <name>       Session name (default: "default")
        -d              Start detached (in background)
    attach, attach-session
                        Attach to an existing session
        -t <name>       Target session name
    ls, list-sessions   List all active sessions
    new-window          Create a new window in current session
    split-window        Split current pane
        -h              Split horizontally (side by side)
        -v              Split vertically (top/bottom, default)
    kill-pane           Close the current pane
    capture-pane        Capture the content of current pane
    server              Run as a server (internal use)
    help                Show this help message
    version             Show version information

OPTIONS:
    -h, --help          Show this help message
    -V, --version       Show version information

KEY BINDINGS (default prefix: Ctrl+B):
    prefix + c          Create new window
    prefix + n          Next window
    prefix + p          Previous window
    prefix + "          Split pane horizontally
    prefix + %          Split pane vertically
    prefix + o          Switch to next pane
    prefix + x          Kill current pane
    prefix + d          Detach from session
    prefix + [          Enter copy mode
    prefix + :          Enter command mode
    prefix + ,          Rename current window
    prefix + w          Window chooser
    prefix + q          Display pane numbers

ENVIRONMENT VARIABLES:
    PSMUX_SESSION_NAME       Default session name
    PSMUX_DEFAULT_SESSION    Fallback default session name
    PSMUX_CURSOR_STYLE       Cursor style (block, underline, bar)
    PSMUX_CURSOR_BLINK       Cursor blinking (true/false)

CONFIG FILES:
    %USERPROFILE%\.psmux.conf     Main configuration file
    %USERPROFILE%\.psmuxrc        Alternative configuration file

EXAMPLES:
    {prog}                          Start or attach to default session
    {prog} new-session -s work      Create a new session named "work"
    {prog} attach -t work           Attach to session "work"
    {prog} ls                       List all sessions
    {prog} split-window -h          Split current pane horizontally

NOTE: psmux includes 'tmux' and 'pmux' aliases - use any command you prefer!

For more information: https://github.com/marlocarlo/psmux
"#, prog = prog);
}

fn print_version() {
    let prog = get_program_name();
    println!("{} {}", prog, VERSION);
}

fn print_commands() {
    println!(r#"Available commands:
  attach-session (attach)   - Attach to a session
  bind-key (bind)           - Bind a key to a command
  break-pane                - Break a pane into a new window
  capture-pane              - Capture the contents of a pane
  choose-tree               - Choose a session, window or pane from a tree
  clear-history (clearhist) - Clear pane scrollback history
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
  source-file (source)      - Execute commands from a file
  split-window (splitw)     - Split a window into panes
  start-server              - Start the psmux server
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

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    
    // Parse -t flag early to set target session for all commands
    if let Some(pos) = args.iter().position(|a| a == "-t") {
        if let Some(target) = args.get(pos + 1) {
            env::set_var("PSMUX_TARGET_SESSION", target);
        }
    }
    
    // Find the actual command by skipping -t and its argument
    let cmd_args: Vec<&String> = args.iter().skip(1).filter(|a| {
        if *a == "-t" { return false; }
        // Check if previous arg was -t
        if let Some(pos) = args.iter().position(|x| x == *a) {
            if pos > 0 && args[pos - 1] == "-t" { return false; }
        }
        true
    }).collect();
    
    let cmd = cmd_args.first().map(|s| s.as_str()).unwrap_or("");
    
    // Handle help and version flags first
    match cmd {
        "-h" | "--help" | "help" => {
            print_help();
            return Ok(());
        }
        "-V" | "--version" | "version" => {
            print_version();
            return Ok(());
        }
        "list-commands" | "lscm" => {
            print_commands();
            return Ok(());
        }
        _ => {}
    }
    
    match cmd {
        "ls" | "list-sessions" => {
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let dir = format!("{}\\.psmux", home);
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        if let Some(name) = e.file_name().to_str() {
                            if let Some((base, ext)) = name.rsplit_once('.') {
                                if ext == "port" {
                                    if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                                        if let Ok(_p) = port_str.trim().parse::<u16>() {
                                            let addr = format!("127.0.0.1:{}", port_str.trim());
                                            if let Ok(mut s) = std::net::TcpStream::connect_timeout(
                                                &addr.parse().unwrap(),
                                                Duration::from_millis(500)
                                            ) {
                                                let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                                                let _ = std::io::Write::write_all(&mut s, b"session-info\n");
                                                let mut br = std::io::BufReader::new(s);
                                                let mut line = String::new();
                                                let _ = br.read_line(&mut line);
                                                if !line.trim().is_empty() { println!("{}", line.trim_end()); } else { println!("{}", base); }
                                            } else {
                                                // stale port file - remove it
                                                let _ = std::fs::remove_file(e.path());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                return Ok(());
            }
            "attach" | "attach-session" => {
                let name = args
                    .iter()
                    .position(|a| a == "-t")
                    .and_then(|i| args.get(i + 1))
                    .map(|s| s.clone())
                    .or_else(resolve_default_session_name)
                    .or_else(resolve_last_session_name)
                    .unwrap_or_else(|| "default".to_string());
                env::set_var("PSMUX_SESSION_NAME", name);
                env::set_var("PSMUX_REMOTE_ATTACH", "1");
            }
            "server" => {
                // Internal command - run headless server (used when spawning background server)
                let name = args.iter().position(|a| a == "-s").and_then(|i| args.get(i+1)).map(|s| s.clone()).unwrap_or_else(|| "default".to_string());
                return run_server(name);
            }
            "new-session" | "new" => {
                let name = args.iter().position(|a| a == "-s").and_then(|i| args.get(i+1)).map(|s| s.clone()).unwrap_or_else(|| "default".to_string());
                let detached = args.iter().any(|a| a == "-d");
                
                // Check if session already exists
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let port_path = format!("{}\\.psmux\\{}.port", home, name);
                if std::path::Path::new(&port_path).exists() {
                    eprintln!("psmux: session '{}' already exists", name);
                    return Ok(());
                }
                
                // Always spawn a background server first
                let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
                let mut cmd = std::process::Command::new(&exe);
                cmd.arg("server").arg("-s").arg(&name);
                // On Windows, use CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW to run in background
                // DETACHED_PROCESS causes PTY issues
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
                }
                let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn server: {e}")))?;
                
                // Wait for server to create port file (up to 2 seconds)
                for _ in 0..20 {
                    if std::path::Path::new(&port_path).exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                
                if detached {
                    // User wants detached session - we're done
                    return Ok(());
                } else {
                    // User wants attached session - set env vars to attach
                    env::set_var("PSMUX_SESSION_NAME", name);
                    env::set_var("PSMUX_REMOTE_ATTACH", "1");
                    // Continue to attach below...
                }
            }
            "new-window" => { send_control("new-window\n".to_string())?; return Ok(()); }
            "split-window" => {
                let flag = if args.iter().any(|a| a == "-h") { "-h" } else { "-v" };
                send_control(format!("split-window {}\n", flag))?; return Ok(());
            }
            "kill-pane" => { send_control("kill-pane\n".to_string())?; return Ok(()); }
            "capture-pane" => {
                // Parse optional flags - cmd_args[0] is command, start from 1
                let mut cmd = "capture-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(target) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", target));
                                i += 1;
                            }
                        }
                        "-S" => {
                            if let Some(start) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -S {}", start));
                                i += 1;
                            }
                        }
                        "-E" => {
                            if let Some(end) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -E {}", end));
                                i += 1;
                            }
                        }
                        "-p" => { cmd.push_str(" -p"); }
                        "-e" => { cmd.push_str(" -e"); }
                        "-J" => { cmd.push_str(" -J"); }
                        "-b" => {
                            if let Some(buf) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -b {}", buf));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // send-keys - Send keys to a pane (critical for scripting)
            "send-keys" | "send" => {
                let mut literal = false;
                let mut keys: Vec<String> = Vec::new();
                // Skip the command itself (index 0 in cmd_args), start at index 1
                for i in 1..cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-l" => { literal = true; }
                        "-R" => { keys.push("__RESET__".to_string()); }
                        "-t" => { } // already handled
                        _ => { keys.push(cmd_args[i].to_string()); }
                    }
                }
                let mut cmd = "send-keys".to_string();
                if literal { cmd.push_str(" -l"); }
                for k in keys { cmd.push_str(&format!(" {}", k)); }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // select-pane - Select the active pane
            "select-pane" | "selectp" => {
                let mut cmd = "select-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-L" => { cmd.push_str(" -L"); }
                        "-R" => { cmd.push_str(" -R"); }
                        "-l" => { cmd.push_str(" -l"); }
                        "-Z" => { cmd.push_str(" -Z"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // select-window - Select a window
            "select-window" | "selectw" => {
                let mut cmd = "select-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-l" => { cmd.push_str(" -l"); }
                        "-n" => { cmd.push_str(" -n"); }
                        "-p" => { cmd.push_str(" -p"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // list-panes - List all panes
            "list-panes" | "lsp" => {
                let mut cmd = "list-panes".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { cmd.push_str(" -a"); }
                        "-s" => { cmd.push_str(" -s"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // list-windows - List all windows
            "list-windows" | "lsw" => {
                let mut cmd = "list-windows".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { cmd.push_str(" -a"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // kill-window - Kill a window
            "kill-window" | "killw" => {
                let mut cmd = "kill-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-a" => { cmd.push_str(" -a"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // kill-session - Kill a session
            "kill-session" => {
                let mut target: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                target = Some(t.to_string());
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(t) = target {
                    env::set_var("PSMUX_TARGET_SESSION", &t);
                }
                send_control("kill-session\n".to_string())?;
                return Ok(());
            }
            // kill-server - Kill all sessions
            "kill-server" => {
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let psmux_dir = format!("{}\\.psmux", home);
                let mut sessions_killed = 0;
                if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "port").unwrap_or(false) {
                            if let Ok(port_str) = std::fs::read_to_string(&path) {
                                if let Ok(port) = port_str.trim().parse::<u16>() {
                                    let addr = format!("127.0.0.1:{}", port);
                                    // Send kill-session to each server
                                    if let Ok(mut stream) = std::net::TcpStream::connect(&addr) {
                                        let _ = std::io::Write::write_all(&mut stream, b"kill-session\n");
                                        sessions_killed += 1;
                                    } else {
                                        // Server not running, just remove stale port file
                                        let _ = std::fs::remove_file(&path);
                                    }
                                }
                            } else {
                                // Can't read port file, remove it
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                }
                if sessions_killed > 0 {
                    // Give servers a moment to clean up
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                return Ok(());
            }
            // has-session - Check if session exists (for scripting)
            "has-session" | "has" => {
                // Get target from env (set from -t flag) or from remaining args
                let target = env::var("PSMUX_TARGET_SESSION").unwrap_or_else(|_| {
                    // Try to get from cmd_args if -t is in there (shouldn't be, but just in case)
                    let mut t = "default".to_string();
                    let mut i = 1;
                    while i < cmd_args.len() {
                        if cmd_args[i].as_str() == "-t" {
                            if let Some(v) = cmd_args.get(i + 1) { t = v.to_string(); }
                            i += 1;
                        } else if !cmd_args[i].starts_with('-') {
                            t = cmd_args[i].to_string();
                            break;
                        }
                        i += 1;
                    }
                    t
                });
                let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                let path = format!("{}\\.psmux\\{}.port", home, target);
                if let Ok(port_str) = std::fs::read_to_string(&path) {
                    if let Ok(port) = port_str.trim().parse::<u16>() {
                        let addr = format!("127.0.0.1:{}", port);
                        if std::net::TcpStream::connect(addr).is_ok() {
                            std::process::exit(0);
                        }
                    }
                }
                std::process::exit(1);
            }
            // rename-session - Rename a session
            "rename-session" | "rename" => {
                let mut new_name: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    if !cmd_args[i].starts_with('-') {
                        new_name = Some(cmd_args[i].to_string());
                        break;
                    }
                    i += 1;
                }
                if let Some(name) = new_name {
                    send_control(format!("rename-session {}\n", name))?;
                }
                return Ok(());
            }
            // swap-pane - Swap panes
            "swap-pane" | "swapp" => {
                let mut cmd = "swap-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // resize-pane - Resize a pane
            "resize-pane" | "resizep" => {
                let mut cmd = "resize-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-L" => { cmd.push_str(" -L"); }
                        "-R" => { cmd.push_str(" -R"); }
                        "-Z" => { cmd.push_str(" -Z"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-x" => {
                            if let Some(v) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -x {}", v));
                                i += 1;
                            }
                        }
                        "-y" => {
                            if let Some(v) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -y {}", v));
                                i += 1;
                            }
                        }
                        s if s.parse::<i32>().is_ok() => {
                            cmd.push_str(&format!(" {}", s));
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // paste-buffer - Paste buffer into pane
            "paste-buffer" | "pasteb" => {
                let mut cmd = "paste-buffer".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -b {}", b));
                                i += 1;
                            }
                        }
                        "-d" => { cmd.push_str(" -d"); }
                        "-p" => { cmd.push_str(" -p"); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // set-buffer - Set buffer contents
            "set-buffer" | "setb" => {
                let mut buffer_name: Option<String> = None;
                let mut data: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => {
                            data = Some(s.to_string());
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let mut cmd = "set-buffer".to_string();
                if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
                if let Some(d) = data { cmd.push_str(&format!(" {}", d)); }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // list-buffers - List paste buffers
            "list-buffers" | "lsb" => {
                let resp = send_control_with_response("list-buffers\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // show-buffer - Show buffer contents
            "show-buffer" | "showb" => {
                let mut buffer_name: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let mut cmd = "show-buffer".to_string();
                if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // delete-buffer - Delete a paste buffer
            "delete-buffer" | "deleteb" => {
                let mut buffer_name: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let mut cmd = "delete-buffer".to_string();
                if let Some(b) = buffer_name { cmd.push_str(&format!(" -b {}", b)); }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // display-message - Display a message
            "display-message" | "display" => {
                let mut message: Vec<String> = Vec::new();
                let mut target: Option<String> = None;
                let mut print_to_stdout = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                target = Some(t.to_string());
                                i += 1;
                            }
                        }
                        "-p" => { print_to_stdout = true; }
                        s => { message.push(s.to_string()); }
                    }
                    i += 1;
                }
                let msg = message.join(" ");
                let mut cmd = "display-message".to_string();
                if let Some(t) = target { cmd.push_str(&format!(" -t {}", t)); }
                if print_to_stdout { cmd.push_str(" -p"); }
                cmd.push_str(&format!(" {}", msg));
                cmd.push('\n');
                if print_to_stdout {
                    let resp = send_control_with_response(cmd)?;
                    print!("{}", resp);
                } else {
                    send_control(cmd)?;
                }
                return Ok(());
            }
            // run-shell - Run a shell command
            "run-shell" | "run" => {
                let mut cmd_to_run: Vec<String> = Vec::new();
                let mut background = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => { background = true; }
                        s => { cmd_to_run.push(s.to_string()); }
                    }
                    i += 1;
                }
                let shell_cmd = cmd_to_run.join(" ");
                // Run the command using the system shell
                if background {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("cmd")
                            .args(["/C", &shell_cmd])
                            .spawn();
                    }
                } else {
                    #[cfg(windows)]
                    {
                        let output = std::process::Command::new("cmd")
                            .args(["/C", &shell_cmd])
                            .output()?;
                        io::stdout().write_all(&output.stdout)?;
                        io::stderr().write_all(&output.stderr)?;
                        std::process::exit(output.status.code().unwrap_or(0));
                    }
                }
                return Ok(());
            }
            // respawn-pane - Restart the pane's process
            "respawn-pane" | "respawnp" => {
                let mut cmd = "respawn-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-k" => { cmd.push_str(" -k"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // last-window - Select last used window
            "last-window" | "last" => {
                send_control("last-window\n".to_string())?;
                return Ok(());
            }
            // last-pane - Select last used pane
            "last-pane" | "lastp" => {
                send_control("last-pane\n".to_string())?;
                return Ok(());
            }
            // next-window - Move to next window
            "next-window" | "next" => {
                send_control("next-window\n".to_string())?;
                return Ok(());
            }
            // previous-window - Move to previous window
            "previous-window" | "prev" => {
                send_control("previous-window\n".to_string())?;
                return Ok(());
            }
            // rotate-window - Rotate panes in window
            "rotate-window" | "rotatew" => {
                let mut cmd = "rotate-window".to_string();
                let mut i = 1;
                while i < args.len() {
                    match args[i].as_str() {
                        "-D" => { cmd.push_str(" -D"); }
                        "-U" => { cmd.push_str(" -U"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // display-panes - Show pane numbers
            "display-panes" | "displayp" => {
                send_control("display-panes\n".to_string())?;
                return Ok(());
            }
            // break-pane - Break pane out to a new window
            "break-pane" | "breakp" => {
                let mut cmd = "break-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-d" => { cmd.push_str(" -d"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // join-pane - Join a pane to another window
            "join-pane" | "joinp" => {
                let mut cmd = "join-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-h" => { cmd.push_str(" -h"); }
                        "-v" => { cmd.push_str(" -v"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // rename-window - Rename current window
            "rename-window" | "renamew" => {
                // cmd_args[0] is the command, cmd_args[1] should be the new name
                if let Some(name) = cmd_args.get(1) {
                    if !name.starts_with('-') {
                        send_control(format!("rename-window {}\n", name))?;
                    }
                }
                return Ok(());
            }
            // zoom-pane - Toggle pane zoom
            "zoom-pane" | "resizep -Z" => {
                send_control("zoom-pane\n".to_string())?;
                return Ok(());
            }
            // source-file - Load a configuration file
            "source-file" | "source" => {
                let mut quiet = false;
                let mut file_path: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-q" => { quiet = true; }
                        "-n" => { /* parse only, don't execute */ }
                        "-v" => { /* verbose */ }
                        s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(path) = file_path {
                    // Expand ~ to home directory
                    let expanded = if path.starts_with('~') {
                        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                        path.replacen('~', &home, 1)
                    } else {
                        path
                    };
                    if let Err(e) = std::fs::read_to_string(&expanded) {
                        if !quiet {
                            eprintln!("psmux: {}: {}", expanded, e);
                            std::process::exit(1);
                        }
                    } else {
                        // Send source-file command to server if attached
                        send_control(format!("source-file {}\n", expanded))?;
                    }
                }
                return Ok(());
            }
            // list-keys - List all key bindings
            "list-keys" | "lsk" => {
                let resp = send_control_with_response("list-keys\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // bind-key - Bind a key to a command
            "bind-key" | "bind" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                send_control(format!("{}\n", cmd_str))?;
                return Ok(());
            }
            // unbind-key - Unbind a key
            "unbind-key" | "unbind" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                send_control(format!("{}\n", cmd_str))?;
                return Ok(());
            }
            // set-option / set - Set an option
            "set-option" | "set" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                send_control(format!("{}\n", cmd_str))?;
                return Ok(());
            }
            // show-options / show - Show options
            "show-options" | "show" => {
                let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
                let resp = send_control_with_response(format!("{}\n", cmd_str))?;
                print!("{}", resp);
                return Ok(());
            }
            // if-shell - Conditional execution
            "if-shell" | "if" => {
                let mut background = false;
                let mut condition: Option<String> = None;
                let mut cmd_true: Option<String> = None;
                let mut cmd_false: Option<String> = None;
                let mut format_mode = false;
                let mut i = 1;
                
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => { background = true; }
                        "-F" => { format_mode = true; }
                        "-t" => { i += 1; } // Skip target
                        s if !s.starts_with('-') => {
                            if condition.is_none() {
                                condition = Some(s.to_string());
                            } else if cmd_true.is_none() {
                                cmd_true = Some(s.to_string());
                            } else if cmd_false.is_none() {
                                cmd_false = Some(s.to_string());
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                
                if let (Some(cond), Some(true_cmd)) = (condition, cmd_true) {
                    let success = if format_mode {
                        // Treat condition as format string - non-empty and non-zero is true
                        !cond.is_empty() && cond != "0"
                    } else {
                        // Run shell command
                        #[cfg(windows)]
                        {
                            std::process::Command::new("cmd")
                                .args(["/C", &cond])
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false)
                        }
                        #[cfg(not(windows))]
                        {
                            std::process::Command::new("sh")
                                .args(["-c", &cond])
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false)
                        }
                    };
                    
                    let cmd_to_run = if success { Some(true_cmd) } else { cmd_false };
                    
                    if let Some(cmd) = cmd_to_run {
                        if background {
                            // Run in background
                            send_control(format!("{}\n", cmd))?;
                        } else {
                            // Execute as psmux command
                            send_control(format!("{}\n", cmd))?;
                        }
                    }
                }
                return Ok(());
            }
            // wait-for - Wait for a signal
            "wait-for" | "wait" => {
                let mut lock = false;
                let mut signal = false;
                let mut unlock = false;
                let mut channel: Option<String> = None;
                let mut i = 1;
                
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-L" => { lock = true; }
                        "-S" => { signal = true; }
                        "-U" => { unlock = true; }
                        s if !s.starts_with('-') => { channel = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                
                if let Some(ch) = channel {
                    if signal {
                        send_control(format!("wait-for -S {}\n", ch))?;
                    } else if lock {
                        send_control(format!("wait-for -L {}\n", ch))?;
                    } else if unlock {
                        send_control(format!("wait-for -U {}\n", ch))?;
                    } else {
                        // Wait for channel - this blocks
                        let resp = send_control_with_response(format!("wait-for {}\n", ch))?;
                        if !resp.is_empty() {
                            print!("{}", resp);
                        }
                    }
                }
                return Ok(());
            }
            // select-layout - Select a layout for the window
            "select-layout" | "selectl" => {
                let mut layout: Option<String> = None;
                let mut next = false;
                let mut prev = false;
                let mut i = 1;
                
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-n" => { next = true; }
                        "-p" => { prev = true; }
                        "-o" => { /* last layout */ }
                        "-E" => { /* spread evenly */ }
                        "-t" => { i += 1; } // Skip target
                        s if !s.starts_with('-') => { layout = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                
                if next {
                    send_control("next-layout\n".to_string())?;
                } else if prev {
                    send_control("previous-layout\n".to_string())?;
                } else if let Some(l) = layout {
                    send_control(format!("select-layout {}\n", l))?;
                } else {
                    send_control("select-layout\n".to_string())?;
                }
                return Ok(());
            }
            // move-window - Move a window
            "move-window" | "movew" => {
                let mut cmd = "move-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { cmd.push_str(" -a"); }
                        "-b" => { cmd.push_str(" -b"); }
                        "-r" => { cmd.push_str(" -r"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-k" => { cmd.push_str(" -k"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // swap-window - Swap windows
            "swap-window" | "swapw" => {
                let mut cmd = "swap-window".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-d" => { cmd.push_str(" -d"); }
                        "-s" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -s {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // list-clients - List all clients
            "list-clients" | "lsc" => {
                let resp = send_control_with_response("list-clients\n".to_string())?;
                print!("{}", resp);
                return Ok(());
            }
            // switch-client - Switch the current client to another session
            "switch-client" | "switchc" => {
                let mut cmd = "switch-client".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-l" => { cmd.push_str(" -l"); }
                        "-n" => { cmd.push_str(" -n"); }
                        "-p" => { cmd.push_str(" -p"); }
                        "-r" => { cmd.push_str(" -r"); }
                        "-c" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -c {}", t));
                                i += 1;
                            }
                        }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // copy-mode - Enter copy mode
            "copy-mode" => {
                let mut cmd = "copy-mode".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-u" => { cmd.push_str(" -u"); }
                        "-d" => { cmd.push_str(" -d"); }
                        "-e" => { cmd.push_str(" -e"); }
                        "-H" => { cmd.push_str(" -H"); }
                        "-q" => { cmd.push_str(" -q"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // clock-mode - Display a clock
            "clock-mode" => {
                send_control("clock-mode\n".to_string())?;
                return Ok(());
            }
            // set-environment / setenv - Set environment variable
            "set-environment" | "setenv" => {
                let mut cmd = "set-environment".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-g" => { cmd.push_str(" -g"); }
                        "-r" => { cmd.push_str(" -r"); }
                        "-u" => { cmd.push_str(" -u"); }
                        "-h" => { cmd.push_str(" -h"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        s => { cmd.push_str(&format!(" {}", s)); }
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // show-environment / showenv - Show environment variables
            "show-environment" | "showenv" => {
                let mut cmd = "show-environment".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-g" => { cmd.push_str(" -g"); }
                        "-s" => { cmd.push_str(" -s"); }
                        "-h" => { cmd.push_str(" -h"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => { cmd.push_str(&format!(" {}", s)); }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                let resp = send_control_with_response(cmd)?;
                print!("{}", resp);
                return Ok(());
            }
            // load-buffer - Load a paste buffer from a file
            "load-buffer" | "loadb" => {
                let mut buffer_name: Option<String> = None;
                let mut file_path: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(path) = file_path {
                    let content = if path == "-" {
                        let mut input = String::new();
                        io::stdin().read_to_string(&mut input)?;
                        input
                    } else {
                        std::fs::read_to_string(&path)?
                    };
                    let mut cmd = "set-buffer".to_string();
                    if let Some(b) = buffer_name {
                        cmd.push_str(&format!(" -b {}", b));
                    }
                    // Escape the content for transmission
                    let escaped = content.replace('\n', "\\n").replace('\r', "\\r");
                    cmd.push_str(&format!(" {}", escaped));
                    cmd.push('\n');
                    send_control(cmd)?;
                }
                return Ok(());
            }
            // save-buffer - Save a paste buffer to a file
            "save-buffer" | "saveb" => {
                let mut buffer_name: Option<String> = None;
                let mut file_path: Option<String> = None;
                let mut append = false;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-a" => { append = true; }
                        "-b" => {
                            if let Some(b) = cmd_args.get(i + 1) {
                                buffer_name = Some(b.to_string());
                                i += 1;
                            }
                        }
                        s if !s.starts_with('-') => { file_path = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(path) = file_path {
                    let mut cmd = "show-buffer".to_string();
                    if let Some(b) = buffer_name {
                        cmd.push_str(&format!(" -b {}", b));
                    }
                    cmd.push('\n');
                    let content = send_control_with_response(cmd)?;
                    if path == "-" {
                        print!("{}", content);
                    } else if append {
                        use std::fs::OpenOptions;
                        let mut file = OpenOptions::new().append(true).create(true).open(&path)?;
                        file.write_all(content.as_bytes())?;
                    } else {
                        std::fs::write(&path, &content)?;
                    }
                }
                return Ok(());
            }
            // clear-history - Clear pane history
            "clear-history" | "clearhist" => {
                let mut cmd = "clear-history".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-H" => { cmd.push_str(" -H"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // pipe-pane - Pipe pane output to a command
            "pipe-pane" | "pipep" => {
                let mut cmd = "pipe-pane".to_string();
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-I" => { cmd.push_str(" -I"); }
                        "-O" => { cmd.push_str(" -O"); }
                        "-o" => { cmd.push_str(" -o"); }
                        "-t" => {
                            if let Some(t) = cmd_args.get(i + 1) {
                                cmd.push_str(&format!(" -t {}", t));
                                i += 1;
                            }
                        }
                        s => { cmd.push_str(&format!(" {}", s)); }
                    }
                    i += 1;
                }
                cmd.push('\n');
                send_control(cmd)?;
                return Ok(());
            }
            // find-window - Search for a window
            "find-window" | "findw" => {
                let mut pattern: Option<String> = None;
                let mut i = 1;
                while i < cmd_args.len() {
                    match cmd_args[i].as_str() {
                        "-C" | "-N" | "-T" | "-i" | "-r" | "-Z" => {}
                        "-t" => { i += 1; }
                        s if !s.starts_with('-') => { pattern = Some(s.to_string()); }
                        _ => {}
                    }
                    i += 1;
                }
                if let Some(p) = pattern {
                    let resp = send_control_with_response(format!("find-window {}\n", p))?;
                    print!("{}", resp);
                }
                return Ok(());
            }
            // list-commands - List all commands
            "list-commands" | "lscm" => {
                print_commands();
                return Ok(());
            }
            _ => {}
        }
    
    // Default behavior: If no PSMUX_REMOTE_ATTACH is set and no specific command matched,
    // we need to either attach to an existing session or create a new one.
    // This ensures sessions persist after detach.
    if env::var("PSMUX_REMOTE_ATTACH").ok().as_deref() != Some("1") {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        let session_name = env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| "default".to_string());
        let port_path = format!("{}\\.psmux\\{}.port", home, session_name);
        
        // Check if port file exists AND server is actually alive
        let server_alive = if std::path::Path::new(&port_path).exists() {
            if let Ok(port_str) = std::fs::read_to_string(&port_path) {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    let addr = format!("127.0.0.1:{}", port);
                    std::net::TcpStream::connect_timeout(
                        &addr.parse().unwrap(),
                        Duration::from_millis(200)
                    ).is_ok()
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        
        if !server_alive {
            // Clean up stale port file if it exists
            let _ = std::fs::remove_file(&port_path);
            // No existing session - create one in background
            let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg("server").arg("-s").arg(&session_name);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
            }
            let _child = cmd.spawn().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("failed to spawn server: {e}")))?;
            
            // Wait for server to start
            for _ in 0..20 {
                if std::path::Path::new(&port_path).exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        
        // Now attach to the session
        env::set_var("PSMUX_SESSION_NAME", &session_name);
        env::set_var("PSMUX_REMOTE_ATTACH", "1");
    }
    
    if env::var("PSMUX_ACTIVE").ok().as_deref() == Some("1") {
        eprintln!("psmux: nested sessions are not allowed");
        return Ok(());
    }
    env::set_var("PSMUX_ACTIVE", "1");
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableBlinking, EnableMouseCapture)?;
    apply_cursor_style(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    // Loop to handle session switching without spawning new processes
    loop {
        let result = run_remote(&mut terminal);
        
        // Check if we should switch to another session
        if let Ok(switch_to) = env::var("PSMUX_SWITCH_TO") {
            env::remove_var("PSMUX_SWITCH_TO");
            env::set_var("PSMUX_SESSION_NAME", &switch_to);
            // Update last_session file
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            let last_path = format!("{}\\.psmux\\last_session", home);
            let _ = std::fs::write(&last_path, &switch_to);
            // Continue loop to attach to new session
            continue;
        }
        
        // Normal exit
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), DisableBlinking, DisableMouseCapture, LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        return result;
    }
}

fn run_remote(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let name = env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| "default".to_string());
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let path = format!("{}\\.psmux\\{}.port", home, name);
    let port = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u16>().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no session"))?;
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = std::net::TcpStream::connect(addr.clone())?;
    let _ = std::io::Write::write_all(&mut stream, b"client-attach\n");
    let last_path = format!("{}\\.psmux\\last_session", home);
    let _ = std::fs::write(&last_path, &name);
    let mut quit = false;
    let mut prefix_armed = false;
    let mut renaming = false;
    let mut rename_buf = String::new();
    let mut pane_renaming = false;
    let mut pane_title_buf = String::new();
    let mut chooser = false;
    let mut choices: Vec<(usize, usize)> = Vec::new();
    let mut tree_chooser = false;
    let mut tree_entries: Vec<(bool, usize, usize, String)> = Vec::new();
    let mut tree_selected: usize = 0;
    // Session chooser state
    let mut session_chooser = false;
    let mut session_entries: Vec<(String, String)> = Vec::new(); // (session_name, info)
    let mut session_selected: usize = 0;
    let current_session = name.clone();
    
    // Helper to connect to server - returns None if connection fails
    let try_connect = || -> Option<std::net::TcpStream> {
        std::net::TcpStream::connect(&addr).ok()
    };
    
    loop {
        // Check if server is still alive before drawing
        if std::net::TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(20)).is_err() {
            // Server is gone, exit gracefully
            break;
        }
        
        terminal.draw(|f| {
            let area = f.size();
            let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)].as_ref()).split(area);
            // update server with client size
            if let Ok(mut cs) = std::net::TcpStream::connect(addr.clone()) {
                let _ = std::io::Write::write_all(&mut cs, format!("client-size {} {}\n", chunks[0].width, chunks[0].height).as_bytes());
            }
            // fetch layout json
            let root: LayoutJson = if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) {
                let _ = std::io::Write::write_all(&mut s, b"dump-layout\n");
                let mut buf = String::new();
                let _ = std::io::Read::read_to_string(&mut s, &mut buf);
                serde_json::from_str(&buf).unwrap_or(LayoutJson::Leaf { id: 0, rows: 0, cols: 0, cursor_row: 0, cursor_col: 0, content: Vec::new() })
            } else {
                LayoutJson::Leaf { id: 0, rows: 0, cols: 0, cursor_row: 0, cursor_col: 0, content: Vec::new() }
            };

            fn render_json(f: &mut Frame, node: &LayoutJson, area: Rect) {
                match node {
                    LayoutJson::Leaf { id: _, rows: _, cols: _, cursor_row, cursor_col, content } => {
                        let pane_block = Block::default().borders(Borders::ALL);
                        let inner = pane_block.inner(area);
                        let mut lines: Vec<Line> = Vec::new();
                        for r in 0..inner.height.min(content.len() as u16) {
                            let mut spans: Vec<Span> = Vec::new();
                            let row = &content[r as usize];
                            for c in 0..inner.width.min(row.len() as u16) {
                                let cell = &row[c as usize];
                                let mut fg = map_color(&cell.fg);
                                let mut bg = map_color(&cell.bg);
                                if cell.inverse { std::mem::swap(&mut fg, &mut bg); }
                                let mut style = Style::default().fg(fg).bg(bg);
                                if cell.bold { style = style.add_modifier(Modifier::BOLD); }
                                if cell.italic { style = style.add_modifier(Modifier::ITALIC); }
                                if cell.underline { style = style.add_modifier(Modifier::UNDERLINED); }
                                spans.push(Span::styled(cell.text.clone(), style));
                            }
                            lines.push(Line::from(spans));
                        }
                        f.render_widget(pane_block, area);
                        f.render_widget(Clear, inner);
                        let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
                        f.render_widget(para, inner);
                        let cy = inner.y + (*cursor_row).min(inner.height.saturating_sub(1));
                        let cx = inner.x + (*cursor_col).min(inner.width.saturating_sub(1));
                        f.set_cursor(cx, cy);
                    }
                    LayoutJson::Split { kind, sizes, children } => {
                        let constraints: Vec<Constraint> = if sizes.len() == children.len() { sizes.iter().map(|p| Constraint::Percentage(*p)).collect() } else { vec![Constraint::Percentage((100 / children.len() as u16) as u16); children.len()] };
                        let rects = if kind == "Horizontal" { Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area) } else { Layout::default().direction(Direction::Vertical).constraints(constraints).split(area) };
                        for (i, child) in children.iter().enumerate() { render_json(f, child, rects[i]); }
                    }
                }
            }

            render_json(f, &root, chunks[0]);
            if session_chooser {
                let overlay = Block::default().borders(Borders::ALL).title("choose-session");
                let oa = centered_rect(70, 20, chunks[0]);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let mut lines: Vec<Line> = Vec::new();
                for (i, (sname, info)) in session_entries.iter().enumerate() {
                    let marker = if sname == &current_session { "*" } else { " " };
                    let line = if i == session_selected {
                        Line::from(Span::styled(format!("{} {}", marker, info), Style::default().bg(Color::Yellow).fg(Color::Black)))
                    } else {
                        Line::from(format!("{} {}", marker, info))
                    };
                    lines.push(line);
                }
                let para = Paragraph::new(Text::from(lines));
                f.render_widget(para, overlay.inner(oa));
            }
            if tree_chooser {
                let overlay = Block::default().borders(Borders::ALL).title("choose-tree");
                let oa = centered_rect(60, 30, chunks[0]);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let mut lines: Vec<Line> = Vec::new();
                for (i, (is_win, wid, pid, name)) in tree_entries.iter().enumerate() {
                    let marker = if *is_win { format!("@{}", wid) } else { format!("%{}", pid) };
                    let prefix = if *is_win { "".to_string() } else { "  ".to_string() };
                    let line = if i == tree_selected { Line::from(Span::styled(format!("{}{} {}", prefix, marker, name), Style::default().bg(Color::Yellow).fg(Color::Black))) } else { Line::from(format!("{}{} {}", prefix, marker, name)) };
                    lines.push(line);
                }
                let para = Paragraph::new(Text::from(lines));
                f.render_widget(para, overlay.inner(oa));
            }
            if chooser {
                let mut rects: Vec<(usize, Rect)> = Vec::new();
                fn rec(node: &LayoutJson, area: Rect, out: &mut Vec<(usize, Rect)>) {
                    match node {
                        LayoutJson::Leaf { id, .. } => { out.push((*id, area)); }
                        LayoutJson::Split { kind, sizes, children } => {
                            let constraints: Vec<Constraint> = if sizes.len() == children.len() { sizes.iter().map(|p| Constraint::Percentage(*p)).collect() } else { vec![Constraint::Percentage((100 / children.len() as u16) as u16); children.len()] };
                            let rects = if kind == "Horizontal" { Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area) } else { Layout::default().direction(Direction::Vertical).constraints(constraints).split(area) };
                            for (i, child) in children.iter().enumerate() { rec(child, rects[i], out); }
                        }
                    }
                }
                rec(&root, chunks[0], &mut rects);
                choices.clear();
                for (i,(pid,r)) in rects.iter().enumerate() { if i<10 { choices.push((i+1,*pid)); let bw=7u16; let bh=3u16; let bx=r.x + r.width.saturating_sub(bw)/2; let by=r.y + r.height.saturating_sub(bh)/2; let b=Rect{ x:bx, y:by, width:bw, height:bh }; let block=Block::default().borders(Borders::ALL).style(Style::default().bg(Color::Yellow).fg(Color::Black)); let inner=block.inner(b); let disp=if i+1==10 {0} else {i+1}; let line=Line::from(Span::styled(format!(" {} ",disp), Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD))); let para=Paragraph::new(line).alignment(Alignment::Center); f.render_widget(Clear,b); f.render_widget(block,b); f.render_widget(para,inner);} }
            }
            let status_bar = Paragraph::new(Line::from(vec![Span::raw(format!("session:{}", name))])).style(Style::default().bg(Color::Green).fg(Color::Black));
            f.render_widget(Clear, chunks[1]);
            f.render_widget(status_bar, chunks[1]);
            if pane_renaming {
                let overlay = Block::default().borders(Borders::ALL).title("set pane title");
                let oa = centered_rect(60, 3, chunks[0]);
                f.render_widget(Clear, oa);
                f.render_widget(&overlay, oa);
                let para = Paragraph::new(format!("title: {}", pane_title_buf));
                f.render_widget(para, overlay.inner(oa));
            }
        })?;
        if event::poll(Duration::from_millis(16))? {
            match event::read()? { Event::Key(key) if key.kind == KeyEventKind::Press => {
                if matches!(key.code, KeyCode::Char('q')) && key.modifiers.contains(KeyModifiers::CONTROL) { quit = true; }
                else if matches!(key.code, KeyCode::Char('b')) && key.modifiers.contains(KeyModifiers::CONTROL) { prefix_armed = true; }
                else if prefix_armed {
                    // tmux-like prefix mappings
                    match key.code {
                        KeyCode::Char('c') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"new-window\n"); } }
                        KeyCode::Char('%') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"split-window -h\n"); } }
                        KeyCode::Char('"') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"split-window -v\n"); } }
                        KeyCode::Char('x') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"kill-pane\n"); } }
                        KeyCode::Char('z') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"zoom-pane\n"); } }
                        KeyCode::Char('[') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"copy-enter\n"); } }
                        KeyCode::Char('n') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"next-window\n"); } }
                        KeyCode::Char('p') => { if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { let _ = std::io::Write::write_all(&mut s, b"previous-window\n"); } }
                        KeyCode::Char('d') => { quit = true; } // Detach
                        KeyCode::Char(',') => { renaming = true; rename_buf.clear(); }
                        KeyCode::Char('t') => { pane_renaming = true; pane_title_buf.clear(); }
                        KeyCode::Char('w') => { 
                            tree_chooser = true; 
                            tree_entries.clear(); 
                            tree_selected = 0; 
                            if let Ok(mut s) = std::net::TcpStream::connect(addr.clone()) { 
                                let _ = std::io::Write::write_all(&mut s, b"list-tree\n"); 
                                let mut buf = String::new(); 
                                let _ = std::io::Read::read_to_string(&mut s, &mut buf); 
                                let infos: Vec<WinTree> = serde_json::from_str(&buf).unwrap_or(Vec::new()); 
                                for wi in infos.into_iter() { 
                                    tree_entries.push((true, wi.id, 0, wi.name)); 
                                    for pi in wi.panes.into_iter() { 
                                        tree_entries.push((false, wi.id, pi.id, pi.title)); 
                                    } 
                                } 
                            } 
                        }
                        KeyCode::Char('s') => {
                            // Session chooser - list all available sessions
                            session_chooser = true;
                            session_entries.clear();
                            session_selected = 0;
                            let dir = format!("{}\\.psmux", home);
                            if let Ok(entries) = std::fs::read_dir(&dir) {
                                for e in entries.flatten() {
                                    if let Some(fname) = e.file_name().to_str() {
                                        if let Some((base, ext)) = fname.rsplit_once('.') {
                                            if ext == "port" {
                                                if let Ok(port_str) = std::fs::read_to_string(e.path()) {
                                                    if let Ok(p) = port_str.trim().parse::<u16>() {
                                                        let sess_addr = format!("127.0.0.1:{}", p);
                                                        // Try to get session info, but don't block too long
                                                        let info = if let Ok(mut ss) = std::net::TcpStream::connect_timeout(
                                                            &sess_addr.parse().unwrap(),
                                                            Duration::from_millis(100)
                                                        ) {
                                                            let _ = ss.set_read_timeout(Some(Duration::from_millis(100)));
                                                            let _ = std::io::Write::write_all(&mut ss, b"session-info\n");
                                                            let mut br = std::io::BufReader::new(ss);
                                                            let mut line = String::new();
                                                            if br.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                                                                line.trim().to_string()
                                                            } else {
                                                                format!("{}: (no info)", base)
                                                            }
                                                        } else {
                                                            format!("{}: (not responding)", base)
                                                        };
                                                        session_entries.push((base.to_string(), info));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // If no sessions found, add current session at minimum
                            if session_entries.is_empty() {
                                session_entries.push((current_session.clone(), format!("{}: (current)", current_session)));
                            }
                            // Select current session
                            for (i, (sname, _)) in session_entries.iter().enumerate() {
                                if sname == &current_session {
                                    session_selected = i;
                                    break;
                                }
                            }
                        }
                        KeyCode::Char('q') => { chooser = true; }
                        KeyCode::Left => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"copy-move -1 0\n"); } }
                        KeyCode::Right => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"copy-move 1 0\n"); } }
                        KeyCode::Up => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"copy-move 0 -1\n"); } }
                        KeyCode::Down => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"copy-move 0 1\n"); } }
                        KeyCode::Char('v') => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"copy-anchor\n"); } }
                        KeyCode::Char('y') => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"copy-yank\n"); } }
                        _ => {}
                    }
                    prefix_armed = false;
                } else {
                    match key.code {
                        // Session chooser handling
                        KeyCode::Up if session_chooser => { if session_selected > 0 { session_selected -= 1; } }
                        KeyCode::Down if session_chooser => { if session_selected + 1 < session_entries.len() { session_selected += 1; } }
                        KeyCode::Enter if session_chooser => {
                            if let Some((sname, _)) = session_entries.get(session_selected) {
                                if sname != &current_session {
                                    // Switch to selected session
                                    // First, detach from current session
                                    let _ = std::net::TcpStream::connect(addr.clone()).and_then(|mut s| { let _ = std::io::Write::write_all(&mut s, b"client-detach\n"); Ok(()) });
                                    
                                    // Store the target session name for re-attach after exiting this loop
                                    env::set_var("PSMUX_SWITCH_TO", sname);
                                    quit = true;
                                }
                                session_chooser = false;
                            }
                        }
                        KeyCode::Esc if session_chooser => { session_chooser = false; }
                        // Tree chooser handling
                        KeyCode::Up if tree_chooser => { if tree_selected>0 { tree_selected-=1; } }
                        KeyCode::Down if tree_chooser => { if tree_selected+1 < tree_entries.len() { tree_selected+=1; } }
                        KeyCode::Enter if tree_chooser => { if let Some((is_win, wid, pid, _)) = tree_entries.get(tree_selected) { if let Some(mut s) = try_connect() { if *is_win { let _ = std::io::Write::write_all(&mut s, format!("focus-window {}\n", wid).as_bytes()); } else { let _ = std::io::Write::write_all(&mut s, format!("focus-pane {}\n", pid).as_bytes()); } } tree_chooser=false; } }
                        KeyCode::Esc if tree_chooser => { tree_chooser=false; }
                        KeyCode::Char(c) if renaming && !key.modifiers.contains(KeyModifiers::CONTROL) => { rename_buf.push(c); }
                        KeyCode::Char(c) if pane_renaming && !key.modifiers.contains(KeyModifiers::CONTROL) => { pane_title_buf.push(c); }
                        KeyCode::Backspace if renaming => { let _ = rename_buf.pop(); }
                        KeyCode::Backspace if pane_renaming => { let _ = pane_title_buf.pop(); }
                        KeyCode::Enter if renaming => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, format!("rename-window {}\n", rename_buf).as_bytes()); } renaming=false; }
                        KeyCode::Enter if pane_renaming => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, format!("set-pane-title {}\n", pane_title_buf).as_bytes()); } pane_renaming=false; }
                        KeyCode::Esc if renaming => { renaming=false; }
                        KeyCode::Esc if pane_renaming => { pane_renaming=false; }
                        KeyCode::Char(d) if chooser && d.is_ascii_digit() => {
                            let raw = d.to_digit(10).unwrap() as usize;
                            let choice = if raw==0 {10} else {raw};
                            if let Some((_,pid)) = choices.iter().find(|(n,_)| *n==choice) { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, format!("focus-pane {}\n", pid).as_bytes()); } chooser=false; }
                        }
                        KeyCode::Esc if chooser => { chooser=false; }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Some(mut s) = try_connect() {
                                let _ = std::io::Write::write_all(&mut s, format!("send-text {}\n", c).as_bytes());
                            }
                        }
                        KeyCode::Enter => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key enter\n"); } }
                        KeyCode::Tab => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key tab\n"); } }
                        KeyCode::Backspace => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key backspace\n"); } }
                        KeyCode::Esc => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key esc\n"); } }
                        KeyCode::Left => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key left\n"); } }
                        KeyCode::Right => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key right\n"); } }
                        KeyCode::Up => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key up\n"); } }
                        KeyCode::Down => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"send-key down\n"); } }
                        _ => {}
                    }
                }
            } Event::Mouse(me) => {
                use crossterm::event::{MouseEventKind,MouseButton};
                match me.kind {
                    MouseEventKind::Down(MouseButton::Left) => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, format!("mouse-down {} {}\n", me.column, me.row).as_bytes()); } }
                    MouseEventKind::Drag(MouseButton::Left) => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, format!("mouse-drag {} {}\n", me.column, me.row).as_bytes()); } }
                    MouseEventKind::Up(MouseButton::Left) => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, format!("mouse-up {} {}\n", me.column, me.row).as_bytes()); } }
                    MouseEventKind::ScrollUp => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"scroll-up\n"); } }
                    MouseEventKind::ScrollDown => { if let Some(mut s) = try_connect() { let _ = std::io::Write::write_all(&mut s, b"scroll-down\n"); } }
                    _ => {}
                }
            } _ => {} }
        }
        if reap_children_placeholder()? { /* no-op */ }
        if quit { break; }
    }
    let _ = std::net::TcpStream::connect(addr).and_then(|mut s| { std::io::Write::write_all(&mut s, b"client-detach\n"); Ok(()) });
    Ok(())
}

fn reap_children_placeholder() -> io::Result<bool> { Ok(false) }

fn send_control(line: String) -> io::Result<()> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let target = env::var("PSMUX_TARGET_SESSION").ok().unwrap_or_else(|| "default".to_string());
    let path = format!("{}\\.psmux\\{}.port", home, target);
    let port = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u16>().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no session"))?;
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = std::net::TcpStream::connect(addr)?;
    let _ = write!(stream, "{}", line);
    Ok(())
}

fn send_control_with_response(line: String) -> io::Result<String> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let target = env::var("PSMUX_TARGET_SESSION").ok().unwrap_or_else(|| "default".to_string());
    let path = format!("{}\\.psmux\\{}.port", home, target);
    let port = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u16>().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no session"))?;
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = std::net::TcpStream::connect(&addr)?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
    let _ = write!(stream, "{}", line);
    let _ = stream.flush();
    let mut buf = Vec::new();
    let mut temp = [0u8; 4096];
    loop {
        match std::io::Read::read(&mut stream, &mut temp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&temp[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => break,
            Err(_) => break,
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let pty_system = PtySystemSelection::default()
        .get()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;

    let mut app = AppState {
        windows: Vec::new(),
        active_idx: 0,
        mode: Mode::Passthrough,
        escape_time_ms: 500,
        prefix_key: (KeyCode::Char('b'), KeyModifiers::CONTROL),
        drag: None,
        last_window_area: Rect { x: 0, y: 0, width: 0, height: 0 },
        mouse_enabled: true,
        paste_buffers: Vec::new(),
        status_left: "psmux:#I".to_string(),
        status_right: "%H:%M".to_string(),
        copy_anchor: None,
        copy_pos: None,
        display_map: Vec::new(),
        binds: Vec::new(),
        control_rx: None,
        control_port: None,
        session_name: env::var("PSMUX_SESSION_NAME").unwrap_or_else(|_| "default".to_string()),
        attached_clients: 1,
        created_at: Local::now(),
        next_win_id: 1,
        next_pane_id: 1,
        zoom_saved: None,
        sync_input: false,
        hooks: std::collections::HashMap::new(),
        wait_channels: std::collections::HashMap::new(),
        pipe_panes: Vec::new(),
        last_window_idx: 0,
        last_pane_path: Vec::new(),
    };

    load_config(&mut app);

    create_window(&*pty_system, &mut app)?;

    let (tx, rx) = mpsc::channel::<CtrlReq>();
    app.control_rx = Some(rx);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    app.control_port = Some(port);
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let dir = format!("{}\\.psmux", home);
    let _ = std::fs::create_dir_all(&dir);
    let regpath = format!("{}\\{}.port", dir, app.session_name);
    let _ = std::fs::write(&regpath, port.to_string());
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(mut stream) = conn {
                let mut line = String::new();
                let mut r = io::BufReader::new(stream.try_clone().unwrap());
                let _ = r.read_line(&mut line);
                let mut parts = line.split_whitespace();
                let cmd = parts.next().unwrap_or("");
                // parse optional target specifier
                let mut args: Vec<&str> = parts.by_ref().collect();
                let mut target_win: Option<usize> = None;
                let mut target_pane: Option<usize> = None;
                let mut start_line: Option<u16> = None;
                let mut end_line: Option<u16> = None;
                let mut i = 0;
                while i < args.len() {
                    if args[i] == "-t" {
                        if let Some(v) = args.get(i+1) {
                            if v.starts_with('%') { if let Ok(pid) = v[1..].parse::<usize>() { target_pane = Some(pid); } }
                            else if v.starts_with('@') { if let Ok(wid) = v[1..].parse::<usize>() { target_win = Some(wid); } }
                        }
                        i += 2; continue;
                    } else if args[i] == "-S" {
                        if let Some(v) = args.get(i+1) { if let Ok(n) = v.parse::<u16>() { start_line = Some(n); } }
                        i += 2; continue;
                    } else if args[i] == "-E" {
                        if let Some(v) = args.get(i+1) { if let Ok(n) = v.parse::<u16>() { end_line = Some(n); } }
                        i += 2; continue;
                    }
                    i += 1;
                }
                if let Some(wid) = target_win { let _ = tx.send(CtrlReq::FocusWindow(wid)); }
                if let Some(pid) = target_pane { let _ = tx.send(CtrlReq::FocusPane(pid)); }
                match cmd {
                    "new-window" => { let _ = tx.send(CtrlReq::NewWindow); }
                    "split-window" => {
                        let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
                        let _ = tx.send(CtrlReq::SplitWindow(kind));
                    }
                    "kill-pane" => { let _ = tx.send(CtrlReq::KillPane); }
                    "capture-pane" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        if start_line.is_some() || end_line.is_some() { let _ = tx.send(CtrlReq::CapturePaneRange(rtx, start_line, end_line)); }
                        else { let _ = tx.send(CtrlReq::CapturePane(rtx)); }
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "client-attach" => { let _ = tx.send(CtrlReq::ClientAttach); let _ = write!(stream, "ok\n"); }
                    "client-detach" => { let _ = tx.send(CtrlReq::ClientDetach); let _ = write!(stream, "ok\n"); }
                    "session-info" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::SessionInfo(rtx));
                        if let Ok(line) = rrx.recv() { let _ = write!(stream, "{}", line); let _ = stream.flush(); }
                    }
                    _ => {}
                }
            }
        }
    });

    let mut last_resize = Instant::now();
    let mut quit = false;
    loop {
        terminal.draw(|f| {
            let area = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
                .split(area);

            app.last_window_area = chunks[0];
            render_window(f, &mut app, chunks[0]);

            let _mode_str = match app.mode { 
                Mode::Passthrough => "", 
                Mode::Prefix { .. } => "PREFIX", 
                Mode::CommandPrompt { .. } => ":", 
                Mode::WindowChooser { .. } => "W", 
                Mode::RenamePrompt { .. } => "REN", 
                Mode::CopyMode => "CPY", 
                Mode::PaneChooser { .. } => "PANE",
                Mode::MenuMode { .. } => "MENU",
                Mode::PopupMode { .. } => "POPUP",
                Mode::ConfirmMode { .. } => "CONFIRM",
            };
            let time_str = Local::now().format("%H:%M").to_string();
            let mut windows_list = String::new();
            for (i, _) in app.windows.iter().enumerate() {
                if i == app.active_idx { windows_list.push_str(&format!(" #[{}]", i+1)); } else { windows_list.push_str(&format!(" {}", i+1)); }
            }
            let status_spans = parse_status(&app.status_left, &app, &time_str);
            let mut right_spans = parse_status(&app.status_right, &app, &time_str);
            let mut combined: Vec<Span<'static>> = status_spans;
            combined.push(Span::raw(" "));
            combined.append(&mut right_spans);
            let status_bar = Paragraph::new(Line::from(combined)).style(Style::default().bg(Color::Green).fg(Color::Black));
            f.render_widget(Clear, chunks[1]);
            f.render_widget(status_bar, chunks[1]);

            if let Mode::CommandPrompt { input } = &app.mode {
                let overlay = Paragraph::new(format!(":{}", input)).block(Block::default().borders(Borders::ALL).title("command"));
                let oa = centered_rect(80, 3, area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::WindowChooser { selected } = app.mode {
                let mut lines: Vec<Line> = Vec::new();
                for (i,w) in app.windows.iter().enumerate() {
                    let marker = if i == selected { ">" } else { " " };
                    lines.push(Line::from(format!("{} [{}] {}", marker, i+1, w.name)));
                }
                let overlay = Paragraph::new(Text::from(lines)).block(Block::default().borders(Borders::ALL).title("windows"));
                let oa = centered_rect(60, (app.windows.len() as u16 + 2).min(10), area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::RenamePrompt { input } = &app.mode {
                let overlay = Paragraph::new(format!("rename: {}", input)).block(Block::default().borders(Borders::ALL).title("rename window"));
                let oa = centered_rect(60, 3, area);
                f.render_widget(Clear, oa);
                f.render_widget(overlay, oa);
            }

            if let Mode::PaneChooser { .. } = &app.mode {
                let win = &app.windows[app.active_idx];
                let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
                compute_rects(&win.root, app.last_window_area, &mut rects);
                for (i, (_, r)) in rects.iter().enumerate() {
                    let n = i + 1;
                    if n > 9 { break; }
                    let bw = 7u16;
                    let bh = 3u16;
                    let bx = r.x + r.width.saturating_sub(bw) / 2;
                    let by = r.y + r.height.saturating_sub(bh) / 2;
                    let b = Rect { x: bx, y: by, width: bw, height: bh };
                    let block = Block::default().borders(Borders::ALL).style(Style::default().bg(Color::Yellow).fg(Color::Black));
                    let inner = block.inner(b);
                    let disp = if n == 10 { 0 } else { n };
                    let line = Line::from(Span::styled(format!(" {} ", disp), Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)));
                    let para = Paragraph::new(line).alignment(Alignment::Center);
                    f.render_widget(Clear, b);
                    f.render_widget(block, b);
                    f.render_widget(para, inner);
                }
            }

            // Render Menu mode
            if let Mode::MenuMode { menu } = &app.mode {
                let item_count = menu.items.len();
                let height = (item_count as u16 + 2).min(20);
                let width = menu.items.iter().map(|i| i.name.len()).max().unwrap_or(10).max(menu.title.len()) as u16 + 8;
                
                // Calculate position based on x/y or center
                let menu_area = if let (Some(x), Some(y)) = (menu.x, menu.y) {
                    let x = if x < 0 { (area.width as i16 + x).max(0) as u16 } else { x as u16 };
                    let y = if y < 0 { (area.height as i16 + y).max(0) as u16 } else { y as u16 };
                    Rect { x: x.min(area.width.saturating_sub(width)), y: y.min(area.height.saturating_sub(height)), width, height }
                } else {
                    centered_rect((width * 100 / area.width.max(1)).max(30), height, area)
                };
                
                let title = if menu.title.is_empty() { "Menu" } else { &menu.title };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title);
                
                let mut lines: Vec<Line> = Vec::new();
                for (i, item) in menu.items.iter().enumerate() {
                    if item.is_separator {
                        lines.push(Line::from("─".repeat(width.saturating_sub(2) as usize)));
                    } else {
                        let marker = if i == menu.selected { ">" } else { " " };
                        let key_str = item.key.map(|k| format!("({})", k)).unwrap_or_default();
                        let style = if i == menu.selected {
                            Style::default().bg(Color::Blue).fg(Color::White)
                        } else {
                            Style::default()
                        };
                        lines.push(Line::from(Span::styled(
                            format!("{} {} {}", marker, item.name, key_str),
                            style
                        )));
                    }
                }
                
                let para = Paragraph::new(Text::from(lines)).block(block);
                f.render_widget(Clear, menu_area);
                f.render_widget(para, menu_area);
            }

            // Render Popup mode
            if let Mode::PopupMode { command, output, width, height, .. } = &app.mode {
                let w = (*width).min(area.width.saturating_sub(4));
                let h = (*height).min(area.height.saturating_sub(4));
                let popup_area = Rect {
                    x: (area.width.saturating_sub(w)) / 2,
                    y: (area.height.saturating_sub(h)) / 2,
                    width: w,
                    height: h,
                };
                
                let title = if command.is_empty() { "Popup" } else { command };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(title);
                
                let para = Paragraph::new(output.as_str())
                    .block(block)
                    .wrap(ratatui::widgets::Wrap { trim: false });
                
                f.render_widget(Clear, popup_area);
                f.render_widget(para, popup_area);
            }

            // Render Confirm mode
            if let Mode::ConfirmMode { prompt, input, .. } = &app.mode {
                let width = (prompt.len() as u16 + 10).min(80);
                let confirm_area = centered_rect((width * 100 / area.width.max(1)).max(40), 3, area);
                
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title("Confirm");
                
                let text = format!("{} {}", prompt, input);
                let para = Paragraph::new(text).block(block);
                
                f.render_widget(Clear, confirm_area);
                f.render_widget(para, confirm_area);
            }
        })?;

        if let Mode::PaneChooser { opened_at } = &app.mode {
            if opened_at.elapsed() > Duration::from_millis(1500) { app.mode = Mode::Passthrough; }
        }

        if event::poll(Duration::from_millis(20))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key(&mut app, key)? {
                        quit = true;
                    }
                }
                Event::Mouse(me) => {
                    let area = app.last_window_area;
                    handle_mouse(&mut app, me, area)?;
                }
                Event::Resize(cols, rows) => {
                    if last_resize.elapsed() > Duration::from_millis(50) {
                        let win = &mut app.windows[app.active_idx];
                        if let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) {
                            let _ = pane.master.resize(PtySize { rows: rows as u16, cols: cols as u16, pixel_width: 0, pixel_height: 0 });
                            let mut parser = pane.term.lock().unwrap();
                            parser.screen_mut().set_size(rows, cols);
                        }
                        last_resize = Instant::now();
                    }
                }
                _ => {}
            }
        }

        loop {
            let req = if let Some(rx) = app.control_rx.as_ref() { rx.try_recv().ok() } else { None };
            let Some(req) = req else { break; };
            match req {
                CtrlReq::NewWindow => {
                    let pty_system = PtySystemSelection::default().get().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;
                    create_window(&*pty_system, &mut app)?;
                }
                CtrlReq::SplitWindow(k) => { let _ = split_active(&mut app, k); }
                CtrlReq::KillPane => { let _ = kill_active_pane(&mut app); }
                CtrlReq::CapturePane(resp) => {
                    if let Some(text) = capture_active_pane_text(&mut app)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneRange(resp, s, e) => {
                    if let Some(text) = capture_active_pane_range(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::FocusWindow(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::FocusPane(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::SessionInfo(resp) => {
                    let attached = if app.attached_clients > 0 { "(attached)" } else { "(detached)" };
                    let windows = app.windows.len();
                    let (w,h) = {
                        let win = &mut app.windows[app.active_idx];
                        let mut size = (0,0);
                        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { size = (p.last_cols as i32, p.last_rows as i32); }
                        size
                    };
                    let created = app.created_at.format("%a %b %e %H:%M:%S %Y");
                    let line = format!("{}: {} windows (created {}) [{}x{}] {}\n", app.session_name, windows, created, w, h, attached);
                    let _ = resp.send(line);
                }
                CtrlReq::ClientAttach => { app.attached_clients = app.attached_clients.saturating_add(1); }
                CtrlReq::ClientDetach => { app.attached_clients = app.attached_clients.saturating_sub(1); }
                CtrlReq::DumpLayout(resp) => {
                    let json = dump_layout_json(&mut app)?;
                    let _ = resp.send(json);
                }
                CtrlReq::SendText(s) => { send_text_to_active(&mut app, &s)?; }
                CtrlReq::SendKey(k) => { send_key_to_active(&mut app, &k)?; }
                CtrlReq::ZoomPane => { toggle_zoom(&mut app); }
                CtrlReq::CopyEnter => { enter_copy_mode(&mut app); }
                CtrlReq::CopyMove(dx, dy) => { move_copy_cursor(&mut app, dx, dy); }
                CtrlReq::CopyAnchor => { if let Some((r,c)) = current_prompt_pos(&mut app) { app.copy_anchor = Some((r,c)); app.copy_pos = Some((r,c)); } }
                CtrlReq::CopyYank => { let _ = yank_selection(&mut app); app.mode = Mode::Passthrough; }
                CtrlReq::ClientSize(w, h) => { app.last_window_area = Rect { x: 0, y: 0, width: w, height: h }; }
                CtrlReq::FocusPaneCmd(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::FocusWindowCmd(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::MouseDown(x,y) => { remote_mouse_down(&mut app, x, y); }
                CtrlReq::MouseDrag(x,y) => { remote_mouse_drag(&mut app, x, y); }
                CtrlReq::MouseUp(_,_) => { app.drag = None; }
                CtrlReq::ScrollUp => { remote_scroll_up(&mut app); }
                CtrlReq::ScrollDown => { remote_scroll_down(&mut app); }
                CtrlReq::NextWindow => { if !app.windows.is_empty() { app.active_idx = (app.active_idx + 1) % app.windows.len(); } }
                CtrlReq::PrevWindow => { if !app.windows.is_empty() { app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); } }
                CtrlReq::RenameWindow(name) => { let win = &mut app.windows[app.active_idx]; win.name = name; }
                CtrlReq::ListWindows(resp) => { let json = list_windows_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ListTree(resp) => { let json = list_tree_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ToggleSync => { app.sync_input = !app.sync_input; }
                CtrlReq::SetPaneTitle(title) => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { p.title = title; }
                }
                // For attach mode, we just ignore the new commands - they're handled by the server
                _ => {}
            }
        }

        if reap_children(&mut app)? {
            quit = true;
        }

        if quit { break; }
    }
    // teardown: kill all pane children
    for win in app.windows.iter_mut() {
        kill_all_children(&mut win.root);
    }
    Ok(())
}

fn create_window(pty_system: &dyn portable_pty::PtySystem, app: &mut AppState) -> io::Result<()> {
    let size = PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system
        .openpty(size)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;

    let shell_cmd = detect_shell();
    let child = pair
        .slave
        .spawn_command(shell_cmd)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;

    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, 0)));
    let term_reader = term.clone();
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;

    thread::spawn(move || {
        let mut local = [0u8; 8192];
        loop {
            match reader.read(&mut local) {
                Ok(n) if n > 0 => {
                    let mut parser = term_reader.lock().unwrap();
                    parser.process(&local[..n]);
                }
                Ok(_) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
    });

    let pane = Pane { master: pair.master, child, term, last_rows: size.rows, last_cols: size.cols, id: app.next_pane_id, title: format!("pane %{}", app.next_pane_id) };
    app.next_pane_id += 1;
    app.windows.push(Window { root: Node::Leaf(pane), active_path: vec![], name: format!("win {}", app.windows.len()+1), id: app.next_win_id });
    app.next_win_id += 1;
    app.active_idx = app.windows.len() - 1;
    Ok(())
}

fn handle_key(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    if matches!(key.code, KeyCode::Char('q')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }

    match app.mode {
        Mode::Passthrough => {
            let is_ctrl_b = (key.code, key.modifiers) == app.prefix_key
                || matches!(key.code, KeyCode::Char(c) if c == '\u{0002}');
            if is_ctrl_b {
                app.mode = Mode::Prefix { armed_at: Instant::now() };
                return Ok(false);
            }
            forward_key_to_active(app, key)?;
            Ok(false)
        }
        Mode::Prefix { armed_at } => {
            let elapsed = armed_at.elapsed().as_millis() as u64;
            let handled = match key.code {
                KeyCode::Left => { move_focus(app, FocusDir::Left); true }
                KeyCode::Right => { move_focus(app, FocusDir::Right); true }
                KeyCode::Up => { move_focus(app, FocusDir::Up); true }
                KeyCode::Down => { move_focus(app, FocusDir::Down); true }
                KeyCode::Char(d) if d.is_ascii_digit() => {
                    let idx = d.to_digit(10).unwrap() as usize;
                    if idx > 0 && idx <= app.windows.len() { app.active_idx = idx - 1; }
                    true
                }
                KeyCode::Char('c') => {
                    let pty_system = PtySystemSelection::default()
                        .get()
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;
                    create_window(&*pty_system, app)?;
                    true
                }
                KeyCode::Char('n') => {
                    if !app.windows.is_empty() {
                        app.active_idx = (app.active_idx + 1) % app.windows.len();
                    }
                    true
                }
                KeyCode::Char('p') => {
                    if !app.windows.is_empty() {
                        app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len();
                    }
                    true
                }
                KeyCode::Char('%') => {
                    split_active(app, LayoutKind::Horizontal)?;
                    true
                }
                KeyCode::Char('"') => {
                    split_active(app, LayoutKind::Vertical)?;
                    true
                }
                KeyCode::Char('x') => {
                    kill_active_pane(app)?;
                    true
                }
                KeyCode::Char('d') => {
                    // detach: exit pmux cleanly
                    return Ok(true);
                }
                KeyCode::Char('w') => { app.mode = Mode::WindowChooser { selected: app.active_idx }; true }
                KeyCode::Char(',') => { app.mode = Mode::RenamePrompt { input: String::new() }; true }
                KeyCode::Char(' ') => { cycle_top_layout(app); true }
                KeyCode::Char('[') => { enter_copy_mode(app); true }
                KeyCode::Char(']') => { paste_latest(app)?; app.mode = Mode::Passthrough; true }
                KeyCode::Char(':') => {
                    app.mode = Mode::CommandPrompt { input: String::new() };
                    true
                }
                KeyCode::Char('q') => {
                    let win = &app.windows[app.active_idx];
                    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
                    compute_rects(&win.root, app.last_window_area, &mut rects);
                    app.display_map.clear();
                    for (i, (path, _)) in rects.into_iter().enumerate() {
                        let n = i + 1;
                        if n <= 10 { app.display_map.push((n, path)); } else { break; }
                    }
                    app.mode = Mode::PaneChooser { opened_at: Instant::now() };
                    true
                }
                _ => false,
            };

            if matches!(app.mode, Mode::Prefix { .. }) {
                if !handled && elapsed < app.escape_time_ms {
                    // Unrecognized after prefix: do not send '^B'; swallow and return
                    return Ok(false);
                }
                app.mode = Mode::Passthrough;
            }
            Ok(false)
        }
        Mode::CommandPrompt { .. } => {
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Passthrough; }
                KeyCode::Enter => { execute_command_prompt(app)?; }
                KeyCode::Backspace => {
                    if let Mode::CommandPrompt { input } = &mut app.mode { let _ = input.pop(); }
                }
                KeyCode::Char(c) => {
                    if let Mode::CommandPrompt { input } = &mut app.mode { input.push(c); }
                }
                _ => {}
            }
            Ok(false)
        }
        Mode::WindowChooser { selected } => {
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Passthrough; }
                KeyCode::Up | KeyCode::Left => { if selected > 0 { if let Mode::WindowChooser { selected: s } = &mut app.mode { *s -= 1; } } }
                KeyCode::Down | KeyCode::Right => { if selected + 1 < app.windows.len() { if let Mode::WindowChooser { selected: s } = &mut app.mode { *s += 1; } } }
                KeyCode::Enter => { if let Mode::WindowChooser { selected: s } = &mut app.mode { app.active_idx = *s; app.mode = Mode::Passthrough; } }
                _ => {}
            }
            Ok(false)
        }
        Mode::RenamePrompt { .. } => {
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Passthrough; }
                KeyCode::Enter => { if let Mode::RenamePrompt { input } = &mut app.mode { app.windows[app.active_idx].name = input.clone(); app.mode = Mode::Passthrough; } }
                KeyCode::Backspace => { if let Mode::RenamePrompt { input } = &mut app.mode { let _ = input.pop(); } }
                KeyCode::Char(c) => { if let Mode::RenamePrompt { input } = &mut app.mode { input.push(c); } }
                _ => {}
            }
            Ok(false)
        }
        Mode::CopyMode => {
            match key.code {
                KeyCode::Esc | KeyCode::Char(']') => { app.mode = Mode::Passthrough; app.copy_anchor = None; app.copy_pos = None; }
                KeyCode::Left => { move_copy_cursor(app, -1, 0); }
                KeyCode::Right => { move_copy_cursor(app, 1, 0); }
                KeyCode::Up => { move_copy_cursor(app, 0, -1); }
                KeyCode::Down => { move_copy_cursor(app, 0, 1); }
                KeyCode::Char('v') => { if let Some((r,c)) = current_prompt_pos(app) { app.copy_anchor = Some((r,c)); app.copy_pos = Some((r,c)); } }
                KeyCode::Char('y') => { yank_selection(app)?; app.mode = Mode::Passthrough; }
                _ => {}
            }
            Ok(false)
        }
        Mode::PaneChooser { .. } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Passthrough; }
                KeyCode::Char(d) if d.is_ascii_digit() => {
                    let raw = d.to_digit(10).unwrap() as usize;
                    let choice = if raw == 0 { 10 } else { raw };
                    if let Some((_, path)) = app.display_map.iter().find(|(n, _)| *n == choice) {
                        let win = &mut app.windows[app.active_idx];
                        win.active_path = path.clone();
                        app.mode = Mode::Passthrough;
                    }
                }
                _ => {}
            }
            Ok(false)
        }
        Mode::MenuMode { ref mut menu } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => { 
                    app.mode = Mode::Passthrough; 
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    // Move selection up, skip separators
                    if menu.selected > 0 {
                        menu.selected -= 1;
                        while menu.selected > 0 && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                            menu.selected -= 1;
                        }
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Move selection down, skip separators
                    if menu.selected + 1 < menu.items.len() {
                        menu.selected += 1;
                        while menu.selected + 1 < menu.items.len() && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                            menu.selected += 1;
                        }
                    }
                }
                KeyCode::Enter => {
                    // Execute selected item's command
                    if let Some(item) = menu.items.get(menu.selected) {
                        if !item.is_separator && !item.command.is_empty() {
                            let cmd = item.command.clone();
                            app.mode = Mode::Passthrough;
                            // Execute the command through command parsing
                            let _ = execute_command_string(app, &cmd);
                        } else {
                            app.mode = Mode::Passthrough;
                        }
                    } else {
                        app.mode = Mode::Passthrough;
                    }
                }
                KeyCode::Char(c) => {
                    // Check if any item has this as a shortcut key
                    if let Some((idx, item)) = menu.items.iter().enumerate().find(|(_, i)| i.key == Some(c)) {
                        if !item.is_separator && !item.command.is_empty() {
                            let cmd = item.command.clone();
                            app.mode = Mode::Passthrough;
                            let _ = execute_command_string(app, &cmd);
                        } else {
                            app.mode = Mode::Passthrough;
                        }
                    }
                }
                _ => {}
            }
            Ok(false)
        }
        Mode::PopupMode { ref mut output, ref mut process, close_on_exit, .. } => {
            let mut should_close = false;
            let mut exit_status: Option<std::process::ExitStatus> = None;
            
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    // Kill the process if running
                    if let Some(ref mut proc) = process {
                        let _ = proc.kill();
                    }
                    should_close = true;
                }
                KeyCode::Char(c) => {
                    // Send character to process stdin if available
                    // For now, just accumulate output
                    output.push(c);
                }
                KeyCode::Enter => {
                    output.push('\n');
                }
                _ => {}
            }
            
            // Check if process has output to read
            if let Some(ref mut proc) = process {
                // Check if process exited
                if let Ok(Some(status)) = proc.try_wait() {
                    exit_status = Some(status);
                    if close_on_exit {
                        should_close = true;
                    }
                }
            }
            
            // Handle exit message if needed (before we potentially close)
            if let Some(status) = exit_status {
                if !close_on_exit {
                    output.push_str(&format!("\n[Process exited with status: {}]", status));
                }
            }
            
            // Now we can safely assign to app.mode
            if should_close {
                app.mode = Mode::Passthrough;
            }
            
            Ok(false)
        }
        Mode::ConfirmMode { ref prompt, ref command, ref mut input } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    // Cancel - don't execute
                    app.mode = Mode::Passthrough;
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    // Confirm - execute the command
                    let cmd = command.clone();
                    app.mode = Mode::Passthrough;
                    let _ = execute_command_string(app, &cmd);
                }
                KeyCode::Char(c) => {
                    input.push(c);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                _ => {}
            }
            Ok(false)
        }
    }
}

fn move_focus(app: &mut AppState, dir: FocusDir) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    // find active index
    let mut active_idx = None;
    for (i, (path, _)) in rects.iter().enumerate() { if *path == win.active_path { active_idx = Some(i); break; } }
    let Some(ai) = active_idx else { return; };
    let (_, arect) = &rects[ai];
    // pick nearest neighbor in direction
    let mut best: Option<(usize, u32)> = None;
    for (i, (_, r)) in rects.iter().enumerate() {
        if i == ai { continue; }
        let candidate = match dir {
            FocusDir::Left => if r.x + r.width <= arect.x { Some((arect.x - (r.x + r.width)) as u32) } else { None },
            FocusDir::Right => if r.x >= arect.x + arect.width { Some((r.x - (arect.x + arect.width)) as u32) } else { None },
            FocusDir::Up => if r.y + r.height <= arect.y { Some((arect.y - (r.y + r.height)) as u32) } else { None },
            FocusDir::Down => if r.y >= arect.y + arect.height { Some((r.y - (arect.y + arect.height)) as u32) } else { None },
        };
        if let Some(dist) = candidate { if best.map_or(true, |(_,bd)| dist < bd) { best = Some((i, dist)); } }
    }
    if let Some((ni, _)) = best { win.active_path = rects[ni].0.clone(); }
}

fn forward_key_to_active(app: &mut AppState, key: KeyEvent) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    let Some(active) = active_pane_mut(&mut win.root, &win.active_path) else { return Ok(()); };
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = write!(active.master, "{}", c);
        }
        KeyCode::Enter => { let _ = write!(active.master, "\r"); }
        KeyCode::Tab => { let _ = write!(active.master, "\t"); }
        KeyCode::Backspace => { let _ = write!(active.master, "\x08"); }
        KeyCode::Esc => { let _ = write!(active.master, "\x1b"); }
        KeyCode::Left => { let _ = write!(active.master, "\x1b[D"); }
        KeyCode::Right => { let _ = write!(active.master, "\x1b[C"); }
        KeyCode::Up => { let _ = write!(active.master, "\x1b[A"); }
        KeyCode::Down => { let _ = write!(active.master, "\x1b[B"); }
        _ => {}
    }
    Ok(())
}

fn split_active(app: &mut AppState, kind: LayoutKind) -> io::Result<()> {
    let pty_system = PtySystemSelection::default().get().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;
    let size = PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    let shell_cmd = detect_shell();
    let child = pair.slave.spawn_command(shell_cmd).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, 0)));
    let term_reader = term.clone();
    let mut reader = pair.master.try_clone_reader().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    thread::spawn(move || {
        let mut local = [0u8; 8192];
        loop {
            match reader.read(&mut local) {
                Ok(n) if n > 0 => { let mut parser = term_reader.lock().unwrap(); parser.process(&local[..n]); }
                Ok(_) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
    });
    let new_leaf = Node::Leaf(Pane { master: pair.master, child, term, last_rows: size.rows, last_cols: size.cols, id: app.next_pane_id, title: format!("pane %{}", app.next_pane_id) });
    app.next_pane_id += 1;
    let win = &mut app.windows[app.active_idx];
    replace_leaf_with_split(&mut win.root, &win.active_path, kind, new_leaf);
    // focus the newly created right child of the split
    let mut new_path = win.active_path.clone();
    new_path.push(1);
    win.active_path = new_path;
    Ok(())
}

fn handle_mouse(app: &mut AppState, me: crossterm::event::MouseEvent, window_area: Rect) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    // compute leaf rects from split tree
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, window_area, &mut rects);
    // compute borders for splits
    let mut borders: Vec<(Vec<usize>, LayoutKind, usize, u16)> = Vec::new();
    compute_split_borders(&win.root, window_area, &mut borders);

    use crossterm::event::{MouseEventKind, MouseButton};
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // focus pane
            for (path, area) in rects.iter() {
                if area.contains(ratatui::layout::Position { x: me.column, y: me.row }) { win.active_path = path.clone(); }
            }
            // check border resize start
            let tol = 1u16;
            for (path, kind, idx, pos) in borders.iter() {
                match kind {
                    LayoutKind::Horizontal => {
                        if me.column >= pos.saturating_sub(tol) && me.column <= pos + tol {
                            // record initial sizes from split node
                            if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) {
                                app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: me.column, start_y: me.row, left_initial: left, _right_initial: right });
                            }
                            break;
                        }
                    }
                    LayoutKind::Vertical => {
                        if me.row >= pos.saturating_sub(tol) && me.row <= pos + tol {
                            if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) {
                                app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: me.column, start_y: me.row, left_initial: left, _right_initial: right });
                            }
                            break;
                        }
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(d) = &app.drag {
                adjust_split_sizes(&mut win.root, d, me.column, me.row);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => { app.drag = None; }
        MouseEventKind::ScrollUp => {
            if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(active.master, "\x1b[A"); }
        }
        MouseEventKind::ScrollDown => {
            if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(active.master, "\x1b[B"); }
        }
        _ => {}
    }
    Ok(())
}

// TODO: implement split-border detection and per-node size adjustment

fn kill_active_pane(app: &mut AppState) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    kill_leaf(&mut win.root, &win.active_path);
    Ok(())
}

fn detect_shell() -> CommandBuilder {
    let pwsh = which::which("pwsh").ok().map(|p| p.to_string_lossy().into_owned());
    let cmd = which::which("cmd").ok().map(|p| p.to_string_lossy().into_owned());
    match pwsh.or(cmd) {
        Some(path) => CommandBuilder::new(path),
        None => CommandBuilder::new("pwsh.exe"),
    }
}

fn execute_command_prompt(app: &mut AppState) -> io::Result<()> {
    let cmdline = match &app.mode { Mode::CommandPrompt { input } => input.clone(), _ => String::new() };
    app.mode = Mode::Passthrough;
    let parts: Vec<&str> = cmdline.split_whitespace().collect();
    if parts.is_empty() { return Ok(()); }
    match parts[0] {
        "new-window" => {
            let pty_system = PtySystemSelection::default().get().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;
            create_window(&*pty_system, app)?;
        }
        "split-window" => {
            let kind = if parts.iter().any(|p| *p == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
            split_active(app, kind)?;
        }
        "kill-pane" => { kill_active_pane(app)?; }
        "capture-pane" => { capture_active_pane(app)?; }
        "save-buffer" => { if let Some(file) = parts.get(1) { save_latest_buffer(app, file)?; } }
        "list-sessions" => { println!("default"); }
        "attach-session" => { /* already attached */ }
        "next-window" => { app.active_idx = (app.active_idx + 1) % app.windows.len(); }
        "previous-window" => { app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); }
        "select-window" => {
            if let Some(tidx) = parts.iter().position(|p| *p == "-t").and_then(|i| parts.get(i+1)) { if let Ok(n) = tidx.parse::<usize>() { if n>0 && n<=app.windows.len() { app.active_idx = n-1; } } }
        }
        _ => {}
    }
    Ok(())
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(height),
            Constraint::Percentage(50),
        ])
        .split(r);
    let middle = popup_layout[1];
    let width = (middle.width * percent_x) / 100;
    let x = middle.x + (middle.width - width) / 2;
    Rect { x, y: middle.y, width, height }
}

/// Parse a menu definition string into a Menu structure
/// Format: "title" "name" "key" "command" ... or separator ""
fn parse_menu_definition(def: &str, x: Option<i16>, y: Option<i16>) -> Menu {
    let mut menu = Menu {
        title: String::new(),
        items: Vec::new(),
        selected: 0,
        x,
        y,
    };
    
    // Simple parsing - split by whitespace and group into items
    let parts: Vec<&str> = def.split_whitespace().collect();
    if parts.is_empty() {
        return menu;
    }
    
    // First part could be -T title
    let mut i = 0;
    while i < parts.len() {
        if parts[i] == "-T" {
            if let Some(title) = parts.get(i + 1) {
                menu.title = title.trim_matches('"').to_string();
                i += 2;
                continue;
            }
        }
        
        // Parse items in groups of 3: name key command
        // Empty name "" means separator
        if let Some(name) = parts.get(i) {
            let name = name.trim_matches('"').to_string();
            if name.is_empty() || name == "-" {
                // Separator
                menu.items.push(MenuItem {
                    name: String::new(),
                    key: None,
                    command: String::new(),
                    is_separator: true,
                });
                i += 1;
            } else {
                let key = parts.get(i + 1).map(|k| k.trim_matches('"').chars().next()).flatten();
                let command = parts.get(i + 2).map(|c| c.trim_matches('"').to_string()).unwrap_or_default();
                menu.items.push(MenuItem {
                    name,
                    key,
                    command,
                    is_separator: false,
                });
                i += 3;
            }
        } else {
            break;
        }
    }
    
    // If no items parsed, create some defaults for demonstration
    if menu.items.is_empty() && !def.is_empty() {
        menu.title = "Menu".to_string();
        menu.items.push(MenuItem {
            name: def.to_string(),
            key: Some('1'),
            command: def.to_string(),
            is_separator: false,
        });
    }
    
    menu
}

/// Fire hooks for a given event
fn fire_hooks(app: &mut AppState, event: &str) {
    if let Some(commands) = app.hooks.get(event).cloned() {
        for cmd in commands {
            // Execute hook command through command string execution
            let _ = execute_command_string(app, &cmd);
        }
    }
}

/// Execute a command string (used by menus, hooks, confirm dialogs, etc.)
fn execute_command_string(app: &mut AppState, cmd: &str) -> io::Result<()> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() { return Ok(()); }
    
    match parts[0] {
        "new-window" | "neww" => {
            // Would need pty_system - use control channel instead
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "new-window\n");
            }
        }
        "split-window" | "splitw" => {
            let flag = if parts.iter().any(|p| *p == "-h") { "-h" } else { "-v" };
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("split-window {}\n", flag));
            }
        }
        "kill-pane" => {
            let _ = kill_active_pane(app);
        }
        "kill-window" | "killw" => {
            if app.windows.len() > 1 {
                app.windows.remove(app.active_idx);
                if app.active_idx >= app.windows.len() {
                    app.active_idx = app.windows.len() - 1;
                }
            }
        }
        "next-window" | "next" => {
            if !app.windows.is_empty() {
                app.last_window_idx = app.active_idx;
                app.active_idx = (app.active_idx + 1) % app.windows.len();
            }
        }
        "previous-window" | "prev" => {
            if !app.windows.is_empty() {
                app.last_window_idx = app.active_idx;
                app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len();
            }
        }
        "last-window" | "last" => {
            if app.last_window_idx < app.windows.len() {
                let tmp = app.active_idx;
                app.active_idx = app.last_window_idx;
                app.last_window_idx = tmp;
            }
        }
        "select-window" | "selectw" => {
            if let Some(t_pos) = parts.iter().position(|p| *p == "-t") {
                if let Some(t) = parts.get(t_pos + 1) {
                    if let Ok(idx) = t.parse::<usize>() {
                        if idx < app.windows.len() {
                            app.last_window_idx = app.active_idx;
                            app.active_idx = idx;
                        }
                    }
                }
            }
        }
        "select-pane" | "selectp" => {
            let dir = if parts.iter().any(|p| *p == "-U") { FocusDir::Up }
                else if parts.iter().any(|p| *p == "-D") { FocusDir::Down }
                else if parts.iter().any(|p| *p == "-L") { FocusDir::Left }
                else if parts.iter().any(|p| *p == "-R") { FocusDir::Right }
                else { return Ok(()); };
            // Save last pane path
            let win = &app.windows[app.active_idx];
            app.last_pane_path = win.active_path.clone();
            move_focus(app, dir);
        }
        "last-pane" | "lastp" => {
            let win = &mut app.windows[app.active_idx];
            if !app.last_pane_path.is_empty() {
                let tmp = win.active_path.clone();
                win.active_path = app.last_pane_path.clone();
                app.last_pane_path = tmp;
            }
        }
        "rename-window" | "renamew" => {
            if let Some(name) = parts.get(1) {
                let win = &mut app.windows[app.active_idx];
                win.name = name.to_string();
            }
        }
        "zoom-pane" | "zoom" | "resizep -Z" => {
            toggle_zoom(app);
        }
        "copy-mode" => {
            enter_copy_mode(app);
        }
        "display-panes" | "displayp" => {
            app.mode = Mode::PaneChooser { opened_at: Instant::now() };
        }
        "confirm-before" | "confirm" => {
            // Parse confirm command
            let rest = parts[1..].join(" ");
            app.mode = Mode::ConfirmMode {
                prompt: format!("Run '{}'?", rest),
                command: rest,
                input: String::new(),
            };
        }
        "display-menu" | "menu" => {
            let rest = parts[1..].join(" ");
            let menu = parse_menu_definition(&rest, None, None);
            if !menu.items.is_empty() {
                app.mode = Mode::MenuMode { menu };
            }
        }
        "display-popup" | "popup" => {
            let rest = parts[1..].join(" ");
            app.mode = Mode::PopupMode {
                command: rest.clone(),
                output: String::new(),
                process: None,
                width: 80,
                height: 24,
                close_on_exit: true,
            };
        }
        _ => {
            // Unknown command - could log or ignore
        }
    }
    Ok(())
}

/// Send a control message to a specific port
fn send_control_to_port(port: u16, msg: &str) -> io::Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    if let Ok(mut stream) = std::net::TcpStream::connect(&addr) {
        let _ = stream.write_all(msg.as_bytes());
    }
    Ok(())
}

/// Get the pane ID of the active pane
fn get_active_pane_id(node: &Node, path: &[usize]) -> Option<usize> {
    match node {
        Node::Leaf(p) => Some(p.id),
        Node::Split { children, .. } => {
            if let Some(&idx) = path.first() {
                if let Some(child) = children.get(idx) {
                    return get_active_pane_id(child, &path[1..]);
                }
            }
            // Default to first child
            children.first().and_then(|c| get_active_pane_id(c, &[]))
        }
    }
}

fn reap_children(app: &mut AppState) -> io::Result<bool> {
    // Transform each window's split tree by pruning exited leaves.
    for i in (0..app.windows.len()).rev() {
        let root = std::mem::replace(&mut app.windows[i].root, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
        match prune_exited(root) {
            Some(new_root) => {
                app.windows[i].root = new_root;
                // ensure active_path remains valid; if not, pick first available leaf
                if !path_exists(&app.windows[i].root, &app.windows[i].active_path) {
                    app.windows[i].active_path = first_leaf_path(&app.windows[i].root);
                }
            }
            None => { app.windows.remove(i); }
        }
    }
    Ok(app.windows.is_empty())
}

fn vt_to_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => match i {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            _ => Color::Reset,
        },
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn apply_cursor_style<W: Write>(out: &mut W) -> io::Result<()> {
    let style = env::var("PSMUX_CURSOR_STYLE").unwrap_or_else(|_| "bar".to_string());
    let blink = env::var("PSMUX_CURSOR_BLINK").unwrap_or_else(|_| "1".to_string()) != "0";
    let code = match style.as_str() {
        "block" => if blink { 1 } else { 2 },
        "underline" => if blink { 3 } else { 4 },
        "bar" | "beam" => if blink { 5 } else { 6 },
        _ => if blink { 5 } else { 6 },
    };
    execute!(out, Print(format!("\x1b[{} q", code)))?;
    Ok(())
}

fn render_window(f: &mut Frame, app: &mut AppState, area: Rect) {
    let win = &mut app.windows[app.active_idx];
    render_node(f, &mut win.root, &win.active_path, &mut Vec::new(), area);
}

fn enter_copy_mode(app: &mut AppState) { app.mode = Mode::CopyMode; }

fn cycle_top_layout(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    // toggle parent of active path, else toggle root
    if !win.active_path.is_empty() {
        let parent_path = &win.active_path[..win.active_path.len()-1].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path.to_vec()) {
            *kind = match *kind { LayoutKind::Horizontal => LayoutKind::Vertical, LayoutKind::Vertical => LayoutKind::Horizontal };
            *sizes = vec![50,50];
        }
    } else {
        if let Node::Split { kind, sizes, .. } = &mut win.root { *kind = match *kind { LayoutKind::Horizontal => LayoutKind::Vertical, LayoutKind::Vertical => LayoutKind::Horizontal }; *sizes = vec![50,50]; }
    }
}

fn render_node(f: &mut Frame, node: &mut Node, active_path: &Vec<usize>, cur_path: &mut Vec<usize>, area: Rect) {
    match node {
        Node::Leaf(pane) => {
            let is_active = *cur_path == *active_path;
            let title = if is_active { "* pane" } else { " pane" };
            let pane_block = Block::default().borders(Borders::ALL).title(title);
            let inner = pane_block.inner(area);
            let target_rows = inner.height.max(1);
            let target_cols = inner.width.max(1);
            if pane.last_rows != target_rows || pane.last_cols != target_cols {
                let _ = pane.master.resize(PtySize { rows: target_rows, cols: target_cols, pixel_width: 0, pixel_height: 0 });
                let mut parser = pane.term.lock().unwrap();
                parser.screen_mut().set_size(target_rows, target_cols);
                pane.last_rows = target_rows;
                pane.last_cols = target_cols;
            }
            let parser = pane.term.lock().unwrap();
            let screen = parser.screen();
            let mut lines: Vec<Line> = Vec::with_capacity(target_rows as usize);
            for r in 0..target_rows {
                let mut spans: Vec<Span> = Vec::with_capacity(target_cols as usize);
                let mut c = 0;
                while c < target_cols {
                    if let Some(cell) = screen.cell(r, c) {
                        let mut fg = vt_to_color(cell.fgcolor());
                        let mut bg = vt_to_color(cell.bgcolor());
                        if cell.inverse() { std::mem::swap(&mut fg, &mut bg); }
                        let mut style = Style::default().fg(fg).bg(bg);
                        if cell.bold() { style = style.add_modifier(Modifier::BOLD); }
                        if cell.italic() { style = style.add_modifier(Modifier::ITALIC); }
                        if cell.underline() { style = style.add_modifier(Modifier::UNDERLINED); }
                        let text = cell.contents().to_string();
                        let w = UnicodeWidthStr::width(text.as_str()) as u16;
                        if w == 0 {
                            spans.push(Span::styled(" ", style));
                            c += 1;
                        } else if w >= 2 && c + 1 < target_cols {
                            spans.push(Span::styled(text, style));
                            spans.push(Span::styled(" ", style));
                            c += 2;
                        } else {
                            spans.push(Span::styled(text, style));
                            c += 1;
                        }
                    } else {
                        spans.push(Span::raw(" "));
                        c += 1;
                    }
                }
                lines.push(Line::from(spans));
            }
            f.render_widget(pane_block, area);
            f.render_widget(Clear, inner);
            let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
            f.render_widget(para, inner);
            if is_active {
                let (cr, cc) = screen.cursor_position();
                let cr = cr.min(target_rows.saturating_sub(1));
                let cc = cc.min(target_cols.saturating_sub(1));
                let cx = inner.x + cc;
                let cy = inner.y + cr;
                f.set_cursor(cx, cy);
            }
        }
        Node::Split { kind, sizes, children } => {
            let constraints: Vec<Constraint> = if sizes.len() == children.len() {
                sizes.iter().map(|p| Constraint::Percentage(*p)).collect()
            } else { vec![Constraint::Percentage((100 / children.len() as u16) as u16); children.len()] };
            let rects = match *kind {
                LayoutKind::Horizontal => Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area),
                LayoutKind::Vertical => Layout::default().direction(Direction::Vertical).constraints(constraints).split(area),
            };
            for (i, child) in children.iter_mut().enumerate() {
                cur_path.push(i);
                render_node(f, child, active_path, cur_path, rects[i]);
                cur_path.pop();
            }
        }
    }
}

fn active_pane_mut<'a>(node: &'a mut Node, path: &Vec<usize>) -> Option<&'a mut Pane> {
    let mut cur = node;
    for &idx in path.iter() {
        match cur {
            Node::Split { children, .. } => { cur = children.get_mut(idx)?; }
            Node::Leaf(_) => return None,
        }
    }
    match cur { Node::Leaf(p) => Some(p), _ => None }
}

fn replace_leaf_with_split(node: &mut Node, path: &Vec<usize>, kind: LayoutKind, new_leaf: Node) {
    if path.is_empty() {
        let old = std::mem::replace(node, Node::Split { kind, sizes: vec![50,50], children: vec![] });
        if let Node::Split { children, .. } = node { children.push(old); children.push(new_leaf); }
        return;
    }
    let mut cur = node;
    for (depth, &idx) in path.iter().enumerate() {
        match cur {
            Node::Split { children, .. } => {
                if depth == path.len()-1 {
                    let leaf = std::mem::replace(&mut children[idx], Node::Split { kind, sizes: vec![50,50], children: vec![] });
                    if let Node::Split { children: c, .. } = &mut children[idx] { c.push(leaf); c.push(new_leaf); }
                    return;
                } else { cur = &mut children[idx]; }
            }
            Node::Leaf(_) => return,
        }
    }
}

fn kill_leaf(node: &mut Node, path: &Vec<usize>) {
    *node = remove_node(std::mem::replace(node, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] }), path);
}

fn remove_node(n: Node, path: &Vec<usize>) -> Node {
    match n {
        Node::Leaf(p) => {
            // if path points here, removing leaf yields an empty split collapse handled by parent; return leaf
            Node::Leaf(p)
        }
        Node::Split { kind, sizes, children } => {
            if path.is_empty() { return Node::Split { kind, sizes, children }; }
            let idx = path[0];
            let mut new_children: Vec<Node> = Vec::new();
            for (i, child) in children.into_iter().enumerate() {
                if i == idx {
                    if path.len() > 1 { new_children.push(remove_node(child, &path[1..].to_vec())); }
                    // else: drop this child (removed)
                } else { new_children.push(child); }
            }
            if new_children.len() == 1 { new_children.into_iter().next().unwrap() }
            else {
                // normalize sizes equally
                let mut eq = vec![100 / new_children.len() as u16; new_children.len()];
                let rem = 100 - eq.iter().sum::<u16>();
                if let Some(last) = eq.last_mut() { *last += rem; }
                Node::Split { kind, sizes: eq, children: new_children }
            }
        }
    }
}

fn compute_rects(node: &Node, area: Rect, out: &mut Vec<(Vec<usize>, Rect)>) {
    fn rec(node: &Node, area: Rect, path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, Rect)>) {
        match node {
            Node::Leaf(_) => { out.push((path.clone(), area)); }
            Node::Split { kind, sizes, children } => {
                let constraints: Vec<Constraint> = if sizes.len() == children.len() {
                    sizes.iter().map(|p| Constraint::Percentage(*p)).collect()
                } else { vec![Constraint::Percentage((100 / children.len() as u16) as u16); children.len()] };
                let rects = match *kind {
                    LayoutKind::Horizontal => Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area),
                    LayoutKind::Vertical => Layout::default().direction(Direction::Vertical).constraints(constraints).split(area),
                };
                for (i, child) in children.iter().enumerate() { path.push(i); rec(child, rects[i], path, out); path.pop(); }
            }
        }
    }
    let mut path = Vec::new();
    rec(node, area, &mut path, out);
}

fn kill_all_children(node: &mut Node) {
    match node {
        Node::Leaf(p) => { let _ = p.child.kill(); }
        Node::Split { children, .. } => { for child in children.iter_mut() { kill_all_children(child); } }
    }
}

fn compute_split_borders(node: &Node, area: Rect, out: &mut Vec<(Vec<usize>, LayoutKind, usize, u16)>) {
    fn rec(node: &Node, area: Rect, path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, LayoutKind, usize, u16)>) {
        match node {
            Node::Leaf(_) => {}
            Node::Split { kind, sizes, children } => {
                let constraints: Vec<Constraint> = if sizes.len() == children.len() {
                    sizes.iter().map(|p| Constraint::Percentage(*p)).collect()
                } else { vec![Constraint::Percentage((100 / children.len() as u16) as u16); children.len()] };
                let rects = match *kind {
                    LayoutKind::Horizontal => Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area),
                    LayoutKind::Vertical => Layout::default().direction(Direction::Vertical).constraints(constraints).split(area),
                };
                for i in 0..children.len()-1 {
                    let pos = match *kind {
                        LayoutKind::Horizontal => rects[i].x + rects[i].width,
                        LayoutKind::Vertical => rects[i].y + rects[i].height,
                    };
                    out.push((path.clone(), *kind, i, pos));
                }
                for (i, child) in children.iter().enumerate() { path.push(i); rec(child, rects[i], path, out); path.pop(); }
            }
        }
    }
    let mut path = Vec::new();
    rec(node, area, &mut path, out);
}

fn split_sizes_at<'a>(node: &'a Node, path: Vec<usize>, idx: usize) -> Option<(u16,u16)> {
    let mut cur = node;
    for &i in path.iter() {
        match cur { Node::Split { children, .. } => { cur = children.get(i)?; } _ => return None }
    }
    if let Node::Split { sizes, .. } = cur {
        if idx+1 < sizes.len() { Some((sizes[idx], sizes[idx+1])) } else { None }
    } else { None }
}

fn adjust_split_sizes(root: &mut Node, d: &DragState, x: u16, y: u16) {
    if let Some(Node::Split { sizes, .. }) = get_split_mut(root, &d.split_path) {
        let total = sizes[d.index] + sizes[d.index+1];
        let min_pct = 5u16;
        let delta: i16 = match d.kind {
            LayoutKind::Horizontal => (x as i32 - d.start_x as i32).clamp(-100, 100) as i16,
            LayoutKind::Vertical => (y as i32 - d.start_y as i32).clamp(-100, 100) as i16,
        };
        let left = (d.left_initial as i16 + delta).clamp(min_pct as i16, (total - min_pct) as i16) as u16;
        let right = total - left;
        sizes[d.index] = left;
        sizes[d.index+1] = right;
    }
}

fn get_split_mut<'a>(node: &'a mut Node, path: &Vec<usize>) -> Option<&'a mut Node> {
    let mut cur = node;
    for &idx in path.iter() {
        match cur { Node::Split { children, .. } => { cur = children.get_mut(idx)?; } _ => return None }
    }
    Some(cur)
}

fn prune_exited(n: Node) -> Option<Node> {
    match n {
        Node::Leaf(mut p) => {
            match p.child.try_wait() {
                Ok(Some(_)) => None,
                _ => Some(Node::Leaf(p)),
            }
        }
        Node::Split { kind, sizes: _sizes, children } => {
            let mut new_children: Vec<Node> = Vec::new();
            for child in children { if let Some(c) = prune_exited(child) { new_children.push(c); } }
            if new_children.is_empty() { None }
            else if new_children.len() == 1 { Some(new_children.remove(0)) }
            else {
                // equalize sizes
                let mut eq = vec![100 / new_children.len() as u16; new_children.len()];
                let rem = 100 - eq.iter().sum::<u16>();
                if let Some(last) = eq.last_mut() { *last += rem; }
                Some(Node::Split { kind, sizes: eq, children: new_children })
            }
        }
    }
}

fn expand_status(fmt: &str, app: &AppState, time_str: &str) -> String {
    let mut s = fmt.to_string();
    let window = &app.windows[app.active_idx];
    s = s.replace("#I", &(app.active_idx + 1).to_string());
    s = s.replace("#W", &window.name);
    s = s.replace("#S", "psmux");
    s = s.replace("%H:%M", time_str);
    s
}

/// Parse a key string like "C-a", "M-x", "F1", "Space" into (KeyCode, KeyModifiers)
fn parse_key_string(key: &str) -> Option<(KeyCode, KeyModifiers)> {
    let key = key.trim();
    let mut mods = KeyModifiers::empty();
    let mut key_part = key;
    
    // Handle prefix modifiers: C- (Ctrl), M- (Alt/Meta), S- (Shift)
    while key_part.len() > 2 {
        if key_part.starts_with("C-") || key_part.starts_with("c-") {
            mods |= KeyModifiers::CONTROL;
            key_part = &key_part[2..];
        } else if key_part.starts_with("M-") || key_part.starts_with("m-") {
            mods |= KeyModifiers::ALT;
            key_part = &key_part[2..];
        } else if key_part.starts_with("S-") || key_part.starts_with("s-") {
            mods |= KeyModifiers::SHIFT;
            key_part = &key_part[2..];
        } else {
            break;
        }
    }
    
    // Parse the key code
    let keycode = match key_part.to_lowercase().as_str() {
        "a" => KeyCode::Char('a'),
        "b" => KeyCode::Char('b'),
        "c" => KeyCode::Char('c'),
        "d" => KeyCode::Char('d'),
        "e" => KeyCode::Char('e'),
        "f" => KeyCode::Char('f'),
        "g" => KeyCode::Char('g'),
        "h" => KeyCode::Char('h'),
        "i" => KeyCode::Char('i'),
        "j" => KeyCode::Char('j'),
        "k" => KeyCode::Char('k'),
        "l" => KeyCode::Char('l'),
        "m" => KeyCode::Char('m'),
        "n" => KeyCode::Char('n'),
        "o" => KeyCode::Char('o'),
        "p" => KeyCode::Char('p'),
        "q" => KeyCode::Char('q'),
        "r" => KeyCode::Char('r'),
        "s" => KeyCode::Char('s'),
        "t" => KeyCode::Char('t'),
        "u" => KeyCode::Char('u'),
        "v" => KeyCode::Char('v'),
        "w" => KeyCode::Char('w'),
        "x" => KeyCode::Char('x'),
        "y" => KeyCode::Char('y'),
        "z" => KeyCode::Char('z'),
        "0" => KeyCode::Char('0'),
        "1" => KeyCode::Char('1'),
        "2" => KeyCode::Char('2'),
        "3" => KeyCode::Char('3'),
        "4" => KeyCode::Char('4'),
        "5" => KeyCode::Char('5'),
        "6" => KeyCode::Char('6'),
        "7" => KeyCode::Char('7'),
        "8" => KeyCode::Char('8'),
        "9" => KeyCode::Char('9'),
        "space" => KeyCode::Char(' '),
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "escape" | "esc" => KeyCode::Esc,
        "backspace" | "bspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "ppage" => KeyCode::PageUp,
        "pagedown" | "npage" => KeyCode::PageDown,
        "insert" | "ic" => KeyCode::Insert,
        "delete" | "dc" => KeyCode::Delete,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        "\"" => KeyCode::Char('"'),
        "%" => KeyCode::Char('%'),
        "," => KeyCode::Char(','),
        "." => KeyCode::Char('.'),
        ":" => KeyCode::Char(':'),
        ";" => KeyCode::Char(';'),
        "[" => KeyCode::Char('['),
        "]" => KeyCode::Char(']'),
        "{" => KeyCode::Char('{'),
        "}" => KeyCode::Char('}'),
        _ => {
            // Single character
            if key_part.len() == 1 {
                KeyCode::Char(key_part.chars().next().unwrap())
            } else {
                return None;
            }
        }
    };
    
    Some((keycode, mods))
}

/// Format a key binding back to string representation
fn format_key_binding(key: &(KeyCode, KeyModifiers)) -> String {
    let (keycode, mods) = key;
    let mut result = String::new();
    
    if mods.contains(KeyModifiers::CONTROL) {
        result.push_str("C-");
    }
    if mods.contains(KeyModifiers::ALT) {
        result.push_str("M-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        result.push_str("S-");
    }
    
    let key_str = match keycode {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Esc => "Escape".to_string(),
        KeyCode::Backspace => "BSpace".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PPage".to_string(),
        KeyCode::PageDown => "NPage".to_string(),
        KeyCode::Insert => "IC".to_string(),
        KeyCode::Delete => "DC".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => "?".to_string(),
    };
    
    result.push_str(&key_str);
    result
}

/// Parse a command string to an Action
fn parse_command_to_action(cmd: &str) -> Option<Action> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() { return None; }
    
    match parts[0] {
        "display-panes" => Some(Action::DisplayPanes),
        "select-pane" => {
            if parts.iter().any(|p| *p == "-U") {
                Some(Action::MoveFocus(FocusDir::Up))
            } else if parts.iter().any(|p| *p == "-D") {
                Some(Action::MoveFocus(FocusDir::Down))
            } else if parts.iter().any(|p| *p == "-L") {
                Some(Action::MoveFocus(FocusDir::Left))
            } else if parts.iter().any(|p| *p == "-R") {
                Some(Action::MoveFocus(FocusDir::Right))
            } else {
                None
            }
        }
        _ => None  // Other commands would need more Action variants
    }
}

/// Format an Action back to a command string
fn format_action(action: &Action) -> String {
    match action {
        Action::DisplayPanes => "display-panes".to_string(),
        Action::MoveFocus(dir) => {
            let flag = match dir {
                FocusDir::Up => "-U",
                FocusDir::Down => "-D",
                FocusDir::Left => "-L",
                FocusDir::Right => "-R",
            };
            format!("select-pane {}", flag)
        }
    }
}

fn load_config(app: &mut AppState) {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    // Try multiple config file locations
    let paths = vec![
        format!("{}\\.psmux.conf", home),
        format!("{}\\.psmuxrc", home),
        format!("{}\\.tmux.conf", home),  // For compatibility
        format!("{}\\.config\\psmux\\psmux.conf", home),
    ];
    for path in paths {
        if let Ok(content) = std::fs::read_to_string(&path) {
            parse_config_content(app, &content);
            break; // Only load the first config found
        }
    }
}

fn parse_config_content(app: &mut AppState, content: &str) {
    for line in content.lines() {
        parse_config_line(app, line);
    }
}

fn parse_config_line(app: &mut AppState, line: &str) {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') { return; }
    
    // Handle command continuation with backslash
    let l = if l.ends_with('\\') {
        l.trim_end_matches('\\').trim()
    } else {
        l
    };
    
    // Parse set-option / set
    if l.starts_with("set-option ") || l.starts_with("set ") {
        parse_set_option(app, l);
    }
    // Handle shorthand: set -g
    else if l.starts_with("set -g ") {
        let rest = &l[7..];
        parse_option_value(app, rest, true);
    }
    // Handle bind-key / bind
    else if l.starts_with("bind-key ") || l.starts_with("bind ") {
        parse_bind_key(app, l);
    }
    // Handle unbind-key / unbind
    else if l.starts_with("unbind-key ") || l.starts_with("unbind ") {
        parse_unbind_key(app, l);
    }
    // Handle source-file / source
    else if l.starts_with("source-file ") || l.starts_with("source ") {
        let parts: Vec<&str> = l.splitn(2, ' ').collect();
        if parts.len() > 1 {
            source_file(app, parts[1].trim());
        }
    }
}

fn parse_set_option(app: &mut AppState, line: &str) {
    // Parse: set-option [-agopqsuw] [-t target] option [value]
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 { return; }
    
    let mut i = 1; // Skip "set" or "set-option"
    let mut is_global = false;
    
    // Parse flags
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('g') { is_global = true; }
            i += 1;
            // Skip -t target
            if p.contains('t') && i < parts.len() { i += 1; }
        } else {
            break;
        }
    }
    
    if i < parts.len() {
        let rest = parts[i..].join(" ");
        parse_option_value(app, &rest, is_global);
    }
}

fn parse_option_value(app: &mut AppState, rest: &str, _is_global: bool) {
    // Split option and value
    let parts: Vec<&str> = rest.splitn(2, ' ').collect();
    if parts.is_empty() { return; }
    
    let key = parts[0].trim();
    let value = if parts.len() > 1 { 
        parts[1].trim().trim_matches('"').trim_matches('\'')
    } else { 
        "" 
    };
    
    match key {
        // Session options
        "status-left" => app.status_left = value.to_string(),
        "status-right" => app.status_right = value.to_string(),
        "mouse" => app.mouse_enabled = matches!(value, "on" | "true" | "1"),
        "prefix" => {
            if let Some(key) = parse_key_name(value) {
                app.prefix_key = key;
            }
        }
        "prefix2" => {
            // Could store secondary prefix if needed
        }
        "escape-time" => {
            if let Ok(ms) = value.parse::<u64>() {
                app.escape_time_ms = ms;
            }
        }
        // Cursor options (via env vars for terminal)
        "cursor-style" => env::set_var("PSMUX_CURSOR_STYLE", value),
        "cursor-blink" => env::set_var("PSMUX_CURSOR_BLINK", if matches!(value, "on"|"true"|"1") { "1" } else { "0" }),
        // Status line options
        "status" => {
            // on/off/2/3/4/5
        }
        "status-style" => {
            // bg=color,fg=color
        }
        "status-position" => {
            // top/bottom
        }
        "status-interval" => {
            // seconds
        }
        "status-justify" => {
            // left/centre/right
        }
        // Window options
        "base-index" => {
            // Starting window index
        }
        "renumber-windows" => {
            // on/off
        }
        "mode-keys" => {
            // vi/emacs - for copy mode
        }
        "status-keys" => {
            // vi/emacs - for command prompt
        }
        // History
        "history-limit" => {
            // lines
        }
        // Window/pane appearance
        "pane-border-style" => {}
        "pane-active-border-style" => {}
        "window-status-format" => {}
        "window-status-current-format" => {}
        "window-status-separator" => {}
        // Behavior
        "remain-on-exit" => {}
        "set-titles" => {}
        "set-titles-string" => {}
        "automatic-rename" => {}
        "allow-rename" => {}
        _ => {}
    }
}

fn parse_bind_key(app: &mut AppState, line: &str) {
    // Parse: bind-key [-cnr] [-T key-table] key command [arguments]
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 { return; }
    
    let mut i = 1; // Skip "bind" or "bind-key"
    let mut _key_table = "prefix".to_string(); // default table
    let mut _repeatable = false;
    
    // Parse flags
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('r') { _repeatable = true; }
            if p.contains('n') { _key_table = "root".to_string(); }
            if p.contains('T') {
                i += 1;
                if i < parts.len() { _key_table = parts[i].to_string(); }
            }
            i += 1;
        } else {
            break;
        }
    }
    
    if i >= parts.len() { return; }
    let key_str = parts[i];
    i += 1;
    
    if i >= parts.len() { return; }
    let command = parts[i..].join(" ");
    
    if let Some(key) = parse_key_name(key_str) {
        // Store the binding - for now we just support display-panes and focus commands
        let action = match command.as_str() {
            "display-panes" => Some(Action::DisplayPanes),
            s if s.starts_with("select-pane -U") => Some(Action::MoveFocus(FocusDir::Up)),
            s if s.starts_with("select-pane -D") => Some(Action::MoveFocus(FocusDir::Down)),
            s if s.starts_with("select-pane -L") => Some(Action::MoveFocus(FocusDir::Left)),
            s if s.starts_with("select-pane -R") => Some(Action::MoveFocus(FocusDir::Right)),
            _ => None,
        };
        if let Some(act) = action {
            app.binds.push(Bind { key, action: act });
        }
    }
}

fn parse_unbind_key(app: &mut AppState, line: &str) {
    // Parse: unbind-key [-an] [-T key-table] key
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 { return; }
    
    let mut i = 1;
    let mut unbind_all = false;
    
    while i < parts.len() {
        let p = parts[i];
        if p.starts_with('-') {
            if p.contains('a') { unbind_all = true; }
            if p.contains('T') { i += 1; } // Skip table name
            i += 1;
        } else {
            break;
        }
    }
    
    if unbind_all {
        app.binds.clear();
        return;
    }
    
    if i < parts.len() {
        if let Some(key) = parse_key_name(parts[i]) {
            app.binds.retain(|b| b.key != key);
        }
    }
}

fn parse_key_name(name: &str) -> Option<(KeyCode, KeyModifiers)> {
    let name = name.trim();
    
    // Handle Ctrl combinations: C-x, ^x
    if name.starts_with("C-") || name.starts_with("^") {
        let ch = if name.starts_with("C-") {
            name.chars().nth(2)
        } else {
            name.chars().nth(1)
        };
        if let Some(c) = ch {
            return Some((KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL));
        }
    }
    
    // Handle Meta/Alt combinations: M-x
    if name.starts_with("M-") {
        if let Some(c) = name.chars().nth(2) {
            return Some((KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::ALT));
        }
    }
    
    // Handle Shift combinations: S-x
    if name.starts_with("S-") {
        if let Some(c) = name.chars().nth(2) {
            return Some((KeyCode::Char(c.to_ascii_uppercase()), KeyModifiers::SHIFT));
        }
    }
    
    // Handle special keys
    match name.to_uppercase().as_str() {
        "ENTER" => return Some((KeyCode::Enter, KeyModifiers::NONE)),
        "TAB" => return Some((KeyCode::Tab, KeyModifiers::NONE)),
        "BTAB" => return Some((KeyCode::BackTab, KeyModifiers::NONE)),
        "ESCAPE" | "ESC" => return Some((KeyCode::Esc, KeyModifiers::NONE)),
        "SPACE" => return Some((KeyCode::Char(' '), KeyModifiers::NONE)),
        "BSPACE" | "BACKSPACE" => return Some((KeyCode::Backspace, KeyModifiers::NONE)),
        "UP" => return Some((KeyCode::Up, KeyModifiers::NONE)),
        "DOWN" => return Some((KeyCode::Down, KeyModifiers::NONE)),
        "LEFT" => return Some((KeyCode::Left, KeyModifiers::NONE)),
        "RIGHT" => return Some((KeyCode::Right, KeyModifiers::NONE)),
        "HOME" => return Some((KeyCode::Home, KeyModifiers::NONE)),
        "END" => return Some((KeyCode::End, KeyModifiers::NONE)),
        "PAGEUP" | "PPAGE" | "PGUP" => return Some((KeyCode::PageUp, KeyModifiers::NONE)),
        "PAGEDOWN" | "NPAGE" | "PGDN" => return Some((KeyCode::PageDown, KeyModifiers::NONE)),
        "INSERT" | "IC" => return Some((KeyCode::Insert, KeyModifiers::NONE)),
        "DELETE" | "DC" => return Some((KeyCode::Delete, KeyModifiers::NONE)),
        "F1" => return Some((KeyCode::F(1), KeyModifiers::NONE)),
        "F2" => return Some((KeyCode::F(2), KeyModifiers::NONE)),
        "F3" => return Some((KeyCode::F(3), KeyModifiers::NONE)),
        "F4" => return Some((KeyCode::F(4), KeyModifiers::NONE)),
        "F5" => return Some((KeyCode::F(5), KeyModifiers::NONE)),
        "F6" => return Some((KeyCode::F(6), KeyModifiers::NONE)),
        "F7" => return Some((KeyCode::F(7), KeyModifiers::NONE)),
        "F8" => return Some((KeyCode::F(8), KeyModifiers::NONE)),
        "F9" => return Some((KeyCode::F(9), KeyModifiers::NONE)),
        "F10" => return Some((KeyCode::F(10), KeyModifiers::NONE)),
        "F11" => return Some((KeyCode::F(11), KeyModifiers::NONE)),
        "F12" => return Some((KeyCode::F(12), KeyModifiers::NONE)),
        _ => {}
    }
    
    // Single character
    if name.len() == 1 {
        if let Some(c) = name.chars().next() {
            return Some((KeyCode::Char(c), KeyModifiers::NONE));
        }
    }
    
    None
}

fn source_file(app: &mut AppState, path: &str) {
    let path = path.trim().trim_matches('"').trim_matches('\'');
    // Expand ~ to home directory
    let expanded = if path.starts_with('~') {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
        path.replacen('~', &home, 1)
    } else {
        path.to_string()
    };
    
    if let Ok(content) = std::fs::read_to_string(&expanded) {
        parse_config_content(app, &content);
    }
}

fn parse_status(fmt: &str, app: &AppState, time_str: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur_style = Style::default();
    let mut i = 0;
    while i < fmt.len() {
        if fmt.as_bytes()[i] == b'#' && i + 1 < fmt.len() && fmt.as_bytes()[i+1] == b'[' {
            // parse style token #[...]
            if let Some(end) = fmt[i+2..].find(']') { 
                let token = &fmt[i+2..i+2+end];
                for part in token.split(',') {
                    let p = part.trim();
                    if p.starts_with("fg=") { cur_style = cur_style.fg(map_color(&p[3..])); }
                    else if p.starts_with("bg=") { cur_style = cur_style.bg(map_color(&p[3..])); }
                    else if p == "bold" { cur_style = cur_style.add_modifier(Modifier::BOLD); }
                    else if p == "italic" { cur_style = cur_style.add_modifier(Modifier::ITALIC); }
                    else if p == "underline" { cur_style = cur_style.add_modifier(Modifier::UNDERLINED); }
                    else if p == "default" { cur_style = Style::default(); }
                }
                i += 2 + end + 1; 
                continue;
            }
        }
        // regular text, expand placeholders
        let mut j = i;
        while j < fmt.len() && !(fmt.as_bytes()[j] == b'#' && j + 1 < fmt.len() && fmt.as_bytes()[j+1] == b'[') { j += 1; }
        let chunk = &fmt[i..j];
        let text = expand_status(chunk, app, time_str);
        spans.push(Span::styled(text, cur_style));
        i = j;
    }
    spans
}

fn map_color(name: &str) -> Color {
    match name.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "default" => Color::Reset,
        _ => Color::Reset,
    }
}

fn current_prompt_pos(app: &mut AppState) -> Option<(u16,u16)> {
    let win = &mut app.windows[app.active_idx];
    let p = active_pane_mut(&mut win.root, &win.active_path)?;
    let parser = p.term.lock().ok()?;
    let (r,c) = parser.screen().cursor_position();
    Some((r,c))
}

fn move_copy_cursor(app: &mut AppState, dx: i16, dy: i16) {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    // track copy position externally (no direct cursor mutation)
    let (r,c) = parser.screen().cursor_position();
    let nr = (r as i16 + dy).max(0) as u16;
    let nc = (c as i16 + dx).max(0) as u16;
    app.copy_pos = Some((nr,nc));
}

fn yank_selection(app: &mut AppState) -> io::Result<()> {
    let (anchor, pos) = match (app.copy_anchor, app.copy_pos) { (Some(a), Some(p)) => (a,p), _ => return Ok(()) };
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(()) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(()) };
    let screen = parser.screen();
    let r0 = anchor.0.min(pos.0); let r1 = anchor.0.max(pos.0);
    let c0 = anchor.1.min(pos.1); let c1 = anchor.1.max(pos.1);
    let mut text = String::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            if let Some(cell) = screen.cell(r, c) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); }
        }
        if r < r1 { text.push('\n'); }
    }
    app.paste_buffers.push(text);
    Ok(())
}

fn paste_latest(app: &mut AppState) -> io::Result<()> {
    if let Some(buf) = app.paste_buffers.last() {
        let win = &mut app.windows[app.active_idx];
        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.master, "{}", buf); }
    }
    Ok(())
}

fn path_exists(node: &Node, path: &Vec<usize>) -> bool {
    let mut cur = node;
    for &idx in path.iter() {
        match cur {
            Node::Split { children, .. } => {
                if let Some(next) = children.get(idx) { cur = next; } else { return false; }
            }
            Node::Leaf(_) => return false,
        }
    }
    matches!(cur, Node::Leaf(_) | Node::Split { .. })
}

fn first_leaf_path(node: &Node) -> Vec<usize> {
    fn rec(n: &Node, path: &mut Vec<usize>) -> Option<Vec<usize>> {
        match n {
            Node::Leaf(_) => Some(path.clone()),
            Node::Split { children, .. } => {
                for (i, child) in children.iter().enumerate() {
                    path.push(i);
                    if let Some(p) = rec(child, path) { return Some(p); }
                    path.pop();
                }
                None
            }
        }
    }
    rec(node, &mut Vec::new()).unwrap_or_default()
}

fn capture_active_pane(app: &mut AppState) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(()) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(()) };
    let screen = parser.screen();
    let mut text = String::new();
    for r in 0..p.last_rows { for c in 0..p.last_cols { if let Some(cell) = screen.cell(r, c) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); } } text.push('\n'); }
    app.paste_buffers.push(text);
    Ok(())
}

fn capture_active_pane_text(app: &mut AppState) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let screen = parser.screen();
    let mut text = String::new();
    for r in 0..p.last_rows { for c in 0..p.last_cols { if let Some(cell) = screen.cell(r, c) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); } } text.push('\n'); }
    Ok(Some(text))
}

fn save_latest_buffer(app: &mut AppState, file: &str) -> io::Result<()> {
    if let Some(buf) = app.paste_buffers.last() { std::fs::write(file, buf)?; }
    Ok(())
}

#[derive(Clone)]
enum Action { DisplayPanes, MoveFocus(FocusDir) }

#[derive(Clone)]
struct Bind { key: (KeyCode, KeyModifiers), action: Action }
enum CtrlReq {
    NewWindow,
    SplitWindow(LayoutKind),
    KillPane,
    CapturePane(mpsc::Sender<String>),
    FocusWindow(usize),
    FocusPane(usize),
    SessionInfo(mpsc::Sender<String>),
    CapturePaneRange(mpsc::Sender<String>, Option<u16>, Option<u16>),
    ClientAttach,
    ClientDetach,
    DumpLayout(mpsc::Sender<String>),
    SendText(String),
    SendKey(String),
    ZoomPane,
    CopyEnter,
    CopyMove(i16, i16),
    CopyAnchor,
    CopyYank,
    ClientSize(u16, u16),
    FocusPaneCmd(usize),
    FocusWindowCmd(usize),
    MouseDown(u16,u16),
    MouseDrag(u16,u16),
    MouseUp(u16,u16),
    ScrollUp,
    ScrollDown,
    NextWindow,
    PrevWindow,
    RenameWindow(String),
    ListWindows(mpsc::Sender<String>),
    ListTree(mpsc::Sender<String>),
    ToggleSync,
    SetPaneTitle(String),
    // New commands for tmux compatibility
    SendKeys(String, bool),  // (keys, literal_flag)
    SelectPane(String),      // direction: U/D/L/R or pane id
    SelectWindow(usize),     // window index
    ListPanes(mpsc::Sender<String>),
    KillWindow,
    KillSession,
    HasSession(mpsc::Sender<bool>),
    RenameSession(String),
    SwapPane(String),        // direction: U/D/L/R
    ResizePane(String, u16), // direction, amount
    SetBuffer(String),       // paste buffer content
    ListBuffers(mpsc::Sender<String>),
    ShowBuffer(mpsc::Sender<String>),
    DeleteBuffer,
    DisplayMessage(mpsc::Sender<String>, String), // format string
    LastWindow,
    LastPane,
    RotateWindow(bool),      // reverse flag
    DisplayPanes,
    BreakPane,
    JoinPane(usize),         // target window
    RespawnPane,
    // Config/Key binding commands
    BindKey(String, String),        // key, command
    UnbindKey(String),              // key
    ListKeys(mpsc::Sender<String>),
    SetOption(String, String),      // option, value
    ShowOptions(mpsc::Sender<String>),
    SourceFile(String),             // file path
    // Additional commands for full tmux parity
    MoveWindow(Option<usize>),      // target index
    SwapWindow(usize),              // target window index
    LinkWindow(String),             // target session
    UnlinkWindow,
    FindWindow(mpsc::Sender<String>, String), // pattern
    MovePane(usize),                // target window
    PipePane(String, bool, bool),   // command, stdin (-I), stdout (-O)
    SelectLayout(String),           // layout name
    NextLayout,
    ListClients(mpsc::Sender<String>),
    SwitchClient(String),           // target session
    LockClient,
    RefreshClient,
    SuspendClient,
    CopyModePageUp,
    ClearHistory,
    SaveBuffer(String),             // file path
    LoadBuffer(String),             // file path
    SetEnvironment(String, String), // key, value
    ShowEnvironment(mpsc::Sender<String>),
    SetHook(String, String),        // hook name, command
    ShowHooks(mpsc::Sender<String>),
    RemoveHook(String),             // hook name
    KillServer,
    WaitFor(String, WaitForOp),     // channel name, operation
    DisplayMenu(String, Option<i16>, Option<i16>),  // menu definition, x, y
    DisplayPopup(String, u16, u16, bool), // command, width, height, close_on_exit
    ConfirmBefore(String, String),  // prompt, command
}

/// Wait-for operation types
#[derive(Clone, Copy)]
enum WaitForOp {
    Wait,   // Default: wait for signal
    Lock,   // -L: lock channel
    Signal, // -S: signal/wake waiters
    Unlock, // -U: unlock channel
}

fn find_window_index_by_id(app: &AppState, wid: usize) -> Option<usize> {
    for (i, w) in app.windows.iter().enumerate() { if w.id == wid { return Some(i); } }
    None
}

fn focus_pane_by_id(app: &mut AppState, pid: usize) {
    let win = &mut app.windows[app.active_idx];
    fn rec(node: &Node, path: &mut Vec<usize>, found: &mut Option<Vec<usize>>, pid: usize) {
        match node {
            Node::Leaf(p) => { if p.id == pid { *found = Some(path.clone()); } }
            Node::Split { children, .. } => {
                for (i, c) in children.iter().enumerate() { path.push(i); rec(c, path, found, pid); path.pop(); }
            }
        }
    }
    let mut found = None;
    rec(&win.root, &mut Vec::new(), &mut found, pid);
    if let Some(p) = found { win.active_path = p; }
}
fn run_server(session_name: String) -> io::Result<()> {
    let pty_system = PtySystemSelection::default()
        .get()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;

    let mut app = AppState {
        windows: Vec::new(),
        active_idx: 0,
        mode: Mode::Passthrough,
        escape_time_ms: 500,
        prefix_key: (KeyCode::Char('b'), KeyModifiers::CONTROL),
        drag: None,
        last_window_area: Rect { x: 0, y: 0, width: 0, height: 0 },
        mouse_enabled: true,
        paste_buffers: Vec::new(),
        status_left: "psmux:#I".to_string(),
        status_right: "%H:%M".to_string(),
        copy_anchor: None,
        copy_pos: None,
        display_map: Vec::new(),
        binds: Vec::new(),
        control_rx: None,
        control_port: None,
        session_name,
        attached_clients: 0,
        created_at: Local::now(),
        next_win_id: 1,
        next_pane_id: 1,
        zoom_saved: None,
        sync_input: false,
        hooks: std::collections::HashMap::new(),
        wait_channels: std::collections::HashMap::new(),
        pipe_panes: Vec::new(),
        last_window_idx: 0,
        last_pane_path: Vec::new(),
    };
    create_window(&*pty_system, &mut app)?;
    let (tx, rx) = mpsc::channel::<CtrlReq>();
    app.control_rx = Some(rx);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    app.control_port = Some(port);
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let dir = format!("{}\\.psmux", home);
    let _ = std::fs::create_dir_all(&dir);
    let regpath = format!("{}\\{}.port", dir, app.session_name);
    let _ = std::fs::write(&regpath, port.to_string());
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(mut stream) = conn {
                let mut line = String::new();
                let mut r = io::BufReader::new(stream.try_clone().unwrap());
                let _ = r.read_line(&mut line);
                let mut parts = line.split_whitespace();
                let cmd = parts.next().unwrap_or("");
                let mut args: Vec<&str> = parts.by_ref().collect();
                let mut target_win: Option<usize> = None;
                let mut target_pane: Option<usize> = None;
                let mut i = 0;
                while i < args.len() {
                    if args[i] == "-t" {
                        if let Some(v) = args.get(i+1) {
                            if v.starts_with('%') { if let Ok(pid) = v[1..].parse::<usize>() { target_pane = Some(pid); } }
                            else if v.starts_with('@') { if let Ok(wid) = v[1..].parse::<usize>() { target_win = Some(wid); } }
                        }
                        i += 2; continue;
                    }
                    i += 1;
                }
                if let Some(wid) = target_win { let _ = tx.send(CtrlReq::FocusWindow(wid)); }
                if let Some(pid) = target_pane { let _ = tx.send(CtrlReq::FocusPane(pid)); }
                match cmd {
                    "new-window" => { let _ = tx.send(CtrlReq::NewWindow); }
                    "split-window" => {
                        let kind = if args.iter().any(|a| *a == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
                        let _ = tx.send(CtrlReq::SplitWindow(kind));
                    }
                    "kill-pane" => { let _ = tx.send(CtrlReq::KillPane); }
                    "capture-pane" => {
                        let print_stdout = args.iter().any(|a| *a == "-p");
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::CapturePane(rtx));
                        if let Ok(text) = rrx.recv() {
                            if print_stdout {
                                let _ = write!(stream, "{}", text);
                            } else {
                                // Save to buffer instead
                                let _ = tx.send(CtrlReq::SetBuffer(text));
                            }
                        }
                    }
                    "dump-layout" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::DumpLayout(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "send-text" => {
                        if let Some(payload) = args.get(0) { let _ = tx.send(CtrlReq::SendText(payload.to_string())); }
                    }
                    "send-key" => {
                        if let Some(payload) = args.get(0) { let _ = tx.send(CtrlReq::SendKey(payload.to_string())); }
                    }
                    "zoom-pane" => { let _ = tx.send(CtrlReq::ZoomPane); }
                    "copy-enter" => { let _ = tx.send(CtrlReq::CopyEnter); }
                    "copy-move" => {
                        if args.len() >= 2 { if let (Ok(dx), Ok(dy)) = (args[0].parse::<i16>(), args[1].parse::<i16>()) { let _ = tx.send(CtrlReq::CopyMove(dx, dy)); } }
                    }
                    "copy-anchor" => { let _ = tx.send(CtrlReq::CopyAnchor); }
                    "copy-yank" => { let _ = tx.send(CtrlReq::CopyYank); }
                    "client-size" => {
                        if args.len() >= 2 { if let (Ok(w), Ok(h)) = (args[0].parse::<u16>(), args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::ClientSize(w, h)); } }
                    }
                    "focus-pane" => {
                        if let Some(pid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = tx.send(CtrlReq::FocusPaneCmd(pid)); }
                    }
                    "focus-window" => {
                        if let Some(wid) = args.get(0).and_then(|s| s.parse::<usize>().ok()) { let _ = tx.send(CtrlReq::FocusWindowCmd(wid)); }
                    }
                    "mouse-down" => {
                        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDown(x,y)); } }
                    }
                    "mouse-drag" => {
                        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseDrag(x,y)); } }
                    }
                    "mouse-up" => {
                        if args.len()>=2 { if let (Ok(x),Ok(y))=(args[0].parse::<u16>(),args[1].parse::<u16>()) { let _ = tx.send(CtrlReq::MouseUp(x,y)); } }
                    }
                    "scroll-up" => { let _ = tx.send(CtrlReq::ScrollUp); }
                    "scroll-down" => { let _ = tx.send(CtrlReq::ScrollDown); }
                    "next-window" => { let _ = tx.send(CtrlReq::NextWindow); }
                    "previous-window" => { let _ = tx.send(CtrlReq::PrevWindow); }
                    "rename-window" => { if let Some(name) = args.get(0) { let _ = tx.send(CtrlReq::RenameWindow((*name).to_string())); } }
                    "list-windows" => { let (rtx, rrx) = mpsc::channel::<String>(); let _ = tx.send(CtrlReq::ListWindows(rtx)); if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); } }
                    "list-tree" => { let (rtx, rrx) = mpsc::channel::<String>(); let _ = tx.send(CtrlReq::ListTree(rtx)); if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); } }
                    "toggle-sync" => { let _ = tx.send(CtrlReq::ToggleSync); }
                    "set-pane-title" => { let title = args.join(" "); let _ = tx.send(CtrlReq::SetPaneTitle(title)); }
                    // New tmux-compatible commands
                    "send-keys" => {
                        let literal = args.iter().any(|a| *a == "-l");
                        let keys: Vec<&str> = args.iter().filter(|a| !a.starts_with('-') && **a != "-l" && **a != "-t").copied().collect();
                        let _ = tx.send(CtrlReq::SendKeys(keys.join(" "), literal));
                    }
                    "select-pane" => {
                        let dir = if args.iter().any(|a| *a == "-U") { "U" }
                            else if args.iter().any(|a| *a == "-D") { "D" }
                            else if args.iter().any(|a| *a == "-L") { "L" }
                            else if args.iter().any(|a| *a == "-R") { "R" }
                            else { "" };
                        let _ = tx.send(CtrlReq::SelectPane(dir.to_string()));
                    }
                    "select-window" => {
                        if let Some(idx) = args.iter().find(|a| !a.starts_with('-')).and_then(|s| s.parse::<usize>().ok()) {
                            let _ = tx.send(CtrlReq::SelectWindow(idx));
                        }
                    }
                    "list-panes" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ListPanes(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "kill-window" => { let _ = tx.send(CtrlReq::KillWindow); }
                    "kill-session" => { let _ = tx.send(CtrlReq::KillSession); }
                    "has-session" => {
                        let (rtx, rrx) = mpsc::channel::<bool>();
                        let _ = tx.send(CtrlReq::HasSession(rtx));
                        if let Ok(exists) = rrx.recv() {
                            if !exists { std::process::exit(1); }
                        }
                    }
                    "rename-session" => {
                        if let Some(name) = args.iter().find(|a| !a.starts_with('-')) {
                            let _ = tx.send(CtrlReq::RenameSession((*name).to_string()));
                        }
                    }
                    "swap-pane" => {
                        let dir = if args.iter().any(|a| *a == "-U") { "U" }
                            else if args.iter().any(|a| *a == "-D") { "D" }
                            else { "D" };
                        let _ = tx.send(CtrlReq::SwapPane(dir.to_string()));
                    }
                    "resize-pane" => {
                        let amount = args.iter().find(|a| a.parse::<u16>().is_ok()).and_then(|s| s.parse::<u16>().ok()).unwrap_or(1);
                        let dir = if args.iter().any(|a| *a == "-U") { "U" }
                            else if args.iter().any(|a| *a == "-D") { "D" }
                            else if args.iter().any(|a| *a == "-L") { "L" }
                            else if args.iter().any(|a| *a == "-R") { "R" }
                            else { "D" };
                        let _ = tx.send(CtrlReq::ResizePane(dir.to_string(), amount));
                    }
                    "set-buffer" => {
                        let content = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
                        let _ = tx.send(CtrlReq::SetBuffer(content));
                    }
                    "paste-buffer" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowBuffer(rtx));
                        if let Ok(text) = rrx.recv() {
                            let _ = tx.send(CtrlReq::SendText(text));
                        }
                    }
                    "list-buffers" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ListBuffers(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "show-buffer" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowBuffer(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "delete-buffer" => { let _ = tx.send(CtrlReq::DeleteBuffer); }
                    "display-message" => {
                        let fmt = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::DisplayMessage(rtx, fmt));
                        if let Ok(text) = rrx.recv() { let _ = writeln!(stream, "{}", text); }
                    }
                    "last-window" => { let _ = tx.send(CtrlReq::LastWindow); }
                    "last-pane" => { let _ = tx.send(CtrlReq::LastPane); }
                    "rotate-window" => {
                        let reverse = args.iter().any(|a| *a == "-D");
                        let _ = tx.send(CtrlReq::RotateWindow(reverse));
                    }
                    "display-panes" => { let _ = tx.send(CtrlReq::DisplayPanes); }
                    "break-pane" => { let _ = tx.send(CtrlReq::BreakPane); }
                    "join-pane" => {
                        if let Some(wid) = args.iter().find(|a| !a.starts_with('-')).and_then(|s| s.parse::<usize>().ok()) {
                            let _ = tx.send(CtrlReq::JoinPane(wid));
                        }
                    }
                    "respawn-pane" => { let _ = tx.send(CtrlReq::RespawnPane); }
                    "session-info" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::SessionInfo(rtx));
                        if let Ok(line) = rrx.recv() { let _ = write!(stream, "{}", line); let _ = stream.flush(); }
                    }
                    "client-attach" => { let _ = tx.send(CtrlReq::ClientAttach); let _ = write!(stream, "ok\n"); }
                    "client-detach" => { let _ = tx.send(CtrlReq::ClientDetach); let _ = write!(stream, "ok\n"); }
                    // Config/Key binding commands
                    "bind-key" | "bind" => {
                        // bind-key <key> <command>
                        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if non_flag_args.len() >= 2 {
                            let key = non_flag_args[0].to_string();
                            let command = non_flag_args[1..].join(" ");
                            let _ = tx.send(CtrlReq::BindKey(key, command));
                        }
                    }
                    "unbind-key" | "unbind" => {
                        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if let Some(key) = non_flag_args.first() {
                            let _ = tx.send(CtrlReq::UnbindKey(key.to_string()));
                        }
                    }
                    "list-keys" | "lsk" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ListKeys(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "set-option" | "set" => {
                        // set-option [-g] <option> <value>
                        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if non_flag_args.len() >= 2 {
                            let option = non_flag_args[0].to_string();
                            let value = non_flag_args[1..].join(" ");
                            let _ = tx.send(CtrlReq::SetOption(option, value));
                        }
                    }
                    "show-options" | "show" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowOptions(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "source-file" | "source" => {
                        let non_flag_args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if let Some(path) = non_flag_args.first() {
                            let _ = tx.send(CtrlReq::SourceFile(path.to_string()));
                        }
                    }
                    // Additional commands for full tmux parity
                    "move-window" => {
                        let target = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok());
                        let _ = tx.send(CtrlReq::MoveWindow(target));
                    }
                    "swap-window" => {
                        if let Some(target) = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok()) {
                            let _ = tx.send(CtrlReq::SwapWindow(target));
                        }
                    }
                    "link-window" => {
                        let target = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
                        let _ = tx.send(CtrlReq::LinkWindow(target));
                    }
                    "unlink-window" => {
                        let _ = tx.send(CtrlReq::UnlinkWindow);
                    }
                    "find-window" => {
                        let pattern = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::FindWindow(rtx, pattern));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "move-pane" => {
                        if let Some(target) = args.iter().find(|a| a.parse::<usize>().is_ok()).and_then(|s| s.parse().ok()) {
                            let _ = tx.send(CtrlReq::MovePane(target));
                        }
                    }
                    "pipe-pane" | "pipep" => {
                        // Parse -I (stdin to pane) and -O (pane to stdout) flags
                        let stdin_flag = args.iter().any(|a| *a == "-I");
                        let stdout_flag = args.iter().any(|a| *a == "-O");
                        let toggle = args.iter().any(|a| *a == "-o");
                        let cmd = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
                        // Default to -O if no flags specified (pipe pane output to command)
                        let (stdin, stdout) = if !stdin_flag && !stdout_flag {
                            (false, true)
                        } else {
                            (stdin_flag, stdout_flag)
                        };
                        let _ = tx.send(CtrlReq::PipePane(cmd, stdin, stdout));
                    }
                    "select-layout" => {
                        let layout = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"tiled").to_string();
                        let _ = tx.send(CtrlReq::SelectLayout(layout));
                    }
                    "next-layout" => {
                        let _ = tx.send(CtrlReq::NextLayout);
                    }
                    "list-clients" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ListClients(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "switch-client" => {
                        let target = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
                        let _ = tx.send(CtrlReq::SwitchClient(target));
                    }
                    "lock-client" => {
                        let _ = tx.send(CtrlReq::LockClient);
                    }
                    "refresh-client" => {
                        let _ = tx.send(CtrlReq::RefreshClient);
                    }
                    "suspend-client" => {
                        let _ = tx.send(CtrlReq::SuspendClient);
                    }
                    "copy-mode-page-up" => {
                        let _ = tx.send(CtrlReq::CopyModePageUp);
                    }
                    "clear-history" => {
                        let _ = tx.send(CtrlReq::ClearHistory);
                    }
                    "save-buffer" => {
                        let path = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
                        let _ = tx.send(CtrlReq::SaveBuffer(path));
                    }
                    "load-buffer" => {
                        let path = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
                        let _ = tx.send(CtrlReq::LoadBuffer(path));
                    }
                    "set-environment" => {
                        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if non_flag.len() >= 2 {
                            let _ = tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), non_flag[1].to_string()));
                        } else if non_flag.len() == 1 {
                            let _ = tx.send(CtrlReq::SetEnvironment(non_flag[0].to_string(), String::new()));
                        }
                    }
                    "show-environment" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowEnvironment(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "set-hook" => {
                        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
                        if non_flag.len() >= 2 {
                            let _ = tx.send(CtrlReq::SetHook(non_flag[0].to_string(), non_flag[1..].join(" ")));
                        }
                    }
                    "show-hooks" => {
                        let (rtx, rrx) = mpsc::channel::<String>();
                        let _ = tx.send(CtrlReq::ShowHooks(rtx));
                        if let Ok(text) = rrx.recv() { let _ = write!(stream, "{}", text); }
                    }
                    "wait-for" => {
                        let lock = args.iter().any(|a| *a == "-L");
                        let signal = args.iter().any(|a| *a == "-S");
                        let unlock = args.iter().any(|a| *a == "-U");
                        let channel = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"").to_string();
                        let op = if lock { WaitForOp::Lock }
                            else if signal { WaitForOp::Signal }
                            else if unlock { WaitForOp::Unlock }
                            else { WaitForOp::Wait };
                        let _ = tx.send(CtrlReq::WaitFor(channel, op));
                    }
                    "display-menu" | "menu" => {
                        // Parse menu definition: display-menu -x P -y P [-T title] name key command ...
                        let mut x_pos: Option<i16> = None;
                        let mut y_pos: Option<i16> = None;
                        let mut i = 0;
                        while i < args.len() {
                            match args[i] {
                                "-x" => { if let Some(v) = args.get(i+1) { x_pos = v.parse().ok(); i += 1; } }
                                "-y" => { if let Some(v) = args.get(i+1) { y_pos = v.parse().ok(); i += 1; } }
                                _ => {}
                            }
                            i += 1;
                        }
                        let menu = args.iter().filter(|a| !a.starts_with('-') && a.parse::<i16>().is_err()).cloned().collect::<Vec<&str>>().join(" ");
                        let _ = tx.send(CtrlReq::DisplayMenu(menu, x_pos, y_pos));
                    }
                    "display-popup" | "popup" => {
                        // Parse popup options: -E close on exit, -w width, -h height
                        let close_on_exit = args.iter().any(|a| *a == "-E");
                        let mut width: u16 = 80;
                        let mut height: u16 = 24;
                        let mut i = 0;
                        while i < args.len() {
                            match args[i] {
                                "-w" => { if let Some(v) = args.get(i+1) { width = v.trim_end_matches('%').parse().unwrap_or(80); i += 1; } }
                                "-h" => { if let Some(v) = args.get(i+1) { height = v.trim_end_matches('%').parse().unwrap_or(24); i += 1; } }
                                _ => {}
                            }
                            i += 1;
                        }
                        let content = args.iter().filter(|a| !a.starts_with('-')).cloned().collect::<Vec<&str>>().join(" ");
                        let _ = tx.send(CtrlReq::DisplayPopup(content, width, height, close_on_exit));
                    }
                    "confirm-before" | "confirm" => {
                        // Parse: confirm-before [-p prompt] command
                        let mut prompt: Option<String> = None;
                        let mut i = 0;
                        while i < args.len() {
                            if args[i] == "-p" {
                                if let Some(p) = args.get(i+1) { prompt = Some(p.to_string()); i += 1; }
                            }
                            i += 1;
                        }
                        let non_flag: Vec<&str> = args.iter().filter(|a| !a.starts_with('-') && Some(&a.to_string()) != prompt.as_ref()).copied().collect();
                        let command = non_flag.join(" ");
                        let prompt_str = prompt.unwrap_or_else(|| format!("Run '{}'?", command));
                        let _ = tx.send(CtrlReq::ConfirmBefore(prompt_str, command));
                    }
                    _ => {}
                }
            }
        }
    });
    loop {
        while let Some(req) = app.control_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match req {
                CtrlReq::NewWindow => { let _ = create_window(&*pty_system, &mut app); }
                CtrlReq::SplitWindow(k) => { let _ = split_active(&mut app, k); }
                CtrlReq::KillPane => { let _ = kill_active_pane(&mut app); }
                CtrlReq::CapturePane(resp) => {
                    if let Some(text) = capture_active_pane_text(&mut app)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::CapturePaneRange(resp, s, e) => {
                    if let Some(text) = capture_active_pane_range(&mut app, s, e)? { let _ = resp.send(text); } else { let _ = resp.send(String::new()); }
                }
                CtrlReq::FocusWindow(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::FocusPane(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::SessionInfo(resp) => {
                    let attached = if app.attached_clients > 0 { "(attached)" } else { "(detached)" };
                    let windows = app.windows.len();
                    let (w,h) = {
                        let win = &mut app.windows[app.active_idx];
                        let mut size = (0,0);
                        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { size = (p.last_cols as i32, p.last_rows as i32); }
                        size
                    };
                    let created = app.created_at.format("%a %b %e %H:%M:%S %Y");
                    let line = format!("{}: {} windows (created {}) [{}x{}] {}\n", app.session_name, windows, created, w, h, attached);
                    let _ = resp.send(line);
                }
                CtrlReq::ClientAttach => { app.attached_clients = app.attached_clients.saturating_add(1); }
                CtrlReq::ClientDetach => { app.attached_clients = app.attached_clients.saturating_sub(1); }
                CtrlReq::DumpLayout(resp) => {
                    let json = dump_layout_json(&mut app)?;
                    let _ = resp.send(json);
                }
                CtrlReq::SendText(s) => { send_text_to_active(&mut app, &s)?; }
                CtrlReq::SendKey(k) => { send_key_to_active(&mut app, &k)?; }
                CtrlReq::ZoomPane => { toggle_zoom(&mut app); }
                CtrlReq::CopyEnter => { enter_copy_mode(&mut app); }
                CtrlReq::CopyMove(dx, dy) => { move_copy_cursor(&mut app, dx, dy); }
                CtrlReq::CopyAnchor => { if let Some((r,c)) = current_prompt_pos(&mut app) { app.copy_anchor = Some((r,c)); app.copy_pos = Some((r,c)); } }
                CtrlReq::CopyYank => { let _ = yank_selection(&mut app); app.mode = Mode::Passthrough; }
                CtrlReq::ClientSize(w, h) => { app.last_window_area = Rect { x: 0, y: 0, width: w, height: h }; }
                CtrlReq::FocusPaneCmd(pid) => { focus_pane_by_id(&mut app, pid); }
                CtrlReq::FocusWindowCmd(wid) => { if let Some(idx) = find_window_index_by_id(&app, wid) { app.active_idx = idx; } }
                CtrlReq::MouseDown(x,y) => { remote_mouse_down(&mut app, x, y); }
                CtrlReq::MouseDrag(x,y) => { remote_mouse_drag(&mut app, x, y); }
                CtrlReq::MouseUp(_,_) => { app.drag = None; }
                CtrlReq::ScrollUp => { remote_scroll_up(&mut app); }
                CtrlReq::ScrollDown => { remote_scroll_down(&mut app); }
                CtrlReq::NextWindow => { if !app.windows.is_empty() { app.active_idx = (app.active_idx + 1) % app.windows.len(); } }
                CtrlReq::PrevWindow => { if !app.windows.is_empty() { app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len(); } }
                CtrlReq::RenameWindow(name) => { let win = &mut app.windows[app.active_idx]; win.name = name; }
                CtrlReq::ListWindows(resp) => { let json = list_windows_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ListTree(resp) => { let json = list_tree_json(&app)?; let _ = resp.send(json); }
                CtrlReq::ToggleSync => { app.sync_input = !app.sync_input; }
                CtrlReq::SetPaneTitle(title) => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { p.title = title; }
                }
                // New tmux-compatible command handlers
                CtrlReq::SendKeys(keys, literal) => {
                    if literal {
                        // Send as literal text
                        send_text_to_active(&mut app, &keys)?;
                    } else {
                        // Parse key names and send
                        let parts: Vec<&str> = keys.split_whitespace().collect();
                        for (i, key) in parts.iter().enumerate() {
                            let is_special = matches!(key.to_uppercase().as_str(), 
                                "ENTER" | "TAB" | "ESCAPE" | "ESC" | "SPACE" | "BSPACE" | "BACKSPACE" |
                                "UP" | "DOWN" | "RIGHT" | "LEFT" | "HOME" | "END" |
                                "PAGEUP" | "PPAGE" | "PAGEDOWN" | "NPAGE" | "DELETE" | "DC" | "INSERT" | "IC" |
                                "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
                            ) || key.to_uppercase().starts_with("C-");
                            
                            match key.to_uppercase().as_str() {
                                "ENTER" => send_text_to_active(&mut app, "\r")?,
                                "TAB" => send_text_to_active(&mut app, "\t")?,
                                "ESCAPE" | "ESC" => send_text_to_active(&mut app, "\x1b")?,
                                "SPACE" => send_text_to_active(&mut app, " ")?,
                                "BSPACE" | "BACKSPACE" => send_text_to_active(&mut app, "\x7f")?,
                                "UP" => send_text_to_active(&mut app, "\x1b[A")?,
                                "DOWN" => send_text_to_active(&mut app, "\x1b[B")?,
                                "RIGHT" => send_text_to_active(&mut app, "\x1b[C")?,
                                "LEFT" => send_text_to_active(&mut app, "\x1b[D")?,
                                "HOME" => send_text_to_active(&mut app, "\x1b[H")?,
                                "END" => send_text_to_active(&mut app, "\x1b[F")?,
                                "PAGEUP" | "PPAGE" => send_text_to_active(&mut app, "\x1b[5~")?,
                                "PAGEDOWN" | "NPAGE" => send_text_to_active(&mut app, "\x1b[6~")?,
                                "DELETE" | "DC" => send_text_to_active(&mut app, "\x1b[3~")?,
                                "INSERT" | "IC" => send_text_to_active(&mut app, "\x1b[2~")?,
                                "F1" => send_text_to_active(&mut app, "\x1bOP")?,
                                "F2" => send_text_to_active(&mut app, "\x1bOQ")?,
                                "F3" => send_text_to_active(&mut app, "\x1bOR")?,
                                "F4" => send_text_to_active(&mut app, "\x1bOS")?,
                                "F5" => send_text_to_active(&mut app, "\x1b[15~")?,
                                "F6" => send_text_to_active(&mut app, "\x1b[17~")?,
                                "F7" => send_text_to_active(&mut app, "\x1b[18~")?,
                                "F8" => send_text_to_active(&mut app, "\x1b[19~")?,
                                "F9" => send_text_to_active(&mut app, "\x1b[20~")?,
                                "F10" => send_text_to_active(&mut app, "\x1b[21~")?,
                                "F11" => send_text_to_active(&mut app, "\x1b[23~")?,
                                "F12" => send_text_to_active(&mut app, "\x1b[24~")?,
                                s if s.starts_with("C-") => {
                                    // Ctrl+key
                                    if let Some(c) = s.chars().nth(2) {
                                        let ctrl = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
                                        send_text_to_active(&mut app, &String::from(ctrl as char))?;
                                    }
                                }
                                _ => {
                                    send_text_to_active(&mut app, key)?;
                                    // Add space after non-special keys unless it's the last one before a special key
                                    if i + 1 < parts.len() {
                                        let next_is_special = matches!(parts[i + 1].to_uppercase().as_str(),
                                            "ENTER" | "TAB" | "ESCAPE" | "ESC" | "SPACE" | "BSPACE" | "BACKSPACE" |
                                            "UP" | "DOWN" | "RIGHT" | "LEFT" | "HOME" | "END" |
                                            "PAGEUP" | "PPAGE" | "PAGEDOWN" | "NPAGE" | "DELETE" | "DC" | "INSERT" | "IC" |
                                            "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
                                        ) || parts[i + 1].to_uppercase().starts_with("C-");
                                        if !next_is_special {
                                            send_text_to_active(&mut app, " ")?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                CtrlReq::SelectPane(dir) => {
                    match dir.as_str() {
                        "U" => { move_focus(&mut app, FocusDir::Up); }
                        "D" => { move_focus(&mut app, FocusDir::Down); }
                        "L" => { move_focus(&mut app, FocusDir::Left); }
                        "R" => { move_focus(&mut app, FocusDir::Right); }
                        _ => {}
                    }
                }
                CtrlReq::SelectWindow(idx) => {
                    if idx < app.windows.len() { app.active_idx = idx; }
                }
                CtrlReq::ListPanes(resp) => {
                    let mut output = String::new();
                    let win = &app.windows[app.active_idx];
                    fn collect_panes(node: &Node, panes: &mut Vec<(usize, u16, u16)>) {
                        match node {
                            Node::Leaf(p) => panes.push((p.id, p.last_cols, p.last_rows)),
                            Node::Split { children, .. } => {
                                for c in children { collect_panes(c, panes); }
                            }
                        }
                    }
                    let mut panes = Vec::new();
                    collect_panes(&win.root, &mut panes);
                    for (i, (id, cols, rows)) in panes.iter().enumerate() {
                        output.push_str(&format!("%{}: [{}x{}]\n", id, cols, rows));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::KillWindow => {
                    if app.windows.len() > 1 {
                        app.windows.remove(app.active_idx);
                        if app.active_idx >= app.windows.len() { app.active_idx = app.windows.len() - 1; }
                    }
                }
                CtrlReq::KillSession => {
                    // Remove port file and exit
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}\\.psmux\\{}.port", home, app.session_name);
                    let _ = std::fs::remove_file(&regpath);
                    std::process::exit(0);
                }
                CtrlReq::HasSession(resp) => {
                    let _ = resp.send(true); // We are running, so session exists
                }
                CtrlReq::RenameSession(name) => {
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let old_path = format!("{}\\.psmux\\{}.port", home, app.session_name);
                    let new_path = format!("{}\\.psmux\\{}.port", home, name);
                    if let Some(port) = app.control_port {
                        let _ = std::fs::remove_file(&old_path);
                        let _ = std::fs::write(&new_path, port.to_string());
                    }
                    app.session_name = name;
                }
                CtrlReq::SwapPane(dir) => {
                    // Swap with adjacent pane in direction
                    match dir.as_str() {
                        "U" => { swap_pane(&mut app, FocusDir::Up); }
                        "D" => { swap_pane(&mut app, FocusDir::Down); }
                        _ => { swap_pane(&mut app, FocusDir::Down); }
                    }
                }
                CtrlReq::ResizePane(dir, amount) => {
                    // Resize pane in direction
                    match dir.as_str() {
                        "U" | "D" => { resize_pane_vertical(&mut app, if dir == "U" { -(amount as i16) } else { amount as i16 }); }
                        "L" | "R" => { resize_pane_horizontal(&mut app, if dir == "L" { -(amount as i16) } else { amount as i16 }); }
                        _ => {}
                    }
                }
                CtrlReq::SetBuffer(content) => {
                    app.paste_buffers.insert(0, content);
                    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
                }
                CtrlReq::ListBuffers(resp) => {
                    let mut output = String::new();
                    for (i, buf) in app.paste_buffers.iter().enumerate() {
                        let preview: String = buf.chars().take(50).collect();
                        output.push_str(&format!("buffer{}: {} bytes: \"{}\"\n", i, buf.len(), preview));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::ShowBuffer(resp) => {
                    let content = app.paste_buffers.first().cloned().unwrap_or_default();
                    let _ = resp.send(content);
                }
                CtrlReq::DeleteBuffer => {
                    if !app.paste_buffers.is_empty() { app.paste_buffers.remove(0); }
                }
                CtrlReq::DisplayMessage(resp, fmt) => {
                    // Parse format string with variables
                    let mut result = fmt.clone();
                    result = result.replace("#S", &app.session_name);
                    result = result.replace("#I", &app.active_idx.to_string());
                    result = result.replace("#W", &app.windows[app.active_idx].name);
                    let win = &app.windows[app.active_idx];
                    let pane_id = get_active_pane_id(&win.root, &win.active_path).unwrap_or(0);
                    result = result.replace("#P", &pane_id.to_string());
                    result = result.replace("#H", &env::var("COMPUTERNAME").or_else(|_| env::var("HOSTNAME")).unwrap_or_default());
                    result = result.replace("#T", &win.name);
                    let _ = resp.send(result);
                }
                CtrlReq::LastWindow => {
                    // Toggle to previous window using tracking
                    if app.windows.len() > 1 && app.last_window_idx < app.windows.len() {
                        let tmp = app.active_idx;
                        app.active_idx = app.last_window_idx;
                        app.last_window_idx = tmp;
                    }
                }
                CtrlReq::LastPane => {
                    // Use tracked last pane path
                    let win = &mut app.windows[app.active_idx];
                    if !app.last_pane_path.is_empty() && path_exists(&win.root, &app.last_pane_path) {
                        let tmp = win.active_path.clone();
                        win.active_path = app.last_pane_path.clone();
                        app.last_pane_path = tmp;
                    } else if !win.active_path.is_empty() {
                        // Fallback: cycle through panes
                        let last = win.active_path.last_mut();
                        if let Some(idx) = last {
                            *idx = (*idx + 1) % 2;
                        }
                    }
                }
                CtrlReq::RotateWindow(reverse) => {
                    // Rotate panes in window
                    rotate_panes(&mut app, reverse);
                }
                CtrlReq::DisplayPanes => {
                    // This would show pane numbers overlay - no-op for server mode
                }
                CtrlReq::BreakPane => {
                    // Break current pane to a new window
                    break_pane_to_window(&mut app);
                }
                CtrlReq::JoinPane(target_win) => {
                    // Join current pane to target window - complex operation
                    // For now, just move focus
                    if target_win < app.windows.len() {
                        app.active_idx = target_win;
                    }
                }
                CtrlReq::RespawnPane => {
                    // Restart the shell in current pane
                    respawn_active_pane(&mut app)?;
                }
                // Config/Key binding command handlers
                CtrlReq::BindKey(key, command) => {
                    // Parse the key and add binding
                    if let Some(kc) = parse_key_string(&key) {
                        let action = parse_command_to_action(&command);
                        if let Some(act) = action {
                            // Remove existing binding for this key if any
                            app.binds.retain(|b| b.key != kc);
                            app.binds.push(Bind { key: kc, action: act });
                        }
                    }
                }
                CtrlReq::UnbindKey(key) => {
                    if let Some(kc) = parse_key_string(&key) {
                        app.binds.retain(|b| b.key != kc);
                    }
                }
                CtrlReq::ListKeys(resp) => {
                    let mut output = String::new();
                    for bind in &app.binds {
                        let key_str = format_key_binding(&bind.key);
                        let action_str = format_action(&bind.action);
                        output.push_str(&format!("bind-key {} {}\n", key_str, action_str));
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::SetOption(option, value) => {
                    match option.as_str() {
                        "status-left" => { app.status_left = value; }
                        "status-right" => { app.status_right = value; }
                        "mouse" => { app.mouse_enabled = value == "on" || value == "true" || value == "1"; }
                        "prefix" => {
                            if let Some(kc) = parse_key_string(&value) {
                                app.prefix_key = kc;
                            }
                        }
                        "escape-time" => {
                            if let Ok(ms) = value.parse::<u64>() {
                                app.escape_time_ms = ms;
                            }
                        }
                        _ => {}
                    }
                }
                CtrlReq::ShowOptions(resp) => {
                    let mut output = String::new();
                    output.push_str(&format!("status-left \"{}\"\n", app.status_left));
                    output.push_str(&format!("status-right \"{}\"\n", app.status_right));
                    output.push_str(&format!("mouse {}\n", if app.mouse_enabled { "on" } else { "off" }));
                    output.push_str(&format!("prefix {}\n", format_key_binding(&app.prefix_key)));
                    output.push_str(&format!("escape-time {}\n", app.escape_time_ms));
                    let _ = resp.send(output);
                }
                CtrlReq::SourceFile(path) => {
                    // Parse and execute commands from file
                    if let Ok(contents) = std::fs::read_to_string(&path) {
                        for line in contents.lines() {
                            let line = line.trim();
                            if line.is_empty() || line.starts_with('#') { continue; }
                            // Parse and execute each line as a command
                            // This is a simplified implementation - full tmux would spawn subprocesses
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.is_empty() { continue; }
                            match parts[0] {
                                "set" | "set-option" => {
                                    if parts.len() >= 3 {
                                        let opt_idx = if parts.get(1) == Some(&"-g") { 2 } else { 1 };
                                        if let (Some(opt), Some(val)) = (parts.get(opt_idx), parts.get(opt_idx + 1)) {
                                            match *opt {
                                                "status-left" => { app.status_left = parts[opt_idx + 1..].join(" ").trim_matches('"').to_string(); }
                                                "status-right" => { app.status_right = parts[opt_idx + 1..].join(" ").trim_matches('"').to_string(); }
                                                "mouse" => { app.mouse_enabled = *val == "on" || *val == "true" || *val == "1"; }
                                                "prefix" => { if let Some(kc) = parse_key_string(val) { app.prefix_key = kc; } }
                                                "escape-time" => { if let Ok(ms) = val.parse::<u64>() { app.escape_time_ms = ms; } }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                "bind" | "bind-key" => {
                                    if parts.len() >= 3 {
                                        let key_idx = if parts.get(1).map(|s| s.starts_with('-')).unwrap_or(false) { 2 } else { 1 };
                                        if let Some(key_str) = parts.get(key_idx) {
                                            if let Some(kc) = parse_key_string(key_str) {
                                                let cmd = parts[key_idx + 1..].join(" ");
                                                if let Some(act) = parse_command_to_action(&cmd) {
                                                    app.binds.retain(|b| b.key != kc);
                                                    app.binds.push(Bind { key: kc, action: act });
                                                }
                                            }
                                        }
                                    }
                                }
                                "unbind" | "unbind-key" => {
                                    if parts.len() >= 2 {
                                        let key_idx = if parts.get(1).map(|s| s.starts_with('-')).unwrap_or(false) { 2 } else { 1 };
                                        if let Some(key_str) = parts.get(key_idx) {
                                            if let Some(kc) = parse_key_string(key_str) {
                                                app.binds.retain(|b| b.key != kc);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // Additional command handlers for full tmux parity
                CtrlReq::MoveWindow(target) => {
                    if let Some(t) = target {
                        if t < app.windows.len() && app.active_idx != t {
                            let win = app.windows.remove(app.active_idx);
                            let insert_idx = if t > app.active_idx { t - 1 } else { t };
                            app.windows.insert(insert_idx.min(app.windows.len()), win);
                            app.active_idx = insert_idx.min(app.windows.len() - 1);
                        }
                    }
                }
                CtrlReq::SwapWindow(target) => {
                    if target < app.windows.len() && app.active_idx != target {
                        app.windows.swap(app.active_idx, target);
                    }
                }
                CtrlReq::LinkWindow(_target) => {
                    // Link window to another session - not fully supported in single-process model
                    // Would require IPC between server instances
                }
                CtrlReq::UnlinkWindow => {
                    // Unlink window - in single session model, just remove the window
                    if app.windows.len() > 1 {
                        app.windows.remove(app.active_idx);
                        if app.active_idx >= app.windows.len() {
                            app.active_idx = app.windows.len() - 1;
                        }
                    }
                }
                CtrlReq::FindWindow(resp, pattern) => {
                    let mut output = String::new();
                    for (i, win) in app.windows.iter().enumerate() {
                        if win.name.contains(&pattern) {
                            output.push_str(&format!("{}: {} []\n", i, win.name));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::MovePane(target_win) => {
                    // Move current pane to target window
                    if target_win < app.windows.len() && target_win != app.active_idx {
                        // This is complex - for now just switch to that window
                        app.active_idx = target_win;
                    }
                }
                CtrlReq::PipePane(cmd, stdin, stdout) => {
                    // Pipe pane output to/from a command
                    let win = &app.windows[app.active_idx];
                    let pane_id = get_active_pane_id(&win.root, &win.active_path).unwrap_or(0);
                    
                    if cmd.is_empty() {
                        // Empty command means stop piping for this pane
                        app.pipe_panes.retain(|p| p.pane_id != pane_id);
                    } else {
                        // Check if we're already piping this pane - toggle off if so
                        if let Some(idx) = app.pipe_panes.iter().position(|p| p.pane_id == pane_id) {
                            // Stop existing pipe
                            if let Some(ref mut proc) = app.pipe_panes[idx].process {
                                let _ = proc.kill();
                            }
                            app.pipe_panes.remove(idx);
                        } else {
                            // Start new pipe process
                            #[cfg(windows)]
                            let process = std::process::Command::new("cmd")
                                .args(["/C", &cmd])
                                .stdin(if stdout { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                                .stdout(if stdin { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                                .stderr(std::process::Stdio::null())
                                .spawn()
                                .ok();
                            
                            app.pipe_panes.push(PipePaneState {
                                pane_id,
                                process,
                                stdin,
                                stdout,
                            });
                        }
                    }
                }
                CtrlReq::SelectLayout(layout) => {
                    apply_layout(&mut app, &layout);
                }
                CtrlReq::NextLayout => {
                    cycle_layout(&mut app);
                }
                CtrlReq::ListClients(resp) => {
                    let mut output = String::new();
                    output.push_str(&format!("/dev/pts/0: {}: {} [{}x{}] (utf8)\n", 
                        app.session_name, 
                        app.windows[app.active_idx].name,
                        app.last_window_area.width,
                        app.last_window_area.height
                    ));
                    let _ = resp.send(output);
                }
                CtrlReq::SwitchClient(_target) => {
                    // Switch to another session - would require IPC
                }
                CtrlReq::LockClient => {
                    // Lock client - placeholder
                }
                CtrlReq::RefreshClient => {
                    // Refresh is automatic in our model
                }
                CtrlReq::SuspendClient => {
                    // Suspend - not applicable on Windows
                }
                CtrlReq::CopyModePageUp => {
                    enter_copy_mode(&mut app);
                    // Page up in copy mode
                    move_copy_cursor(&mut app, 0, -20);
                }
                CtrlReq::ClearHistory => {
                    let win = &mut app.windows[app.active_idx];
                    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
                        if let Ok(mut parser) = p.term.lock() {
                            // Create a new parser to clear history
                            *parser = vt100::Parser::new(p.last_rows, p.last_cols, 1000);
                        }
                    }
                }
                CtrlReq::SaveBuffer(path) => {
                    if let Some(content) = app.paste_buffers.first() {
                        let _ = std::fs::write(&path, content);
                    }
                }
                CtrlReq::LoadBuffer(path) => {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        app.paste_buffers.insert(0, content);
                        if app.paste_buffers.len() > 10 {
                            app.paste_buffers.pop();
                        }
                    }
                }
                CtrlReq::SetEnvironment(key, value) => {
                    env::set_var(&key, &value);
                }
                CtrlReq::ShowEnvironment(resp) => {
                    let mut output = String::new();
                    for (key, value) in env::vars() {
                        if key.starts_with("PSMUX") || key.starts_with("TMUX") {
                            output.push_str(&format!("{}={}\n", key, value));
                        }
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::SetHook(hook, cmd) => {
                    // Add hook command to the hooks map
                    app.hooks.entry(hook).or_insert_with(Vec::new).push(cmd);
                }
                CtrlReq::ShowHooks(resp) => {
                    let mut output = String::new();
                    for (name, commands) in &app.hooks {
                        for cmd in commands {
                            output.push_str(&format!("{} -> {}\n", name, cmd));
                        }
                    }
                    if output.is_empty() {
                        output.push_str("(no hooks)\n");
                    }
                    let _ = resp.send(output);
                }
                CtrlReq::RemoveHook(hook) => {
                    app.hooks.remove(&hook);
                }
                CtrlReq::KillServer => {
                    // Kill this server
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let regpath = format!("{}\\.psmux\\{}.port", home, app.session_name);
                    let _ = std::fs::remove_file(&regpath);
                    std::process::exit(0);
                }
                CtrlReq::WaitFor(channel, op) => {
                    match op {
                        WaitForOp::Lock => {
                            // Lock the channel - create if doesn't exist
                            let entry = app.wait_channels.entry(channel).or_insert_with(|| WaitChannel {
                                locked: false,
                                waiters: Vec::new(),
                            });
                            entry.locked = true;
                        }
                        WaitForOp::Unlock => {
                            // Unlock the channel and signal all waiters
                            if let Some(ch) = app.wait_channels.get_mut(&channel) {
                                ch.locked = false;
                                // Signal all waiters
                                for waiter in ch.waiters.drain(..) {
                                    let _ = waiter.send(());
                                }
                            }
                        }
                        WaitForOp::Signal => {
                            // Signal all waiters without unlocking
                            if let Some(ch) = app.wait_channels.get_mut(&channel) {
                                for waiter in ch.waiters.drain(..) {
                                    let _ = waiter.send(());
                                }
                            }
                        }
                        WaitForOp::Wait => {
                            // Wait operation would block - we handle this differently
                            // For now, just create the channel if it doesn't exist
                            app.wait_channels.entry(channel).or_insert_with(|| WaitChannel {
                                locked: false,
                                waiters: Vec::new(),
                            });
                        }
                    }
                }
                CtrlReq::DisplayMenu(menu_def, x, y) => {
                    // Parse menu definition and enter menu mode
                    let menu = parse_menu_definition(&menu_def, x, y);
                    if !menu.items.is_empty() {
                        app.mode = Mode::MenuMode { menu };
                    }
                }
                CtrlReq::DisplayPopup(command, width, height, close_on_exit) => {
                    // Enter popup mode with the command
                    if !command.is_empty() {
                        // Start the command process
                        #[cfg(windows)]
                        let process = std::process::Command::new("cmd")
                            .args(["/C", &command])
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped())
                            .spawn()
                            .ok();
                        
                        app.mode = Mode::PopupMode {
                            command: command.clone(),
                            output: String::new(),
                            process,
                            width,
                            height,
                            close_on_exit,
                        };
                    } else {
                        // No command - just open a shell popup
                        app.mode = Mode::PopupMode {
                            command: String::new(),
                            output: "Press 'q' or Escape to close\n".to_string(),
                            process: None,
                            width,
                            height,
                            close_on_exit: true,
                        };
                    }
                }
                CtrlReq::ConfirmBefore(prompt, cmd) => {
                    // Enter confirm mode
                    let prompt_text = if prompt.is_empty() {
                        format!("Confirm: {} (y/n)?", cmd)
                    } else {
                        format!("{} (y/n)?", prompt)
                    };
                    app.mode = Mode::ConfirmMode {
                        prompt: prompt_text,
                        command: cmd,
                        input: String::new(),
                    };
                }
            }
        }
        // Check if all windows/panes have exited
        if reap_children(&mut app)? {
            // All windows are gone - clean up port file and exit server
            let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
            let regpath = format!("{}\\.psmux\\{}.port", home, app.session_name);
            let _ = std::fs::remove_file(&regpath);
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

fn capture_active_pane_range(app: &mut AppState, s: Option<u16>, e: Option<u16>) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let screen = parser.screen();
    let start = s.unwrap_or(0).min(p.last_rows.saturating_sub(1));
    let end = e.unwrap_or(p.last_rows.saturating_sub(1)).min(p.last_rows.saturating_sub(1));
    let mut text = String::new();
    for r in start..=end { for c in 0..p.last_cols { if let Some(cell) = screen.cell(r, c) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); } } text.push('\n'); }
    Ok(Some(text))
}
#[derive(Serialize, Deserialize)]
struct CellJson { text: String, fg: String, bg: String, bold: bool, italic: bool, underline: bool, inverse: bool }

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum LayoutJson {
    #[serde(rename = "split")]
    Split { kind: String, sizes: Vec<u16>, children: Vec<LayoutJson> },
    #[serde(rename = "leaf")]
    Leaf { id: usize, rows: u16, cols: u16, cursor_row: u16, cursor_col: u16, content: Vec<Vec<CellJson>> },
}

fn dump_layout_json(app: &mut AppState) -> io::Result<String> {
    fn build(node: &mut Node) -> LayoutJson {
        match node {
            Node::Split { kind, sizes, children } => {
                let k = match *kind { LayoutKind::Horizontal => "Horizontal".to_string(), LayoutKind::Vertical => "Vertical".to_string() };
                let mut ch: Vec<LayoutJson> = Vec::new();
                for c in children.iter_mut() { ch.push(build(c)); }
                LayoutJson::Split { kind: k, sizes: sizes.clone(), children: ch }
            }
            Node::Leaf(p) => {
                let parser = p.term.lock().unwrap();
                let screen = parser.screen();
                let (cr, cc) = screen.cursor_position();
                if let Some(t) = infer_title_from_prompt(&screen, p.last_rows, p.last_cols) { p.title = t; }
                let mut lines: Vec<Vec<CellJson>> = Vec::new();
                for r in 0..p.last_rows {
                    let mut row: Vec<CellJson> = Vec::new();
                    for c in 0..p.last_cols {
                        if let Some(cell) = screen.cell(r, c) {
                            let fg = color_to_name(cell.fgcolor());
                            let bg = color_to_name(cell.bgcolor());
                            let text = cell.contents().to_string();
                            row.push(CellJson { text, fg, bg, bold: cell.bold(), italic: cell.italic(), underline: cell.underline(), inverse: cell.inverse() });
                        } else {
                            row.push(CellJson { text: " ".to_string(), fg: "default".to_string(), bg: "default".to_string(), bold: false, italic: false, underline: false, inverse: false });
                        }
                    }
                    lines.push(row);
                }
                LayoutJson::Leaf { id: p.id, rows: p.last_rows, cols: p.last_cols, cursor_row: cr, cursor_col: cc, content: lines }
            }
        }
    }
    let win = &mut app.windows[app.active_idx];
    let root = build(&mut win.root);
    let s = serde_json::to_string(&root).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("json error: {e}")))?;
    Ok(s)
}

fn infer_title_from_prompt(screen: &vt100::Screen, rows: u16, cols: u16) -> Option<String> {
    let mut last: Option<String> = None;
    for r in (0..rows).rev() {
        let mut s = String::new();
        for c in 0..cols { if let Some(cell) = screen.cell(r, c) { s.push_str(&cell.contents().to_string()); } else { s.push(' '); } }
        let t = s.trim_end().to_string();
        if !t.trim().is_empty() { last = Some(t); break; }
    }
    let Some(line) = last else { return None };
    let trimmed = line.trim().to_string();
    if let Some(pos) = trimmed.rfind('>') {
        let before = trimmed[..pos].trim().to_string();
        if before.contains("\\") || before.contains("/") {
            let parts: Vec<&str> = before.trim_matches(|ch: char| ch == '"').split(['\\','/']).collect();
            if let Some(base) = parts.last() { return Some(base.to_string()); }
        }
        return Some(before);
    }
    if let Some(pos) = trimmed.rfind('$') { return Some(trimmed[..pos].trim().to_string()); }
    if let Some(pos) = trimmed.rfind('#') { return Some(trimmed[..pos].trim().to_string()); }
    Some(trimmed)
}

fn resolve_last_session_name() -> Option<String> {
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).ok()?;
    let dir = format!("{}\\.psmux", home);
    let last = std::fs::read_to_string(format!("{}\\last_session", dir)).ok();
    if let Some(name) = last {
        let name = name.trim().to_string();
        let p = format!("{}\\{}.port", dir, name);
        if std::path::Path::new(&p).exists() { return Some(name); }
    }
    let mut picks: Vec<(String, std::time::SystemTime)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Some(fname) = e.file_name().to_str() {
                if let Some((base, ext)) = fname.rsplit_once('.') {
                    if ext == "port" { if let Ok(md) = e.metadata() { picks.push((base.to_string(), md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH))); } }
                }
            }
        }
    }
    picks.sort_by_key(|(_, t)| *t);
    picks.last().map(|(n, _)| n.clone())
}

fn resolve_default_session_name() -> Option<String> {
    if let Ok(name) = env::var("PSMUX_DEFAULT_SESSION") {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).ok()?;
        let p = format!("{}\\.psmux\\{}.port", home, name);
        if std::path::Path::new(&p).exists() { return Some(name); }
    }
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).ok()?;
    let candidates = [format!("{}\\.psmuxrc", home), format!("{}\\.psmux\\pmuxrc", home)];
    for cfg in candidates.iter() {
        if let Ok(text) = std::fs::read_to_string(cfg) {
            let line = text.lines().find(|l| !l.trim().is_empty())?;
            let name = if let Some(rest) = line.strip_prefix("default-session ") { rest.trim().to_string() } else { line.trim().to_string() };
            let p = format!("{}\\.psmux\\{}.port", home, name);
            if std::path::Path::new(&p).exists() { return Some(name); }
        }
    }
    None
}

#[derive(Serialize, Deserialize)]
struct WinInfo { id: usize, name: String, active: bool }

#[derive(Serialize, Deserialize)]
struct PaneInfo { id: usize, title: String }

#[derive(Serialize, Deserialize)]
struct WinTree { id: usize, name: String, active: bool, panes: Vec<PaneInfo> }

fn list_windows_json(app: &AppState) -> io::Result<String> {
    let mut v: Vec<WinInfo> = Vec::new();
    for (i, w) in app.windows.iter().enumerate() { v.push(WinInfo { id: w.id, name: w.name.clone(), active: i == app.active_idx }); }
    let s = serde_json::to_string(&v).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("json error: {e}")))?;
    Ok(s)
}

fn list_tree_json(app: &AppState) -> io::Result<String> {
    fn collect_panes(node: &Node, out: &mut Vec<PaneInfo>) {
        match node {
            Node::Leaf(p) => { out.push(PaneInfo { id: p.id, title: p.title.clone() }); }
            Node::Split { children, .. } => { for c in children.iter() { collect_panes(c, out); } }
        }
    }
    let mut v: Vec<WinTree> = Vec::new();
    for (i, w) in app.windows.iter().enumerate() {
        let mut panes = Vec::new();
        collect_panes(&w.root, &mut panes);
        v.push(WinTree { id: w.id, name: w.name.clone(), active: i == app.active_idx, panes });
    }
    let s = serde_json::to_string(&v).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("json error: {e}")))?;
    Ok(s)
}

fn color_to_name(c: vt100::Color) -> String {
    match c {
        vt100::Color::Default => "default".to_string(),
        vt100::Color::Idx(i) => format!("idx:{}", i),
        vt100::Color::Rgb(r,g,b) => format!("rgb:{},{},{}", r,g,b),
    }
}

fn send_text_to_active(app: &mut AppState, text: &str) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.master, "{}", text); }
    Ok(())
}

fn send_key_to_active(app: &mut AppState, k: &str) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
        match k {
            "enter" => { let _ = write!(p.master, "\r"); }
            "tab" => { let _ = write!(p.master, "\t"); }
            "backspace" => { let _ = write!(p.master, "\x08"); }
            "esc" => { let _ = write!(p.master, "\x1b"); }
            "left" => { let _ = write!(p.master, "\x1b[D"); }
            "right" => { let _ = write!(p.master, "\x1b[C"); }
            "up" => { let _ = write!(p.master, "\x1b[A"); }
            "down" => { let _ = write!(p.master, "\x1b[B"); }
            _ => {}
        }
    }
    Ok(())
}

/// Apply a named layout to the current window
fn apply_layout(app: &mut AppState, layout: &str) {
    let win = &mut app.windows[app.active_idx];
    
    // Count panes
    fn count_panes(node: &Node) -> usize {
        match node {
            Node::Leaf(_) => 1,
            Node::Split { children, .. } => children.iter().map(count_panes).sum(),
        }
    }
    let pane_count = count_panes(&win.root);
    if pane_count < 2 { return; }
    
    // Get all pane IDs to preserve them
    fn collect_panes(node: &mut Node, panes: &mut Vec<Pane>) {
        match node {
            Node::Leaf(_) => {
                // We need to take the pane - this is tricky with ownership
            }
            Node::Split { children, .. } => {
                for c in children { collect_panes(c, panes); }
            }
        }
    }
    
    match layout.to_lowercase().as_str() {
        "even-horizontal" | "even-h" => {
            // All panes in a horizontal row
            if let Node::Split { kind, sizes, .. } = &mut win.root {
                *kind = LayoutKind::Horizontal;
                let size = 100 / sizes.len().max(1) as u16;
                for s in sizes.iter_mut() { *s = size; }
            }
        }
        "even-vertical" | "even-v" => {
            // All panes in a vertical column
            if let Node::Split { kind, sizes, .. } = &mut win.root {
                *kind = LayoutKind::Vertical;
                let size = 100 / sizes.len().max(1) as u16;
                for s in sizes.iter_mut() { *s = size; }
            }
        }
        "main-horizontal" | "main-h" => {
            // Main pane on top, others below
            if let Node::Split { kind, sizes, .. } = &mut win.root {
                *kind = LayoutKind::Vertical;
                if sizes.len() >= 2 {
                    sizes[0] = 60;
                    let remaining = 40 / (sizes.len() - 1).max(1) as u16;
                    for s in sizes.iter_mut().skip(1) { *s = remaining; }
                }
            }
        }
        "main-vertical" | "main-v" => {
            // Main pane on left, others to the right
            if let Node::Split { kind, sizes, .. } = &mut win.root {
                *kind = LayoutKind::Horizontal;
                if sizes.len() >= 2 {
                    sizes[0] = 60;
                    let remaining = 40 / (sizes.len() - 1).max(1) as u16;
                    for s in sizes.iter_mut().skip(1) { *s = remaining; }
                }
            }
        }
        "tiled" => {
            // Try to make a grid
            if let Node::Split { sizes, .. } = &mut win.root {
                let size = 100 / sizes.len().max(1) as u16;
                for s in sizes.iter_mut() { *s = size; }
            }
        }
        _ => {}
    }
}

/// Cycle through available layouts
fn cycle_layout(app: &mut AppState) {
    static LAYOUTS: [&str; 5] = ["even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled"];
    
    // Determine current layout type based on split configuration
    let win = &app.windows[app.active_idx];
    let (kind, sizes) = match &win.root {
        Node::Leaf(_) => return, // Can't cycle layouts with single pane
        Node::Split { kind, sizes, .. } => (*kind, sizes.clone()),
    };
    
    // Try to determine which layout we're currently in
    let current_idx = if sizes.is_empty() {
        0
    } else if sizes.iter().all(|s| *s == sizes[0]) {
        // All equal sizes - even layout
        match kind {
            LayoutKind::Horizontal => 0, // even-horizontal
            LayoutKind::Vertical => 1,   // even-vertical
        }
    } else if sizes.len() >= 2 && sizes[0] > sizes[1] {
        // First pane is larger - main layout
        match kind {
            LayoutKind::Vertical => 2,   // main-horizontal
            LayoutKind::Horizontal => 3, // main-vertical
        }
    } else {
        4 // tiled
    };
    
    // Cycle to next layout
    let next_idx = (current_idx + 1) % LAYOUTS.len();
    apply_layout(app, LAYOUTS[next_idx]);
}

fn toggle_zoom(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if app.zoom_saved.is_none() {
        let mut saved: Vec<(Vec<usize>, Vec<u16>)> = Vec::new();
        for depth in 0..win.active_path.len() {
            let p = win.active_path[..depth].to_vec();
            if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) {
                let idx = win.active_path.get(depth).copied().unwrap_or(0);
                saved.push((p.clone(), sizes.clone()));
                for i in 0..sizes.len() { sizes[i] = if i == idx { 100 } else { 0 }; }
            }
        }
        app.zoom_saved = Some(saved);
    } else {
        if let Some(saved) = app.zoom_saved.take() {
            for (p, sz) in saved.into_iter() {
                if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) { *sizes = sz; }
            }
        }
    }
}

fn remote_mouse_down(app: &mut AppState, x: u16, y: u16) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    for (path, area) in rects.iter() { if area.contains(ratatui::layout::Position { x, y }) { win.active_path = path.clone(); } }
    let mut borders: Vec<(Vec<usize>, LayoutKind, usize, u16)> = Vec::new();
    compute_split_borders(&win.root, app.last_window_area, &mut borders);
    let tol = 1u16;
    for (path, kind, idx, pos) in borders.iter() {
        match kind {
            LayoutKind::Horizontal => {
                if x >= pos.saturating_sub(tol) && x <= pos + tol { if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) { app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: x, start_y: y, left_initial: left, _right_initial: right }); } break; }
            }
            LayoutKind::Vertical => {
                if y >= pos.saturating_sub(tol) && y <= pos + tol { if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) { app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: x, start_y: y, left_initial: left, _right_initial: right }); } break; }
            }
        }
    }
}

fn remote_mouse_drag(app: &mut AppState, x: u16, y: u16) { let win = &mut app.windows[app.active_idx]; if let Some(d) = &app.drag { adjust_split_sizes(&mut win.root, d, x, y); } }

fn remote_scroll_up(app: &mut AppState) { let win = &mut app.windows[app.active_idx]; if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.master, "\x1b[A"); } }
fn remote_scroll_down(app: &mut AppState) { let win = &mut app.windows[app.active_idx]; if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.master, "\x1b[B"); } }

// New helper functions for tmux compatibility

fn swap_pane(app: &mut AppState, dir: FocusDir) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    
    let mut active_idx = None;
    for (i, (path, _)) in rects.iter().enumerate() { 
        if *path == win.active_path { active_idx = Some(i); break; } 
    }
    let Some(ai) = active_idx else { return; };
    let (_, arect) = &rects[ai];
    
    // Find nearest neighbor in direction
    let mut best: Option<(usize, u32)> = None;
    for (i, (_, r)) in rects.iter().enumerate() {
        if i == ai { continue; }
        let candidate = match dir {
            FocusDir::Left => if r.x + r.width <= arect.x { Some((arect.x - (r.x + r.width)) as u32) } else { None },
            FocusDir::Right => if r.x >= arect.x + arect.width { Some((r.x - (arect.x + arect.width)) as u32) } else { None },
            FocusDir::Up => if r.y + r.height <= arect.y { Some((arect.y - (r.y + r.height)) as u32) } else { None },
            FocusDir::Down => if r.y >= arect.y + arect.height { Some((r.y - (arect.y + arect.height)) as u32) } else { None },
        };
        if let Some(dist) = candidate { 
            if best.map_or(true, |(_,bd)| dist < bd) { best = Some((i, dist)); } 
        }
    }
    
    // Swap content with neighbor (just swap focus for now - full swap is complex)
    if let Some((ni, _)) = best { 
        win.active_path = rects[ni].0.clone(); 
    }
}

fn resize_pane_vertical(app: &mut AppState, amount: i16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    
    // Find the parent split that controls vertical sizing
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == LayoutKind::Vertical {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    let new_size = (sizes[idx] as i16 + amount).max(1) as u16;
                    let diff = new_size as i16 - sizes[idx] as i16;
                    sizes[idx] = new_size;
                    // Adjust sibling
                    if idx + 1 < sizes.len() {
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        sizes[idx - 1] = (sizes[idx - 1] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

fn resize_pane_horizontal(app: &mut AppState, amount: i16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    
    // Find the parent split that controls horizontal sizing
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == LayoutKind::Horizontal {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    let new_size = (sizes[idx] as i16 + amount).max(1) as u16;
                    let diff = new_size as i16 - sizes[idx] as i16;
                    sizes[idx] = new_size;
                    // Adjust sibling
                    if idx + 1 < sizes.len() {
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        sizes[idx - 1] = (sizes[idx - 1] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

fn rotate_panes(app: &mut AppState, _reverse: bool) {
    // Rotate panes within a window - simplified implementation
    let win = &mut app.windows[app.active_idx];
    match &mut win.root {
        Node::Split { children, .. } if children.len() >= 2 => {
            // Simple rotation: swap first and last
            let last_idx = children.len() - 1;
            children.swap(0, last_idx);
        }
        _ => {}
    }
}

fn break_pane_to_window(app: &mut AppState) {
    // This would require extracting the current pane from its split and creating a new window
    // For now, just create a new window
    let pty_system = match PtySystemSelection::default().get() {
        Ok(p) => p,
        Err(_) => return,
    };
    let _ = create_window(&*pty_system, app);
}

fn respawn_active_pane(app: &mut AppState) -> io::Result<()> {
    // Respawn the shell in the active pane
    let pty_system = PtySystemSelection::default().get().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("pty system error: {e}")))?;
    let win = &mut app.windows[app.active_idx];
    let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) else { return Ok(()); };
    
    let size = PtySize { rows: pane.last_rows, cols: pane.last_cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("openpty error: {e}")))?;
    let shell_cmd = detect_shell();
    let child = pair.slave.spawn_command(shell_cmd).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("spawn shell error: {e}")))?;
    let term: Arc<Mutex<vt100::Parser>> = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, 0)));
    let term_reader = term.clone();
    let mut reader = pair.master.try_clone_reader().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("clone reader error: {e}")))?;
    
    thread::spawn(move || {
        let mut local = [0u8; 8192];
        loop {
            match reader.read(&mut local) {
                Ok(n) if n > 0 => { let mut parser = term_reader.lock().unwrap(); parser.process(&local[..n]); }
                Ok(_) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
    });
    
    pane.master = pair.master;
    pane.child = child;
    pane.term = term;
    
    Ok(())
}
