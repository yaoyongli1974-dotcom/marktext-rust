//! MarkText — the desktop markdown editor, ported to Rust.
//!
//! The window shows a Markdown document twice: as source on the left and as
//! the rendered preview on the right. Both come out of the same engine, the
//! `marktext-core` crate, which is a port of MarkText's `packages/muya`.

mod app;
mod preview;
mod theme;

use std::path::PathBuf;

use app::{MarkTextApp, APP_NAME};

fn main() -> eframe::Result {
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

    eframe::run_native(
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
    )
}
