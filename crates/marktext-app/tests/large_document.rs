//! A headless layout of a large document.
//!
//! The test measures how much memory and how much time the editor needs to
//! show a document of a few megabytes. It runs without a window, so the
//! numbers are the ones of the layout itself and not of the window around it.
//!
//! The document is named by the environment variable `MARKTEXT_LARGE_DOCUMENT`;
//! without it the test is skipped, because the file is not part of the
//! repository.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use marktext_app::large::{DocumentScale, SourceChunks};
use marktext_app::preview;
use marktext_app::theme::Theme;

/// An allocator that keeps count of the memory that is in use.
///
/// The peak tells how much the layout of a document costs, which is the
/// number that decides whether a large document can be shown at all.
struct Tracking;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(new_size, Ordering::Relaxed) + new_size;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

/// The memory that is in use, in bytes.
fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// The largest amount of memory that was in use at the same time, in bytes.
fn peak_bytes() -> usize {
    PEAK.load(Ordering::Relaxed)
}

/// A frame of input with a window of a realistic size.
///
/// Without a screen rectangle the panels have no height, and a test that
/// measures what is drawn would measure an empty window.
fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(900.0, 700.0),
        )),
        predicted_dt: 1.0 / 60.0,
        events,
        ..Default::default()
    }
}

#[test]
fn a_large_document_is_laid_out_in_windows() {
    let Ok(path) = std::env::var("MARKTEXT_LARGE_DOCUMENT") else {
        eprintln!("跳过：没有设置 MARKTEXT_LARGE_DOCUMENT");
        return;
    };
    let source = std::fs::read_to_string(&path).expect("读取文档");
    println!("文档：{} 字节", source.len());

    // The document has to be recognized as a large one, otherwise the editor
    // would lay it out in one piece.
    let scale = DocumentScale::of(&source);
    assert_eq!(scale, DocumentScale::Large, "文档应当按大文档处理");

    let start = Instant::now();
    let document = marktext_core::parse_document(&source).expect("解析文档");
    println!(
        "解析：{:?}，{} 个块",
        start.elapsed(),
        document.blocks.len()
    );

    let chunks = SourceChunks::new(&source);
    println!("分块：{} 块", chunks.pieces().len());

    let palette = Theme::Light.palette();
    let options = marktext_core::InlineOptions::default();
    let widths = 600.0_f32;

    // The heights are worked out once, like the editor does.
    let start = Instant::now();
    let heights = preview::estimate_block_heights(&document.blocks, widths);
    println!(
        "估算高度：{:?}，总高 {:.0} 像素",
        start.elapsed(),
        heights.iter().sum::<f32>()
    );

    // The memory the editor holds before the layout starts.
    let before = live_bytes();

    let ctx = egui::Context::default();
    // The fonts are installed, so that the numbers match the ones of the
    // running editor.
    marktext_app::theme::install_fonts(&ctx);
    let mut headings = Vec::new();
    let start = Instant::now();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(widths);
            let _ = preview::draw_blocks_windowed(
                ui,
                &document.blocks,
                &heights,
                &palette,
                &options,
                &mut headings,
            );
        });
    });
    let elapsed = start.elapsed();
    let after = live_bytes();

    println!("布局一帧：{elapsed:?}");
    println!("布局新增内存：{} KB", (after.saturating_sub(before)) / 1024);
    println!("峰值内存：{} MB", peak_bytes() / 1024 / 1024);
    println!("画出的标题数：{}", headings.len());

    // The layout has to be quick enough to keep up with the frame rate, and
    // the memory has to stay far below what a full layout of the document
    // would need.
    assert!(elapsed.as_millis() < 500, "一帧的布局耗时过长：{elapsed:?}");
    assert!(
        peak_bytes() < 512 * 1024 * 1024,
        "布局用掉的内存过多：{} MB",
        peak_bytes() / 1024 / 1024
    );
}

#[test]
fn the_source_of_a_large_document_is_drawn_in_chunks() {
    let Ok(path) = std::env::var("MARKTEXT_LARGE_DOCUMENT") else {
        eprintln!("跳过：没有设置 MARKTEXT_LARGE_DOCUMENT");
        return;
    };
    let source = std::fs::read_to_string(&path).expect("读取文档");
    let mut chunks = SourceChunks::new(&source);

    let ctx = egui::Context::default();
    marktext_app::theme::install_fonts(&ctx);
    let before = live_bytes();
    let start = Instant::now();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(600.0);
            marktext_app::large::draw_chunks(ui, &mut chunks);
        });
    });
    let elapsed = start.elapsed();
    let after = live_bytes();

    println!("源码区布局一帧：{elapsed:?}");
    println!(
        "源码区新增内存：{} KB",
        (after.saturating_sub(before)) / 1024
    );
    println!("源码区峰值内存：{} MB", peak_bytes() / 1024 / 1024);

    assert!(
        elapsed.as_millis() < 500,
        "源码区一帧的布局耗时过长：{elapsed:?}"
    );
    assert!(
        peak_bytes() < 512 * 1024 * 1024,
        "源码区用掉的内存过多：{} MB",
        peak_bytes() / 1024 / 1024
    );
}

#[test]
fn the_outline_of_a_large_document_is_drawn_in_windows() {
    let Ok(path) = std::env::var("MARKTEXT_LARGE_DOCUMENT") else {
        eprintln!("跳过：没有设置 MARKTEXT_LARGE_DOCUMENT");
        return;
    };
    let source = std::fs::read_to_string(&path).expect("读取文档");
    let document = marktext_core::parse_document(&source).expect("解析文档");
    let toc = marktext_core::build_toc(&document.blocks, "");
    println!("大纲条目：{}", toc.len());

    let palette = Theme::Light.palette();
    let ctx = egui::Context::default();
    marktext_app::theme::install_fonts(&ctx);
    let before = live_bytes();
    let start = Instant::now();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::SidePanel::left("sidebar").show(ctx, |ui| {
            marktext_app::app::draw_toc(ui, &toc, &palette);
        });
    });
    let elapsed = start.elapsed();
    let after = live_bytes();

    println!("大纲布局一帧：{elapsed:?}");
    println!("大纲新增内存：{} KB", (after.saturating_sub(before)) / 1024);

    assert!(
        elapsed.as_millis() < 500,
        "大纲一帧的布局耗时过长：{elapsed:?}"
    );
    assert!(
        after.saturating_sub(before) < 128 * 1024 * 1024,
        "大纲用掉的内存过多：{} MB",
        (after.saturating_sub(before)) / 1024 / 1024
    );
}

#[test]
fn only_the_blocks_on_screen_are_drawn() {
    // A document of many blocks, far more than fit on the screen.
    let mut source = String::new();
    for i in 0..2000 {
        source.push_str(&format!("## 小节 {i}\n\n第 {i} 段的正文。\n\n"));
    }
    let document = marktext_core::parse_document(&source).expect("解析文档");
    let heights = preview::estimate_block_heights(&document.blocks, 600.0);
    let palette = Theme::Light.palette();
    let options = marktext_core::InlineOptions::default();

    let ctx = egui::Context::default();
    marktext_app::theme::install_fonts(&ctx);

    let mut drawn = 0usize;
    let mut headings = Vec::new();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(600.0);
            drawn = preview::draw_blocks_windowed(
                ui,
                &document.blocks,
                &heights,
                &palette,
                &options,
                &mut headings,
            );
        });
    });

    println!("总块数：{}，画出的块数：{}", document.blocks.len(), drawn);
    assert!(drawn > 0, "应当画出一些块");
    assert!(
        drawn < document.blocks.len() / 10,
        "画出的块太多了：{drawn} / {}",
        document.blocks.len()
    );
    // The first heading of the document is the one on screen.
    assert_eq!(
        headings.first().map(|(text, _)| text.as_str()),
        Some("小节 0")
    );
}

#[test]
fn the_visible_blocks_follow_the_scroll_position() {
    // A hundred blocks of a hundred pixels each.
    let heights = vec![100.0_f32; 100];

    // A window of three hundred pixels at the top of the document.
    assert_eq!(preview::visible_block_range(&heights, 0.0, 300.0), 0..4);

    // The same window a thousand pixels further down.
    assert_eq!(
        preview::visible_block_range(&heights, 1000.0, 1300.0),
        9..14
    );

    // A window below the end of the document stays inside it.
    assert_eq!(
        preview::visible_block_range(&heights, 9900.0, 10200.0),
        98..100
    );

    // An empty document has no blocks to show.
    assert_eq!(preview::visible_block_range(&[], 0.0, 300.0), 0..0);
}

#[test]
fn only_a_few_blocks_are_drawn_of_a_long_document() {
    let mut source = String::new();
    for i in 0..2000 {
        source.push_str(&format!(
            "## 小节 {i}

第 {i} 段的正文。

"
        ));
    }
    let document = marktext_core::parse_document(&source).expect("解析文档");
    let heights = preview::estimate_block_heights(&document.blocks, 600.0);
    let palette = Theme::Light.palette();
    let options = marktext_core::InlineOptions::default();

    let ctx = egui::Context::default();
    marktext_app::theme::install_fonts(&ctx);

    let mut drawn = 0usize;
    let mut headings = Vec::new();
    let _ = ctx.run(input(Vec::new()), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(600.0);
            drawn = preview::draw_blocks_windowed(
                ui,
                &document.blocks,
                &heights,
                &palette,
                &options,
                &mut headings,
            );
        });
    });

    println!("总块数：{}，画出的块数：{}", document.blocks.len(), drawn);
    assert!(drawn > 0, "应当画出一些块");
    assert!(
        drawn * 4 < document.blocks.len(),
        "画出的块太多了：{drawn} / {}",
        document.blocks.len()
    );
}

#[test]
fn a_small_document_is_laid_out_as_a_whole() {
    let source = "# 标题\n\n正文。\n";
    let document = marktext_core::parse_document(source).expect("解析文档");
    assert_eq!(DocumentScale::of(source), DocumentScale::Small);

    let palette = Theme::Light.palette();
    let options = marktext_core::InlineOptions::default();
    let heights = preview::estimate_block_heights(&document.blocks, 600.0);

    let ctx = egui::Context::default();
    let mut headings = Vec::new();
    let _ = ctx.run(input(Vec::new()), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(600.0);
            let _ = preview::draw_blocks_windowed(
                ui,
                &document.blocks,
                &heights,
                &palette,
                &options,
                &mut headings,
            );
        });
    });
    assert_eq!(headings.len(), 1, "标题应当被找到");
}
