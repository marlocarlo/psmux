use std::ffi::OsString;
use std::fmt;

use lexopt::Arg::{Long, Short, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KillWindowCommand {
    target: Option<String>,
    all: bool,
}

impl KillWindowCommand {
    pub fn parse<I, S>(command: &str, args: I) -> Option<Result<Self, ParseError>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if !matches!(command, "kill-window" | "killw") {
            return None;
        }

        Some(Self::parse_args(args))
    }

    fn parse_args<I, S>(args: I) -> Result<Self, ParseError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args = crate::cli::normalize_flag_equals(
            args.into_iter()
                .map(|argument| argument.as_ref().to_string())
                .collect(),
        );
        let args = args.into_iter().map(OsString::from);
        let mut parser = lexopt::Parser::from_args(args);
        let mut command = Self::default();

        while let Some(argument) = parser.next().map_err(|_| ParseError::InvalidArguments)? {
            match argument {
                Short('a') => command.all = true,
                Short('t') => {
                    command.target = Some(
                        parser
                            .value()
                            .map_err(|_| ParseError::MissingValue('t'))?
                            .into_string()
                            .map_err(|_| ParseError::InvalidArguments)?,
                    );
                }
                Long(_) => {
                    let _ = parser.optional_value();
                }
                // Preserve the old leniency for options this slice does not own.
                Short(_) | Value(_) => {}
            }
        }

        Ok(command)
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn effective_target(&self, transport_target: Option<&str>) -> Option<String> {
        let Some(command_target) = self.target() else {
            return transport_target.map(str::to_string);
        };
        let Some(transport_target) = transport_target else {
            return Some(command_target.to_string());
        };
        let parsed_command_target = crate::cli::parse_target(command_target);
        // An inline target that names no window keeps the transport window.
        if parsed_command_target.window.is_none()
            && parsed_command_target.window_name.is_none()
        {
            return Some(transport_target.to_string());
        }
        if has_explicit_session(command_target) {
            return Some(command_target.to_string());
        }
        let Some(session) = crate::cli::parse_target(transport_target).session else {
            return Some(command_target.to_string());
        };
        let command_target = crate::cli::strip_exact_match_prefix(command_target);
        if command_target.starts_with(':') {
            Some(format!("{session}{command_target}"))
        } else {
            Some(format!("{session}:{command_target}"))
        }
    }

    /// The target travels in the protocol's `TARGET` line; other flags stay in
    /// the command body.
    pub fn to_wire_command(&self) -> String {
        let mut line = String::from("kill-window");
        if self.all {
            line.push_str(" -a");
        }
        line.push('\n');
        line
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    MissingValue(char),
    InvalidArguments,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(option) => write!(formatter, "-{option} expects an argument"),
            Self::InvalidArguments => write!(formatter, "kill-window: invalid arguments"),
        }
    }
}

impl std::error::Error for ParseError {}

fn has_explicit_session(target: &str) -> bool {
    let target = crate::cli::strip_exact_match_prefix(target);
    // parse_target reads an unqualified token as a session name; this asks
    // whether the user wrote a session qualifier.
    if matches!(target.as_bytes().first(), Some(b':' | b'@' | b'%')) {
        return false;
    }
    if let Some((session, _)) = target.split_once(':') {
        return !session.is_empty();
    }
    if target.starts_with('.') {
        return false;
    }
    if let Some((session, pane)) = target.rsplit_once('.') {
        if pane.parse::<usize>().is_ok() {
            return !session.is_empty();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<KillWindowCommand, ParseError> {
        KillWindowCommand::parse("kill-window", args.iter().copied()).unwrap()
    }

    #[test]
    fn aliases_share_the_parser() {
        for command in ["kill-window", "killw"] {
            assert_eq!(
                KillWindowCommand::parse(command, ["-t"]).unwrap(),
                Err(ParseError::MissingValue('t'))
            );
        }
        assert!(KillWindowCommand::parse("kill-session", ["-t"]).is_none());
    }

    #[test]
    fn accepts_separate_attached_clustered_and_equals_target_forms() {
        for (args, expected) in [
            (&["-t", "@42"][..], "@42"),
            (&["-t@42"][..], "@42"),
            (&["-at", "@42"][..], "@42"),
            (&["-t=@42"][..], "@42"),
        ] {
            assert_eq!(parse(args).unwrap().target(), Some(expected));
        }
    }

    #[test]
    fn last_target_wins() {
        let command = parse(&["-t", ":1", "-t:2"]).unwrap();
        assert_eq!(command.target(), Some(":2"));
    }

    #[test]
    fn option_value_may_begin_with_a_dash() {
        let command = parse(&["-t", "-a"]).unwrap();
        assert_eq!(command.target(), Some("-a"));
    }

    #[test]
    fn double_dash_ends_option_parsing() {
        let command = parse(&["--", "-t", "@42"]).unwrap();
        assert_eq!(command.target(), None);
    }

    #[test]
    fn explicit_target_overrides_transport_target() {
        let inherited = parse(&[]).unwrap();
        let explicit = parse(&["-t", ":2"]).unwrap();

        assert_eq!(
            inherited.effective_target(Some("work:1.%3")).as_deref(),
            Some("work:1.%3")
        );
        assert_eq!(
            explicit.effective_target(Some("work:1.%3")).as_deref(),
            Some("work:2")
        );
    }

    #[test]
    fn explicit_session_replaces_transport_session() {
        for target in ["other:2", "$0:2", ".other:2"] {
            let command = parse(&["-t", target]).unwrap();
            assert_eq!(
                command.effective_target(Some("work:1")).as_deref(),
                Some(target)
            );
        }
    }

    #[test]
    fn bare_and_pane_only_targets_do_not_discard_the_transport_window() {
        for target in ["2", "logs", "%3", ".1", "work.2", "other:%3", "$0"] {
            let command = parse(&["-t", target]).unwrap();
            assert_eq!(
                command.effective_target(Some("work:1")).as_deref(),
                Some("work:1")
            );
        }
    }

    #[test]
    fn window_id_with_pane_inherits_the_transport_session() {
        let command = parse(&["-t", "@2.0"]).unwrap();
        assert_eq!(
            command.effective_target(Some("work:1")).as_deref(),
            Some("work:@2.0")
        );
    }

    #[test]
    fn wire_command_preserves_non_transport_flags() {
        let command = parse(&["-a", "-t", ":Build Logs"]).unwrap();
        assert_eq!(command.to_wire_command(), "kill-window -a\n");
    }

    #[test]
    fn unknown_options_are_ignored() {
        for args in [&["--future=value"][..], &["-x=value"][..]] {
            assert_eq!(parse(args).unwrap(), KillWindowCommand::default());
        }
    }
}
