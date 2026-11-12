//! HTML rendering: the block tree to a static HTML document.
//!
//! Ported from MarkText's `packages/muya/src/state/markdownToHtml.ts` and
//! `renderToStaticHTML.ts`.
//!
//! Code highlighting is delegated to the host: the renderer emits a
//! `language-*` class on the `<code>` element so a client-side highlighter
//! such as Prism can pick it up, which keeps the engine free of a JavaScript
//! dependency.

use crate::autolink::AutoLinkKind;
use crate::document::{Align, Block, BlockKind, CodeBlockKind, ListStyle, MathStyle};
use crate::inline::{self, Inline, InlineOptions, ReferenceDefinition};

/// Options that control how a document is rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Whether raw HTML from the document is kept as-is.
    pub allow_raw_html: bool,
    /// Whether the GFM `tagfilter` extension is applied.
    ///
    /// The extension removes the tags that can be used to smuggle script
    /// into a page by replacing the leading `<` of the offending tag with
    /// the entity `&lt;`.
    pub gfm_tagfilter: bool,
    /// Whether a `language-*` class is added to code blocks.
    pub highlight_code: bool,
    /// Whether math is wrapped in a KaTeX-friendly span.
    pub render_math: bool,
    /// The options used when parsing inline text.
    pub inline: InlineOptions,
    /// The link reference definitions of the document.
    ///
    /// The renderer needs them in order to resolve the reference links and
    /// the reference images.
    pub references: Vec<ReferenceDefinition>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            allow_raw_html: true,
            gfm_tagfilter: false,
            highlight_code: true,
            render_math: true,
            inline: InlineOptions::default(),
            references: Vec::new(),
        }
    }
}

/// Look up a link reference definition by its label.
fn find_reference<'a>(
    references: &'a [ReferenceDefinition],
    label: &str,
) -> Option<&'a ReferenceDefinition> {
    references
        .iter()
        .find(|definition| definition.label == label)
}

/// Render a block tree to a complete HTML document.
pub fn render_to_html(blocks: &[Block], options: RenderOptions) -> String {
    let mut body = String::new();
    render_blocks(blocks, &mut body, &options, false);
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n{body}</body>\n</html>\n"
    )
}

/// Render a block tree to an HTML fragment.
pub fn render_html(blocks: &[Block], options: RenderOptions) -> String {
    let mut body = String::new();
    render_blocks(blocks, &mut body, &options, false);
    body
}

/// Render sibling blocks into `out`.
///
/// `tight` says that the paragraphs are the direct children of a tight list
/// item, where the specification asks for the text without a `<p>` wrapper.
fn render_blocks(blocks: &[Block], out: &mut String, options: &RenderOptions, tight: bool) {
    for block in blocks {
        if tight && block.kind == BlockKind::Paragraph {
            // A tight list item holds its text directly, without a paragraph
            // and without the line break that follows it.
            render_inline(block.text(), out, options);
            continue;
        }
        cr(out);
        render_block(block, out, options, tight);
    }
}

/// Write a line break in front of a block that follows inline content.
///
/// The reference implementation keeps its markup unambiguous by putting a
/// line break in front of every block that does not start the output, so
/// that the text of a list item and the block that follows it end up on
/// lines of their own.
fn cr(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// Render one block into `out`.
fn render_block(block: &Block, out: &mut String, options: &RenderOptions, tight: bool) {
    match block.kind {
        BlockKind::Paragraph => {
            out.push_str("<p>");
            render_inline(block.text(), out, options);
            out.push_str("</p>\n");
        }
        BlockKind::AtxHeading => {
            let level = block.level.unwrap_or(1).clamp(1, 6);
            out.push_str(&format!("<h{level}>"));
            render_inline(atx_heading_content(block.text(), level), out, options);
            out.push_str(&format!("</h{level}>\n"));
        }
        BlockKind::SetextHeading => {
            let level = block.level.unwrap_or(1).clamp(1, 6);
            out.push_str(&format!("<h{level}>"));
            render_inline(block.text(), out, options);
            out.push_str(&format!("</h{level}>\n"));
        }
        BlockKind::ThematicBreak => out.push_str("<hr>\n"),
        BlockKind::CodeBlock => render_code_block(block, out, options),
        BlockKind::HtmlBlock => {
            if options.allow_raw_html {
                if options.gfm_tagfilter {
                    filter_html_block(block.text(), out);
                } else {
                    out.push_str(block.text());
                }
                out.push('\n');
            }
        }
        BlockKind::MathBlock => render_math_block(block, out, options),
        BlockKind::Diagram => render_diagram_block(block, out),
        BlockKind::BlockQuote => {
            out.push_str("<blockquote>\n");
            render_blocks(block.children(), out, options, false);
            out.push_str("</blockquote>\n");
        }
        BlockKind::GithubAlert => {
            let alert = block.alert_type.as_deref().unwrap_or("NOTE").to_lowercase();
            out.push_str(&format!(
                "<div class=\"markdown-alert markdown-alert-{alert}\">\n"
            ));
            render_blocks(block.children(), out, options, false);
            out.push_str("</div>\n");
        }
        BlockKind::BulletList => render_list(block, out, "ul", options),
        BlockKind::OrderList => render_list(block, out, "ol", options),
        BlockKind::TaskList => render_task_list(block, out, options),
        BlockKind::ListItem | BlockKind::TaskListItem => {
            out.push_str("<li>");
            render_blocks(block.children(), out, options, tight);
            out.push_str("</li>\n");
        }
        BlockKind::Table => render_table(block, out, options),
        BlockKind::TableRow | BlockKind::TableCell => {}
        BlockKind::Frontmatter => {
            let lang = escape_html_text(block.frontmatter_lang.as_deref().unwrap_or("yaml"));
            out.push_str(&format!(
                "<pre class=\"front-matter\" data-lang=\"{lang}\">"
            ));
            escape_html(block.text(), out);
            out.push_str("</pre>\n");
        }
        BlockKind::Footnote => {
            let identifier = block.identifier.as_deref().unwrap_or("");
            let identifier = escape_html_text(identifier);
            out.push_str(&format!(
                "<div class=\"footnote\" id=\"fn-{identifier}\">\n"
            ));
            render_blocks(block.children(), out, options, false);
            out.push_str("</div>\n");
        }
    }
}

/// Render a code block.
fn render_code_block(block: &Block, out: &mut String, options: &RenderOptions) {
    let info = crate::parser::clean_info_string(&block.lang.clone().unwrap_or_default());
    let language = info.split_whitespace().next().unwrap_or("").to_string();
    let class = if options.highlight_code && !language.is_empty() {
        format!(" class=\"language-{}\"", escape_html_text(&language))
    } else {
        String::new()
    };
    let kind = block.code_kind.unwrap_or(CodeBlockKind::Fenced);
    out.push_str(&format!("<pre><code{class}>"));
    // An indented code block keeps its four-space indentation out of the text.
    if kind == CodeBlockKind::Indented {
        for line in block.text().split('\n') {
            escape_html(line, out);
            out.push('\n');
        }
    } else {
        let text = block.text();
        escape_html(text, out);
        // The content of a fenced code block always ends with a line break,
        // unless the block is empty.
        if !text.is_empty() {
            out.push('\n');
        }
    }
    out.push_str("</code></pre>\n");
}

/// Render a display math block.
fn render_math_block(block: &Block, out: &mut String, options: &RenderOptions) {
    if !options.render_math {
        out.push_str("<pre>");
        escape_html(block.text(), out);
        out.push_str("</pre>\n");
        return;
    }
    let style = block.math_style.unwrap_or(MathStyle::Dollars);
    let class = match style {
        MathStyle::Gfm => "math math-display math-style-gfm",
        MathStyle::Dollars => "math math-display",
    };
    out.push_str(&format!("<pre class=\"{class}\">"));
    escape_html(block.text(), out);
    out.push_str("</pre>\n");
}

/// Render a diagram block as a placeholder for the host to fill in.
fn render_diagram_block(block: &Block, out: &mut String) {
    let diagram_type = block.diagram_type.as_deref().unwrap_or("mermaid");
    out.push_str(&format!(
        "<div class=\"mu-diagram\" data-type=\"{diagram_type}\">"
    ));
    escape_html(block.text(), out);
    out.push_str("</div>\n");
}

/// Render a bullet or ordered list.
fn render_list(block: &Block, out: &mut String, tag: &str, options: &RenderOptions) {
    let mut attributes = String::new();
    if tag == "ol" {
        let start = block.list.as_ref().and_then(|meta| meta.start).unwrap_or(1);
        if start != 1 {
            attributes.push_str(&format!(" start=\"{start}\""));
        }
        if let Some(style) = block.list.as_ref().and_then(|meta| meta.style) {
            if style != ListStyle::Decimal {
                attributes.push_str(&format!(" type=\"{}\"", ordered_list_type(style)));
            }
        }
    }
    out.push_str(&format!("<{tag}{attributes}>\n"));
    // A tight list holds the text of its items directly, without a `<p>`
    // wrapper around it.
    let tight = !block.list.as_ref().map(|meta| meta.loose).unwrap_or(false);
    for item in block.children() {
        render_block(item, out, options, tight);
    }
    out.push_str(&format!("</{tag}>\n"));
}

/// Render a task list, adding the checkbox inputs.
fn render_task_list(block: &Block, out: &mut String, options: &RenderOptions) {
    // A task list can be ordered (`1. [ ] a`), in which case the number and
    // style of the marker has to be kept just like for an ordinary list.
    let ordered = block.list.as_ref().and_then(|meta| meta.style).is_some();
    let tag = if ordered { "ol" } else { "ul" };
    let mut attributes = String::new();
    if ordered {
        let start = block.list.as_ref().and_then(|meta| meta.start).unwrap_or(1);
        if start != 1 {
            attributes.push_str(&format!(" start=\"{start}\""));
        }
        if let Some(style) = block.list.as_ref().and_then(|meta| meta.style) {
            if style != ListStyle::Decimal {
                attributes.push_str(&format!(" type=\"{}\"", ordered_list_type(style)));
            }
        }
    }

    out.push_str(&format!(
        "<{tag} class=\"contains-task-list\"{attributes}>\n"
    ));
    let tight = !block.list.as_ref().map(|meta| meta.loose).unwrap_or(false);
    for item in block.children() {
        out.push_str("<li class=\"task-list-item\">");
        let checked = item.checked == Some(true);
        out.push_str(&format!(
            "<input type=\"checkbox\" disabled{}> ",
            if checked { " checked" } else { "" }
        ));
        render_blocks(item.children(), out, options, tight);
        out.push_str("</li>\n");
    }
    out.push_str(&format!("</{tag}>\n"));
}

/// The text of an ATX heading without its markers.
///
/// The block keeps the line as it was written so that the document can be
/// serialized back byte for byte; the renderer needs the heading's content.
fn atx_heading_content(text: &str, level: u8) -> &str {
    let hashes = "#".repeat(level as usize);
    let Some(rest) = text.strip_prefix(hashes.as_str()) else {
        return text.trim();
    };
    // The content starts behind the whitespace that follows the marker.
    let rest = rest.trim_start();
    let trimmed = rest.trim_end();
    // An optional closing sequence of `#`s is not part of the content.  It
    // may be of any length, but it has to be preceded by a space or a tab —
    // or be the whole content, in which case the whitespace behind the
    // opening marker is what precedes it.
    if trimmed.ends_with('#') {
        let run = trimmed.len() - trimmed.trim_end_matches('#').len();
        let before = &trimmed[..trimmed.len() - run];
        if before.is_empty() || before.ends_with(' ') || before.ends_with('\t') {
            return before.trim_end();
        }
    }
    trimmed
}

/// Render a GFM table.
fn render_table(block: &Block, out: &mut String, options: &RenderOptions) {
    out.push_str("<table>\n");
    let rows = block.children();
    // The first row holds the header of the table, the rest are its body.
    if let Some(header) = rows.first() {
        out.push_str("<thead>\n");
        render_table_row(header, "th", out, options);
        out.push_str("</thead>\n");
    }
    // A table without a body has no `tbody` at all.
    if rows.len() > 1 {
        out.push_str("<tbody>\n");
        for row in &rows[1..] {
            render_table_row(row, "td", out, options);
        }
        out.push_str("</tbody>\n");
    }
    out.push_str("</table>\n");
}

/// Render one row of a GFM table.
fn render_table_row(row: &Block, tag: &str, out: &mut String, options: &RenderOptions) {
    out.push_str("<tr>\n");
    for cell in row.children() {
        let align = cell.align.unwrap_or(Align::None);
        let attribute = match align {
            Align::None => String::new(),
            Align::Left => " align=\"left\"".to_string(),
            Align::Center => " align=\"center\"".to_string(),
            Align::Right => " align=\"right\"".to_string(),
        };
        out.push_str(&format!("<{tag}{attribute}>"));
        render_inline(cell.text(), out, options);
        out.push_str(&format!("</{tag}>\n"));
    }
    out.push_str("</tr>\n");
}

/// The HTML `type` attribute of an ordered list.
fn ordered_list_type(style: ListStyle) -> &'static str {
    match style {
        ListStyle::LowerAlpha => "a",
        ListStyle::UpperAlpha => "A",
        ListStyle::LowerRoman => "i",
        ListStyle::UpperRoman => "I",
        _ => "1",
    }
}

/// Render inline text into `out`.
fn render_inline(text: &str, out: &mut String, options: &RenderOptions) {
    // The parser needs the definitions of the document to tell a reference
    // link from a plain bracket.
    let mut inline_options = options.inline.clone();
    inline_options.references = options.references.clone();
    let tokens = inline::parse(text, inline_options);
    for token in &tokens {
        render_inline_token(token, out, options);
    }
}

/// Render one inline token into `out`.
fn render_inline_token(token: &Inline, out: &mut String, options: &RenderOptions) {
    match token {
        Inline::Text { raw } => escape_html(raw, out),
        Inline::Emphasis { children, .. } => {
            out.push_str("<em>");
            render_inline_tokens(children, out, options);
            out.push_str("</em>");
        }
        Inline::Strong { children, .. } => {
            out.push_str("<strong>");
            render_inline_tokens(children, out, options);
            out.push_str("</strong>");
        }
        Inline::Del { children, .. } => {
            out.push_str("<del>");
            render_inline_tokens(children, out, options);
            out.push_str("</del>");
        }
        Inline::Mark { children, .. } => {
            out.push_str("<mark>");
            render_inline_tokens(children, out, options);
            out.push_str("</mark>");
        }
        Inline::InlineCode { content, .. } => {
            out.push_str("<code>");
            escape_html(content, out);
            out.push_str("</code>");
        }
        Inline::InlineMath { content, .. } => {
            if options.render_math {
                out.push_str("<span class=\"math math-inline\">");
                escape_html(content, out);
                out.push_str("</span>");
            } else {
                escape_html(content, out);
            }
        }
        Inline::Emoji { content } => {
            out.push_str(&format!(
                "<span class=\"emoji\">:{}:</span>",
                escape_html_text(content)
            ));
        }
        Inline::Link {
            href,
            title,
            children,
            ..
        } => {
            let title = title
                .as_ref()
                .map(|t| format!(" title=\"{}\"", escape_html_text(&resolve_references(t))))
                .unwrap_or_default();
            out.push_str(&format!("<a href=\"{}\"{title}>", escape_href(href)));
            render_inline_tokens(children, out, options);
            out.push_str("</a>");
        }
        Inline::ReferenceLink { label, children } => {
            match find_reference(&options.references, label) {
                Some(definition) => {
                    let title = definition
                        .title
                        .as_ref()
                        .map(|t| format!(" title=\"{}\"", escape_html_text(&resolve_references(t))))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "<a href=\"{}\"{title}>",
                        escape_href(&definition.href)
                    ));
                    render_inline_tokens(children, out, options);
                    out.push_str("</a>");
                }
                // A reference that no definition matches stays text.
                None => {
                    out.push('[');
                    render_inline_tokens(children, out, options);
                    out.push(']');
                }
            }
        }
        Inline::Image {
            src, title, alt, ..
        } => {
            let title = title
                .as_ref()
                .map(|t| format!(" title=\"{}\"", escape_html_text(t)))
                .unwrap_or_default();
            out.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\"{title}>",
                escape_html_text(src),
                escape_html_text(alt)
            ));
        }
        Inline::ReferenceImage { label, alt } => {
            match find_reference(&options.references, label) {
                Some(definition) => {
                    let title = definition
                        .title
                        .as_ref()
                        .map(|t| format!(" title=\"{}\"", escape_html_text(&resolve_references(t))))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "<img src=\"{}\" alt=\"{}\"{title}>",
                        escape_html_text(&definition.href),
                        escape_html_text(alt)
                    ));
                }
                // A reference that no definition matches stays text.
                None => {
                    out.push_str("![");
                    escape_html(alt, out);
                    out.push(']');
                }
            }
        }
        Inline::AutoLink { href, email } => {
            // The text of an autolink is the address without the `mailto:`
            // prefix that only the link target carries.
            let address = if *email {
                href.strip_prefix("mailto:").unwrap_or(href)
            } else {
                href.as_str()
            };
            let target = if *email && !href.starts_with("mailto:") {
                format!("mailto:{href}")
            } else {
                href.clone()
            };
            // An autolink is written as it is, so only the percent encoding
            // applies to it — no escape and no reference is resolved.
            out.push_str(&format!("<a href=\"{}\">", escape_destination(&target)));
            escape_html(address, out);
            out.push_str("</a>");
        }
        Inline::AutoLinkExtension { kind, text } => {
            // The extended autolink of GitHub Flavored Markdown: the text
            // of the link is the address itself, the target may carry a
            // scheme the text does not name.
            let target = match kind {
                AutoLinkKind::Www => format!("http://{text}"),
                AutoLinkKind::Url => text.clone(),
                AutoLinkKind::Email => format!("mailto:{text}"),
            };
            out.push_str(&format!("<a href=\"{}\">", escape_destination(&target)));
            escape_html(text, out);
            out.push_str("</a>");
        }
        Inline::HtmlTag { raw } => {
            if options.allow_raw_html {
                if options.gfm_tagfilter && is_filtered_tag(raw) {
                    out.push_str("&lt;");
                    out.push_str(&raw[1..]);
                } else {
                    out.push_str(raw);
                }
            } else {
                escape_html(raw, out);
            }
        }
        Inline::HtmlEscape { raw, decoded } => match decoded {
            // A known reference stands for the text it names.
            Some(text) => escape_html(text, out),
            // An unknown one stays the text it was written as.
            None => escape_html(raw, out),
        },
        Inline::Escape { content, .. } => escape_html(content, out),
        Inline::HardBreak { .. } => out.push_str("<br>\n"),
        Inline::SoftBreak { .. } => out.push('\n'),
        Inline::Footnote { identifier } => {
            let identifier = escape_html_text(identifier);
            out.push_str(&format!(
                "<sup class=\"footnote-ref\"><a href=\"#fn-{identifier}\" id=\"fnref-{identifier}\">{identifier}</a></sup>"
            ));
        }
        Inline::Delim { marker, count, .. } => {
            for _ in 0..*count {
                out.push(*marker);
            }
        }
    }
}

/// Render a list of inline tokens.
fn render_inline_tokens(tokens: &[Inline], out: &mut String, options: &RenderOptions) {
    for token in tokens {
        render_inline_token(token, out, options);
    }
}

/// Escape the text of a string for use in an HTML attribute.
pub fn escape_html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Whether a character may stay as it is inside a link destination.
fn is_safe_link_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '_'
                | '.'
                | '+'
                | '@'
                | '#'
                | '%'
                | '/'
                | '?'
                | '&'
                | '='
                | ':'
                | ';'
                | ','
                | '$'
                | '!'
                | '*'
                | '('
                | ')'
                | '~'
                | '\''
        )
}

/// Write the bytes of a character as percent encoded triplets.
fn write_percent_encoded(ch: char, out: &mut String) {
    let mut buffer = [0u8; 4];
    for byte in ch.encode_utf8(&mut buffer).as_bytes() {
        out.push('%');
        out.push_str(&format!("{byte:02X}"));
    }
}

/// Resolve the backslash escapes and character references of a destination.
fn resolve_destination(text: &str) -> String {
    resolve_references(text)
}

/// Resolve the backslash escapes and the character references of a text.
///
/// A backslash in front of an ASCII punctuation character stands for that
/// character, and a character reference stands for the character it names.
pub fn resolve_references(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 1 < bytes.len() && bytes[index + 1].is_ascii_punctuation() => {
                out.push(bytes[index + 1] as char);
                index += 2;
            }
            b'&' => match crate::entities::decode_reference(&text[index..]) {
                Some((length, decoded)) => {
                    out.push_str(&decoded);
                    index += length;
                }
                None => {
                    out.push('&');
                    index += 1;
                }
            },
            _ => {
                let ch = text[index..].chars().next().unwrap_or('\u{FFFD}');
                out.push(ch);
                index += ch.len_utf8();
            }
        }
    }
    out
}

/// Escape a link destination for use in an `href` attribute.
///
/// The backslash escapes and the character references of the destination are
/// resolved first, then every character that may not appear in a URL is
/// percent encoded.
pub fn escape_href(text: &str) -> String {
    escape_destination(&resolve_destination(text))
}

/// Escape a destination that was written without backslash escapes.
fn escape_destination(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if is_safe_link_char(ch) {
            match ch {
                '&' => out.push_str("&amp;"),
                '\'' => out.push_str("&#x27;"),
                _ => out.push(ch),
            }
        } else {
            write_percent_encoded(ch, &mut out);
        }
    }
    out
}

/// Escape text for use in element content.
fn escape_html(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // The reference implementation escapes the double quote in text
            // content as well, so the specification examples match.
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// The HTML tags that the GFM `tagfilter` extension removes.
///
/// They are the tags that can carry script into a page or that swallow the
/// rest of the document.
const FILTERED_TAGS: [&str; 9] = [
    "title",
    "textarea",
    "style",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "script",
    "plaintext",
];

/// Whether the text opens or closes one of the filtered tags.
///
/// The text is the remainder of the document starting at the `<` that is
/// being tested, exactly as the reference implementation passes it.  The
/// tag name is matched case insensitively and has to be followed by a
/// delimiter, so `<scripture>` is left alone while `<SCRIPT>` is not.
fn is_filtered_tag(text: &str) -> bool {
    let bytes = text.as_bytes();
    // A tag needs at least a `<` and one character of the name.
    if bytes.len() < 3 || bytes[0] != b'<' {
        return false;
    }
    let mut index = 1;
    if bytes[index] == b'/' {
        index += 1;
    }
    for expected in FILTERED_TAGS {
        if tag_name_matches(bytes, index, expected) {
            return true;
        }
    }
    false
}

/// Whether the tag name that starts at `index` is `name` and is followed by
/// a delimiter.
fn tag_name_matches(bytes: &[u8], index: usize, name: &str) -> bool {
    let mut cursor = index;
    for expected in name.bytes() {
        if cursor >= bytes.len() {
            return false;
        }
        if bytes[cursor].to_ascii_lowercase() != expected {
            return false;
        }
        cursor += 1;
    }
    if cursor >= bytes.len() {
        // The name runs to the end of the text without a delimiter.
        return false;
    }
    let delimiter = bytes[cursor];
    if delimiter == b'>' || is_html_space(delimiter) {
        return true;
    }
    // A self closing tag such as `<script/>`.
    delimiter == b'/' && cursor + 1 < bytes.len() && bytes[cursor + 1] == b'>'
}

/// Whether the byte is one of the spaces the HTML syntax recognises.
fn is_html_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r')
}

/// Apply the `tagfilter` extension to a raw HTML block.
///
/// Every `<` of the block is looked at on its own: when it opens one of the
/// filtered tags the `<` is written as `&lt;`, otherwise it is written as
/// it is.  The rest of the block is never touched, so the tags that stay
/// allowed keep their meaning.
fn filter_html_block(text: &str, out: &mut String) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            if is_filtered_tag(&text[index..]) {
                out.push_str("&lt;");
            } else {
                out.push('<');
            }
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index] != b'<' {
            index += 1;
        }
        out.push_str(&text[start..index]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    /// Render a document with the `tagfilter` extension switched on.
    fn filtered(markdown: &str) -> String {
        let blocks = parse(markdown).expect("the document must parse");
        let options = RenderOptions {
            gfm_tagfilter: true,
            ..RenderOptions::default()
        };
        render_html(&blocks, options)
    }

    #[test]
    fn keeps_the_tags_that_are_allowed() {
        assert_eq!(filtered("<em>x</em>"), "<p><em>x</em></p>\n");
    }

    #[test]
    fn filters_the_disallowed_tags_of_a_paragraph() {
        assert_eq!(
            filtered("a <title> b <style> c"),
            "<p>a &lt;title> b &lt;style> c</p>\n"
        );
    }

    #[test]
    fn filters_the_disallowed_tags_case_insensitively() {
        assert_eq!(filtered("a <XMP> b"), "<p>a &lt;XMP> b</p>\n");
    }

    #[test]
    fn leaves_a_tag_with_a_longer_name_alone() {
        assert_eq!(filtered("a <scripture> b"), "<p>a <scripture> b</p>\n");
    }

    #[test]
    fn filters_the_disallowed_tags_inside_a_html_block() {
        // The block holds a `<blockquote>`, which is allowed, and a `<xmp>`,
        // which is not.
        assert_eq!(
            filtered("<blockquote>\n  <xmp> text\n</blockquote>\n"),
            "<blockquote>\n  &lt;xmp> text\n</blockquote>\n"
        );
    }
}
