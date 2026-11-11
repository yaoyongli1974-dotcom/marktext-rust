//! Support for documents that are too large to be laid out in one piece.
//!
//! egui lays out every widget it is handed, and its `TextEdit` turns the whole
//! text into a single layout job. A document of a few megabytes therefore
//! cannot be shown with the naive widgets: the layout of the whole document is
//! built on every frame, which costs gigabytes of memory and eventually
//! aborts the process.
//!
//! This module decides when a document has outgrown that treatment and keeps
//! the pieces the editor needs to show such a document in windows: the source
//! is cut into chunks, and the preview draws the blocks that are on screen
//! only.

/// The size at which a document is shown in windows.
pub const LARGE_DOCUMENT_BYTES: usize = 512 * 1024;

/// The number of lines at which a document is shown in windows.
pub const LARGE_DOCUMENT_LINES: usize = 20_000;

/// The number of bytes a source chunk aims for.
pub const CHUNK_BYTES: usize = 16 * 1024;

/// The number of lines a source chunk aims for.
pub const CHUNK_LINES: usize = 200;

/// How a document is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentScale {
    /// The document is small enough to be rendered as a whole.
    Small,
    /// The document is large, so only the visible window is rendered.
    Large,
}

impl DocumentScale {
    /// The scale a document with the given source needs.
    pub fn of(source: &str) -> DocumentScale {
        if source.len() >= LARGE_DOCUMENT_BYTES {
            return DocumentScale::Large;
        }
        // Counting the lines walks the whole text, so it is only done for
        // documents that are small enough for the walk to be cheap.
        if source.lines().count() >= LARGE_DOCUMENT_LINES {
            return DocumentScale::Large;
        }
        DocumentScale::Small
    }

    /// Whether the document is rendered in windows.
    pub fn is_large(self) -> bool {
        self == DocumentScale::Large
    }
}

/// The source of a document, cut into pieces small enough to lay out.
///
/// Every piece holds whole lines and no trailing line break, so joining the
/// pieces with a line break gives the source back unchanged. A piece that is
/// edited in place is written back into the same slot, which keeps the
/// mapping between pieces and lines stable even when the edit adds or
/// removes lines.
#[derive(Debug, Clone, Default)]
pub struct SourceChunks {
    pieces: Vec<String>,
}

impl SourceChunks {
    /// Cut `source` into chunks.
    pub fn new(source: &str) -> SourceChunks {
        let mut pieces = Vec::new();
        if source.is_empty() {
            return SourceChunks { pieces };
        }

        let mut current = String::new();
        let mut lines = 0usize;
        for line in source.split('\n') {
            if lines > 0 {
                current.push('\n');
            }
            current.push_str(line);
            lines += 1;
            if current.len() >= CHUNK_BYTES || lines >= CHUNK_LINES {
                pieces.push(std::mem::take(&mut current));
                lines = 0;
            }
        }
        if lines > 0 {
            pieces.push(current);
        }
        SourceChunks { pieces }
    }

    /// The chunks of the source.
    pub fn pieces(&self) -> &[String] {
        &self.pieces
    }

    /// The chunks of the source, for editing.
    pub fn pieces_mut(&mut self) -> &mut [String] {
        &mut self.pieces
    }

    /// The source the chunks stand for.
    pub fn source(&self) -> String {
        self.pieces.join("\n")
    }

    /// The line at which every chunk starts.
    pub fn line_offsets(&self) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(self.pieces.len());
        let mut line = 0usize;
        for piece in &self.pieces {
            offsets.push(line);
            line += count_lines(piece);
        }
        offsets
    }
}

/// The number of lines a piece of source holds.
pub fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// What the chunk editor has to say after a layout.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChunkEdit {
    /// Whether one of the chunks was edited.
    pub changed: bool,
    /// The chunk the cursor is in, the line that chunk starts at, and the
    /// position of the cursor inside the chunk.
    pub cursor: Option<(usize, usize, usize)>,
}

/// Draw the chunks of a large document, and only the ones on screen.
///
/// A text edit widget lays out the whole text it holds, so a document of a
/// few megabytes cannot be handed to one. The chunks are drawn one by one
/// instead, and only the ones that lie inside the visible window are laid
/// out at all.
pub fn draw_chunks(ui: &mut egui::Ui, chunks: &mut SourceChunks) -> ChunkEdit {
    let mut edit = ChunkEdit::default();
    let row_height = crate::preview::ROW_HEIGHT;

    egui::ScrollArea::vertical()
        .id_salt("source_scroll")
        .show(ui, |ui| {
            let width = ui.available_width();
            let offsets = chunks.line_offsets();
            let heights: Vec<f32> = chunks
                .pieces()
                .iter()
                .map(|piece| count_lines(piece) as f32 * row_height + 8.0)
                .collect();
            let total: f32 = heights.iter().sum();
            // The room the whole source takes up, so that the scroll bar knows
            // how far it reaches.
            ui.allocate_space(egui::vec2(width, total));

            let origin = ui.min_rect().min;
            let clip = ui.clip_rect();
            // A row of slack on both sides keeps a chunk from being skipped
            // when the estimate is a little off.
            let visible_top = clip.min.y - origin.y - row_height;
            let visible_bottom = clip.max.y - origin.y + row_height;

            let mut offset = 0.0f32;
            let mut first = None;
            let mut last = 0usize;
            for (index, height) in heights.iter().enumerate() {
                if first.is_none() && offset + height >= visible_top {
                    first = Some(index);
                }
                if offset <= visible_bottom {
                    last = index;
                }
                offset += height;
            }
            let first = first.unwrap_or(0);
            let last = last.min(chunks.pieces().len().saturating_sub(1));

            let mut y: f32 = heights[..first].iter().sum();
            for index in first..=last {
                let height = heights[index];
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(0.0, y),
                    egui::vec2(width, height),
                );
                let lines = count_lines(&chunks.pieces()[index]).max(1);
                let start_line = offsets[index];
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    // The text of the chunk is moved out for the time of the
                    // edit, so that it does not have to be copied every frame.
                    let mut text = std::mem::take(&mut chunks.pieces_mut()[index]);
                    let output = egui::TextEdit::multiline(&mut text)
                        .id_source(("source_chunk", index))
                        .code_editor()
                        .font(crate::theme::body_font(14.0))
                        .desired_width(f32::INFINITY)
                        .desired_rows(lines)
                        .frame(false)
                        .show(ui);
                    if output.response.changed() {
                        edit.changed = true;
                    }
                    if let Some(range) = output.cursor_range {
                        edit.cursor = Some((index, start_line, range.primary.index));
                    }
                    chunks.pieces_mut()[index] = text;
                });
                y += height;
            }
        });

    edit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_documents_stay_small() {
        assert_eq!(DocumentScale::of("hello"), DocumentScale::Small);
        assert_eq!(
            DocumentScale::of(&"x".repeat(LARGE_DOCUMENT_BYTES - 1)),
            DocumentScale::Small
        );
    }

    #[test]
    fn large_documents_are_detected_by_size() {
        assert_eq!(
            DocumentScale::of(&"x".repeat(LARGE_DOCUMENT_BYTES)),
            DocumentScale::Large
        );
    }

    #[test]
    fn large_documents_are_detected_by_line_count() {
        let source = "x\n".repeat(LARGE_DOCUMENT_LINES);
        assert_eq!(DocumentScale::of(&source), DocumentScale::Large);
    }

    #[test]
    fn chunks_join_back_to_the_source() {
        let source = "alpha\nbeta\ngamma\n\ndelta";
        let chunks = SourceChunks::new(source);
        assert_eq!(chunks.source(), source);
    }

    #[test]
    fn chunks_keep_every_line() {
        let source = (0..1000)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = SourceChunks::new(&source);
        assert_eq!(chunks.source(), source);
        // The chunks are small enough to be laid out on their own.
        assert!(chunks.pieces().len() > 1);
        for piece in chunks.pieces() {
            assert!(piece.len() < CHUNK_BYTES + 64);
        }
    }

    #[test]
    fn chunk_offsets_count_the_lines_before_them() {
        // More lines than a chunk holds, so that the source is cut into
        // several pieces.
        let source = (0..500)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = SourceChunks::new(&source);
        assert_eq!(chunks.pieces().len(), 3);
        assert_eq!(chunks.line_offsets(), vec![0, 200, 400]);
    }

    #[test]
    fn an_empty_source_has_no_chunks() {
        let chunks = SourceChunks::new("");
        assert!(chunks.pieces().is_empty());
        assert_eq!(chunks.source(), "");
    }

    #[test]
    fn a_chunk_that_grows_keeps_the_rest_intact() {
        let source = (0..500)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut chunks = SourceChunks::new(&source);

        // The second chunk grows by a line.
        chunks.pieces_mut()[1] = "grown\nlines".to_string();

        // The chunk before it is untouched.
        assert_eq!(
            chunks.pieces()[0],
            (0..200)
                .map(|i| format!("line{i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        // The chunks after it keep their text, but start further down.
        assert_eq!(chunks.line_offsets(), vec![0, 200, 202]);
        assert!(chunks.pieces()[2].starts_with("line400"));

        // The source still holds every part of the document.
        let joined = chunks.source();
        assert!(joined.starts_with("line0\n"));
        assert!(joined.contains("grown\nlines"));
        assert!(joined.ends_with("line499"));
    }
}
