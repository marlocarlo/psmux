// Tests for run-shell shell binary resolution and build_run_shell_command
//
// Covers:
// 1. resolve_shell_binary fallback between pwsh/powershell
// 2. build_run_shell_command Case 1 (shell binary prefix)
// 3. build_run_shell_command Case 2 (bare .ps1 file)
// 4. build_run_shell_command Case 3 (generic command)
// 5. expand_run_shell_path tilde expansion
// 6. run-shell command parsing from execute_command_string

use super::*;

fn mock_app() -> AppState {
    let mut app = AppState::new("test_session".to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app
}

fn make_window(name: &str, id: usize) -> crate::types::Window {
    crate::types::Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: name.to_string(),
        id,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: std::time::Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: vec![],
        zoom_saved: None,
        linked_from: None,
    }
}

fn mock_app_with_window() -> AppState {
    let mut app = mock_app();
    app.windows.push(make_window("shell", 0));
    app
}

// ─── resolve_shell_binary tests ─────────────────────────────────────────────

#[test]
#[cfg(windows)]
fn resolve_shell_binary_pwsh_returns_valid_shell() {
    // On this test machine, at least one of pwsh/powershell must exist
    let result = resolve_shell_binary("pwsh");
    // Should either return "pwsh" (if found) or a full path to powershell (fallback)
    let lower = result.to_lowercase();
    assert!(
        lower.contains("pwsh") || lower.contains("powershell"),
        "resolve_shell_binary('pwsh') should return a PS shell, got: {}",
        result
    );
}

#[test]
#[cfg(windows)]
fn resolve_shell_binary_powershell_returns_valid_shell() {
    let result = resolve_shell_binary("powershell");
    let lower = result.to_lowercase();
    assert!(
        lower.contains("pwsh") || lower.contains("powershell"),
        "resolve_shell_binary('powershell') should return a PS shell, got: {}",
        result
    );
}

#[test]
#[cfg(windows)]
fn resolve_shell_binary_powershell_exe_returns_valid_shell() {
    let result = resolve_shell_binary("powershell.exe");
    let lower = result.to_lowercase();
    assert!(
        lower.contains("pwsh") || lower.contains("powershell"),
        "resolve_shell_binary('powershell.exe') should return a PS shell, got: {}",
        result
    );
}

#[test]
#[cfg(windows)]
fn resolve_shell_binary_cmd_passthrough() {
    let result = resolve_shell_binary("cmd");
    assert_eq!(result, "cmd", "cmd should pass through unchanged");
}

#[test]
#[cfg(windows)]
fn resolve_shell_binary_cmd_exe_passthrough() {
    let result = resolve_shell_binary("cmd.exe");
    assert_eq!(result, "cmd.exe", "cmd.exe should pass through unchanged");
}

#[test]
#[cfg(windows)]
fn resolve_shell_binary_arbitrary_passthrough() {
    let result = resolve_shell_binary("notepad");
    assert_eq!(result, "notepad", "unknown binaries should pass through unchanged");
}

#[test]
#[cfg(windows)]
fn resolve_shell_binary_full_path_passthrough() {
    let result = resolve_shell_binary(r"C:\Windows\System32\cmd.exe");
    assert_eq!(result, r"C:\Windows\System32\cmd.exe", "full paths should pass through unchanged");
}

// ─── build_run_shell_command tests ──────────────────────────────────────────

#[test]
#[cfg(windows)]
fn build_run_shell_command_pwsh_prefix_creates_valid_command() {
    // Simulates what PPM plugin.conf does:
    // bind-key I run-shell 'pwsh -NoProfile -ExecutionPolicy Bypass -File "~/.psmux/plugins/ppm/scripts/install_plugins.ps1"'
    let cmd = build_run_shell_command("pwsh -NoProfile -Command echo hello");
    let prog = cmd.get_program().to_string_lossy().to_lowercase();
    assert!(
        prog.contains("pwsh") || prog.contains("powershell"),
        "Should resolve to a valid PS shell, got: {}",
        prog
    );
}

#[test]
#[cfg(windows)]
fn build_run_shell_command_powershell_prefix_creates_valid_command() {
    let cmd = build_run_shell_command("powershell.exe -ExecutionPolicy Bypass -File test.ps1");
    let prog = cmd.get_program().to_string_lossy().to_lowercase();
    assert!(
        prog.contains("pwsh") || prog.contains("powershell"),
        "Should resolve to a valid PS shell, got: {}",
        prog
    );
}

#[test]
#[cfg(windows)]
fn build_run_shell_command_cmd_prefix_passes_through() {
    let cmd = build_run_shell_command("cmd /c echo hello");
    let prog = cmd.get_program().to_string_lossy().to_lowercase();
    assert!(
        prog.contains("cmd"),
        "cmd should pass through to cmd, got: {}",
        prog
    );
}

#[test]
#[cfg(windows)]
fn build_run_shell_command_generic_command_uses_shell_wrapper() {
    let cmd = build_run_shell_command("echo hello");
    let prog = cmd.get_program().to_string_lossy().to_lowercase();
    // Generic commands get wrapped in a shell (Case 3)
    assert!(
        prog.contains("pwsh") || prog.contains("powershell") || prog.contains("cmd"),
        "Generic command should be wrapped in a shell, got: {}",
        prog
    );
}

#[test]
#[cfg(windows)]
fn build_run_shell_command_preserves_args() {
    let cmd = build_run_shell_command("pwsh -NoProfile -Command echo hello world");
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
    assert!(args.contains(&"-NoProfile".to_string()), "Should preserve -NoProfile arg");
    assert!(args.contains(&"-Command".to_string()), "Should preserve -Command arg");
}

// ─── expand_run_shell_path tests ────────────────────────────────────────────

#[test]
fn expand_tilde_forward_slash() {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let result = crate::util::expand_run_shell_path("~/.psmux/plugins/ppm/ppm.ps1");
    assert!(
        result.contains(&home),
        "~ should be expanded to home dir. Got: {}",
        result
    );
    assert!(
        !result.starts_with('~'),
        "Result should not start with ~ after expansion. Got: {}",
        result
    );
}

#[test]
fn expand_tilde_backslash() {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let result = crate::util::expand_run_shell_path(r"~\.psmux\plugins\ppm\ppm.ps1");
    assert!(
        result.contains(&home),
        "~ should be expanded to home dir. Got: {}",
        result
    );
}

#[test]
fn expand_no_tilde_unchanged() {
    let result = crate::util::expand_run_shell_path("/absolute/path/script.ps1");
    assert_eq!(result, "/absolute/path/script.ps1");
}

#[test]
fn expand_dollar_home_not_expanded() {
    // $HOME is a shell variable, not handled by expand_run_shell_path
    let result = crate::util::expand_run_shell_path("$HOME/.psmux/plugins/ppm/ppm.ps1");
    assert!(
        result.starts_with("$HOME"),
        "$HOME should NOT be expanded (that is shell's job). Got: {}",
        result
    );
}

// ─── run-shell command parsing via execute_command_string ───────────────────

#[test]
fn run_shell_no_args_shows_usage() {
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "run-shell").unwrap();
    assert!(
        app.status_message.is_some(),
        "run-shell with no args should set a status message"
    );
    let (msg, ..) = app.status_message.as_ref().unwrap();
    assert!(
        msg.contains("usage"),
        "Should show usage message, got: {}",
        msg
    );
}

#[test]
fn run_alias_no_args_shows_usage() {
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "run").unwrap();
    assert!(
        app.status_message.is_some(),
        "run with no args should set a status message"
    );
    let (msg, ..) = app.status_message.as_ref().unwrap();
    assert!(
        msg.contains("usage"),
        "Should show usage message, got: {}",
        msg
    );
}

#[test]
fn run_shell_background_flag_doesnt_block() {
    let mut app = mock_app_with_window();
    // -b flag should fire and forget
    execute_command_string(&mut app, "run-shell -b echo test").unwrap();
    // Should NOT set a "running:" status message (that is for foreground only)
    if let Some((msg, ..)) = &app.status_message {
        assert!(
            !msg.contains("running:"),
            "Background run should not set running status, got: {}",
            msg
        );
    }
}

#[test]
fn run_shell_foreground_sets_running_status() {
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "run-shell echo test_run_shell").unwrap();
    assert!(
        app.status_message.is_some(),
        "Foreground run-shell should set status message"
    );
    let (msg, ..) = app.status_message.as_ref().unwrap();
    assert!(
        msg.contains("running:"),
        "Should show 'running: ...' status, got: {}",
        msg
    );
}

#[test]
fn run_shell_quoted_command() {
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "run-shell 'echo hello world'").unwrap();
    assert!(
        app.status_message.is_some(),
        "Quoted run-shell should set status message"
    );
}

#[test]
fn run_shell_double_quoted_command() {
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, r#"run-shell "echo hello world""#).unwrap();
    assert!(
        app.status_message.is_some(),
        "Double quoted run-shell should set status message"
    );
}

// ─── ensure_background tests ────────────────────────────────────────────────

#[test]
fn ensure_background_adds_flag_to_run_shell() {
    let result = ensure_background("run-shell echo test");
    assert!(result.contains("-b"), "Should add -b flag. Got: {}", result);
    assert!(result.starts_with("run-shell -b"), "Flag should be right after command. Got: {}", result);
}

#[test]
fn ensure_background_adds_flag_to_run_alias() {
    let result = ensure_background("run echo test");
    assert!(result.contains("-b"), "Should add -b flag. Got: {}", result);
    assert!(result.starts_with("run -b"), "Flag should be right after alias. Got: {}", result);
}

#[test]
fn ensure_background_noop_when_already_background() {
    let result = ensure_background("run-shell -b echo test");
    assert_eq!(result, "run-shell -b echo test", "Should not double add -b flag");
}

#[test]
fn ensure_background_noop_for_non_run_commands() {
    let result = ensure_background("display-message hello");
    assert_eq!(result, "display-message hello", "Non run commands should be unchanged");
}

// ─── parse_command_line tests for edge cases ────────────────────────────────

#[test]
fn parse_command_line_simple() {
    let parts = parse_command_line("echo hello world");
    assert_eq!(parts, vec!["echo", "hello", "world"]);
}

#[test]
fn parse_command_line_double_quoted() {
    let parts = parse_command_line(r#"echo "hello world""#);
    assert_eq!(parts, vec!["echo", "hello world"]);
}

#[test]
fn parse_command_line_single_quoted() {
    let parts = parse_command_line("echo 'hello world'");
    assert_eq!(parts, vec!["echo", "hello world"]);
}

#[test]
fn parse_command_line_mixed_quotes() {
    let parts = parse_command_line(r#"run 'pwsh -Command "echo test"'"#);
    assert_eq!(parts, vec!["run", r#"pwsh -Command "echo test""#]);
}

#[test]
fn parse_command_line_windows_path() {
    let parts = parse_command_line(r#"pwsh -File "C:\Users\test\script.ps1""#);
    assert_eq!(parts, vec!["pwsh", "-File", r"C:\Users\test\script.ps1"]);
}

#[test]
fn parse_command_line_backslash_in_double_quotes() {
    // Backslash should be literal inside double quotes (Windows paths)
    let parts = parse_command_line(r#""C:\Program Files\test.exe" -arg"#);
    assert_eq!(parts, vec![r"C:\Program Files\test.exe", "-arg"]);
}

#[test]
fn parse_command_line_empty_string() {
    let parts = parse_command_line("");
    assert!(parts.is_empty());
}
