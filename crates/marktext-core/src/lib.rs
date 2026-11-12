//! Core engine of the MarkText Rust port.
//!
//! The engine mirrors the architecture of MarkText's `packages/muya` TypeScript
//! package: a document is parsed into a tree of *blocks*, each block holding
//! raw inline text, and every block remembers enough of its original spelling
//! to serialize back to byte-identical Markdown.

pub mod autolink;
pub mod casefold;
pub mod document;
pub mod entities;
pub mod inline;
pub mod list_marker;
pub mod parser;
pub mod render;
pub mod search;
pub mod serializer;
pub mod toc;
mod unicode;

pub use document::{
    Align, Block, BlockKind, CodeBlockKind, Document, FenceChar, ListDelimiter, ListMeta,
    ListStyle, MathStyle,
};
pub use inline::{Inline, InlineOptions, ReferenceDefinition};
pub use parser::{
    clean_info_string, parse, parse_document_with_options, parse_with_options, ParseError,
    ParseOptions,
};
pub use render::{render_html, render_to_html, RenderOptions};
pub use search::{search_document, SearchError, SearchMatch, SearchOptions};
pub use serializer::{serialize, serialize_document, ExportOptions};
pub use toc::{build_toc, heading_plain_text, TocItem};

/// Parse a Markdown document and keep the source it came from.
pub fn parse_document(markdown: &str) -> Result<Document, ParseError> {
    parse_document_with_options(markdown, ParseOptions::default())
}
