// Multi-binary crate (psmux, pmux, tmux) sharing all modules —
// suppress dead_code warnings for functions only used by a subset of binaries.
#![allow(dead_code)]

// ─── Category modules ───────────────────────────────────────────────────────
// Each category folder groups related domain modules.
// `pub use` re-exports every child module to the crate root so that existing
// `crate::module::item` paths keep working unchanged.

mod base;       // types, tree, pane, layout, session, util
mod net;        // server, client, control, cross_session, cross_session_server, proxy_pane
mod tui;        // input, copy_mode, rendering, style, window_ops, popup, help
mod engine;     // commands, cmd_dispatch, config, format
mod os;         // platform, ssh_input
mod shell;      // cli, debug_log

pub use base::*;
pub use net::*;
pub use tui::*;
pub use engine::*;
pub use os::*;
pub use shell::*;

use std::io::{self, IsTerminal};
use std::env;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use crossterm::terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute};
use crossterm::cursor::{EnableBlinking, DisableBlinking};
use crossterm::event::{EnableMouseCapture, DisableMouseCapture, EnableBracketedPaste, DisableBracketedPaste};

use crate::platform::enable_virtual_terminal_processing;
use crate::cli::{print_help, print_version, print_commands};
use crate::session::{cleanup_stale_port_files, send_control,
    send_control_with_response};
use crate::rendering::apply_cursor_style;
use crate::client::run_remote;
use crate::ssh_input::{send_mouse_enable, InputSource};

fn main() {
    if let Err(e) = run_main() {
        // Print a user-friendly error message instead of Rust's Debug format
        // which shows "Error: Custom { kind: Other, error: \"...\" }"  (fixes #47)
        let msg = e.to_string();
        eprintln!("psmux: {}", msg);
        std::process::exit(1);
    }
}

fn run_main() -> io::Result<()> {
    let args: Vec<String> = crate::cli::normalize_flag_equals(env::args().collect());

    // Clean up any stale port files at startup
    cleanup_stale_port_files();

    let parsed = cmd_dispatch::args::parse_and_resolve_args(&args)?;
    let l_socket_name = parsed.l_socket_name;
    let f_config_file = parsed.f_config_file;
    let control_mode = parsed.control_mode;
    let cmd_args = parsed.cmd_args;
    let cmd = cmd_args.first().map(|s| s.as_str()).unwrap_or("");

    // Handle help and version flags first
    match cmd {
        "-h" | "--help" | "help" => {
            print_help();
            return Ok(());
        }
        "-V" | "-v" | "--version" | "version" => {
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
        "kill-server" => { return cmd_dispatch::session::handle_kill_server(&l_socket_name); }
        "ls" | "list-sessions" => { return cmd_dispatch::session::handle_list_sessions(&cmd_args, &l_socket_name); }
        "a" | "at" | "attach" | "attach-session" => {
            let name = cmd_dispatch::new_session::resolve_attach_name(&args, &l_socket_name);
            env::set_var("PSMUX_SESSION_NAME", &name);
            env::set_var("PSMUX_REMOTE_ATTACH", "1");
        }
        "server" => { return cmd_dispatch::session::handle_server_cmd(&args); }
        "new-session" | "new" => {
            if !cmd_dispatch::new_session::handle_new_session(&cmd_args, &l_socket_name, &f_config_file)? {
                return Ok(());
            }
        }
        "new-window" | "neww" => { return cmd_dispatch::window::handle_new_window(&cmd_args); }
        "split-window" | "splitw" => { return cmd_dispatch::window::handle_split_window(&cmd_args); }
        "kill-pane" | "killp" => { send_control("kill-pane\n".to_string())?; return Ok(()); }
        "capture-pane" | "capturep" => { return cmd_dispatch::pane::handle_capture_pane(&cmd_args); }
        "send-keys" | "send" | "send-key" => { return cmd_dispatch::keys_input::handle_send_keys(&cmd_args); }
        "send-paste" => { return cmd_dispatch::keys_input::handle_send_paste(&cmd_args); }
        "select-pane" | "selectp" => { return cmd_dispatch::pane::handle_select_pane(&cmd_args); }
        "select-window" | "selectw" => { return cmd_dispatch::window::handle_select_window(&cmd_args); }
        "list-panes" | "lsp" => { return cmd_dispatch::pane::handle_list_panes(&cmd_args); }
        "list-windows" | "lsw" => { return cmd_dispatch::window::handle_list_windows(&cmd_args); }
        "kill-window" | "killw" => { return cmd_dispatch::window::handle_kill_window(&cmd_args); }
        "kill-session" | "kill-ses" => { return cmd_dispatch::session::handle_kill_session(&cmd_args, &l_socket_name); }
        "has-session" | "has" => { return cmd_dispatch::session::handle_has_session(&cmd_args, &l_socket_name); }
        "rename-session" | "rename" => { return cmd_dispatch::session::handle_rename_session(&cmd_args); }
        "swap-pane" | "swapp" => { return cmd_dispatch::pane::handle_swap_pane(&cmd_args); }
        "resize-pane" | "resizep" => { return cmd_dispatch::pane::handle_resize_pane(&cmd_args); }
        "paste-buffer" | "pasteb" => { return cmd_dispatch::buffer::handle_paste_buffer(&cmd_args); }
        "set-buffer" | "setb" => { return cmd_dispatch::buffer::handle_set_buffer(&cmd_args); }
        "list-buffers" | "lsb" => { return cmd_dispatch::buffer::handle_list_buffers(&cmd_args); }
        "show-buffer" | "showb" => { return cmd_dispatch::buffer::handle_show_buffer(&cmd_args); }
        "delete-buffer" | "deleteb" => { return cmd_dispatch::buffer::handle_delete_buffer(&cmd_args); }
        "display-message" | "display" => { return cmd_dispatch::misc_display::handle_display_message(&cmd_args); }
        "run-shell" | "run" => { return cmd_dispatch::misc_display::handle_run_shell(&cmd_args); }
        "respawn-pane" | "respawnp" | "resp" => { return cmd_dispatch::pane::handle_respawn_pane(&cmd_args); }
        "last-window" | "last" => { send_control("last-window\n".to_string())?; return Ok(()); }
        "last-pane" | "lastp" => { send_control("last-pane\n".to_string())?; return Ok(()); }
        "next-window" | "next" => { send_control("next-window\n".to_string())?; return Ok(()); }
        "previous-window" | "prev" => { send_control("previous-window\n".to_string())?; return Ok(()); }
        "rotate-window" | "rotatew" => { return cmd_dispatch::window::handle_rotate_window(&cmd_args); }
        "display-panes" | "displayp" => { send_control("display-panes\n".to_string())?; return Ok(()); }
        "break-pane" | "breakp" => { return cmd_dispatch::window::handle_break_pane(&cmd_args); }
        "join-pane" | "joinp" | "move-pane" | "movep" => { return cmd_dispatch::pane::handle_join_pane(&cmd_args); }
        "rename-window" | "renamew" => {
            if let Some(name) = cmd_args.get(1) {
                if !name.starts_with('-') {
                    send_control(format!("rename-window {}\n", crate::util::quote_arg(name)))?;
                }
            }
            return Ok(());
        }
        "zoom-pane" | "resizep -Z" => { send_control("zoom-pane\n".to_string())?; return Ok(()); }
        "source-file" | "source" => { return cmd_dispatch::keys_input::handle_source_file(&cmd_args); }
        "list-keys" | "lsk" => { return cmd_dispatch::keys_input::handle_list_keys(&cmd_args); }
        "bind-key" | "bind" => {
            let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
            match send_control(format!("{}\n", cmd_str)) {
                Ok(()) => {},
                Err(e) if e.to_string().contains("no session") => {
                    eprintln!("warning: no active session; bind-key will take effect when set inside a session or via config file");
                },
                Err(e) => return Err(e),
            }
            return Ok(());
        }
        "unbind-key" | "unbind" => {
            let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
            match send_control(format!("{}\n", cmd_str)) {
                Ok(()) => {},
                Err(e) if e.to_string().contains("no session") => {
                    eprintln!("warning: no active session; unbind-key will take effect when set inside a session or via config file");
                },
                Err(e) => return Err(e),
            }
            return Ok(());
        }
        "set-option" | "set" => { return cmd_dispatch::keys_input::handle_set_option(&cmd_args); }
        "show-options" | "show" | "show-window-options" | "showw" => {
            let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
            let resp = send_control_with_response(format!("{}\n", cmd_str))?;
            print!("{}", resp);
            return Ok(());
        }
        "if-shell" | "if" => { return cmd_dispatch::misc_flow::handle_if_shell(&cmd_args); }
        "wait-for" | "wait" => { return cmd_dispatch::misc_flow::handle_wait_for(&cmd_args); }
        "select-layout" | "selectl" => { return cmd_dispatch::misc_display::handle_select_layout(&cmd_args); }
        "move-window" | "movew" => { return cmd_dispatch::window::handle_move_window(&cmd_args); }
        "swap-window" | "swapw" => { return cmd_dispatch::window::handle_swap_window(&cmd_args); }
        "list-clients" | "lsc" => {
            let resp = send_control_with_response("list-clients\n".to_string())?;
            print!("{}", resp);
            return Ok(());
        }
        "switch-client" | "switchc" => { return cmd_dispatch::misc_display::handle_switch_client(&cmd_args); }
        "copy-mode" => { return cmd_dispatch::keys_input::handle_copy_mode(&cmd_args); }
        "clock-mode" => { send_control("clock-mode\n".to_string())?; return Ok(()); }
        "choose-buffer" | "chooseb" => {
            let resp = send_control_with_response("choose-buffer\n".to_string())?;
            print!("{}", resp);
            return Ok(());
        }
        "set-environment" | "setenv" => { return cmd_dispatch::misc_flow::handle_set_environment(&cmd_args); }
        "show-environment" | "showenv" => { return cmd_dispatch::misc_flow::handle_show_environment(&cmd_args); }
        "load-buffer" | "loadb" => { return cmd_dispatch::buffer::handle_load_buffer(&cmd_args); }
        "save-buffer" | "saveb" => { return cmd_dispatch::buffer::handle_save_buffer(&cmd_args); }
        "clear-history" | "clearhist" => { return cmd_dispatch::buffer::handle_clear_history(&cmd_args); }
        "pipe-pane" | "pipep" => { return cmd_dispatch::misc_display::handle_pipe_pane(&cmd_args); }
        "find-window" | "findw" => { return cmd_dispatch::misc_display::handle_find_window(&cmd_args); }
        "set-hook" => {
            let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
            send_control(format!("{}\n", cmd_str))?;
            return Ok(());
        }
        "show-hooks" => {
            let cmd_str: String = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
            let resp = send_control_with_response(format!("{}\n", cmd_str))?;
            print!("{}", resp);
            return Ok(());
        }
        "next-layout" => { send_control("next-layout\n".to_string())?; return Ok(()); }
        "previous-layout" => { send_control("previous-layout\n".to_string())?; return Ok(()); }
        "choose-tree" | "choose-window" | "choose-session" => {
            send_control(format!("{}\n", cmd))?;
            return Ok(());
        }
        "command-prompt" => { return cmd_dispatch::misc_display::handle_command_prompt(&cmd_args); }
        "display-menu" | "menu" => {
            let parts: Vec<String> = cmd_args.iter().map(|s| {
                if s.contains(' ') || s.contains('"') { format!("\"{}\"" , s.replace('"', "\\\"")) } else { s.to_string() }
            }).collect();
            send_control(format!("{}\n", parts.join(" ")))?;
            return Ok(());
        }
        "display-popup" | "popup" => {
            let parts: Vec<String> = cmd_args.iter().map(|s| {
                if s.contains(' ') || s.contains('"') { format!("\"{}\"" , s.replace('"', "\\\"")) } else { s.to_string() }
            }).collect();
            send_control(format!("{}\n", parts.join(" ")))?;
            return Ok(());
        }
        "server-info" | "info" => {
            let resp = send_control_with_response("server-info\n".to_string())?;
            print!("{}", resp);
            return Ok(());
        }
        "start-server" | "start" | "warmup" => { return cmd_dispatch::misc_flow::handle_start_server(&l_socket_name); }
        "confirm-before" | "confirm" => {
            let parts: Vec<String> = cmd_args.iter().map(|s| {
                if s.contains(' ') || s.contains('"') { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() }
            }).collect();
            send_control(format!("{}\n", parts.join(" ")))?;
            return Ok(());
        }
        "refresh-client" | "refresh" => { return cmd_dispatch::misc_display::handle_refresh_client(&cmd_args); }
        "send-prefix" => { send_control("send-prefix\n".to_string())?; return Ok(()); }
        "show-messages" | "showmsgs" => {
            let resp = send_control_with_response("show-messages\n".to_string())?;
            if !resp.trim().is_empty() {
                print!("{}", resp);
            }
            return Ok(());
        }
        "suspend-client" | "suspendc" => {
            // No-op on Windows — no SIGTSTP concept
            return Ok(());
        }
        "lock-client" | "lockc" | "lock-server" | "lock" | "lock-session" | "locks" => {
            // No-op on Windows — no terminal locking concept
            return Ok(());
        }
        "resize-window" | "resizew" => { return cmd_dispatch::window::handle_resize_window(&cmd_args); }
        "customize-mode" => { send_control("customize-mode\n".to_string())?; return Ok(()); }
        "choose-client" => {
            // Single-client model — returns current client info
            let resp = send_control_with_response("list-clients\n".to_string())?;
            print!("{}", resp);
            return Ok(());
        }
        "respawn-window" | "respawnw" => { send_control("respawn-window\n".to_string())?; return Ok(()); }
        "link-window" | "linkw" => {
            let full = cmd_args.iter().map(|s| s.as_str()).collect::<Vec<&str>>().join(" ");
            send_control(format!("{}\n", full))?;
            return Ok(());
        }
        "unlink-window" | "unlinkw" => { send_control("unlink-window\n".to_string())?; return Ok(()); }
        _ => {
            // Unknown command - print error and exit
            if !cmd.is_empty() {
                eprintln!("psmux: unknown command: {}", cmd);
                eprintln!("Run 'psmux --help' for usage information.");
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown command: {}", cmd)));
            }
        }
    }

    // Default behavior (bare `psmux` with no command):
    // tmux-compatible: always create a new session with the next available
    // numeric name (0, 1, 2, ...) and attach to it.

    // Control mode: connect to server with CONTROL/CONTROL_NOECHO protocol
    // instead of launching the TUI client. Must be checked before the
    // is_terminal() gate since control mode reads from piped stdin.
    if control_mode > 0 {
        return cmd_dispatch::control::run_control_mode(control_mode);
    }

    //
    // If stdin is not a terminal (headless/non-interactive environment, e.g.
    // winget validation pipeline), print version and exit cleanly — starting
    // a TUI session would fail without an interactive console.
    if !std::io::stdin().is_terminal() {
        print_version();
        return Ok(());
    }
    if env::var("PSMUX_REMOTE_ATTACH").ok().as_deref() != Some("1") {
        cmd_dispatch::control::handle_default_session(&l_socket_name)?;
    }

    // Prevent nesting: similar to tmux checking $TMUX.
    // PSMUX_ACTIVE is set on the client process itself.
    // PSMUX_SESSION is set on child panes spawned by the server.
    // Both indicate we are already inside psmux.
    // Override with PSMUX_ALLOW_NESTING=1 if nesting is intentional.
    if env::var("PSMUX_ALLOW_NESTING").ok().as_deref() != Some("1") {
        if env::var("PSMUX_ACTIVE").ok().as_deref() == Some("1")
            || env::var("PSMUX_SESSION").ok().filter(|v| !v.is_empty()).is_some()
        {
            eprintln!("psmux: sessions should be nested with care, unset PSMUX_SESSION to force");
            return Ok(());
        }
    }
    env::set_var("PSMUX_ACTIVE", "1");

    let mut stdout = crate::platform::create_writer();
    enable_virtual_terminal_processing();
    enable_raw_mode()?;

    // Detect terminal type for input handling.
    // Use VT input parsing for SSH sessions and terminals that send VT mouse
    // sequences through ConPTY (e.g. JetBrains JediTerm).
    let use_vt_input = crate::ssh_input::needs_vt_input();

    // For standard terminals (not SSH), clear VTI flag from stdin if
    // crossterm or another layer set it. Keeps normal ReadConsoleInputW
    // behavior via proper INPUT_RECORDs.
    if !use_vt_input {
        crate::platform::disable_vti_on_stdin();
    }

    execute!(stdout, EnterAlternateScreen, EnableBlinking, EnableMouseCapture, EnableBracketedPaste)?;
    apply_cursor_style(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let input = InputSource::new(use_vt_input)?;

    // For VT input mode (SSH / JetBrains), explicitly (re-)send mouse-enable
    // escape sequences.  ConPTY may have consumed crossterm's
    // EnableMouseCapture output without forwarding it.
    if use_vt_input {
        send_mouse_enable();
    }

    // Loop to handle session switching without spawning new processes
    let result = loop {
        let result = run_remote(&mut terminal, &input);

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

        break result;
    };

    // Terminal cleanup — always runs, even on error, to prevent leaked
    // SGR attributes (invisible text), stuck raw mode, or stale cursor style.
    let _ = disable_raw_mode();
    let out = terminal.backend_mut();
    // Reset all SGR attributes (fg/bg color, bold, hidden, etc.) BEFORE
    // leaving the alternate screen.  SGR state is global and NOT restored
    // by the alternate-screen save/restore mechanism (\x1b[?1049l).
    // Without this, the last ratatui frame's foreground color can persist
    // into the main screen, making typed text invisible.
    let _ = execute!(out, crossterm::style::Print("\x1b[0m"));
    // Reset cursor style to terminal default (\x1b[0 q)
    let _ = execute!(out, crossterm::style::Print("\x1b[0 q"));
    let _ = execute!(out, DisableBlinking, DisableMouseCapture, DisableBracketedPaste, LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}
