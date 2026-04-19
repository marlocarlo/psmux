#[allow(unused_imports)]
use std::env;
use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::types::{AppState, Action, Bind};
use crate::commands::parse_command_to_action;

// Track the current config file being parsed (for #{current_file}, #{d:current_file})
use super::*;

/// Execute a run-shell / run command from config or hooks.
/// Syntax: run-shell [-b] <command>
/// Always spawns non-blocking to avoid deadlocks when hooks fire on the
/// server thread (scripts may call back to psmux via CLI).
pub(crate) fn parse_run_shell(app: &mut AppState, line: &str) {
    // Use quote-aware parser to properly handle nested quotes and escapes
    let args = crate::commands::parse_command_line(line);
    if args.len() < 2 { return; }
    let mut cmd_parts: Vec<&str> = Vec::new();
    for arg in &args[1..] {
        if arg == "-b" { /* background flag — always spawn anyway */ }
        else { cmd_parts.push(arg); }
    }
    let shell_cmd = cmd_parts.join(" ");
    if shell_cmd.is_empty() { return; }

    // Expand ~ to home directory + XDG fallback for plugin paths
    let shell_cmd = crate::util::expand_run_shell_path(&shell_cmd);

    // ── Handle .tmux files natively ──────────────────────────────────
    // .tmux files are bash scripts used by tmux plugins. On Windows they
    // can't be executed by pwsh. Parse them for `tmux source`, `tmux set`,
    // etc. and apply the extracted commands as config lines.
    let trimmed_cmd = shell_cmd.trim().trim_matches('\'').trim_matches('"');
    if trimmed_cmd.ends_with(".tmux") {
        let tmux_path = std::path::Path::new(trimmed_cmd);
        if tmux_path.is_file() {
            parse_tmux_entry_script(app, tmux_path);
            return;
        }
    }
    // Also handle .ps1 files natively when possible: if the command is a
    // bare .ps1 path (no arguments), we can run it directly with pwsh -File
    // which is more reliable than -Command for script paths with spaces.

    // Always spawn non-blocking: run-shell commands from hooks may call back
    // to the psmux server (e.g., `psmux set -g @option value`), which would
    // deadlock if we blocked the server thread with .output().
    // Set PSMUX_TARGET_SESSION so child scripts connect to the correct server
    // (especially important when using -L socket namespaces like in tppanel preview).
    let target_session = app.port_file_base();
    let mut cmd = crate::commands::build_run_shell_command(&shell_cmd);
    if !target_session.is_empty() {
        cmd.env("PSMUX_TARGET_SESSION", &target_session);
    }
    let _ = cmd.spawn();
}

/// Parse a `.tmux` entry script (bash) and extract tmux commands from it.
///
/// .tmux files are the standard entry point for tmux plugins. They are bash
/// scripts that typically call `tmux source <file>`, `tmux set -g ...`, etc.
/// On Windows we can't run bash, so we parse the script and translate the
/// tmux CLI calls into psmux config lines.
///
/// Supported patterns:
/// Parse a .ps1 plugin entry script and extract psmux set/bind commands.
///
/// Plugin .ps1 scripts use patterns like:
///   & $PSMUX set -g key value 2>&1 | Out-Null
///   & $PSMUX bind-key ...
///
/// We extract the psmux command portion and apply it as config.
/// Returns true if all extracted values are literal (no unresolved PS variables).
/// Returns false if the script uses PowerShell variables that need runtime eval.
pub(crate) fn parse_ps1_plugin_script(app: &mut AppState, content: &str) -> bool {
    let mut has_ps_vars = false;
    let mut applied_any = false;

    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') { continue; }

        // Match patterns like: & $PSMUX set -g ... 2>&1 | Out-Null
        // Also: & $PSMUX bind-key ... 2>&1 | Out-Null
        let cmd_start = if let Some(pos) = l.find("$PSMUX ") {
            // Ensure it's preceded by "& " (PowerShell call operator)
            let prefix = &l[..pos];
            if prefix.trim_end().ends_with('&') {
                Some(pos + 7) // skip "$PSMUX "
            } else {
                None
            }
        } else {
            None
        };

        let cmd = match cmd_start {
            Some(start) => &l[start..],
            None => continue,
        };

        // Strip trailing PowerShell noise: 2>&1 | Out-Null, 2>$null
        let cmd = cmd.split(" 2>&1").next().unwrap_or(cmd);
        let cmd = cmd.split(" 2>$null").next().unwrap_or(cmd);
        let cmd = cmd.trim();

        // Check for unresolved PowerShell variables (e.g., $bg1, $fg)
        // but not $PSMUX or $TMUX which are expected patterns
        if cmd.contains('$') {
            // Check if it's a PS variable reference (not env var pattern)
            let has_var = cmd.split('$').skip(1).any(|part| {
                let first_word: String = part.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                !first_word.is_empty() && first_word != "PSMUX" && first_word != "TMUX"
                    && first_word != "env" && first_word != "null"
            });
            if has_var { has_ps_vars = true; }
        }

        if cmd.starts_with("set ") || cmd.starts_with("set-option ")
            || cmd.starts_with("bind-key ") || cmd.starts_with("bind ")
            || cmd.starts_with("setw ") || cmd.starts_with("set-window-option ") {
            if !has_ps_vars {
                parse_config_line(app, cmd);
                applied_any = true;
            }
        }
    }

    // Return true if we applied commands and they were all literal
    applied_any && !has_ps_vars
}

///   tmux source[-file] "path"       → source-file "path"
///   tmux set[-option] [-g] key val  → set [-g] key val
///   tmux setw key val               → setw key val
///   PLUGIN_DIR=...                  → track for variable expansion
pub(crate) fn parse_tmux_entry_script(app: &mut AppState, path: &std::path::Path) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Determine the directory of the .tmux file for $PLUGIN_DIR / ${PLUGIN_DIR}
    let plugin_dir = path.parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Also look for PLUGIN_DIR assignment in the script (may differ)
    let mut script_plugin_dir = plugin_dir.clone();
    // Common pattern:  PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    // We can't evaluate bash, so we just use the file's parent directory.

    for line in content.lines() {
        let l = line.trim();
        // Skip empty lines, comments, shebang
        if l.is_empty() || l.starts_with('#') { continue; }

        // Track explicit PLUGIN_DIR assignment (best-effort)
        if l.starts_with("PLUGIN_DIR=") || l.starts_with("export PLUGIN_DIR=") {
            // If it's a simple literal path, use it
            let val = l.splitn(2, '=').nth(1).unwrap_or("").trim_matches('"').trim_matches('\'');
            if !val.contains('$') && !val.contains('`') && !val.is_empty() {
                script_plugin_dir = val.to_string();
            }
            // Otherwise keep using the .tmux file's parent dir
            continue;
        }

        // Skip other bash-isms (variable assignments, if/fi, for, etc.)
        if l.contains("BASH_SOURCE") || l.starts_with("cd ") || l.starts_with("export ")
            || l.starts_with("if ") || l == "fi" || l.starts_with("for ")
            || l.starts_with("done") || l.starts_with("then") || l.starts_with("else")
            || l.starts_with("local ") || l.starts_with("readonly ") {
            continue;
        }

        // Extract tmux commands: look for lines starting with `tmux `
        let tmux_cmd = if l.starts_with("tmux ") {
            &l[5..]
        } else if l.starts_with("\"$TMUX_PROGRAM\" ") || l.starts_with("$TMUX_PROGRAM ") {
            // Some plugins use $TMUX_PROGRAM variable
            let start = l.find(' ').unwrap_or(l.len());
            l[start..].trim()
        } else {
            continue;
        };

        // Expand $PLUGIN_DIR, ${PLUGIN_DIR}, $CURRENT_DIR, ${CURRENT_DIR}
        let expanded = tmux_cmd
            .replace("${PLUGIN_DIR}", &script_plugin_dir)
            .replace("$PLUGIN_DIR", &script_plugin_dir)
            .replace("${CURRENT_DIR}", &script_plugin_dir)
            .replace("$CURRENT_DIR", &script_plugin_dir);

        // Now parse the tmux subcommand as a psmux config line
        let expanded = expanded.trim();
        if expanded.starts_with("source-file ") || expanded.starts_with("source ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("set-option ") || expanded.starts_with("set ")
            || expanded.starts_with("set -g ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("setw ") || expanded.starts_with("set-window-option ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("run-shell ") || expanded.starts_with("run ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("bind-key ") || expanded.starts_with("bind ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("if-shell ") || expanded.starts_with("if ") {
            parse_config_line(app, expanded);
        } else if expanded.starts_with("set-hook ") {
            parse_config_line(app, expanded);
        } else {
            // Try to parse it anyway — it might be a valid config directive
            parse_config_line(app, expanded);
        }
    }

    // Fallback: if we didn't find any tmux commands in the script, try to
    // source .conf files from the same directory (many themes ship both
    // .tmux entry script and .conf files).
    // Check if we actually parsed anything by looking at common indicators
    // (status-left, status-right being changed from defaults).
    // For now, also auto-source any *_tmux.conf or *.conf files in the dir.
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if ext == "conf" {
                        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        // Source companion .conf files (but not the .tmux script itself)
                        // Prioritize files like plugin_name_options.conf, plugin_name.conf
                        if fname.ends_with("_tmux.conf") || fname.ends_with("_options_tmux.conf") {
                            source_file(app, &p.to_string_lossy());
                        }
                    }
                }
            }
        }
    }
}

/// Execute an if-shell / if command from config.
/// Syntax: if-shell [-bF] <condition> <true-cmd> [<false-cmd>]
/// Runs the condition command (or evaluates format with -F), then executes the
/// appropriate branch command as a config line.
pub(crate) fn parse_if_shell(app: &mut AppState, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 { return; }

    let mut format_mode = false;
    let mut _background = false;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1;
    while i < parts.len() {
        match parts[i] {
            "-b" => { _background = true; }
            "-F" => { format_mode = true; }
            "-bF" | "-Fb" => { _background = true; format_mode = true; }
            "-t" => { i += 1; } // skip target
            s => {
                // Handle quoted strings that might span multiple parts
                if s.starts_with('"') || s.starts_with('\'') {
                    let quote = s.chars().next().unwrap();
                    if s.ends_with(quote) && s.len() > 1 {
                        positional.push(s[1..s.len()-1].to_string());
                    } else {
                        let mut buf = s[1..].to_string();
                        i += 1;
                        while i < parts.len() {
                            buf.push(' ');
                            buf.push_str(parts[i]);
                            if parts[i].ends_with(quote) {
                                buf.truncate(buf.len() - 1);
                                break;
                            }
                            i += 1;
                        }
                        positional.push(buf);
                    }
                } else {
                    positional.push(s.to_string());
                }
            }
        }
        i += 1;
    }

    if positional.len() < 2 { return; }
    let condition = &positional[0];
    let true_cmd = &positional[1];
    let false_cmd = positional.get(2);

    let success = if format_mode {
        let expanded = crate::format::expand_format(condition, app);
        !expanded.is_empty() && expanded != "0"
    } else if condition == "true" || condition == "1" {
        true
    } else if condition == "false" || condition == "0" {
        false
    } else {
        let (shell_prog, shell_args) = crate::commands::resolve_run_shell();
        let mut c = std::process::Command::new(&shell_prog);
        for a in &shell_args { c.arg(a); }
        c.arg(condition);
        { use crate::platform::HideWindowCommandExt; c.hide_window(); }
        c.status().map(|s| s.success()).unwrap_or(false)
    };

    let cmd_to_run = if success { Some(true_cmd) } else { false_cmd };
    if let Some(cmd) = cmd_to_run {
        // Execute the branch as a config line (recursive — supports set, bind, source, etc.)
        parse_config_line(app, cmd);
    }
}

#[cfg(test)]
#[path = "../../../tests-rs/test_config_plugin_paths.rs"]
mod tests_plugin_paths;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue137_env_leak.rs"]
mod tests_issue137_env_leak;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue157_bind_key_case.rs"]
mod tests_issue157_bind_key_case;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue145_source_file.rs"]
mod tests_issue145_source_file;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue193_scroll_enter_copy_mode.rs"]
mod tests_issue193_scroll_enter_copy_mode;

#[cfg(test)]
#[path = "../../../tests-rs/test_issue198_unbind_individual.rs"]
mod tests_issue198_unbind_individual;

#[cfg(test)]
#[path = "../../../tests-rs/test_config_exhaustive.rs"]
mod tests_config_exhaustive;
