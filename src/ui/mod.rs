pub(crate) mod command_palette;
pub(crate) mod pane;
pub(crate) mod scratchpad;
pub(crate) mod sidebar;
pub(crate) mod tab_bar;

use eframe::egui;
use rfd::FileDialog;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::AppConfig;
use crate::terminal::{
    default_terminal_cwd, AiOutputPane, BrowserPane, Pane, TerminalPane, TerminalTab,
};
use crate::terminal::clipboard::{install_paste_monitor, read_clipboard, save_clipboard_image, take_cmd_v_pressed};
use crate::theme::{ThemePalette, ThemePreset};
use crate::watcher::{ChangeKind, FileWatcher};

use self::command_palette::{CommandPaletteState, PaletteAction};
use self::pane::{
    find_row_url_spans, open_url, render_pane, render_pane_dispatch, resolve_terminal_color,
};
use self::scratchpad::ScratchpadState;

const WINDOW_WIDTH: f32 = 1180.0;
const WINDOW_HEIGHT: f32 = 760.0;
const TOP_BAR_HEIGHT: f32 = 40.0;
const TAB_BAR_HEIGHT: f32 = 38.0;
const MINIMIZED_HEIGHT: f32 = 40.0;
const SIDEBAR_DEFAULT_WIDTH: f32 = 340.0;
const PANE_SEPARATOR_WIDTH: f32 = 1.0;
const DEBUG_LOG_MAX: usize = 200;

#[derive(Clone, Copy)]
enum AppSymbol {
    Privacy,
}

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
    debug_log: VecDeque<String>,
    show_debug: bool,
    recent_notes: Vec<PathBuf>,
    renaming_pane: Option<(usize, usize)>,
    pane_rename_buffer: String,
    // Feature 3: Scratchpad
    scratchpad: ScratchpadState,
    // Feature 7: Command Palette
    command_palette: CommandPaletteState,
    // Feature 6: Checkpoints
    checkpoint_label: String,
    checkpoints: Vec<(String, String)>,
    checkpoint_error: Option<String>,
    // Feature 4: File Watcher
    file_watcher: Option<FileWatcher>,
    last_error: Option<String>,
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
            scratchpad: ScratchpadState::default(),
            command_palette: CommandPaletteState::default(),
            checkpoint_label: String::new(),
            checkpoints: Vec::new(),
            checkpoint_error: None,
            file_watcher: None,
            last_error: None,
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

        let Ok(config) = serde_json::from_str::<AppConfig>(&contents) else {
            self.terminal_tabs[0].note_status =
                "Could not read saved settings. Using defaults.".to_owned();
            return;
        };

        self.theme = config.theme;
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

    fn save_config(&mut self) {
        let ti = self.active_terminal;
        let config = AppConfig {
            notes_root: self.notes_root.clone(),
            current_note_file: self.terminal_tabs[ti].current_note_file.clone(),
            theme: self.theme,
            recent_notes: self.recent_notes.clone(),
        };

        let support_dir = Self::app_support_dir();
        if let Err(err) = fs::create_dir_all(&support_dir) {
            self.terminal_tabs[ti].note_status =
                format!("Could not create app settings folder: {err}");
            return;
        }

        match serde_json::to_string_pretty(&config) {
            Ok(contents) => {
                if let Err(err) = fs::write(Self::config_path(), contents) {
                    self.terminal_tabs[ti].note_status = format!("Could not save settings: {err}");
                }
            }
            Err(err) => {
                self.terminal_tabs[ti].note_status = format!("Could not encode settings: {err}");
            }
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

    fn render_markdown_preview(
        ui: &mut egui::Ui,
        markdown: &mut String,
        palette: ThemePalette,
        available_height: f32,
    ) -> bool {
        let mut changed = false;
        let indent_px = 16.0;

        egui::ScrollArea::vertical()
            .max_height(available_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;

                let lines: Vec<String> = markdown.lines().map(|s| s.to_owned()).collect();
                let mut line_idx = 0;

                while line_idx < lines.len() {
                    let line = &lines[line_idx];
                    let trimmed = line.trim();
                    let indent = Self::indent_level(line);
                    let left_margin = indent as f32 * indent_px;

                    if trimmed.is_empty() {
                        ui.add_space(6.0);
                        line_idx += 1;
                        continue;
                    }

                    if let Some(heading) = trimmed.strip_prefix("### ") {
                        ui.add_space(2.0);
                        Self::markdown_label(ui, heading, palette, palette.text, 15.0, false);
                    } else if let Some(heading) = trimmed.strip_prefix("## ") {
                        ui.add_space(6.0);
                        Self::markdown_label(ui, heading, palette, palette.text, 17.0, false);
                        let rule = egui::vec2(ui.available_width().min(160.0), 2.0);
                        let (rect, _) = ui.allocate_exact_size(rule, egui::Sense::hover());
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(2),
                            palette.accent.linear_multiply(0.45),
                        );
                        ui.add_space(2.0);
                    } else if let Some(heading) = trimmed.strip_prefix("# ") {
                        ui.add_space(8.0);
                        Self::markdown_label(ui, heading, palette, palette.text, 21.0, false);
                        let rule = egui::vec2(ui.available_width().min(220.0), 2.0);
                        let (rect, _) = ui.allocate_exact_size(rule, egui::Sense::hover());
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(2),
                            palette.accent.linear_multiply(0.6),
                        );
                        ui.add_space(4.0);
                    } else if trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
                        let task_text = &trimmed[6..];
                        ui.horizontal_wrapped(|ui| {
                            if left_margin > 0.0 {
                                ui.add_space(left_margin);
                            }
                            let mut checked = true;
                            if Self::markdown_checkbox(ui, &mut checked, palette).changed() {
                                Self::toggle_line_checkbox(markdown, line_idx, false);
                                changed = true;
                            }
                            Self::markdown_label(
                                ui,
                                task_text,
                                palette,
                                palette.muted_text,
                                14.0,
                                true,
                            );
                        });
                    } else if trimmed.starts_with("- [ ] ") {
                        let task_text = &trimmed[6..];
                        ui.horizontal_wrapped(|ui| {
                            if left_margin > 0.0 {
                                ui.add_space(left_margin);
                            }
                            let mut checked = false;
                            if Self::markdown_checkbox(ui, &mut checked, palette).changed() {
                                Self::toggle_line_checkbox(markdown, line_idx, true);
                                changed = true;
                            }
                            Self::markdown_label(ui, task_text, palette, palette.text, 14.0, false);
                        });
                    } else if let Some(bullet_text) = trimmed
                        .strip_prefix("- ")
                        .or_else(|| trimmed.strip_prefix("* "))
                    {
                        ui.horizontal_wrapped(|ui| {
                            if left_margin > 0.0 {
                                ui.add_space(left_margin);
                            }
                            ui.label(
                                egui::RichText::new("\u{2022}")
                                    .size(16.0)
                                    .color(palette.accent),
                            );
                            Self::markdown_label(
                                ui,
                                bullet_text,
                                palette,
                                palette.text,
                                14.0,
                                false,
                            );
                        });
                    } else if let Some((number, item_text)) =
                        trimmed.split_once(". ").filter(|(n, _)| {
                            !n.is_empty() && n.chars().all(|ch| ch.is_ascii_digit())
                        })
                    {
                        ui.horizontal_wrapped(|ui| {
                            if left_margin > 0.0 {
                                ui.add_space(left_margin);
                            }
                            ui.label(
                                egui::RichText::new(format!("{number}."))
                                    .strong()
                                    .color(palette.accent),
                            );
                            Self::markdown_label(
                                ui,
                                item_text,
                                palette,
                                palette.text,
                                14.0,
                                false,
                            );
                        });
                    } else if let Some(quote_text) = trimmed.strip_prefix("> ") {
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
                                        quote_text,
                                        palette,
                                        palette.muted_text,
                                        14.0,
                                        false,
                                    );
                                });
                            });
                    } else if trimmed.starts_with("```") {
                        let mut code_lines = Vec::new();
                        line_idx += 1;

                        while line_idx < lines.len() {
                            let code_line = &lines[line_idx];
                            if code_line.trim().starts_with("```") {
                                break;
                            }
                            code_lines.push(code_line.clone());
                            line_idx += 1;
                        }

                        let code_block = if code_lines.is_empty() {
                            " ".to_owned()
                        } else {
                            code_lines.join("\n")
                        };

                        egui::Frame::NONE
                            .fill(palette.input_bg)
                            .stroke(egui::Stroke::new(1.0, palette.border))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(code_block)
                                            .monospace()
                                            .size(13.0)
                                            .color(egui::Color32::WHITE),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                            });
                    } else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                    } else {
                        if left_margin > 0.0 {
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(left_margin);
                                Self::markdown_label(
                                    ui,
                                    trimmed,
                                    palette,
                                    palette.text,
                                    14.0,
                                    false,
                                );
                            });
                        } else {
                            Self::markdown_label(ui, trimmed, palette, palette.text, 14.0, false);
                        }
                    }

                    line_idx += 1;
                }
            });

        changed
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
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
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
        let cwd = self.active_tab()
            .panes
            .get(self.active_tab().active_pane)
            .and_then(|p| p.as_terminal())
            .map(|t| t.cwd.clone())
            .unwrap_or_else(default_terminal_cwd);
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

    // Feature 6: Checkpoint methods
    fn create_checkpoint(&mut self) {
        let label = self.checkpoint_label.trim().to_owned();
        if label.is_empty() { return; }
        let ti = self.active_terminal;
        let pi = self.terminal_tabs[ti].active_pane;
        let cwd = self.terminal_tabs[ti].panes.get(pi)
            .and_then(|p| p.as_terminal())
            .map(|t| t.cwd.clone())
            .unwrap_or_else(default_terminal_cwd);

        let msg = format!("stickyterminal: {}", label);
        let result = std::process::Command::new("git")
            .args(["stash", "push", "--include-untracked", "-m", &msg])
            .current_dir(&cwd)
            .output();

        match result {
            Ok(out) if out.status.success() => {
                self.checkpoint_error = None;
                self.checkpoint_label.clear();
                self.refresh_checkpoints(&cwd.clone());
            }
            Ok(out) => {
                self.checkpoint_error =
                    Some(String::from_utf8_lossy(&out.stderr).trim().to_owned());
            }
            Err(e) => { self.checkpoint_error = Some(e.to_string()); }
        }
    }

    fn refresh_checkpoints(&mut self, cwd: &std::path::Path) {
        let result = std::process::Command::new("git")
            .args(["stash", "list", "--format=%gd|%s"])
            .current_dir(cwd)
            .output();
        if let Ok(out) = result {
            if out.status.success() {
                self.checkpoints = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter_map(|line| {
                        let mut parts = line.splitn(2, '|');
                        let stash_ref = parts.next()?.trim().to_owned();
                        let subject = parts.next()?.trim().to_owned();
                        let label = subject
                            .strip_prefix("stickyterminal: ")
                            .unwrap_or(&subject)
                            .to_owned();
                        Some((label, stash_ref))
                    })
                    .take(5)
                    .collect();
            }
        }
    }

    fn restore_checkpoint(&mut self, stash_ref: String) {
        let ti = self.active_terminal;
        let pi = self.terminal_tabs[ti].active_pane;
        let cwd = self.terminal_tabs[ti].panes.get(pi)
            .and_then(|p| p.as_terminal())
            .map(|t| t.cwd.clone())
            .unwrap_or_else(default_terminal_cwd);

        let result = std::process::Command::new("git")
            .args(["stash", "apply", &stash_ref])
            .current_dir(&cwd)
            .output();
        match result {
            Ok(out) if !out.status.success() => {
                self.checkpoint_error =
                    Some(String::from_utf8_lossy(&out.stderr).trim().to_owned());
            }
            Err(e) => { self.checkpoint_error = Some(e.to_string()); }
            _ => {
                self.checkpoint_error = None;
                self.refresh_checkpoints(&cwd.clone());
            }
        }
    }

    // Feature 3: Scratchpad render
    fn render_scratchpad(&mut self, ctx: &egui::Context, palette: ThemePalette) {
        if !self.scratchpad.open { return; }

        let mut send = false;
        let mut close = false;

        egui::Window::new("Prompt Scratchpad")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(true)
            .collapsible(false)
            .default_size([520.0, 280.0])
            .frame(
                egui::Frame::window(ctx.style().as_ref())
                    .fill(palette.sidebar_bg)
                    .stroke(egui::Stroke::new(1.0, palette.border)),
            )
            .show(ctx, |ui| {
                if !self.scratchpad.history.is_empty() {
                    ui.label(egui::RichText::new("Recent").color(palette.muted_text).small());
                    egui::ScrollArea::vertical()
                        .max_height(80.0)
                        .id_salt("scratch_hist")
                        .show(ui, |ui| {
                            let mut restore = None;
                            for (i, entry) in self.scratchpad.history.iter().enumerate() {
                                let preview = if entry.len() > 60 {
                                    format!("{}...", &entry[..60])
                                } else {
                                    entry.clone()
                                };
                                if ui.selectable_label(false, &preview).clicked() {
                                    restore = Some(i);
                                }
                            }
                            if let Some(i) = restore {
                                self.scratchpad.buffer = self.scratchpad.history[i].clone();
                            }
                        });
                    ui.separator();
                }

                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.scratchpad.buffer)
                        .desired_width(f32::INFINITY)
                        .desired_rows(6)
                        .hint_text(
                            "Type your prompt here... (Enter to send, Shift+Enter for newline)",
                        ),
                );

                if self.scratchpad.open {
                    response.request_focus();
                }

                if response.has_focus()
                    && ctx.input(|i| {
                        i.key_pressed(egui::Key::Enter) && !i.modifiers.shift
                    })
                {
                    send = true;
                }

                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Send").strong()).clicked() {
                        send = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });

        if send && !self.scratchpad.buffer.trim().is_empty() {
            let text = self.scratchpad.buffer.trim().to_owned();
            self.scratchpad.history.retain(|e| e != &text);
            self.scratchpad.history.push_front(text.clone());
            self.scratchpad.history.truncate(20);
            let ti = self.active_terminal;
            let pi = self.terminal_tabs[ti].active_pane;
            if let Some(pane) = self.terminal_tabs[ti].panes.get_mut(pi) {
                if let Some(term) = pane.as_terminal_mut() {
                    term.write_bytes(text.as_bytes());
                    term.write_bytes(b"\r");
                }
            }
            self.scratchpad.buffer.clear();
            self.scratchpad.open = false;
        }

        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.scratchpad.open = false;
        }
    }

    // Feature 7: Command Palette render
    fn render_command_palette(&mut self, ctx: &egui::Context, palette: ThemePalette) {
        if !self.command_palette.open { return; }

        let mut items: Vec<PaletteAction> = Vec::new();
        for (i, tab) in self.terminal_tabs.iter().enumerate() {
            items.push(PaletteAction::SwitchTab(i, tab.title.clone()));
        }
        for note in &self.recent_notes.clone() {
            items.push(PaletteAction::OpenNote(note.clone()));
        }
        items.push(PaletteAction::NewTab);
        items.push(PaletteAction::SplitPane);
        items.push(PaletteAction::ToggleSidebar);
        items.push(PaletteAction::SetTheme(ThemePreset::Warp));
        items.push(PaletteAction::SetTheme(ThemePreset::WarpLight));
        items.push(PaletteAction::SetTheme(ThemePreset::Terminal));
        items.push(PaletteAction::SetTheme(ThemePreset::Midnight));
        items.push(PaletteAction::TogglePrivacy);

        let q = self.command_palette.query.to_lowercase();
        let filtered: Vec<PaletteAction> = if q.is_empty() {
            items
        } else {
            let mut prefix: Vec<PaletteAction> = items
                .iter()
                .filter(|a| a.label().to_lowercase().starts_with(&q))
                .cloned()
                .collect();
            let mut contains: Vec<PaletteAction> = items
                .iter()
                .filter(|a| {
                    !a.label().to_lowercase().starts_with(&q)
                        && a.label().to_lowercase().contains(&q)
                })
                .cloned()
                .collect();
            prefix.append(&mut contains);
            prefix
        };
        let visible: Vec<PaletteAction> = filtered.into_iter().take(8).collect();

        if self.command_palette.selected >= visible.len() {
            self.command_palette.selected = visible.len().saturating_sub(1);
        }

        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.command_palette.selected = (self.command_palette.selected + 1)
                .min(visible.len().saturating_sub(1));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.command_palette.selected = self.command_palette.selected.saturating_sub(1);
        }

        let mut execute: Option<PaletteAction> = None;
        let mut close = false;

        egui::Window::new("##cmd_palette")
            .title_bar(false)
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(0.0, TOP_BAR_HEIGHT + TAB_BAR_HEIGHT),
            )
            .fixed_size([480.0, 320.0])
            .frame(
                egui::Frame::window(ctx.style().as_ref())
                    .fill(palette.sidebar_bg)
                    .stroke(egui::Stroke::new(1.5, palette.accent)),
            )
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.command_palette.query)
                        .desired_width(f32::INFINITY)
                        .hint_text("Search tabs, notes, actions...")
                        .font(egui::TextStyle::Body),
                );
                response.request_focus();

                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, action) in visible.iter().enumerate() {
                        let selected = i == self.command_palette.selected;
                        let bg = if selected {
                            palette.accent.linear_multiply(0.2)
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        let frame = egui::Frame::NONE
                            .fill(bg)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 4));
                        frame.show(ui, |ui| {
                            let label = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(action.label()).color(if selected {
                                        palette.accent
                                    } else {
                                        palette.text
                                    }),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if label.clicked() {
                                execute = Some(action.clone());
                            }
                        });
                    }
                });

                if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(action) = visible.get(self.command_palette.selected) {
                        execute = Some(action.clone());
                    }
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });

        if let Some(action) = execute {
            self.command_palette.open = false;
            self.command_palette.query.clear();
            match action {
                PaletteAction::SwitchTab(i, _) => self.switch_terminal_tab(i),
                PaletteAction::OpenNote(path) => {
                    let ti = self.active_terminal;
                    self.terminal_tabs[ti].current_note_file = Some(path);
                    self.load_current_note();
                }
                PaletteAction::NewTab => self.add_terminal_tab(),
                PaletteAction::SplitPane => {
                    let ti = self.active_terminal;
                    let uid = self.alloc_pane_uid();
                    let pi = self.terminal_tabs[ti].active_pane;
                    let cwd = self.terminal_tabs[ti].panes[pi]
                        .as_terminal()
                        .map(|t| t.cwd.clone())
                        .unwrap_or_else(default_terminal_cwd);
                    self.terminal_tabs[ti]
                        .panes
                        .push(Pane::Terminal(TerminalPane::new(uid, cwd)));
                }
                PaletteAction::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
                PaletteAction::SetTheme(t) => {
                    self.theme = t;
                    self.save_config();
                }
                PaletteAction::TogglePrivacy => {
                    self.privacy_mode = !self.privacy_mode;
                    // apply_window_mode will be called next frame
                    self.save_config();
                }
            }
        }

        if close {
            self.command_palette.open = false;
        }
    }

    /// Render the tab bar
    fn render_tab_bar(
        &mut self,
        ui: &mut egui::Ui,
        palette: ThemePalette,
    ) -> (
        Option<usize>,
        Option<usize>,
        Option<usize>,
        Option<(usize, usize)>,
    ) {
        let mut switch_to = None;
        let mut close_tab = None;
        let mut rename_tab = None;
        let mut move_tab = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;

            for index in 0..self.terminal_tabs.len() {
                let selected = index == self.active_terminal;
                let renaming = self.renaming_tab == Some(index);

                if renaming {
                    let response = ui.add_sized(
                        [140.0, 28.0],
                        egui::TextEdit::singleline(&mut self.rename_buffer)
                            .clip_text(false)
                            .desired_width(140.0),
                    );

                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        self.commit_tab_rename();
                    }

                    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                        self.cancel_tab_rename();
                    }

                    response.request_focus();
                    continue;
                }

                let (tab_fill, tab_text_color) = if selected {
                    (palette.active_tab_bg, palette.active_tab_text)
                } else {
                    (egui::Color32::TRANSPARENT, palette.tab_text)
                };

                let tab_label = {
                    let pane_count = self.terminal_tabs[index].panes.len();
                    if pane_count > 1 {
                        format!("{} ({})", self.terminal_tabs[index].title, pane_count)
                    } else {
                        self.terminal_tabs[index].title.clone()
                    }
                };

                let tab_frame = egui::Frame::NONE
                    .fill(tab_fill)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 4));

                let response = tab_frame
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&tab_label)
                                .size(12.5)
                                .color(tab_text_color),
                        );
                    })
                    .response;

                let response = response.interact(egui::Sense::click_and_drag());

                if response.clicked() {
                    switch_to = Some(index);
                }

                response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        rename_tab = Some(index);
                        ui.close();
                    }

                    if ui
                        .add_enabled(self.terminal_tabs.len() > 1, egui::Button::new("Close"))
                        .clicked()
                    {
                        close_tab = Some(index);
                        ui.close();
                    }
                });

                if response.dragged() {
                    if let Some(pointer_pos) = response.interact_pointer_pos() {
                        if pointer_pos.x < response.rect.left() && index > 0 {
                            move_tab = Some((index, index - 1));
                        } else if pointer_pos.x > response.rect.right()
                            && index + 1 < self.terminal_tabs.len()
                        {
                            move_tab = Some((index, index + 1));
                        }
                    }
                }
            }

            ui.add_space(4.0);
            let plus_btn = Self::tab_plus_button(ui, palette);
            if plus_btn.clicked() {
                self.add_terminal_tab();
            }
            plus_btn.on_hover_text("New tab (Cmd+T)");

            let tab = &self.terminal_tabs[self.active_terminal];
            if tab.panes.len() > 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "pane {}/{}",
                            tab.active_pane + 1,
                            tab.panes.len()
                        ))
                        .small()
                        .color(palette.muted_text),
                    );
                });
            }
        });

        (switch_to, close_tab, rename_tab, move_tab)
    }

    /// Render all panes in an auto-grid layout with drag-to-swap
    fn render_panes(&mut self, ui: &mut egui::Ui, palette: ThemePalette, ctx: &egui::Context) {
        let tab_idx = self.active_terminal;
        let num_panes = self.terminal_tabs[tab_idx].panes.len();
        let active_pane_idx = self.terminal_tabs[tab_idx].active_pane;

        if num_panes == 1 {
            let pane_uid = self.terminal_tabs[tab_idx].panes[0].uid();
            let pane_id = ui.id().with(("pane_uid", pane_uid));
            {
                let pane = &mut self.terminal_tabs[tab_idx].panes[0];
                render_pane_dispatch(pane, ui, palette, ctx, pane_id, true);
            }
            let logs = self.terminal_tabs[tab_idx].panes[0]
                .as_terminal_mut()
                .map(|t| std::mem::take(&mut t.pending_logs))
                .unwrap_or_default();
            for msg in logs {
                self.log_debug(msg);
            }
            return;
        }

        let (cols, rows) = Self::grid_dims(num_panes);
        let total_width = ui.available_width();
        let total_height = ui.available_height();
        let gap = PANE_SEPARATOR_WIDTH;
        let pane_width = (total_width - gap * (cols as f32 - 1.0)) / cols as f32;
        let pane_height = (total_height - gap * (rows as f32 - 1.0)) / rows as f32;

        let mut pane_rects: Vec<egui::Rect> = Vec::with_capacity(num_panes);
        let origin = ui.cursor().min;

        for idx in 0..num_panes {
            let col = idx % cols;
            let row = idx / cols;
            let x = origin.x + col as f32 * (pane_width + gap);
            let y = origin.y + row as f32 * (pane_height + gap);
            pane_rects.push(egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(pane_width, pane_height),
            ));
        }

        let painter = ui.painter();
        for row in 0..rows {
            let panes_in_row = if (row + 1) * cols <= num_panes {
                cols
            } else {
                num_panes - row * cols
            };

            for col in 1..panes_in_row {
                let x = origin.x + col as f32 * (pane_width + gap) - gap;
                let y_top = origin.y + row as f32 * (pane_height + gap);
                let sep_rect =
                    egui::Rect::from_min_size(egui::pos2(x, y_top), egui::vec2(gap, pane_height));
                painter.rect_filled(sep_rect, egui::CornerRadius::ZERO, palette.border);
            }

            if row + 1 < rows {
                let y = origin.y + (row + 1) as f32 * (pane_height + gap) - gap;
                let sep_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, y),
                    egui::vec2(total_width, gap),
                );
                painter.rect_filled(sep_rect, egui::CornerRadius::ZERO, palette.border);
            }
        }

        const BAR_H: f32 = 24.0;

        let mut pending_focus: Option<usize> = None;
        let mut pending_swap: Option<(usize, usize)> = None;
        let mut pending_close: Option<usize> = None;
        let mut pending_rename_start: Option<(usize, String)> = None;
        let mut pending_rename_commit = false;
        let mut pending_rename_cancel = false;
        let mut create_ai_mirror_for: Option<u64> = None;

        for pane_idx in 0..num_panes {
            let full_rect = pane_rects[pane_idx];
            let is_active = pane_idx == active_pane_idx;
            let pane_uid = self.terminal_tabs[tab_idx].panes[pane_idx].uid();
            let pane_id = ui.id().with(("pane_uid", pane_uid));

            let bar_rect =
                egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), BAR_H));
            let content_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.min.x, full_rect.min.y + BAR_H),
                full_rect.max,
            );

            let bar_bg = if is_active {
                palette.surface
            } else {
                palette.bar_bg
            };
            ui.painter()
                .rect_filled(bar_rect, egui::CornerRadius::ZERO, bar_bg);

            let handle_rect = egui::Rect::from_min_size(
                egui::pos2(bar_rect.left(), bar_rect.top()),
                egui::vec2(28.0, BAR_H),
            );
            let handle_id = pane_id.with("bar_handle");
            let handle_resp = ui.interact(handle_rect, handle_id, egui::Sense::click_and_drag());
            let handle_color = if handle_resp.hovered() || handle_resp.dragged() {
                palette.accent
            } else {
                palette.muted_text.linear_multiply(0.5)
            };
            {
                let cx = handle_rect.center().x;
                let cy = handle_rect.center().y;
                let dx = 3.0_f32;
                let dy = 3.0_f32;
                let r = 1.2_f32;
                for row in [-1i32, 0, 1] {
                    for col in [-1i32, 1] {
                        ui.painter().circle_filled(
                            egui::pos2(cx + col as f32 * dx, cy + row as f32 * dy),
                            r,
                            handle_color,
                        );
                    }
                }
            }

            if handle_resp.clicked() {
                pending_focus = Some(pane_idx);
            }
            if handle_resp.drag_started() {
                let handle_center = handle_rect.center();
                ui.data_mut(|d| {
                    d.insert_temp(egui::Id::new("bar_drag_from"), pane_idx);
                    d.insert_temp(egui::Id::new("bar_drag_origin"), handle_center);
                });
            }
            if handle_resp.drag_stopped() {
                let from: Option<usize> =
                    ui.data(|d| d.get_temp(egui::Id::new("bar_drag_from")));
                if let Some(from_idx) = from {
                    if let Some(pos) = handle_resp.interact_pointer_pos() {
                        for (to_idx, to_rect) in pane_rects.iter().enumerate() {
                            if to_idx != from_idx && to_rect.contains(pos) {
                                pending_swap = Some((from_idx, to_idx));
                                break;
                            }
                        }
                    }
                }
                ui.data_mut(|d| {
                    d.remove_by_type::<usize>();
                    d.remove_by_type::<egui::Pos2>();
                });
            }

            let close_btn_size = egui::vec2(BAR_H, BAR_H);
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(bar_rect.right() - close_btn_size.x, bar_rect.top()),
                close_btn_size,
            );
            let close_id = pane_id.with("bar_close");
            let close_resp = ui.interact(close_rect, close_id, egui::Sense::click());
            let close_color = if close_resp.hovered() {
                egui::Color32::from_rgb(220, 80, 80)
            } else {
                palette.muted_text.linear_multiply(0.5)
            };
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(14.0),
                close_color,
            );
            if close_resp.clicked() {
                pending_close = Some(pane_idx);
            }

            let title_rect = egui::Rect::from_min_max(
                egui::pos2(bar_rect.left() + 28.0, bar_rect.top()),
                egui::pos2(bar_rect.right() - close_btn_size.x, bar_rect.bottom()),
            );
            let is_renaming = self.renaming_pane == Some((tab_idx, pane_idx));

            if is_renaming {
                let rename_id = pane_id.with("bar_rename_edit");
                let mut rename_ui =
                    ui.new_child(egui::UiBuilder::new().max_rect(title_rect.shrink(2.0)));
                let edit_resp = rename_ui.add(
                    egui::TextEdit::singleline(&mut self.pane_rename_buffer)
                        .id(rename_id)
                        .desired_width(title_rect.width() - 4.0)
                        .font(egui::TextStyle::Small)
                        .frame(false),
                );
                edit_resp.request_focus();
                let pressed_enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                let pressed_esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                if pressed_esc {
                    pending_rename_cancel = true;
                } else if pressed_enter {
                    pending_rename_commit = true;
                } else if edit_resp.lost_focus() {
                    pending_rename_commit = true;
                }
            } else {
                let current_title =
                    self.terminal_tabs[tab_idx].panes[pane_idx].title().to_owned();
                let display_title = if current_title.is_empty() {
                    format!("Terminal {}", pane_idx + 1)
                } else {
                    current_title.clone()
                };
                let title_color = if is_active {
                    palette.text
                } else {
                    palette.muted_text
                };
                let title_id = pane_id.with("bar_title");
                let title_resp = ui.interact(title_rect, title_id, egui::Sense::click());
                ui.painter().text(
                    title_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &display_title,
                    egui::FontId::proportional(11.0),
                    title_color,
                );
                if title_resp.double_clicked() {
                    pending_rename_start = Some((pane_idx, display_title));
                }
                if title_resp.clicked() {
                    pending_focus = Some(pane_idx);
                }
                // Context menu: "Mirror as AI Panel"
                let is_terminal_pane = self.terminal_tabs[tab_idx].panes[pane_idx]
                    .as_terminal()
                    .is_some();
                if is_terminal_pane {
                    title_resp.context_menu(|ui| {
                        if ui.button("Mirror as AI Panel").clicked() {
                            create_ai_mirror_for = Some(pane_uid);
                            ui.close();
                        }
                    });
                }
            }

            let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
            child_ui.set_clip_rect(content_rect);

            {
                let pane = &mut self.terminal_tabs[tab_idx].panes[pane_idx];
                render_pane_dispatch(pane, &mut child_ui, palette, ctx, pane_id, is_active);
            }
            let logs = self.terminal_tabs[tab_idx].panes[pane_idx]
                .as_terminal_mut()
                .map(|t| std::mem::take(&mut t.pending_logs))
                .unwrap_or_default();
            for msg in logs {
                self.log_debug(msg);
            }

            let pane_has_focus = self.terminal_tabs[tab_idx].panes[pane_idx].has_focus();
            let pane_uid_for_log = self.terminal_tabs[tab_idx].panes[pane_idx].uid();
            if pane_has_focus && !is_active {
                let old_active = self.terminal_tabs[tab_idx].active_pane;
                self.terminal_tabs[tab_idx].active_pane = pane_idx;
                self.log_debug(format!(
                    "focus_change: pane {old_active} -> {pane_idx} (uid={})",
                    pane_uid_for_log
                ));
            }
        }

        // Draw drag line
        {
            let from: Option<usize> = ui.data(|d| d.get_temp(egui::Id::new("bar_drag_from")));
            if from.is_some() {
                if let Some(origin_pos) =
                    ui.data(|d| d.get_temp::<egui::Pos2>(egui::Id::new("bar_drag_origin")))
                {
                    if let Some(ptr) = ctx.input(|i| i.pointer.hover_pos()) {
                        ui.painter().line_segment(
                            [origin_pos, ptr],
                            egui::Stroke::new(1.5, palette.accent.linear_multiply(0.55)),
                        );
                        ctx.request_repaint();
                    }
                }
            }
        }

        if let Some((pane_idx, current)) = pending_rename_start {
            self.renaming_pane = Some((tab_idx, pane_idx));
            self.pane_rename_buffer = current;
        }
        if pending_rename_commit {
            if let Some((t, p)) = self.renaming_pane {
                let new_title = self.pane_rename_buffer.trim().to_owned();
                if let Some(term) = self.terminal_tabs[t].panes[p].as_terminal_mut() {
                    term.title = new_title;
                }
            }
            self.renaming_pane = None;
        }
        if pending_rename_cancel {
            self.renaming_pane = None;
        }

        if let Some(pane_idx) = pending_focus {
            self.terminal_tabs[tab_idx].active_pane = pane_idx;
        }
        if let Some((from, to)) = pending_swap {
            let from_uid = self.terminal_tabs[tab_idx].panes[from].uid();
            let to_uid = self.terminal_tabs[tab_idx].panes[to].uid();
            self.terminal_tabs[tab_idx].panes.swap(from, to);
            let active = self.terminal_tabs[tab_idx].active_pane;
            if active == from {
                self.terminal_tabs[tab_idx].active_pane = to;
            } else if active == to {
                self.terminal_tabs[tab_idx].active_pane = from;
            }
            self.log_debug(format!(
                "bar_swap: {from}(uid={from_uid}) <-> {to}(uid={to_uid})"
            ));
        }
        if let Some(close_idx) = pending_close {
            let before = self.terminal_tabs[tab_idx].panes.len();
            self.terminal_tabs[tab_idx].close_pane(close_idx);
            self.log_debug(format!(
                "bar_close: pane {close_idx}, {before} -> {} panes",
                self.terminal_tabs[tab_idx].panes.len()
            ));
            if self.renaming_pane == Some((tab_idx, close_idx)) {
                self.renaming_pane = None;
            }
        }

        // Feature 2: Create AI mirror pane
        if let Some(source_uid) = create_ai_mirror_for {
            let (tx, rx) = std::sync::mpsc::sync_channel(256);
            for pane in &mut self.terminal_tabs[tab_idx].panes {
                if pane.uid() == source_uid {
                    if let Some(term) = pane.as_terminal_mut() {
                        term.mirror_tx = Some(tx);
                    }
                    break;
                }
            }
            let uid = self.alloc_pane_uid();
            let ai_pane = AiOutputPane {
                uid,
                title: format!("AI Output (pane {})", source_uid),
                lines: std::collections::VecDeque::new(),
                mirror_rx: rx,
                source_pane_uid: source_uid,
            };
            self.terminal_tabs[tab_idx].panes.push(Pane::AiOutput(ai_pane));
        }

        let grid_rect = egui::Rect::from_min_size(
            origin,
            egui::vec2(total_width, rows as f32 * (pane_height + gap) - gap),
        );
        ui.allocate_rect(grid_rect, egui::Sense::hover());
    }

    fn grid_dims(n: usize) -> (usize, usize) {
        match n {
            0 | 1 => (1, 1),
            2 => (2, 1),
            3 => (3, 1),
            4 => (2, 2),
            5 | 6 => (3, 2),
            7..=9 => (3, 3),
            10..=12 => (4, 3),
            _ => {
                let cols = (n as f32).sqrt().ceil() as usize;
                let rows = (n + cols - 1) / cols;
                (cols, rows)
            }
        }
    }
}

impl eframe::App for GhostStickiesApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.startup_tasks_run {
            self.startup_tasks_run = true;
        }

        self.apply_window_mode(ctx);

        // ── App-level paste detection ──
        {
            let nsevent_cmd_v = take_cmd_v_pressed();

            let all_events = ctx.input(|i| i.events.clone());
            for e in &all_events {
                if let egui::Event::Paste(t) = e {
                    self.log_debug(format!("app_paste: Event::Paste text.len()={}", t.len()));
                }
            }
            if nsevent_cmd_v {
                self.log_debug("app_paste: NSEvent Cmd+V detected".to_owned());
            }

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
                        if let Some(term) = self.terminal_tabs[ti].panes[pi].as_terminal_mut() {
                            term.paste_chip = Some(filename);
                            term.paste_text(&path_str);
                        }
                    } else {
                        self.log_debug(
                            "app_paste: save_clipboard_image returned None".to_owned(),
                        );
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
            input.modifiers.command
                && input.modifiers.shift
                && input.key_pressed(egui::Key::D)
        });
        let next_pane = ctx
            .input(|input| input.modifiers.command && input.key_pressed(egui::Key::CloseBracket));
        let prev_pane =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::OpenBracket));
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

        // Feature 3: Scratchpad toggle (Cmd+P)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) {
            self.scratchpad.open = !self.scratchpad.open;
        }

        // Feature 7: Command Palette toggle (Cmd+K)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.command_palette.open = !self.command_palette.open;
            self.command_palette.query.clear();
            self.command_palette.selected = 0;
        }

        let mut received_output = false;
        for tab in &mut self.terminal_tabs {
            tab.ensure_all_started();
            if tab.drain_all_output() {
                received_output = true;
            }
        }
        // Feature 4: drain file watcher
        if let Some(watcher) = &mut self.file_watcher {
            if watcher.drain() {
                received_output = true;
            }
        }
        if received_output {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(Duration::from_millis(16));

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

                ui.painter().text(
                    ui.max_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    "StickyTerminal",
                    egui::FontId::proportional(13.0),
                    palette.muted_text,
                );

                ui.horizontal(|ui| {
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
                    }

                    ui.add_space(12.0);

                    // Feature 8: New Browser Pane button
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("B").size(11.0).color(palette.muted_text),
                            )
                            .frame(false),
                        )
                        .on_hover_text("New Browser Pane")
                        .clicked()
                    {
                        let uid = self.alloc_pane_uid();
                        let ti = self.active_terminal;
                        self.terminal_tabs[ti]
                            .panes
                            .push(Pane::Browser(BrowserPane::new(uid)));
                    }

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
                                        egui::RichText::new(preset.label())
                                            .color(palette.accent)
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
                let (switch_to, close_tab, rename_tab, move_tab) =
                    self.render_tab_bar(ui, palette);

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
                                            let _ =
                                                egui::containers::menu::MenuButton::from_button(
                                                    Self::note_action_button("Recent", palette),
                                                )
                                                .ui(ui, |ui| {
                                                    for path in &recent_copy {
                                                        let name = path
                                                            .file_name()
                                                            .and_then(|n| n.to_str())
                                                            .unwrap_or("?");
                                                        if ui
                                                            .selectable_label(false, name)
                                                            .clicked()
                                                        {
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
                    let content_height =
                        (ui.available_height() - status_height - 8.0).max(100.0);

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
                                        .hint_text(
                                            "Write markdown here. Cmd+L to add a checkbox.",
                                        ),
                                    );
                                    if r.changed() {
                                        text_edit_changed = true;
                                    }
                                });
                        } else {
                            let preview_changed = Self::render_markdown_preview(
                                ui,
                                &mut self.terminal_tabs[ti].notes_markdown,
                                palette,
                                content_height,
                            );
                            if preview_changed {
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

                    // Feature 6: Checkpoints section
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new("Checkpoints")
                        .default_open(false)
                        .show(ui, |ui| {
                            if let Some(err) = &self.checkpoint_error.clone() {
                                ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
                            }
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.checkpoint_label)
                                        .hint_text("label...")
                                        .desired_width(ui.available_width() - 52.0),
                                );
                                if ui.button("Save").clicked() {
                                    self.create_checkpoint();
                                }
                            });
                            let to_restore = {
                                let mut r = None;
                                for (label, stash_ref) in &self.checkpoints.clone() {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(label)
                                                .small()
                                                .color(palette.text),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.small_button("Restore").clicked() {
                                                    r = Some(stash_ref.clone());
                                                }
                                            },
                                        );
                                    });
                                }
                                r
                            };
                            if let Some(stash_ref) = to_restore {
                                self.restore_checkpoint(stash_ref);
                            }
                        });

                    // Feature 4: File Changes section
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new("File Changes")
                        .default_open(true)
                        .show(ui, |ui| {
                            if self.file_watcher.is_none() {
                                if ui.button("Watch workspace...").clicked() {
                                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                        match FileWatcher::start(path) {
                                            Ok(w) => self.file_watcher = Some(w),
                                            Err(e) => self.last_error = Some(e.to_string()),
                                        }
                                    }
                                }
                                if let Some(err) = &self.last_error.clone() {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(255, 100, 100),
                                        err,
                                    );
                                }
                            } else if let Some(watcher) = &self.file_watcher {
                                let watched = watcher.watched_path.clone();
                                let changes: Vec<_> =
                                    watcher.recent_changes.iter().cloned().collect();
                                if changes.is_empty() {
                                    ui.label(
                                        egui::RichText::new("Watching for changes...")
                                            .color(palette.muted_text)
                                            .small(),
                                    );
                                }
                                for change in &changes {
                                    let (icon, color) = match change.kind {
                                        ChangeKind::Created => (
                                            "+",
                                            egui::Color32::from_rgb(100, 220, 100),
                                        ),
                                        ChangeKind::Modified => ("~", palette.accent),
                                        ChangeKind::Deleted => (
                                            "-",
                                            egui::Color32::from_rgb(220, 100, 100),
                                        ),
                                    };
                                    let rel = change
                                        .path
                                        .strip_prefix(&watched)
                                        .unwrap_or(&change.path);
                                    let name = rel.display().to_string();
                                    ui.colored_label(color, format!("{} {}", icon, name));
                                }
                                if ui.small_button("Stop watching").clicked() {
                                    self.file_watcher = None;
                                }
                            }
                        });

                    // Feature 5: Smart Context Snippets — drop files into sidebar
                    let any_pane_focused =
                        self.terminal_tabs[ti].panes.iter().any(|p| p.has_focus());
                    if !any_pane_focused {
                        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
                        for file in &dropped_files {
                            if let Some(path) = &file.path {
                                if let Ok(contents) = std::fs::read_to_string(path) {
                                    let ext = path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("");
                                    let snippet =
                                        format!("\n\n```{}\n{}\n```\n", ext, contents);
                                    self.terminal_tabs[ti]
                                        .notes_markdown
                                        .push_str(&snippet);
                                    self.terminal_tabs[ti].notes_dirty = true;
                                    self.terminal_tabs[ti].notes_render_cache = None;
                                    self.terminal_tabs[ti].last_type_time =
                                        Some(std::time::Instant::now());
                                }
                            }
                        }
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

        // Feature 3: Scratchpad window
        self.render_scratchpad(ctx, palette);

        // Feature 7: Command Palette window
        self.render_command_palette(ctx, palette);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.palette().bg.to_normalized_gamma_f32()
    }
}
