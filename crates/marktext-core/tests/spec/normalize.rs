//! Port of MarkText's `packages/muya/test/spec/runner.ts` HTML normaliser.
//!
//! The normaliser exists so that trivial output differences — whitespace,
//! self-closing style, attribute order — do not masquerade as spec
//! compliance failures.  The rules are a 1:1 translation of the JavaScript
//! original so that a conformance number reported here means exactly the
//! same thing it means upstream.
//!
//! * Self-closing void elements are normalised to the `<br />` style.
//! * Attributes inside a tag are sorted alphabetically.
//! * Whitespace between two adjacent tag boundaries collapses to nothing.
//! * Whitespace directly after a void self-closing tag is dropped.
//! * Leading and trailing newlines are stripped.
//!
//! Content *inside* tags is never touched: `>foo\n<` does not collapse
//! because `foo` is not whitespace.  That is what keeps the trailing
//! newline of fenced code blocks intact.

use std::sync::OnceLock;

use regex::Regex;

/// The void elements that are normalised to the `<tag />` style.
const VOID_TAGS: [&str; 5] = ["br", "hr", "img", "wbr", "input"];

/// The pattern that matches one void element.
fn void_pattern(tag: &str) -> Regex {
    Regex::new(&format!(r"(?i)<{tag}(?:\s+[^>]*?)?\s*/?\s*>")).expect("valid void pattern")
}

/// The pattern that matches a tag whose attributes can be re-sorted.
fn attribute_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)<([a-z][\w-]*)[ \t\n\r]+([a-z_:][^>]*?)(\/?)>")
            .expect("valid attribute pattern")
    })
}

/// The pattern that matches a single `name` or `name="value"` pair.
fn attribute_pair_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)([a-z_:][\w:.-]*)(?:\s*=\s*("[^"]*"|'[^']*'|[^\s"'>`]+))?"#)
            .expect("valid attribute pair pattern")
    })
}

/// The pattern that matches whitespace between two adjacent tags.
fn inter_tag_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r">[ \t\n\r]+<").expect("valid inter-tag pattern"))
}

/// The pattern that matches whitespace directly after a void element.
fn after_void_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)(<(?:br|hr|wbr|img|input)\s+\/>)[ \t\n\r]+")
            .expect("valid after-void pattern")
    })
}

/// Canonicalise an HTML fragment so that trivially-different but
/// semantically-equal outputs compare equal.
pub fn normalize_html(html: &str) -> String {
    let mut out = html.to_string();

    // 1. Normalise the self-closing void elements.
    for tag in VOID_TAGS {
        out = void_pattern(tag)
            .replace_all(&out, |caps: &regex::Captures| {
                let attrs = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                if attrs.is_empty() {
                    format!("<{tag} />")
                } else {
                    format!("<{tag} {attrs} />")
                }
            })
            .into_owned();
    }

    // 2. Sort the attributes of every tag that carries any.
    out = attribute_pattern()
        .replace_all(&out, |caps: &regex::Captures| {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let attrs = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let self_close = caps.get(3).map(|m| m.as_str()).unwrap_or("");

            let mut pairs: Vec<(String, String)> = Vec::new();
            for pair in attribute_pair_pattern().captures_iter(attrs) {
                let key = pair.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                let value = pair.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
                pairs.push((key, value));
            }
            pairs.sort_by(|a, b| a.0.cmp(&b.0));

            let rebuilt = pairs
                .iter()
                .map(|(key, value)| {
                    if value.is_empty() {
                        key.clone()
                    } else {
                        format!("{key}={value}")
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let slash = if self_close.is_empty() { "" } else { " /" };

            if rebuilt.is_empty() {
                format!("<{name}{slash}>")
            } else {
                format!("<{name} {rebuilt}{slash}>")
            }
        })
        .into_owned();

    // 3. Collapse the whitespace that sits between two adjacent tags.
    out = inter_tag_pattern().replace_all(&out, "><").into_owned();

    // 4. Drop the whitespace that directly follows a void element.
    out = after_void_pattern().replace_all(&out, "${1}").into_owned();

    // 5. Strip the leading and trailing newlines.
    out = out.trim_start_matches('\n').to_string();
    out = out.trim_end_matches('\n').to_string();

    out
}

/// Compare a rendered HTML fragment against the expected HTML of a spec
/// example.  Equality is decided after normalisation.
pub fn compare_html(actual: &str, expected: &str) -> bool {
    normalize_html(actual) == normalize_html(expected)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_void_elements() {
        assert_eq!(normalize_html("<p>a<br>b</p>"), "<p>a<br />b</p>");
        assert_eq!(normalize_html("<p>a<br/>b</p>"), "<p>a<br />b</p>");
        // The upstream normaliser rewrites the tag name to lower case.
        assert_eq!(normalize_html("<p>a<BR />b</p>"), "<p>a<br />b</p>");
    }

    #[test]
    fn sorts_attributes() {
        assert_eq!(
            normalize_html(r#"<a title="t" href="u">x</a>"#),
            r#"<a href="u" title="t">x</a>"#
        );
    }

    #[test]
    fn keeps_content_between_tags() {
        // `>foo\n<` carries content and must not collapse.
        assert_eq!(
            normalize_html("<pre><code>foo\n</code></pre>"),
            "<pre><code>foo\n</code></pre>"
        );
    }

    #[test]
    fn strips_surrounding_newlines() {
        assert_eq!(normalize_html("\n<p>x</p>\n"), "<p>x</p>");
    }
}
