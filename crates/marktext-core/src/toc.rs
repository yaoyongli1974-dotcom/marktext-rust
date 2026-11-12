//! Table of contents: the outline of a document.
//!
//! Ported from MarkText's `packages/muya/src/state/getTOC.ts`.
//!
//! Two slugs are produced for every heading. The `slug` is stable within a
//! document, while the `github_slug` follows GitHub's anchor rules so an
//! exported document links the way GitHub renders it.

use std::collections::HashMap;

use crate::autolink::AutoLinkKind;
use crate::document::{Block, BlockKind};
use crate::inline::{self, Inline, InlineOptions};

/// One entry of the table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocItem {
    /// The heading depth, 1 through 6.
    pub level: u8,
    /// The heading text without any inline markup.
    pub content: String,
    /// The heading text as HTML.
    pub content_html: String,
    /// The anchor used to navigate inside the editor.
    pub slug: String,
    /// The anchor GitHub would generate for the same heading.
    pub github_slug: String,
}

/// Build the table of contents of a document.
///
/// `prefix` disambiguates the slugs when several documents share an outline,
/// which is how the desktop app keeps the navigation stable across tabs.
pub fn build_toc(blocks: &[Block], prefix: &str) -> Vec<TocItem> {
    let mut items = Vec::new();
    let mut slugs: HashMap<String, usize> = HashMap::new();
    let mut github_slugs: HashMap<String, usize> = HashMap::new();
    collect(blocks, prefix, &mut items, &mut slugs, &mut github_slugs);
    items
}

/// Collect the headings of a block list.
fn collect(
    blocks: &[Block],
    prefix: &str,
    items: &mut Vec<TocItem>,
    slugs: &mut HashMap<String, usize>,
    github_slugs: &mut HashMap<String, usize>,
) {
    for (index, block) in blocks.iter().enumerate() {
        match block.kind {
            BlockKind::AtxHeading | BlockKind::SetextHeading => {
                let level = block.level.unwrap_or(1).clamp(1, 6);
                let raw = heading_text(block);
                let content = heading_plain_text(block);
                let content_html = heading_html(&raw);

                let base = slugify(&content);
                let slug = unique(slugs, &base, prefix, index);
                let github_base = github_slugify(&content);
                let github_slug = unique(github_slugs, &github_base, prefix, index);

                items.push(TocItem {
                    level,
                    content,
                    content_html,
                    slug,
                    github_slug,
                });
            }
            _ => {}
        }
        if let Some(children) = &block.children {
            collect(children, prefix, items, slugs, github_slugs);
        }
    }
}

/// The raw text of a heading, without the `#` markers.
fn heading_text(block: &Block) -> String {
    let text = block.text();
    match block.kind {
        BlockKind::AtxHeading => {
            let hashes = text.chars().take_while(|c| *c == '#').count();
            text[hashes..].trim().to_string()
        }
        _ => text.trim().to_string(),
    }
}

/// The plain text of a heading, with the inline markup removed.
///
/// This is the same text the outline shows, so the preview can anchor a
/// heading by it and a click in the outline finds the heading even when it
/// contains emphasis, code or a link.
pub fn heading_plain_text(block: &Block) -> String {
    plain_text(&heading_text(block))
}

/// The plain text of a heading, with the inline markup removed.
fn plain_text(text: &str) -> String {
    let tokens = inline::parse(text, InlineOptions::default());
    let mut out = String::new();
    for token in &tokens {
        push_plain_text(token, &mut out);
    }
    out
}

/// Collect the plain text of an inline token.
fn push_plain_text(token: &Inline, out: &mut String) {
    match token {
        Inline::Text { raw } => out.push_str(raw),
        Inline::Emphasis { children, .. }
        | Inline::Strong { children, .. }
        | Inline::Del { children, .. }
        | Inline::Mark { children, .. } => {
            for child in children {
                push_plain_text(child, out);
            }
        }
        Inline::InlineCode { content, .. } => out.push_str(content),
        Inline::InlineMath { content, .. } => out.push_str(content),
        Inline::Emoji { content } => out.push_str(content),
        Inline::Link { children, .. } => {
            for child in children {
                push_plain_text(child, out);
            }
        }
        Inline::ReferenceLink { children, .. } => {
            for child in children {
                push_plain_text(child, out);
            }
        }
        Inline::Image { alt, .. } | Inline::ReferenceImage { alt, .. } => out.push_str(alt),
        Inline::AutoLink { href, .. } => out.push_str(href),
        Inline::AutoLinkExtension { kind, text } => match kind {
            AutoLinkKind::Www => out.push_str(&format!("http://{text}")),
            AutoLinkKind::Url => out.push_str(text),
            AutoLinkKind::Email => out.push_str(&format!("mailto:{text}")),
        },
        Inline::HtmlTag { .. }
        | Inline::HtmlEscape { .. }
        | Inline::Escape { .. }
        | Inline::HardBreak { .. }
        | Inline::SoftBreak { .. } => {}
        Inline::Footnote { identifier } => out.push_str(identifier),
        Inline::Superscript { content }
        | Inline::Subscript { content }
        | Inline::Insert { content }
        | Inline::Delete { content } => out.push_str(content),
        Inline::Delim { marker, count, .. } => {
            for _ in 0..*count {
                out.push(*marker);
            }
        }
    }
}

/// The HTML of a heading, with the inline markup rendered.
fn heading_html(text: &str) -> String {
    let options = crate::render::RenderOptions::default();
    crate::render::render_html(&[Block::leaf(BlockKind::Paragraph, text)], options)
        .trim()
        .trim_start_matches("<p>")
        .trim_end_matches("</p>")
        .trim_end_matches('\n')
        .to_string()
}

/// Turn a heading into an anchor slug.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_dash = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_was_dash = false;
        } else if ch == '_' || ch == '-' {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    trimmed.to_string()
}

/// Turn a heading into a GitHub-style anchor.
pub fn github_slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_dash = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Make a slug unique within a document by appending a counter.
fn unique(slugs: &mut HashMap<String, usize>, base: &str, prefix: &str, index: usize) -> String {
    let counter = slugs.entry(base.to_string()).or_insert(0);
    let slug = if *counter == 0 {
        base.to_string()
    } else {
        format!("{base}-{}", *counter)
    };
    *counter += 1;
    if prefix.is_empty() {
        slug
    } else {
        format!("{prefix}-{index}-{slug}")
    }
}
