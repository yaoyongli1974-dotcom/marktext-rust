//! The preview pane: the block tree drawn as formatted text.
//!
//! MarkText renders its preview by turning the block tree into DOM nodes.
//! The Rust port does the same thing, only with egui widgets instead of DOM
//! nodes: every block kind has a function here that draws the matching
//! widget, and the inline syntax is drawn with the token stream the engine
//! produces.

use egui::{Align, Color32, Layout, RichText, Sense, Stroke, TextFormat, Ui, Vec2};
use marktext_core::document::{Align as CellAlign, Block, BlockKind};
use marktext_core::inline::{Inline, InlineOptions};

use crate::theme::Palette;

/// The size of the body text in the preview.
pub const BODY_SIZE: f32 = 15.0;

/// Draw the block tree into `ui`.
///
/// `scroll_to` names a heading whose text should be scrolled into view; the
/// pane remembers where every heading was drawn and jumps there the next time
/// the document is laid out.
pub fn draw_blocks(
    ui: &mut Ui,
    blocks: &[Block],
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
) {
    for block in blocks {
        draw_block(ui, block, palette, inline_options, headings, 0);
    }
}

/// Draw one block and its children.
fn draw_block(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    match block.kind {
        BlockKind::Paragraph => {
            draw_paragraph(ui, block, palette, inline_options);
        }
        BlockKind::AtxHeading | BlockKind::SetextHeading => {
            draw_heading(ui, block, palette, inline_options, headings);
        }
        BlockKind::ThematicBreak => {
            ui.add_space(6.0);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, palette.rule);
            ui.add_space(6.0);
        }
        BlockKind::CodeBlock => {
            draw_code_block(ui, block, palette);
        }
        BlockKind::HtmlBlock => {
            draw_raw_block(ui, block.text(), palette);
        }
        BlockKind::BlockQuote => {
            draw_quote(ui, block, palette, inline_options, headings, depth);
        }
        BlockKind::GithubAlert => {
            draw_alert(ui, block, palette, inline_options, headings, depth);
        }
        BlockKind::BulletList | BlockKind::OrderList | BlockKind::TaskList => {
            draw_list(ui, block, palette, inline_options, headings, depth);
        }
        BlockKind::ListItem | BlockKind::TaskListItem => {
            draw_list_item(ui, block, palette, inline_options, headings, depth);
        }
        BlockKind::Table => {
            draw_table(ui, block, palette, inline_options);
        }
        BlockKind::MathBlock => {
            draw_math(ui, block, palette);
        }
        BlockKind::Frontmatter => {
            draw_frontmatter(ui, block, palette);
        }
        BlockKind::Diagram => {
            draw_diagram(ui, block, palette);
        }
        BlockKind::Footnote => {
            draw_footnote(ui, block, palette, inline_options, headings, depth);
        }
        BlockKind::TableRow | BlockKind::TableCell => {}
    }
}

/// Draw a paragraph.
fn draw_paragraph(ui: &mut Ui, block: &Block, palette: &Palette, inline_options: &InlineOptions) {
    let text = block.text();
    if text.is_empty() {
        ui.add_space(4.0);
        return;
    }
    let mut job = inline_job(text, palette, inline_options, InlineStyle::body());
    job.wrap.max_width = ui.available_width();
    ui.add_space(4.0);
    ui.label(egui::WidgetText::from(job));
    ui.add_space(4.0);
}

/// Draw a heading and remember where it was drawn.
fn draw_heading(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
) {
    let level = block.level.unwrap_or(1).clamp(1, 6);
    let size = match level {
        1 => 26.0,
        2 => 22.0,
        3 => 19.0,
        4 => 17.0,
        5 => 16.0,
        _ => 15.0,
    };
    let text = heading_text(block);
    let mut job = inline_job(
        &text,
        palette,
        inline_options,
        InlineStyle {
            size,
            bold: true,
            ..InlineStyle::body()
        },
    );
    job.wrap.max_width = ui.available_width();

    ui.add_space(10.0);
    let response = ui.label(egui::WidgetText::from(job));
    ui.add_space(4.0);

    // The heading is anchored by its plain text, which is what the outline
    // panel shows as well.
    headings.push((text, response.rect.min.to_vec2()));
}

/// The text of a heading without its markers.
fn heading_text(block: &Block) -> String {
    let text = block.text();
    match block.kind {
        BlockKind::AtxHeading => {
            let hashes = text.chars().take_while(|c| *c == '#').count();
            text[hashes..].trim().to_string()
        }
        _ => text.trim().to_string(),
    }
}

/// Draw a code block.
fn draw_code_block(ui: &mut Ui, block: &Block, palette: &Palette) {
    let language = marktext_core::clean_info_string(&block.lang.clone().unwrap_or_default());
    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.code_background)
        .stroke(Stroke::new(1.0_f32, palette.code_border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10));

    frame.show(ui, |ui| {
        if !language.is_empty() {
            ui.label(
                RichText::new(language)
                    .small()
                    .weak()
                    .color(palette.weak_text),
            );
            ui.add_space(2.0);
        }
        // The code block is drawn verbatim — no inline syntax is applied to it.
        let format = format_for(InlineStyle::code(BODY_SIZE), palette);
        let mut job = egui::text::LayoutJob::default();
        for line in block.text().split('\n') {
            job.append(line, 0.0, format.clone());
        }
        job.wrap.max_width = ui.available_width();
        ui.label(egui::WidgetText::from(job));
    });
    ui.add_space(8.0);
}

/// Draw a block of raw HTML as monospaced text.
fn draw_raw_block(ui: &mut Ui, text: &str, palette: &Palette) {
    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.code_background)
        .stroke(Stroke::new(1.0_f32, palette.code_border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        for line in text.split('\n') {
            ui.label(RichText::new(line).monospace().color(palette.weak_text));
        }
    });
    ui.add_space(8.0);
}

/// Draw a block quote with its left bar.
fn draw_quote(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    let indent = 12.0;
    let content_width = (ui.available_width() - indent).max(40.0);

    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 0.0), Sense::hover());
        // The bar is drawn after the content, so its height is patched below.
        let bar_rect = rect;
        ui.add_space(0.0);
        ui.allocate_ui_with_layout(
            Vec2::new(content_width, 0.0),
            Layout::top_down(Align::LEFT),
            |ui| {
                ui.set_width(content_width);
                for child in block.children() {
                    draw_block(ui, child, palette, inline_options, headings, depth + 1);
                }
            },
        );
        let bar_rect = egui::Rect::from_min_max(
            bar_rect.min,
            egui::pos2(bar_rect.max.x, ui.min_rect().bottom()),
        );
        ui.painter().rect_filled(bar_rect, 0.0, palette.quote_bar);
    });
    ui.add_space(8.0);
}

/// Draw a GitHub alert.
fn draw_alert(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    let alert_type = block.alert_type.as_deref().unwrap_or("NOTE").to_lowercase();
    let color = alert_color(&alert_type, palette);

    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.panel)
        .stroke(Stroke::new(1.0_f32, color))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        ui.label(
            RichText::new(alert_type.to_uppercase())
                .strong()
                .color(color),
        );
        ui.add_space(4.0);
        for child in block.children() {
            draw_block(ui, child, palette, inline_options, headings, depth + 1);
        }
    });
    ui.add_space(8.0);
}

/// The colour a GitHub alert is drawn in.
fn alert_color(alert_type: &str, palette: &Palette) -> Color32 {
    match alert_type {
        "note" => Color32::from_rgb(31, 111, 235),
        "tip" => Color32::from_rgb(26, 127, 55),
        "important" => Color32::from_rgb(191, 135, 0),
        "warning" => Color32::from_rgb(191, 106, 0),
        "caution" => Color32::from_rgb(191, 32, 32),
        _ => palette.accent,
    }
}

/// Draw a list and its items.
fn draw_list(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    let meta = block.list.clone().unwrap_or_default();
    let mut number = meta.start.unwrap_or(1);
    let mut style = meta
        .style
        .unwrap_or(marktext_core::document::ListStyle::Decimal);

    for child in block.children() {
        let marker = match block.kind {
            BlockKind::OrderList => ordered_marker(number, style),
            BlockKind::TaskList => "☑".to_string(),
            _ => meta.marker.clone().unwrap_or_else(|| "-".to_string()),
        };
        if matches!(block.kind, BlockKind::OrderList) {
            number += 1;
            let (_, parsed) = marker_parts(&marker);
            style = parsed;
        }
        draw_list_item_with_marker(ui, child, &marker, palette, inline_options, headings, depth);
    }
    ui.add_space(4.0);
}

/// The marker of an ordered list item.
fn ordered_marker(number: u64, style: marktext_core::document::ListStyle) -> String {
    use marktext_core::document::ListStyle;
    let text = match style {
        ListStyle::LowerAlpha => alpha(number, false),
        ListStyle::UpperAlpha => alpha(number, true),
        ListStyle::LowerRoman => roman(number, false),
        ListStyle::UpperRoman => roman(number, true),
        _ => number.to_string(),
    };
    match style {
        ListStyle::LowerAlpha | ListStyle::UpperAlpha => format!("({text})"),
        _ => format!("{text}."),
    }
}

/// The style implied by a marker, so a list can change its numbering.
fn marker_parts(marker: &str) -> (u64, marktext_core::document::ListStyle) {
    use marktext_core::document::ListStyle;
    let trimmed = marker.trim_end_matches(['.', ')']);
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return (trimmed.parse().unwrap_or(1), ListStyle::Decimal);
    }
    if trimmed.chars().all(|c| c.is_ascii_alphabetic()) && trimmed.len() == 1 {
        let upper = trimmed.chars().next().unwrap().is_ascii_uppercase();
        let value = (trimmed.chars().next().unwrap() as u8 - b'a' + 1) as u64;
        return (
            value,
            if upper {
                ListStyle::UpperAlpha
            } else {
                ListStyle::LowerAlpha
            },
        );
    }
    if trimmed.chars().all(|c| c.is_ascii_uppercase()) {
        if let Some(value) = roman_value(trimmed) {
            return (value, ListStyle::UpperRoman);
        }
    }
    if trimmed.chars().all(|c| c.is_ascii_lowercase()) {
        if let Some(value) = roman_value(trimmed) {
            return (value, ListStyle::LowerRoman);
        }
    }
    (1, ListStyle::Decimal)
}

/// The number a Roman numeral stands for.
fn roman_value(text: &str) -> Option<u64> {
    let value = match text {
        "I" => 1,
        "II" => 2,
        "III" => 3,
        "IV" => 4,
        "V" => 5,
        "VI" => 6,
        "VII" => 7,
        "VIII" => 8,
        "IX" => 9,
        "X" => 10,
        "XI" => 11,
        "XII" => 12,
        "XIII" => 13,
        "XIV" => 14,
        "XV" => 15,
        "XVI" => 16,
        "XVII" => 17,
        "XVIII" => 18,
        "XIX" => 19,
        "XX" => 20,
        "XL" => 40,
        "L" => 50,
        "C" => 100,
        "D" => 500,
        "M" => 1000,
        _ => return None,
    };
    Some(value)
}

/// The letter of an alphabetic list marker.
fn alpha(number: u64, upper: bool) -> String {
    let index = ((number - 1) % 26) as u8;
    let letter = (b'a' + index) as char;
    if upper {
        letter.to_ascii_uppercase().to_string()
    } else {
        letter.to_string()
    }
}

/// The Roman numeral of an ordered list number.
fn roman(number: u64, upper: bool) -> String {
    const TABLE: &[(u64, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];

    let mut rest = number;
    let mut out = String::new();
    for (amount, symbol) in TABLE {
        while rest >= *amount {
            out.push_str(symbol);
            rest -= amount;
        }
    }
    if upper {
        out
    } else {
        out.to_lowercase()
    }
}

/// Draw a list item without a marker of its own.
fn draw_list_item(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    draw_list_item_with_marker(ui, block, "", palette, inline_options, headings, depth);
}

/// Draw a list item with the marker that introduces it.
fn draw_list_item_with_marker(
    ui: &mut Ui,
    block: &Block,
    marker: &str,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    // A task item shows a checkbox instead of the marker of the list.
    let is_task = block.kind == BlockKind::TaskListItem;
    let indent = 18.0;
    let content_width = (ui.available_width() - indent).max(40.0);

    ui.horizontal_top(|ui| {
        if is_task {
            let checked = block.checked == Some(true);
            let symbol = if checked { "☑" } else { "☐" };
            ui.label(RichText::new(symbol).color(palette.accent));
        } else if !marker.is_empty() {
            ui.label(RichText::new(marker).color(palette.weak_text));
        }
        ui.allocate_ui_with_layout(
            Vec2::new(content_width, 0.0),
            Layout::top_down(Align::LEFT),
            |ui| {
                ui.set_width(content_width);
                for child in block.children() {
                    draw_block(ui, child, palette, inline_options, headings, depth + 1);
                }
            },
        );
    });
}

/// Draw a table.
fn draw_table(ui: &mut Ui, block: &Block, palette: &Palette, inline_options: &InlineOptions) {
    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.background)
        .stroke(Stroke::new(1.0_f32, palette.table_border))
        .inner_margin(egui::Margin::same(8));
    frame.show(ui, |ui| {
        egui::Grid::new("preview_table")
            .striped(true)
            .spacing(egui::vec2(12.0, 6.0))
            .show(ui, |ui| {
                for (row_index, row) in block.children().iter().enumerate() {
                    let header = row_index == 0;
                    for cell in row.children() {
                        let align = cell.align.unwrap_or(CellAlign::None);
                        let mut job = inline_job(
                            cell.text(),
                            palette,
                            inline_options,
                            InlineStyle {
                                bold: header,
                                ..InlineStyle::body()
                            },
                        );
                        job.wrap.max_width = (ui.available_width() / 2.0).max(80.0);
                        job.halign = match align {
                            CellAlign::Center => Align::Center,
                            CellAlign::Right => Align::RIGHT,
                            _ => Align::LEFT,
                        };
                        let text = egui::WidgetText::from(job);
                        if header {
                            egui::Frame::new()
                                .fill(palette.table_header)
                                .inner_margin(egui::Margin::same(2))
                                .show(ui, |ui| {
                                    ui.label(text);
                                });
                        } else {
                            ui.label(text);
                        }
                    }
                    ui.end_row();
                }
            });
    });
    ui.add_space(8.0);
}

/// Draw a math block.
fn draw_math(ui: &mut Ui, block: &Block, palette: &Palette) {
    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.code_background)
        .stroke(Stroke::new(1.0_f32, palette.code_border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        for line in block.text().split('\n') {
            ui.label(RichText::new(line).monospace().color(palette.accent));
        }
    });
    ui.add_space(8.0);
}

/// Draw the frontmatter block.
fn draw_frontmatter(ui: &mut Ui, block: &Block, palette: &Palette) {
    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.code_background)
        .stroke(Stroke::new(1.0_f32, palette.code_border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        let language = block.frontmatter_lang.as_deref().unwrap_or("yaml");
        ui.label(
            RichText::new(language)
                .small()
                .weak()
                .color(palette.weak_text),
        );
        ui.add_space(2.0);
        for line in block.text().split('\n') {
            ui.label(RichText::new(line).monospace().color(palette.weak_text));
        }
    });
    ui.add_space(8.0);
}

/// Draw a diagram block, which the engine cannot render on its own.
fn draw_diagram(ui: &mut Ui, block: &Block, palette: &Palette) {
    let diagram_type = block.diagram_type.as_deref().unwrap_or("mermaid");
    let mut frame = egui::Frame::new();
    frame = frame
        .fill(palette.code_background)
        .stroke(Stroke::new(1.0_f32, palette.code_border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        ui.label(
            RichText::new(format!("{diagram_type} 图"))
                .small()
                .weak()
                .color(palette.weak_text),
        );
        ui.add_space(2.0);
        for line in block.text().split('\n') {
            ui.label(RichText::new(line).monospace().color(palette.text));
        }
    });
    ui.add_space(8.0);
}

/// Draw a footnote.
fn draw_footnote(
    ui: &mut Ui,
    block: &Block,
    palette: &Palette,
    inline_options: &InlineOptions,
    headings: &mut Vec<(String, Vec2)>,
    depth: usize,
) {
    let identifier = block.identifier.as_deref().unwrap_or("");
    ui.horizontal_top(|ui| {
        ui.label(
            RichText::new(format!("[{identifier}]"))
                .strong()
                .color(palette.accent),
        );
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 0.0),
            Layout::top_down(Align::LEFT),
            |ui| {
                for child in block.children() {
                    draw_block(ui, child, palette, inline_options, headings, depth + 1);
                }
            },
        );
    });
}

/// The style applied to a run of inline text.
#[derive(Debug, Clone, Copy)]
pub struct InlineStyle {
    /// The font size in points.
    pub size: f32,
    /// Whether the text is bold.
    pub bold: bool,
    /// Whether the text is italic.
    pub italic: bool,
    /// Whether the text is struck through.
    pub strike: bool,
    /// Whether the text is highlighted.
    pub highlight: bool,
    /// Whether the text is code.
    pub code: bool,
    /// Whether the text is a link.
    pub link: bool,
}

impl InlineStyle {
    /// The style of ordinary body text.
    pub fn body() -> InlineStyle {
        InlineStyle {
            size: BODY_SIZE,
            bold: false,
            italic: false,
            strike: false,
            highlight: false,
            code: false,
            link: false,
        }
    }

    /// The style of code, which uses the monospace family.
    pub fn code(size: f32) -> InlineStyle {
        InlineStyle {
            size: size - 1.0,
            code: true,
            ..InlineStyle::body()
        }
    }
}

/// Build a layout job out of the inline token stream of `text`.
fn inline_job(
    text: &str,
    palette: &Palette,
    options: &InlineOptions,
    style: InlineStyle,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let tokens = marktext_core::inline::parse(text, options.clone());
    for token in &tokens {
        append_token(&mut job, token, palette, style);
    }
    if job.text.is_empty() {
        job.append(" ", 0.0, format_for(style, palette));
    }
    job
}

/// Append one inline token to the layout job.
fn append_token(
    job: &mut egui::text::LayoutJob,
    token: &Inline,
    palette: &Palette,
    style: InlineStyle,
) {
    match token {
        Inline::Text { raw } => {
            job.append(raw, 0.0, format_for(style, palette));
        }
        Inline::Emphasis { children, .. } => {
            let child_style = InlineStyle {
                italic: true,
                ..style
            };
            append_children(job, children, palette, child_style);
        }
        Inline::Strong { children, .. } => {
            let child_style = InlineStyle {
                bold: true,
                ..style
            };
            append_children(job, children, palette, child_style);
        }
        Inline::Del { children, .. } => {
            let child_style = InlineStyle {
                strike: true,
                ..style
            };
            append_children(job, children, palette, child_style);
        }
        Inline::Mark { children, .. } => {
            let child_style = InlineStyle {
                highlight: true,
                ..style
            };
            append_children(job, children, palette, child_style);
        }
        Inline::InlineCode { content, .. } => {
            let code_style = InlineStyle {
                code: true,
                ..style
            };
            job.append(content, 0.0, format_for(code_style, palette));
        }
        Inline::InlineMath { content, .. } => {
            let math_style = InlineStyle {
                code: true,
                ..style
            };
            job.append(content, 0.0, format_for(math_style, palette));
        }
        Inline::Emoji { content } => {
            job.append(content, 0.0, format_for(style, palette));
        }
        Inline::Link { children, .. } => {
            let link_style = InlineStyle {
                link: true,
                ..style
            };
            append_children(job, children, palette, link_style);
        }
        Inline::ReferenceLink { children, .. } => {
            let link_style = InlineStyle {
                link: true,
                ..style
            };
            append_children(job, children, palette, link_style);
        }
        Inline::Image { alt, src, .. } => {
            let image_style = InlineStyle {
                code: true,
                ..style
            };
            job.append(
                &format!("[{alt}]({src})"),
                0.0,
                format_for(image_style, palette),
            );
        }
        Inline::ReferenceImage { alt, label } => {
            let image_style = InlineStyle {
                code: true,
                ..style
            };
            job.append(
                &format!("[{alt}][{label}]"),
                0.0,
                format_for(image_style, palette),
            );
        }
        Inline::AutoLink { href, .. } => {
            let link_style = InlineStyle {
                link: true,
                ..style
            };
            job.append(href, 0.0, format_for(link_style, palette));
        }
        Inline::AutoLinkExtension { kind, text } => {
            // The extended autolink is shown as the text it was written as.
            let link_style = InlineStyle {
                link: true,
                ..style
            };
            let _ = kind;
            job.append(text, 0.0, format_for(link_style, palette));
        }
        Inline::HtmlTag { raw } => {
            let tag_style = InlineStyle {
                code: true,
                ..style
            };
            job.append(raw, 0.0, format_for(tag_style, palette));
        }
        Inline::HtmlEscape { raw, .. } => {
            job.append(raw, 0.0, format_for(style, palette));
        }
        Inline::Escape { content, .. } => {
            job.append(content, 0.0, format_for(style, palette));
        }
        Inline::HardBreak { raw } => {
            job.append(raw, 0.0, format_for(style, palette));
        }
        Inline::SoftBreak { raw } => {
            job.append(raw, 0.0, format_for(style, palette));
        }
        Inline::Footnote { identifier } => {
            let footnote_style = InlineStyle {
                code: true,
                ..style
            };
            job.append(
                &format!("[^{identifier}]"),
                0.0,
                format_for(footnote_style, palette),
            );
        }
        Inline::Delim { marker, count, .. } => {
            let raw = marker.to_string().repeat(*count);
            job.append(&raw, 0.0, format_for(style, palette));
        }
    }
}

/// Append the children of an inline token.
fn append_children(
    job: &mut egui::text::LayoutJob,
    children: &[Inline],
    palette: &Palette,
    style: InlineStyle,
) {
    for child in children {
        append_token(job, child, palette, style);
    }
}

/// The text format a run of inline text is drawn with.
fn format_for(style: InlineStyle, palette: &Palette) -> TextFormat {
    let font_id = if style.code {
        crate::theme::code_font(style.size)
    } else {
        crate::theme::body_font(style.size)
    };

    let mut color = palette.text;
    if style.link {
        color = palette.accent;
    }
    if style.bold {
        color = color.gamma_multiply(1.0);
    }

    let mut format = TextFormat {
        font_id,
        color,
        italics: style.italic,
        ..Default::default()
    };

    if style.link {
        format.underline = Stroke::new(1.0_f32, palette.accent);
    }
    if style.strike {
        format.strikethrough = Stroke::new(1.0_f32, color);
    }
    if style.highlight {
        format.background = palette.accent.gamma_multiply(0.35);
        format.expand_bg = 1.0;
    }
    if style.code {
        format.background = palette.code_background;
        format.expand_bg = 1.0;
    }

    format
}

/// The font style used for the body text, exposed for the editor.
#[allow(dead_code)]
pub fn body_text_style() -> egui::TextStyle {
    egui::TextStyle::Body
}
