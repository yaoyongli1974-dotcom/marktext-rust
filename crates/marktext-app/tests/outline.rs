//! Tests for the outline navigation of the windowed preview.
//!
//! The windowed preview only draws the blocks that are on screen, so the
//! positions collected during a layout do not cover the whole document. A
//! heading further down has to be located through the cached block heights,
//! otherwise clicking it in the outline does nothing.

use marktext_app::preview::estimated_heading_offset;
use marktext_core::parse_document;

fn blocks(source: &str) -> Vec<marktext_core::Block> {
    parse_document(source).expect("valid markdown").blocks
}

#[test]
fn a_top_level_heading_is_placed_at_its_block_offset() {
    let blocks = blocks("# First\n\nbody\n\n## Second\n\ntail\n");
    let heights = vec![30.0, 20.0, 26.0, 20.0];
    assert_eq!(
        estimated_heading_offset(&blocks, &heights, "First"),
        Some(0.0)
    );
    assert_eq!(
        estimated_heading_offset(&blocks, &heights, "Second"),
        Some(50.0)
    );
}

#[test]
fn a_heading_nested_in_a_quote_scrolls_to_the_quote() {
    let blocks = blocks("body\n\n> ## Inside\n>\n> text\n");
    let heights = vec![20.0, 44.0];
    assert_eq!(
        estimated_heading_offset(&blocks, &heights, "Inside"),
        Some(20.0)
    );
}

#[test]
fn an_unknown_heading_has_no_offset() {
    let blocks = blocks("# First\n");
    assert_eq!(estimated_heading_offset(&blocks, &[30.0], "Missing"), None);
}
