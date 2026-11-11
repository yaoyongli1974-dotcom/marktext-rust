//! MarkText — the desktop markdown editor, ported to Rust.
//!
//! The window shows a Markdown document twice: as source on the left and as
//! the rendered preview on the right. Both come out of the same engine, the
//! `marktext-core` crate, which is a port of MarkText's `packages/muya`.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use marktext_app::app::{MarkTextApp, APP_NAME};

fn main() -> ExitCode {
    // A panic must never take the window down silently. The release build on
    // Windows runs without a console, so the panic hook is the only place
    // where the user can be told what went wrong.
    install_panic_hook();

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
            marktext_app::theme::install_fonts(&cc.egui_ctx);
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

/// Report a panic before the process dies.
///
/// The release build is compiled with `panic = "abort"`, so the process ends
/// right after the hook returns. Without a hook the user would only see the
/// window vanish, with no hint of what went wrong.
fn install_panic_hook() {
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);

        // The original hook writes the message and the backtrace to stderr.
        previous(info);

        report_failure(&message);
    }));
}

/// The text of a panic, without the location noise.
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "内部错误".to_string()
    }
}
