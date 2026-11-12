//! Theme handling: the palette and the fonts.
//!
//! MarkText ships a couple of dozen CSS themes. The Rust port keeps the same
//! idea — a small palette object that the whole UI reads from — and ships
//! two built-in themes, a light one and a dark one, so the app is usable out
//! of the box.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak};

/// Whether [`install_fonts`] has registered the `bold` family.
///
/// A caller that never installs the app fonts (a test, or a library user that
/// only renders a preview) has no bold family, and asking egui for one would
/// panic, so [`bold_font`] falls back to the proportional family until the
/// family exists.
static BOLD_FAMILY_INSTALLED: AtomicBool = AtomicBool::new(false);

/// A colour palette used by every panel of the app.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The colour of the window background.
    pub background: Color32,
    /// The colour of the panels that sit on top of the background.
    pub panel: Color32,
    /// The colour of the text.
    pub text: Color32,
    /// The colour of dimmed text, such as placeholders.
    pub weak_text: Color32,
    /// The accent colour, used for links and the active selection.
    pub accent: Color32,
    /// The background of a code block.
    pub code_background: Color32,
    /// The border of a code block.
    pub code_border: Color32,
    /// The colour of a block quote's left bar.
    pub quote_bar: Color32,
    /// The background of a table header cell.
    pub table_header: Color32,
    /// The border of a table cell.
    pub table_border: Color32,
    /// The colour of a horizontal rule.
    pub rule: Color32,
}

impl Palette {
    /// The light theme, close to MarkText's default GitHub-like look.
    pub const LIGHT: Palette = Palette {
        background: Color32::from_rgb(255, 255, 255),
        panel: Color32::from_rgb(246, 248, 250),
        text: Color32::from_rgb(36, 41, 47),
        weak_text: Color32::from_rgb(140, 149, 159),
        accent: Color32::from_rgb(9, 105, 218),
        code_background: Color32::from_rgb(246, 248, 250),
        code_border: Color32::from_rgb(218, 224, 230),
        quote_bar: Color32::from_rgb(208, 215, 222),
        table_header: Color32::from_rgb(246, 248, 250),
        table_border: Color32::from_rgb(218, 224, 230),
        rule: Color32::from_rgb(218, 224, 230),
    };

    /// The dark theme, close to MarkText's `dark` theme.
    pub const DARK: Palette = Palette {
        background: Color32::from_rgb(24, 26, 31),
        panel: Color32::from_rgb(31, 34, 40),
        text: Color32::from_rgb(212, 217, 223),
        weak_text: Color32::from_rgb(125, 133, 144),
        accent: Color32::from_rgb(88, 166, 255),
        code_background: Color32::from_rgb(31, 34, 40),
        code_border: Color32::from_rgb(56, 61, 70),
        quote_bar: Color32::from_rgb(72, 79, 90),
        table_header: Color32::from_rgb(31, 34, 40),
        table_border: Color32::from_rgb(56, 61, 70),
        rule: Color32::from_rgb(56, 61, 70),
    };
}

/// The two themes the app can switch between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    /// The palette of this theme.
    pub fn palette(self) -> Palette {
        match self {
            Theme::Light => Palette::LIGHT,
            Theme::Dark => Palette::DARK,
        }
    }

    /// Whether this theme is the dark one.
    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }
}

/// The system fonts the app asks the operating system for.
///
/// Every entry is a pair of the file to read and the name to show for it.
/// The list is tried in order and the first one that exists is used. On
/// Windows the Chinese fonts are shipped with the system, so a Chinese
/// document renders correctly without any extra download.
const SYSTEM_FONT_CANDIDATES: &[(&str, &str)] = &[
    ("C:/Windows/Fonts/msyh.ttc", "Microsoft YaHei"),
    ("C:/Windows/Fonts/simhei.ttf", "SimHei"),
    ("C:/Windows/Fonts/simsun.ttc", "SimSun"),
    ("/System/Library/Fonts/PingFang.ttc", "PingFang SC"),
    (
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "Noto Sans CJK",
    ),
];

/// The font used for the code editor and for code blocks.
const MONOSPACE_CANDIDATES: &[(&str, &str)] = &[
    ("C:/Windows/Fonts/consola.ttf", "Consolas"),
    ("C:/Windows/Fonts/cour.ttf", "Courier New"),
    ("/System/Library/Fonts/Menlo.ttc", "Menlo"),
    (
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "DejaVu Sans Mono",
    ),
];

/// The bold fonts the app asks the operating system for.
///
/// egui's built-in fonts have no bold face, so strong text and headings would
/// otherwise be drawn with the same weight as the body. The family falls back
/// to the proportional family for every glyph the bold font does not cover
/// (a Latin-only bold font, for example, still leaves Chinese unemphasised).
const BOLD_FONT_CANDIDATES: &[(&str, &str)] = &[
    ("C:/Windows/Fonts/msyhbd.ttc", "Microsoft YaHei Bold"),
    ("C:/Windows/Fonts/arialbd.ttf", "Arial Bold"),
    (
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "Arial Bold",
    ),
    (
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
        "Noto Sans CJK Bold",
    ),
    (
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "DejaVu Sans Bold",
    ),
];

/// Install the fonts the app needs.
///
/// The proportional family gets a system CJK font appended so Chinese text
/// renders, the monospace family gets a system monospace font so code blocks
/// line up, and a `bold` family is registered so strong text and headings can
/// be drawn with a bold face. When a font cannot be found the app simply keeps
/// egui's built-in fonts, which still cover Latin text.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    match load_first(SYSTEM_FONT_CANDIDATES) {
        Some((name, data)) => {
            fonts
                .font_data
                .insert(name.clone(), Arc::new(FontData::from_owned(data)));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.push(name.clone());
                }
            }
        }
        None => {
            log("字体：没有找到任何可用的中文字体，中文将显示为方框");
        }
    }

    if let Some((name, data)) = load_first(MONOSPACE_CANDIDATES) {
        let tweaked = FontData::from_owned(data).tweak(FontTweak {
            scale: 1.0,
            // A monospace font sits a little lower than a proportional one,
            // so it is nudged down to line up with the body text.
            y_offset_factor: 0.0,
            y_offset: 0.0,
            baseline_offset_factor: 0.0,
        });
        fonts.font_data.insert(name.clone(), Arc::new(tweaked));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, name);
        }
    }

    // The bold family: the system bold font first, then every fallback of the
    // proportional family, so glyphs the bold font lacks (for example Chinese
    // in a Latin-only bold face) still render.
    let mut bold = Vec::new();
    if let Some((name, data)) = load_first(BOLD_FONT_CANDIDATES) {
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(data)));
        bold.push(name);
    }
    if let Some(proportional) = fonts.families.get(&FontFamily::Proportional) {
        bold.extend(proportional.iter().cloned());
    }
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    BOLD_FAMILY_INSTALLED.store(true, Ordering::Relaxed);

    log(&format!(
        "字体：Proportional 族 = {:?}",
        fonts.families.get(&FontFamily::Proportional)
    ));

    ctx.set_fonts(fonts);
}

/// Load the first candidate that exists on disk and can be read.
///
/// The result holds the name of the font and its bytes.
fn load_first(candidates: &[(&str, &str)]) -> Option<(String, Vec<u8>)> {
    for (path, name) in candidates {
        let path = std::path::Path::new(path);
        if !path.is_file() {
            continue;
        }
        match std::fs::read(path) {
            Ok(data) => {
                log(&format!("字体：已装载 {name}（{}）", path.display()));
                return Some(((*name).to_string(), data));
            }
            Err(error) => {
                log(&format!("字体：读取 {} 失败 —— {error}", path.display()));
            }
        }
    }
    None
}

/// Write a diagnostic message.
///
/// The release build on Windows has no console window, so the messages are
/// only useful while developing; they are dropped from the release build.
fn log(message: &str) {
    #[cfg(debug_assertions)]
    eprintln!("{message}");
    #[cfg(not(debug_assertions))]
    let _ = message;
}

/// The font id used for the body text.
pub fn body_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// The font id used for bold body text.
///
/// The family is always registered by [`install_fonts`]; when no bold font was
/// found it falls back to the proportional family and looks like normal text.
pub fn bold_font(size: f32) -> FontId {
    if BOLD_FAMILY_INSTALLED.load(Ordering::Relaxed) {
        FontId::new(size, FontFamily::Name("bold".into()))
    } else {
        body_font(size)
    }
}

/// The font id used for code.
pub fn code_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
