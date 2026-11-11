//! Regression tests for the helpers that can be tested without a window.
//!
//! The editor used to compute the line number of the cursor by slicing the
//! source at the cursor index. The index that egui reports counts
//! *characters*, while a Rust slice is indexed by *bytes*, so any document
//! with Chinese text made the editor panic — and since the release build
//! aborts on panic and has no console window, the application simply
//! vanished.

use marktext_app::line_of_cursor;

#[test]
fn counts_the_lines_of_an_ascii_document() {
    let source = "first\nsecond\nthird";
    assert_eq!(line_of_cursor(source, 0), 1);
    assert_eq!(line_of_cursor(source, 6), 2);
    assert_eq!(line_of_cursor(source, 13), 3);
}

#[test]
fn counts_the_lines_of_a_document_with_chinese_text() {
    let source = "第一行\n第二行中文\n第三行";
    // The cursor counts characters, so the index of the second line's start
    // is four characters in — the three characters of the first line and the
    // newline — and the third line starts ten characters in.
    assert_eq!(line_of_cursor(source, 0), 1);
    assert_eq!(line_of_cursor(source, 4), 2);
    assert_eq!(line_of_cursor(source, 10), 3);
}

#[test]
fn a_cursor_inside_a_multibyte_character_does_not_panic() {
    let source = "中文";
    // Every index between zero and the length of the text is valid, even
    // though only some of them are on a character boundary.
    for cursor in 0..=source.chars().count() {
        let line = line_of_cursor(source, cursor);
        assert_eq!(line, 1);
    }
}

#[test]
fn a_cursor_behind_the_end_of_the_text_stays_on_the_last_line() {
    let source = "one\ntwo";
    assert_eq!(line_of_cursor(source, 100), 2);
}

#[test]
fn an_empty_document_has_one_line() {
    assert_eq!(line_of_cursor("", 0), 1);
}
