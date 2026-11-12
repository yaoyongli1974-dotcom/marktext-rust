//! Serialization: the block tree back to Markdown.
//!
//! Ported from MarkText's `packages/muya/src/state/stateToMarkdown.ts`.
//!
//! The serializer is *lossless* for an unmodified document. It achieves that
//! by keeping the original spelling of everything that is not plain text —
//! the fence character and length of a code block, the bullet character of a
//! list, the ordered item markers, the underline of a setext heading — and by
//! reproducing the blank-line separation rules exactly.
//!
//! The separation rules live in [`resolve_boundary`], a port of MarkText's
//! `BlockBoundaryResolver`: a blank line is the default separator between two
//! blocks, because *not* separating them can change how the document parses
//! back. Inside a tight list item the rule relaxes, so the items of a compact
//! list stay compact.

use crate::document::{
    Align, Block, BlockKind, CodeBlockKind, FenceChar, ListDelimiter, ListMeta, ListStyle,
    MathStyle,
};
use crate::list_marker::{
    continues_list, format_list_marker, min_spaces_after_marker, parse_list_marker, ListLevel,
    ListMarker, MarkerPosition,
};

/// Options that control how a document is written back to Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    /// The number of spaces a nested list is indented by.
    pub list_indentation: usize,
    /// Whether pandoc-style fancy list numbering is written.
    pub fancy_lists: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            list_indentation: 1,
            fancy_lists: false,
        }
    }
}

/// Serialize a block tree to Markdown.
pub fn serialize(blocks: &[Block], options: ExportOptions) -> String {
    let mut writer = Writer {
        options,
        list_type: Vec::new(),
        is_loose_parent_list: true,
        list_indentation_count: options.list_indentation.clamp(1, 4),
    };
    writer.write_blocks(blocks, "", "", false)
}

/// Serialize a whole document, including its reference definitions.
///
/// Link reference definitions are not blocks: the parser keeps them in
/// `Document::references` because they apply to the whole document. Writing
/// only the blocks therefore dropped every `[label]: destination` line.
/// The definitions are appended at the end, where they still apply to every
/// reference; their original position among the blocks is not kept.
pub fn serialize_document(document: &crate::document::Document, options: ExportOptions) -> String {
    let mut output = serialize(&document.blocks, options);
    if document.references.is_empty() {
        return output;
    }

    if !output.is_empty() {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
    }

    for reference in &document.references {
        write_reference_definition(reference, &mut output);
    }
    output
}

/// Write one link reference definition.
fn write_reference_definition(reference: &crate::inline::ReferenceDefinition, output: &mut String) {
    output.push('[');
    output.push_str(&reference.label);
    output.push_str("]: ");
    output.push_str(&reference.href);
    if let Some(title) = &reference.title {
        output.push_str(" \"");
        for character in title.chars() {
            if matches!(character, '"' | '\\') {
                output.push('\\');
            }
            output.push(character);
        }
        output.push('"');
    }
    output.push('\n');
}

/// The metadata of the list currently being written.
#[derive(Debug, Clone)]
struct WrittenList {
    name: BlockKind,
    meta: ListMeta,
}

/// The mutable state the serializer carries while walking the tree.
struct Writer {
    options: ExportOptions,
    /// The metadata of each list currently being written, outermost first.
    list_type: Vec<ListMeta>,
    /// Whether the list that was written last was loose.
    is_loose_parent_list: bool,
    /// The number of spaces a nested list is indented by.
    list_indentation_count: usize,
}

/// How two consecutive blocks are separated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Separation {
    /// No blank line between them.
    Adjacent,
    /// Exactly one blank line between them.
    BlankLine,
    /// The two blocks cannot stay apart; one of them has to be rewritten.
    RewriteRequired,
}

/// These spellings end their block without consuming the next line.
fn is_self_terminating(kind: &BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::AtxHeading
            | BlockKind::SetextHeading
            | BlockKind::ThematicBreak
            | BlockKind::CodeBlock
            | BlockKind::MathBlock
            | BlockKind::Diagram
    )
}

/// Whether a block kind is one of the three list flavours.
fn is_any_list_state(kind: &BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::OrderList | BlockKind::BulletList | BlockKind::TaskList
    )
}

/// Whether a block kind is a list item.
fn is_list_item_state(kind: &BlockKind) -> bool {
    matches!(kind, BlockKind::ListItem | BlockKind::TaskListItem)
}

impl Writer {
    /// Write a sequence of sibling blocks.
    fn write_blocks(
        &mut self,
        states: &[Block],
        indent: &str,
        list_indent: &str,
        is_tight_item: bool,
    ) -> String {
        let mut result: Vec<String> = Vec::new();
        // The list written last, needed to decide whether two adjacent lists
        // would merge into one when the document is parsed again.
        let mut previous_list: Option<WrittenList> = None;
        let mut previous_state: Option<Block> = None;

        for original_state in states {
            // Only a list can merge with the list before it.
            if !is_any_list_state(&original_state.kind) {
                previous_list = None;
            }

            let state: Block = if is_any_list_state(&original_state.kind) {
                let needs_paragraph_separator = previous_state
                    .as_ref()
                    .map(|previous| {
                        previous.kind == BlockKind::Paragraph && !previous.text().trim().is_empty()
                    })
                    .unwrap_or(false)
                    && self.list_needs_paragraph_separator(original_state);
                let follows_tight_list = previous_state
                    .as_ref()
                    .map(|previous| is_any_list_state(&previous.kind))
                    .unwrap_or(false)
                    && !previous_state.as_ref().map(list_is_loose).unwrap_or(true)
                    && !list_is_loose(original_state);
                previous_list = Some(self.write_list_block(
                    original_state,
                    &mut result,
                    indent,
                    list_indent,
                    previous_list,
                    ListContext {
                        needs_paragraph_separator,
                        follows_tight_list,
                    },
                ));
                original_state.clone()
            } else if is_list_item_state(&original_state.kind) {
                self.write_list_item_block(
                    original_state,
                    &mut result,
                    indent,
                    list_indent,
                    previous_state.is_none(),
                );
                original_state.clone()
            } else {
                let (prepared, separation) = self.prepare_block_boundary(
                    original_state,
                    previous_state.as_ref(),
                    is_tight_item,
                );
                self.write_simple_block(&prepared, &mut result, indent, separation);
                prepared
            };

            previous_state = Some(state);
        }

        result.concat()
    }

    /// Decide the separator between two blocks.
    ///
    /// Ported from MarkText's `BlockBoundaryResolver`: a blank line is the
    /// default, because a missing separator can change how the two blocks
    /// parse back. Inside a tight list item the rule relaxes so that a compact
    /// list stays compact.
    fn resolve_boundary(
        &self,
        previous: &Block,
        next: &Block,
        tight_list_item: bool,
    ) -> Separation {
        // Two indented code blocks cannot be separated by a blank line at all.
        if previous.kind == BlockKind::CodeBlock
            && previous.code_kind == Some(CodeBlockKind::Indented)
            && next.kind == BlockKind::CodeBlock
            && next.code_kind == Some(CodeBlockKind::Indented)
        {
            return Separation::RewriteRequired;
        }

        if !tight_list_item || next.kind == BlockKind::GithubAlert {
            return Separation::BlankLine;
        }

        if is_self_terminating(&previous.kind) {
            return Separation::Adjacent;
        }

        if previous.kind == BlockKind::Paragraph {
            return if self.interrupts_paragraph(next, true) {
                Separation::Adjacent
            } else {
                Separation::BlankLine
            };
        }

        // A plain paragraph can lazily continue a quote. Only an interrupting
        // block written outside the `>` prefix reliably ends that quote.
        if previous.kind == BlockKind::BlockQuote && next.kind != BlockKind::BlockQuote {
            return if self.interrupts_paragraph(next, false) {
                Separation::Adjacent
            } else {
                Separation::BlankLine
            };
        }

        Separation::BlankLine
    }

    /// Whether a block interrupts an open paragraph.
    fn interrupts_paragraph(&self, next: &Block, setext_precedence: bool) -> bool {
        match next.kind {
            BlockKind::AtxHeading | BlockKind::BlockQuote | BlockKind::Diagram => true,
            BlockKind::CodeBlock => next.code_kind == Some(CodeBlockKind::Fenced),
            // Directly below paragraph text, contiguous dashes are a setext
            // underline instead of the intended thematic break.
            BlockKind::ThematicBreak => !setext_precedence || !is_dashes_only(next.text()),
            _ => false,
        }
    }

    /// Adjust a block when the separator between it and its predecessor
    /// requires the block to be rewritten.
    fn prepare_block_boundary(
        &self,
        state: &Block,
        previous: Option<&Block>,
        tight_list_item: bool,
    ) -> (Block, Separation) {
        let mut separation = match previous {
            Some(previous) => self.resolve_boundary(previous, state, tight_list_item),
            None => Separation::Adjacent,
        };
        let mut state = state.clone();

        if separation == Separation::RewriteRequired && state.kind == BlockKind::CodeBlock {
            // Blank lines remain part of an indented code block, so the next
            // block is fenced on output instead.
            state.code_kind = Some(CodeBlockKind::Fenced);
            separation = if tight_list_item {
                Separation::Adjacent
            } else {
                Separation::BlankLine
            };
        }

        (state, separation)
    }
}

/// Whether the text consists only of dashes and trailing whitespace.
fn is_dashes_only(text: &str) -> bool {
    let trimmed = text.trim_end();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '-')
}

/// Whether a list is loose, i.e. whether its items are separated by blank lines.
fn list_is_loose(block: &Block) -> bool {
    block.list.as_ref().map(|meta| meta.loose).unwrap_or(false)
}

/// Whether two adjacent lists would merge into one when parsed again.
fn would_merge_adjacent_lists(previous: &ListMeta, meta: &ListMeta) -> bool {
    if let (Some(previous_delimiter), Some(delimiter)) = (previous.delimiter, meta.delimiter) {
        let list = ListLevel {
            style: previous.style.unwrap_or(ListStyle::Decimal),
            delimiter: previous_delimiter,
        };
        let next = ListMarker {
            value: meta.start.unwrap_or(1),
            style: meta.style.unwrap_or(ListStyle::Decimal),
            delimiter,
        };
        return continues_list(
            &list,
            &next,
            meta.source_markers
                .as_ref()
                .and_then(|markers| markers.first())
                .map(String::as_str),
        );
    }

    previous.marker.is_some() && previous.marker == meta.marker
}

/// The context a list is written in.
#[derive(Clone, Copy)]
struct ListContext {
    /// Whether a blank line has to separate the list from what came before.
    needs_paragraph_separator: bool,
    /// Whether the list follows a tight list.
    follows_tight_list: bool,
}

impl Writer {
    /// Write a list, preserving the markers the author used.
    fn write_list_block(
        &mut self,
        state: &Block,
        result: &mut Vec<String>,
        indent: &str,
        list_indent: &str,
        previous_list: Option<WrittenList>,
        context: ListContext,
    ) -> WrittenList {
        let ListContext {
            needs_paragraph_separator,
            follows_tight_list,
        } = context;
        let mut insert_new_line = self.is_loose_parent_list || needs_paragraph_separator;
        self.is_loose_parent_list = true;

        let mut meta = state.list.clone().unwrap_or_default();
        // The original markers only survive while the list still holds the
        // items it was parsed with.
        if state.kind != BlockKind::BulletList && !self.should_preserve_order_markers(state) {
            meta.source_markers = None;
        }

        let would_merge_lists = previous_list
            .as_ref()
            .map(|previous| would_merge_adjacent_lists(&previous.meta, &meta))
            .unwrap_or(false);

        if previous_list.is_some() && !would_merge_lists {
            insert_new_line = false;
        }

        // Adjacent tight lists with the same marker are one Markdown list (the
        // parser splits it where items switch between plain and task), so a
        // blank line would only make that list loose on the next parse.
        if would_merge_lists && follows_tight_list {
            insert_new_line = false;
        }

        // A blank line cannot keep two lists apart when their markers would
        // merge on reparse, so an HTML comment separates them instead.
        let needs_list_separator = self.options.fancy_lists
            && would_merge_lists
            && previous_list
                .as_ref()
                .map(|previous| previous.name == state.kind)
                .unwrap_or(false)
            && meta.delimiter.is_some();
        if needs_list_separator {
            self.insert_line_break(result, indent);
            result.push(format!("{indent}<!-- -->\n"));
            insert_new_line = true;
        }

        if insert_new_line {
            self.insert_line_break(result, indent);
        }

        self.list_type.push(meta.clone());
        result.push(self.write_list(state, indent, list_indent));
        self.list_type.pop();

        WrittenList {
            name: state.kind.clone(),
            meta,
        }
    }

    /// Whether the list still holds the items its original markers describe.
    fn should_preserve_order_markers(&self, state: &Block) -> bool {
        let Some(source_markers) = state
            .list
            .as_ref()
            .and_then(|meta| meta.source_markers.as_ref())
        else {
            return false;
        };
        if source_markers.len() != state.children().len() {
            return false;
        }
        state.children().iter().enumerate().all(|(index, child)| {
            child.order_marker.as_deref() == Some(source_markers[index].as_str())
        })
    }

    /// Whether a list needs a blank line before it.
    fn list_needs_paragraph_separator(&self, state: &Block) -> bool {
        let meta = state.list.clone().unwrap_or_default();

        // CommonMark §5.2 requires a blank line before empty items or non-1
        // decimal markers.
        if state.kind != BlockKind::BulletList {
            if let Some(start) = meta.start {
                let style = meta.style.unwrap_or(ListStyle::Decimal);
                if style != ListStyle::Decimal || meta.delimiter == Some(ListDelimiter::ParenBoth) {
                    return false;
                }
                let marker = if self.should_preserve_order_markers(state) {
                    meta.source_markers
                        .as_ref()
                        .and_then(|markers| markers.first())
                        .cloned()
                } else {
                    None
                };
                let needs = match marker {
                    Some(marker) => !is_simple_decimal_marker(&marker),
                    None => start != 1,
                };
                if needs {
                    return true;
                }
            }
        }
        if state.kind == BlockKind::TaskList {
            return false;
        }
        let first = state
            .children()
            .first()
            .and_then(|item| item.children().first());
        match first {
            None => true,
            Some(child) => child.kind == BlockKind::Paragraph && child.text().trim().is_empty(),
        }
    }

    /// Write the items of a list.
    fn write_list(&mut self, state: &Block, indent: &str, list_indent: &str) -> String {
        self.write_blocks(state.children(), indent, list_indent, false)
    }

    /// Write a single list item.
    fn write_list_item_block(
        &mut self,
        state: &Block,
        result: &mut Vec<String>,
        indent: &str,
        list_indent: &str,
        first_item: bool,
    ) {
        let loose = self
            .list_type
            .last()
            .map(|meta| meta.loose)
            .unwrap_or(false);

        self.is_loose_parent_list = loose;
        if loose {
            self.insert_line_break(result, indent);
        }

        // An item of a tight list keeps its direct children blank-line free
        // unless a pair of them genuinely needs a separator.
        result.push(self.write_list_item(
            state,
            &format!("{indent}{list_indent}"),
            !loose,
            first_item,
        ));
        self.is_loose_parent_list = true;
    }

    /// Write a list item with its marker and its content.
    fn write_list_item(
        &mut self,
        state: &Block,
        indent: &str,
        is_tight_item: bool,
        first_item: bool,
    ) -> String {
        let children = state.children();
        let list_info = self.list_type.last().cloned().unwrap_or_default();

        let mut item_marker = if let Some(marker) = &list_info.marker {
            format!("{marker} ")
        } else if list_info.start.is_some() {
            let first_text = if state.kind == BlockKind::ListItem
                && children
                    .first()
                    .map(|c| c.kind == BlockKind::Paragraph)
                    .unwrap_or(false)
            {
                children[0].text()
            } else {
                ""
            };
            self.ordered_item_marker(first_text, first_item)
        } else {
            "- ".to_string()
        };

        let (indent, new_indent) = self.list_item_indent(indent, &item_marker);
        // Additional nesting indentation beyond the parent item's content column.
        let list_indent = " ".repeat(self.list_indentation_count.saturating_sub(1));

        if state.kind == BlockKind::TaskListItem {
            item_marker.push_str(if state.checked == Some(true) {
                "[x] "
            } else {
                "[ ] "
            });
        }

        if children.is_empty() {
            return format!("{indent}{item_marker}\n");
        }

        let content = self.write_blocks(children, &new_indent, &list_indent, is_tight_item);
        let first_content = if content.starts_with(&new_indent) {
            content[new_indent.len()..].to_string()
        } else {
            content
        };
        format!("{indent}{item_marker}{first_content}")
    }

    /// The next item's marker with the spaces after it; advances the list count.
    fn ordered_item_marker(&mut self, first_text: &str, first_item: bool) -> String {
        let Some(list_info) = self.list_type.last_mut() else {
            return "- ".to_string();
        };

        let style = list_info.style.unwrap_or(ListStyle::Decimal);
        let preserved = list_info.source_markers.as_mut().and_then(|markers| {
            if markers.is_empty() {
                None
            } else {
                Some(markers.remove(0))
            }
        });

        let mut n = list_info.start.unwrap_or(1);
        list_info.start = Some(n + 1);
        // The list count is capped so that the marker never grows too wide.
        if style == ListStyle::Decimal && n > 999_999_999 {
            n = 1;
        }

        let delimiter = list_info.delimiter.unwrap_or(ListDelimiter::Dot);
        let order_marker = preserved
            .as_ref()
            .and_then(|marker| parse_list_marker(marker, Some(style)))
            .unwrap_or(ListMarker {
                value: n,
                style,
                delimiter,
            });

        let text = preserved.unwrap_or_else(|| {
            format_list_marker(
                &order_marker,
                if first_item {
                    MarkerPosition::First
                } else {
                    MarkerPosition::Continuation
                },
            )
        });
        let spaces = min_spaces_after_marker(&order_marker, first_text);
        format!("{text}{}", " ".repeat(spaces))
    }

    /// Compute the indentation of an item's content.
    ///
    /// The content column sits right behind the marker, so a nested list
    /// continues at the parent's content column.
    fn list_item_indent(&self, indent: &str, item_marker: &str) -> (String, String) {
        let indent = indent.to_string();
        let new_indent = format!("{}{}", indent, " ".repeat(item_marker.chars().count()));
        (indent, new_indent)
    }

    /// Insert a blank line, unless the output is still empty.
    fn insert_line_break(&self, result: &mut Vec<String>, indent: &str) {
        if result.is_empty() {
            return;
        }
        // A blank line inside a list item must not carry trailing whitespace.
        // For blockquote-style indents like `> ` the `>` is kept so the quote
        // stays continuous — only the trailing run of plain spaces is dropped.
        let trimmed = indent.trim_end_matches(' ');
        result.push(format!("{trimmed}\n"));
    }
}

/// Whether a marker is a plain `1.` or `1)` decimal marker.
fn is_simple_decimal_marker(marker: &str) -> bool {
    let bytes = marker.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_digit() && (bytes[1] == b'.' || bytes[1] == b')')
}

impl Writer {
    /// Write a block that has no special list handling.
    fn write_simple_block(
        &mut self,
        state: &Block,
        result: &mut Vec<String>,
        indent: &str,
        separation: Separation,
    ) {
        // A frontmatter block is always first, so it needs no leading break.
        if state.kind != BlockKind::Frontmatter && separation != Separation::Adjacent {
            self.insert_line_break(result, indent);
        }

        match state.kind {
            BlockKind::Frontmatter => result.push(write_frontmatter(state)),
            BlockKind::Paragraph | BlockKind::ThematicBreak => {
                result.push(write_text_paragraph(state, indent))
            }
            BlockKind::AtxHeading => result.push(format!("{indent}{}\n", state.text().trim_end())),
            BlockKind::SetextHeading => {
                let underline = state.underline.as_deref().unwrap_or("---");
                for line in state.text().split('\n') {
                    result.push(format!("{indent}{line}\n"));
                }
                result.push(format!("{indent}{underline}\n"));
            }
            BlockKind::CodeBlock => result.push(write_code_block(state, indent)),
            BlockKind::HtmlBlock => {
                for line in state.text().split('\n') {
                    result.push(format!("{indent}{line}\n"));
                }
            }
            BlockKind::MathBlock => result.push(write_math_block(state, indent)),
            BlockKind::Diagram => result.push(write_diagram_block(state, indent)),
            BlockKind::BlockQuote => {
                let inner = self.write_blocks(state.children(), &format!("{indent}> "), "", false);
                result.push(inner);
            }
            BlockKind::GithubAlert => {
                let prefix = format!("{indent}> ");
                let body = self.write_blocks(state.children(), &prefix, "", false);
                let marker = state.alert_marker.clone().unwrap_or_else(|| {
                    format!("[!{}]", state.alert_type.as_deref().unwrap_or("NOTE"))
                });
                result.push(format!("{prefix}{marker}\n"));
                if state.blank_line_after_marker == Some(true) && !body.trim().is_empty() {
                    result.push(format!("{prefix}\n"));
                }
                result.push(body);
            }
            BlockKind::Table => result.push(write_table(state, indent)),
            BlockKind::Footnote => {
                // The first line sits flush after the prefix; the rest keeps
                // the four-space continuation indent.
                let identifier = state.identifier.as_deref().unwrap_or("");
                let inner_indent = format!("{indent}    ");
                let inner = self.write_blocks(state.children(), &inner_indent, "", false);
                let stripped = inner.replacen(&inner_indent, "", 1);
                result.push(format!("{indent}[^{identifier}]: {stripped}"));
            }
            _ => {}
        }
    }
}

/// Write a paragraph or a thematic break.
fn write_text_paragraph(block: &Block, indent: &str) -> String {
    let mut result = String::new();
    for line in block.text().split('\n') {
        result.push_str(&format!("{indent}{line}\n"));
    }
    result
}

/// Write a frontmatter block.
fn write_frontmatter(block: &Block) -> String {
    let lang = block.frontmatter_lang.as_deref().unwrap_or("yaml");
    let style = block.frontmatter_style.as_deref().unwrap_or("-");
    let (start, end) = match (lang, style) {
        ("toml", _) => ("+++", "+++"),
        ("json", ";") => (";;;", ";;;"),
        ("json", _) => ("{", "}"),
        _ => ("---", "---"),
    };
    let mut result = String::new();
    result.push_str(start);
    result.push('\n');
    for line in block.text().split('\n') {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(end);
    result.push('\n');
    result
}

/// Write a code block, choosing a fence that cannot be closed early.
fn write_code_block(block: &Block, indent: &str) -> String {
    let mut result = String::new();
    let text = block.text();
    let kind = block.code_kind.unwrap_or(CodeBlockKind::Fenced);

    if kind == CodeBlockKind::Indented {
        for line in text.split('\n') {
            result.push_str(&format!("{indent}    {line}\n"));
        }
        return result;
    }

    let info = block.lang.clone().unwrap_or_default();
    // A backtick fence's info string may not contain a backtick
    // (CommonMark §4.5), so fall back to a tilde fence rather than emit
    // markdown that would not read back as a code block.
    let fence_char = if block.fence_char == Some(FenceChar::Tilde) || info.contains('`') {
        FenceChar::Tilde
    } else {
        FenceChar::Backtick
    };
    let length = code_fence_length(text, block.fence_length, fence_char);
    let fence: String = fence_char.as_char().to_string().repeat(length);

    if info.is_empty() {
        result.push_str(&format!("{indent}{fence}\n"));
    } else {
        result.push_str(&format!("{indent}{fence}{info}\n"));
    }
    // `split` yields one empty line for an empty body; writing it would add a
    // blank line that was not in canonical Markdown.
    if !text.is_empty() {
        for line in text.split('\n') {
            result.push_str(&format!("{indent}{line}\n"));
        }
    }
    result.push_str(&format!("{indent}{fence}\n"));
    result
}

/// The fence length a code block needs.
///
/// The opening fence must be longer than any interior line made only of the
/// same fence character, at least as long as the original fence, and never
/// shorter than the Markdown minimum of three.
fn code_fence_length(text: &str, stored: Option<usize>, fence_char: FenceChar) -> usize {
    let marker = fence_char.as_char();
    let mut longest_interior = 0usize;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() && trimmed.chars().all(|c| c == marker) {
            longest_interior = longest_interior.max(trimmed.chars().count());
        }
    }
    3.max(stored.unwrap_or(3)).max(longest_interior + 1)
}

/// Write a math block.
fn write_math_block(block: &Block, indent: &str) -> String {
    let mut result = String::new();
    let style = block.math_style.unwrap_or(MathStyle::Dollars);
    let (open, close) = match style {
        MathStyle::Gfm => ("```math", "```"),
        MathStyle::Dollars => ("$$", "$$"),
    };
    result.push_str(&format!("{indent}{open}\n"));
    for line in block.text().split('\n') {
        result.push_str(&format!("{indent}{line}\n"));
    }
    result.push_str(&format!("{indent}{close}\n"));
    result
}

/// Write a diagram block.
fn write_diagram_block(block: &Block, indent: &str) -> String {
    let mut result = String::new();
    let text = block.text();
    let diagram_type = block.diagram_type.as_deref().unwrap_or("mermaid");
    // A backtick fence's info string may not contain a backtick, and the
    // opening fence has to be longer than any run of the fence character in
    // the body; otherwise a diagram whose body contains a fence would be
    // written as markdown that no longer reads back as one diagram.
    let fence_char = if diagram_type.contains('`') {
        FenceChar::Tilde
    } else {
        FenceChar::Backtick
    };
    let fence: String = fence_char
        .as_char()
        .to_string()
        .repeat(code_fence_length(text, None, fence_char));
    result.push_str(&format!("{indent}{fence}{diagram_type}\n"));
    for line in text.split('\n') {
        result.push_str(&format!("{indent}{line}\n"));
    }
    result.push_str(&format!("{indent}{fence}\n"));
    result
}

/// Write a GFM table.
///
/// The cells are re-aligned to the widest column, which is what MarkText does
/// as well: the table is edited as a grid, so the source spacing is not
/// preserved.
fn write_table(block: &Block, indent: &str) -> String {
    let rows: Vec<Vec<String>> = block
        .children()
        .iter()
        .map(|row| {
            row.children()
                .iter()
                .map(|cell| escape_table_cell(cell.text().trim()))
                .collect()
        })
        .collect();
    if rows.is_empty() {
        return String::new();
    }

    // The column width is the widest cell plus the padding around it.
    let columns = block.children()[0].children().len();
    let mut widths = vec![5usize; columns];
    for row in &rows {
        for (index, cell) in row.iter().enumerate().take(columns) {
            widths[index] = widths[index].max(display_width(cell) + 2);
        }
    }

    let mut result = String::new();
    for (row_index, row) in rows.iter().enumerate() {
        let mut line = format!("{indent}|");
        for (index, width) in widths.iter().enumerate() {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            let fill = width.saturating_sub(1).saturating_sub(display_width(cell));
            line.push(' ');
            line.push_str(cell);
            for _ in 0..fill {
                line.push(' ');
            }
            line.push('|');
        }
        // Cells beyond the header's columns are ignored by GFM renderers, but
        // they are part of the source and have to be written back.
        for cell in row.iter().skip(columns) {
            line.push(' ');
            line.push_str(cell);
            line.push_str(" |");
        }
        result.push_str(&line);
        result.push('\n');

        if row_index == 0 {
            let mut delimiter = format!("{indent}|");
            for (index, width) in widths.iter().enumerate() {
                let align = block.children()[0]
                    .children()
                    .get(index)
                    .and_then(|cell| cell.align)
                    .unwrap_or(Align::None);
                let raw = "-".repeat(width.saturating_sub(2));
                let cell = match align {
                    Align::Left => format!(":{raw} "),
                    Align::Center => format!(":{raw}:"),
                    Align::Right => format!(" {raw}:"),
                    Align::None => format!(" {raw} "),
                };
                delimiter.push_str(&cell);
                delimiter.push('|');
            }
            result.push_str(&delimiter);
            result.push('\n');
        }
    }
    result
}

/// Escape a pipe inside a table cell.
fn escape_table_cell(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            result.push('\\');
            if let Some(next) = chars.next() {
                result.push(next);
            }
            continue;
        }
        if ch == '|' {
            result.push_str("\\|");
            continue;
        }
        result.push(ch);
    }
    result
}

/// The display width of a string, counting wide characters as two columns.
fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| {
            let code = ch as u32;
            if (0x1100..=0x115F).contains(&code)
                || (0x2E80..=0xA4CF).contains(&code)
                || (0xAC00..=0xD7A3).contains(&code)
                || (0xF900..=0xFAFF).contains(&code)
                || (0xFE30..=0xFE6F).contains(&code)
                || (0xFF00..=0xFF60).contains(&code)
                || (0xFFE0..=0xFFE6).contains(&code)
            {
                2
            } else {
                1
            }
        })
        .sum()
}
