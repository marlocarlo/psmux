use super::*;

/// Parse a command line string, respecting quoted arguments
pub fn parse_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_double_quotes = false;
    let mut in_single_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if in_single_quotes {
            // Inside single quotes: everything is literal (no escape processing)
            if c == '\'' {
                in_single_quotes = false;
            } else {
                current.push(c);
            }
        } else if c == '\\' && in_double_quotes {
            // Inside double quotes, recognise two escape sequences:
            //   \"  → literal double-quote
            //   \\  → literal backslash
            // All other backslashes are kept literal because psmux is a
            // Windows-native tool where backslash is the normal path
            // separator (e.g. "C:\Program Files\Git\bin\bash.exe").
            if i + 1 < chars.len() && chars[i + 1] == '"' {
                current.push('"');
                i += 1; // skip the quote
            } else if i + 1 < chars.len() && chars[i + 1] == '\\' {
                current.push('\\');
                i += 1; // skip the second backslash
            } else {
                current.push(c); // literal backslash
            }
        } else if c == '"' {
            in_double_quotes = !in_double_quotes;
        } else if c == '\'' && !in_double_quotes {
            in_single_quotes = true;
        } else if c.is_whitespace() && !in_double_quotes {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push(c);
        }
        i += 1;
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}
