//! Conformance suite against the upstream Markdown specifications.
//!
//! Two suites are checked:
//!
//! * **CommonMark 0.31.2** — 652 examples, the reference implementation's
//!   own test set.
//! * **GFM 0.29-gfm** — 672 examples, the `cmark-gfm` variant that adds
//!   tables, task lists, strikethrough and the GitHub autolink extension.
//!
//! The harness mirrors MarkText's original `packages/muya/test/spec`
//! suite: the rendered HTML is compared after normalisation, and the
//! examples that are known to fail are pinned in a *regression-lock* file.
//! An example on that list must keep failing — if it starts passing the
//! list has gone stale and the suite fails so the entry can be removed.
//!
//! Regenerate the lock file after an intentional engine change with:
//!
//! ```text
//! MARKTEXT_UPDATE_EXPECTED_FAILURES=1 cargo test -p marktext-core --test spec
//! ```

#[path = "spec/normalize.rs"]
mod normalize;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde::Deserialize;

use marktext_core::{
    parse_document_with_options, render_html, InlineOptions, ParseOptions, RenderOptions,
};

use normalize::normalize_html;

/// One example of a specification suite.
#[derive(Debug, Deserialize)]
struct SpecExample {
    /// The Markdown input.
    markdown: String,
    /// The expected HTML output.
    html: String,
    /// The section of the specification the example belongs to.
    section: String,
    /// The position of the example inside the specification.
    number: usize,
    /// The extensions the example needs.
    ///
    /// Only the GFM specification uses this: it marks an example with the
    /// extensions that have to be switched on for it, exactly the way
    /// `cmark-gfm`'s own runner reads the extension list off the example
    /// fence.  An empty list — the CommonMark case — means that no extension
    /// is enabled and the example has to be rendered as plain CommonMark.
    #[serde(default)]
    extensions: Vec<String>,
}

impl SpecExample {
    /// Whether the example is one the reference implementation skips.
    ///
    /// `cmark-gfm` skips an example marked `disabled`: the reference
    /// implementation does not produce the HTML the specification shows for
    /// it, so the example is not a conformance requirement.
    fn is_disabled(&self) -> bool {
        self.extensions.iter().any(|name| name == "disabled")
    }
}

/// The pinned set of examples that are known to fail.
#[derive(Debug, Default, Deserialize)]
struct ExpectedFailures {
    #[serde(default)]
    commonmark: BTreeSet<usize>,
    #[serde(default)]
    gfm: BTreeSet<usize>,
}

const COMMONMARK_FIXTURE: &str = include_str!("fixtures/commonmark-0.31.2.json");
const GFM_FIXTURE: &str = include_str!("fixtures/gfm-0.29.json");
const EXPECTED_FAILURES_FIXTURE: &str = include_str!("fixtures/expected-failures.json");

/// Where the lock file lives, relative to the crate root.
const EXPECTED_FAILURES_PATH: &str = "tests/fixtures/expected-failures.json";

/// The parser switches the specification suites run with.
///
/// Everything that is a MarkText extension rather than part of CommonMark
/// or GFM is switched off, so the suites measure the standard behaviour of
/// the engine and nothing else.
fn spec_parse_options() -> ParseOptions {
    ParseOptions {
        footnote: false,
        tex_math_dollars: false,
        tex_math_gfm: false,
        tex_math_single_backslash: false,
        tex_math_double_backslash: false,
        frontmatter: false,
        github_alerts: false,
        fancy_lists: false,
    }
}

/// The renderer switches the specification suites run with.
///
/// The extensions are switched on per example, exactly the way `cmark-gfm`'s
/// own runner does it: the GFM specification marks the examples that need an
/// extension with the extension's name on the example fence, and the runner
/// enables exactly those.  That is why the plain *Autolinks* section of the
/// GFM specification expects a bare `foo@bar.example.com` to stay text while
/// the *Autolinks (extension)* section expects it to become a link.
///
/// Everything that is a MarkText extension rather than part of CommonMark or
/// GFM stays switched off, so the suites measure the standard behaviour of
/// the engine and nothing else.
fn spec_render_options(extensions: &[String]) -> RenderOptions {
    let has = |name: &str| extensions.iter().any(|value| value == name);
    RenderOptions {
        allow_raw_html: true,
        gfm_tagfilter: has("tagfilter"),
        highlight_code: true,
        render_math: false,
        inline: InlineOptions {
            tex_math_dollars: false,
            tex_math_gfm: false,
            footnote: false,
            emoji: false,
            mark_syntax: false,
            inline_diff: false,
            super_sub_script: false,
            gfm_autolink: has("autolink"),
            references: Vec::new(),
        },
        references: Vec::new(),
    }
}

/// Render one example the way the specification suites expect.
fn render_example(example: &SpecExample) -> String {
    let document = parse_document_with_options(&example.markdown, spec_parse_options())
        .expect("the specification examples are valid Markdown");
    let mut options = spec_render_options(&example.extensions);
    options.references = document.references;
    render_html(&document.blocks, options)
}

/// The outcome of running one suite.
struct SuiteOutcome {
    /// The examples that failed.
    failures: BTreeSet<usize>,
    /// How many examples failed per section.
    by_section: BTreeMap<String, usize>,
    /// The first failure of every section, kept for the diagnostic dump.
    samples: BTreeMap<String, FailureSample>,
    /// Every failure, kept for the detailed diagnostic dump.
    all_samples: Vec<FailureSample>,
    /// The examples the reference implementation skips.
    skipped: usize,
}

/// One failure, kept so that the diagnostic dump can show the difference.
#[derive(Clone)]
struct FailureSample {
    number: usize,
    section: String,
    markdown: String,
    expected: String,
    actual: String,
}

/// Run a whole suite and collect the failures.
fn run_suite(examples: &[SpecExample]) -> SuiteOutcome {
    let mut failures = BTreeSet::new();
    let mut by_section: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: BTreeMap<String, FailureSample> = BTreeMap::new();
    let mut all_samples: Vec<FailureSample> = Vec::new();
    let mut skipped = 0usize;

    for example in examples {
        if example.is_disabled() {
            skipped += 1;
            continue;
        }
        let actual = render_example(example);
        if !normalize::compare_html(&actual, &example.html) {
            failures.insert(example.number);
            *by_section.entry(example.section.clone()).or_insert(0) += 1;
            let sample = FailureSample {
                number: example.number,
                section: example.section.clone(),
                markdown: example.markdown.clone(),
                expected: normalize_html(&example.html),
                actual: normalize_html(&actual),
            };
            samples
                .entry(example.section.clone())
                .or_insert_with(|| sample.clone());
            all_samples.push(sample);
        }
    }

    SuiteOutcome {
        failures,
        by_section,
        samples,
        all_samples,
        skipped,
    }
}

/// Print a short report for a suite.
fn report(name: &str, total: usize, outcome: &SuiteOutcome) {
    println!("── {name} ──");
    println!("  examples   : {total}");
    println!("  failures   : {}", outcome.failures.len());
    if outcome.skipped > 0 {
        println!(
            "  skipped    : {} (disabled by the reference implementation)",
            outcome.skipped
        );
    }
    if !outcome.by_section.is_empty() {
        println!("  by section :");
        for (section, count) in &outcome.by_section {
            println!("    {section:<40} {count}");
        }
    }
    dump_samples(name, outcome);
}

/// Write the failure samples to a file so that they can be inspected with a
/// UTF-8 aware editor — the console encoding of a Windows terminal mangles
/// the non-ASCII characters the specifications are full of.
///
/// `MARKTEXT_SPEC_DUMP_ALL=1` writes every failure instead of only the
/// first one of every section.  `MARKTEXT_SPEC_SECTION=<name>` narrows that
/// dump down to a single section.
fn dump_samples(name: &str, outcome: &SuiteOutcome) {
    if std::env::var("MARKTEXT_SPEC_DUMP").is_err() {
        return;
    }
    let mut body = String::new();
    for sample in outcome.samples.values() {
        body.push_str(&format!(
            "── #{} §{} ──\nmarkdown : {:?}\nexpected : {:?}\nactual   : {:?}\n\n",
            sample.number, sample.section, sample.markdown, sample.expected, sample.actual
        ));
    }
    let path = std::path::Path::new("target").join(format!("spec-dump-{name}.txt"));
    let _ = fs::create_dir_all("target");
    let _ = fs::write(path, body);

    let wants_all = std::env::var("MARKTEXT_SPEC_DUMP_ALL")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !wants_all {
        return;
    }
    let filter = std::env::var("MARKTEXT_SPEC_SECTION").ok();
    let mut body = String::new();
    for sample in &outcome.all_samples {
        if let Some(filter) = &filter {
            if !sample.section.contains(filter.as_str()) {
                continue;
            }
        }
        body.push_str(&format!(
            "── #{} §{} ──\nmarkdown : {:?}\nexpected : {:?}\nactual   : {:?}\n\n",
            sample.number, sample.section, sample.markdown, sample.expected, sample.actual
        ));
    }
    let path = std::path::Path::new("target").join(format!("spec-dump-all-{name}.txt"));
    let _ = fs::write(path, body);
}

/// The numbers that fail, formatted for the lock file.
fn format_numbers(numbers: &BTreeSet<usize>) -> String {
    numbers
        .iter()
        .map(|number| format!("{number}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Assert the regression-lock contract of a suite.
///
/// An example that is *not* on the lock file must pass.  An example that
/// *is* on the lock file must still fail — a passing example means the
/// lock file has gone stale and has to be pruned.
fn assert_locked(name: &str, outcome: &SuiteOutcome, locked: &BTreeSet<usize>) {
    let mut unexpected: Vec<usize> = Vec::new();
    for number in &outcome.failures {
        if !locked.contains(number) {
            unexpected.push(*number);
        }
    }
    let mut stale: Vec<usize> = Vec::new();
    for number in locked {
        if !outcome.failures.contains(number) {
            stale.push(*number);
        }
    }

    assert!(
        unexpected.is_empty(),
        "{name}: {} example(s) regressed and are not on the lock file: {}\n\
         Add them to {EXPECTED_FAILURES_PATH} or fix the engine.",
        unexpected.len(),
        format_numbers(&unexpected.iter().copied().collect())
    );
    assert!(
        stale.is_empty(),
        "{name}: {} example(s) on the lock file now pass and must be removed: {}\n\
         Remove them from {EXPECTED_FAILURES_PATH}.",
        stale.len(),
        format_numbers(&stale.iter().copied().collect())
    );
}

/// Write the lock file back to disk.
fn write_lock_file(commonmark: &BTreeSet<usize>, gfm: &BTreeSet<usize>) {
    let body = format!(
        "{{\n  \
         \"_comment\": \"Spec examples that the Rust engine fails. Listed example numbers are asserted to STILL fail by tests/spec.rs — if an example here starts passing, the suite reports an \\\"unexpected pass\\\" and the entry must be removed. Compliance can only go up.\",\n  \
         \"commonmark\": [{}],\n  \
         \"gfm\": [{}]\n\
         }}\n",
        format_numbers(commonmark),
        format_numbers(gfm),
    );
    fs::write(EXPECTED_FAILURES_PATH, body).expect("the lock file can be written");
}

/// Whether the lock file should be regenerated instead of asserted.
fn should_update_lock_file() -> bool {
    std::env::var("MARKTEXT_UPDATE_EXPECTED_FAILURES")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Parse a fixture file into its examples.
///
/// The specification fixtures write a tab as `→` (U+2192) so that the tab
/// survives a round trip through formats that would otherwise mangle it.
/// Both the Markdown input and the expected HTML are written that way, so
/// both have to be decoded before the example can be run.
fn parse_fixture(raw: &str) -> Vec<SpecExample> {
    let mut examples: Vec<SpecExample> =
        serde_json::from_str(raw).expect("the fixture is valid JSON");
    for example in &mut examples {
        example.markdown = decode_tab_markers(&example.markdown);
        example.html = decode_tab_markers(&example.html);
    }
    examples
}

/// The character the specification fixtures use for a tab.
const TAB_MARKER: char = '\u{2192}';

/// Replace every tab marker with a real tab.
fn decode_tab_markers(text: &str) -> String {
    text.replace(TAB_MARKER, "\t")
}

#[test]
fn commonmark_0_31_2_conformance() {
    let examples = parse_fixture(COMMONMARK_FIXTURE);
    let outcome = run_suite(&examples);
    report("CommonMark 0.31.2", examples.len(), &outcome);

    if should_update_lock_file() {
        let locked = ExpectedFailures {
            commonmark: outcome.failures.iter().copied().collect(),
            gfm: load_gfm_lock(),
        };
        write_lock_file(&outcome.failures, &locked.gfm);
        return;
    }

    let locked: BTreeSet<usize> = load_locked().commonmark.into_iter().collect();
    assert_locked("CommonMark 0.31.2", &outcome, &locked);
}

#[test]
fn gfm_0_29_conformance() {
    let examples = parse_fixture(GFM_FIXTURE);
    let outcome = run_suite(&examples);
    report("GFM 0.29-gfm", examples.len(), &outcome);

    if should_update_lock_file() {
        let locked = ExpectedFailures {
            commonmark: load_commonmark_lock(),
            gfm: outcome.failures.iter().copied().collect(),
        };
        write_lock_file(&locked.commonmark, &outcome.failures);
        return;
    }

    let locked: BTreeSet<usize> = load_locked().gfm.into_iter().collect();
    assert_locked("GFM 0.29-gfm", &outcome, &locked);
}

/// The pinned failures of both suites.
fn load_locked() -> ExpectedFailures {
    serde_json::from_str(EXPECTED_FAILURES_FIXTURE).expect("the lock file is valid JSON")
}

/// The pinned CommonMark failures.
fn load_commonmark_lock() -> BTreeSet<usize> {
    load_locked().commonmark.into_iter().collect()
}

/// The pinned GFM failures.
fn load_gfm_lock() -> BTreeSet<usize> {
    load_locked().gfm.into_iter().collect()
}
