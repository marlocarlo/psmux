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
