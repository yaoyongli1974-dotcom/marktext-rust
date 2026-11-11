//! MarkText — the desktop markdown editor, ported to Rust.
//!
//! The window shows a Markdown document twice: as source on the left and as
//! the rendered preview on the right. Both come out of the same engine, the
//! `marktext-core` crate, which is a port of MarkText's `packages/muya`.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod preview;
mod theme;

use std::path::PathBuf;
use std::process::ExitCode;

use app::{MarkTextApp, APP_NAME};

fn main() -> ExitCode {
    // The first argument is the file to open, the same way the original
    // accepts a path on the command line.
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|path| path.exists());

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title(APP_NAME)
            .with_drag_and_drop(true),
        ..Default::default()
    };

    let result = eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            let mut app = MarkTextApp::new();
            if let Some(path) = path {
                app.open_path(&path);
            }
            Ok(Box::new(app))
        }),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // The release build on Windows has no console window, so the error
            // has to reach the user through a dialog.
            report_failure(&error.to_string());
            ExitCode::FAILURE
        }
    }
}

/// Tell the user that the editor could not start.
///
/// A dialog is used because the release build on Windows runs without a
/// console window; on the other platforms the message also goes to stderr.
fn report_failure(message: &str) {
    eprintln!("{APP_NAME}: {message}");

    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(APP_NAME)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
