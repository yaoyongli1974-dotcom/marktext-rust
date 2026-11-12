//! Tests for the full-text search used by the find panel and the CLI.
//!
//! The reported offsets are byte offsets into the text the user is editing,
//! so they must stay on character boundaries even when case folding changes
//! the byte length of the text (for example `İ` lowercases to `i` plus a
//! combining dot).

use marktext_core::search::context_of;
use marktext_core::{search_document, SearchError, SearchOptions};

fn literal(case_insensitive: bool) -> SearchOptions {
    SearchOptions {
        case_insensitive,
        regex: false,
        context: 0,
    }
}

fn regex(case_insensitive: bool) -> SearchOptions {
    SearchOptions {
        case_insensitive,
        regex: true,
        context: 0,
    }
}

#[test]
fn finds_every_literal_occurrence() {
    let text = "foo bar foo baz foo";
    let matches = search_document(text, "foo", literal(false)).unwrap();
    let starts: Vec<usize> = matches.iter().map(|m| m.start).collect();
    assert_eq!(starts, vec![0, 8, 16]);
    assert!(matches.iter().all(|m| m.text == "foo"));
}

#[test]
fn case_insensitive_search_reports_the_original_spelling() {
    let text = "Foo fOO foo";
    let matches = search_document(text, "foo", literal(true)).unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].text, "Foo");
    assert_eq!(matches[1].text, "fOO");
    assert_eq!(matches[2].text, "foo");
}

#[test]
fn a_case_folding_that_changes_the_byte_length_does_not_panic() {
    // `İ` (U+0130) is two bytes and lowercases to `i` plus U+0307, which is
    // three bytes. The old implementation lowercased the haystack and then
    // sliced the original text with an offset taken from the lowercase copy,
    // so this input panicked ("byte index is not a char boundary").
    let text = "İstanbul";
    let matches = search_document(text, "i", literal(true)).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].start, 0);
    // The whole `İ` is highlighted, because a match cannot end in the middle
    // of a character.
    assert_eq!(matches[0].end, "İ".len());
    assert_eq!(matches[0].text, "İ");

    // The same query without case folding is not a match: `İ` is not `i`.
    assert!(search_document(text, "i", literal(false))
        .unwrap()
        .is_empty());
}

#[test]
fn overlapping_bytes_are_not_matched_twice() {
    let matches = search_document("aaaa", "aa", literal(false)).unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].start, 0);
    assert_eq!(matches[1].start, 2);
}

#[test]
fn an_empty_query_has_no_matches() {
    assert!(search_document("anything", "", literal(true))
        .unwrap()
        .is_empty());
}

#[test]
fn the_context_never_slices_through_a_character() {
    // The match starts after a two-byte character; one byte of context would
    // land inside it, which used to panic.
    let text = "é中";
    let matches = search_document(text, "中", literal(false)).unwrap();
    let context = context_of(text, &matches[0], 1);
    assert_eq!(context, text);
}

#[test]
fn a_regular_expression_is_really_a_regular_expression() {
    let text = "alpha\nbeta\ngamma";
    let matches = search_document(text, r"^[a-z]+$", regex(true)).unwrap();
    let texts: Vec<&str> = matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn a_regular_expression_can_be_case_insensitive() {
    let matches = search_document("Alpha BETA", "alpha|beta", regex(true)).unwrap();
    assert_eq!(matches.len(), 2);
}

#[test]
fn an_invalid_regular_expression_is_reported() {
    let error = search_document("text", "(", regex(true)).unwrap_err();
    assert!(matches!(error, SearchError::InvalidRegex { .. }));
    assert!(error.to_string().contains("正则"));
}

#[test]
fn a_regular_expression_can_match_the_empty_string() {
    let matches = search_document("ab", "^", regex(false)).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].start, 0);
    assert_eq!(matches[0].end, 0);
}
