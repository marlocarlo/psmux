//! Tab-completion for the `:` command prompt.
//!
//! The prompt itself lives in `client.rs` as flat key-handler arms inside the
//! event loop, which is not reachable from a test.  Keeping the logic here
//! means it can be unit-tested directly rather than mirrored — see the header
//! of `tests-rs/test_issue345_command_prompt_utf8.rs` for what mirroring the
//! prompt's editor cost us the last time.
//!
//! v1 completes the first token only.  [`CompletionContext`] is the seam for
//! later work: completing option names after `set -g `, or targets after
//! `-t `, means adding a variant plus an arm in [`candidates_for`].
//! [`complete`], [`apply`] and the client's key handling stay unchanged.

/// What the word under the cursor should be completed against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// The first token of the command line: complete against the command catalog.
    CommandName,
    /// Anything psmux cannot complete yet — arguments, flags, option names,
    /// targets.  Yields no candidates, which makes Tab a swallowed no-op.
    Unsupported,
}

/// Byte range of the word under the cursor.  Both ends are always on UTF-8
/// char boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
}

/// The result of a completion attempt.  The `Extended`/`Ambiguous` split is
/// what produces tmux's two-stage Tab: the first press extends as far as the
/// input is unambiguous, and only a press that makes no progress opens the
/// candidate list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionOutcome {
    /// Nothing matches — swallow the key and change nothing.
    NoMatch,
    /// Exactly one match.  Insert it plus a trailing space; completion ends.
    Unique { replacement: String },
    /// The common prefix is strictly longer than what was typed.  Extend the
    /// word, but do not open the list.
    Extended { replacement: String },
    /// The common prefix adds nothing and several candidates remain.  Open the
    /// candidate list.
    Ambiguous { matches: Vec<String> },
}

/// Byte range of the whitespace-delimited word containing `cursor`.
///
/// Returns an empty span when the cursor sits on whitespace.
pub fn word_at_cursor(buf: &str, cursor: usize) -> WordSpan {
    let bytes = buf.as_bytes();
    let cursor = cursor.min(buf.len());
    let is_sep = |b: u8| b == b' ' || b == b'\t';
    // Scanning raw bytes for ASCII whitespace is char-boundary-safe: UTF-8
    // continuation bytes are all >= 0x80, so they can never match.
    let mut start = cursor;
    while start > 0 && !is_sep(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = cursor;
    while end < bytes.len() && !is_sep(bytes[end]) {
        end += 1;
    }
    WordSpan { start, end }
}

/// Decide what `span` should be completed against.
///
/// A word is a command name when nothing but whitespace precedes it.  Note
/// that `;`-chained command lines (`new-window ; spl`) currently classify the
/// second command as [`CompletionContext::Unsupported`]; splitting on chains
/// via `crate::config::split_chained_commands_pub` is deliberately left for
/// the same follow-up that adds argument completion.
pub fn classify(buf: &str, span: &WordSpan) -> CompletionContext {
    if buf[..span.start].trim().is_empty() {
        CompletionContext::CommandName
    } else {
        CompletionContext::Unsupported
    }
}

/// All candidates available in a given context, unfiltered.
pub fn candidates_for(ctx: &CompletionContext) -> Vec<String> {
    match ctx {
        CompletionContext::CommandName => crate::server::helpers::command_name_candidates()
            .into_iter()
            .map(String::from)
            .collect(),
        CompletionContext::Unsupported => Vec::new(),
    }
}

/// Longest prefix shared by every item, truncated to a char boundary.
pub fn longest_common_prefix(items: &[String]) -> String {
    let mut iter = items.iter();
    let first = match iter.next() {
        Some(f) => f,
        None => return String::new(),
    };
    let mut prefix_len = first.len();
    for item in iter {
        // Compare by char, not byte, so the result is always a valid boundary.
        // Command names are ASCII today, but this will later be fed session and
        // window names.
        let common: usize = first
            .chars()
            .zip(item.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum();
        prefix_len = prefix_len.min(common);
    }
    first[..prefix_len].to_string()
}

/// Attempt to complete the word under the cursor.
///
/// Returns the span that any replacement applies to, alongside the outcome.
pub fn complete(buf: &str, cursor: usize) -> (WordSpan, CompletionOutcome) {
    let span = word_at_cursor(buf, cursor);
    let ctx = classify(buf, &span);
    let word = &buf[span.start..span.end];
    // Case-sensitive prefix match, matching tmux.
    let matches: Vec<String> = candidates_for(&ctx)
        .into_iter()
        .filter(|c| c.starts_with(word))
        .collect();
    let outcome = match matches.len() {
        0 => CompletionOutcome::NoMatch,
        1 => CompletionOutcome::Unique { replacement: format!("{} ", matches[0]) },
        _ => {
            let lcp = longest_common_prefix(&matches);
            if lcp.len() > word.len() {
                CompletionOutcome::Extended { replacement: lcp }
            } else {
                CompletionOutcome::Ambiguous { matches }
            }
        }
    };
    (span, outcome)
}

/// Splice `replacement` over `span`, returning the new buffer and the new byte
/// cursor (immediately after the replacement).
///
/// The returned cursor is always on a char boundary, which the prompt's
/// rendering and editing arms rely on (issue #345).
pub fn apply(buf: &str, span: &WordSpan, replacement: &str) -> (String, usize) {
    let mut out = String::with_capacity(buf.len() + replacement.len());
    out.push_str(&buf[..span.start]);
    out.push_str(replacement);
    let cursor = out.len();
    out.push_str(&buf[span.end..]);
    (out, cursor)
}

#[cfg(test)]
#[path = "../tests-rs/test_command_completion.rs"]
mod tests_command_completion;
