// Issue #299 - OSC parsing for command identity in shell-integration.
//
// Three escape-sequence patterns are recognized for "what command is running
// in this terminal":
//
//   * `\e]133;C;cmdline_url=<url-escaped>\a`  — kitty's fish integration
//   * `\e]1337;SetUserVar=WEZTERM_PROG=<b64>\a` — WezTerm's bash/zsh
//   * `\e]633;E;<escaped>;<nonce>\a`          — VS Code's pwsh script
//
// All produce the same observable state: `Screen::shell_command()` returns
// the literal command string. The state is cleared at prompt start and command
// done, for both 133 (`\e]133;A\a` / `\e]133;D\a`) and 633 (`\e]633;A\a` /
// `\e]633;D\a`).
//
// Tests use `vt100::Parser` directly via the public crate API, exactly like
// the OSC 9;4 progress-indicator tests in test_issue269_osc94_dropped.rs.
//
// Run with: cargo test --test test_issue299_osc_command_state -- --nocapture

const ST: &[u8] = b"\x1b\\";

// ---------------------------------------------------------------------------
// Helpers — byte builders that exactly match the on-wire shape.
// ---------------------------------------------------------------------------

fn osc133_a() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]133;A");
    v.extend_from_slice(ST);
    v
}

fn osc133_c_bare() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]133;C");
    v.extend_from_slice(ST);
    v
}

fn osc133_c_cmdline_url(cmd: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]133;C;cmdline_url=");
    v.extend_from_slice(cmd.as_bytes());
    v.extend_from_slice(ST);
    v
}

fn osc133_d() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]133;D");
    v.extend_from_slice(ST);
    v
}

fn osc1337_setuservar(name: &str, value_b64: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]1337;SetUserVar=");
    v.extend_from_slice(name.as_bytes());
    v.push(b'=');
    v.extend_from_slice(value_b64.as_bytes());
    v.extend_from_slice(ST);
    v
}

fn osc633_e(cmd: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]633;E;");
    v.extend_from_slice(cmd.as_bytes());
    v.extend_from_slice(ST);
    v
}

fn osc633_e_with_nonce(cmd: &str, nonce: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]633;E;");
    v.extend_from_slice(cmd.as_bytes());
    v.push(b';');
    v.extend_from_slice(nonce.as_bytes());
    v.extend_from_slice(ST);
    v
}

fn osc633_a() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]633;A");
    v.extend_from_slice(ST);
    v
}

fn osc633_c() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]633;C");
    v.extend_from_slice(ST);
    v
}

fn osc633_d() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]633;D;0");
    v.extend_from_slice(ST);
    v
}

// Regression-guard helper.
fn osc0_title(title: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]0;");
    v.extend_from_slice(title.as_bytes());
    v.extend_from_slice(ST);
    v
}

fn osc7_path(host: &str, path: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"\x1b]7;file://");
    v.extend_from_slice(host.as_bytes());
    v.extend_from_slice(path.as_bytes());
    v.extend_from_slice(ST);
    v
}

// ---------------------------------------------------------------------------
// Baseline — initial state has no shell_command.
// ---------------------------------------------------------------------------

#[test]
fn baseline_initial_shell_command_is_none() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(b"hello");
    assert_eq!(p.screen().shell_command(), None);
}

// ---------------------------------------------------------------------------
// OSC 133;C with cmdline_url= (kitty fish pattern; psmux-recommended for pwsh)
// ---------------------------------------------------------------------------

#[test]
fn fix_osc133c_with_cmdline_url_captures_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc133_c_cmdline_url("git%20status"));
    assert_eq!(p.screen().shell_command(), Some("git status"));
}

#[test]
fn fix_osc133c_cmdline_url_decodes_percent_encoding() {
    let mut p = vt100::Parser::new(24, 80, 0);
    // dotnet build -c Release
    p.process(&osc133_c_cmdline_url("dotnet%20build%20-c%20Release"));
    assert_eq!(p.screen().shell_command(), Some("dotnet build -c Release"));
}

#[test]
fn fix_osc133c_cmdline_url_decodes_quotes() {
    let mut p = vt100::Parser::new(24, 80, 0);
    // echo "hello world"
    p.process(&osc133_c_cmdline_url("echo%20%22hello%20world%22"));
    assert_eq!(p.screen().shell_command(), Some("echo \"hello world\""));
}

#[test]
fn fix_osc133c_cmdline_url_handles_unicode() {
    let mut p = vt100::Parser::new(24, 80, 0);
    // echo café (é = %C3%A9)
    p.process(&osc133_c_cmdline_url("echo%20caf%C3%A9"));
    assert_eq!(p.screen().shell_command(), Some("echo café"));
}

// ---------------------------------------------------------------------------
// OSC 133 state machine — A clears, D clears, C without param leaves alone
// ---------------------------------------------------------------------------

#[test]
fn fix_osc133a_clears_shell_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc133_c_cmdline_url("copilot"));
    assert_eq!(p.screen().shell_command(), Some("copilot"));
    p.process(&osc133_a());
    assert_eq!(p.screen().shell_command(), None, "OSC 133;A should clear");
}

#[test]
fn fix_osc133d_clears_shell_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc133_c_cmdline_url("copilot"));
    assert_eq!(p.screen().shell_command(), Some("copilot"));
    p.process(&osc133_d());
    assert_eq!(p.screen().shell_command(), None, "OSC 133;D should clear");
}

#[test]
fn fix_bare_osc133c_without_param_leaves_current_value() {
    // Realistic sequence from oh-my-posh + SetUserVar emitter:
    //   1337;SetUserVar=cmd=base64(copilot)  ← cache
    //   133;C                                 ← latch (bare)
    let mut p = vt100::Parser::new(24, 80, 0);
    let b64 = base64_encode(b"copilot");
    p.process(&osc1337_setuservar("WEZTERM_PROG", &b64));
    assert_eq!(p.screen().shell_command(), Some("copilot"));
    p.process(&osc133_c_bare());
    assert_eq!(
        p.screen().shell_command(),
        Some("copilot"),
        "bare OSC 133;C should preserve the pending command"
    );
}

#[test]
fn fix_prompt_cycle_full_round_trip() {
    let mut p = vt100::Parser::new(24, 80, 0);
    // Prompt starts, command typed and submitted, output, done
    p.process(&osc133_a());
    assert_eq!(p.screen().shell_command(), None);
    p.process(&osc133_c_cmdline_url("ls%20-la"));
    assert_eq!(p.screen().shell_command(), Some("ls -la"));
    p.process(&osc133_d());
    assert_eq!(p.screen().shell_command(), None);
}

// ---------------------------------------------------------------------------
// OSC 633 lifecycle — VS Code emits 633 (not 133), so the command set by
// 633;E must clear on 633;A (next prompt) and 633;D (command done), and must
// survive 633;C (pre-execution, when the command is actually running).
// Regression for a finished command latching stale and shadowing the
// process-tree fallback.
// ---------------------------------------------------------------------------

#[test]
fn fix_osc633a_clears_shell_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc633_e("copilot"));
    assert_eq!(p.screen().shell_command(), Some("copilot"));
    p.process(&osc633_a());
    assert_eq!(p.screen().shell_command(), None, "OSC 633;A should clear");
}

#[test]
fn fix_osc633d_clears_shell_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc633_e("copilot"));
    assert_eq!(p.screen().shell_command(), Some("copilot"));
    p.process(&osc633_d());
    assert_eq!(p.screen().shell_command(), None, "OSC 633;D should clear");
}

#[test]
fn fix_osc633_vscode_full_cycle() {
    // VS Code order: A prompt-start, E command-line, C pre-exec, D;<exit> done.
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc633_a());
    assert_eq!(p.screen().shell_command(), None);
    p.process(&osc633_e("dotnet build"));
    assert_eq!(p.screen().shell_command(), Some("dotnet build"));
    p.process(&osc633_c());
    assert_eq!(
        p.screen().shell_command(),
        Some("dotnet build"),
        "633;C (pre-exec) should keep the running command"
    );
    p.process(&osc633_d());
    assert_eq!(p.screen().shell_command(), None, "633;D should clear when done");
}

// ---------------------------------------------------------------------------
// OSC 1337 ; SetUserVar = WEZTERM_PROG — WezTerm precedent
// ---------------------------------------------------------------------------

#[test]
fn fix_osc1337_setuservar_wezterm_prog_captures_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    // Base64 of "vim foo.txt"
    let b64 = base64_encode(b"vim foo.txt");
    p.process(&osc1337_setuservar("WEZTERM_PROG", &b64));
    assert_eq!(p.screen().shell_command(), Some("vim foo.txt"));
}

#[test]
fn fix_osc1337_setuservar_other_vars_are_ignored() {
    // Only WEZTERM_PROG is recognized for the "current command" channel.
    // Other vars (WEZTERM_USER, WEZTERM_HOST, custom names) are not.
    let mut p = vt100::Parser::new(24, 80, 0);
    let b64 = base64_encode(b"username");
    p.process(&osc1337_setuservar("WEZTERM_USER", &b64));
    assert_eq!(p.screen().shell_command(), None);
}

#[test]
fn fix_osc1337_setuservar_with_invalid_base64_is_ignored() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc1337_setuservar("WEZTERM_PROG", "!!!not-base64!!!"));
    assert_eq!(p.screen().shell_command(), None);
}

// ---------------------------------------------------------------------------
// OSC 633 ; E — VS Code's shellIntegration.ps1
// ---------------------------------------------------------------------------

#[test]
fn fix_osc633e_captures_command() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc633_e("Get-ChildItem"));
    assert_eq!(p.screen().shell_command(), Some("Get-ChildItem"));
}

#[test]
fn fix_osc633e_with_nonce_uses_command_only() {
    // VS Code's shellIntegration.ps1 appends a nonce after the command.
    // We parse the command (everything between `633;E;` and the next `;` or ST).
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc633_e_with_nonce("ls", "abc123nonce"));
    assert_eq!(p.screen().shell_command(), Some("ls"));
}

// ---------------------------------------------------------------------------
// Regression guards — existing OSC handlers untouched.
// ---------------------------------------------------------------------------

#[test]
fn regression_osc0_title_still_captured() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc0_title("My Terminal"));
    // Title is captured (existing behavior).
    assert!(p.screen().title().contains("My Terminal"));
    // shell_command is untouched.
    assert_eq!(p.screen().shell_command(), None);
}

#[test]
fn regression_osc7_path_still_captured() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc7_path("host", "/home/user"));
    assert_eq!(p.screen().path(), Some("/home/user"));
    assert_eq!(p.screen().shell_command(), None);
}

#[test]
fn regression_cmd_capture_does_not_appear_in_grid() {
    // OSC sequences are state-machine bytes, NOT printable. They must not
    // leak into the rendered cell grid.
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&osc133_c_cmdline_url("copilot"));
    // No "133" or "cmdline_url" should appear on screen.
    let visible: String = (0..24)
        .map(|r| {
            (0..80)
                .filter_map(|c| p.screen().cell(r, c).map(|c| c.contents()))
                .collect::<String>()
        })
        .collect();
    assert!(!visible.contains("133"), "OSC bytes leaked into grid: {visible:?}");
    assert!(!visible.contains("cmdline_url"));
}

// ---------------------------------------------------------------------------
// Cross-chunk feeding — OSC sequences split across process() calls.
// ---------------------------------------------------------------------------

#[test]
fn fix_osc133c_split_across_chunks() {
    let bytes = osc133_c_cmdline_url("vim%20test.txt");
    // Split at every offset; the answer should still come out.
    for split in 1..bytes.len() {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(&bytes[..split]);
        p.process(&bytes[split..]);
        assert_eq!(
            p.screen().shell_command(),
            Some("vim test.txt"),
            "split at {} broke parsing",
            split
        );
    }
}

// ---------------------------------------------------------------------------
// Local base64 encoder so the tests don't pull a base64 dep.
// ---------------------------------------------------------------------------

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
