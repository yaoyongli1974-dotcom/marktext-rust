//! Inline parsing: the raw text of a block into a token stream.
//!
//! Ported from MarkText's `packages/muya/src/inline`. The emphasis algorithm
//! follows CommonMark's delimiter-stack procedure.

use serde::{Deserialize, Serialize};

use crate::autolink::AutoLinkKind;

/// One inline token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text { raw: String },
    /// Emphasis (`*` or `_`).
    Emphasis { marker: char, children: Vec<Inline> },
    /// Strong emphasis (`**` or `__`).
    Strong { marker: char, children: Vec<Inline> },
    /// Strikethrough (`~~`).
    Del {
        marker: String,
        children: Vec<Inline>,
    },
    /// Highlight (`==`).
    Mark {
        marker: String,
        children: Vec<Inline>,
    },
    /// Inline code span.
    InlineCode { marker: char, content: String },
    /// Inline math (`$...$`).
    InlineMath { marker: String, content: String },
    /// An emoji shortcode such as `:smile:`.
    Emoji { content: String },
    /// A link with inline text.
    Link {
        marker: char,
        href: String,
        title: Option<String>,
        children: Vec<Inline>,
    },
    /// A reference link such as `[text][label]`.
    ReferenceLink {
        label: String,
        children: Vec<Inline>,
    },
    /// An image.
    Image {
        marker: char,
        src: String,
        title: Option<String>,
        alt: String,
    },
    /// A reference image such as `![alt][label]`.
    ReferenceImage { label: String, alt: String },
    /// An autolink (`<https://example.com>`).
    AutoLink { href: String, email: bool },
    /// An extended autolink such as `www.example.com` or `foo@bar.com`.
    AutoLinkExtension { kind: AutoLinkKind, text: String },
    /// A raw HTML tag.
    HtmlTag { raw: String },
    /// An HTML entity such as `&amp;`.
    ///
    /// The text it stands for is decoded while parsing, so that the renderer
    /// can emit the characters instead of the reference.  It is a string and
    /// not a character because a few references stand for more than one
    /// character — `&ngE;` is `≧` followed by a combining mark.
    HtmlEscape {
        raw: String,
        decoded: Option<String>,
    },
    /// A backslash escape.
    Escape { raw: String, content: String },
    /// A hard line break.
    HardBreak { raw: String },
    /// A soft line break.
    SoftBreak { raw: String },
    /// A footnote reference.
    Footnote { identifier: String },
    /// Superscript (`^...^`).
    Superscript { content: String },
    /// Subscript (`~...~`).
    Subscript { content: String },
    /// Inserted text (`{++...++}`), the inline diff extension.
    Insert { content: String },
    /// Deleted text (`{--...--}`), the inline diff extension.
    Delete { content: String },
    /// A delimiter run that is still waiting to be resolved into emphasis.
    ///
    /// The run lives in the same list as the other tokens so that the
    /// emphasis algorithm can find and remove it without tracking a separate
    /// index space.
    Delim {
        marker: char,
        count: usize,
        can_open: bool,
        can_close: bool,
    },
}

/// A reference definition, e.g. `[label]: https://example.com "title"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDefinition {
    pub label: String,
    pub href: String,
    pub title: Option<String>,
}

/// Feature switches for inline parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineOptions {
    pub tex_math_dollars: bool,
    pub tex_math_gfm: bool,
    pub footnote: bool,
    pub emoji: bool,
    pub mark_syntax: bool,
    pub inline_diff: bool,
    pub super_sub_script: bool,
    /// Whether plain text may turn into an extended autolink.
    ///
    /// This is the autolink extension of GitHub Flavored Markdown: a `www.`
    /// address, an address that names its scheme and an email address all
    /// become links.
    pub gfm_autolink: bool,
    /// The link reference definitions of the document.
    ///
    /// The parser needs them to tell a reference link from a plain bracket:
    /// a bracket that matches no definition stays the text it was written as.
    pub references: Vec<ReferenceDefinition>,
}

impl InlineOptions {
    /// Whether the document holds a definition for the label.
    pub fn has_reference(&self, label: &str) -> bool {
        self.references
            .iter()
            .any(|definition| definition.label == label)
    }
}

impl Default for InlineOptions {
    fn default() -> Self {
        InlineOptions {
            tex_math_dollars: true,
            tex_math_gfm: false,
            footnote: false,
            emoji: true,
            mark_syntax: true,
            inline_diff: true,
            super_sub_script: true,
            gfm_autolink: true,
            references: Vec::new(),
        }
    }
}

/// Parse inline text into tokens.
pub fn parse(text: &str, options: InlineOptions) -> Vec<Inline> {
    let mut tokens = scan(text, options);
    resolve_emphasis(&mut tokens);
    merge_adjacent_text(&mut tokens);
    trim_trailing_whitespace(&mut tokens);
    tokens
}

/// Drop the whitespace at the very end of the text.
///
/// CommonMark removes the final whitespace of a paragraph, so a line that
/// ends in two spaces does not open a hard line break when nothing follows
/// it.
fn trim_trailing_whitespace(tokens: &mut [Inline]) {
    if let Some(Inline::Text { raw }) = tokens.last_mut() {
        let trimmed = raw.trim_end();
        if trimmed.len() != raw.len() {
            *raw = trimmed.to_string();
        }
    }
}

/// Collect the reference definitions of a document.
///
/// MarkText keeps definitions as ordinary paragraph text and scans them here,
/// which keeps an unmodified document byte-identical.
pub fn collect_reference_definitions(text: &str) -> Vec<ReferenceDefinition> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        let Some(close) = trimmed.find("]:") else {
            continue;
        };
        let label = &trimmed[1..close];
        if label.is_empty() || label.contains(char::is_whitespace) {
            continue;
        }
        let rest = trimmed[close + 2..].trim();
        if rest.is_empty() {
            continue;
        }
        let (href, title) = split_link_destination(rest);
        out.push(ReferenceDefinition {
            label: normalize_label(label),
            href,
            title,
        });
    }
    out
}

/// Normalize a reference label: trim, collapse whitespace, case fold.
///
/// The case folding is the full Unicode one the specification asks for, so
/// `ẞ` and `SS` end up as the same label.
pub fn normalize_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut last_was_space = false;
    for ch in label.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push_str(&crate::casefold::fold(ch));
            last_was_space = false;
        }
    }
    out
}

/// Whether a link label holds an unescaped bracket.
///
/// The specification forbids brackets in the label of a reference link, so
/// that a bracket that matches no definition cannot swallow the text around
/// it.
fn label_has_bracket(label: &str) -> bool {
    let bytes = label.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' | b']' => return true,
            _ => index += 1,
        }
    }
    false
}

/// Split a link destination and its optional title.
fn split_link_destination(text: &str) -> (String, Option<String>) {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('<') {
        if let Some(end) = rest.find('>') {
            let href = rest[..end].to_string();
            let title = rest[end + 1..]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            let title = if title.is_empty() { None } else { Some(title) };
            return (href, title);
        }
    }
    match text.find(char::is_whitespace) {
        Some(index) => {
            let href = text[..index].to_string();
            let title = text[index..]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            let title = if title.is_empty() { None } else { Some(title) };
            (href, title)
        }
        None => (text.to_string(), None),
    }
}

/// Scan the text into a flat token list, recording emphasis delimiters.
fn scan(text: &str, options: InlineOptions) -> Vec<Inline> {
    let bytes = text.as_bytes();
    let mut tokens: Vec<Inline> = Vec::new();
    let mut text_start = 0usize;
    let mut index = 0usize;

    macro_rules! flush_text {
        ($end:expr) => {
            if $end > text_start {
                tokens.push(Inline::Text {
                    raw: text[text_start..$end].to_string(),
                });
            }
        };
    }

    while index < bytes.len() {
        let ch = bytes[index];
        match ch {
            b'\\' => {
                if let Some(next) = bytes.get(index + 1) {
                    if is_escapable(*next) {
                        flush_text!(index);
                        tokens.push(Inline::Escape {
                            raw: text[index..index + 2].to_string(),
                            content: text[index + 1..index + 2].to_string(),
                        });
                        index += 2;
                        text_start = index;
                        continue;
                    }
                }
                index += 1;
            }
            b'`' => {
                if let Some((token, end)) = parse_code_span(text, index) {
                    flush_text!(index);
                    tokens.push(token);
                    index = end;
                    text_start = index;
                    continue;
                }
                // A backtick string that opens no code span stays literal
                // text, and the whole string is skipped: a shorter string
                // inside it may not open a code span of its own.
                index += count_run(bytes, index, b'`');
            }
            b'$' => {
                if let Some((token, end)) = parse_inline_math(text, index, &options) {
                    flush_text!(index);
                    tokens.push(token);
                    index = end;
                    text_start = index;
                    continue;
                }
                index += 1;
            }
            b'<' => {
                if let Some((token, end)) = parse_angle(text, index) {
                    flush_text!(index);
                    tokens.push(token);
                    index = end;
                    text_start = index;
                    continue;
                }
                index += 1;
            }
            b'&' => {
                if let Some((token, end)) = parse_entity(text, index) {
                    flush_text!(index);
                    tokens.push(token);
                    index = end;
                    text_start = index;
                    continue;
                }
                index += 1;
            }
            b'!' => {
                if let Some((token, end)) = parse_image(text, index, &options) {
                    flush_text!(index);
                    tokens.push(token);
                    index = end;
                    text_start = index;
                    continue;
                }
                index += 1;
            }
            b'[' => {
                if let Some((token, end)) = parse_link(text, index, &options) {
                    flush_text!(index);
                    tokens.push(token);
                    index = end;
                    text_start = index;
                    continue;
                }
                index += 1;
            }
            b':' => {
                if options.emoji {
                    if let Some((token, end)) = parse_emoji(text, index) {
                        flush_text!(index);
                        tokens.push(token);
                        index = end;
                        text_start = index;
                        continue;
                    }
                }
                index += 1;
            }
            b'^' => {
                if options.super_sub_script {
                    if let Some((token, end)) = parse_script(text, index, '^') {
                        flush_text!(index);
                        tokens.push(token);
                        index = end;
                        text_start = index;
                        continue;
                    }
                }
                index += 1;
            }
            b'~' => {
                let run = count_run(bytes, index, b'~');
                if run >= 2 {
                    if let Some((token, end)) = parse_delimited(text, index, b'~', 2, &options) {
                        flush_text!(index);
                        tokens.push(token);
                        index = end;
                        text_start = index;
                        continue;
                    }
                }
                if options.super_sub_script {
                    if let Some((token, end)) = parse_script(text, index, '~') {
                        flush_text!(index);
                        tokens.push(token);
                        index = end;
                        text_start = index;
                        continue;
                    }
                }
                index += run;
                continue;
            }
            b'=' => {
                if options.mark_syntax {
                    let run = count_run(bytes, index, b'=');
                    if run >= 2 {
                        if let Some((token, end)) = parse_delimited(text, index, b'=', 2, &options)
                        {
                            flush_text!(index);
                            tokens.push(token);
                            index = end;
                            text_start = index;
                            continue;
                        }
                    }
                    index += run;
                    continue;
                }
                index += 1;
            }
            b'{' => {
                if options.inline_diff {
                    if let Some((token, end)) = parse_inline_diff(text, index) {
                        flush_text!(index);
                        tokens.push(token);
                        index = end;
                        text_start = index;
                        continue;
                    }
                }
                index += 1;
            }
            b'*' | b'_' => {
                let run = count_run(bytes, index, ch);
                let marker = ch as char;
                let before = char_before(bytes, index);
                let after = char_at(bytes, index + run);
                let (can_open, can_close) = flanking(before, after, marker);
                flush_text!(index);
                tokens.push(Inline::Delim {
                    marker,
                    count: run,
                    can_open,
                    can_close,
                });
                index += run;
                text_start = index;
            }
            b'\n' => {
                // The whitespace and the backslash in front of a break are
                // not part of the text.
                let mut end = index;
                let mut spaces = 0usize;
                while end > text_start && text.as_bytes()[end - 1] == b' ' {
                    end -= 1;
                    spaces += 1;
                }
                // A backslash that is not part of an escape marks a hard
                // break as well.
                let backslash = end > text_start && text.as_bytes()[end - 1] == b'\\';
                let is_hard = spaces >= 2 || backslash;
                let break_start = if backslash { end - 1 } else { end };
                flush_text!(break_start);
                // The break itself is the source from the end of the text
                // (including the trailing spaces or the backslash) through
                // the newline. It must not be taken from `text_start`: that
                // still points at the beginning of the line, which used to
                // duplicate the whole line in the preview.
                let raw = text[break_start..=index].to_string();
                if is_hard {
                    tokens.push(Inline::HardBreak { raw });
                } else {
                    tokens.push(Inline::SoftBreak { raw });
                }
                index += 1;
                text_start = index;
            }
            _ => {
                // The extended autolink of GitHub Flavored Markdown turns
                // plain text into a link.
                if options.gfm_autolink {
                    if let Some((kind, link_text)) =
                        crate::autolink::match_extended_autolink(text, index)
                    {
                        flush_text!(index);
                        let length = link_text.len();
                        tokens.push(Inline::AutoLinkExtension {
                            kind,
                            text: link_text,
                        });
                        index += length;
                        text_start = index;
                        continue;
                    }
                }
                index += 1;
            }
        }
    }

    flush_text!(text.len());
    resolve_emphasis(&mut tokens);
    tokens
}

/// The character before or after a run, treating the string edges as
/// whitespace so a run at the boundary behaves like CommonMark requires.
fn char_at(bytes: &[u8], index: usize) -> char {
    let Some(byte) = bytes.get(index) else {
        return '\n';
    };
    let length = utf8_length(*byte);
    if length == 1 {
        return *byte as char;
    }
    // The character has to be complete; a truncated one is read as the end
    // of the line.
    let Some(window) = bytes.get(index..index + length) else {
        return '\n';
    };
    match std::str::from_utf8(window) {
        Ok(text) => text.chars().next().unwrap_or('\n'),
        Err(_) => '\n',
    }
}

/// The character that ends right before `index`.
///
/// The boundary of the character is found by walking back over the
/// continuation bytes of a UTF-8 sequence.
fn char_before(bytes: &[u8], index: usize) -> char {
    if index == 0 {
        return '\n';
    }
    let mut start = index - 1;
    while start > 0 && (bytes[start] & 0xC0) == 0x80 {
        start -= 1;
    }
    match std::str::from_utf8(&bytes[start..index]) {
        Ok(text) => text.chars().next_back().unwrap_or('\n'),
        Err(_) => '\n',
    }
}

/// How many bytes the UTF-8 sequence that starts with the byte holds.
fn utf8_length(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Count how many times `byte` repeats at `index`.
fn count_run(bytes: &[u8], index: usize, byte: u8) -> usize {
    let mut count = 0;
    while index + count < bytes.len() && bytes[index + count] == byte {
        count += 1;
    }
    count
}

/// Whether an ASCII byte may be backslash-escaped.
fn is_escapable(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'"'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b'-'
            | b'.'
            | b'/'
            | b':'
            | b';'
            | b'<'
            | b'='
            | b'>'
            | b'?'
            | b'@'
            | b'['
            | b'\\'
            | b']'
            | b'^'
            | b'_'
            | b'`'
            | b'{'
            | b'|'
            | b'}'
            | b'~'
    )
}

/// Compute the left- and right-flanking properties of a delimiter run.
///
/// The classification of the neighbouring characters follows the rules the
/// original editor uses: a character is punctuation when it belongs to the
/// Unicode `P` or `S` general category, and the CJK ranges act as a
/// boundary of their own.
fn flanking(before: char, after: char, marker: char) -> (bool, bool) {
    let before_is_space = crate::unicode::is_whitespace(before);
    let after_is_space = crate::unicode::is_whitespace(after);
    let before_is_punct = crate::unicode::is_punctuation(before);
    let after_is_punct = crate::unicode::is_punctuation(after);
    let before_is_cjk = crate::unicode::is_cjk(before);
    let after_is_cjk = crate::unicode::is_cjk(after);

    let before_boundary = before_is_space || before_is_punct || before_is_cjk;
    let after_boundary = after_is_space || after_is_punct || after_is_cjk;

    let left_flanking = !after_is_space && (!after_is_punct || before_boundary);
    let right_flanking = !before_is_space && (!before_is_punct || after_boundary);

    // `_` may not open emphasis when it is flanked by whitespace on both
    // sides, and may not close when preceded by whitespace.
    if marker == '_' {
        let can_open = left_flanking && (!right_flanking || before_boundary);
        let can_close = right_flanking && (!left_flanking || after_boundary);
        return (can_open, can_close);
    }

    (left_flanking, right_flanking)
}

/// Parse a code span starting at `start`.
fn parse_code_span(text: &str, start: usize) -> Option<(Inline, usize)> {
    let bytes = text.as_bytes();
    let open_run = count_run(bytes, start, b'`');
    let mut index = start + open_run;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let close_run = count_run(bytes, index, b'`');
            if close_run == open_run {
                let content = &text[start + open_run..index];
                // Line endings inside a code span become spaces.
                let content = content.replace("\r\n", " ").replace('\n', " ");
                let content = strip_code_padding(&content);
                return Some((
                    Inline::InlineCode {
                        marker: '`',
                        content: content.to_string(),
                    },
                    index + close_run,
                ));
            }
            index += close_run;
            continue;
        }
        index += 1;
    }
    None
}

/// Strip the padding spaces CommonMark removes from a code span.
///
/// A code span that both begins and ends with a space loses one space at
/// each end, unless it consists of spaces only.
fn strip_code_padding(content: &str) -> &str {
    if content.len() >= 2
        && content.starts_with(' ')
        && content.ends_with(' ')
        && content.chars().any(|c| c != ' ')
    {
        return &content[1..content.len() - 1];
    }
    content
}

/// Parse inline math starting at `start`.
fn parse_inline_math(text: &str, start: usize, options: &InlineOptions) -> Option<(Inline, usize)> {
    let bytes = text.as_bytes();
    if bytes[start] != b'$' {
        return None;
    }
    if options.tex_math_gfm && bytes.get(start + 1) == Some(&b'`') {
        if let Some(end) = text[start + 2..].find("`$") {
            let content = &text[start + 2..start + 2 + end];
            return Some((
                Inline::InlineMath {
                    marker: "$`".to_string(),
                    content: content.to_string(),
                },
                start + 2 + end + 2,
            ));
        }
    }
    if !options.tex_math_dollars {
        return None;
    }
    // `$$` is display math and is handled at the block level.
    if bytes.get(start + 1) == Some(&b'$') {
        return None;
    }
    if bytes
        .get(start + 1)
        .map(|b| b.is_ascii_whitespace())
        .unwrap_or(true)
    {
        return None;
    }
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'$' {
            if bytes
                .get(index + 1)
                .map(|b| b.is_ascii_digit())
                .unwrap_or(false)
            {
                index += 1;
                continue;
            }
            let content = &text[start + 1..index];
            if content.is_empty() {
                return None;
            }
            return Some((
                Inline::InlineMath {
                    marker: "$".to_string(),
                    content: content.to_string(),
                },
                index + 1,
            ));
        }
        index += 1;
    }
    None
}

/// Parse an autolink or a raw HTML tag starting at `start`.
fn parse_angle(text: &str, start: usize) -> Option<(Inline, usize)> {
    if let Some((token, end)) = parse_autolink(text, start) {
        return Some((token, end));
    }
    let end = scan_raw_html(text, start)?;
    Some((
        Inline::HtmlTag {
            raw: text[start..end].to_string(),
        },
        end,
    ))
}

/// Parse an autolink (`<https://example.com>`) starting at `start`.
///
/// An autolink may not hold a space or a `<`.  A backslash is part of the
/// address: the specification takes the address literally and only percent
/// encodes it for the link target, so `<https://example.com/\*>` links to
/// `https://example.com/%5C*` and shows the backslash as it was written.
fn parse_autolink(text: &str, start: usize) -> Option<(Inline, usize)> {
    let (email, end) = scan_autolink(text, start)?;
    let inner = &text[start + 1..end - 1];
    let token = if email {
        Inline::AutoLink {
            href: format!("mailto:{inner}"),
            email: true,
        }
    } else {
        Inline::AutoLink {
            href: inner.to_string(),
            email: false,
        }
    };
    Some((token, end))
}

/// Scan the autolink that starts at `start`.
///
/// The result tells whether the autolink holds an email address and where
/// the autolink ends.
fn scan_autolink(text: &str, start: usize) -> Option<(bool, usize)> {
    let bytes = text.as_bytes();
    let close = bytes[start + 1..]
        .iter()
        .position(|byte| *byte == b'>')
        .map(|offset| offset + start + 1)?;
    let inner = &text[start + 1..close];
    if inner.is_empty() || inner.contains(char::is_whitespace) || inner.contains('<') {
        return None;
    }

    // An absolute URI: a scheme of two to thirty-two characters, a colon
    // and the rest of the address.
    if let Some(scheme_end) = inner.find(':') {
        let scheme = &inner[..scheme_end];
        if (2..=32).contains(&scheme.len())
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        {
            return Some((false, close + 1));
        }
    }

    // An email address.
    if is_email_address(inner) {
        return Some((true, close + 1));
    }
    None
}

/// Whether the text is an email address the specification accepts.
fn is_email_address(text: &str) -> bool {
    let Some(at) = text.rfind('@') else {
        return false;
    };
    let (local, domain) = (&text[..at], &text[at + 1..]);
    if local.is_empty() || local.len() > 64 {
        return false;
    }
    if !local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~-.".contains(c))
    {
        return false;
    }
    if domain.is_empty() {
        return false;
    }
    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return false;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }
    }
    true
}

/// Whether the byte is whitespace as the HTML syntax defines it.
fn is_html_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

/// Whether the byte may be part of a tag name.
fn is_tag_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// Scan a raw HTML construct that starts at `start`.
///
/// Beside the tags the syntax holds a comment, a processing instruction, a
/// declaration and a CDATA section.
fn scan_raw_html(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    // A comment.  The two short forms are complete on their own.
    if rest.starts_with("<!-->") {
        return Some(start + 5);
    }
    if rest.starts_with("<!--->") {
        return Some(start + 6);
    }
    if let Some(after) = rest.strip_prefix("<!--") {
        let end = after.find("-->")? + 4;
        return Some(start + end + 3);
    }
    // A processing instruction.
    if let Some(after) = rest.strip_prefix("<?") {
        let end = after.find("?>")? + 2;
        return Some(start + end + 2);
    }
    // A CDATA section.
    if let Some(after) = rest.strip_prefix("<![CDATA[") {
        let end = after.find("]]>")? + 9;
        return Some(start + end + 3);
    }
    // A declaration.
    if let Some(after) = rest.strip_prefix("<!") {
        let end = after.find('>')? + 2;
        return Some(start + end + 1);
    }
    scan_raw_html_tag(text, start)
}

/// Scan a raw HTML tag that starts at `start`.
///
/// The result is the index behind the tag.  A tag may reach over a line
/// break, because a quoted attribute value may hold one.
pub(crate) fn scan_raw_html_tag(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let rest = &text[start..];

    // A closing tag: a name, optional whitespace and the closing bracket.
    if rest.starts_with("</") {
        let mut index = start + 2;
        let name_start = index;
        while index < bytes.len() && is_tag_name_byte(bytes[index]) {
            index += 1;
        }
        if index == name_start || !bytes.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
            return None;
        }
        while index < bytes.len() && is_html_space(bytes[index]) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'>') {
            return None;
        }
        return Some(index + 1);
    }

    // An open tag: a name, attributes and the closing bracket.
    if bytes.get(start) != Some(&b'<') {
        return None;
    }
    let mut index = start + 1;
    let name_start = index;
    while index < bytes.len() && is_tag_name_byte(bytes[index]) {
        index += 1;
    }
    if index == name_start || !bytes.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }

    loop {
        // Every attribute is separated from the one before it by whitespace.
        let before_space = index;
        while index < bytes.len() && is_html_space(bytes[index]) {
            index += 1;
        }
        let had_space = index > before_space;

        // The tag ends at the closing bracket, which may follow the name
        // directly.
        match bytes.get(index) {
            Some(b'>') => return Some(index + 1),
            Some(b'/') if bytes.get(index + 1) == Some(&b'>') => return Some(index + 2),
            _ => {}
        }
        if !had_space {
            return None;
        }

        // The name of the attribute.
        let attribute_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric()
                || matches!(bytes[index], b'_' | b'.' | b':' | b'-'))
        {
            index += 1;
        }
        if index == attribute_start
            || !bytes
                .get(attribute_start)
                .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':'))
        {
            return None;
        }

        // The optional value of the attribute.
        let after_name = index;
        while index < bytes.len() && is_html_space(bytes[index]) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            index = after_name;
            continue;
        }
        index += 1;
        while index < bytes.len() && is_html_space(bytes[index]) {
            index += 1;
        }
        match bytes.get(index) {
            Some(quote @ (b'"' | b'\'')) => {
                let quote = *quote;
                index += 1;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                if index >= bytes.len() {
                    return None;
                }
                index += 1;
            }
            Some(_) => {
                let value_start = index;
                while index < bytes.len()
                    && !is_html_space(bytes[index])
                    && !matches!(bytes[index], b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
                {
                    index += 1;
                }
                if index == value_start {
                    return None;
                }
            }
            None => return None,
        }
    }
}

/// Parse an HTML entity starting at `start`.
fn parse_entity(text: &str, start: usize) -> Option<(Inline, usize)> {
    let rest = &text[start..];
    // Only a reference that names a known character is a reference; anything
    // else stays the text it was written as.
    let (length, decoded) = crate::entities::decode_reference(rest)?;
    Some((
        Inline::HtmlEscape {
            raw: rest[..length].to_string(),
            decoded: Some(decoded),
        },
        start + length,
    ))
}

/// Parse an emoji shortcode starting at `start`.
fn parse_emoji(text: &str, start: usize) -> Option<(Inline, usize)> {
    let rest = &text[start + 1..];
    let end = rest.find(':')?;
    if end == 0 || end > 32 {
        return None;
    }
    let name = &rest[..end];
    if name.len() < 2
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-')
    {
        return None;
    }
    // `12:30:45` and similar times are not emoji shortcodes. A digit-only
    // name between two digits is far more likely to be a clock than `:100:`.
    let before = text[..start].chars().next_back();
    let after = text[start + end + 2..].chars().next();
    if name.chars().all(|c| c.is_ascii_digit())
        && before.is_some_and(|c| c.is_ascii_digit())
        && after.is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some((
        Inline::Emoji {
            content: name.to_string(),
        },
        start + end + 2,
    ))
}

/// Parse a `^super^` or `~sub~` script starting at `start`.
fn parse_script(text: &str, start: usize, marker: char) -> Option<(Inline, usize)> {
    let rest = &text[start + 1..];
    let end = rest.find(marker)?;
    if end == 0 {
        return None;
    }
    let content = &rest[..end];
    if content.contains(char::is_whitespace) {
        return None;
    }
    let token = if marker == '^' {
        Inline::Superscript {
            content: content.to_string(),
        }
    } else {
        Inline::Subscript {
            content: content.to_string(),
        }
    };
    Some((token, start + end + 2))
}

/// Parse a `{++diff++}` or `{--diff--}` span starting at `start`.
fn parse_inline_diff(text: &str, start: usize) -> Option<(Inline, usize)> {
    let rest = &text[start..];
    let (close, inserted) = if rest.starts_with("{++") {
        ("++}", true)
    } else if rest.starts_with("{--") {
        ("--}", false)
    } else {
        return None;
    };
    let end = rest.find(close)?;
    let content = rest[3..end].to_string();
    let token = if inserted {
        Inline::Insert { content }
    } else {
        Inline::Delete { content }
    };
    Some((token, start + end + 3))
}

/// Parse a `~~del~~` or `==mark==` span starting at `start`.
fn parse_delimited(
    text: &str,
    start: usize,
    marker: u8,
    length: usize,
    options: &InlineOptions,
) -> Option<(Inline, usize)> {
    let bytes = text.as_bytes();
    let open: String = text[start..start + length].to_string();
    let mut index = start + length;
    while index < bytes.len() {
        if bytes[index] == marker {
            let run = count_run(bytes, index, marker);
            if run >= length {
                let inner = &text[start + length..index];
                let children = parse(inner, options.clone());
                let token = if marker == b'~' {
                    Inline::Del {
                        marker: open,
                        children,
                    }
                } else {
                    Inline::Mark {
                        marker: open,
                        children,
                    }
                };
                return Some((token, index + length));
            }
            index += run;
            continue;
        }
        index += 1;
    }
    None
}

/// Parse an image starting at `start`.
fn parse_image(text: &str, start: usize, options: &InlineOptions) -> Option<(Inline, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'!') || bytes.get(start + 1) != Some(&b'[') {
        return None;
    }
    let (alt, after_alt) = match_matching_bracket(text, start + 1)?;
    let alt = alt.trim();

    // An inline image: the parenthesis has to follow the bracket directly.
    if bytes.get(after_alt) == Some(&b'(') {
        if let Some((href, title, end)) = parse_link_destination(text, after_alt) {
            return Some((
                Inline::Image {
                    marker: '!',
                    src: href,
                    title,
                    alt: strip_emphasis_markers(alt),
                },
                end,
            ));
        }
    }

    // A full or a collapsed reference image.
    if bytes.get(after_alt) == Some(&b'[') {
        if let Some((label, end)) = match_matching_bracket(text, after_alt) {
            let label = label.trim();
            let target = if label.is_empty() { alt } else { label };
            if !label_has_bracket(target) {
                let normalized = normalize_label(target);
                if options.has_reference(&normalized) {
                    return Some((
                        Inline::ReferenceImage {
                            label: normalized,
                            alt: strip_emphasis_markers(alt),
                        },
                        end,
                    ));
                }
            }
        }
    }

    // A shortcut reference image.
    if !matches!(bytes.get(after_alt), Some(b'[') | Some(b']')) && !label_has_bracket(alt) {
        let normalized = normalize_label(alt);
        if options.has_reference(&normalized) {
            return Some((
                Inline::ReferenceImage {
                    label: normalized,
                    alt: strip_emphasis_markers(alt),
                },
                after_alt,
            ));
        }
    }

    None
}

/// Parse a link starting at `start`.
///
/// The three flavours of a link are tried in the order the specification
/// gives: an inline link, a full or collapsed reference link and finally a
/// shortcut reference link.  A bracket that matches none of them stays the
/// text it was written as.
fn parse_link(text: &str, start: usize, options: &InlineOptions) -> Option<(Inline, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    // The link text ends at the bracket that closes it.  Code spans,
    // autolinks and raw HTML bind more tightly than the brackets, and a
    // link text that holds a link of its own never becomes a link.
    let info = find_link_text(text, start, options)?;
    if info.holds_link {
        return None;
    }
    let after_label = info.end;
    let label = info.label.trim();

    if options.footnote && label.starts_with('^') && label.len() > 1 {
        return Some((
            Inline::Footnote {
                identifier: label[1..].to_string(),
            },
            after_label,
        ));
    }

    // An inline link: the parenthesis has to follow the bracket directly.
    if bytes.get(after_label) == Some(&b'(') {
        if let Some((href, title, end)) = parse_link_destination(text, after_label) {
            let children = parse(label, link_text_options(options));
            return Some((
                Inline::Link {
                    marker: '[',
                    href,
                    title,
                    children,
                },
                end,
            ));
        }
        // A destination that cannot be parsed leaves the bracket to the
        // reference rules below.
    }

    // A full or a collapsed reference link: the second bracket follows the
    // first one directly.
    if bytes.get(after_label) == Some(&b'[') {
        if let Some((reference, end)) = match_matching_bracket(text, after_label) {
            let reference = reference.trim();
            let target = if reference.is_empty() {
                label
            } else {
                reference
            };
            if !label_has_bracket(target) {
                let normalized = normalize_label(target);
                if options.has_reference(&normalized) {
                    let children = parse(label, link_text_options(options));
                    return Some((
                        Inline::ReferenceLink {
                            label: normalized,
                            children,
                        },
                        end,
                    ));
                }
            }
        }
    }

    // A shortcut reference link: the label may not be followed directly by
    // a link label, it may not hold a bracket itself, and it has to match a
    // definition.
    if bytes.get(after_label) != Some(&b'[') && !label_has_bracket(label) {
        let normalized = normalize_label(label);
        if options.has_reference(&normalized) {
            let children = parse(label, link_text_options(options));
            return Some((
                Inline::ReferenceLink {
                    label: normalized,
                    children,
                },
                after_label,
            ));
        }
    }

    // Not a link at all: the bracket stays the text it was written as.
    None
}

/// The options for the text inside a link.
///
/// The extended autolink of GitHub Flavored Markdown does not reach into
/// the text of a link.
fn link_text_options(options: &InlineOptions) -> InlineOptions {
    let mut options = options.clone();
    options.gfm_autolink = false;
    options
}

/// The link text that a `[` opens.
struct LinkText {
    /// The raw text between the brackets.
    label: String,
    /// The offset behind the closing bracket.
    end: usize,
    /// Whether the link text holds a link of its own.
    ///
    /// A link may not contain another link, so such a link text never
    /// becomes a link itself.
    holds_link: bool,
}

/// Find the closing bracket of the link text that opens at `start`.
///
/// Code spans, autolinks and raw HTML bind more tightly than the brackets
/// of a link text, so the brackets they hold do not count.
fn find_link_text(text: &str, start: usize, options: &InlineOptions) -> Option<LinkText> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut openers: Vec<Opener> = vec![Opener {
        position: start,
        image: false,
    }];
    let mut holds_link = false;
    let mut pending_image = false;
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => match scan_code_span_end(text, index) {
                Some(end) => index = end,
                None => index += 1,
            },
            b'<' => match scan_autolink_end(text, index) {
                Some(end) => index = end,
                None => match scan_raw_html(text, index) {
                    Some(end) => index = end,
                    None => index += 1,
                },
            },
            b'!' => {
                // A `![` opens an image description, not a link.
                if bytes.get(index + 1) == Some(&b'[') {
                    pending_image = true;
                }
                index += 1;
            }
            b'[' => {
                openers.push(Opener {
                    position: index,
                    image: pending_image,
                });
                pending_image = false;
                index += 1;
            }
            b']' => {
                let open = openers.pop()?;
                if openers.is_empty() {
                    // This bracket closes the link text.
                    return Some(LinkText {
                        label: text[start + 1..index].to_string(),
                        end: index + 1,
                        holds_link,
                    });
                }
                // A bracket inside the link text: it may hold a link of
                // its own, which would keep the link text from becoming a
                // link.  An image description never holds one.
                if !open.image && bracket_forms_link(text, open.position, index, options) {
                    holds_link = true;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// A bracket that is still open inside a link text.
struct Opener {
    /// Where the bracket stands.
    position: usize,
    /// Whether the bracket opens an image description.
    image: bool,
}

/// Whether the bracket that opens at `open` and closes at `close` forms a
/// link of its own.
fn bracket_forms_link(text: &str, open: usize, close: usize, options: &InlineOptions) -> bool {
    let bytes = text.as_bytes();
    let after = close + 1;
    let link_text = text[open + 1..close].trim();

    // An inline link.
    if bytes.get(after) == Some(&b'(') && parse_link_destination(text, after).is_some() {
        return true;
    }
    // A full or a collapsed reference link.
    if bytes.get(after) == Some(&b'[') {
        let Some((label, _)) = match_matching_bracket(text, after) else {
            return false;
        };
        let label = label.trim();
        let target = if label.is_empty() { link_text } else { label };
        return !label_has_bracket(target) && options.has_reference(&normalize_label(target));
    }
    // A shortcut reference link.
    if !label_has_bracket(link_text) {
        return options.has_reference(&normalize_label(link_text));
    }
    false
}

/// The offset behind the code span that starts at `start`.
fn scan_code_span_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let open_run = count_run(bytes, start, b'`');
    let mut index = start + open_run;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let close_run = count_run(bytes, index, b'`');
            if close_run == open_run {
                return Some(index + close_run);
            }
            index += close_run;
            continue;
        }
        index += 1;
    }
    None
}

/// The offset behind the autolink that starts at `start`.
fn scan_autolink_end(text: &str, start: usize) -> Option<usize> {
    scan_autolink(text, start).map(|(_, end)| end)
}

/// Find the matching `]` for the `[` at `start`, honoring nesting and escapes.
fn match_matching_bracket(text: &str, start: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => {
                depth += 1;
                index += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[start + 1..index].to_string(), index + 1));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Parse `(href "title")` starting at `start`.
fn parse_link_destination(text: &str, start: usize) -> Option<(String, Option<String>, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'(') {
        return None;
    }
    let mut index = start + 1;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let href_start = index;
    let href;
    if bytes.get(index) == Some(&b'<') {
        // A destination in angle brackets may not hold a line break, and its
        // closing bracket has to be an unescaped one.
        index += 1;
        let mut closed = None;
        while index < bytes.len() {
            match bytes[index] {
                b'>' => {
                    closed = Some(index);
                    break;
                }
                b'\\' => index += 2,
                b'\n' => break,
                _ => index += 1,
            }
        }
        let closed = closed?;
        href = text[href_start + 1..closed].to_string();
        index = closed + 1;
    } else {
        // A plain destination ends at the first whitespace or at the
        // parenthesis that closes the link.  Parentheses inside it have to
        // balance.
        let mut depth = 0usize;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => {
                    if index + 1 < bytes.len() && bytes[index + 1].is_ascii_punctuation() {
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                b'(' => {
                    depth += 1;
                    index += 1;
                }
                b')' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    index += 1;
                }
                // A destination that is not written in angle brackets may not
                // hold unescaped whitespace — that is where the title starts.
                b' ' | b'\t' | b'\n' => break,
                _ => index += 1,
            }
        }
        if index >= bytes.len() {
            return None;
        }
        href = text[href_start..index].to_string();
    }
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut title = None;
    if let Some(quote) = bytes
        .get(index)
        .copied()
        .filter(|b| matches!(b, b'"' | b'\'' | b'('))
    {
        let (content, end) = if quote == b'(' {
            // A title in parentheses: the parentheses inside it have to
            // balance.
            let mut depth = 0usize;
            let mut cursor = index;
            let mut closed = None;
            while cursor < bytes.len() {
                match bytes[cursor] {
                    b'\\' => cursor += 2,
                    b'(' => {
                        depth += 1;
                        cursor += 1;
                    }
                    b')' => {
                        depth -= 1;
                        cursor += 1;
                        if depth == 0 {
                            closed = Some(cursor - 1);
                            break;
                        }
                    }
                    _ => cursor += 1,
                }
            }
            let closed = closed?;
            (&text[index + 1..closed], closed + 1)
        } else {
            let mut cursor = index + 1;
            while cursor < bytes.len() && bytes[cursor] != quote {
                if bytes[cursor] == b'\\' {
                    cursor += 1;
                }
                cursor += 1;
            }
            if cursor >= bytes.len() {
                return None;
            }
            (&text[index + 1..cursor], cursor + 1)
        };
        title = Some(content.to_string());
        index = end;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
    }
    if bytes.get(index) != Some(&b')') {
        return None;
    }
    Some((href, title, index + 1))
}

/// Remove the emphasis markers from an image's alt text.
fn strip_emphasis_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '*' | '_' | '~' | '=') {
            let total = text.chars().filter(|other| *other == character).count();
            if total >= 2 {
                continue;
            }
        }
        out.push(character);
    }
    out
}

/// A delimiter run the emphasis resolver works with.
///
/// The resolver follows the delimiter stack of the reference implementation:
/// every run of `*` or `_` becomes a delimiter that may open or close
/// emphasis, and the runs are paired off the same way.
struct Delimiter {
    /// The character the run consists of.
    marker: char,
    /// The length of the whole run.
    ///
    /// The rule of three uses the length of the *original* run, even when
    /// part of it has already been consumed.
    length: usize,
    /// How many delimiters of the run are still available.
    remaining: usize,
    /// Whether the run may open emphasis.
    can_open: bool,
    /// Whether the run may close emphasis.
    can_close: bool,
    /// The index of the token that holds the run.
    token: usize,
    /// The delimiter that stands before this one in the stack.
    previous: Option<usize>,
    /// The delimiter that stands after this one in the stack.
    next: Option<usize>,
}

impl Delimiter {
    /// Whether the run may open emphasis.
    fn can_open(&self) -> bool {
        self.can_open && self.remaining > 0
    }

    /// Whether the run may close emphasis.
    fn can_close(&self) -> bool {
        self.can_close && self.remaining > 0
    }
}

/// Whether an opener and a closer may be paired.
///
/// This is the "rule of three" of the specification: when one of the two
/// runs can both open and close emphasis, the sum of the lengths of the
/// runs must not be a multiple of three, unless both lengths are multiples
/// of three.
fn may_pair(opener: &Delimiter, closer: &Delimiter) -> bool {
    if !(closer.can_open || opener.can_close) {
        return true;
    }
    if closer.length % 3 == 0 {
        return true;
    }
    (opener.length + closer.length) % 3 != 0
}

/// Resolve emphasis with CommonMark's delimiter-stack algorithm.
///
/// The scan emits one placeholder token per delimiter run.  This pass walks
/// the runs the way the reference implementation does: it looks for a closer
/// and then searches backwards for the nearest opener that may match it.
/// When a pair is found, the tokens between them become the content of an
/// emphasis or strong token, and the runs that are left over keep their
/// place in front of and behind the new token.
fn resolve_emphasis(tokens: &mut Vec<Inline>) {
    let mut delims: Vec<Delimiter> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if let Inline::Delim {
            marker,
            count,
            can_open,
            can_close,
        } = token
        {
            delims.push(Delimiter {
                marker: *marker,
                length: *count,
                remaining: *count,
                can_open: *can_open,
                can_close: *can_close,
                token: index,
                previous: None,
                next: None,
            });
        }
    }
    for index in 1..delims.len() {
        delims[index].previous = Some(index - 1);
        delims[index - 1].next = Some(index);
    }

    let mut closer = if delims.is_empty() {
        None
    } else {
        Some(0usize)
    };
    while let Some(index) = closer {
        if !delims[index].can_close() {
            closer = delims[index].next;
            continue;
        }

        // Look back for the nearest opener that can match this closer.
        let mut opener = None;
        let mut candidate = delims[index].previous;
        while let Some(candidate_index) = candidate {
            let delim = &delims[candidate_index];
            if delim.can_open()
                && delim.marker == delims[index].marker
                && may_pair(delim, &delims[index])
            {
                opener = Some(candidate_index);
                break;
            }
            candidate = delims[candidate_index].previous;
        }

        let Some(opener_index) = opener else {
            // Without an opener the run stays literal text.
            closer = delims[index].next;
            continue;
        };

        closer = insert_emphasis(tokens, &mut delims, opener_index, index);
    }
}

/// Pair an opener with a closer and turn the text between them into an
/// emphasis token.
///
/// The function returns the closer that has to be looked at next: the closer
/// itself when it still holds delimiters, the one behind it otherwise.
fn insert_emphasis(
    tokens: &mut Vec<Inline>,
    delims: &mut [Delimiter],
    opener: usize,
    closer: usize,
) -> Option<usize> {
    // Two characters on both sides make strong emphasis, one makes plain
    // emphasis.
    let use_delims = if delims[opener].remaining >= 2 && delims[closer].remaining >= 2 {
        2
    } else {
        1
    };
    delims[opener].remaining -= use_delims;
    delims[closer].remaining -= use_delims;

    let opener_token = delims[opener].token;
    let closer_token = delims[closer].token;

    // The content between the two runs becomes the content of the emphasis.
    let children: Vec<Inline> = tokens[opener_token + 1..closer_token].to_vec();
    let marker = delims[closer].marker;
    let node = if use_delims == 2 {
        Inline::Strong { marker, children }
    } else {
        Inline::Emphasis { marker, children }
    };

    // The delimiters that are left over keep their place in front of and
    // behind the new emphasis.
    let mut replacement: Vec<Inline> = Vec::with_capacity(3);
    if delims[opener].remaining > 0 {
        replacement.push(Inline::Delim {
            marker: delims[opener].marker,
            count: delims[opener].remaining,
            can_open: delims[opener].can_open,
            can_close: delims[opener].can_close,
        });
    }
    replacement.push(node);
    if delims[closer].remaining > 0 {
        replacement.push(Inline::Delim {
            marker: delims[closer].marker,
            count: delims[closer].remaining,
            can_open: delims[closer].can_open,
            can_close: delims[closer].can_close,
        });
    }

    // Replace the two runs and the content between them.
    let removed = closer_token - opener_token + 1;
    tokens.splice(opener_token..=closer_token, replacement.iter().cloned());
    let shift = replacement.len() as isize - removed as isize;
    for delim in delims.iter_mut() {
        if delim.token > closer_token {
            delim.token = (delim.token as isize + shift) as usize;
        }
    }

    // The delimiters that lie between the two runs become part of the
    // emphasis and may not be matched any more.
    let mut between = delims[opener].next;
    while let Some(between_index) = between {
        if between_index == closer {
            break;
        }
        let next = delims[between_index].next;
        remove_delimiter(delims, between_index);
        between = next;
    }

    // A run that has been used up completely leaves the stack.
    if delims[opener].remaining == 0 {
        remove_delimiter(delims, opener);
    }
    if delims[closer].remaining == 0 {
        let next = delims[closer].next;
        remove_delimiter(delims, closer);
        next
    } else {
        delims[closer].token = opener_token + replacement.len() - 1;
        Some(closer)
    }
}

/// Take a delimiter out of the stack.
fn remove_delimiter(delims: &mut [Delimiter], index: usize) {
    let previous = delims[index].previous;
    let next = delims[index].next;
    if let Some(previous_index) = previous {
        delims[previous_index].next = next;
    }
    if let Some(next_index) = next {
        delims[next_index].previous = previous;
    }
    delims[index].previous = None;
    delims[index].next = None;
    delims[index].remaining = 0;
}

/// Merge neighbouring text tokens into one.
///
/// The scanner emits the delimiter runs as separate tokens; a run that never
/// found a partner becomes literal text and is merged back into its
/// surroundings.
fn merge_adjacent_text(tokens: &mut Vec<Inline>) {
    let mut merged: Vec<Inline> = Vec::with_capacity(tokens.len());
    for token in std::mem::take(tokens) {
        // A run that never found a partner is literal text again.
        let token = match token {
            Inline::Delim { marker, count, .. } => Inline::Text {
                raw: marker.to_string().repeat(count),
            },
            other => other,
        };

        let mut merged_into_previous = false;
        if let Inline::Text { raw: more } = &token {
            if let Some(Inline::Text { raw }) = merged.last_mut() {
                raw.push_str(more);
                merged_into_previous = true;
            }
        }
        if !merged_into_previous {
            merged.push(token);
        }
    }
    *tokens = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(tokens: &[Inline]) -> Vec<&'static str> {
        tokens
            .iter()
            .map(|t| match t {
                Inline::Text { .. } => "text",
                Inline::Emphasis { .. } => "em",
                Inline::Strong { .. } => "strong",
                Inline::Del { .. } => "del",
                Inline::Mark { .. } => "mark",
                Inline::InlineCode { .. } => "code",
                Inline::InlineMath { .. } => "math",
                Inline::Emoji { .. } => "emoji",
                Inline::Link { .. } => "link",
                Inline::ReferenceLink { .. } => "reflink",
                Inline::Image { .. } => "image",
                Inline::ReferenceImage { .. } => "refimage",
                Inline::AutoLink { .. } => "autolink",
                Inline::AutoLinkExtension { .. } => "autolink_extension",
                Inline::HtmlTag { .. } => "html",
                Inline::HtmlEscape { .. } => "entity",
                Inline::Escape { .. } => "escape",
                Inline::HardBreak { .. } => "hardbreak",
                Inline::SoftBreak { .. } => "softbreak",
                Inline::Footnote { .. } => "footnote",
                Inline::Superscript { .. } => "sup",
                Inline::Subscript { .. } => "sub",
                Inline::Insert { .. } => "ins",
                Inline::Delete { .. } => "delete",
                Inline::Delim { .. } => "delim",
            })
            .collect()
    }

    #[test]
    fn parses_emphasis() {
        let tokens = parse("*foo*", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["em"]);
    }

    #[test]
    fn parses_strong() {
        let tokens = parse("**foo**", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["strong"]);
    }

    #[test]
    fn parses_superscript_and_subscript() {
        let tokens = parse("x^2^ H~2~O", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["text", "sup", "text", "sub", "text"]);
    }

    #[test]
    fn parses_both_directions_of_an_inline_diff() {
        let tokens = parse("{++new++} {--old--}", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["ins", "text", "delete"]);
    }

    #[test]
    fn scripts_are_off_when_the_option_is_off() {
        let options = InlineOptions {
            super_sub_script: false,
            ..InlineOptions::default()
        };
        let tokens = parse("x^2^ H~2~O", options);
        assert_eq!(kinds(&tokens), vec!["text"]);
    }

    #[test]
    fn parses_nested_emphasis() {
        let tokens = parse("**bold *and italic* here**", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["strong"]);
        match &tokens[0] {
            Inline::Strong { children, .. } => {
                assert_eq!(kinds(children), vec!["text", "em", "text"]);
            }
            _ => panic!("expected strong"),
        }
    }

    #[test]
    fn underscore_does_not_open_intraword() {
        let tokens = parse("snake_case_word", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["text"]);
    }

    #[test]
    fn parses_code_span() {
        let tokens = parse("`code`", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["code"]);
        match &tokens[0] {
            Inline::InlineCode { content, .. } => assert_eq!(content, "code"),
            _ => panic!("expected code"),
        }
    }

    #[test]
    fn parses_link() {
        let tokens = parse("[text](https://example.com)", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["link"]);
    }

    #[test]
    fn parses_image() {
        let tokens = parse("![alt](img.png)", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["image"]);
    }

    #[test]
    fn parses_autolink() {
        let tokens = parse("<https://example.com>", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["autolink"]);
    }

    #[test]
    fn parses_escape() {
        let tokens = parse(r"\*not emphasis\*", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["escape", "text", "escape"]);
    }

    #[test]
    fn parses_strikethrough() {
        let tokens = parse("~~gone~~", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["del"]);
    }

    #[test]
    fn parses_highlight() {
        let tokens = parse("==bright==", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["mark"]);
    }

    #[test]
    fn parses_inline_math() {
        let tokens = parse("$x^2$", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["math"]);
    }

    #[test]
    fn unclosed_emphasis_stays_text() {
        let tokens = parse("*not closed", InlineOptions::default());
        assert_eq!(kinds(&tokens), vec!["text"]);
    }

    #[test]
    fn a_break_carries_only_its_own_source() {
        // The break used to be built from `text_start`, which still pointed at
        // the beginning of the line: the token then carried the whole line,
        // and the preview drew the line twice.
        let soft = parse("alpha\nbeta", InlineOptions::default());
        assert_eq!(kinds(&soft), vec!["text", "softbreak", "text"]);
        match &soft[1] {
            Inline::SoftBreak { raw } => assert_eq!(raw, "\n"),
            other => panic!("expected a soft break, got {other:?}"),
        }

        let hard = parse("alpha  \nbeta", InlineOptions::default());
        match &hard[1] {
            Inline::HardBreak { raw } => assert_eq!(raw, "  \n"),
            other => panic!("expected a hard break, got {other:?}"),
        }
    }
}
