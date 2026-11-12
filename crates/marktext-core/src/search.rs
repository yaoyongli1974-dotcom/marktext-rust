//! Full-text search over a document.
//!
//! The search runs over the raw source so the reported offsets can be mapped
//! back to the text the user is editing, which is what a "find in document"
//! panel needs. Offsets are *byte* offsets and always lie on a character
//! boundary, even when case folding changes the byte length of the text.
//!
//! Regex searches use the [`regex`] crate (RE2 syntax: no backreferences and
//! no look-around) and are anchored per line, so `^` and `$` match the
//! beginning and end of every line, which is what a find panel is expected to
//! do.

use std::fmt;

use regex::RegexBuilder;

/// One hit of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// The byte offset of the match in the searched text.
    pub start: usize,
    /// The byte offset just past the match.
    pub end: usize,
    /// The matched text.
    pub text: String,
}

/// Options that control a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    /// Whether the query is matched case-insensitively.
    pub case_insensitive: bool,
    /// Whether the query is treated as a regular expression.
    pub regex: bool,
    /// The number of characters around a match that [`context_of`] reports.
    pub context: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            case_insensitive: true,
            regex: false,
            context: 0,
        }
    }
}

/// Why a search could not be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// The query was marked as a regular expression but does not compile.
    InvalidRegex {
        /// The diagnostics of the regex parser.
        message: String,
    },
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::InvalidRegex { message } => {
                write!(formatter, "正则表达式无效：{message}")
            }
        }
    }
}

impl std::error::Error for SearchError {}

/// Find every occurrence of `query` in `text`.
///
/// An empty query has no matches. A literal search returns non-overlapping
/// matches; a regex search returns exactly what the regex engine reports,
/// including zero-length matches such as the ones produced by `^`.
pub fn search_document(
    text: &str,
    query: &str,
    options: SearchOptions,
) -> Result<Vec<SearchMatch>, SearchError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }

    if options.regex {
        return search_regex(text, query, options);
    }

    Ok(search_literal(text, query, options.case_insensitive))
}

/// Find every occurrence of a literal string.
fn search_literal(text: &str, query: &str, case_insensitive: bool) -> Vec<SearchMatch> {
    if !case_insensitive {
        return text
            .match_indices(query)
            .map(|(start, matched)| SearchMatch {
                start,
                end: start + matched.len(),
                text: matched.to_string(),
            })
            .collect();
    }

    // ASCII documents are the common case; matching them byte by byte avoids
    // building a lowercase copy of the whole document.
    if text.is_ascii() && query.is_ascii() {
        return search_ascii_case_insensitive(text, query);
    }

    search_unicode_case_insensitive(text, query)
}

/// The ASCII fast path of a case-insensitive literal search.
fn search_ascii_case_insensitive(text: &str, query: &str) -> Vec<SearchMatch> {
    let haystack = text.as_bytes();
    let needle = query.as_bytes();
    let mut matches = Vec::new();
    let mut start = 0usize;

    while start + needle.len() <= haystack.len() {
        if haystack[start..start + needle.len()].eq_ignore_ascii_case(needle) {
            matches.push(SearchMatch {
                start,
                end: start + needle.len(),
                text: text[start..start + needle.len()].to_string(),
            });
            start += needle.len();
        } else {
            start += 1;
        }
    }

    matches
}

/// The character range one original character occupies in the folded text.
struct FoldedChar {
    /// Where the lowercase form of the character ends in the folded text.
    folded_end: usize,
    /// Where the character starts in the original text.
    original_start: usize,
    /// Where the character ends in the original text.
    original_end: usize,
}

/// Lowercase `text` and remember where every original character went.
///
/// A character can lowercase to more than one character (`İ` becomes `i` and
/// a combining dot), so the folded text cannot be indexed with offsets taken
/// from the original text or the other way around. The map makes it possible
/// to report a match that lies inside such an expansion as the whole original
/// character.
fn fold(text: &str) -> (String, Vec<FoldedChar>) {
    let mut folded = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.chars().count());

    for (original_start, character) in text.char_indices() {
        for lowercase in character.to_lowercase() {
            folded.push(lowercase);
        }
        map.push(FoldedChar {
            folded_end: folded.len(),
            original_start,
            original_end: original_start + character.len_utf8(),
        });
    }

    (folded, map)
}

/// A case-insensitive literal search that survives case folding.
fn search_unicode_case_insensitive(text: &str, query: &str) -> Vec<SearchMatch> {
    let (folded, map) = fold(text);
    let needle: String = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut from = 0usize;

    while from < folded.len() {
        let Some(relative) = folded[from..].find(&needle) else {
            break;
        };
        let folded_start = from + relative;
        let folded_end = folded_start + needle.len();

        // The first character whose lowercase form ends after the match
        // starts, and the first one that ends at or after it ends.
        let first = map.partition_point(|entry| entry.folded_end <= folded_start);
        let last = map
            .partition_point(|entry| entry.folded_end < folded_end)
            .min(map.len().saturating_sub(1));

        if let (Some(first), Some(last)) = (map.get(first), map.get(last)) {
            matches.push(SearchMatch {
                start: first.original_start,
                end: last.original_end,
                text: text[first.original_start..last.original_end].to_string(),
            });
        }

        // Continue past the match; folding can change the byte length, so the
        // advance is computed in folded coordinates.
        from = folded_end.max(folded_start + 1);
    }

    matches
}

/// Find every occurrence of a regular expression in `text`.
fn search_regex(
    text: &str,
    query: &str,
    options: SearchOptions,
) -> Result<Vec<SearchMatch>, SearchError> {
    let regex = RegexBuilder::new(query)
        .case_insensitive(options.case_insensitive)
        .multi_line(true)
        .build()
        .map_err(|error| SearchError::InvalidRegex {
            message: error.to_string(),
        })?;

    Ok(regex
        .find_iter(text)
        .map(|found| SearchMatch {
            start: found.start(),
            end: found.end(),
            text: found.as_str().to_string(),
        })
        .collect())
}

/// The text around a match, used to show a context line in a search panel.
///
/// The requested number of bytes is widened to the nearest character
/// boundaries, so a multi-byte character around the match is never sliced in
/// half.
pub fn context_of(text: &str, matched: &SearchMatch, context: usize) -> String {
    let mut start = matched.start.saturating_sub(context);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }

    let mut end = (matched.end + context).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }

    text[start..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_search_is_non_overlapping() {
        let matches = search_literal("aaaa", "aa", false);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn the_folded_map_covers_every_character() {
        let (folded, map) = fold("aİé");
        assert_eq!(folded, "ai̇é");
        assert_eq!(map.first().unwrap().original_start, 0);
        assert_eq!(map.last().unwrap().original_end, "aİé".len());
        for window in map.windows(2) {
            assert!(window[0].folded_end <= window[1].folded_end);
        }
    }

    #[test]
    fn an_empty_context_still_returns_the_match() {
        let matched = SearchMatch {
            start: 1,
            end: 3,
            text: "bc".to_string(),
        };
        assert_eq!(context_of("abcd", &matched, 0), "bc");
    }
}
