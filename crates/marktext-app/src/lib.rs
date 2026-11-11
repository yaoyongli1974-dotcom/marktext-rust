//! The desktop editor, split into a library and a thin binary.
//!
//! The library holds the parts that can be tested without a window: the
//! layout of the editor, the palette, and the helpers that turn a cursor
//! position into a line number. The binary only starts the window.

pub mod app;
pub mod large;
pub mod preview;
pub mod theme;

/// The line number the cursor sits on.
///
/// The cursor index that egui reports counts *characters*, not bytes, so it
/// has to be turned into a byte offset before the text can be sliced.
/// Slicing at a byte that is not on a character boundary panics, which is
/// what used to make the editor vanish while Chinese text was edited.
pub fn line_of_cursor(source: &str, cursor: usize) -> usize {
    let offset = source
        .char_indices()
        .nth(cursor)
        .map(|(offset, _)| offset)
        .unwrap_or(source.len());
    source[..offset].matches('\n').count() + 1
}
