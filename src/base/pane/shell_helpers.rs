use super::*;

pub fn detect_shell() -> CommandBuilder {
    build_command(None, false, false)
}

/// Apply user-defined environment variables (from set-environment -g) to a CommandBuilder.
/// This ensures variables set via config or runtime `set-environment` are explicitly
/// passed to every child pane, in addition to process inheritance.
pub fn apply_user_environment(builder: &mut CommandBuilder, environment: &std::collections::HashMap<String, String>) {
    for (key, value) in environment {
        builder.env(key, value);
    }
}
