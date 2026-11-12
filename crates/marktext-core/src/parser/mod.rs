//! Block-level parsing: Markdown source into the block tree.
//!
//! Ported from MarkText's `packages/muya/src/utils/marked` (a fork of
//! `marked`) plus `packages/muya/src/state/markdownToState.ts`.
//!
//! The parser is line-oriented. It walks the source line by line and decides
//! the block kind from the line's shape, then collects the block's lines until
//! the block ends. Every leaf keeps its *raw* text, so a document that is
//! parsed and re-serialized without edits comes back byte-identical.

use crate::document::{
    Align, Block, BlockKind, CodeBlockKind, Document, FenceChar, ListDelimiter, ListMeta,
    ListStyle, MathStyle,
};
use crate::inline::{normalize_label, ReferenceDefinition};
use crate::list_marker::parse_list_marker;
use serde::{Deserialize, Serialize};

/// Errors that can occur while parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseError {
    /// The source contained a NUL byte, which the parser cannot represent.
    NullByte { offset: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NullByte { offset } => {
                write!(f, "source contains a NUL byte at offset {offset}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Feature switches for the parser, mirroring MarkText's lexer options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseOptions {
    /// Parse `[^id]` footnote references and definitions.
    pub footnote: bool,
    /// Parse `$...$` inline math.
    pub tex_math_dollars: bool,
    /// Parse GitHub's `` $`...`$ `` inline math.
    pub tex_math_gfm: bool,
    /// Parse `\[...\]` and `\(...\)` inline math.
    pub tex_math_single_backslash: bool,
    /// Parse `\\[...\\]` and `\\(...\\)` inline math.
    pub tex_math_double_backslash: bool,
    /// Parse frontmatter at the top of the document.
    pub frontmatter: bool,
    /// Parse `> [!NOTE]` style GitHub alerts.
    pub github_alerts: bool,
    /// Support pandoc-style fancy ordered-list numbering.
    pub fancy_lists: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            footnote: false,
            tex_math_dollars: true,
            tex_math_gfm: false,
            tex_math_single_backslash: false,
            tex_math_double_backslash: false,
            frontmatter: true,
            github_alerts: false,
            fancy_lists: false,
        }
    }
}

/// Parse a Markdown document into its block tree.
pub fn parse(markdown: &str) -> Result<Vec<Block>, ParseError> {
    parse_with_options(markdown, ParseOptions::default())
}

/// Parse a Markdown document with explicit feature switches.
pub fn parse_with_options(markdown: &str, options: ParseOptions) -> Result<Vec<Block>, ParseError> {
    Ok(parse_document_with_options(markdown, options)?.blocks)
}

/// Parse a Markdown document with explicit feature switches.
///
/// Unlike [`parse_with_options`] this also returns the link reference
/// definitions the document holds, which the renderer needs in order to
/// resolve the reference links.
pub fn parse_document_with_options(
    markdown: &str,
    options: ParseOptions,
) -> Result<Document, ParseError> {
    if let Some(offset) = markdown.find('\0') {
        return Err(ParseError::NullByte { offset });
    }

    let lines: Vec<&str> = markdown.split('\n').collect();
    let bases = vec![0usize; lines.len()];
    let mut parser = BlockParser {
        lines,
        bases,
        pos: 0,
        options,
        definitions: Vec::new(),
        depth: 0,
    };

    let mut blocks = Vec::new();
    if options.frontmatter {
        parser.parse_frontmatter(&mut blocks);
    }
    parser.parse_blocks(&mut blocks);

    Ok(Document {
        blocks,
        source: markdown.to_string(),
        references: parser.definitions,
    })
}

/// A line of the source, split into its indentation and its content.
#[derive(Clone, Copy)]
struct Line<'a> {
    /// The line as it was written.
    raw: &'a str,
    /// The line without its leading whitespace.
    content: &'a str,
    /// The indentation width, counting a tab as 4 columns.
    width: usize,
    /// The column of the document the line starts at.
    ///
    /// The column matters because a tab reaches to the next multiple of
    /// four: the width of a tab depends on the column it starts in, and
    /// that column is not the same for every line of a nested fragment.
    base: usize,
}

impl<'a> Line<'a> {
    /// The number of columns the line is indented by.
    fn indent_width(&self) -> usize {
        self.width
    }
}

/// A line of a nested fragment together with the column it starts at.
#[derive(Clone)]
struct FragmentLine {
    /// The text of the line.
    text: String,
    /// The column of the document the line starts at.
    base: usize,
}

impl FragmentLine {
    /// A line that starts at the beginning of a line.
    fn plain(text: &str) -> Self {
        FragmentLine {
            text: text.to_string(),
            base: 0,
        }
    }
}

/// View a fragment line as a line of the parser.
///
/// The fragment owns its text; the returned line borrows it for the duration
/// of the call, which is enough to ask [`line_starts_block`] about it.
fn line_of_fragment(fragment: &FragmentLine) -> Line<'_> {
    let trimmed = fragment.text.trim_start_matches([' ', '\t']);
    let indent = &fragment.text[..fragment.text.len() - trimmed.len()];
    Line {
        raw: &fragment.text,
        content: trimmed,
        width: tab_to_columns(indent, fragment.base),
        base: fragment.base,
    }
}

/// Remove the marker of a block quote from a line.
///
/// The marker is the `>` together with the single column of whitespace that
/// may follow it.  A tab behind the marker is only used up by one column,
/// so the rest of it stays with the content: the reference implementation
/// keeps the remainder of the tab as the indentation of the content, and
/// the column it starts in decides how wide the tab is.
fn strip_quote_marker(line: &Line<'_>) -> FragmentLine {
    let rest = &line.content[1..];
    // The column the `>` stands in.
    let marker_column = line.base + line.width;
    if let Some(after) = rest.strip_prefix(' ') {
        return FragmentLine {
            text: after.to_string(),
            base: marker_column + 2,
        };
    }
    if let Some(after) = rest.strip_prefix('\t') {
        // The column the tab starts in decides how wide it is.
        let tab_column = marker_column + 1;
        let width = 4 - (tab_column % 4);
        if width == 1 {
            // The tab is one column wide, so it is used up completely.
            return FragmentLine {
                text: after.to_string(),
                base: tab_column + 1,
            };
        }
        // Only one column of the tab is used up.  The columns that are left
        // become the indentation of the content.
        let mut text = " ".repeat(width - 1);
        text.push_str(after);
        return FragmentLine {
            text,
            base: tab_column + 1,
        };
    }
    FragmentLine {
        text: rest.to_string(),
        base: marker_column + 1,
    }
}

struct BlockParser<'a> {
    lines: Vec<&'a str>,
    /// The column every line starts at.
    ///
    /// The lines of the document itself start at column zero; the lines of a
    /// nested fragment start at the column their container left them at.
    bases: Vec<usize>,
    pos: usize,
    options: ParseOptions,
    /// The link reference definitions the document holds.
    definitions: Vec<ReferenceDefinition>,
    /// How deeply this fragment is nested in quotes and lists.
    depth: usize,
}

/// The deepest nesting of quotes and lists that is parsed as structure.
///
/// Each level recurses through the block parser, the renderer, the serializer
/// and the outline builder; without a limit a source of a few hundred nested
/// `>` markers overflows the stack and aborts the process (the release profile
/// uses `panic = "abort"`). Deeper content is kept as a paragraph instead, so
/// the text is not lost.
///
/// The limit has to leave room for the debug-build frames on the smallest
/// stack the code runs on, which is the Windows test worker: at 100 levels it
/// still overflowed there, so 32 leaves a wide margin.
const MAX_NESTING: usize = 32;

/// Expand tabs to the next multiple of four, as CommonMark requires.
///
/// The text starts at the column `base` of the document, which decides
/// where the tab stops are.
fn tab_to_columns(text: &str, base: usize) -> usize {
    let mut column = base;
    for ch in text.chars() {
        if ch == '\t' {
            column += 4 - (column % 4);
        } else {
            column += 1;
        }
    }
    column - base
}

/// Remove the first `columns` columns of leading whitespace from `text`.
///
/// The text starts at the column `base` of the document.  A tab that
/// straddles the boundary is split: the part that falls inside the removed
/// columns is dropped and the part that falls outside becomes spaces, which
/// is what the reference implementation does when it strips the indentation
/// of an indented code block.
fn strip_columns(text: &str, base: usize, columns: usize) -> String {
    let mut column = base;
    let mut removed = 0usize;
    let mut index = 0usize;
    let mut out = String::new();

    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        match ch {
            ' ' => {
                if removed + 1 > columns {
                    break;
                }
                removed += 1;
                column += 1;
                index += 1;
            }
            '\t' => {
                let advance = 4 - (column % 4);
                if removed + advance > columns {
                    // The tab reaches past the boundary, so the columns that
                    // stay behind it become spaces.
                    let keep = advance - (columns - removed);
                    out.push_str(&" ".repeat(keep));
                    removed = columns;
                    column += advance;
                    index += 1;
                    continue;
                }
                removed += advance;
                column += advance;
                index += 1;
            }
            _ => break,
        }
    }

    out.push_str(&text[index..]);
    out
}

impl<'a> BlockParser<'a> {
    /// Split the next line into indent and content, without consuming it.
    fn peek_line(&self) -> Option<Line<'a>> {
        let raw = *self.lines.get(self.pos)?;
        let trimmed = raw.trim_start_matches([' ', '\t']);
        let indent = &raw[..raw.len() - trimmed.len()];
        Some(Line {
            raw,
            content: trimmed,
            width: tab_to_columns(indent, self.bases[self.pos]),
            base: self.bases[self.pos],
        })
    }

    /// Consume the next line, returning its indent and content.
    fn next_line(&mut self) -> Option<Line<'a>> {
        let line = self.peek_line()?;
        self.pos += 1;
        Some(line)
    }

    /// The next line that holds content, skipping blank lines.
    fn peek_content_line(&self) -> Option<Line<'a>> {
        let mut index = self.pos;
        while index < self.lines.len() {
            let raw = self.lines[index];
            let trimmed = raw.trim_start_matches([' ', '\t']);
            if trimmed.is_empty() {
                index += 1;
                continue;
            }
            let indent = &raw[..raw.len() - trimmed.len()];
            return Some(Line {
                raw,
                content: trimmed,
                width: tab_to_columns(indent, self.bases[index]),
                base: self.bases[index],
            });
        }
        None
    }

    /// True when the source is exhausted.
    fn at_end(&self) -> bool {
        self.pos >= self.lines.len()
    }

    /// Parse a sequence of blocks until the source ends or a block of a
    /// containing context begins.
    fn parse_blocks(&mut self, out: &mut Vec<Block>) {
        loop {
            // A blank line between two blocks decides whether the list that
            // holds them is loose, so the block before it is remembered.
            if self.skip_blank_lines() {
                if let Some(previous) = out.last_mut() {
                    mark_blank_line_after(previous);
                }
            }
            if self.at_end() {
                break;
            }
            let Some(line) = self.peek_line() else { break };

            if line.content.is_empty() {
                break;
            }

            if let Some(block) = self.parse_frontmatter_block() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_math_block() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_fenced_code() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_atx_heading() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_thematic_break() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_block_quote() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_list() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_table() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_html_block() {
                out.push(block);
                continue;
            }
            // A footnote definition also starts with `[`, and its label starts
            // with `^`; it has to be tried before the link reference
            // definition, which would otherwise swallow it as a definition for
            // the label `^1`.
            if let Some(block) = self.parse_footnote_definition() {
                out.push(block);
                continue;
            }
            // A link reference definition produces no block of its own.
            //
            // It is tried before the setext heading on purpose: the line
            // `[foo]: /url` followed by `===` is a definition and a
            // paragraph, not a heading.
            if self.parse_link_reference_definition() {
                continue;
            }
            if let Some(block) = self.parse_setext_heading() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_indented_code() {
                out.push(block);
                continue;
            }
            if let Some(block) = self.parse_paragraph() {
                out.push(block);
                continue;
            }

            // Nothing matched; consume the line so the loop always advances.
            self.next_line();
        }
    }

    /// Skip lines that hold only whitespace.
    ///
    /// The function reports whether it skipped at least one line.
    fn skip_blank_lines(&mut self) -> bool {
        let mut skipped = false;
        while let Some(line) = self.peek_line() {
            if line.content.is_empty() {
                self.pos += 1;
                skipped = true;
            } else {
                break;
            }
        }
        skipped
    }

    /// Parse frontmatter, which must be the first block of the document.
    fn parse_frontmatter(&mut self, out: &mut Vec<Block>) {
        if let Some(block) = self.parse_frontmatter_block() {
            out.push(block);
        }
    }

    /// Parse a frontmatter block if the source starts with one.
    ///
    /// The fences are `---` (YAML), `+++` (TOML), `;;;` (JSON) and `{`/`}` (JSON).
    fn parse_frontmatter_block(&mut self) -> Option<Block> {
        // Frontmatter only opens a document; a `---` further down is a
        // thematic break or a setext underline.
        if self.pos != 0 {
            return None;
        }
        // Frontmatter is an extension of MarkText, so a document that does
        // not want it never opens one — the `---` then stays a thematic
        // break.
        if !self.options.frontmatter {
            return None;
        }
        let first = self.peek_line()?;
        if first.indent_width() != 0 {
            return None;
        }
        let (style, lang, closer) = match first.content {
            "---" => ("-", "yaml", "---"),
            "+++" => ("+", "toml", "+++"),
            ";;;" => (";", "json", ";;;"),
            "{" => ("{", "json", "}"),
            _ => return None,
        };
        // A `{` opener only counts when the block closes with `}`.
        if style == "{" {
            return None;
        }

        self.next_line();
        let mut body = Vec::new();
        loop {
            let Some(line) = self.next_line() else {
                // Unterminated frontmatter is not frontmatter.
                self.pos -= 1 + body.len();
                return None;
            };
            if line.content == closer {
                break;
            }
            body.push(line.content);
        }

        let mut block = Block::leaf(BlockKind::Frontmatter, body.join("\n"));
        block.frontmatter_lang = Some(lang.to_string());
        block.frontmatter_style = Some(style.to_string());
        Some(block)
    }

    /// Parse a display-math block (`$$ ... $$`).
    fn parse_math_block(&mut self) -> Option<Block> {
        if !self.options.tex_math_dollars {
            return None;
        }
        let line = self.peek_line()?;
        if line.indent_width() != 0 || line.content != "$$" {
            return None;
        }
        self.next_line();
        let mut body = Vec::new();
        loop {
            let Some(line) = self.next_line() else {
                self.pos -= 1 + body.len();
                return None;
            };
            if line.content == "$$" {
                break;
            }
            body.push(line.content);
        }
        let mut block = Block::leaf(BlockKind::MathBlock, body.join("\n"));
        block.math_style = Some(MathStyle::Dollars);
        Some(block)
    }

    /// Parse a fenced code block, or a diagram block when the info string names
    /// a diagram language.
    fn parse_fenced_code(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 {
            return None;
        }
        let content = line.content;
        let fence_char = if content.starts_with("```") {
            FenceChar::Backtick
        } else if content.starts_with("~~~") {
            FenceChar::Tilde
        } else {
            return None;
        };
        let fence_len = content
            .chars()
            .take_while(|c| *c == fence_char.as_char())
            .count();
        if fence_len < 3 {
            return None;
        }
        // The info string is kept as it was written, so that a document that
        // is parsed and written back comes out unchanged.  A backtick fence's
        // info string may not contain a backtick.
        let info = content[fence_len..].trim().to_string();
        if fence_char == FenceChar::Backtick && info.contains('`') {
            return None;
        }

        // The indentation of the opening fence is stripped from the content
        // of the block.
        let fence_indent = line.width;
        self.next_line();
        let mut body = Vec::new();
        let mut closed = false;
        while let Some(line) = self.next_line() {
            let trimmed = line.content.trim();
            let is_closing = line.indent_width() < 4
                && trimmed.len() >= fence_len
                && trimmed.chars().all(|c| c == fence_char.as_char());
            if is_closing {
                closed = true;
                break;
            }
            body.push(strip_columns(
                self.lines[self.pos - 1],
                self.bases[self.pos - 1],
                fence_indent,
            ));
        }
        // A fence that is never closed reaches to the end of the document.
        // The final line ending of the document does not open another line,
        // so an empty last line is not part of the block.
        if !closed
            && body.last().is_some_and(|line| line.is_empty())
            && self.pos >= self.lines.len()
        {
            body.pop();
        }

        let text = body.join("\n");
        let language = clean_info_string(&info);
        let language = language.split_whitespace().next().unwrap_or("").to_string();
        if let Some(diagram_type) = diagram_type_of_lang(&language) {
            let mut block = Block::leaf(BlockKind::Diagram, text);
            block.diagram_type = Some(diagram_type.to_string());
            block.diagram_lang = Some(
                if diagram_type == "vega-lite" {
                    "json"
                } else {
                    "yaml"
                }
                .to_string(),
            );
            return Some(block);
        }

        let mut block = Block::leaf(BlockKind::CodeBlock, text);
        block.code_kind = Some(CodeBlockKind::Fenced);
        block.lang = Some(info);
        block.fence_char = Some(fence_char);
        if fence_len > 3 {
            block.fence_length = Some(fence_len);
        }
        Some(block)
    }

    /// Parse a link reference definition (`[label]: destination "title"`).
    ///
    /// A definition produces no block of its own; it only registers the
    /// label so that the reference links of the document can be resolved.
    /// The return value says whether a definition was consumed.
    ///
    /// A definition may reach over several lines.  The label may hold line
    /// endings, the destination and the title are separated by at most one
    /// line ending, and the title itself may reach over lines as long as it
    /// holds no blank line.
    fn parse_link_reference_definition(&mut self) -> bool {
        let Some(line) = self.peek_line() else {
            return false;
        };
        // Up to three spaces of indentation are allowed; four would turn
        // the line into an indented code block.
        if line.indent_width() >= 4 {
            return false;
        }
        if !line.content.starts_with('[') {
            return false;
        }

        // The candidate lines are collected one by one until the definition
        // is complete or turns out to be impossible.
        let mut text = String::from(line.content);
        let mut count = 1usize;
        while count <= MAX_DEFINITION_LINES {
            let complete = self.pos + count >= self.lines.len();
            match scan_link_definition(&text, complete) {
                Scan::Done(definition) => {
                    let consumed = text[..definition.end].matches('\n').count() + 1;
                    self.pos += consumed;
                    self.definitions.push(ReferenceDefinition {
                        label: normalize_label(&definition.label),
                        href: definition.href,
                        title: definition.title,
                    });
                    return true;
                }
                Scan::Invalid => return false,
                Scan::NeedMore => {}
            }
            let Some(next) = self.lines.get(self.pos + count) else {
                return false;
            };
            text.push('\n');
            text.push_str(next);
            count += 1;
        }
        false
    }

    /// Parse an ATX heading (`# Title`).
    fn parse_atx_heading(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 {
            return None;
        }
        let hashes = line.content.chars().take_while(|c| *c == '#').count();
        if hashes == 0 || hashes > 6 {
            return None;
        }
        let rest = &line.content[hashes..];
        // A space is required after the marker run.
        if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
            return None;
        }
        self.next_line();
        let text = format!("{}{}", "#".repeat(hashes), rest);
        let mut block = Block::leaf(BlockKind::AtxHeading, text.trim_end());
        block.level = Some(hashes as u8);
        Some(block)
    }

    /// Parse a setext heading (a paragraph underlined with `=` or `-`).
    /// Parse a setext heading: a paragraph line followed by an underline.
    ///
    /// CommonMark calls the underline a *setext heading underline*. It turns
    /// the line above it into a heading of level one (`=`) or two (`-`).
    fn parse_setext_heading(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 || line.content.is_empty() {
            return None;
        }
        // Only a plain paragraph line can carry an underline; a list item or
        // a code block ends before the underline is reached.
        if self.line_starts_block() {
            return None;
        }
        let next = self.lines.get(self.pos + 1)?;
        let next_indent = next.len() - next.trim_start_matches([' ', '\t']).len();
        if tab_to_columns(&next[..next_indent], self.bases[self.pos + 1]) >= 4 {
            return None;
        }
        let underline = next.trim();
        let is_equals = !underline.is_empty() && underline.chars().all(|c| c == '=');
        let is_dash = !underline.is_empty() && underline.chars().all(|c| c == '-');
        if !is_equals && !is_dash {
            return None;
        }

        self.pos += 2;
        let mut block = Block::leaf(
            BlockKind::SetextHeading,
            line.content.trim_end().to_string(),
        );
        block.level = Some(if is_equals { 1 } else { 2 });
        block.underline = Some(underline.to_string());
        Some(block)
    }

    /// Parse a thematic break (`---`, `***` or `___`).
    fn parse_thematic_break(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 {
            return None;
        }
        let content = line.content;
        if !is_thematic_break(content) {
            return None;
        }
        self.next_line();
        // The indentation of a thematic break is part of its line.
        let raw = self.lines[self.pos - 1].to_string();
        Some(Block::leaf(BlockKind::ThematicBreak, raw))
    }

    /// Parse a block quote, including GitHub alerts.
    fn parse_block_quote(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 || !line.content.starts_with('>') {
            return None;
        }

        // Collect the quote's raw lines, stripping one level of `>`.
        //
        // A line that carries no `>` marker may only continue the quote when
        // the quote ends with a paragraph — the *lazy continuation* of the
        // specification — and when the line does not start a block of its
        // own.
        let mut inner_lines: Vec<FragmentLine> = Vec::new();
        // Whether the fragment so far ends with a paragraph, which is what
        // allows a lazy continuation. `None` means "not known yet"; the probe
        // is only run when a line without `>` arrives, and a run of lazy
        // lines reuses the answer because a plain line cannot end the
        // paragraph it continues.
        let mut ends_with_paragraph: Option<bool> = None;
        while let Some(line) = self.next_line() {
            if line.content.starts_with('>') {
                let stripped = strip_quote_marker(&line);
                if ends_with_paragraph == Some(true) {
                    // A marked line that is ordinary paragraph text keeps the
                    // paragraph going. Anything else (a blank line, a block
                    // start or a setext underline) may change the last block,
                    // so the next probe has to look again.
                    let fragment_line = line_of_fragment(&stripped);
                    let keeps_paragraph = !fragment_line.content.is_empty()
                        && !is_setext_underline(fragment_line.content)
                        && !line_starts_block(&fragment_line, true, self.options.fancy_lists);
                    if !keeps_paragraph {
                        ends_with_paragraph = None;
                    }
                }
                inner_lines.push(stripped);
                continue;
            }
            if line.content.is_empty() {
                // A blank line always separates block quotes.
                self.pos -= 1;
                break;
            }
            // The probe tells whether the quote ends with a paragraph: only a
            // paragraph may be continued by a line that carries no `>` marker.
            // Parsing the whole fragment per lazy line is quadratic, so the
            // answer is cached; consecutive lazy lines all continue the same
            // paragraph.
            if ends_with_paragraph.is_none() {
                let probe = self.parse_probe_blocks(inner_lines.clone());
                ends_with_paragraph = Some(innermost_block_is_paragraph(&probe));
            }
            if ends_with_paragraph != Some(true)
                || line_starts_block(&line, true, self.options.fancy_lists)
            {
                self.pos -= 1;
                break;
            }
            if is_setext_underline(line.content) {
                // A setext heading underline cannot be a lazy continuation
                // line, so the line joins the quote as ordinary text.
                inner_lines.push(FragmentLine {
                    text: format!("\\{}", line.content),
                    base: line.base,
                });
                continue;
            }
            // Lazy continuation: a plain line continues the quote.  Its
            // indentation is kept, because a line that is indented by four
            // columns starts an indented code block and never interrupts a
            // paragraph.
            let mut continued = " ".repeat(line.indent_width());
            continued.push_str(line.content);
            inner_lines.push(FragmentLine {
                text: continued,
                base: line.base,
            });
        }

        // A GitHub alert is a quote whose first line is `[!NOTE]` and friends.
        if self.options.github_alerts {
            if let Some((alert_type, alert_marker, blank_after)) = match_alert_marker(&inner_lines)
            {
                let body = inner_lines[1..].to_vec();
                let children = self.parse_sub_blocks(body);
                let mut block = Block::container(BlockKind::GithubAlert, children);
                block.alert_type = Some(alert_type);
                block.alert_marker = Some(alert_marker);
                block.blank_line_after_marker = Some(blank_after);
                return Some(block);
            }
        }

        let children = self.parse_sub_blocks(inner_lines);
        Some(Block::container(BlockKind::BlockQuote, children))
    }

    /// Parse a GFM table.
    fn parse_table(&mut self) -> Option<Block> {
        let header_line = self.peek_line()?;
        if !header_line.content.contains('|') {
            return None;
        }
        let delimiter_line = self.lines.get(self.pos + 1)?;
        if !is_table_delimiter_row(delimiter_line) {
            return None;
        }

        let align = parse_table_alignments(delimiter_line);
        let header = split_table_row(header_line.content);
        // The header row has to match the delimiter row in the number of
        // cells; if it does not, the rows are no table at all.
        if header.len() != align.len() {
            return None;
        }
        let columns = header.len();

        self.pos += 2;
        let mut rows: Vec<Vec<String>> = Vec::new();
        while let Some(line) = self.peek_line() {
            // The table ends at the first empty line and at the beginning of
            // another block-level structure.
            if line.content.is_empty() || self.line_starts_block() {
                break;
            }
            let mut cells = split_table_row(line.content);
            // A row with fewer cells gets empty ones. A row with more cells
            // keeps them: GFM renders only the header's columns, but the
            // extra source must survive a round trip.
            if cells.len() < columns {
                cells.resize(columns, String::new());
            }
            rows.push(cells);
            self.pos += 1;
        }

        let mut table = Block::container(BlockKind::Table, Vec::new());
        let mut header_cells = Vec::new();
        for (index, text) in header.iter().enumerate() {
            let mut cell = Block::leaf(BlockKind::TableCell, text.clone());
            cell.align = Some(align.get(index).copied().unwrap_or(Align::None));
            header_cells.push(cell);
        }
        table.children = Some(vec![Block::container(BlockKind::TableRow, header_cells)]);

        for row in rows {
            let mut cells = Vec::new();
            for (index, text) in row.iter().enumerate() {
                let mut cell = Block::leaf(BlockKind::TableCell, text.clone());
                cell.align = Some(align.get(index).copied().unwrap_or(Align::None));
                cells.push(cell);
            }
            table
                .children
                .get_or_insert_with(Vec::new)
                .push(Block::container(BlockKind::TableRow, cells));
        }
        Some(table)
    }

    /// Parse an HTML block.
    /// Parse an HTML block.
    ///
    /// The standard defines seven ways in which a block of raw HTML starts.
    /// The first five end at a closing marker, the last two at the first
    /// blank line.
    fn parse_html_block(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 {
            return None;
        }
        let kind = html_block_kind(line.content)?;
        let mut body = Vec::new();
        while let Some(line) = self.next_line() {
            if line.content.is_empty() {
                // Only the sixth and the seventh kind of HTML block end at a
                // blank line; the first five run until their closing marker.
                if matches!(kind, HtmlBlockKind::Marker(_)) {
                    body.push(String::new());
                    continue;
                }
                self.pos -= 1;
                break;
            }
            // The content of an HTML block keeps the indentation it was
            // written with.
            body.push(line.raw.to_string());
            if let HtmlBlockKind::Marker(marker) = kind {
                if line.content.to_ascii_lowercase().contains(marker) {
                    break;
                }
            }
        }
        // The first five kinds of HTML block end at their closing marker.  A
        // block whose marker never appears runs to the end of the document,
        // exactly like the reference implementation does.
        let text = body.join("\n");
        // A lone `<img ...>` is treated as a paragraph so it renders inline.
        if is_single_image(&text) {
            return Some(Block::leaf(BlockKind::Paragraph, text));
        }
        Some(Block::leaf(BlockKind::HtmlBlock, text))
    }

    /// Parse a footnote definition (`[^id]: text`).
    fn parse_footnote_definition(&mut self) -> Option<Block> {
        if !self.options.footnote {
            return None;
        }
        let line = self.peek_line()?;
        let content = line.content;
        if !content.starts_with("[^") {
            return None;
        }
        let close = content.find("]:")?;
        let identifier = &content[2..close];
        if identifier.is_empty() || identifier.contains(char::is_whitespace) {
            return None;
        }
        self.next_line();
        let first = content[close + 2..].trim_start().to_string();
        let mut body = vec![FragmentLine::plain(&first)];
        while let Some(line) = self.next_line() {
            if line.content.is_empty() {
                let next_is_indented = self
                    .peek_line()
                    .map(|l| l.indent_width() >= 4)
                    .unwrap_or(false);
                if next_is_indented {
                    body.push(FragmentLine::plain(""));
                } else {
                    self.pos -= 1;
                    break;
                }
            } else if line.indent_width() >= 4 {
                body.push(FragmentLine::plain(line.content));
            } else {
                self.pos -= 1;
                break;
            }
        }
        let children = self.parse_sub_blocks(body);
        let mut block = Block::container(BlockKind::Footnote, children);
        block.identifier = Some(identifier.to_string());
        Some(block)
    }

    /// Parse an indented code block.
    fn parse_indented_code(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() < 4 || line.content.is_empty() {
            return None;
        }
        let mut body = Vec::new();
        // Whether a blank line follows the block.
        let mut last_line_blank = false;
        while let Some(line) = self.next_line() {
            if line.content.is_empty() {
                // A blank line only continues the code block when a later line
                // is indented as well.  The blank lines in between belong to
                // the block too, and they keep the indentation they were
                // written with.
                let blanks_start = self.pos - 1;
                let mut blanks = vec![strip_columns(line.raw, line.base, 4)];
                let mut continues = false;
                while let Some(next) = self.peek_line() {
                    if next.content.is_empty() {
                        blanks.push(strip_columns(next.raw, next.base, 4));
                        self.pos += 1;
                        continue;
                    }
                    continues = next.indent_width() >= 4;
                    break;
                }
                if continues {
                    body.extend(blanks);
                } else {
                    // The blank lines do not belong to the block; they are
                    // given back to the parser.
                    self.pos = blanks_start;
                    last_line_blank = true;
                    break;
                }
            } else if line.indent_width() >= 4 {
                // Only four columns of indentation are removed, so that the
                // surplus stays part of the code and a tab inside the code
                // survives the round trip.
                body.push(strip_columns(line.raw, line.base, 4));
            } else {
                self.pos -= 1;
                break;
            }
        }
        let mut block = Block::leaf(BlockKind::CodeBlock, body.join("\n"));
        block.code_kind = Some(CodeBlockKind::Indented);
        block.last_line_blank = Some(last_line_blank);
        Some(block)
    }

    /// Parse a paragraph: consecutive non-blank lines that start no other block.
    fn parse_paragraph(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 {
            return None;
        }

        let mut body = Vec::new();
        // Whether a blank line follows the paragraph.
        let mut last_line_blank = false;
        // A setext underline turns the whole paragraph into a heading, so the
        // level and the underline are remembered while the paragraph grows.
        let mut setext: Option<(u8, String)> = None;
        while let Some(line) = self.next_line() {
            if line.content.is_empty() {
                self.pos -= 1;
                last_line_blank = true;
                break;
            }
            // A setext underline below the last line of the paragraph turns
            // the paragraph into a heading of that level.
            if !body.is_empty() && self.peek_line_is_setext_underline() {
                body.push(line.content.trim_end().to_string());
                let underline = self.lines[self.pos].trim().to_string();
                let level = if underline.chars().all(|c| c == '=') {
                    1
                } else {
                    2
                };
                setext = Some((level, underline));
                self.pos += 1;
                break;
            }
            // A line that starts another block interrupts the paragraph.
            if !body.is_empty() && line_starts_block(&line, true, self.options.fancy_lists) {
                self.pos -= 1;
                break;
            }
            body.push(line.content.to_string());
        }
        let text = body.join("\n");
        if let Some((level, underline)) = setext {
            let mut block = Block::leaf(BlockKind::SetextHeading, text.trim_end().to_string());
            block.level = Some(level);
            block.underline = Some(underline);
            return Some(block);
        }
        let mut block = Block::leaf(BlockKind::Paragraph, text);
        block.last_line_blank = Some(last_line_blank);
        Some(block)
    }

    /// Whether the line after the current one is a setext heading underline.
    fn peek_line_is_setext_underline(&self) -> bool {
        let Some(next) = self.lines.get(self.pos) else {
            return false;
        };
        let indent = next.len() - next.trim_start_matches([' ', '\t']).len();
        if tab_to_columns(&next[..indent], self.bases[self.pos]) >= 4 {
            return false;
        }
        let underline = next.trim();
        !underline.is_empty()
            && (underline.chars().all(|c| c == '=') || underline.chars().all(|c| c == '-'))
    }

    /// Whether the current line starts a block other than a paragraph.
    fn line_starts_block(&self) -> bool {
        match self.peek_line() {
            Some(line) => line_starts_block(&line, false, self.options.fancy_lists),
            None => false,
        }
    }

    /// Parse a list of any flavour.
    fn parse_list(&mut self) -> Option<Block> {
        let line = self.peek_line()?;
        if line.indent_width() >= 4 {
            return None;
        }
        // The marker decides which flavour of list this is.
        list_marker_at_start(line.content, self.options.fancy_lists)?;
        let mut items: Vec<Block> = Vec::new();
        let mut ordered = false;
        let mut bullet_marker: Option<String> = None;
        let mut delimiter = ListDelimiter::Dot;
        let mut style = ListStyle::Decimal;
        let mut start: Option<u64> = None;
        let mut source_markers: Vec<String> = Vec::new();
        let mut all_tasks = true;
        let mut any_task = false;
        // Whether the items of this list so far are task items.
        let mut list_is_task: Option<bool> = None;
        // Whether a blank line separates two items of this list.
        let mut blank_between_items = false;
        // A blank line was seen before the item that is now examined.
        let mut pending_blank = false;
        // For every item, whether a blank line separates two of its blocks.
        let mut item_blanks: Vec<bool> = Vec::new();

        while let Some(line) = self.peek_line() {
            if line.indent_width() >= 4 {
                break;
            }
            let Some(item_marker) = list_marker_at_start(line.content, self.options.fancy_lists)
            else {
                break;
            };

            // The list kind is decided by its first item; a later item must
            // carry the same kind of marker to belong to this list.
            let same_kind = if items.is_empty() {
                true
            } else {
                match (&bullet_marker, &item_marker) {
                    (Some(existing), ListMarkerKind::Bullet { marker, .. }) => existing == marker,
                    (None, ListMarkerKind::Ordered { marker, .. }) => {
                        // A later ordered item may change its delimiter or
                        // style, but it stays in the same list.
                        match parse_list_marker(marker, Some(style)) {
                            Some(parsed) => {
                                if parsed.delimiter != delimiter {
                                    false
                                } else {
                                    if parsed.style != ListStyle::Decimal {
                                        style = parsed.style;
                                    }
                                    true
                                }
                            }
                            None => false,
                        }
                    }
                    _ => false,
                }
            };

            // A task item and a plain item belong to different lists, which is
            // why the parser splits a list where the items switch.
            let is_task = item_marker.is_task();
            let same_task_kind = match list_is_task {
                Some(previous) => previous == is_task,
                None => true,
            };

            // A blank line only makes the list loose when it separates two
            // items of *this* list. A blank line before a different kind of
            // list belongs to neither of them.
            if pending_blank {
                if !same_kind || !same_task_kind {
                    break;
                }
                blank_between_items = true;
                pending_blank = false;
            }
            if !same_kind {
                break;
            }
            if !same_task_kind {
                break;
            }
            list_is_task = Some(is_task);

            let raw_marker = item_marker.raw().to_string();
            let geometry = item_marker
                .geometry(line.content, line.width)
                .unwrap_or(ItemGeometry {
                    offset: raw_marker.len(),
                    indent: line.width + raw_marker.len(),
                    leading: 0,
                });
            if is_task {
                any_task = true;
            } else {
                all_tasks = false;
            }

            // Collect the item's lines.
            // The padding behind the marker is written out as spaces, so that
            // the nested parser sees the indentation the reference
            // implementation works with.
            let mut first_content = " ".repeat(geometry.leading);
            first_content.push_str(&line.content[geometry.offset..]);
            if is_task {
                // The checkbox is stored in the item's metadata, so it is
                // removed from the text the item holds.
                first_content = strip_task_checkbox(&first_content).to_string();
            }
            let mut item_lines = vec![FragmentLine {
                text: first_content,
                base: line.base,
            }];
            // A list item may begin with at most one blank line: when the
            // opening line of the item is blank, a second blank line ends
            // the item.
            let item_starts_blank = item_lines[0].text.trim().is_empty();
            self.pos += 1;
            let content_indent = geometry.indent;
            // Whether a blank line separates two blocks inside this item.
            let mut item_has_blank = false;
            while let Some(line) = self.next_line() {
                if line.content.is_empty() {
                    if item_starts_blank {
                        // The item began with a blank line, so the blank line
                        // that follows ends it.  The list itself may go on
                        // with another item.
                        pending_blank = true;
                        while self.pos < self.lines.len() && self.lines[self.pos].trim().is_empty()
                        {
                            self.pos += 1;
                        }
                        break;
                    }
                    // The next content line decides who owns the blank line.
                    let next = self.peek_content_line();
                    let next_indent = next.map(|l| l.indent_width()).unwrap_or(0);
                    let next_is_item = next
                        .map(|l| {
                            next_indent < content_indent
                                && list_marker_at_start(l.content, self.options.fancy_lists)
                                    .is_some()
                        })
                        .unwrap_or(false);
                    if next_is_item {
                        // The blank line may separate two items of this list,
                        // which is decided once the next item is known.
                        pending_blank = true;
                        // Skip the blank lines so the next item can be found.
                        while self.pos < self.lines.len() && self.lines[self.pos].trim().is_empty()
                        {
                            self.pos += 1;
                        }
                        break;
                    }
                    if next_indent >= content_indent {
                        // The blank line is inside the item.
                        item_lines.push(FragmentLine::plain(""));
                        item_has_blank = true;
                        continue;
                    }
                    // The blank line ends the item and belongs to no list.
                    self.pos -= 1;
                    break;
                } else if line.indent_width() >= content_indent {
                    // The indentation beyond the item's content column is
                    // relative and has to survive the round trip.
                    let extra = line.indent_width().saturating_sub(content_indent);
                    let mut stripped = " ".repeat(extra);
                    stripped.push_str(line.content);
                    item_lines.push(FragmentLine {
                        text: stripped,
                        base: line.base,
                    });
                } else if line.indent_width() < 4
                    && list_marker_at_start(line.content, self.options.fancy_lists).is_some()
                {
                    // A line that is indented by four columns or more starts
                    // an indented code block, which never interrupts a
                    // paragraph; it continues the item instead.
                    self.pos -= 1;
                    break;
                } else if is_thematic_break(line.content) {
                    // A thematic break ends the item and the list with it.
                    self.pos -= 1;
                    break;
                } else {
                    // Lazy continuation of the item's paragraph.  The
                    // indentation is kept, because a line that is indented by
                    // four columns starts an indented code block and never
                    // interrupts a paragraph.
                    let mut continued = " ".repeat(line.indent_width());
                    continued.push_str(line.content);
                    item_lines.push(FragmentLine {
                        text: continued,
                        base: line.base,
                    });
                }
            }
            item_blanks.push(item_has_blank);

            let children = self.parse_sub_blocks(item_lines);
            let mut item = if is_task {
                let checked = item_marker.checked();
                let mut block = Block::container(BlockKind::TaskListItem, children);
                block.checked = Some(checked);
                block
            } else {
                Block::container(BlockKind::ListItem, children)
            };
            if let ListMarkerKind::Ordered { marker, .. } = &item_marker {
                item.order_marker = Some(marker.clone());
                source_markers.push(raw_marker.clone());
            }
            items.push(item);

            if let ListMarkerKind::Bullet { marker: m, .. } = &item_marker {
                if bullet_marker.is_none() {
                    bullet_marker = Some(m.clone());
                }
            }
            if let ListMarkerKind::Ordered { marker, .. } = &item_marker {
                if !ordered {
                    ordered = true;
                    if let Some(parsed) = parse_list_marker(marker, None) {
                        start = Some(parsed.value);
                        delimiter = parsed.delimiter;
                        style = parsed.style;
                    }
                }
            }
        }

        if items.is_empty() {
            return None;
        }

        let loose = blank_between_items || list_is_loose(&items);
        let kind = if any_task && all_tasks {
            BlockKind::TaskList
        } else if ordered {
            BlockKind::OrderList
        } else {
            BlockKind::BulletList
        };

        let mut list_meta = ListMeta {
            loose,
            marker: bullet_marker.clone(),
            start: if ordered {
                Some(start.unwrap_or(1))
            } else {
                None
            },
            style: if ordered { Some(style) } else { None },
            delimiter: if ordered { Some(delimiter) } else { None },
            source_markers: if ordered && !source_markers.is_empty() {
                Some(source_markers)
            } else {
                None
            },
        };
        if kind == BlockKind::TaskList && !ordered {
            list_meta.style = None;
            list_meta.start = None;
            list_meta.delimiter = None;
            list_meta.source_markers = None;
        }

        let mut block = Block::container(kind, items);
        block.list = Some(list_meta);
        Some(block)
    }

    /// Parse a nested Markdown fragment into blocks.
    fn parse_sub_blocks(&mut self, lines: Vec<FragmentLine>) -> Vec<Block> {
        if self.depth + 1 > MAX_NESTING {
            // Keep the text instead of recursing until the stack runs out.
            let text = lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return vec![Block::leaf(BlockKind::Paragraph, text)];
        }
        let (blocks, definitions) = self.parse_fragment(lines);
        // A definition inside a list item or a quote belongs to the document.
        self.definitions.extend(definitions);
        blocks
    }

    /// Parse a nested Markdown fragment into blocks, keeping its definitions
    /// to itself.
    ///
    /// The method is used to look at the structure of a fragment without
    /// changing the document: the definitions of the probe are thrown away.
    fn parse_probe_blocks(&mut self, lines: Vec<FragmentLine>) -> Vec<Block> {
        if self.depth + 1 > MAX_NESTING {
            return Vec::new();
        }
        self.parse_fragment(lines).0
    }

    /// Parse a nested Markdown fragment into blocks and its definitions.
    fn parse_fragment(
        &mut self,
        lines: Vec<FragmentLine>,
    ) -> (Vec<Block>, Vec<ReferenceDefinition>) {
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        let bases: Vec<usize> = lines.iter().map(|line| line.base).collect();
        let mut nested = BlockParser {
            lines: texts,
            bases,
            pos: 0,
            options: self.options,
            definitions: Vec::new(),
            depth: self.depth + 1,
        };
        let mut blocks = Vec::new();
        nested.parse_blocks(&mut blocks);
        (blocks, nested.definitions)
    }
}

/// The kind of list marker at the start of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ListMarkerKind {
    /// `-`, `+` or `*`, optionally followed by a task checkbox.
    Bullet { marker: String, task: Option<bool> },
    /// `1.`, `1)` or `(1)`, optionally followed by a task checkbox.
    Ordered { marker: String, task: Option<bool> },
}

impl ListMarkerKind {
    /// The marker as written.
    fn raw(&self) -> &str {
        match self {
            ListMarkerKind::Bullet { marker, .. } => marker,
            ListMarkerKind::Ordered { marker, .. } => marker,
        }
    }

    /// The geometry of the item's content column.
    fn geometry(&self, content: &str, base: usize) -> Option<ItemGeometry> {
        marker_padding(content, self.raw().len(), base)
    }

    /// Whether the marker denotes a task item.
    fn is_task(&self) -> bool {
        match self {
            ListMarkerKind::Bullet { task, .. } | ListMarkerKind::Ordered { task, .. } => {
                task.is_some()
            }
        }
    }

    /// The checkbox state of a task item.
    fn checked(&self) -> bool {
        match self {
            ListMarkerKind::Bullet { task, .. } | ListMarkerKind::Ordered { task, .. } => {
                task.unwrap_or(false)
            }
        }
    }
}

/// The block-level tag names that start an HTML block of the sixth kind.
const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// The kind of HTML block a line starts.
#[derive(Clone, Copy)]
enum HtmlBlockKind {
    /// The block ends at the first blank line.
    ///
    /// This is the sixth kind of HTML block; it may interrupt a paragraph.
    BlankLine,
    /// The block ends at the first blank line as well, but it may not
    /// interrupt a paragraph.
    ///
    /// This is the seventh kind of HTML block: a complete tag on a line of
    /// its own.
    BlankLineWithoutInterrupt,
    /// The block ends at the line that holds the closing marker.
    Marker(&'static str),
}

/// The HTML block a line starts, if any.
fn html_block_kind(content: &str) -> Option<HtmlBlockKind> {
    let lower = content.to_ascii_lowercase();

    // The first kind: a tag that ends with a matching closing tag.
    for (prefix, marker) in [
        ("<script", "</script>"),
        ("<pre", "</pre>"),
        ("<style", "</style>"),
        ("<textarea", "</textarea>"),
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == '>') {
                return Some(HtmlBlockKind::Marker(marker));
            }
        }
    }
    // The second to the fifth kind: a comment, a processing instruction, a
    // declaration or a CDATA section.
    if lower.starts_with("<!--") {
        return Some(HtmlBlockKind::Marker("-->"));
    }
    if lower.starts_with("<?") {
        return Some(HtmlBlockKind::Marker("?>"));
    }
    if lower.starts_with("<![cdata[") {
        return Some(HtmlBlockKind::Marker("]]>"));
    }
    if lower.starts_with("<!") && lower[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Some(HtmlBlockKind::Marker(">"));
    }
    // The sixth kind: one of the known block-level tag names.
    if let Some((name, rest)) = leading_tag(content) {
        if HTML_BLOCK_TAGS.contains(&name.as_str()) && is_tag_name_boundary(rest) {
            return Some(HtmlBlockKind::BlankLine);
        }
    }
    // The seventh kind: a complete tag followed by nothing but whitespace.
    if is_complete_tag_line(content) {
        return Some(HtmlBlockKind::BlankLineWithoutInterrupt);
    }
    None
}

/// The name of the tag a line starts with and the text behind the name.
fn leading_tag(content: &str) -> Option<(String, &str)> {
    let rest = content.strip_prefix('<')?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let name_end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .unwrap_or(rest.len());
    if name_end == 0 {
        return None;
    }
    Some((rest[..name_end].to_ascii_lowercase(), &rest[name_end..]))
}

/// Whether the text behind a tag name ends the tag name.
fn is_tag_name_boundary(rest: &str) -> bool {
    let Some(ch) = rest.chars().next() else {
        // The tag name ends the line.
        return true;
    };
    if ch.is_whitespace() || ch == '>' {
        return true;
    }
    // A slash only ends the tag name when it closes the tag.
    ch == '/' && rest[1..].starts_with('>')
}

/// Whether the line holds a complete tag and nothing else.
///
/// The tag is read with the same scanner the inline parser uses, so that a
/// tag that the inline parser does not accept is not a block either.
fn is_complete_tag_line(content: &str) -> bool {
    let Some(end) = crate::inline::scan_raw_html_tag(content, 0) else {
        return false;
    };
    // Only whitespace may follow the tag.
    content[end..].trim().is_empty()
}

/// The number of lines a link reference definition may reach over.
const MAX_DEFINITION_LINES: usize = 200;

/// The parts of a link reference definition.
struct LinkDefinition {
    /// The label as it was written, without the brackets.
    label: String,
    /// The destination as it was written.
    href: String,
    /// The title as it was written, if the definition carries one.
    title: Option<String>,
    /// The byte offset behind the definition in the scanned text.
    end: usize,
}

/// The outcome of scanning one part of a link reference definition.
enum Scan<T> {
    /// The part was found.
    Done(T),
    /// The text ends before the part is complete; more text may help.
    NeedMore,
    /// The text cannot hold this part.
    Invalid,
}

/// Scan a link reference definition at the start of `text`.
///
/// `text` holds the candidate lines joined by line endings.  `complete`
/// tells whether the text reaches the end of the container, which decides
/// whether the text may still grow.
///
/// CommonMark allows the label to reach over lines, the destination and the
/// title to be separated by at most one line ending, and the title itself
/// to reach over lines as long as it holds no blank line.
fn scan_link_definition(text: &str, complete: bool) -> Scan<LinkDefinition> {
    let bytes = text.as_bytes();
    let mut index = skip_blanks(text, 0);

    // The label runs from the opening bracket to its closing bracket.
    let (label, after_label) = match scan_definition_label(text, index) {
        Scan::Done(value) => value,
        Scan::NeedMore => return Scan::NeedMore,
        Scan::Invalid => return Scan::Invalid,
    };
    // The colon follows the label.
    if bytes.get(after_label) != Some(&b':') {
        return Scan::Invalid;
    }
    index = after_label + 1;

    // The destination follows, separated by at most one line ending.
    index = skip_blanks_and_one_break(text, index);
    let (href, after_destination, angled) = match scan_definition_destination(text, index) {
        Scan::Done(value) => value,
        Scan::NeedMore => return Scan::NeedMore,
        Scan::Invalid => return Scan::Invalid,
    };
    if href.is_empty() && !angled {
        return Scan::Invalid;
    }

    // The title is optional.  It has to be separated from the destination
    // by whitespace, and nothing but blanks may follow it on its line.
    let after_ws = skip_blanks_and_one_break(text, after_destination);
    if after_ws >= bytes.len() {
        // The text ends here; only a complete text may end the definition.
        if !complete {
            return Scan::NeedMore;
        }
    } else if after_ws > after_destination && matches!(bytes[after_ws], b'"' | b'\'' | b'(') {
        match scan_definition_title(text, after_ws, complete) {
            Scan::Done((title, after_title)) => {
                let after_blanks = skip_blanks(text, after_title);
                match at_line_end(text, after_blanks, complete) {
                    Scan::Done(()) => {
                        return Scan::Done(LinkDefinition {
                            label,
                            href,
                            title: Some(title),
                            end: after_blanks,
                        });
                    }
                    Scan::NeedMore => return Scan::NeedMore,
                    // Something follows the title: the definition has no
                    // title after all.
                    Scan::Invalid => {}
                }
            }
            Scan::NeedMore => return Scan::NeedMore,
            // The title cannot be read; the definition may still stand
            // without one.
            Scan::Invalid => {}
        }
    }

    // Without a title the destination has to end its own line.
    let after_blanks = skip_blanks(text, after_destination);
    match at_line_end(text, after_blanks, complete) {
        Scan::Done(()) => Scan::Done(LinkDefinition {
            label,
            href,
            title: None,
            end: after_blanks,
        }),
        Scan::NeedMore => Scan::NeedMore,
        Scan::Invalid => Scan::Invalid,
    }
}

/// Scan the label of a link reference definition.
///
/// The label runs from the opening bracket to its closing bracket.  It may
/// reach over lines but must not hold an unescaped bracket.
fn scan_definition_label(text: &str, start: usize) -> Scan<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return Scan::Invalid;
    }
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => return Scan::Invalid,
            b']' => {
                let label = &text[start + 1..index];
                // The label must hold something else than blanks and it may
                // not be longer than 999 characters.
                if !label.chars().any(|ch| !ch.is_whitespace()) {
                    return Scan::Invalid;
                }
                if label.chars().count() > 999 {
                    return Scan::Invalid;
                }
                return Scan::Done((label.to_string(), index + 1));
            }
            _ => index += 1,
        }
    }
    Scan::NeedMore
}

/// Scan the destination of a link reference definition.
///
/// The return value holds the destination, the byte offset behind it and
/// whether the angle bracket form was used.
fn scan_definition_destination(text: &str, start: usize) -> Scan<(String, usize, bool)> {
    let bytes = text.as_bytes();
    if start >= bytes.len() {
        return Scan::NeedMore;
    }
    if bytes[start] == b'<' {
        // The angle bracket form holds no line ending and no bracket of its
        // own, but it may be empty.
        let mut index = start + 1;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => index += 2,
                b'\n' | b'<' => return Scan::Invalid,
                b'>' => {
                    let href = text[start + 1..index].to_string();
                    return Scan::Done((href, index + 1, true));
                }
                _ => index += 1,
            }
        }
        return Scan::NeedMore;
    }
    // The bare form ends at the first blank and may hold balanced
    // parentheses.
    let mut index = start;
    let mut depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
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
            b' ' | b'\t' | b'\n' => break,
            _ => index += 1,
        }
    }
    let index = index.min(bytes.len());
    if index == start {
        // A bare destination may not be empty.
        return Scan::Invalid;
    }
    Scan::Done((text[start..index].to_string(), index, false))
}

/// Scan the title of a link reference definition.
///
/// The title may reach over lines as long as it holds no blank line.  The
/// return value holds the title and the byte offset behind it.
fn scan_definition_title(text: &str, start: usize, complete: bool) -> Scan<(String, usize)> {
    let bytes = text.as_bytes();
    let Some(&opening) = bytes.get(start) else {
        return Scan::NeedMore;
    };
    let closing = match opening {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return Scan::Invalid,
    };
    let mut index = start + 1;
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'\n' => {
                // A title may not reach over a blank line.
                match blank_line_follows(text, index, complete) {
                    Scan::Done(()) => return Scan::Invalid,
                    Scan::NeedMore => return Scan::NeedMore,
                    Scan::Invalid => {}
                }
                index += 1;
            }
            b'(' if closing == b')' => {
                depth += 1;
                index += 1;
            }
            b')' if closing == b')' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Scan::Done((text[start + 1..index - 1].to_string(), index));
                }
            }
            _ => {
                if bytes[index] == closing {
                    return Scan::Done((text[start + 1..index].to_string(), index + 1));
                }
                index += 1;
            }
        }
    }
    Scan::NeedMore
}

/// Skip the spaces and tabs that separate two parts of a definition.
fn skip_blanks(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    while matches!(bytes.get(index), Some(b' ') | Some(b'\t')) {
        index += 1;
    }
    index
}

/// Skip the spaces, tabs and at most one line ending that separate two
/// parts of a definition.
fn skip_blanks_and_one_break(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    let mut seen_break = false;
    loop {
        match bytes.get(index) {
            Some(b' ') | Some(b'\t') => index += 1,
            Some(b'\n') => {
                if seen_break {
                    break;
                }
                seen_break = true;
                index += 1;
            }
            _ => break,
        }
    }
    index
}

/// Whether the text ends at `index` with a line ending.
///
/// A text that may still grow has no line ending at its end yet.
fn at_line_end(text: &str, index: usize, complete: bool) -> Scan<()> {
    match text.as_bytes().get(index) {
        Some(b'\n') => Scan::Done(()),
        None => {
            if complete {
                Scan::Done(())
            } else {
                Scan::NeedMore
            }
        }
        _ => Scan::Invalid,
    }
}

/// Whether the line that follows the line ending at `index` is a blank one.
fn blank_line_follows(text: &str, index: usize, complete: bool) -> Scan<()> {
    let bytes = text.as_bytes();
    let mut cursor = index + 1;
    while let Some(&byte) = bytes.get(cursor) {
        match byte {
            b'\n' => return Scan::Done(()),
            b' ' | b'\t' => cursor += 1,
            _ => return Scan::Invalid,
        }
    }
    if complete {
        Scan::Done(())
    } else {
        Scan::NeedMore
    }
}

/// Mark the block that a blank line follows.
///
/// The reference implementation puts the flag on the innermost block that was
/// open when the blank line arrived and on the list item that holds it.  A
/// list that is still open hands the flag over to its last item, so that the
/// blank line belongs to the item and not to the list.
fn mark_blank_line_after(block: &mut Block) {
    match block.kind {
        BlockKind::BulletList | BlockKind::OrderList | BlockKind::TaskList => {
            if let Some(item) = block.children_mut().last_mut() {
                mark_blank_line_after(item);
            }
        }
        BlockKind::ListItem | BlockKind::TaskListItem => {
            block.last_line_blank = Some(true);
            if let Some(child) = block.children_mut().last_mut() {
                mark_blank_line_after(child);
            }
        }
        _ => {
            block.last_line_blank = Some(true);
        }
    }
}

/// Whether a list is loose.
///
/// The list is loose when one of its items holds two blocks that a blank
/// line separates — the rule of the reference implementation.  A blank line
/// that separates two items of the list is handled by the parser itself,
/// because the blank line then belongs to no item at all.
fn list_is_loose(items: &[Block]) -> bool {
    for (index, item) in items.iter().enumerate() {
        let has_next_item = index + 1 < items.len();
        let children = item.children();
        for (child_index, child) in children.iter().enumerate() {
            let has_next_child = child_index + 1 < children.len();
            if (has_next_item || has_next_child) && ends_with_blank_line(child) {
                return true;
            }
        }
    }
    false
}

/// Whether a block ends with a blank line.
///
/// A list or a list item ends with a blank line when its last child does;
/// every other block carries the flag itself.
fn ends_with_blank_line(block: &Block) -> bool {
    match block.kind {
        BlockKind::BulletList
        | BlockKind::OrderList
        | BlockKind::TaskList
        | BlockKind::ListItem
        | BlockKind::TaskListItem => match block.children().last() {
            Some(last) => ends_with_blank_line(last),
            None => block.last_line_blank.unwrap_or(false),
        },
        _ => block.last_line_blank.unwrap_or(false),
    }
}

/// Whether the innermost open block of a fragment is a paragraph.
///
/// Only a paragraph may be continued by a line that carries no `>` marker,
/// so the check looks at the last block of the fragment and, for a quote or
/// a list, at the last block inside it.
fn innermost_block_is_paragraph(blocks: &[Block]) -> bool {
    let Some(block) = blocks.last() else {
        return false;
    };
    match block.kind {
        // A paragraph that a blank line follows is closed; the quote may not
        // be continued by a line that carries no `>` marker any more.
        BlockKind::Paragraph => !block.last_line_blank.unwrap_or(false),
        BlockKind::BlockQuote
        | BlockKind::GithubAlert
        | BlockKind::ListItem
        | BlockKind::TaskListItem => innermost_block_is_paragraph(block.children()),
        BlockKind::BulletList | BlockKind::OrderList | BlockKind::TaskList => {
            match block.children().last() {
                Some(item) => innermost_block_is_paragraph(item.children()),
                None => false,
            }
        }
        _ => false,
    }
}

/// Whether a line is a thematic break.
///
/// CommonMark allows three or more of the same marker character, optionally
/// separated by spaces: `---`, `- - -` and `___` all qualify.
fn is_thematic_break(content: &str) -> bool {
    let bytes = content.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    let marker = bytes[0];
    if !matches!(marker, b'-' | b'*' | b'_') {
        return false;
    }
    let mut count = 0usize;
    for byte in bytes {
        if *byte == marker {
            count += 1;
        } else if *byte != b' ' && *byte != b'\t' {
            return false;
        }
    }
    count >= 3
}

/// The geometry of the content column of a list item.
#[derive(Clone, Copy)]
struct ItemGeometry {
    /// The byte offset of the content inside the line.
    offset: usize,
    /// The absolute column the content of the item starts at.
    indent: usize,
    /// The number of columns the content is pushed to the right by the
    /// padding, measured from the content column.
    leading: usize,
}

/// The padding between a list marker and the content of the item.
///
/// CommonMark allows up to four columns of padding.  More than that and the
/// item starts with an indented code block, in which case the content column
/// sits directly behind the marker and the surplus padding stays part of
/// the content.
fn marker_padding(content: &str, marker_len: usize, base: usize) -> Option<ItemGeometry> {
    let bytes = content.as_bytes();
    let mut offset = marker_len;
    // A tab advances to the next tab stop, which is counted from the column
    // the marker ends in.
    let mut column = base + marker_len;
    let mut padding = 0usize;
    while offset < bytes.len() && (bytes[offset] == b' ' || bytes[offset] == b'\t') {
        let advance = if bytes[offset] == b'\t' {
            4 - (column % 4)
        } else {
            1
        };
        padding += advance;
        column += advance;
        offset += 1;
    }
    // A marker has to be followed by a space, or it ends the line.
    if padding == 0 && offset < bytes.len() {
        return None;
    }
    let content_column = base + marker_len + padding;
    let (indent, leading) = if padding == 0 {
        // The marker ends the line, so the content of the item starts in the
        // column right behind it.
        (base + marker_len + 1, 0)
    } else if padding > 4 {
        // More than four columns of padding: the content starts directly
        // behind the marker and the surplus stays part of the content.
        (base + marker_len + 1, padding - 1)
    } else {
        (content_column, 0)
    };
    Some(ItemGeometry {
        offset,
        indent,
        leading,
    })
}

/// Whether the list marker at the start of the line may interrupt a paragraph.
///
/// An empty list item never interrupts a paragraph, and an ordered list only
/// does so when it starts with `1` — the two rules the specification states
/// for a list that interrupts a paragraph.
fn list_marker_interrupts_paragraph(content: &str, fancy: bool) -> bool {
    let Some(marker) = list_marker_at_start(content, fancy) else {
        return false;
    };
    // The item has to hold something else than the marker itself.
    let rest = content[marker.raw().len()..].trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return false;
    }
    if let ListMarkerKind::Ordered { marker, .. } = &marker {
        // Only a list that starts with `1` interrupts a paragraph.
        let Some(parsed) = parse_list_marker(marker, None) else {
            return false;
        };
        if parsed.value != 1 {
            return false;
        }
    }
    true
}

/// Whether the given line starts a block other than a paragraph.
/// Whether a line starts a block that has to be parsed on its own.
///
/// While a paragraph is being read, `in_paragraph` is set: an indented code
/// block can never interrupt a paragraph, so the indentation is ignored in
/// that case and the line simply continues the paragraph.
fn line_starts_block(line: &Line<'_>, in_paragraph: bool, fancy: bool) -> bool {
    if line.indent_width() >= 4 {
        return !in_paragraph;
    }
    let content = line.content;
    if content.starts_with('>')
        || content.starts_with("```")
        || content.starts_with("~~~")
        || content == "$$"
    {
        return true;
    }
    if content.starts_with('#') {
        let hashes = content.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) {
            let rest = &content[hashes..];
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                return true;
            }
        }
    }
    if content.len() >= 3 && is_thematic_break(content) {
        return true;
    }
    // An HTML block of the first to the sixth kind interrupts a paragraph;
    // the seventh kind may not.
    if matches!(
        html_block_kind(content),
        Some(HtmlBlockKind::BlankLine) | Some(HtmlBlockKind::Marker(_))
    ) {
        return true;
    }
    if list_marker_at_start(content, fancy).is_some() {
        // A list interrupts a paragraph only when its item is not empty and,
        // for an ordered list, when it starts with `1`.
        return !in_paragraph || list_marker_interrupts_paragraph(content, fancy);
    }
    false
}

/// Clean the info string of a fenced code block.
///
/// CommonMark parses backslash escapes and entity references in the info
/// string, so `` ``` f&ouml;&ouml; `` names the language `föö`.  Both are
/// handled in a single left-to-right pass, which is what keeps an escaped
/// ampersand (`\&ouml;`) the literal text `&ouml;` instead of turning it
/// into `ö`.
pub fn clean_info_string(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 1 < bytes.len()
            && bytes[index + 1].is_ascii_punctuation()
        {
            out.push(bytes[index + 1] as char);
            index += 2;
            continue;
        }
        if bytes[index] == b'&' {
            if let Some((length, decoded)) = crate::entities::decode_reference(&text[index..]) {
                out.push_str(&decoded);
                index += length;
                continue;
            }
        }
        let ch = text[index..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// Whether a line is a setext heading underline.
///
/// A line of `=` characters is the underline of a level one heading, a line
/// of `-` characters the underline of a level two heading.
fn is_setext_underline(content: &str) -> bool {
    let trimmed = content.trim();
    !trimmed.is_empty() && (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-'))
}

/// Detect a list marker at the start of a line.
fn list_marker_at_start(content: &str, fancy: bool) -> Option<ListMarkerKind> {
    let bytes = content.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    // Bullet marker.
    let first = bytes[0];
    if matches!(first, b'-' | b'+' | b'*') {
        let marker = &content[..1];
        // A thematic break is not a list item.
        if is_thematic_break(content) {
            return None;
        }
        let offset = marker_padding(content, 1, 0)?.offset;
        let rest = &content[offset..];
        return Some(ListMarkerKind::Bullet {
            marker: marker.to_string(),
            task: read_task_checkbox(rest),
        });
    }

    // Ordered marker: digits followed by `.` or `)`, or `(digits)`.
    let digits = content.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && digits <= 9 {
        let rest = &content[digits..];
        if rest.starts_with('.') || rest.starts_with(')') {
            let marker = &content[..digits + 1];
            let offset = marker_padding(content, digits + 1, 0)?.offset;
            let body = &content[offset..];
            let task = read_task_checkbox(body);
            return Some(ListMarkerKind::Ordered {
                marker: marker.to_string(),
                task,
            });
        }
    }
    if content.starts_with('(') {
        let close = content.find(')')?;
        let inner = &content[1..close];
        if !inner.is_empty() && inner.len() <= 9 && inner.bytes().all(|b| b.is_ascii_digit()) {
            let marker = &content[..close + 1];
            let offset = marker_padding(content, close + 1, 0)?.offset;
            let body = &content[offset..];
            let task = read_task_checkbox(body);
            return Some(ListMarkerKind::Ordered {
                marker: marker.to_string(),
                task,
            });
        }
    }

    // Pandoc-style alphabetic and roman markers. They are only recognized
    // when the option is on: at the start of a line `A. Smith` is prose much
    // more often than it is a list.
    if fancy {
        let (end, body) = if content.starts_with('(') {
            let close = content.find(')')?;
            let inner = &content[1..close];
            if inner.is_empty() || !inner.bytes().all(|b| b.is_ascii_alphabetic()) {
                return None;
            }
            (close + 1, inner)
        } else {
            let run = content
                .bytes()
                .take_while(|b| b.is_ascii_alphabetic())
                .count();
            let delimiter = *content.as_bytes().get(run)?;
            if run == 0 || !matches!(delimiter, b'.' | b')') {
                return None;
            }
            (run + 1, &content[..run])
        };
        let marker = &content[..end];
        if let Some(parsed) = parse_list_marker(marker, None) {
            if parsed.style != ListStyle::Decimal && !body.is_empty() {
                let offset = marker_padding(content, end, 0)?.offset;
                let rest = &content[offset..];
                return Some(ListMarkerKind::Ordered {
                    marker: marker.to_string(),
                    task: read_task_checkbox(rest),
                });
            }
        }
    }

    None
}

/// Read a `[ ]` / `[x]` task checkbox at the start of a list item's content.
fn read_task_checkbox(body: &str) -> Option<bool> {
    let bytes = body.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
        return None;
    }
    match bytes[1] {
        b' ' => Some(false),
        b'x' | b'X' => Some(true),
        _ => None,
    }
}

/// Remove the checkbox of a task item from its text.
///
/// The checkbox itself is kept in the item's metadata, so the text of the
/// item starts at the word that follows it.
fn strip_task_checkbox(content: &str) -> &str {
    let bytes = content.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
        return content;
    }
    if !matches!(bytes[1], b' ' | b'x' | b'X') {
        return content;
    }
    let rest = &content[3..];
    rest.strip_prefix(' ').unwrap_or(rest)
}

/// Whether a line is a GFM table delimiter row (`---|:--:|---`).
fn is_table_delimiter_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.contains('-') {
        return false;
    }
    for cell in trimmed.trim_matches('|').split('|') {
        let cell = cell.trim();
        if cell.is_empty() {
            return false;
        }
        let left = cell.starts_with(':');
        let right = cell.ends_with(':');
        let core = cell.trim_matches(':');
        if core.is_empty() || !core.bytes().all(|b| b == b'-') {
            return false;
        }
        let _ = (left, right);
    }
    true
}

/// Read the alignment of each column from a table delimiter row.
fn parse_table_alignments(line: &str) -> Vec<Align> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| {
            let cell = cell.trim();
            let left = cell.starts_with(':');
            let right = cell.ends_with(':');
            match (left, right) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                (false, false) => Align::None,
            }
        })
        .collect()
}

/// Split a table row into its cells, honoring backslash escapes.
///
/// The escape in front of a pipe is resolved, so that the cell holds the
/// literal pipe.  The serializer writes the escape back, which keeps the
/// Markdown of the document intact.
fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed).to_string();
    let trimmed = trimmed.strip_suffix('|').unwrap_or(&trimmed).to_string();

    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                // A backslash escapes the next character, including a pipe.
                match chars.next() {
                    Some('|') => current.push('|'),
                    Some(next) => {
                        current.push('\\');
                        current.push(next);
                    }
                    None => current.push('\\'),
                }
            }
            '|' => {
                cells.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    cells.push(current.trim().to_string());
    cells
}

/// Whether an HTML fragment is a single image tag.
fn is_single_image(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("<img")
        && trimmed.ends_with('>')
        && !trimmed[4..trimmed.len() - 1].contains('<')
        && !trimmed[4..trimmed.len() - 1].contains('>')
}

/// The diagram type a fence language renders as, or `None` for a code language.
fn diagram_type_of_lang(lang: &str) -> Option<&'static str> {
    match lang {
        "mermaid" => Some("mermaid"),
        "plantuml" => Some("plantuml"),
        "vega-lite" => Some("vega-lite"),
        "flowchart" => Some("flowchart"),
        "sequence" => Some("sequence"),
        _ => None,
    }
}

/// Match a GitHub alert marker at the start of a quote's content.
fn match_alert_marker(lines: &[FragmentLine]) -> Option<(String, String, bool)> {
    let first = lines.first()?;
    let trimmed = first.text.trim();
    let inner = trimmed.strip_prefix("[!")?.strip_suffix(']')?;
    let upper = inner.to_ascii_uppercase();
    if !matches!(
        upper.as_str(),
        "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION"
    ) {
        return None;
    }
    // A blank line after the marker is recorded so it survives a round trip.
    let blank_after = lines
        .get(1)
        .map(|l| l.text.trim().is_empty())
        .unwrap_or(false);
    Some((upper, trimmed.to_string(), blank_after))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(blocks: &[Block]) -> Vec<BlockKind> {
        blocks.iter().map(|b| b.kind.clone()).collect()
    }

    #[test]
    fn parses_heading_and_paragraph() {
        let blocks = parse("# Title\n\nHello").unwrap();
        assert_eq!(
            kinds(&blocks),
            vec![BlockKind::AtxHeading, BlockKind::Paragraph]
        );
        assert_eq!(blocks[0].level, Some(1));
        assert_eq!(blocks[0].text(), "# Title");
        assert_eq!(blocks[1].text(), "Hello");
    }

    #[test]
    fn parses_setext_heading() {
        let blocks = parse("Title\n=====\n\nBody").unwrap();
        assert_eq!(
            kinds(&blocks),
            vec![BlockKind::SetextHeading, BlockKind::Paragraph]
        );
        assert_eq!(blocks[0].level, Some(1));
        assert_eq!(blocks[0].underline.as_deref(), Some("====="));
    }

    #[test]
    fn parses_fenced_code_with_language() {
        let blocks = parse("```rust\nfn main() {}\n```").unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, BlockKind::CodeBlock);
        assert_eq!(blocks[0].lang.as_deref(), Some("rust"));
        assert_eq!(blocks[0].text(), "fn main() {}");
    }

    #[test]
    fn parses_bullet_list() {
        let blocks = parse("- one\n- two").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BulletList]);
        assert_eq!(blocks[0].children().len(), 2);
        assert_eq!(blocks[0].children()[0].kind, BlockKind::ListItem);
    }

    #[test]
    fn parses_task_list() {
        let blocks = parse("- [ ] todo\n- [x] done").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::TaskList]);
        let items = blocks[0].children();
        assert_eq!(items[0].checked, Some(false));
        assert_eq!(items[1].checked, Some(true));
    }

    #[test]
    fn parses_nested_list() {
        let blocks = parse("- one\n  - nested\n- two").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BulletList]);
        let first = &blocks[0].children()[0];
        assert_eq!(first.kind, BlockKind::ListItem);
        let inner_kinds: Vec<BlockKind> = first.children().iter().map(|b| b.kind.clone()).collect();
        assert!(inner_kinds.contains(&BlockKind::BulletList));
    }

    #[test]
    fn parses_block_quote() {
        let blocks = parse("> quoted\n> more").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BlockQuote]);
        assert_eq!(blocks[0].children().len(), 1);
        assert_eq!(blocks[0].children()[0].text(), "quoted\nmore");
    }

    #[test]
    fn a_lazy_line_continues_a_quote_paragraph() {
        let blocks = parse("> quoted\nlazy\n").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BlockQuote]);
        assert_eq!(blocks[0].children()[0].text(), "quoted\nlazy");
    }

    #[test]
    fn a_lazy_line_does_not_extend_a_heading_in_a_quote() {
        let blocks = parse("> # heading\nlazy\n").unwrap();
        assert_eq!(
            kinds(&blocks),
            vec![BlockKind::BlockQuote, BlockKind::Paragraph]
        );
        assert_eq!(kinds(blocks[0].children()), vec![BlockKind::AtxHeading]);
        assert_eq!(blocks[1].text(), "lazy");
    }

    #[test]
    fn a_lazy_line_does_not_extend_a_fence_in_a_quote() {
        let blocks = parse("> ```\n> code\n> ```\nlazy\n").unwrap();
        assert_eq!(
            kinds(&blocks),
            vec![BlockKind::BlockQuote, BlockKind::Paragraph]
        );
        assert_eq!(kinds(blocks[0].children()), vec![BlockKind::CodeBlock]);
        assert_eq!(blocks[1].text(), "lazy");
    }

    #[test]
    fn a_long_run_of_lazy_lines_stays_one_paragraph() {
        // The probe that decides whether a lazy line may continue the quote
        // used to re-parse the whole quote for every line, which is quadratic.
        let mut source = String::from("> quoted\n");
        for index in 0..5_000 {
            source.push_str(&format!("lazy {index}\n"));
        }
        let blocks = parse(&source).unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BlockQuote]);
        let paragraph = &blocks[0].children()[0];
        assert_eq!(paragraph.kind, BlockKind::Paragraph);
        assert!(paragraph.text().starts_with("quoted\nlazy 0\n"));
        assert!(paragraph.text().ends_with("lazy 4999"));
    }

    #[test]
    fn counts_the_columns_of_a_tab_behind_the_quote_marker() {
        // The tab behind the `>` reaches to the fourth column, so it is
        // used up by one column only and leaves two columns behind.  The
        // second tab fills the columns four to seven, which makes the
        // content of the quote indented by six columns — enough for an
        // indented code block that keeps the two columns that are left.
        let blocks = parse(">\t\tfoo\n").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BlockQuote]);
        let quote = &blocks[0];
        assert_eq!(kinds(quote.children()), vec![BlockKind::CodeBlock]);
        assert_eq!(quote.children()[0].text(), "  foo");
    }

    #[test]
    fn counts_the_columns_of_a_tab_after_the_quote_marker() {
        // A space behind the `>` is used up completely, so the tab that
        // follows starts in the third column and reaches to the fourth.
        let blocks = parse("> \tfoo\n").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::BlockQuote]);
        let quote = &blocks[0];
        assert_eq!(kinds(quote.children()), vec![BlockKind::Paragraph]);
        assert_eq!(quote.children()[0].text(), "foo");
    }

    #[test]
    fn keeps_the_columns_of_an_indented_code_block() {
        // The tab of the second line fills the columns zero to three, so
        // the four columns of the indentation use it up completely.
        let blocks = parse("    foo\n\tbar\n").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::CodeBlock]);
        assert_eq!(blocks[0].text(), "foo\nbar");
    }

    #[test]
    fn parses_table() {
        let blocks = parse("| a | b |\n| --- | ---: |\n| 1 | 2 |").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::Table]);
        let rows = blocks[0].children();
        assert_eq!(rows.len(), 2);
        // The alignment comes from the delimiter row and applies to every row.
        assert_eq!(rows[0].children()[0].align, Some(Align::None));
        assert_eq!(rows[0].children()[1].align, Some(Align::Right));
        assert_eq!(rows[1].children()[1].align, Some(Align::Right));
    }

    #[test]
    fn parses_frontmatter() {
        let blocks = parse("---\ntitle: x\n---\n\nBody").unwrap();
        assert_eq!(
            kinds(&blocks),
            vec![BlockKind::Frontmatter, BlockKind::Paragraph]
        );
        assert_eq!(blocks[0].text(), "title: x");
        assert_eq!(blocks[0].frontmatter_lang.as_deref(), Some("yaml"));
    }

    #[test]
    fn parses_thematic_break() {
        let blocks = parse("---").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::ThematicBreak]);
    }

    #[test]
    fn parses_math_block() {
        let blocks = parse("$$\nx^2\n$$").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::MathBlock]);
        assert_eq!(blocks[0].text(), "x^2");
    }

    #[test]
    fn parses_diagram_block() {
        let blocks = parse("```mermaid\ngraph TD;\n```").unwrap();
        assert_eq!(kinds(&blocks), vec![BlockKind::Diagram]);
        assert_eq!(blocks[0].diagram_type.as_deref(), Some("mermaid"));
    }

    #[test]
    fn rejects_null_bytes() {
        assert!(parse("a\0b").is_err());
    }

    #[test]
    fn deep_nesting_stops_at_the_limit_instead_of_overflowing() {
        // A few hundred levels used to abort the process with a stack
        // overflow; the deepest levels are kept as text.
        let source = format!("{} x\n", ">".repeat(600));
        let blocks = parse(&source).expect("deep nesting must still parse");
        assert!(!blocks.is_empty());
    }

    #[test]
    fn footnote_definitions_are_not_swallowed_by_link_references() {
        let options = ParseOptions {
            footnote: true,
            ..ParseOptions::default()
        };
        let document = parse_document_with_options("[^1]: note\n\nsee [^1]\n", options)
            .expect("valid markdown");
        assert!(document
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Footnote));
        assert!(document.references.is_empty());
    }
}
