//! Round-trip regression tests.
//!
//! The fixtures are the ones MarkText ships in
//! `packages/muya/test/spec/fixtures/marktext-round-trip`. They cover the
//! CommonMark basics and the GFM additions that MarkText supports.
//!
//! MarkText's own contract is *convergence* rather than identity: parsing and
//! serializing a document twice produces the same Markdown as parsing and
//! serializing it once. A document therefore only has to reach a fixed point,
//! not have to survive the first pass byte for byte.
//!
//! The tests below therefore check two things:
//!
//! 1. the round trip converges, which is the contract MarkText itself uses;
//! 2. the round trip keeps the document's structure, which catches a parser
//!    that silently drops a block.

use std::fs;
use std::path::{Path, PathBuf};

use marktext_core::{parse_document, serialize, BlockKind, ExportOptions};

/// The fixtures of this crate.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every fixture file, sorted by path so the output is stable.
fn fixture_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(&fixtures_dir(), &mut files);
    files.sort();
    files
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
}

/// Parse and serialize a document once.
fn round_trip(source: &str) -> String {
    let document = parse_document(source).expect("the fixture must parse");
    serialize(&document.blocks, ExportOptions::default())
}

/// Count the blocks of a document by kind.
fn count_kinds(source: &str) -> Vec<(BlockKind, usize)> {
    let document = parse_document(source).expect("the fixture must parse");
    let mut counts: Vec<(BlockKind, usize)> = Vec::new();
    collect_kinds(&document.blocks, &mut counts);
    counts
}

fn collect_kinds(blocks: &[marktext_core::Block], counts: &mut Vec<(BlockKind, usize)>) {
    for block in blocks {
        match counts.iter_mut().find(|(kind, _)| *kind == block.kind) {
            Some((_, count)) => *count += 1,
            None => counts.push((block.kind.clone(), 1)),
        }
        collect_kinds(block.children(), counts);
    }
}

#[test]
fn round_trip_converges_for_every_fixture() {
    let mut failures = Vec::new();

    for file in fixture_files() {
        let source = fs::read_to_string(&file).expect("the fixture must be readable");
        let first = round_trip(&source);
        let second = round_trip(&first);

        if first != second {
            let name = file
                .strip_prefix(fixtures_dir())
                .unwrap_or(&file)
                .display()
                .to_string();
            failures.push(format!(
                "{name}: 第一次往返的结果与第二次不同\n--- 第一次 ---\n{first}\n--- 第二次 ---\n{second}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "以下夹具的往返没有收敛：\n\n{}",
        failures.join("\n\n")
    );
}

#[test]
fn round_trip_keeps_the_block_structure() {
    let mut failures = Vec::new();

    for file in fixture_files() {
        let source = fs::read_to_string(&file).expect("the fixture must be readable");
        let first = round_trip(&source);
        let before = count_kinds(&source);
        let after = count_kinds(&first);

        if before != after {
            let name = file
                .strip_prefix(fixtures_dir())
                .unwrap_or(&file)
                .display()
                .to_string();
            failures.push(format!("{name}: 往返前 {before:?}，往返后 {after:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "以下夹具的往返改变了文档结构：\n\n{}",
        failures.join("\n")
    );
}
