//! Document model: the block tree that mirrors MarkText's `TState`.
//!
//! MarkText keeps a document as a tree of blocks where every leaf stores the
//! *raw* inline Markdown of that block rather than a parsed representation.
//! The inline syntax is parsed lazily, on demand, for rendering only. That
//! split is what makes an unmodified round trip possible, so the Rust port
//! keeps it: blocks own raw text, and [`crate::inline`] owns tokenization.

use crate::inline::ReferenceDefinition;
use serde::{Deserialize, Serialize};

/// Horizontal alignment of a table cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    None,
    Left,
    Center,
    Right,
}

impl Align {
    /// The GFM alignment marker used in the delimiter row.
    pub fn marker(self) -> &'static str {
        match self {
            Align::None => "none",
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}

/// Whether a code block was authored with a fence or with indentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeBlockKind {
    Indented,
    Fenced,
}

/// The character used to fence a code block. CommonMark §4.5 distinguishes
/// backtick fences from tilde fences, and the distinction survives a round
/// trip because it changes how the block re-parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FenceChar {
    Backtick,
    Tilde,
}

impl FenceChar {
    pub fn as_char(self) -> char {
        match self {
            FenceChar::Backtick => '`',
            FenceChar::Tilde => '~',
        }
    }
}

/// How a math block was authored: `$$` fences or a ```math fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MathStyle {
    /// `$$ ... $$`
    Dollars,
    /// ```math ... ```
    Gfm,
}

/// Numbering style of an ordered list, following pandoc's `fancy_lists`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum ListStyle {
    /// Pandoc's `DefaultStyle`, written as `#.` and numbered like decimal.
    Default,
    #[default]
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

/// The character(s) that terminate an ordered-list marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ListDelimiter {
    /// `1.`
    #[default]
    Dot,
    /// `1)`
    Paren,
    /// `(1)`
    ParenBoth,
}

impl ListDelimiter {
    pub fn as_str(self) -> &'static str {
        match self {
            ListDelimiter::Dot => ".",
            ListDelimiter::Paren => ")",
            ListDelimiter::ParenBoth => "()",
        }
    }
}

/// Metadata shared by every list flavour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ListMeta {
    /// A loose list renders its items as separate paragraphs.
    pub loose: bool,
    /// For unordered lists: the bullet character (`-`, `+` or `*`).
    pub marker: Option<String>,
    /// For ordered lists: the number the list starts at.
    pub start: Option<u64>,
    /// For ordered lists: the numbering style.
    pub style: Option<ListStyle>,
    /// For ordered lists: the marker that terminates each item.
    pub delimiter: Option<ListDelimiter>,
    /// Original item markers, kept so an unmodified document re-serializes with
    /// the author's own spelling (`1.`, `007.`, `10)`) instead of a normalized one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_markers: Option<Vec<String>>,
}

/// The kind of a block, without its payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "name", rename_all = "kebab-case")]
pub enum BlockKind {
    Paragraph,
    AtxHeading,
    SetextHeading,
    ThematicBreak,
    CodeBlock,
    HtmlBlock,
    BlockQuote,
    GithubAlert,
    ListItem,
    TaskListItem,
    OrderList,
    BulletList,
    TaskList,
    Table,
    TableRow,
    TableCell,
    MathBlock,
    Frontmatter,
    Diagram,
    Footnote,
}

/// One node of the document tree.
///
/// Leaves carry `text` (the raw inline Markdown); containers carry
/// `children`. The split mirrors MarkText's leaf/container state union, but a
/// single struct keeps the serializer's dispatch simple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    /// Raw inline text for leaf blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Child blocks for container blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Block>>,
    /// Heading level, for the two heading kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Bullet character, for `bullet-list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    /// Whether the code block used a fence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_kind: Option<CodeBlockKind>,
    /// Info string of a fenced code block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Fence character of a fenced code block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fence_char: Option<FenceChar>,
    /// Fence length of a fenced code block, when it exceeds the minimum of 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fence_length: Option<usize>,
    /// List metadata, for the three list kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<ListMeta>,
    /// Setext underline, e.g. `===` or `---`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    /// Cell alignment, for `table.cell`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// Checkbox state, for `task-list-item`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Ordered-list marker of this specific item, e.g. `007.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_marker: Option<String>,
    /// Math block style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math_style: Option<MathStyle>,
    /// Frontmatter language (`yaml`, `toml`, `json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_lang: Option<String>,
    /// Frontmatter fence style (`-`, `+`, `;`, `{`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_style: Option<String>,
    /// Diagram language (`mermaid`, `plantuml`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram_lang: Option<String>,
    /// Diagram type as written in the fence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram_type: Option<String>,
    /// Footnote identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// GitHub alert type (`note`, `tip`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_type: Option<String>,
    /// Original alert marker, so a non-canonical spelling survives a round trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_marker: Option<String>,
    /// Whether a blank line followed the alert marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blank_line_after_marker: Option<bool>,
    /// Whether a blank line follows the block.
    ///
    /// The flag decides whether the list that holds the block is loose, which
    /// is the rule the reference implementation uses: a list is loose when a
    /// blank line separates two of the blocks it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_line_blank: Option<bool>,
}

impl Block {
    /// Create a leaf block holding raw text.
    pub fn leaf(kind: BlockKind, text: impl Into<String>) -> Self {
        Block {
            kind,
            text: Some(text.into()),
            children: None,
            level: None,
            marker: None,
            code_kind: None,
            lang: None,
            fence_char: None,
            fence_length: None,
            list: None,
            underline: None,
            align: None,
            checked: None,
            order_marker: None,
            math_style: None,
            frontmatter_lang: None,
            frontmatter_style: None,
            diagram_lang: None,
            diagram_type: None,
            identifier: None,
            alert_type: None,
            alert_marker: None,
            blank_line_after_marker: None,
            last_line_blank: None,
        }
    }

    /// Create a container block holding children.
    pub fn container(kind: BlockKind, children: Vec<Block>) -> Self {
        Block {
            children: Some(children),
            text: None,
            ..Block::leaf(kind, "")
        }
    }

    /// The raw text of a leaf block.
    pub fn text(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }

    /// The children of a container block.
    pub fn children(&self) -> &[Block] {
        self.children.as_deref().unwrap_or(&[])
    }

    /// The children of a container block, mutably.
    pub fn children_mut(&mut self) -> &mut [Block] {
        match self.children.as_mut() {
            Some(children) => children.as_mut_slice(),
            None => &mut [],
        }
    }

    /// Whether this block is a list of any flavour.
    pub fn is_list(&self) -> bool {
        matches!(
            self.kind,
            BlockKind::OrderList | BlockKind::BulletList | BlockKind::TaskList
        )
    }

    /// Whether this block is a list item of any flavour.
    pub fn is_list_item(&self) -> bool {
        matches!(self.kind, BlockKind::ListItem | BlockKind::TaskListItem)
    }
}

/// A parsed document: the root block list plus the source it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// The root blocks.
    pub blocks: Vec<Block>,
    /// The original Markdown source.
    pub source: String,
    /// The link reference definitions the document holds.
    pub references: Vec<ReferenceDefinition>,
}

impl Document {
    /// All headings in the document, in document order, with their depth.
    pub fn headings(&self) -> Vec<(u8, &Block)> {
        let mut out = Vec::new();
        collect_headings(&self.blocks, &mut out);
        out
    }
}

fn collect_headings<'a>(blocks: &'a [Block], out: &mut Vec<(u8, &'a Block)>) {
    for block in blocks {
        match block.kind {
            BlockKind::AtxHeading | BlockKind::SetextHeading => {
                out.push((block.level.unwrap_or(1), block));
            }
            _ => {}
        }
        if let Some(children) = &block.children {
            collect_headings(children, out);
        }
    }
}
