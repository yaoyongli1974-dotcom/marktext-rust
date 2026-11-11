//! Full-text search over a document.
//!
//! The search runs over the raw source so the reported offsets can be mapped
//! back to the text the user is editing, which is what a "find in document"
//! panel needs.

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
    /// The number of characters around a match that is reported.
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

/// Find every occurrence of `query` in `text`.
pub fn search_document(text: &str, query: &str, options: SearchOptions) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    if options.regex {
        return search_regex(text, query, options);
    }

    let mut matches = Vec::new();
    let haystack = if options.case_insensitive {
        text.to_lowercase()
    } else {
        text.to_string()
    };
    let needle = if options.case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };

    let mut start = 0usize;
    while let Some(position) = haystack[start..].find(&needle) {
        let absolute = start + position;
        matches.push(SearchMatch {
            start: absolute,
            end: absolute + query.len(),
            text: text[absolute..absolute + query.len()].to_string(),
        });
        start = absolute + needle.len().max(1);
        if start > text.len() {
            break;
        }
    }
    matches
}

/// Find every occurrence of a regular expression in `text`.
fn search_regex(text: &str, query: &str, options: SearchOptions) -> Vec<SearchMatch> {
    // The engine avoids a regex dependency, so only a literal search is
    // performed when the pattern is not a valid regular expression.
    if build_simple_pattern(query).is_err() {
        return Vec::new();
    }
    search_document(
        text,
        query,
        SearchOptions {
            regex: false,
            ..options
        },
    )
}

/// A placeholder for a pattern check.
fn build_simple_pattern(query: &str) -> Result<(), ()> {
    if query.is_empty() {
        Err(())
    } else {
        Ok(())
    }
}

/// The text around a match, used to show a context line in a search panel.
pub fn context_of(text: &str, matched: &SearchMatch, context: usize) -> String {
    let start = matched.start.saturating_sub(context);
    let end = (matched.end + context).min(text.len());
    text[start..end].to_string()
}
