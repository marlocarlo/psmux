use super::*;

/// Build a CommandBuilder for direct execution (no shell wrapping).
/// raw_args[0] is the program, rest are its arguments.
/// Used when -- separator is specified in new-session.
pub fn build_raw_command(raw_args: &[String]) -> CommandBuilder {
    if raw_args.is_empty() {
        return build_command(None, true, false);
    }
    let program = &raw_args[0];
    let mut builder = CommandBuilder::new(program);
    // Set CWD explicitly — portable_pty on Windows defaults to USERPROFILE
    // (home dir) when no cwd is set on CommandBuilder.
    if let Ok(dir) = std::env::current_dir() { builder.cwd(dir); }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("PSMUX_SESSION", "1");
    if raw_args.len() > 1 {
        let args: Vec<&str> = raw_args[1..].iter().map(|s| s.as_str()).collect();
        builder.args(args);
    }
    builder
}
