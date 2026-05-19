// Integration tests for issue #299: feed byte streams a shell's integration
// hooks emit (OSC 7 for cwd, OSC 133 for prompt/command marks) through
// vt100::Parser and verify Screen::path() and Screen::shell_command() resolve
// correctly. The first two tests drive synthetic OSC 133 streams; the last two
// drive a captured oh-my-posh + command-mark stream (see the E2E note below).

#[test]
fn synthetic_osc133_stream_round_trips_through_parser() {
    // A synthetic two-cycle stream exercising the full OSC 133 state machine
    // (A prompt-start, B input-start, C command, D done) interleaved with OSC 7
    // cwd updates. Two commands: `git status`, then `dotnet build`.
    let stream = b"\
\x1b]7;file://dev-host/C:/repos/example\x07\
\x1b]133;A\x07\
\x1b]133;B\x07\
\x1b]133;C;cmdline_url=git%20status\x07\
output of git status\n\
\x1b]133;D;0\x07\
\x1b]7;file://dev-host/C:/repos/example\x07\
\x1b]133;A\x07\
\x1b]133;B\x07\
\x1b]133;C;cmdline_url=dotnet%20build\x07";

    let mut p = vt100::Parser::new(24, 80, 0);

    // Feed it chunk by chunk to also exercise the OSC parser's cross-chunk
    // stitching path.
    let chunks: Vec<&[u8]> = stream.chunks(13).collect();
    for chunk in chunks {
        p.process(chunk);
    }

    // After processing the full stream, the parser should be in the state
    // "command 'dotnet build' is running" because the last sequence was
    // OSC 133;C;cmdline_url=dotnet%20build with no subsequent D.
    assert_eq!(
        p.screen().shell_command(),
        Some("dotnet build"),
        "final shell_command should reflect the last C-without-D"
    );

    // OSC 7 should have set the path. parse_osc7_uri strips the hostname and keeps
    // the leading slash, so `file://dev-host/C:/repos/example` becomes
    // `/C:/repos/example`.
    assert_eq!(
        p.screen().path(),
        Some("/C:/repos/example"),
        "OSC 7 must populate Screen::path"
    );
}

#[test]
fn osc133_idle_state_after_done() {
    // Same shape but trimmed at OSC 133;D — should be Idle, no command.
    let stream = b"\
\x1b]7;file://host/some/path\x07\
\x1b]133;A\x07\
\x1b]133;B\x07\
\x1b]133;C;cmdline_url=ls\x07\
output\n\
\x1b]133;D;0\x07";

    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(stream);

    assert_eq!(
        p.screen().shell_command(),
        None,
        "after OSC 133;D the shell should be reported as idle"
    );
}

// ---------------------------------------------------------------------------
// E2E: a captured pwsh + oh-my-posh (pwd: osc7) + command-mark stream
// ---------------------------------------------------------------------------
//
// fixtures/issue299-omp-snippet-e2e.bin holds two real oh-my-posh prompt
// renders (each an ANSI body + OSC 0 title + OSC 7 cwd) around one command
// cycle: OSC 133;C;cmdline_url=git%20status, the command's output, then
// OSC 133;D. It was captured from `oh-my-posh print primary` in a generic cwd,
// with the OSC 133 marks emitted per the shell hook's format and the machine
// hostname scrubbed to `dev-host`.
//
// This proves the division of labor: oh-my-posh emits OSC 7 (cwd) while the
// shell hooks emit OSC 133;C/D (command identity); both flow through the same
// byte stream, so psmux's parser surfaces Screen::path() (from OSC 7) and
// Screen::shell_command() (from OSC 133;C;cmdline_url=) together.

const OMP_SNIPPET_E2E_STREAM: &[u8] =
    include_bytes!("fixtures/issue299-omp-snippet-e2e.bin");

#[test]
fn e2e_omp_cwd_and_command_marks_coexist() {
    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(OMP_SNIPPET_E2E_STREAM);

    // oh-my-posh (via pwd: osc7) must have populated the OSC 7 path. Assert a
    // structural property that holds regardless of slash direction.
    let path = p.screen().path().expect("OSC 7 path should be set");
    assert!(
        path.contains("example"),
        "captured cwd was C:/repos/example; path={path:?}"
    );

    // The command marks surfaced the command identity, then OSC 133;D cleared
    // it; the trailing prompt render emits no OSC 133, so the final state is
    // Idle. Same check as osc133_idle_state_after_done, but on the real stream.
    assert_eq!(
        p.screen().shell_command(),
        None,
        "OSC 133;D must have cleared shell_command"
    );
}

#[test]
fn e2e_command_identity_visible_before_done() {
    // Same captured stream, trimmed just AFTER OSC 133;C;cmdline_url and BEFORE
    // OSC 133;D. At that moment shell_command must hold "git status".
    let stream = OMP_SNIPPET_E2E_STREAM;
    let needle_d = b"\x1b]133;D";
    let d_pos = stream
        .windows(needle_d.len())
        .position(|w| w == needle_d)
        .expect("133;D marker");

    let mut p = vt100::Parser::new(24, 80, 0);
    p.process(&stream[..d_pos]);

    assert_eq!(
        p.screen().shell_command(),
        Some("git status"),
        "mid-stream (after OSC 133;C;cmdline_url, before OSC 133;D), shell_command must hold the typed command"
    );

    // And the path is still set from the first prompt's OSC 7.
    let path = p.screen().path().expect("OSC 7 path should still be set");
    assert!(path.contains("example"));
}
