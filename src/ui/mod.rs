pub(crate) mod pane;
pub(crate) mod tab_bar;
pub(crate) mod sidebar;

use anyhow::Context as _;
use eframe::egui;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_os = "macos")]
use objc::runtime::Object;
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};

use rfd::FileDialog;

use crate::config::{migrate_config, AppConfig, CONFIG_VERSION};
use crate::notes::{hash_string, parse_markdown, ParsedMarkdownLine};
use crate::terminal::{
    default_terminal_cwd, install_paste_monitor, read_clipboard, save_clipboard_image,
    CMD_V_PRESSED, TerminalTab,
};
use crate::theme::{ThemePalette, ThemePreset};
use crate::{
    MINIMIZED_HEIGHT, SIDEBAR_DEFAULT_WIDTH, TAB_BAR_HEIGHT, TOP_BAR_HEIGHT, WINDOW_HEIGHT,
    WINDOW_WIDTH,
};

pub(crate) struct GhostStickiesApp {
    notes_root: Option<PathBuf>,
    theme: ThemePreset,
    minimized: bool,
    sidebar_open: bool,
    privacy_mode: bool,
    startup_tasks_run: bool,
    applied_privacy_mode: Option<bool>,
    next_tab_number: usize,
    next_pane_uid: u64,
    terminal_tabs: Vec<TerminalTab>,
    active_terminal: usize,
    renaming_tab: Option<usize>,
    rename_buffer: String,
    // Debug log
    debug_log: VecDeque<String>,
    show_debug: bool,
    recent_notes: Vec<PathBuf>,
    renaming_pane: Option<(usize, usize)>, // (tab_idx, pane_idx)
    pane_rename_buffer: String,
    last_error: Option<String>,
    last_activity: std::time::Instant,
}

pub(crate) const DEBUG_LOG_MAX: usize = 200;

#[derive(Clone, Copy)]
pub(crate) enum AppSymbol {
    Privacy,
}

impl Default for GhostStickiesApp {
    fn default() -> Self {
        let cwd = default_terminal_cwd();

        Self {
            notes_root: None,
            theme: ThemePreset::default(),
            minimized: false,
            sidebar_open: false,
            privacy_mode: false,
            startup_tasks_run: false,
            applied_privacy_mode: None,
            next_tab_number: 2,
            next_pane_uid: 2,
            terminal_tabs: vec![TerminalTab::new(1, 1, cwd)],
            active_terminal: 0,
            renaming_tab: None,
            rename_buffer: String::new(),
            debug_log: VecDeque::new(),
            show_debug: false,
            recent_notes: Vec::new(),
            renaming_pane: None,
            pane_rename_buffer: String::new(),
            last_error: None,
            last_activity: std::time::Instant::now(),
        }
    }
}


impl GhostStickiesApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let fonts = egui::FontDefinitions::default();
        cc.egui_ctx.set_fonts(fonts);

        install_paste_monitor();

        let mut app = Self::default();
        app.load_saved_config();
        app
    }

    fn symbol_image(symbol: AppSymbol) -> egui::Image<'static> {
        let image = match symbol {
            AppSymbol::Privacy => egui::include_image!("../../assets/eye.circle.png"),
        };

        egui::Image::new(image).fit_to_exact_size(egui::vec2(14.0, 14.0))
    }

    fn symbol_button(
        ui: &mut egui::Ui,
        symbol: AppSymbol,
        tooltip: &str,
        selected: bool,
    ) -> egui::Response {
        ui.add(
            egui::Button::image(Self::symbol_image(symbol))
                .selected(selected)
                .frame(true)
                .corner_radius(egui::CornerRadius::same(4))
                .min_size(egui::vec2(22.0, 22.0)),
        )
        .on_hover_text(tooltip)
    }

    fn app_support_dir() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("StickyTerminal")
        } else {
            PathBuf::from(".stickyterminal")
        }
    }

    fn config_path() -> PathBuf {
        Self::app_support_dir().join("config.json")
    }

    fn load_saved_config(&mut self) {
        let config_path = Self::config_path();
        let Ok(contents) = fs::read_to_string(&config_path) else {
            return;
        };

        let Ok(raw_config) = serde_json::from_str::<AppConfig>(&contents) else {
            self.terminal_tabs[0].note_status =
                "Could not read saved settings. Using defaults.".to_owned();
            return;
        };

        let config = migrate_config(raw_config);

        self.theme = config.theme;
        self.sidebar_open = config.sidebar_open;
        self.terminal_tabs[0].current_note_file = config.current_note_file;
        self.recent_notes = config.recent_notes;

        if let Some(root) = config.notes_root {
            self.notes_root = Some(root);
            if self.terminal_tabs[0].current_note_file.is_none() {
                self.terminal_tabs[0].current_note_file = self.default_note_file();
            }
            self.load_current_note();
        }
    }

    fn save_config_inner(&self) -> anyhow::Result<()> {
        let ti = self.active_terminal;
        let config = AppConfig {
            notes_root: self.notes_root.clone(),
            current_note_file: self.terminal_tabs[ti].current_note_file.clone(),
            theme: self.theme,
            recent_notes: self.recent_notes.clone(),
            version: CONFIG_VERSION,
            sidebar_open: self.sidebar_open,
        };

        let support_dir = Self::app_support_dir();
        fs::create_dir_all(&support_dir)
            .context("Could not create app settings folder")?;

        let contents = serde_json::to_string_pretty(&config)
            .context("Could not encode settings")?;

        fs::write(Self::config_path(), contents)
            .context("Could not save settings")?;

        Ok(())
    }

    fn save_config(&mut self) {
        if let Err(err) = self.save_config_inner() {
            self.last_error = Some(format!("Save config failed: {err:#}"));
        }
    }

    fn normalize_notes_root(path: PathBuf) -> PathBuf {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("StickyTerminal"))
            .unwrap_or(false)
        {
            path
        } else {
            path.join("StickyTerminal")
        }
    }

    fn default_note_file(&self) -> Option<PathBuf> {
        self.notes_root.as_ref().map(|root| root.join("inbox.md"))
    }

    fn note_file_path(&self) -> Option<PathBuf> {
        self.terminal_tabs[self.active_terminal]
            .current_note_file
            .clone()
    }

    fn choose_notes_root(&mut self) {
        let start_dir = self
            .notes_root
            .clone()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));

        let Some(selected_dir) = FileDialog::new().set_directory(start_dir).pick_folder() else {
            return;
        };

        let root = Self::normalize_notes_root(selected_dir);
        if let Err(err) = fs::create_dir_all(&root) {
            let ti = self.active_terminal;
            self.terminal_tabs[ti].note_status = format!("Could not create notes folder: {err}");
            return;
        }

        self.notes_root = Some(root.clone());
        let ti = self.active_terminal;
        let note_still_inside_root = self.terminal_tabs[ti]
            .current_note_file
            .as_ref()
            .map(|path| path.starts_with(&root))
            .unwrap_or(false);
        if !note_still_inside_root {
            self.terminal_tabs[ti].current_note_file = self.default_note_file();
        }
        self.terminal_tabs[ti].note_status = format!("Using notes folder: {}", root.display());
        self.save_config();
        self.load_current_note();
    }

    fn choose_existing_note(&mut self) {
        let ti = self.active_terminal;
        let Some(root) = self.notes_root.clone() else {
            self.terminal_tabs[ti].note_status = "Choose your notes folder first.".to_owned();
            return;
        };

        let Some(file) = FileDialog::new()
            .set_directory(&root)
            .add_filter("Markdown", &["md", "markdown", "txt"])
            .pick_file()
        else {
            return;
        };

        if !file.starts_with(&root) {
            self.terminal_tabs[ti].note_status = "Pick a note inside your notes folder.".to_owned();
            return;
        }

        self.terminal_tabs[ti].current_note_file = Some(file);
        self.load_current_note();
    }

    fn add_to_recent_notes(&mut self) {
        if let Some(path) = self.terminal_tabs[self.active_terminal]
            .current_note_file
            .clone()
        {
            self.recent_notes.retain(|p| p != &path);
            self.recent_notes.insert(0, path);
            self.recent_notes.truncate(10);
        }
    }

    fn save_current_note_silent(&mut self) {
        let Some(path) = self.note_file_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let ti = self.active_terminal;
        if fs::write(&path, &self.terminal_tabs[ti].notes_markdown).is_ok() {
            self.terminal_tabs[ti].notes_dirty = false;
            self.terminal_tabs[ti].last_type_time = None;
        }
    }

    fn save_current_note(&mut self) {
        let ti = self.active_terminal;
        let Some(path) = self.note_file_path() else {
            self.terminal_tabs[ti].note_status =
                "Choose your notes folder and a note first.".to_owned();
            return;
        };

        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                self.terminal_tabs[ti].note_status =
                    format!("Could not create note folders: {err}");
                return;
            }
        }

        match fs::write(&path, &self.terminal_tabs[ti].notes_markdown) {
            Ok(_) => {
                self.terminal_tabs[ti].note_status = format!("Saved {}", path.display());
                self.terminal_tabs[ti].notes_dirty = false;
                self.terminal_tabs[ti].last_type_time = None;
                self.save_config();
            }
            Err(err) => {
                self.terminal_tabs[ti].note_status = format!("Could not save note: {err}");
            }
        }
    }

    fn load_current_note(&mut self) {
        let ti = self.active_terminal;
        let Some(path) = self.note_file_path() else {
            self.terminal_tabs[ti].note_status = "Pick a note file to start writing.".to_owned();
            return;
        };

        self.add_to_recent_notes();
        self.save_config();

        let ti = self.active_terminal;
        match fs::read_to_string(&path) {
            Ok(contents) => {
                self.terminal_tabs[ti].notes_markdown = contents;
                self.terminal_tabs[ti].notes_dirty = false;
                self.terminal_tabs[ti].last_type_time = None;
                self.terminal_tabs[ti].note_status = format!("Loaded {}", path.display());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.terminal_tabs[ti].notes_markdown = "# Inbox\n\nStart writing here.".to_owned();
                self.terminal_tabs[ti].notes_dirty = false;
                self.terminal_tabs[ti].last_type_time = None;
                self.terminal_tabs[ti].note_status = format!("New note ready: {}", path.display());
            }
            Err(err) => {
                self.terminal_tabs[ti].note_status = format!("Could not load note: {err}");
            }
        }
    }

    fn create_new_note(&mut self) {
        let ti = self.active_terminal;
        let Some(root) = self.notes_root.clone() else {
            self.terminal_tabs[ti].note_status = "Choose your notes folder first.".to_owned();
            return;
        };

        let Some(path) = FileDialog::new()
            .set_directory(&root)
            .set_file_name("note.md")
            .add_filter("Markdown", &["md"])
            .save_file()
        else {
            return;
        };

        if !path.starts_with(&root) {
            self.terminal_tabs[ti].note_status =
                "Save the note inside your notes folder.".to_owned();
            return;
        }

        self.terminal_tabs[ti].current_note_file = Some(if path.extension().is_none() {
            path.with_extension("md")
        } else {
            path
        });
        self.terminal_tabs[ti].notes_markdown = "# New note\n\n".to_owned();
        self.terminal_tabs[ti].note_status =
            "New note ready. Press Save to write it to disk.".to_owned();
        self.save_config();
    }

    fn note_surface_frame(palette: ThemePalette) -> egui::Frame {
        egui::Frame::NONE
            .fill(palette.surface)
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(10))
    }

    fn note_action_button(label: &str, palette: ThemePalette) -> egui::Button<'static> {
        egui::Button::new(
            egui::RichText::new(label.to_owned())
                .small()
                .color(palette.muted_text),
        )
        .corner_radius(egui::CornerRadius::same(6))
        .min_size(egui::vec2(0.0, 24.0))
    }

    fn tab_plus_button(ui: &mut egui::Ui, palette: ThemePalette) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), egui::Sense::click());

        if ui.is_rect_visible(rect) {
            let fill = if response.is_pointer_button_down_on() {
                palette.active_tab_bg
            } else if response.hovered() {
                palette.tab_bg
            } else {
                egui::Color32::TRANSPARENT
            };
            let stroke = if response.hovered() {
                egui::Stroke::new(1.0, palette.border)
            } else {
                egui::Stroke::NONE
            };

            ui.painter().rect(
                rect,
                egui::CornerRadius::same(6),
                fill,
                stroke,
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center() + egui::vec2(0.0, -0.5),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(16.0),
                if response.hovered() {
                    palette.text
                } else {
                    palette.muted_text
                },
            );
        }

        response
    }

    /// Calculate indent level from leading whitespace (each 2 spaces or 1 tab = 1 level)
    fn indent_level(line: &str) -> usize {
        let leading: usize = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| if c == '\t' { 2 } else { 1 })
            .sum();
        leading / 2
    }

    fn markdown_checkbox(
        ui: &mut egui::Ui,
        checked: &mut bool,
        palette: ThemePalette,
    ) -> egui::Response {
        let size = egui::vec2(18.0, 18.0);
        let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());

        if response.clicked() {
            *checked = !*checked;
            response.mark_changed();
        }

        let border = egui::Color32::from_rgba_premultiplied(255, 255, 255, 235);
        let fill = if *checked {
            egui::Color32::WHITE
        } else if response.hovered() {
            egui::Color32::from_rgba_premultiplied(255, 255, 255, 14)
        } else {
            egui::Color32::TRANSPARENT
        };

        ui.painter().rect(
            rect,
            egui::CornerRadius::same(5),
            fill,
            egui::Stroke::new(1.25, border),
            egui::StrokeKind::Inside,
        );

        if *checked {
            let stroke = egui::Stroke::new(2.3, egui::Color32::BLACK);
            let a = egui::pos2(rect.left() + 4.2, rect.center().y + 0.4);
            let b = egui::pos2(rect.left() + 7.8, rect.bottom() - 4.6);
            let c = egui::pos2(rect.right() - 3.6, rect.top() + 5.0);
            ui.painter().line_segment([a, b], stroke);
            ui.painter().line_segment([b, c], stroke);
        }

        if response.hovered() {
            ui.painter().rect_stroke(
                rect.expand(0.5),
                egui::CornerRadius::same(5),
                egui::Stroke::new(1.0, palette.accent.linear_multiply(0.45)),
                egui::StrokeKind::Outside,
            );
        }

        response
    }

    fn append_markdown_segment(
        job: &mut egui::text::LayoutJob,
        text: &str,
        palette: ThemePalette,
        base_color: egui::Color32,
        base_size: f32,
        bold: bool,
        italic: bool,
        code: bool,
        link: bool,
        strikethrough: bool,
    ) {
        if text.is_empty() {
            return;
        }

        let mut format = egui::TextFormat {
            font_id: if code {
                egui::FontId::monospace((base_size - 1.0).max(12.0))
            } else {
                egui::FontId::proportional(base_size)
            },
            color: if code {
                egui::Color32::WHITE
            } else if link {
                palette.accent
            } else {
                base_color
            },
            italics: italic,
            background: if code {
                palette.input_bg.linear_multiply(1.25)
            } else {
                egui::Color32::TRANSPARENT
            },
            ..Default::default()
        };

        if bold {
            format.font_id.size += 0.5;
        }

        if link {
            format.underline = egui::Stroke::new(1.0, palette.accent.linear_multiply(0.8));
        }

        if strikethrough {
            format.strikethrough = egui::Stroke::new(1.0, format.color.linear_multiply(0.7));
        }

        job.append(text, 0.0, format);
    }

    fn inline_markdown_job(
        text: &str,
        palette: ThemePalette,
        base_color: egui::Color32,
        base_size: f32,
        strikethrough: bool,
    ) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        let mut buffer = String::new();
        let mut i = 0usize;
        let mut bold = false;
        let mut italic = false;
        let mut code = false;

        while i < text.len() {
            let rest = &text[i..];

            if !code && rest.starts_with("**") {
                Self::append_markdown_segment(
                    &mut job,
                    &buffer,
                    palette,
                    base_color,
                    base_size,
                    bold,
                    italic,
                    code,
                    false,
                    strikethrough,
                );
                buffer.clear();
                bold = !bold;
                i += 2;
                continue;
            }

            if rest.starts_with('`') {
                Self::append_markdown_segment(
                    &mut job,
                    &buffer,
                    palette,
                    base_color,
                    base_size,
                    bold,
                    italic,
                    code,
                    false,
                    strikethrough,
                );
                buffer.clear();
                code = !code;
                i += 1;
                continue;
            }

            if !code && rest.starts_with('*') {
                Self::append_markdown_segment(
                    &mut job,
                    &buffer,
                    palette,
                    base_color,
                    base_size,
                    bold,
                    italic,
                    code,
                    false,
                    strikethrough,
                );
                buffer.clear();
                italic = !italic;
                i += 1;
                continue;
            }

            if !code && rest.starts_with('[') {
                if let Some(close_bracket) = rest.find("](") {
                    let after_open = &rest[1..close_bracket];
                    let link_rest = &rest[close_bracket + 2..];
                    if let Some(close_paren) = link_rest.find(')') {
                        Self::append_markdown_segment(
                            &mut job,
                            &buffer,
                            palette,
                            base_color,
                            base_size,
                            bold,
                            italic,
                            code,
                            false,
                            strikethrough,
                        );
                        buffer.clear();
                        Self::append_markdown_segment(
                            &mut job,
                            after_open,
                            palette,
                            base_color,
                            base_size,
                            bold,
                            italic,
                            false,
                            true,
                            strikethrough,
                        );
                        i += close_bracket + 2 + close_paren + 1;
                        continue;
                    }
                }
            }

            let mut chars = rest.chars();
            if let Some(ch) = chars.next() {
                buffer.push(ch);
                i += ch.len_utf8();
            } else {
                break;
            }
        }

        Self::append_markdown_segment(
            &mut job,
            &buffer,
            palette,
            base_color,
            base_size,
            bold,
            italic,
            code,
            false,
            strikethrough,
        );

        job
    }

    fn markdown_label(
        ui: &mut egui::Ui,
        text: &str,
        palette: ThemePalette,
        base_color: egui::Color32,
        base_size: f32,
        strikethrough: bool,
    ) {
        ui.add(
            egui::Label::new(Self::inline_markdown_job(
                text,
                palette,
                base_color,
                base_size,
                strikethrough,
            ))
            .wrap_mode(egui::TextWrapMode::Wrap),
        );
    }

    /// Render pre-parsed markdown blocks. Call `parse_markdown()` once when content
    /// changes and cache the result; call this every frame with the cached slice.
    /// `markdown` is passed only so interactive checkboxes can toggle lines in-place.
    fn render_from_blocks(
        ui: &mut egui::Ui,
        blocks: &[ParsedMarkdownLine],
        markdown: &mut String,
        palette: ThemePalette,
        available_height: f32,
    ) -> bool {
        let mut changed = false;
        let indent_px = 16.0_f32;

        egui::ScrollArea::vertical()
            .max_height(available_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;

                for block in blocks {
                    match block {
                        ParsedMarkdownLine::Empty => {
                            ui.add_space(6.0);
                        }
                        ParsedMarkdownLine::H3(text) => {
                            ui.add_space(2.0);
                            Self::markdown_label(ui, text, palette, palette.text, 15.0, false);
                        }
                        ParsedMarkdownLine::H2(text) => {
                            ui.add_space(6.0);
                            Self::markdown_label(ui, text, palette, palette.text, 17.0, false);
                            let rule = egui::vec2(ui.available_width().min(160.0), 2.0);
                            let (rect, _) = ui.allocate_exact_size(rule, egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                egui::CornerRadius::same(2),
                                palette.accent.linear_multiply(0.45),
                            );
                            ui.add_space(2.0);
                        }
                        ParsedMarkdownLine::H1(text) => {
                            ui.add_space(8.0);
                            Self::markdown_label(ui, text, palette, palette.text, 21.0, false);
                            let rule = egui::vec2(ui.available_width().min(220.0), 2.0);
                            let (rect, _) = ui.allocate_exact_size(rule, egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                egui::CornerRadius::same(2),
                                palette.accent.linear_multiply(0.6),
                            );
                            ui.add_space(4.0);
                        }
                        ParsedMarkdownLine::CheckedTask { text, line_idx, indent } => {
                            let left_margin = *indent as f32 * indent_px;
                            ui.horizontal_wrapped(|ui| {
                                if left_margin > 0.0 {
                                    ui.add_space(left_margin);
                                }
                                let mut checked = true;
                                if Self::markdown_checkbox(ui, &mut checked, palette).changed() {
                                    Self::toggle_line_checkbox(markdown, *line_idx, false);
                                    changed = true;
                                }
                                Self::markdown_label(
                                    ui,
                                    text,
                                    palette,
                                    palette.muted_text,
                                    14.0,
                                    true,
                                );
                            });
                        }
                        ParsedMarkdownLine::UncheckedTask { text, line_idx, indent } => {
                            let left_margin = *indent as f32 * indent_px;
                            ui.horizontal_wrapped(|ui| {
                                if left_margin > 0.0 {
                                    ui.add_space(left_margin);
                                }
                                let mut checked = false;
                                if Self::markdown_checkbox(ui, &mut checked, palette).changed() {
                                    Self::toggle_line_checkbox(markdown, *line_idx, true);
                                    changed = true;
                                }
                                Self::markdown_label(
                                    ui,
                                    text,
                                    palette,
                                    palette.text,
                                    14.0,
                                    false,
                                );
                            });
                        }
                        ParsedMarkdownLine::Bullet { text, indent } => {
                            let left_margin = *indent as f32 * indent_px;
                            ui.horizontal_wrapped(|ui| {
                                if left_margin > 0.0 {
                                    ui.add_space(left_margin);
                                }
                                ui.label(
                                    egui::RichText::new("\u{2022}")
                                        .size(16.0)
                                        .color(palette.accent),
                                );
                                Self::markdown_label(ui, text, palette, palette.text, 14.0, false);
                            });
                        }
                        ParsedMarkdownLine::Numbered { num, text, indent } => {
                            let left_margin = *indent as f32 * indent_px;
                            ui.horizontal_wrapped(|ui| {
                                if left_margin > 0.0 {
                                    ui.add_space(left_margin);
                                }
                                ui.label(
                                    egui::RichText::new(format!("{num}."))
                                        .strong()
                                        .color(palette.accent),
                                );
                                Self::markdown_label(ui, text, palette, palette.text, 14.0, false);
                            });
                        }
                        ParsedMarkdownLine::Blockquote(text) => {
                            egui::Frame::NONE
                                .fill(palette.sidebar_soft_bg.linear_multiply(0.55))
                                .stroke(egui::Stroke::new(1.0, palette.border))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new("\u{258d}")
                                                .size(18.0)
                                                .color(palette.accent),
                                        );
                                        Self::markdown_label(
                                            ui,
                                            text,
                                            palette,
                                            palette.muted_text,
                                            14.0,
                                            false,
                                        );
                                    });
                                });
                        }
                        ParsedMarkdownLine::CodeBlock(code) => {
                            egui::Frame::NONE
                                .fill(palette.input_bg)
                                .stroke(egui::Stroke::new(1.0, palette.border))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(code.as_str())
                                                .monospace()
                                                .size(13.0)
                                                .color(egui::Color32::WHITE),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                    );
                                });
                        }
                        ParsedMarkdownLine::Rule => {
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }
                        ParsedMarkdownLine::Paragraph { text, indent } => {
                            let left_margin = *indent as f32 * indent_px;
                            if left_margin > 0.0 {
                                ui.horizontal_wrapped(|ui| {
                                    ui.add_space(left_margin);
                                    Self::markdown_label(
                                        ui,
                                        text,
                                        palette,
                                        palette.text,
                                        14.0,
                                        false,
                                    );
                                });
                            } else {
                                Self::markdown_label(ui, text, palette, palette.text, 14.0, false);
                            }
                        }
                    }
                }
            });

        changed
    }

    /// Scan one terminal row and return URL spans as (start_col, end_col_inclusive, url).
    fn find_row_url_spans(screen: &vt100::Screen, row: u16, cols: u16) -> Vec<(u16, u16, String)> {
        // Build a char→column map so byte positions in the string map back to terminal cols.
        let mut char_to_col: Vec<u16> = Vec::with_capacity(cols as usize);
        let mut row_str = String::with_capacity(cols as usize);
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                if cell.is_wide_continuation() {
                    continue;
                }
                let content = cell.contents();
                if content.is_empty() {
                    char_to_col.push(col);
                    row_str.push(' ');
                } else {
                    for ch in content.chars() {
                        char_to_col.push(col);
                        row_str.push(ch);
                    }
                }
            } else {
                char_to_col.push(col);
                row_str.push(' ');
            }
        }

        let mut spans: Vec<(u16, u16, String)> = Vec::new();
        let mut search_from = 0usize;
        loop {
            let found = ["https://", "http://", "ftp://"]
                .iter()
                .filter_map(|p| {
                    row_str[search_from..]
                        .find(p)
                        .map(|pos| (search_from + pos, *p))
                })
                .min_by_key(|(pos, _)| *pos);
            let Some((abs_start, prefix)) = found else {
                break;
            };
            let url_tail = &row_str[abs_start..];
            let url_end = url_tail
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '>' | '<')
                })
                .unwrap_or(url_tail.len());
            if url_end > prefix.len() {
                let url = url_tail[..url_end].to_string();
                let start_col = char_to_col.get(abs_start).copied().unwrap_or(0);
                let end_col = char_to_col
                    .get(abs_start + url_end - 1)
                    .copied()
                    .unwrap_or(start_col);
                spans.push((start_col, end_col, url));
                search_from = abs_start + url_end;
            } else {
                search_from = abs_start + prefix.len();
            }
        }
        spans
    }

    fn open_url(url: &str) {
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(url).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }

    fn toggle_line_checkbox(markdown: &mut String, line_index: usize, checked: bool) {
        let mut lines: Vec<String> = markdown.lines().map(|s| s.to_owned()).collect();
        if line_index < lines.len() {
            if checked {
                lines[line_index] = lines[line_index].replacen("- [ ] ", "- [x] ", 1);
            } else {
                let line = &lines[line_index];
                if line.contains("- [x] ") {
                    lines[line_index] = line.replacen("- [x] ", "- [ ] ", 1);
                } else if line.contains("- [X] ") {
                    lines[line_index] = line.replacen("- [X] ", "- [ ] ", 1);
                }
            }
            *markdown = lines.join("\n");
        }
    }

    fn insert_checkbox_line(markdown: &mut String) {
        if markdown.ends_with('\n') || markdown.is_empty() {
            markdown.push_str("- [ ] ");
        } else {
            markdown.push_str("\n- [ ] ");
        }
    }

    #[cfg(target_os = "macos")]
    fn apply_macos_share_privacy(&self, enabled: bool) {
        unsafe {
            let ns_app_class = class!(NSApplication);
            let app: *mut Object = msg_send![ns_app_class, sharedApplication];
            if app.is_null() {
                return;
            }

            let windows: *mut Object = msg_send![app, windows];
            if windows.is_null() {
                return;
            }

            let count: usize = msg_send![windows, count];
            for i in 0..count {
                let window: *mut Object = msg_send![windows, objectAtIndex: i];
                if window.is_null() {
                    continue;
                }

                let sharing_type = if enabled { 0isize } else { 1isize };
                let _: () = msg_send![window, setSharingType: sharing_type];
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn apply_macos_share_privacy(&self, _enabled: bool) {}

    fn toggle_fullscreen(ctx: &egui::Context) {
        let is_fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen));
    }

    fn apply_window_mode(&mut self, ctx: &egui::Context) {
        if self.applied_privacy_mode == Some(self.privacy_mode) {
            return;
        }

        self.apply_macos_share_privacy(self.privacy_mode);
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.privacy_mode {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }));
        self.applied_privacy_mode = Some(self.privacy_mode);
    }

    fn active_tab(&self) -> &TerminalTab {
        &self.terminal_tabs[self.active_terminal]
    }

    fn active_tab_mut(&mut self) -> &mut TerminalTab {
        &mut self.terminal_tabs[self.active_terminal]
    }

    fn alloc_pane_uid(&mut self) -> u64 {
        let uid = self.next_pane_uid;
        self.next_pane_uid += 1;
        uid
    }

    fn log_debug(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.debug_log.push_back(msg);
        if self.debug_log.len() > DEBUG_LOG_MAX {
            self.debug_log.pop_front();
        }
    }

    fn add_terminal_tab(&mut self) {
        let cwd = self.active_tab().active_pane().cwd.clone();
        let number = self.next_tab_number;
        self.next_tab_number += 1;
        let uid = self.alloc_pane_uid();
        self.terminal_tabs.push(TerminalTab::new(number, uid, cwd));
        self.active_terminal = self.terminal_tabs.len().saturating_sub(1);
        self.renaming_tab = None;
        self.rename_buffer.clear();
        self.log_debug(format!("add_terminal_tab: tab={number} pane_uid={uid}"));
    }

    fn switch_terminal_tab(&mut self, index: usize) {
        if index < self.terminal_tabs.len() {
            self.active_terminal = index;
        }
    }

    fn move_terminal_tab(&mut self, from: usize, to: usize) {
        if from >= self.terminal_tabs.len() || to >= self.terminal_tabs.len() || from == to {
            return;
        }

        let tab = self.terminal_tabs.remove(from);
        self.terminal_tabs.insert(to, tab);

        self.active_terminal = if self.active_terminal == from {
            to
        } else if from < self.active_terminal && to >= self.active_terminal {
            self.active_terminal.saturating_sub(1)
        } else if from > self.active_terminal && to <= self.active_terminal {
            self.active_terminal.saturating_add(1)
        } else {
            self.active_terminal
        };

        if let Some(index) = self.renaming_tab {
            self.renaming_tab = if index == from {
                Some(to)
            } else if from < index && to >= index {
                Some(index.saturating_sub(1))
            } else if from > index && to <= index {
                Some(index.saturating_add(1))
            } else {
                Some(index)
            };
        }
    }

    fn close_terminal_tab(&mut self, index: usize) {
        if self.terminal_tabs.len() <= 1 || index >= self.terminal_tabs.len() {
            return;
        }

        self.terminal_tabs.remove(index);

        if self.active_terminal >= self.terminal_tabs.len() {
            self.active_terminal = self.terminal_tabs.len().saturating_sub(1);
        } else if index < self.active_terminal {
            self.active_terminal = self.active_terminal.saturating_sub(1);
        }

        if let Some(rename_index) = self.renaming_tab {
            self.renaming_tab = if rename_index == index {
                None
            } else if index < rename_index {
                Some(rename_index.saturating_sub(1))
            } else {
                Some(rename_index)
            };
        }
    }

    fn start_tab_rename(&mut self, index: usize) {
        if index >= self.terminal_tabs.len() {
            return;
        }

        self.renaming_tab = Some(index);
        self.rename_buffer = self.terminal_tabs[index].title.clone();
    }

    fn commit_tab_rename(&mut self) {
        let Some(index) = self.renaming_tab else {
            return;
        };

        let name = self.rename_buffer.trim();
        if !name.is_empty() && index < self.terminal_tabs.len() {
            self.terminal_tabs[index].title = name.to_owned();
        }

        self.renaming_tab = None;
        self.rename_buffer.clear();
    }

    fn cancel_tab_rename(&mut self) {
        self.renaming_tab = None;
        self.rename_buffer.clear();
    }

    fn ansi_index_color(index: u8) -> egui::Color32 {
        match index {
            0 => egui::Color32::from_rgb(0, 0, 0),
            1 => egui::Color32::from_rgb(205, 49, 49),
            2 => egui::Color32::from_rgb(13, 188, 121),
            3 => egui::Color32::from_rgb(229, 229, 16),
            4 => egui::Color32::from_rgb(36, 114, 200),
            5 => egui::Color32::from_rgb(188, 63, 188),
            6 => egui::Color32::from_rgb(17, 168, 205),
            7 => egui::Color32::from_rgb(229, 229, 229),
            8 => egui::Color32::from_rgb(102, 102, 102),
            9 => egui::Color32::from_rgb(241, 76, 76),
            10 => egui::Color32::from_rgb(35, 209, 139),
            11 => egui::Color32::from_rgb(245, 245, 67),
            12 => egui::Color32::from_rgb(59, 142, 234),
            13 => egui::Color32::from_rgb(214, 112, 214),
            14 => egui::Color32::from_rgb(41, 184, 219),
            15 => egui::Color32::from_rgb(255, 255, 255),
            16..=231 => {
                let value = index - 16;
                let r = value / 36;
                let g = (value % 36) / 6;
                let b = value % 6;
                let channel = |component: u8| {
                    if component == 0 {
                        0
                    } else {
                        55 + component * 40
                    }
                };
                egui::Color32::from_rgb(channel(r), channel(g), channel(b))
            }
            232..=255 => {
                let level = 8 + (index - 232) * 10;
                egui::Color32::from_rgb(level, level, level)
            }
        }
    }

    fn resolve_terminal_color(color: vt100::Color, default_color: egui::Color32) -> egui::Color32 {
        match color {
            vt100::Color::Default => default_color,
            vt100::Color::Idx(index) => Self::ansi_index_color(index),
            vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
        }
    }

}

impl eframe::App for GhostStickiesApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.startup_tasks_run {
            self.startup_tasks_run = true;
        }

        self.apply_window_mode(ctx);

        // ── Error toast ──
        if let Some(err) = self.last_error.clone() {
            egui::Window::new("⚠ Error")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(&err);
                    if ui.button("Dismiss").clicked() {
                        self.last_error = None;
                    }
                });
        }

        // ── App-level paste detection (runs regardless of pane focus state) ──
        {
            // CMD_V_PRESSED is set by the low-level NSEvent monitor in install_paste_monitor().
            // It fires even when macOS suppresses the Cmd+V key event from reaching egui
            // (which happens when the clipboard contains only image data).
            let nsevent_cmd_v = CMD_V_PRESSED.swap(false, std::sync::atomic::Ordering::Relaxed);

            // Also watch for egui-level paste events (text paste path).
            let all_events = ctx.input(|i| i.events.clone());
            for e in &all_events {
                if let egui::Event::Paste(t) = e {
                    self.log_debug(format!("app_paste: Event::Paste text.len()={}", t.len()));
                }
            }
            if nsevent_cmd_v {
                self.log_debug("app_paste: NSEvent Cmd+V detected".to_owned());
            }

            // Only attempt image paste when Cmd+V came from the low-level monitor
            // AND there is no text in the clipboard (image-only case).
            if nsevent_cmd_v {
                let has_text = read_clipboard().map(|t| !t.is_empty()).unwrap_or(false);
                self.log_debug(format!("app_paste: has_text={has_text}"));

                if !has_text {
                    self.log_debug("app_paste: no text → save_clipboard_image".to_owned());
                    let mut img_logs: Vec<String> = Vec::new();
                    let ti = self.active_terminal;
                    let pi = self.terminal_tabs[ti].active_pane;
                    if let Some(img_path) = save_clipboard_image(&mut img_logs) {
                        let filename = img_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("image")
                            .to_owned();
                        // Shorten path for terminal display: replace $HOME prefix with ~
                        let path_str = if let Some(home) = std::env::var_os("HOME") {
                            let abs = img_path.to_string_lossy();
                            let home_str = home.to_string_lossy();
                            if abs.starts_with(home_str.as_ref()) {
                                format!("~{}", &abs[home_str.len()..])
                            } else {
                                abs.into_owned()
                            }
                        } else {
                            img_path.to_string_lossy().into_owned()
                        };
                        self.log_debug(format!("app_paste: saved → {path_str}"));
                        self.terminal_tabs[ti].panes[pi].paste_chip = Some(filename);
                        self.terminal_tabs[ti].panes[pi].paste_text(&path_str);
                    } else {
                        self.log_debug("app_paste: save_clipboard_image returned None".to_owned());
                    }
                    for msg in img_logs {
                        self.log_debug(msg);
                    }
                }
            }
        }

        let open_new_tab =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::T));
        let insert_checkbox =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::L));
        let split_pane =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::D));
        let close_pane = ctx.input(|input| {
            input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::D)
        });
        let next_pane = ctx
            .input(|input| input.modifiers.command && input.key_pressed(egui::Key::CloseBracket));
        let prev_pane =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::OpenBracket));
        // Cmd+Shift+Arrow to move/swap the active pane in the grid
        let move_pane_left = ctx.input(|input| {
            input.modifiers.command
                && input.modifiers.shift
                && input.key_pressed(egui::Key::ArrowLeft)
        });
        let move_pane_right = ctx.input(|input| {
            input.modifiers.command
                && input.modifiers.shift
                && input.key_pressed(egui::Key::ArrowRight)
        });
        let move_pane_up = ctx.input(|input| {
            input.modifiers.command
                && input.modifiers.shift
                && input.key_pressed(egui::Key::ArrowUp)
        });
        let move_pane_down = ctx.input(|input| {
            input.modifiers.command
                && input.modifiers.shift
                && input.key_pressed(egui::Key::ArrowDown)
        });
        let toggle_debug = ctx.input(|input| {
            input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::L)
        });

        let zoom_in = ctx.input(|i| {
            i.modifiers.command && i.key_pressed(egui::Key::Equals)
        });
        let zoom_out =
            ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Minus));
        let zoom_reset =
            ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num0));

        let mut received_output = false;
        for tab in &mut self.terminal_tabs {
            tab.ensure_all_started();
            if tab.drain_all_output() {
                received_output = true;
            }
        }

        let had_input = ctx.input(|i| !i.events.is_empty());
        if received_output || had_input {
            self.last_activity = std::time::Instant::now();
            ctx.request_repaint();
        } else if self.last_activity.elapsed() < Duration::from_millis(500) {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        // Autosave notes after 1.5 s of inactivity
        let ti = self.active_terminal;
        if self.terminal_tabs[ti].notes_dirty {
            if let Some(t) = self.terminal_tabs[ti].last_type_time {
                if t.elapsed() > Duration::from_millis(1500) {
                    self.save_current_note_silent();
                }
            }
        }

        if open_new_tab {
            self.add_terminal_tab();
        }

        if split_pane && !close_pane {
            let uid = self.alloc_pane_uid();
            self.active_tab_mut().split_pane(uid);
            self.log_debug(format!(
                "split_pane: new pane_uid={uid}, total_panes={}",
                self.active_tab().panes.len()
            ));
        }

        if close_pane {
            let before = self.active_tab().panes.len();
            self.active_tab_mut().close_active_pane();
            self.log_debug(format!(
                "close_pane: {before} -> {} panes",
                self.active_tab().panes.len()
            ));
        }

        if next_pane {
            let before = self.active_tab().active_pane;
            self.active_tab_mut().focus_next_pane();
            let after = self.active_tab().active_pane;
            self.log_debug(format!("focus_next_pane: {before} -> {after}"));
        }

        if prev_pane {
            let before = self.active_tab().active_pane;
            self.active_tab_mut().focus_prev_pane();
            let after = self.active_tab().active_pane;
            self.log_debug(format!("focus_prev_pane: {before} -> {after}"));
        }

        // Move active pane in the grid
        let mut kbd_swap: Option<(usize, usize)> = None;
        {
            let tab = self.active_tab_mut();
            let n = tab.panes.len();
            if n > 1 {
                let idx = tab.active_pane;
                let (cols, _rows) = Self::grid_dims(n);

                let swap_with = if move_pane_left && idx % cols > 0 {
                    Some(idx - 1)
                } else if move_pane_right && idx % cols < cols - 1 && idx + 1 < n {
                    Some(idx + 1)
                } else if move_pane_up && idx >= cols {
                    Some(idx - cols)
                } else if move_pane_down && idx + cols < n {
                    Some(idx + cols)
                } else {
                    None
                };

                if let Some(target) = swap_with {
                    tab.panes.swap(idx, target);
                    tab.active_pane = target;
                    kbd_swap = Some((idx, target));
                }
            }
        }
        if let Some((from, to)) = kbd_swap {
            self.log_debug(format!("keyboard_swap_pane: {from} <-> {to}"));
        }

        if insert_checkbox && self.sidebar_open {
            let ti = self.active_terminal;
            Self::insert_checkbox_line(&mut self.terminal_tabs[ti].notes_markdown);
            self.terminal_tabs[ti].editing_notes = true;
        }

        if toggle_debug {
            self.show_debug = !self.show_debug;
            self.log_debug(format!("debug window toggled: {}", self.show_debug));
        }

        if let Some(tab) = self.terminal_tabs.get_mut(self.active_terminal) {
            if let Some(pane) = tab.panes.get_mut(tab.active_pane) {
                if zoom_in {
                    pane.font_scale = (pane.font_scale + 0.1).min(2.5);
                }
                if zoom_out {
                    pane.font_scale = (pane.font_scale - 0.1).max(0.5);
                }
                if zoom_reset {
                    pane.font_scale = 1.0;
                }
            }
        }

        let palette = self.theme.palette();

        let mut style = (*ctx.style()).clone();
        style.visuals.window_fill = palette.bg;
        style.visuals.panel_fill = palette.bg;
        style.visuals.extreme_bg_color = palette.input_bg;
        style.visuals.selection.bg_fill = palette.selection;
        style.visuals.widgets.active.bg_fill = palette.surface;
        style.visuals.widgets.hovered.bg_fill = palette.surface;
        style.visuals.widgets.inactive.bg_fill = palette.tab_bg;
        style.visuals.widgets.noninteractive.bg_fill = palette.surface;
        style.visuals.widgets.active.fg_stroke.color = palette.text;
        style.visuals.widgets.hovered.fg_stroke.color = palette.text;
        style.visuals.widgets.inactive.fg_stroke.color = palette.tab_text;
        style.visuals.widgets.active.weak_bg_fill = palette.surface;
        style.visuals.widgets.hovered.weak_bg_fill = palette.surface;
        style.visuals.widgets.inactive.weak_bg_fill = palette.tab_bg;
        style.visuals.override_text_color = Some(palette.text);
        style.visuals.window_corner_radius = egui::CornerRadius::same(12);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
        ctx.set_style(style);

        let mut start_drag = false;
        let mut quit_requested = false;
        let mut privacy_toggled = false;
        let mut theme_changed = None;

        // ── Top bar ──
        egui::TopBottomPanel::top("top_bar")
            .exact_height(TOP_BAR_HEIGHT)
            .frame(
                egui::Frame::NONE
                    .fill(palette.bar_bg)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| {
                let drag_response = ui.interact(
                    ui.max_rect(),
                    ui.id().with("top_bar_drag"),
                    egui::Sense::drag(),
                );
                if drag_response.dragged() {
                    start_drag = true;
                }

                // Draw "StickyTerminal" centered over the bar (painted before layout so it's behind controls)
                ui.painter().text(
                    ui.max_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    "StickyTerminal",
                    egui::FontId::proportional(13.0),
                    palette.muted_text,
                );

                ui.horizontal(|ui| {
                    // Traffic light buttons
                    let dot_size = egui::vec2(12.0, 12.0);

                    let (close_rect, close_resp) =
                        ui.allocate_exact_size(dot_size, egui::Sense::click());
                    let close_color = if close_resp.hovered() {
                        egui::Color32::from_rgb(255, 95, 86)
                    } else {
                        egui::Color32::from_rgb(255, 95, 86).linear_multiply(0.7)
                    };
                    ui.painter()
                        .circle_filled(close_rect.center(), 6.0, close_color);
                    if close_resp.clicked() {
                        quit_requested = true;
                    }

                    ui.add_space(4.0);
                    let (min_rect, min_resp) =
                        ui.allocate_exact_size(dot_size, egui::Sense::click());
                    let min_color = if min_resp.hovered() {
                        egui::Color32::from_rgb(255, 189, 46)
                    } else {
                        egui::Color32::from_rgb(255, 189, 46).linear_multiply(0.7)
                    };
                    ui.painter()
                        .circle_filled(min_rect.center(), 6.0, min_color);
                    if min_resp.clicked() {
                        self.minimized = !self.minimized;
                        let target_height = if self.minimized {
                            MINIMIZED_HEIGHT
                        } else {
                            WINDOW_HEIGHT
                        };
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            WINDOW_WIDTH,
                            target_height,
                        )));
                    }

                    ui.add_space(4.0);
                    let (max_rect, max_resp) =
                        ui.allocate_exact_size(dot_size, egui::Sense::click());
                    let max_color = if max_resp.hovered() {
                        egui::Color32::from_rgb(39, 201, 63)
                    } else {
                        egui::Color32::from_rgb(39, 201, 63).linear_multiply(0.7)
                    };
                    ui.painter()
                        .circle_filled(max_rect.center(), 6.0, max_color);
                    if max_resp.clicked() {
                        Self::toggle_fullscreen(ctx);
                    }

                    ui.add_space(16.0);

                    // Sidebar toggle
                    let sidebar_icon_color = if self.sidebar_open {
                        palette.accent
                    } else {
                        palette.muted_text
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("|||")
                                    .size(11.0)
                                    .color(sidebar_icon_color),
                            )
                            .frame(false),
                        )
                        .on_hover_text(if self.sidebar_open {
                            "Hide sidebar"
                        } else {
                            "Show sidebar"
                        })
                        .clicked()
                    {
                        self.sidebar_open = !self.sidebar_open;
                        self.save_config();
                    }

                    ui.add_space(12.0);

                    ui.add_space(8.0);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if Self::symbol_button(
                            ui,
                            AppSymbol::Privacy,
                            if self.privacy_mode {
                                "Disable privacy mode"
                            } else {
                                "Enable privacy mode"
                            },
                            self.privacy_mode,
                        )
                        .clicked()
                        {
                            privacy_toggled = true;
                        }

                        ui.menu_button(
                            egui::RichText::new("Help")
                                .size(12.0)
                                .color(palette.muted_text),
                            |ui| {
                                let logs_label = if self.show_debug {
                                    "Hide Logs"
                                } else {
                                    "View Logs"
                                };
                                if ui.selectable_label(self.show_debug, logs_label).clicked() {
                                    self.show_debug = !self.show_debug;
                                    ui.close();
                                }
                            },
                        );

                        ui.menu_button(
                            egui::RichText::new("Theme")
                                .size(12.0)
                                .color(palette.muted_text),
                            |ui| {
                                for preset in ThemePreset::ALL {
                                    let label = if self.theme == preset {
                                        egui::RichText::new(preset.label()).color(palette.accent)
                                    } else {
                                        egui::RichText::new(preset.label()).color(palette.text)
                                    };
                                    if ui.selectable_label(self.theme == preset, label).clicked() {
                                        theme_changed = Some(preset);
                                        ui.close();
                                    }
                                }
                            },
                        );
                    });
                });
            });

        if let Some(theme) = theme_changed {
            self.theme = theme;
            self.save_config();
        }

        if privacy_toggled {
            self.privacy_mode = !self.privacy_mode;
            self.apply_window_mode(ctx);
            self.save_config();
        }

        if quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if start_drag {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        if self.minimized {
            return;
        }

        // ── Tab bar ──
        egui::TopBottomPanel::top("tab_bar")
            .exact_height(TAB_BAR_HEIGHT)
            .frame(
                egui::Frame::NONE
                    .fill(palette.bar_bg)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ctx, |ui| {
                let (switch_to, close_tab, rename_tab, move_tab) = self.render_tab_bar(ui, palette);

                if let Some((from, to)) = move_tab {
                    self.move_terminal_tab(from, to);
                }

                if let Some(index) = close_tab {
                    self.close_terminal_tab(index);
                }

                if let Some(index) = rename_tab {
                    self.start_tab_rename(index);
                }

                if let Some(index) = switch_to {
                    self.switch_terminal_tab(index);
                }
            });

        // ── Sidebar (Notes) ──
        if self.sidebar_open {
            egui::SidePanel::left("notes_sidebar")
                .resizable(true)
                .default_width(SIDEBAR_DEFAULT_WIDTH)
                .width_range(250.0..=560.0)
                .frame(
                    egui::Frame::NONE
                        .fill(palette.sidebar_bg)
                        .inner_margin(egui::Margin::same(12)),
                )
                .show(ctx, |ui| {
                    let ti = self.active_terminal;
                    let mut choose_folder = false;
                    let mut open_note = false;
                    let mut new_note = false;
                    let mut save_note = false;
                    let mut open_recent_note: Option<PathBuf> = None;
                    let mut text_edit_changed = false;
                    let note_text = self.terminal_tabs[ti]
                        .current_note_file
                        .as_ref()
                        .map(|path| {
                            if let Some(root) = &self.notes_root {
                                path.strip_prefix(root)
                                    .map(|relative| relative.display().to_string())
                                    .unwrap_or_else(|_| path.display().to_string())
                            } else {
                                path.display().to_string()
                            }
                        })
                        .unwrap_or_else(|| "No note selected".to_owned());

                    // Header row
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Notes")
                                .strong()
                                .size(16.0)
                                .color(palette.text),
                        );
                        if self.terminal_tabs[ti].notes_dirty {
                            ui.label(
                                egui::RichText::new("\u{25cf}")
                                    .small()
                                    .color(palette.accent.linear_multiply(0.8)),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let toggle_label = if self.terminal_tabs[ti].editing_notes {
                                "Preview"
                            } else {
                                "Edit"
                            };
                            if ui
                                .add(Self::note_action_button(toggle_label, palette))
                                .clicked()
                            {
                                self.terminal_tabs[ti].editing_notes =
                                    !self.terminal_tabs[ti].editing_notes;
                            }
                        });
                    });
                    ui.add_space(4.0);

                    // File controls
                    Self::note_surface_frame(palette).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&note_text)
                                    .small()
                                    .color(palette.muted_text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;

                                    if ui.add(Self::note_action_button("Save", palette)).clicked() {
                                        save_note = true;
                                    }
                                    if ui.add(Self::note_action_button("New", palette)).clicked() {
                                        new_note = true;
                                    }
                                    if ui.add(Self::note_action_button("Open", palette)).clicked() {
                                        open_note = true;
                                    }
                                    {
                                        let recent_copy = self.recent_notes.clone();
                                        if !recent_copy.is_empty() {
                                            let _ = egui::containers::menu::MenuButton::from_button(
                                                Self::note_action_button("Recent", palette),
                                            )
                                            .ui(ui, |ui| {
                                                for path in &recent_copy {
                                                    let name = path
                                                        .file_name()
                                                        .and_then(|n| n.to_str())
                                                        .unwrap_or("?");
                                                    if ui.selectable_label(false, name).clicked() {
                                                        open_recent_note = Some(path.clone());
                                                        ui.close();
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    if ui
                                        .add(Self::note_action_button("Folder", palette))
                                        .clicked()
                                    {
                                        choose_folder = true;
                                    }
                                },
                            );
                        });
                    });

                    ui.add_space(6.0);

                    let status_height = 20.0;
                    let content_height = (ui.available_height() - status_height - 8.0).max(100.0);

                    Self::note_surface_frame(palette).show(ui, |ui| {
                        if self.terminal_tabs[ti].editing_notes {
                            egui::ScrollArea::vertical()
                                .max_height(content_height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    let r = ui.add(
                                        egui::TextEdit::multiline(
                                            &mut self.terminal_tabs[ti].notes_markdown,
                                        )
                                        .desired_width(ui.available_width())
                                        .desired_rows(40)
                                        .hint_text("Write markdown here. Cmd+L to add a checkbox."),
                                    );
                                    if r.changed() {
                                        text_edit_changed = true;
                                    }
                                });
                        } else {
                            // Rebuild the parsed-block cache only when content changes.
                            {
                                let current_hash =
                                    hash_string(&self.terminal_tabs[ti].notes_markdown);
                                let cache_valid = self.terminal_tabs[ti]
                                    .notes_render_cache
                                    .as_ref()
                                    .map(|(h, _)| *h == current_hash)
                                    .unwrap_or(false);
                                if !cache_valid {
                                    let parsed =
                                        parse_markdown(&self.terminal_tabs[ti].notes_markdown);
                                    self.terminal_tabs[ti].notes_render_cache =
                                        Some((current_hash, parsed));
                                }
                            }
                            // Render from the cached blocks. Use field destructuring so the
                            // borrow checker can see that `notes_render_cache` (read) and
                            // `notes_markdown` (write, for checkbox toggles) are disjoint.
                            let preview_changed = {
                                let tab = &mut self.terminal_tabs[ti];
                                let TerminalTab {
                                    ref notes_render_cache,
                                    ref mut notes_markdown,
                                    ..
                                } = *tab;
                                let blocks =
                                    notes_render_cache.as_ref().unwrap().1.as_slice();
                                Self::render_from_blocks(
                                    ui,
                                    blocks,
                                    notes_markdown,
                                    palette,
                                    content_height,
                                )
                                // `blocks` drops here; borrow of notes_render_cache ends.
                            };
                            if preview_changed {
                                // Invalidate cache so it is rebuilt with updated checkboxes.
                                self.terminal_tabs[ti].notes_render_cache = None;
                                save_note = true;
                            }
                        }
                    });

                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&self.terminal_tabs[ti].note_status)
                            .small()
                            .color(palette.muted_text),
                    );

                    if choose_folder {
                        self.choose_notes_root();
                    }

                    if open_note {
                        self.choose_existing_note();
                    }

                    if new_note {
                        self.create_new_note();
                    }

                    if save_note {
                        self.save_current_note();
                    }

                    if let Some(path) = open_recent_note {
                        let ti = self.active_terminal;
                        self.terminal_tabs[ti].current_note_file = Some(path);
                        self.load_current_note();
                    }

                    if text_edit_changed {
                        let ti = self.active_terminal;
                        self.terminal_tabs[ti].notes_dirty = true;
                        self.terminal_tabs[ti].last_type_time = Some(std::time::Instant::now());
                    }
                });
        }

        // ── Debug log window ──
        if self.show_debug {
            egui::Window::new("Debug Log")
                .default_size([480.0, 320.0])
                .collapsible(true)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{} entries", self.debug_log.len()))
                                .small()
                                .color(palette.muted_text),
                        );
                        if ui.button("Clear").clicked() {
                            self.debug_log.clear();
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for entry in &self.debug_log {
                                ui.label(
                                    egui::RichText::new(entry)
                                        .monospace()
                                        .size(11.0)
                                        .color(palette.text),
                                );
                            }
                        });
                });
        }

        // ── Central panel: terminal panes ──
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(palette.bg)
                    .inner_margin(egui::Margin::same(4)),
            )
            .show(ctx, |ui| {
                self.render_panes(ui, palette, ctx);
            });
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.palette().bg.to_normalized_gamma_f32()
    }
}
