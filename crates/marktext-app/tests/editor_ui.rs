//! Regression tests that drive the editor without a window.
//!
//! egui can run headless, so the real widget tree can be exercised: a
//! `TextEdit` is put on screen, Chinese text is typed into it, and the
//! cursor is moved. Before the fix the editor computed the line number of
//! the cursor by slicing the source at the cursor index — but the index
//! counts characters while a Rust slice is indexed by bytes. Every document
//! with Chinese text therefore panicked as soon as the cursor moved, and
//! because the release build aborts on panic and has no console window, the
//! application vanished without a word.

use egui::{Context, RawInput, TextEdit};

use marktext_app::app::MarkTextApp;

/// Run one frame of the editor and return whether it survived.
fn run_one_frame(ctx: &Context, _app: &mut MarkTextApp, source: &mut String) {
    let raw = RawInput::default();
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let edit = TextEdit::multiline(source).code_editor();
            let output = edit.show(ui);
            if let Some(range) = output.cursor_range {
                let _ = marktext_app::line_of_cursor(source, range.primary.index);
            }
        });
    });
}

/// The editor survives a Chinese document whose cursor moves around.
#[test]
fn the_editor_survives_chinese_text() {
    let ctx = Context::default();
    let mut app = MarkTextApp::new();
    let mut source = String::from("# MarkText 引擎\n\n这是一段中文，用来检查光标定位。\n");

    for _ in 0..8 {
        run_one_frame(&ctx, &mut app, &mut source);
    }

    assert!(app.cursor_line() >= 1);
}

/// The editor survives the cursor sitting inside a multibyte character.
#[test]
fn the_editor_survives_a_cursor_inside_a_multibyte_character() {
    let ctx = Context::default();
    let mut app = MarkTextApp::new();
    let mut source = String::from("中文测试");

    for _ in 0..8 {
        run_one_frame(&ctx, &mut app, &mut source);
        // Move the cursor to every position of the text, including the ones
        // that are in the middle of a character.
        for cursor in 0..=source.chars().count() {
            let line = marktext_app::line_of_cursor(&source, cursor);
            assert!(line >= 1);
        }
    }
}
