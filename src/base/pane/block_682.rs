#[allow(unused_imports)]
use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::types::{AppState, Pane, Node, LayoutKind, Window};
use crate::tree::{replace_leaf_with_split, active_pane_mut, kill_leaf};
use crate::format::hostname_cached;

/// Sentinel value for cursor_shape: means "no DECSCUSR received from child yet".
/// When ConPTY passthrough mode is unavailable, DECSCUSR sequences from child
/// processes are consumed by ConPTY and never forwarded.  Using this sentinel
/// lets the rendering code skip emitting any cursor-shape override, so the
/// real terminal keeps its user-configured default cursor.
use super::*;

/// Sync PowerShell's $PWD to the OS-level CWD (#111).
/// PowerShell's `cd` (Set-Location) only updates `$PWD` internally and
/// does NOT call Win32 SetCurrentDirectory(). This means the process PEB
/// still shows the original spawn directory, causing #{pane_current_path}
/// to always return the initial CWD.
///
/// Instead of wrapping the `prompt` function (which conflicts with prompt
/// customizers like Starship, oh-my-posh, etc.), we wrap the three cmdlets
/// that actually change directories: Set-Location, Push-Location, and
/// Pop-Location.  This is invisible to prompt customizers and survives
/// `. $PROFILE` reloads.
pub(crate) const CWD_SYNC: &str = concat!(
    "if (-not (Test-Path variable:Global:__psmux_cwd_hook)) { ",
    "$Global:__psmux_cwd_hook = $true; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {}; ",
    "function Global:Set-Location { ",
    "Microsoft.PowerShell.Management\\Set-Location @args; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {} ",
    "}; ",
    "function Global:Push-Location { ",
    "Microsoft.PowerShell.Management\\Push-Location @args; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {} ",
    "}; ",
    "function Global:Pop-Location { ",
    "Microsoft.PowerShell.Management\\Pop-Location @args; ",
    "try { [System.IO.Directory]::SetCurrentDirectory($PWD.ProviderPath) } catch {} ",
    "} }",
);

/// Build the full interactive init string for PowerShell:
/// 1. Disable PSReadLine predictions (before profile — prevents #109 crash)
/// 2. Source the user's profile scripts
/// 3. If allow_predictions is false, re-disable predictions after the profile;
///    if allow_predictions is true, restore the saved original PredictionSource
///    only when the profile did not set one explicitly (#150)
/// 4. Install CWD sync hook (enables #{pane_current_path} — #111)
/// 5. Optionally append the env shim
pub(crate) fn build_psrl_init(env_shim: bool, allow_predictions: bool) -> String {
    let (pre_profile, post_profile) = if allow_predictions {
        (PSRL_CRASH_GUARD, PSRL_PRED_RESTORE)
    } else {
        (PSRL_FIX, PSRL_FIX)
    };
    let mut s = format!("{}; {}; {}; {}", pre_profile, PROFILE_SOURCE, post_profile, CWD_SYNC);
    if env_shim {
        s.push_str("; ");
        s.push_str(ENV_SHIM_PS);
    }
    s
}

/// On Windows, translate Unix-style shell wrappers to Windows equivalents.
///
/// Tools like Overstory wrap agent commands in `/bin/bash -c '...'` for
/// environment setup (unset/export). This doesn't work on Windows because
/// `/bin/bash` doesn't exist. This function:
/// 1. If the command is `/bin/bash -c '...'` or `/bin/sh -c '...'`, try to
///    find `bash.exe` in PATH and rewrite to use the resolved path.
/// 2. If bash isn't available, extract the inner script and translate
///    common bash patterns (unset, export, &&) to PowerShell equivalents.
/// 3. For other Unix absolute paths (/usr/bin/foo), try to resolve the
///    basename from PATH.
#[cfg(windows)]
pub(crate) fn resolve_unix_path(cmd: &str) -> String {
    let trimmed = cmd.trim();

    // General case: resolve Unix absolute paths (e.g. /usr/bin/python3)
    if trimmed.starts_with('/') {
        let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
        let program = parts[0];
        let basename = std::path::Path::new(program)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(program);
        if let Ok(resolved) = which::which(basename) {
            let rest = if parts.len() > 1 { parts[1] } else { "" };
            if rest.is_empty() {
                return format!("\"{}\"", resolved.to_string_lossy());
            } else {
                return format!("\"{}\" {}", resolved.to_string_lossy(), rest);
            }
        }
    }

    // No translation needed
    cmd.to_string()
}

/// Detect if a command is a `/bin/bash -c '...'` or similar pattern.
/// Returns Some((inner_script, shell_name)) if matched.
#[cfg(windows)]
pub(crate) fn detect_bash_c_wrapper(cmd: &str) -> Option<(&str, &str)> {
    let shell_prefixes = [
        ("/bin/bash -c ", "bash"),
        ("/bin/sh -c ", "sh"),
        ("/usr/bin/bash -c ", "bash"),
        ("/usr/bin/sh -c ", "sh"),
        ("/usr/bin/env bash -c ", "bash"),
        ("/usr/bin/env sh -c ", "sh"),
    ];
    for (prefix, shell_name) in &shell_prefixes {
        if cmd.starts_with(prefix) {
            let rest = &cmd[prefix.len()..];
            // Strip outer quotes (single or double)
            let inner = if (rest.starts_with('\'') && rest.ends_with('\''))
                || (rest.starts_with('"') && rest.ends_with('"'))
            {
                &rest[1..rest.len() - 1]
            } else {
                rest
            };
            return Some((inner, shell_name));
        }
    }
    None
}

/// Parse a bash-style env setup script and extract environment modifications
/// plus the final command.  Returns (env_removes, env_sets, final_command).
///
/// This approach is **shell-agnostic**: instead of translating bash syntax to
/// PowerShell/cmd syntax, we parse the env operations and apply them directly
/// on the `CommandBuilder` (via `env_remove()` / `env()`).  The final command
/// is then executed through whatever default shell the user has configured,
/// without any env-manipulation syntax that could be shell-incompatible.
#[cfg(windows)]
pub(crate) fn parse_bash_env_script(script: &str) -> (Vec<String>, Vec<(String, String)>, String) {
    let mut removes: Vec<String> = Vec::new();
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut final_parts: Vec<String> = Vec::new();

    let segments: Vec<&str> = script.split("&&").collect();
    for seg in &segments {
        let seg = seg.trim();
        if seg.is_empty() { continue; }

        if seg.starts_with("unset ") {
            let vars: Vec<&str> = seg["unset ".len()..].split_whitespace().collect();
            for var in vars {
                removes.push(var.to_string());
            }
        } else if seg.starts_with("export ") {
            let assign = &seg["export ".len()..];
            if let Some(eq_pos) = assign.find('=') {
                let var = assign[..eq_pos].to_string();
                let mut val = assign[eq_pos + 1..].trim().to_string();
                // Strip outer quotes
                if (val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\''))
                {
                    val = val[1..val.len() - 1].to_string();
                }
                // Resolve $PATH / ${PATH} references to the actual current PATH value.
                // Also fix Unix `:` separator to Windows `;`.
                if let Ok(current_path) = std::env::var("PATH") {
                    val = val.replace(":$PATH", &format!(";{}", current_path))
                             .replace(":${PATH}", &format!(";{}", current_path))
                             .replace("$PATH:", &format!("{};", current_path))
                             .replace("${PATH}:", &format!("{};", current_path))
                             .replace("$PATH", &current_path)
                             .replace("${PATH}", &current_path);
                }
                sets.push((var, val));
            }
        } else {
            // Final command or unknown segment — preserve as-is
            final_parts.push(seg.to_string());
        }
    }

    let final_cmd = final_parts.join(" && ");
    (removes, sets, final_cmd)
}

pub fn build_command(command: Option<&str>, env_shim: bool, allow_predictions: bool) -> CommandBuilder {
    // Capture CWD early — portable_pty on Windows defaults to USERPROFILE
    // (home dir) when no cwd is set on CommandBuilder, so we must set it
    // explicitly to honour the caller's working directory.
    let cwd = std::env::current_dir().ok();
    if let Some(cmd) = command {
        // On Windows, detect `/bin/bash -c '...'` wrappers used by tools like
        // Overstory and omc for env var setup before launching agents.
        // Instead of translating to shell-specific syntax (which breaks if the
        // user's default shell is bash, cmd, or a different PowerShell version),
        // we parse the env operations from the bash script and apply them directly
        // on the CommandBuilder.  The final command is then passed to whatever
        // shell `cached_shell()` resolves to, env-manipulation-free.
        #[cfg(windows)]
        let (env_removes, env_sets, cmd) = {
            let trimmed = cmd.trim();
            if let Some((inner_script, _)) = detect_bash_c_wrapper(trimmed) {
                let (removes, sets, final_cmd) = parse_bash_env_script(inner_script);
                let final_cmd = if final_cmd.is_empty() {
                    cmd.to_string()
                } else {
                    resolve_unix_path(&final_cmd)
                };
                (removes, sets, final_cmd)
            } else {
                (Vec::new(), Vec::new(), resolve_unix_path(cmd))
            }
        };
        #[cfg(not(windows))]
        let (env_removes, env_sets, cmd) = (Vec::<String>::new(), Vec::<(String, String)>::new(), cmd.to_string());

        let shell = cached_shell().map(|s| s.to_string());

        match shell {
            Some(path) => {
                let mut builder = CommandBuilder::new(&path);
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                for var in &env_removes { builder.env_remove(var); }
                for (k, v) in &env_sets { builder.env(k, v); }

                let stem = std::path::Path::new(&path).file_stem()
                    .and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if stem == "pwsh" || stem == "powershell" {
                    builder.args(["-NoLogo", "-Command", &cmd]);
                } else if matches!(stem.as_str(), "bash" | "sh" | "zsh" | "fish" | "dash" | "ash") {
                    builder.args(["-c", &cmd]);
                } else {
                    builder.args(["/C", &cmd]);
                }
                builder
            }
            None => {
                let mut builder = CommandBuilder::new("pwsh.exe");
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                for var in &env_removes { builder.env_remove(var); }
                for (k, v) in &env_sets { builder.env(k, v); }
                builder.args(["-NoLogo", "-Command", &cmd]);
                builder
            }
        }
    } else {
        let shell = cached_shell().map(|s| s.to_string());
        // PSReadLine v2.2.6+ enables PredictionSource HistoryAndPlugin by default.
        // Predictions cause display corruption in terminal multiplexers because
        // PSReadLine's VT rendering races with ConPTY output capture.
        // Issue #109: GetHistoryItems() throws NullReferenceException when
        // predictions are enabled in the profile before PSReadLine is fully
        // initialized inside ConPTY.  We use -NoProfile and source profiles
        // ourselves, sandwiching them between prediction-disable commands.
        let psrl_init = build_psrl_init(env_shim, allow_predictions);
        match shell {
            Some(path) => {
                let mut builder = CommandBuilder::new(&path);
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                if path.to_lowercase().contains("pwsh") {
                    builder.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &psrl_init]);
                }
                builder
            }
            None => {
                let mut builder = CommandBuilder::new("pwsh.exe");
                if let Some(ref dir) = cwd { builder.cwd(dir); }
                builder.env("TERM", "xterm-256color");
                builder.env("COLORTERM", "truecolor");
                builder.env("PSMUX_SESSION", "1");
                // Apply the same -NoProfile + manual profile sourcing for
                // the fallback pwsh.exe path (previously had no PSRL fix).
                builder.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &psrl_init]);
                builder
            }
        }
    }
}

/// Cached resolved default-shell path to avoid repeated `which::which()` scans.
pub(crate) static CACHED_DEFAULT_SHELL: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();

pub(crate) static CACHED_DEFAULT_SHELL_MAP: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> = std::sync::Mutex::new(None);

/// Resolve a program name via `which`, caching the result.
pub(crate) fn cached_which(program: &str) -> String {
    // Fast path: check if already cached in the global OnceLock for the default
    // (most common case is always the same shell)
    let mut map = CACHED_DEFAULT_SHELL_MAP.lock().unwrap_or_else(|e| e.into_inner());
    let map = map.get_or_insert_with(std::collections::HashMap::new);
    if let Some(cached) = map.get(program) {
        return cached.clone();
    }
    let resolved = which::which(program).ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    map.insert(program.to_string(), resolved.clone());
    resolved
}

/// Split a shell config value into (program, extra_args), handling paths
/// that contain spaces (e.g. `C:/Program Files/Git/bin/bash.exe`).
///
/// Resolution order:
/// 1. If the whole string resolves to an existing executable, use it as-is.
/// 2. Otherwise, use quote-aware tokenising so that users can write
///    `"C:/Program Files/Git/bin/bash.exe" --login` with quotes.
pub(crate) fn resolve_shell_program(shell_path: &str) -> (String, Vec<String>) {
    // Fast path: whole string is the program (possibly with spaces in path).
    if std::path::Path::new(shell_path).is_file()
        || which::which(shell_path).is_ok()
    {
        return (shell_path.to_string(), vec![]);
    }

    // Quote-aware split (handles `"path with spaces" arg1 arg2`).
    let parsed = crate::commands::parse_command_line(shell_path);
    if parsed.is_empty() {
        return (shell_path.to_string(), vec![]);
    }
    let program = parsed[0].clone();
    let extra = parsed[1..].to_vec();
    (program, extra)
}

/// Build a CommandBuilder that launches the given shell path interactively.
/// Used when `default-shell` / `default-command` is configured.
/// Supports pwsh, powershell, cmd, and any arbitrary executable.
pub fn build_default_shell(shell_path: &str, env_shim: bool, allow_predictions: bool) -> CommandBuilder {
    let (program, extra_args) = resolve_shell_program(shell_path);

    // Resolve bare names via cached `which` — avoids repeated PATH scans.
    let resolved = cached_which(&program);

    let lower = resolved.to_lowercase();
    let mut builder = CommandBuilder::new(&resolved);
    // Set CWD explicitly — portable_pty on Windows defaults to USERPROFILE
    // (home dir) when no cwd is set on CommandBuilder.
    if let Ok(dir) = std::env::current_dir() { builder.cwd(dir); }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("PSMUX_SESSION", "1");

    // Prepend extra arguments (e.g. -NoProfile) BEFORE our -NoExit/-Command block
    // so they're interpreted as flags rather than as -Command arguments.
    if !extra_args.is_empty() {
        builder.args(extra_args.clone());
    }

    if lower.contains("pwsh") || lower.contains("powershell") {
        // Issue #109: -NoProfile + manual profile sourcing to prevent
        // PSReadLine GetHistoryItems NullReferenceException.
        // If the user already passed -NoProfile in extra_args, we still
        // add ours (PowerShell accepts duplicates harmlessly) and skip
        // profile sourcing only if they explicitly opted out.
        let has_noprofile = extra_args.iter()
            .any(|a| a.eq_ignore_ascii_case("-NoProfile"));
        let psrl_init = if has_noprofile {
            // User explicitly wants no profile — just apply PSRL fix + shim.
            let mut s = PSRL_FIX.to_string();
            if env_shim {
                s.push_str("; ");
                s.push_str(ENV_SHIM_PS);
            }
            s
        } else {
            build_psrl_init(env_shim, allow_predictions)
        };
        if !has_noprofile {
            builder.args(["-NoProfile"]);
        }
        builder.args(["-NoLogo", "-NoExit", "-Command", &psrl_init]);
    }

    builder
}

