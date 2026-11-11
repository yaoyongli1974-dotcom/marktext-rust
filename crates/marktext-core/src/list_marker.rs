//! Ordered-list markers as pandoc's `fancy_lists` extension reads and writes
//! them: <https://pandoc.org/MANUAL.html#extension-fancy_lists>
//!
//! Ported from MarkText's `packages/muya/src/utils/listMarker.ts`, which is
//! itself a port of the same pandoc rules.

use crate::document::{ListDelimiter, ListStyle};

/// A parsed list marker such as `IV.`, `(a)` or `3)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMarker {
    pub value: u64,
    pub style: ListStyle,
    pub delimiter: ListDelimiter,
}

/// The numbering scheme of a list, used to decide whether a following marker
/// continues the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListLevel {
    pub style: ListStyle,
    pub delimiter: ListDelimiter,
}

const ROMAN_WEIGHTS: [u64; 13] = [1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
const ROMAN_SYMBOLS: [&str; 13] = [
    "M", "CM", "D", "CD", "C", "XC", "L", "XL", "X", "IX", "V", "IV", "I",
];
/// Indices whose repeated group counts once per letter instead of per unit.
const ROMAN_REPEATED: [usize; 4] = [0, 4, 8, 12];

/// Read a Roman numeral in upper case. Pandoc's grammar also accepts
/// non-canonical forms such as `IIII`.
fn read_roman(upper: &str) -> Option<u64> {
    // The grammar is positional, so walk the symbols left to right and match
    // the longest symbol at each position.
    let bytes = upper.as_bytes();
    let mut idx = 0usize;
    let mut total = 0u64;
    for (weight_index, symbol) in ROMAN_SYMBOLS.iter().enumerate() {
        let sym_bytes = symbol.as_bytes();
        // Each weight may repeat; repeated groups collapse to a single unit
        // except for the weights pandoc counts per letter.
        let repeats = if ROMAN_REPEATED.contains(&weight_index) {
            count_repeats(bytes, &mut idx, sym_bytes)
        } else {
            if bytes[idx..].starts_with(sym_bytes) {
                idx += sym_bytes.len();
                1
            } else {
                0
            }
        };
        if repeats > 0 {
            total += ROMAN_WEIGHTS[weight_index] * repeats;
        }
    }
    if idx != bytes.len() {
        return None;
    }
    if total > 0 {
        Some(total)
    } else {
        None
    }
}

fn count_repeats(bytes: &[u8], idx: &mut usize, symbol: &[u8]) -> u64 {
    let mut count = 0u64;
    while bytes[*idx..].starts_with(symbol) {
        *idx += symbol.len();
        count += 1;
    }
    count
}

/// Write a value as a Roman numeral.
fn write_roman(mut value: u64) -> String {
    let mut result = String::new();
    for (index, weight) in ROMAN_WEIGHTS.iter().enumerate() {
        while value >= *weight {
            result.push_str(ROMAN_SYMBOLS[index]);
            value -= *weight;
        }
    }
    result
}

/// Pandoc's own writer wraps past `z`; any single letter continues an alpha
/// list, so the wrapped text still reads back as the same list.
fn write_alpha(value: u64) -> String {
    let offset = ((value - 1) % 26) as u8;
    ((b'a' + offset) as char).to_string()
}

fn read_decimal(body: &str) -> Option<u64> {
    if body.is_empty() || body.len() > 9 || !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    body.parse().ok()
}

/// Read a marker's body according to its style.
fn read_style(style: ListStyle, body: &str) -> Option<u64> {
    match style {
        ListStyle::Default | ListStyle::Decimal => read_decimal(body),
        ListStyle::LowerAlpha => {
            let bytes = body.as_bytes();
            if bytes.len() == 1 && bytes[0].is_ascii_lowercase() {
                Some((bytes[0] - b'a' + 1) as u64)
            } else {
                None
            }
        }
        ListStyle::UpperAlpha => {
            let bytes = body.as_bytes();
            if bytes.len() == 1 && bytes[0].is_ascii_uppercase() {
                Some((bytes[0] - b'A' + 1) as u64)
            } else {
                None
            }
        }
        ListStyle::LowerRoman => {
            if !body.bytes().all(|b| b.is_ascii_lowercase()) || body.is_empty() {
                None
            } else {
                read_roman(&body.to_ascii_uppercase())
            }
        }
        ListStyle::UpperRoman => {
            if !body.bytes().all(|b| b.is_ascii_uppercase()) || body.is_empty() {
                None
            } else {
                read_roman(body)
            }
        }
    }
}

/// Write a value according to its style.
fn write_style(style: ListStyle, value: u64) -> String {
    match style {
        ListStyle::Default => "#".to_string(),
        ListStyle::Decimal => value.to_string(),
        ListStyle::LowerAlpha => write_alpha(value).to_string(),
        ListStyle::UpperAlpha => write_alpha(value).to_ascii_uppercase(),
        ListStyle::LowerRoman => write_roman(value).to_ascii_lowercase(),
        ListStyle::UpperRoman => write_roman(value),
    }
}

/// A list's first marker picks its style: `i`/`I` is roman one, any other
/// single letter is alphabetic, and longer letter runs are roman.
fn first_marker_style(body: &str) -> ListStyle {
    if body == "#" {
        return ListStyle::Default;
    }
    if body.starts_with(|c: char| c.is_ascii_digit()) {
        return ListStyle::Decimal;
    }
    let upper = body == body.to_ascii_uppercase();
    if body.len() > 1 || body == "i" || body == "I" {
        return if upper {
            ListStyle::UpperRoman
        } else {
            ListStyle::LowerRoman
        };
    }
    if upper {
        ListStyle::UpperAlpha
    } else {
        ListStyle::LowerAlpha
    }
}

/// Read a marker such as `IV.`, `(a)` or `3)`.
///
/// Without `continue_style` the text is read as a list's first marker; with
/// it, as a later item of a list in that style (so `v.` continues an `i.` list
/// as five).
pub fn parse_list_marker(text: &str, continue_style: Option<ListStyle>) -> Option<ListMarker> {
    // `^(?:\(([^()]+)\)|([^().]+)([.)]))$`
    let bytes = text.as_bytes();
    let (body, delimiter) = if text.starts_with('(') && text.ends_with(')') && text.len() >= 3 {
        let inner = &text[1..text.len() - 1];
        if inner.contains('(') || inner.contains(')') {
            return None;
        }
        (inner, ListDelimiter::ParenBoth)
    } else {
        let open = text.find(['.', ')'])?;
        let body = &text[..open];
        let close = &text[open..];
        if body.is_empty() || body.contains('.') || body.contains(')') {
            return None;
        }
        let delimiter = match close {
            "." => ListDelimiter::Dot,
            ")" => ListDelimiter::Paren,
            _ => return None,
        };
        // Only a single closing character may follow the body.
        if close.len() != 1 {
            return None;
        }
        (body, delimiter)
    };
    let _ = bytes;

    let style = continue_style.unwrap_or_else(|| first_marker_style(body));
    // Pandoc reads `#` as one in any style, so it continues every list.
    let value = if body == "#" {
        1
    } else {
        read_style(style, body)?
    };

    Some(ListMarker {
        value,
        style,
        delimiter,
    })
}

/// Where a marker sits in its list, which changes how roman numerals are
/// spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerPosition {
    First,
    Continuation,
}

/// Format a marker for output.
///
/// `First` uses additive Roman numerals where V/X/L/C/D/M would otherwise
/// read as letters.
pub fn format_list_marker(marker: &ListMarker, position: MarkerPosition) -> String {
    let mut body = write_style(marker.style, marker.value);
    if position == MarkerPosition::First
        && matches!(marker.style, ListStyle::LowerRoman | ListStyle::UpperRoman)
        && body.len() == 1
        && marker.value > 1
    {
        let additive = write_roman(marker.value / 5).repeat(5);
        body = if marker.style == ListStyle::UpperRoman {
            additive
        } else {
            additive.to_ascii_lowercase()
        };
    }

    match marker.delimiter {
        ListDelimiter::ParenBoth => format!("({body})"),
        ListDelimiter::Dot => format!("{body}."),
        ListDelimiter::Paren => format!("{body})"),
    }
}

/// Pandoc wants two spaces after a marker that could be an initial, so that
/// `B. Russell won` stays a paragraph.
fn is_initial_like_roman(value: u64) -> bool {
    matches!(value, 1 | 5 | 10 | 50 | 100 | 500 | 1000)
}

/// The fewest spaces pandoc accepts after a marker.
///
/// `text` disambiguates a following page number.
pub fn min_spaces_after_marker(marker: &ListMarker, text: &str) -> usize {
    let initial_like = marker.style == ListStyle::UpperAlpha
        || (marker.style == ListStyle::UpperRoman && is_initial_like_roman(marker.value));
    // A single space in `p. 5` denotes a page reference, including after alpha wraps.
    let page_reference = marker.style == ListStyle::LowerAlpha
        && write_alpha(marker.value) == "p"
        && text.starts_with(|c: char| c.is_ascii_digit());

    if marker.delimiter == ListDelimiter::Dot && (initial_like || page_reference) {
        2
    } else {
        1
    }
}

/// Whether `next` continues `list`.
///
/// Continuation is directional: `i.` can continue an alpha list, but `a.` cannot
/// continue a roman list. `source_marker` retains an authored marker when
/// available.
pub fn continues_list(list: &ListLevel, next: &ListMarker, source_marker: Option<&str>) -> bool {
    let fallback = format_list_marker(next, MarkerPosition::First);
    let source_marker = source_marker.unwrap_or(&fallback);

    let continuation = match parse_list_marker(source_marker, Some(list.style)) {
        Some(marker) if marker.delimiter == list.delimiter => marker,
        _ => return false,
    };
    let written_marker =
        parse_list_marker(source_marker, Some(next.style)).unwrap_or_else(|| next.clone());

    min_spaces_after_marker(&written_marker, "") >= min_spaces_after_marker(&continuation, "")
}

/// The list delimiter implied by a raw marker character.
pub fn to_list_delimiter(value: &str) -> ListDelimiter {
    match value {
        "." => ListDelimiter::Dot,
        ")" => ListDelimiter::Paren,
        "()" => ListDelimiter::ParenBoth,
        _ => ListDelimiter::Dot,
    }
}

/// The marker style for each nesting depth of an academic outline.
const OUTLINE_LADDER: [ListLevel; 7] = [
    ListLevel {
        style: ListStyle::UpperRoman,
        delimiter: ListDelimiter::Dot,
    },
    ListLevel {
        style: ListStyle::UpperAlpha,
        delimiter: ListDelimiter::Dot,
    },
    ListLevel {
        style: ListStyle::Decimal,
        delimiter: ListDelimiter::Dot,
    },
    ListLevel {
        style: ListStyle::LowerAlpha,
        delimiter: ListDelimiter::Dot,
    },
    ListLevel {
        style: ListStyle::LowerRoman,
        delimiter: ListDelimiter::Dot,
    },
    ListLevel {
        style: ListStyle::Decimal,
        delimiter: ListDelimiter::ParenBoth,
    },
    ListLevel {
        style: ListStyle::LowerAlpha,
        delimiter: ListDelimiter::ParenBoth,
    },
];

/// The ladder level for a zero-based depth; deeper levels reuse the last one.
pub fn outline_level(depth: usize) -> ListLevel {
    OUTLINE_LADDER[depth.min(OUTLINE_LADDER.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_decimal_markers() {
        let marker = parse_list_marker("3.", None).unwrap();
        assert_eq!(marker.value, 3);
        assert_eq!(marker.style, ListStyle::Decimal);
        assert_eq!(marker.delimiter, ListDelimiter::Dot);
    }

    #[test]
    fn reads_parenthesized_markers() {
        let marker = parse_list_marker("(a)", None).unwrap();
        assert_eq!(marker.value, 1);
        assert_eq!(marker.style, ListStyle::LowerAlpha);
        assert_eq!(marker.delimiter, ListDelimiter::ParenBoth);
    }

    #[test]
    fn first_marker_picks_roman_for_i() {
        let marker = parse_list_marker("i.", None).unwrap();
        assert_eq!(marker.style, ListStyle::LowerRoman);
        assert_eq!(marker.value, 1);
    }

    #[test]
    fn roman_continues_with_letter_style() {
        // `v.` continues an `i.` list as five.
        let marker = parse_list_marker("v.", Some(ListStyle::LowerRoman)).unwrap();
        assert_eq!(marker.value, 5);
    }

    #[test]
    fn writes_roman_numerals() {
        let marker = ListMarker {
            value: 4,
            style: ListStyle::LowerRoman,
            delimiter: ListDelimiter::Dot,
        };
        assert_eq!(
            format_list_marker(&marker, MarkerPosition::Continuation),
            "iv."
        );
    }

    #[test]
    fn writes_alpha_with_wraparound() {
        let marker = ListMarker {
            value: 27,
            style: ListStyle::LowerAlpha,
            delimiter: ListDelimiter::Paren,
        };
        assert_eq!(
            format_list_marker(&marker, MarkerPosition::Continuation),
            "a)"
        );
    }

    #[test]
    fn hash_marker_is_one_in_any_style() {
        let marker = parse_list_marker("#.", Some(ListStyle::LowerRoman)).unwrap();
        assert_eq!(marker.value, 1);
    }

    #[test]
    fn rejects_non_markers() {
        assert!(parse_list_marker("nope", None).is_none());
        assert!(parse_list_marker("1..", None).is_none());
    }
}
