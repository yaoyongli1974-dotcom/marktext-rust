//! Tests for pandoc-style fancy ordered-list markers.
//!
//! They are opt-in because `A. Smith` at the start of a line is prose in most
//! documents. With the option on the marker has to be a valid alphabetic or
//! roman numeral and has to be followed by whitespace, just like the decimal
//! markers.

use marktext_core::{parse_with_options, BlockKind, ListStyle, ParseOptions};

fn parse_lists(source: &str) -> Vec<marktext_core::Block> {
    let options = ParseOptions {
        fancy_lists: true,
        ..ParseOptions::default()
    };
    parse_with_options(source, options).expect("valid markdown")
}

#[test]
fn alphabetic_markers_start_a_list() {
    let blocks = parse_lists("a. one\nb. two\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, BlockKind::OrderList);
    let list = blocks[0].list.as_ref().expect("list metadata");
    assert_eq!(list.style, Some(ListStyle::LowerAlpha));
    assert_eq!(blocks[0].children().len(), 2);
}

#[test]
fn roman_markers_continue_by_their_style() {
    let blocks = parse_lists("i. one\nii. two\niii. three\n");
    assert_eq!(blocks.len(), 1);
    let list = blocks[0].list.as_ref().expect("list metadata");
    assert_eq!(list.style, Some(ListStyle::LowerRoman));
    assert_eq!(blocks[0].children().len(), 3);
}

#[test]
fn parenthesized_alphabetic_markers_are_accepted() {
    let blocks = parse_lists("(a) one\n(b) two\n");
    assert_eq!(blocks[0].kind, BlockKind::OrderList);
    assert_eq!(blocks[0].children().len(), 2);
}

#[test]
fn the_markers_are_off_by_default() {
    let blocks = parse_with_options("A. Smith\n", ParseOptions::default()).expect("valid markdown");
    assert_eq!(blocks[0].kind, BlockKind::Paragraph);
}

#[test]
fn a_word_followed_by_a_period_is_not_a_marker() {
    // `Auto.` is not a numeral, and the option must not turn it into a list.
    let blocks = parse_lists("Auto. repair\n");
    assert_eq!(blocks[0].kind, BlockKind::Paragraph);
}

#[test]
fn the_source_markers_are_written_back() {
    use marktext_core::{parse_document_with_options, serialize_document, ExportOptions};
    let options = ParseOptions {
        fancy_lists: true,
        ..ParseOptions::default()
    };
    let source = "a. one\nb. two\n";
    let document = parse_document_with_options(source, options).expect("valid markdown");
    let output = serialize_document(&document, ExportOptions::default());
    assert!(output.contains("a. one"), "{output}");
    assert!(output.contains("b. two"), "{output}");
}
