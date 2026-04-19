use std::io;
use std::env;

pub(crate) struct ParsedArgs<'a> {
    pub l_socket_name: Option<String>,
    pub f_config_file: Option<String>,
    pub control_mode: u8,
    pub cmd_args: Vec<&'a String>,
}

pub(crate) fn parse_and_resolve_args<'a>(args: &'a [String]) -> io::Result<ParsedArgs<'a>> {
    // Parse -L flag early (tmux-compatible: names the server socket for namespace isolation)
    // In psmux, -L <name> creates a namespace prefix for session port/key files.
    // Sessions under -L "foo" are stored as "foo__sessionname.port".
    // IMPORTANT: Only recognize -L as a global flag when it appears BEFORE the subcommand.
    // This avoids conflict with subcommand flags (e.g. select-pane -L, resize-pane -L).
    let mut l_socket_name: Option<String> = None;
    let mut f_config_file: Option<String> = None;
    let mut control_mode: u8 = 0; // 0=off, 1=-C (echo), 2=-CC (no echo)
    {
        let mut i = 1; // skip binary name
        while i < args.len() {
            let arg = &args[i];
            if arg == "-CC" {
                control_mode = 2;
                i += 1;
            } else if arg == "-C" {
                control_mode = 1;
                i += 1;
            } else if arg == "-L" && i + 1 < args.len() {
                l_socket_name = Some(args[i + 1].clone());
                i += 2;
            } else if arg == "-f" && i + 1 < args.len() {
                f_config_file = Some(args[i + 1].clone());
                i += 2;
            } else if (arg == "-S" || arg == "-t") && i + 1 < args.len() {
                i += 2; // skip other global flag-value pairs
            } else if arg.starts_with('-') {
                i += 1; // skip single global flags (e.g. -v, -V)
            } else {
                break; // hit the subcommand name — stop scanning for global flags
            }
        }
    }

    // Set PSMUX_CONFIG_FILE if -f was provided, so load_config() picks it up.
    if let Some(ref cf) = f_config_file {
        env::set_var("PSMUX_CONFIG_FILE", cf);
    }

    // Parse -t flag early to set target session for all commands
    // Supports session:window.pane format (e.g., "dev:0.1")
    // PSMUX_TARGET_SESSION stores the port file base name (for port file lookup)
    // PSMUX_TARGET_FULL stores the full target (session:window.pane) for the server
    if let Some(pos) = args.iter().position(|a| a == "-t") {
        if let Some(target) = args.get(pos + 1) {
            // Store the full target for the server to parse
            env::set_var("PSMUX_TARGET_FULL", target);
            // Extract just the session name for port file lookup
            let parsed_target = crate::cli::parse_target(target);
            let has_explicit_session = parsed_target.session.is_some();
            let session = parsed_target.session.unwrap_or_else(|| "default".to_string());
            // Apply -L namespace prefix for port file lookup
            let port_file_base = if let Some(ref l) = l_socket_name {
                format!("{}__{}", l, session)
            } else {
                session.clone()
            };
            // If the -t target includes an explicit session name, use it
            // directly. Otherwise (e.g. -t %2, -t :1.0) fall through to
            // the TMUX env var resolution below so we connect to the right
            // server when invoked from inside a psmux pane.
            //
            // Exception: for switch-client, -t is the DESTINATION session,
            // not the server to route the command to. Skip setting
            // PSMUX_TARGET_SESSION so the TMUX-based fallback below resolves
            // the current (source) session for routing. PSMUX_TARGET_FULL
            // still carries the destination for the server handler.
            let is_switch_client = args.iter().any(|a| a == "switch-client" || a == "switchc");
            if has_explicit_session && !is_switch_client {
                env::set_var("PSMUX_TARGET_SESSION", &port_file_base);
            }
        }
    }
    if env::var("PSMUX_TARGET_SESSION").is_err() {
        // No explicit session from -t: try to resolve from TMUX env var (set inside psmux panes)
        // TMUX format: /tmp/psmux-<pid>/<socket_name>,<port>,<session_idx>
        if let Ok(tmux_val) = env::var("TMUX") {
            // Extract the port from the TMUX value
            let parts: Vec<&str> = tmux_val.split(',').collect();
            if parts.len() >= 2 {
                if let Ok(port) = parts[1].trim().parse::<u16>() {
                    // Look up which session owns this port (port file base
                    // already includes -L namespace prefix if applicable)
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
                    let psmux_dir = format!("{}\\.psmux", home);
                    if let Ok(entries) = std::fs::read_dir(&psmux_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().map(|e| e == "port").unwrap_or(false) {
                                if let Ok(port_str) = std::fs::read_to_string(&path) {
                                    if let Ok(file_port) = port_str.trim().parse::<u16>() {
                                        if file_port == port {
                                            if let Some(port_file_base) = path.file_stem().and_then(|s| s.to_str()) {
                                                // Skip warm (standby) sessions — they are internal-only
                                                if !crate::session::is_warm_session(port_file_base) {
                                                    env::set_var("PSMUX_TARGET_SESSION", port_file_base);
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Fallback: if no -t flag and session still not resolved (e.g. TMUX pointed
    // to a warm session, or no TMUX at all), pick the most recent real session.
    // When -L namespace is active, only resolve within that namespace.
    if env::var("PSMUX_TARGET_SESSION").is_err() {
        if let Some(name) = crate::session::resolve_last_session_name_ns(l_socket_name.as_deref()) {
            env::set_var("PSMUX_TARGET_SESSION", &name);
        }
    }

    // Find the actual command by skipping global -t/-L and their arguments.
    // -t is stripped everywhere (the global handler already set PSMUX_TARGET_SESSION).
    // -L is only stripped BEFORE the subcommand (global socket namespace flag);
    // after the subcommand, -L is kept (e.g. select-pane -L, resize-pane -L).
    let cmd_args: Vec<&'a String> = {
        let mut result = Vec::new();
        let mut i = 1; // skip binary name
        let mut found_subcommand = false;
        while i < args.len() {
            if !found_subcommand {
                // Before subcommand: skip global flags with values
                if (args[i] == "-t" || args[i] == "-L" || args[i] == "-f" || args[i] == "-S") && i + 1 < args.len() {
                    i += 2; // skip flag and its value
                    continue;
                } else if args[i] == "-h" || args[i] == "--help"
                       || args[i] == "-V" || args[i] == "-v" || args[i] == "--version" {
                    // Treat help/version flags as the subcommand itself
                    found_subcommand = true;
                    // fall through to push
                } else if args[i].starts_with('-') {
                    i += 1; // skip single global flags (e.g. -v)
                    continue;
                } else {
                    found_subcommand = true;
                    // fall through to push the subcommand name
                }
            } else {
                // After subcommand: strip only -t (and its value)
                if args[i] == "-t" && i + 1 < args.len() {
                    i += 2;
                    continue;
                }
            }
            result.push(&args[i]);
            i += 1;
        }
        result
    };

    Ok(ParsedArgs {
        l_socket_name,
        f_config_file,
        control_mode,
        cmd_args,
    })
}
