// Tab-completion for the ':' command prompt.
//
// Unlike test_issue345_command_prompt_utf8.rs — which had to re-implement the
// prompt's editor because it lives inline in client.rs's event loop — these
// call the real functions in src/completion.rs.

use crate::completion::{
    apply, candidates_for, classify, complete, longest_common_prefix, word_at_cursor,
    CompletionContext, CompletionOutcome, WordSpan,
};

// ── word_at_cursor ──────────────────────────────────────────────────────────

#[test]
fn word_span_at_start_of_buffer() {
    assert_eq!(word_at_cursor("", 0), WordSpan { start: 0, end: 0 });
}

#[test]
fn word_span_covers_whole_first_token() {
    // Cursor mid-word still spans the entire word.
    assert_eq!(word_at_cursor("new-window", 3), WordSpan { start: 0, end: 10 });
    assert_eq!(word_at_cursor("new-window", 10), WordSpan { start: 0, end: 10 });
}

#[test]
fn word_span_is_empty_on_trailing_space() {
    let s = "new-window ";
    assert_eq!(word_at_cursor(s, s.len()), WordSpan { start: 11, end: 11 });
}

#[test]
fn word_span_finds_later_argument() {
    let s = "new-window -n foo";
    // Cursor inside "foo".
    assert_eq!(word_at_cursor(s, 15), WordSpan { start: 14, end: 17 });
}

#[test]
fn word_span_handles_multibyte_without_splitting() {
    let s = "中文 ne";
    let span = word_at_cursor(s, s.len());
    // "中文" is 6 bytes + 1 space, so "ne" starts at 7.
    assert_eq!(span, WordSpan { start: 7, end: 9 });
    assert_eq!(&s[span.start..span.end], "ne");
}

// ── classify ────────────────────────────────────────────────────────────────

#[test]
fn first_token_is_a_command_name() {
    let s = "new";
    assert_eq!(classify(s, &word_at_cursor(s, 3)), CompletionContext::CommandName);
}

#[test]
fn leading_whitespace_still_yields_command_name() {
    let s = "   new";
    assert_eq!(classify(s, &word_at_cursor(s, 6)), CompletionContext::CommandName);
}

#[test]
fn later_token_is_unsupported_in_v1() {
    let s = "set -g sta";
    assert_eq!(classify(s, &word_at_cursor(s, s.len())), CompletionContext::Unsupported);
}

#[test]
fn unsupported_context_yields_no_candidates() {
    assert!(candidates_for(&CompletionContext::Unsupported).is_empty());
}

// ── longest_common_prefix ───────────────────────────────────────────────────

#[test]
fn lcp_of_empty_is_empty() {
    assert_eq!(longest_common_prefix(&[]), "");
}

#[test]
fn lcp_of_one_item_is_that_item() {
    assert_eq!(longest_common_prefix(&["kill-server".to_string()]), "kill-server");
}

#[test]
fn lcp_finds_shared_prefix() {
    let items = vec!["ab".to_string(), "ac".to_string()];
    assert_eq!(longest_common_prefix(&items), "a");
}

#[test]
fn lcp_of_disjoint_items_is_empty() {
    let items = vec!["kill-pane".to_string(), "new-window".to_string()];
    assert_eq!(longest_common_prefix(&items), "");
}

#[test]
fn lcp_lands_on_char_boundary() {
    // Shared "中" then diverging — the result must not split the 3-byte char.
    let items = vec!["中文".to_string(), "中间".to_string()];
    let lcp = longest_common_prefix(&items);
    assert_eq!(lcp, "中");
    assert_eq!(lcp.len(), 3);
}

// ── complete ────────────────────────────────────────────────────────────────

#[test]
fn unique_match_completes_fully_with_trailing_space() {
    let (_, outcome) = complete("kill-ser", 8);
    assert_eq!(outcome, CompletionOutcome::Unique { replacement: "kill-server ".to_string() });
}

#[test]
fn ambiguous_prefix_extends_to_common_prefix_first() {
    // "disp" matches only the display-* family, so the first Tab can extend to
    // "display" without picking for the user.
    let (_, outcome) = complete("disp", 4);
    match outcome {
        CompletionOutcome::Extended { replacement } => {
            assert_eq!(replacement, "display");
        }
        other => panic!("expected Extended, got {:?}", other),
    }
}

#[test]
fn prefix_shared_by_unrelated_families_opens_the_list_immediately() {
    // "ne" spans both new-* and next-*, so there is nothing to extend and the
    // first Tab must go straight to the list rather than guessing.
    match complete("ne", 2).1 {
        CompletionOutcome::Ambiguous { matches } => {
            assert!(matches.iter().any(|m| m == "new-window"));
            assert!(matches.iter().any(|m| m == "next-window"));
        }
        other => panic!("expected Ambiguous, got {:?}", other),
    }
}

#[test]
fn second_tab_on_common_prefix_opens_the_list() {
    // "new" IS already the common prefix, so there is no progress to make.
    let (_, outcome) = complete("new", 3);
    match outcome {
        CompletionOutcome::Ambiguous { matches } => {
            for expected in ["new-session", "new-window", "new", "neww"] {
                assert!(
                    matches.iter().any(|m| m == expected),
                    "expected {} among {:?}",
                    expected,
                    matches
                );
            }
        }
        other => panic!("expected Ambiguous, got {:?}", other),
    }
}

#[test]
fn aliases_are_completable() {
    // "splitw" is only reachable as an alias of split-window.
    let (_, outcome) = complete("splitw", 6);
    assert_eq!(outcome, CompletionOutcome::Unique { replacement: "splitw ".to_string() });
}

#[test]
fn no_match_yields_nomatch() {
    assert_eq!(complete("zzz", 3).1, CompletionOutcome::NoMatch);
}

#[test]
fn empty_buffer_offers_the_whole_catalog() {
    match complete("", 0).1 {
        CompletionOutcome::Ambiguous { matches } => {
            assert_eq!(matches, crate::server::helpers::command_name_candidates());
        }
        other => panic!("expected Ambiguous, got {:?}", other),
    }
}

#[test]
fn arguments_are_not_completed_in_v1() {
    // Guards the seam: when argument completion lands, this test changes
    // deliberately rather than silently.
    assert_eq!(complete("set -g sta", 10).1, CompletionOutcome::NoMatch);
}

// ── apply ───────────────────────────────────────────────────────────────────

#[test]
fn apply_splices_and_moves_cursor_past_replacement() {
    let span = WordSpan { start: 0, end: 2 };
    let (buf, cursor) = apply("ne", &span, "new-window ");
    assert_eq!(buf, "new-window ");
    assert_eq!(cursor, 11);
}

#[test]
fn apply_preserves_text_after_the_span() {
    let s = "ne -n foo";
    let span = word_at_cursor(s, 2);
    let (buf, cursor) = apply(s, &span, "new-window");
    assert_eq!(buf, "new-window -n foo");
    assert_eq!(cursor, 10);
}

#[test]
fn apply_keeps_cursor_on_a_char_boundary_with_multibyte_prefix() {
    let s = "中文 ne";
    let span = word_at_cursor(s, s.len());
    let (buf, cursor) = apply(s, &span, "new-window ");
    assert_eq!(buf, "中文 new-window ");
    // The panic in issue #345 came from a cursor inside a UTF-8 sequence.
    assert!(buf.is_char_boundary(cursor));
    // And the render path slices here every frame.
    let _ = &buf[..cursor];
}

// ── catalog integrity ───────────────────────────────────────────────────────

#[test]
fn candidates_are_sorted_deduped_and_non_empty() {
    let c = crate::server::helpers::command_name_candidates();
    assert!(!c.is_empty());
    let mut sorted = c.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(c, sorted, "command_name_candidates must be sorted and deduped");
}

#[test]
fn every_candidate_is_a_command_the_parser_accepts() {
    // Completion must never offer a token that config parsing would warn about.
    let app = crate::types::AppState::new("completion_test".to_string());
    for cand in crate::server::helpers::command_name_candidates() {
        assert!(
            crate::config::is_known_command(&app, cand),
            "completion offers {:?} but is_known_command rejects it",
            cand
        );
    }
}

#[test]
fn split_command_entry_handles_both_encodings() {
    use crate::server::helpers::split_command_entry;
    assert_eq!(split_command_entry("command-prompt"), ("command-prompt", None));
    assert_eq!(split_command_entry("split-window (splitw)"), ("split-window", Some("splitw")));
}
