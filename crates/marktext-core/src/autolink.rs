//! The extended autolink extension of GitHub Flavored Markdown.
//!
//! Ported from MarkText's `packages/muya/src/inline/parser/autoLink.ts` and
//! the `auto_link_extension` rule of `packages/muya/src/inline/parser/rules.ts`.
//!
//! The extension turns three kinds of plain text into links:
//!
//! * a `www.` address, whose target carries the `http://` scheme,
//! * an address that names its scheme itself, and
//! * an email address, whose target carries the `mailto:` scheme.
//!
//! The text of the link ends at the first blank.  The characters that do not
//! belong to the address are trimmed off its end afterwards.

/// The kind of an extended autolink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoLinkKind {
    /// A `www.` address, whose target carries the `http://` scheme.
    Www,
    /// An address that names its scheme itself.
    Url,
    /// An email address, whose target carries the `mailto:` scheme.
    Email,
}

/// The characters an extended autolink may follow.
///
/// The link has to start a word of its own, so it may not follow a letter
/// or a digit.
const BOUNDARY: &[u8] = b"* _~(";

/// The characters that end the text of an extended autolink.
const TRIMMED_END: &[u8] = b"?!.,:*_~";

/// Match the extended autolink that starts at `index`.
///
/// The result holds the kind of the link and the text it covers.
pub fn match_extended_autolink(text: &str, index: usize) -> Option<(AutoLinkKind, String)> {
    let bytes = text.as_bytes();
    if index > bytes.len() {
        return None;
    }
    // The link may only start at the beginning of the text or behind one of
    // the characters that cannot be part of a word.
    if index > 0 && !BOUNDARY.contains(&bytes[index - 1]) {
        return None;
    }
    if let Some(end) = match_www(text, index) {
        let raw = &text[index..end];
        return Some((AutoLinkKind::Www, trim_extent(raw)));
    }
    if let Some(end) = match_url(text, index) {
        let raw = &text[index..end];
        return Some((AutoLinkKind::Url, trim_extent(raw)));
    }
    if let Some(end) = match_email(text, index) {
        return Some((AutoLinkKind::Email, text[index..end].to_string()));
    }
    None
}

/// Match the `www.` autolink that starts at `index`.
///
/// The result is the offset behind the link.
fn match_www(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if !text[index..].starts_with("www.") {
        return None;
    }
    let mut cursor = index + 4;
    // The name of the host.
    let name_start = cursor;
    while cursor < bytes.len() && matches!(bytes[cursor], b'a'..=b'z' | b'_' | b'-') {
        cursor += 1;
    }
    if cursor == name_start || bytes.get(cursor) != Some(&b'.') {
        return None;
    }
    cursor += 1;
    // The top level domain.
    let domain_start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_lowercase() {
        cursor += 1;
    }
    if cursor - domain_start < 2 {
        return None;
    }
    // The domain may not run into a word.
    if cursor < bytes.len() && (is_word_byte(bytes[cursor]) || bytes[cursor] == b'-') {
        return None;
    }
    let cursor = skip_to_space_or_angle(bytes, cursor);
    if !lookahead(text, cursor) {
        return None;
    }
    Some(cursor)
}

/// Match the address autolink that starts at `index`.
///
/// The result is the offset behind the link.
fn match_url(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let scheme = if text[index..].starts_with("http://") {
        7
    } else if text[index..].starts_with("https://") {
        8
    } else if text[index..].starts_with("ftp://") {
        6
    } else {
        return None;
    };
    let host_start = index + scheme;
    let mut host_end = host_start;
    while host_end < bytes.len() && is_host_byte(bytes[host_end]) {
        host_end += 1;
    }

    // The host is one of four shapes; they are tried in this order.
    let mut end = None;
    // A name with a top level domain.
    for split in (host_start + 1..=host_end).rev() {
        if bytes.get(split) != Some(&b'.') {
            continue;
        }
        let domain_start = split + 1;
        let mut domain_end = domain_start;
        while domain_end < bytes.len() && bytes[domain_end].is_ascii_lowercase() {
            domain_end += 1;
        }
        if domain_end - domain_start < 2 {
            continue;
        }
        if domain_end < bytes.len()
            && (is_word_byte(bytes[domain_end]) || bytes[domain_end] == b'-')
        {
            continue;
        }
        end = Some(domain_end);
        break;
    }
    // An address made of digits and dots.
    if end.is_none() {
        let mut digits_end = host_start;
        while digits_end < bytes.len() && matches!(bytes[digits_end], b'0'..=b'9' | b'.') {
            digits_end += 1;
        }
        if digits_end > host_start {
            end = Some(digits_end);
        }
    }
    // The name of the local host.
    if end.is_none() && text[host_start..].starts_with("localhost") {
        end = Some(host_start + 9);
    }
    // An address in brackets.
    if end.is_none() && bytes.get(host_start) == Some(&b'[') {
        let mut bracket_end = host_start + 1;
        while bracket_end < bytes.len()
            && matches!(bytes[bracket_end], b'a'..=b'f' | b'0'..=b'9' | b'.' | b':')
        {
            bracket_end += 1;
        }
        if bracket_end > host_start + 1 && bytes.get(bracket_end) == Some(&b']') {
            end = Some(bracket_end + 1);
        }
    }
    let end = end?;
    // The host may not run into a word.
    if end < bytes.len() && (is_word_byte(bytes[end]) || bytes[end] == b'-') {
        return None;
    }
    let cursor = skip_to_space_or_angle(bytes, end);
    if !lookahead(text, cursor) {
        return None;
    }
    Some(cursor)
}

/// Match the email autolink that starts at `index`.
///
/// The result is the offset behind the link.
fn match_email(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    // RFC 5321 limits the local part to 64 characters. Stopping there also
    // keeps the scan cheap: the local part allows `*`, `_` and other
    // punctuation, so without a limit a document full of `*a*` would make
    // every `a` scan to the end of the file looking for an `@` that is not
    // there, which is quadratic.
    const MAX_LOCAL_PART: usize = 64;
    let mut cursor = index;
    while cursor < bytes.len()
        && cursor - index < MAX_LOCAL_PART
        && is_local_part_byte(bytes[cursor])
    {
        cursor += 1;
    }
    if cursor == index || bytes.get(cursor) != Some(&b'@') {
        return None;
    }
    cursor += 1;
    // The name of the host, which may hold several labels.
    cursor = match_domain_label(bytes, cursor)?;
    while bytes.get(cursor) == Some(&b'.') {
        let Some(end) = match_domain_label(bytes, cursor + 1) else {
            break;
        };
        cursor = end;
    }
    if !lookahead(text, cursor) {
        return None;
    }
    Some(cursor)
}

/// Match one label of the host of an email address.
///
/// The label begins and ends with a letter or a digit and holds at most
/// sixty-two characters.
fn match_domain_label(bytes: &[u8], cursor: usize) -> Option<usize> {
    let first = *bytes.get(cursor)?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    let limit = (cursor + 62).min(bytes.len());
    let mut end = cursor + 1;
    while end < limit && matches!(bytes[end], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-') {
        end += 1;
    }
    // The label may not end with a hyphen.
    while end > cursor + 1 && !bytes[end - 1].is_ascii_alphanumeric() {
        end -= 1;
    }
    Some(end)
}

/// Trim the characters that do not belong to the link.
///
/// A `<` ends the link, the punctuation `?!.,:*_~` at its end does not
/// belong to it, a `)` does not belong to it when the link holds more
/// closing than opening parentheses, and a `;` that closes a character
/// reference does not belong to it either.
fn trim_extent(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut end = match raw.find('<') {
        Some(index) => index,
        None => raw.len(),
    };
    // The number of closing parentheses that have no partner.  It only has
    // to be counted once.
    let mut excess_closing: Option<i32> = None;
    let mut changed = true;
    while changed && end > 0 {
        changed = false;
        let last = bytes[end - 1];
        if TRIMMED_END.contains(&last) {
            end -= 1;
            changed = true;
        } else if last == b')' {
            let excess = match excess_closing {
                Some(value) => value,
                None => {
                    let mut count = 0i32;
                    for &byte in &bytes[..end] {
                        if byte == b'(' {
                            count -= 1;
                        } else if byte == b')' {
                            count += 1;
                        }
                    }
                    excess_closing = Some(count);
                    count
                }
            };
            if excess > 0 {
                end -= 1;
                excess_closing = Some(excess - 1);
                changed = true;
            }
        } else if last == b';' && end >= 2 {
            // A `;` that closes a character reference does not belong to the
            // link.
            let mut start = end - 2;
            while start > 0 && bytes[start].is_ascii_alphanumeric() {
                start -= 1;
            }
            if start < end - 2 && bytes[start] == b'&' {
                end = start;
                changed = true;
            }
        }
    }
    raw[..end].to_string()
}

/// Whether the character behind the link may end it.
fn lookahead(text: &str, index: usize) -> bool {
    let Some(rest) = text.get(index..) else {
        return true;
    };
    let Some(ch) = rest.chars().next() else {
        return true;
    };
    if ch.is_whitespace() {
        return true;
    }
    if matches!(
        ch,
        '?' | '!' | ',' | ':' | '*' | '~' | '(' | ')' | '<' | '>'
    ) {
        return true;
    }
    if ch == '.' {
        // A dot that is not followed by a word character ends the link.
        return !rest[1..].chars().next().map(is_word_char).unwrap_or(false);
    }
    false
}

/// The offset behind the text that runs up to the next blank or angle
/// bracket.
fn skip_to_space_or_angle(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'<' {
        index += 1;
    }
    index
}

/// Whether the byte is a word character.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether the character is a word character.
fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Whether the byte may be part of the name of a host.
fn is_host_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

/// Whether the byte may be part of the name in front of the `@` of an email
/// address.
fn is_local_part_byte(byte: u8) -> bool {
    is_word_byte(byte)
        || matches!(
            byte,
            b'.' | b'!'
                | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'/'
                | b'='
                | b'?'
                | b'^'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'-'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn match_at(text: &str) -> Option<(AutoLinkKind, String)> {
        match_extended_autolink(text, 0)
    }

    #[test]
    fn matches_a_www_address() {
        assert_eq!(
            match_at("www.commonmark.org"),
            Some((AutoLinkKind::Www, "www.commonmark.org".to_string()))
        );
    }

    #[test]
    fn trims_the_punctuation_at_the_end() {
        assert_eq!(
            match_at("www.commonmark.org."),
            Some((AutoLinkKind::Www, "www.commonmark.org".to_string()))
        );
    }

    #[test]
    fn keeps_the_balanced_parentheses() {
        assert_eq!(
            match_at("www.google.com/search?q=Markup+(business)"),
            Some((
                AutoLinkKind::Www,
                "www.google.com/search?q=Markup+(business)".to_string()
            ))
        );
    }

    #[test]
    fn trims_the_unbalanced_parentheses() {
        assert_eq!(
            match_at("www.google.com/search?q=Markup+(business)))"),
            Some((
                AutoLinkKind::Www,
                "www.google.com/search?q=Markup+(business)".to_string()
            ))
        );
    }

    #[test]
    fn cuts_the_link_at_the_angle_bracket() {
        assert_eq!(
            match_at("www.commonmark.org/he<lp"),
            Some((AutoLinkKind::Www, "www.commonmark.org/he".to_string()))
        );
    }

    #[test]
    fn cuts_the_link_at_the_character_reference() {
        assert_eq!(
            match_at("www.google.com/search?q=commonmark&hl;"),
            Some((
                AutoLinkKind::Www,
                "www.google.com/search?q=commonmark".to_string()
            ))
        );
    }

    #[test]
    fn matches_an_address_with_its_scheme() {
        assert_eq!(
            match_at("http://commonmark.org"),
            Some((AutoLinkKind::Url, "http://commonmark.org".to_string()))
        );
    }

    #[test]
    fn matches_an_email_address() {
        assert_eq!(
            match_at("foo@bar.baz"),
            Some((AutoLinkKind::Email, "foo@bar.baz".to_string()))
        );
    }

    #[test]
    fn rejects_an_email_address_that_ends_in_a_hyphen() {
        assert_eq!(match_at("a.b-c_d@a.b-"), None);
    }
}
