//! The desktop application.
//!
//! The layout follows MarkText's: a side bar with the outline on the left,
//! the document in the middle, and a status bar along the bottom. The
//! document itself is shown twice — as Markdown source on the left half and
//! as the rendered preview on the right half — which is the split view
//! MarkText offers as its "source code" mode.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{
    Align, Color32, Context, Key, Layout, Modifiers, RichText, ScrollArea, Sense, Stroke, TextEdit,
    Ui, Vec2,
};
use marktext_core::{
    build_toc, parse, search_document, Document, InlineOptions, ParseOptions, RenderOptions,
    SearchMatch, SearchOptions, TocItem,
};

use crate::large::{self, DocumentScale, SourceChunks};
use crate::preview;
use crate::theme::{self, Palette, Theme};

/// The name the window carries.
pub const APP_NAME: &str = "MarkText";

/// How long the editor waits before it parses a large document again.
///
/// Parsing a document of a few megabytes takes a moment, and doing that on
/// every keystroke would make the editor stutter. A small document is parsed
/// right away, because there the parse is cheaper than the delay.
const REPARSE_DELAY: Duration = Duration::from_millis(400);

/// The height of one row of the outline.
const TOC_ROW_HEIGHT: f32 = 22.0;

/// How the document area is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutMode {
    /// Only the Markdown source is shown.
    Source,
    /// The source and the preview sit next to each other.
    Split,
    /// Only the rendered preview is shown.
    Preview,
}

impl LayoutMode {
    /// The label of the mode in the menu bar.
    fn label(self) -> &'static str {
        match self {
            LayoutMode::Source => "源码",
            LayoutMode::Split => "分栏",
            LayoutMode::Preview => "预览",
        }
    }

    /// The next mode in the cycle.
    fn next(self) -> LayoutMode {
        match self {
            LayoutMode::Source => LayoutMode::Split,
            LayoutMode::Split => LayoutMode::Preview,
            LayoutMode::Preview => LayoutMode::Source,
        }
    }
}

/// How the "unsaved changes" dialog was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseDecision {
    /// Save the document, then close.
    Save,
    /// Close and throw the changes away.
    Discard,
    /// Keep editing.
    Cancel,
}

/// The state of the application.
pub struct MarkTextApp {
    /// The Markdown source of the open document.
    source: String,
    /// The path the document was loaded from, if any.
    path: Option<PathBuf>,
    /// Whether the document has unsaved changes.
    dirty: bool,
    /// The parsed document.
    document: Document,
    /// The outline of the document.
    toc: Vec<TocItem>,
    /// The current layout mode.
    layout: LayoutMode,
    /// The current theme.
    theme: Theme,
    /// Whether the side bar is shown.
    show_sidebar: bool,
    /// The search query of the side bar's search box.
    search_query: String,
    /// Whether the search box is expanded.
    search_open: bool,
    /// The paragraph the cursor is in, counted from one.
    cursor_line: usize,
    /// The status message shown in the status bar.
    status: String,
    /// Whether the last parse failed.
    parse_failed: bool,
    /// The vertical offsets of the headings of the last preview layout.
    heading_offsets: Vec<(String, Vec2)>,
    /// The heading the preview should scroll to.
    pending_scroll: Option<String>,
    /// The parser options.
    parse_options: ParseOptions,
    /// The inline options.
    inline_options: InlineOptions,
    /// How the document is rendered.
    scale: DocumentScale,
    /// The source cut into chunks, for a document that is too large to be laid
    /// out in one piece.
    chunks: SourceChunks,
    /// The estimated height of every block of the preview.
    preview_heights: Vec<f32>,
    /// The source revision the cached heights belong to.
    preview_heights_revision: u64,
    /// The width the heights were estimated for.
    preview_width: f32,
    /// Whether a large document waits for its reparse.
    pending_reparse: bool,
    /// When the waiting document is parsed again.
    reparse_deadline: Option<Instant>,
    /// The word and character count of the document.
    stats: DocumentStats,
    /// A counter that changes whenever the source is parsed again.
    ///
    /// The search results are kept until this changes, so that a large
    /// document is not searched again on every frame the panel is open.
    source_revision: u64,
    /// The cached result of the sidebar search.
    search_cache: SearchCache,
    /// The action waiting for the "unsaved changes" dialog to be answered.
    pending_action: Option<PendingAction>,
}

/// An action that has to wait while the user decides what to do with unsaved
/// edits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingAction {
    /// Start an empty document.
    New,
    /// Ask for a file to open.
    Open,
    /// Open a file that is already known, such as a dropped one.
    OpenPath(PathBuf),
    /// Close the window.
    Close,
}

/// The last search that was run for the sidebar.
#[derive(Debug, Clone, Default)]
struct SearchCache {
    /// The query the matches belong to.
    query: String,
    /// The source revision the matches belong to.
    revision: u64,
    /// The matches of that query.
    matches: Vec<SearchMatch>,
}

/// The word and character count of a document.
///
/// Both numbers are expensive to compute for a large document, so they are
/// kept and only refreshed when the source changes.
#[derive(Debug, Clone, Copy, Default)]
struct DocumentStats {
    words: usize,
    chars: usize,
}

impl DocumentStats {
    /// Count the words and characters of a source.
    fn of(source: &str) -> DocumentStats {
        DocumentStats {
            words: source.split_whitespace().count(),
            chars: source.chars().count(),
        }
    }
}

impl MarkTextApp {
    /// Create the application with an empty document.
    pub fn new() -> MarkTextApp {
        let source = WELCOME.to_string();
        let document = marktext_core::parse_document(&source).unwrap_or_else(|_| Document {
            blocks: parse(&source).unwrap_or_default(),
            source: source.clone(),
            references: Vec::new(),
        });
        let toc = build_toc(&document.blocks, "");
        MarkTextApp {
            source,
            path: None,
            dirty: false,
            document,
            toc,
            layout: LayoutMode::Split,
            theme: Theme::Light,
            show_sidebar: true,
            search_query: String::new(),
            search_open: false,
            cursor_line: 1,
            status: "就绪".to_string(),
            parse_failed: false,
            heading_offsets: Vec::new(),
            pending_scroll: None,
            parse_options: ParseOptions::default(),
            inline_options: InlineOptions::default(),
            scale: DocumentScale::Small,
            chunks: SourceChunks::default(),
            preview_heights: Vec::new(),
            preview_width: 0.0,
            preview_heights_revision: 0,
            pending_reparse: false,
            reparse_deadline: None,
            stats: DocumentStats::of(WELCOME),
            source_revision: 0,
            search_cache: SearchCache::default(),
            pending_action: None,
        }
    }

    /// The line the cursor is on, counted from one.
    pub fn cursor_line(&self) -> usize {
        self.cursor_line
    }

    /// Load a file into the application.
    pub fn open_path(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(source) => {
                self.source = source;
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.scale = DocumentScale::of(&self.source);
                self.chunks = SourceChunks::new(&self.source);
                self.reparse();
                self.status = format!("已打开 {}", path.display());
            }
            Err(err) => {
                self.status = format!("无法打开 {}：{err}", path.display());
            }
        }
    }

    /// Save the document, asking for a path when there is none yet.
    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            self.save_as();
            return;
        };
        self.flush_source();
        match std::fs::write(&path, &self.source) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("已保存 {}", path.display());
            }
            Err(err) => {
                self.status = format!("无法保存 {}：{err}", path.display());
            }
        }
    }

    /// Write the chunks of a large document back into the source.
    ///
    /// A large document is parsed with a delay, so the source can still be
    /// behind the chunks. Saving has to use the text that is on the screen.
    fn flush_source(&mut self) {
        if self.scale.is_large() {
            self.source = self.chunks.source();
        }
    }

    /// Save the document under a new name.
    fn save_as(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("另存为")
            .add_filter("Markdown", &["md", "markdown"])
            .add_filter("所有文件", &["*"])
            .save_file()
        else {
            return;
        };
        self.flush_source();
        match std::fs::write(&path, &self.source) {
            Ok(()) => {
                self.path = Some(path.clone());
                self.dirty = false;
                self.status = format!("已保存 {}", path.display());
            }
            Err(err) => {
                self.status = format!("无法保存 {}：{err}", path.display());
            }
        }
    }

    /// Open a document through a file dialog.
    fn open_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("打开 Markdown 文件")
            .add_filter("Markdown", &["md", "markdown"])
            .add_filter("所有文件", &["*"])
            .pick_file()
        else {
            return;
        };
        self.open_path(&path);
    }

    /// Re-parse the source and rebuild the outline.
    ///
    /// A large document waits until the typing stops, because parsing a few
    /// megabytes on every keystroke would cost more than the delay.
    fn reparse(&mut self) {
        if self.scale.is_large() {
            self.pending_reparse = true;
            self.reparse_deadline = Some(Instant::now() + REPARSE_DELAY);
            return;
        }
        self.reparse_now();
    }

    /// Parse the source right away.
    fn reparse_now(&mut self) {
        self.pending_reparse = false;
        self.reparse_deadline = None;
        if self.scale.is_large() {
            // The chunks hold the edited source of a large document.
            self.source = self.chunks.source();
        }
        self.source_revision = self.source_revision.wrapping_add(1);
        self.update_scale();
        self.stats = DocumentStats::of(&self.source);
        match parse_with(&self.source, &self.parse_options) {
            Ok(document) => {
                self.document = document;
                self.toc = build_toc(&self.document.blocks, "");
                self.parse_failed = false;
            }
            Err(err) => {
                self.parse_failed = true;
                self.status = format!("解析失败：{err}");
            }
        }
    }

    /// Keep the layout mode in step with the size of the document.
    ///
    /// A document can cross the threshold while it is being edited — a long
    /// log pasted into an empty buffer, or the tail of a large file deleted.
    /// Without this check the mode would stay whatever it was when the file
    /// was opened.
    fn update_scale(&mut self) {
        let scale = DocumentScale::of(&self.source);
        if scale == self.scale {
            if scale.is_large() && self.chunks.pieces().is_empty() {
                self.chunks = SourceChunks::new(&self.source);
            }
            return;
        }

        self.scale = scale;
        if scale.is_large() {
            self.chunks = SourceChunks::new(&self.source);
            self.status = "文档较大，已切换到窗口模式".to_string();
        } else {
            self.chunks = SourceChunks::default();
        }
    }

    /// Parse a large document once the typing has stopped.
    fn run_pending_reparse(&mut self, ctx: &Context) {
        let Some(deadline) = self.reparse_deadline else {
            return;
        };
        let now = Instant::now();
        if now >= deadline {
            self.reparse_now();
        } else {
            ctx.request_repaint_after(deadline - now);
        }
    }

    /// The palette of the current theme.
    fn palette(&self) -> Palette {
        self.theme.palette()
    }
}

/// Parse the source with the given options.
fn parse_with(source: &str, options: &ParseOptions) -> Result<marktext_core::Document, String> {
    marktext_core::parse_document_with_options(source, *options).map_err(|err| err.to_string())
}

impl eframe::App for MarkTextApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx, self.theme);

        self.handle_confirmation(ctx);
        self.handle_dropped_files(ctx);
        self.handle_shortcuts(ctx);
        self.run_pending_reparse(ctx);

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| self.menu_bar(ui));
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| self.status_bar(ui));

        if self.show_sidebar {
            egui::SidePanel::left("sidebar")
                .resizable(true)
                .default_width(220.0)
                .show(ctx, |ui| self.sidebar(ui));
        }

        egui::CentralPanel::default().show(ctx, |ui| self.central(ui));
    }
}

/// Apply the theme's colours to egui.
fn apply_theme(ctx: &Context, theme: Theme) {
    let mut style = (*ctx.style()).clone();
    let palette = theme.palette();
    style.visuals.panel_fill = palette.background;
    style.visuals.window_fill = palette.panel;
    style.visuals.extreme_bg_color = palette.code_background;
    style.visuals.code_bg_color = palette.code_background;
    style.visuals.override_text_color = Some(palette.text);
    style.visuals.weak_text_color = Some(palette.weak_text);
    style.visuals.hyperlink_color = palette.accent;
    style.visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, palette.accent);
    style.spacing.text_edit_width = f32::INFINITY;
    ctx.set_visuals(style.visuals.clone());
    ctx.set_style(style);
    ctx.set_theme(if theme.is_dark() {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
}

impl Default for MarkTextApp {
    fn default() -> MarkTextApp {
        MarkTextApp::new()
    }
}

impl MarkTextApp {
    /// Ask before an action throws away unsaved edits.
    fn request_action(&mut self, ctx: &Context, action: PendingAction) {
        if self.dirty {
            self.pending_action = Some(action);
        } else {
            self.perform(ctx, action);
        }
    }

    /// Carry out an action that may have waited for the dialog.
    fn perform(&mut self, ctx: &Context, action: PendingAction) {
        match action {
            PendingAction::New => self.new_document(),
            PendingAction::Open => self.open_dialog(),
            PendingAction::OpenPath(path) => self.open_path(&path),
            PendingAction::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Start an empty document.
    fn new_document(&mut self) {
        self.source.clear();
        self.path = None;
        self.dirty = false;
        self.scale = DocumentScale::Small;
        self.chunks = SourceChunks::default();
        self.reparse();
        self.status = "新建文档".to_string();
    }

    /// Show the "unsaved changes" dialog and run the waiting action.
    ///
    /// The window close button, the New and Open menu items and a dropped file
    /// all go through here; without it each of them replaced the document in
    /// place and every edit since the last save was lost.
    fn handle_confirmation(&mut self, ctx: &Context) {
        if ctx.input(|input| input.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_action.get_or_insert(PendingAction::Close);
        }

        let Some(action) = self.pending_action.clone() else {
            return;
        };

        // The edits may have been undone or saved while the dialog was open.
        if !self.dirty {
            self.pending_action = None;
            self.perform(ctx, action);
            return;
        }

        let mut decision = None;
        egui::Window::new("未保存的更改")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("当前文档有未保存的更改。");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("保存并继续").clicked() {
                        decision = Some(CloseDecision::Save);
                    }
                    if ui.button("不保存").clicked() {
                        decision = Some(CloseDecision::Discard);
                    }
                    if ui.button("取消").clicked() {
                        decision = Some(CloseDecision::Cancel);
                    }
                });
            });

        match decision {
            Some(CloseDecision::Save) => {
                self.save();
                // `save` falls back to "save as"; if that dialog is cancelled
                // the document is still dirty and nothing may be discarded.
                if !self.dirty {
                    self.pending_action = None;
                    self.perform(ctx, action);
                }
            }
            Some(CloseDecision::Discard) => {
                // New and Open reset `dirty` themselves, and only once the
                // document was actually replaced. Clearing it here would
                // throw the edits away even when the file dialog is
                // cancelled. A close, on the other hand, must not re-enter
                // this dialog.
                if matches!(action, PendingAction::Close) {
                    self.dirty = false;
                }
                self.pending_action = None;
                self.perform(ctx, action);
            }
            Some(CloseDecision::Cancel) => {
                self.pending_action = None;
            }
            // The request has been cancelled, so the dialog keeps the window
            // alive until it is answered.
            None => ctx.request_repaint(),
        }
    }

    /// Open a file that was dropped on the window.
    fn handle_dropped_files(&mut self, ctx: &Context) {
        if self.pending_action.is_some() {
            return;
        }
        let path = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .find_map(|file| file.path.clone())
        });
        if let Some(path) = path {
            self.request_action(ctx, PendingAction::OpenPath(path));
        }
    }

    /// Handle the keyboard shortcuts.
    fn handle_shortcuts(&mut self, ctx: &Context) {
        let save = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S));
        if save {
            self.save();
        }

        let open = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O));
        if open {
            self.request_action(ctx, PendingAction::Open);
        }

        let toggle_layout = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::L));
        if toggle_layout {
            self.layout = self.layout.next();
        }

        let toggle_sidebar = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::B));
        if toggle_sidebar {
            self.show_sidebar = !self.show_sidebar;
        }
    }

    /// Draw the menu bar.
    fn menu_bar(&mut self, ui: &mut Ui) {
        let palette = self.palette();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("文件", |ui| {
                if ui.button("新建").clicked() {
                    self.request_action(ui.ctx(), PendingAction::New);
                    ui.close();
                }
                if ui.button("打开…").clicked() {
                    self.request_action(ui.ctx(), PendingAction::Open);
                    ui.close();
                }
                if ui.button("保存").clicked() {
                    self.save();
                    ui.close();
                }
                if ui.button("另存为…").clicked() {
                    self.save_as();
                    ui.close();
                }
                ui.separator();
                if ui.button("导出 HTML…").clicked() {
                    self.export_html();
                    ui.close();
                }
            });

            ui.menu_button("视图", |ui| {
                for mode in [LayoutMode::Source, LayoutMode::Split, LayoutMode::Preview] {
                    let mut selected = self.layout == mode;
                    if ui.checkbox(&mut selected, mode.label()).changed() {
                        self.layout = mode;
                        ui.close();
                    }
                }
                ui.separator();
                let mut show_sidebar = self.show_sidebar;
                if ui.checkbox(&mut show_sidebar, "显示侧边栏").changed() {
                    self.show_sidebar = show_sidebar;
                }
                ui.separator();
                let mut dark = self.theme.is_dark();
                if ui.checkbox(&mut dark, "深色主题").changed() {
                    self.theme = if dark { Theme::Dark } else { Theme::Light };
                }
            });

            ui.menu_button("引擎", |ui| {
                let mut footnotes = self.parse_options.footnote;
                if ui.checkbox(&mut footnotes, "脚注").changed() {
                    self.parse_options.footnote = footnotes;
                    self.reparse();
                }
                let mut alerts = self.parse_options.github_alerts;
                if ui.checkbox(&mut alerts, "GitHub 警告块").changed() {
                    self.parse_options.github_alerts = alerts;
                    self.reparse();
                }
                let mut fancy = self.parse_options.fancy_lists;
                if ui.checkbox(&mut fancy, "pandoc 花式列表").changed() {
                    self.parse_options.fancy_lists = fancy;
                    self.reparse();
                }
            });
        });

        // The layout switch lives right of the menus, like MarkText's.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(self.layout.label())
                    .color(palette.weak_text)
                    .strong(),
            );
            ui.add_space(8.0);
        });
    }

    /// Draw the side bar with the outline and the search box.
    fn sidebar(&mut self, ui: &mut Ui) {
        let palette = self.palette();

        ui.horizontal(|ui| {
            let mut search_open = self.search_open;
            if ui
                .button(if search_open {
                    "隐藏搜索"
                } else {
                    "搜索"
                })
                .clicked()
            {
                search_open = !search_open;
            }
            self.search_open = search_open;
            ui.label(RichText::new("大纲").strong().color(palette.weak_text));
        });

        if self.search_open {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.search_query);
                if ui.button("×").clicked() {
                    self.search_query.clear();
                }
            });
            self.draw_search_results(ui);
        }

        ui.separator();

        // The outline of a large document holds thousands of headings, so
        // only the ones on screen are drawn.
        if let Some(text) = draw_toc(ui, &self.toc, &palette) {
            self.pending_scroll = Some(text);
        }
    }

    /// Draw the search results.
    fn draw_search_results(&mut self, ui: &mut Ui) {
        let palette = self.palette();
        if self.search_query.is_empty() {
            self.search_cache = SearchCache::default();
            return;
        }

        // Searching lowercases (and for a regex compiles) the whole document,
        // so it is only redone when the query or the source changed, not on
        // every frame the panel stays open.
        if self.search_cache.query != self.search_query
            || self.search_cache.revision != self.source_revision
        {
            let result = search_document(
                &self.source,
                &self.search_query,
                SearchOptions {
                    case_insensitive: true,
                    regex: false,
                    context: 24,
                },
            );
            let matches = match result {
                Ok(matches) => matches,
                Err(err) => {
                    self.status = err.to_string();
                    Vec::new()
                }
            };
            self.search_cache = SearchCache {
                query: self.search_query.clone(),
                revision: self.source_revision,
                matches,
            };
        }

        if self.search_cache.matches.is_empty() {
            ui.label(RichText::new("没有匹配").weak().color(palette.weak_text));
            return;
        }

        // The rows are collected before they are drawn: drawing needs a
        // mutable borrow of the app, and the matches live inside it.
        let total = self.search_cache.matches.len();
        let rows: Vec<(String, usize)> = self
            .search_cache
            .matches
            .iter()
            .take(50)
            .map(|found| {
                (
                    marktext_core::search::context_of(&self.source, found, 24),
                    found.start,
                )
            })
            .collect();

        ui.label(
            RichText::new(format!("{total} 处匹配"))
                .weak()
                .color(palette.weak_text),
        );
        // The results get their own scroll area: without it, a query with many
        // hits pushes the outline out of the side panel.
        egui::ScrollArea::vertical()
            .id_salt("search_results")
            .max_height(220.0)
            .show(ui, |ui| {
                for (text, start) in rows {
                    let response = ui.add(
                        egui::Label::new(RichText::new(text.replace('\n', " ")).small())
                            .sense(Sense::click()),
                    );
                    if response.clicked() {
                        self.status = format!("第 {} 字节处命中", start);
                    }
                }
            });
    }

    /// Draw the document area.
    fn central(&mut self, ui: &mut Ui) {
        let palette = self.palette();
        match self.layout {
            LayoutMode::Source => {
                self.source_editor(ui, palette);
            }
            LayoutMode::Preview => {
                self.preview_pane(ui, palette);
            }
            LayoutMode::Split => {
                ui.columns(2, |columns| {
                    egui::Frame::new()
                        .stroke(Stroke::new(1.0_f32, palette.rule))
                        .inner_margin(egui::Margin::same(8))
                        .show(&mut columns[0], |ui| self.source_editor(ui, palette));
                    egui::Frame::new()
                        .stroke(Stroke::new(1.0_f32, palette.rule))
                        .inner_margin(egui::Margin::same(8))
                        .show(&mut columns[1], |ui| self.preview_pane(ui, palette));
                });
            }
        }
    }

    /// Draw the Markdown source editor.
    fn source_editor(&mut self, ui: &mut Ui, palette: Palette) {
        if self.scale.is_large() {
            self.source_editor_windowed(ui);
        } else {
            self.source_editor_whole(ui);
        }
        let _ = palette;
    }

    /// Draw the whole source in one text edit.
    ///
    /// A text edit widget lays out the whole text it holds, so this is only
    /// used for a document that is small enough for that.
    fn source_editor_whole(&mut self, ui: &mut Ui) {
        let editor = TextEdit::multiline(&mut self.source)
            .code_editor()
            .font(theme::body_font(14.0))
            .desired_width(f32::INFINITY)
            .hint_text("在此输入 Markdown…");

        let output = editor.show(ui);
        if output.response.changed() {
            self.dirty = true;
            self.reparse();
        }
        if let Some(range) = output.cursor_range {
            self.cursor_line = crate::line_of_cursor(&self.source, range.primary.index);
        }
    }

    /// Draw the source of a large document in chunks.
    ///
    /// The source is cut into chunks and only the chunks that are on screen
    /// are laid out, so the memory the editor needs stays the same no matter
    /// how long the document is.
    fn source_editor_windowed(&mut self, ui: &mut Ui) {
        // The chunks are taken out of the editor for the time of the layout,
        // because the layout borrows them while the editor is borrowed too.
        let mut chunks = std::mem::take(&mut self.chunks);
        let edit = large::draw_chunks(ui, &mut chunks);
        self.chunks = chunks;

        if edit.changed {
            self.dirty = true;
            self.reparse();
        }
        if let Some((index, start_line, cursor_index)) = edit.cursor {
            let text = &self.chunks.pieces()[index];
            self.cursor_line = start_line + crate::line_of_cursor(text, cursor_index);
        }
    }

    /// Draw the rendered preview.
    fn preview_pane(&mut self, ui: &mut Ui, palette: Palette) {
        let mut headings = Vec::new();

        let scale = self.scale;
        // The heights are only needed when the preview is drawn in windows.
        // They have to be worked out before the blocks are borrowed, because
        // working them out needs the editor for itself.
        if scale.is_large() {
            self.update_preview_heights(ui.available_width());
        }

        // A heading picked in the outline is scrolled to by feeding the offset
        // that was recorded during the previous layout back into the scroll
        // area. The windowed preview has only drawn the headings that were on
        // screen, so a heading further down is located through the estimated
        // block heights instead.
        let scroll_offset = self.pending_scroll.as_ref().and_then(|target| {
            self.heading_offsets
                .iter()
                .find(|(text, _)| text == target)
                .map(|(_, position)| position.y)
                .or_else(|| {
                    if scale.is_large() {
                        preview::estimated_heading_offset(
                            &self.document.blocks,
                            &self.preview_heights,
                            target,
                        )
                    } else {
                        None
                    }
                })
        });

        let mut scroll_area = ScrollArea::vertical().id_salt("preview_scroll");
        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }

        let heights = if scale.is_large() {
            Some(&self.preview_heights[..])
        } else {
            None
        };
        let blocks = &self.document.blocks;
        let inline_options = &self.inline_options;

        scroll_area.show(ui, |ui| {
            ui.set_width(ui.available_width());
            if blocks.is_empty() {
                ui.label(RichText::new("空文档").weak().color(palette.weak_text));
            } else if let Some(heights) = heights {
                preview::draw_blocks_windowed(
                    ui,
                    blocks,
                    heights,
                    &palette,
                    inline_options,
                    &mut headings,
                );
            } else {
                preview::draw_blocks(ui, blocks, &palette, inline_options, &mut headings);
            }
        });

        self.heading_offsets = headings;
        self.pending_scroll = None;
    }

    /// The estimated height of every block of the preview.
    ///
    /// The heights change when the width of the pane changes and whenever the
    /// document is parsed again: editing one paragraph keeps the block count
    /// the same but changes the number of wrapped lines. Keying the cache on
    /// the source revision is what keeps later blocks from being placed at a
    /// stale offset after an ordinary edit.
    fn update_preview_heights(&mut self, width: f32) {
        if (self.preview_width - width).abs() > 1.0
            || self.preview_heights_revision != self.source_revision
        {
            self.preview_heights = preview::estimate_block_heights(&self.document.blocks, width);
            self.preview_width = width;
            self.preview_heights_revision = self.source_revision;
        }
    }

    /// Draw the status bar.
    fn status_bar(&mut self, ui: &mut Ui) {
        let palette = self.palette();
        // The counts are taken from the cache: counting the words of a large
        // document on every frame would keep the processor busy for good.
        let words = self.stats.words;
        let chars = self.stats.chars;
        let title = self
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未命名".to_string());
        let dirty = if self.dirty { " ●" } else { "" };

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{title}{dirty}")).color(palette.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{words} 词 · {chars} 字符")).color(palette.weak_text),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("第 {} 行", self.cursor_line)).color(palette.weak_text),
                );
                ui.separator();
                let status_color = if self.parse_failed {
                    Color32::from_rgb(207, 34, 46)
                } else {
                    palette.weak_text
                };
                ui.label(RichText::new(&self.status).color(status_color));
            });
        });
    }

    /// Export the document as an HTML file.
    fn export_html(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导出 HTML")
            .add_filter("HTML", &["html", "htm"])
            .save_file()
        else {
            return;
        };
        // The preview of a large document can be a moment behind the text.
        self.flush_source();
        self.reparse_now();
        let options = RenderOptions {
            references: self.document.references.clone(),
            ..RenderOptions::default()
        };
        let html = marktext_core::render_to_html(&self.document.blocks, options);
        match std::fs::write(&path, html) {
            Ok(()) => {
                self.status = format!("已导出 {}", path.display());
            }
            Err(err) => {
                self.status = format!("无法导出 {}：{err}", path.display());
            }
        }
    }
}

/// The document the app starts with.
const WELCOME: &str = "# MarkText

这是 **MarkText** 的 Rust 移植版本，引擎由 `marktext-core` 驱动。

## 功能

- 左侧大纲面板，标题可点击跳转
- Markdown 源码编辑与实时渲染预览
- 全文搜索、HTML 导出
- 往返收敛：重复 `parse → serialize` 会得到相同结果

## 快捷键

| 快捷键 | 作用 |
| ------ | ---- |
| `Ctrl+S` | 保存 |
| `Ctrl+O` | 打开 |
| `Ctrl+L` | 切换布局 |
| `Ctrl+B` | 切换侧边栏 |

> 引擎严格对齐原版 `packages/muya` 的架构：块树保存原始行内文本，因此未修改的文档可以稳定往返。

```rust
let blocks = marktext_core::parse(source)?;
let markdown = marktext_core::serialize(&blocks, Default::default());
```

任务清单：

- [x] 块级解析器
- [x] 行内解析器
- [x] 收敛式序列化
- [ ] 实时协同编辑
";

/// Draw the outline of the document.
///
/// The outline of a large document holds thousands of headings, so only the
/// ones that are on screen are drawn. The heading that was clicked on is
/// returned, so that the preview can jump to it.
pub fn draw_toc(ui: &mut Ui, toc: &[TocItem], palette: &Palette) -> Option<String> {
    let mut clicked = None;

    ScrollArea::vertical().id_salt("toc_scroll").show(ui, |ui| {
        if toc.is_empty() {
            ui.label(
                RichText::new("文档没有标题")
                    .weak()
                    .color(palette.weak_text),
            );
            return;
        }
        let width = ui.available_width();
        let total = toc.len() as f32 * TOC_ROW_HEIGHT;
        ui.allocate_space(egui::vec2(width, total));

        let origin = ui.min_rect().min;
        let clip = ui.clip_rect();
        let visible_top = clip.min.y - origin.y - TOC_ROW_HEIGHT;
        let visible_bottom = clip.max.y - origin.y + TOC_ROW_HEIGHT;
        let first = ((visible_top / TOC_ROW_HEIGHT).floor().max(0.0) as usize).min(toc.len());
        let last = ((visible_bottom / TOC_ROW_HEIGHT).ceil().max(0.0) as usize).min(toc.len());

        for (index, item) in toc.iter().enumerate().skip(first).take(last - first) {
            let indent = (item.level.saturating_sub(1)) as f32 * 12.0;
            let y = index as f32 * TOC_ROW_HEIGHT;
            let rect = egui::Rect::from_min_size(
                origin + egui::vec2(0.0, y),
                egui::vec2(width, TOC_ROW_HEIGHT),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    let response = ui.add(
                        egui::Label::new(RichText::new(&item.content).color(palette.text))
                            .sense(Sense::click()),
                    );
                    if response.clicked() {
                        clicked = Some(item.content.clone());
                    }
                });
            });
        }
    });

    clicked
}
