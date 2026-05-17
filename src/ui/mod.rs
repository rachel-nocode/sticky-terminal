pub(crate) mod pane;

use eframe::egui;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::AppConfig;
use crate::sticky::{self, MenuAction, PaperColor};
use crate::terminal::clipboard::{
    install_paste_monitor, read_clipboard, save_clipboard_image, take_cmd_v_pressed,
};
use crate::terminal::{default_terminal_cwd, TerminalPane};

/// Window inner height when collapsed to just the header.
const MINIMIZED_H: f32 = 68.0;
/// Window inner height needed to show the dropdown while minimized.
const MENU_FIT_H: f32 = 188.0;

/// StickyTerminal — the whole app window is a single paper sticky note holding
/// one terminal. Tiny, borderless, always-on-top: stick it anywhere on screen.
pub(crate) struct GhostStickiesApp {
    paper: PaperColor,
    privacy_mode: bool,
    applied_privacy_mode: Option<bool>,
    minimized: bool,
    menu_open: bool,
    /// True once the OS window shadow has been switched off.
    shadow_disabled: bool,
    /// Window inner size to restore to when un-minimizing.
    expanded_size: egui::Vec2,
    terminal: TerminalPane,
}

impl GhostStickiesApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::install_fonts(&cc.egui_ctx);
        install_paste_monitor();

        Self {
            paper: Self::load_paper(),
            privacy_mode: false,
            applied_privacy_mode: None,
            minimized: false,
            menu_open: false,
            shadow_disabled: false,
            expanded_size: egui::vec2(340.0, 380.0),
            terminal: TerminalPane::new(default_terminal_cwd()),
        }
    }

    /// JetBrains Mono for the terminal, Inter for UI text.
    fn install_fonts(ctx: &egui::Context) {
        use std::sync::Arc;
        let mut fonts = egui::FontDefinitions::default();

        fonts.font_data.insert(
            "JetBrainsMono".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../../assets/fonts/JetBrainsMono-Regular.ttf"
            ))),
        );
        fonts.font_data.insert(
            "Inter".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../../assets/fonts/Inter-Variable.ttf"
            ))),
        );

        if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            mono.insert(0, "JetBrainsMono".to_owned());
        }
        if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            prop.insert(0, "Inter".to_owned());
        }
        ctx.set_fonts(fonts);
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

    fn load_paper() -> PaperColor {
        std::fs::read_to_string(Self::config_path())
            .ok()
            .and_then(|s| serde_json::from_str::<AppConfig>(&s).ok())
            .map(|c| c.paper)
            .unwrap_or_default()
    }

    fn save_config(&self) {
        let _ = std::fs::create_dir_all(Self::app_support_dir());
        if let Ok(s) = serde_json::to_string_pretty(&AppConfig { paper: self.paper }) {
            let _ = std::fs::write(Self::config_path(), s);
        }
    }

    /// Hide the sticky from screen recordings / shares when privacy is on.
    #[cfg(target_os = "macos")]
    fn apply_macos_share_privacy(&self, enabled: bool) {
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
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

    /// Kill the OS window shadow — it hugs the card and reads as a dark ring.
    /// The sticky paints its own soft shadow instead.
    #[cfg(target_os = "macos")]
    fn disable_os_window_shadow(&self) {
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
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
                let _: () = msg_send![window, setHasShadow: false];
                let _: () = msg_send![window, invalidateShadow];
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn disable_os_window_shadow(&self) {}

    fn apply_privacy(&mut self) {
        if self.applied_privacy_mode == Some(self.privacy_mode) {
            return;
        }
        self.apply_macos_share_privacy(self.privacy_mode);
        self.applied_privacy_mode = Some(self.privacy_mode);
    }

    /// Collapse to the header bar / expand back to the saved size.
    fn toggle_minimize(&mut self, ctx: &egui::Context) {
        self.minimized = !self.minimized;
        if !self.minimized {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.expanded_size));
        }
        // Minimized resizing is handled each frame by `sync_window`.
    }

    /// Keep the window height in step with the minimized / menu state.
    fn sync_window(&self, ctx: &egui::Context) {
        if !self.minimized {
            return;
        }
        let target_h = if self.menu_open {
            MENU_FIT_H
        } else {
            MINIMIZED_H
        };
        let cur = ctx.content_rect().size();
        if (cur.y - target_h).abs() > 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                cur.x, target_h,
            )));
        }
    }

    /// Cmd+V image paste — drop the image to a temp file, paste its path.
    fn handle_image_paste(&mut self) {
        if !take_cmd_v_pressed() {
            return;
        }
        if read_clipboard().map(|t| !t.is_empty()).unwrap_or(false) {
            return; // plain text — the terminal handles it itself
        }
        let mut logs: Vec<String> = Vec::new();
        if let Some(img_path) = save_clipboard_image(&mut logs) {
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
            self.terminal.paste_chip = Some(filename);
            self.terminal.paste_text(&path_str);
        }
    }
}

impl eframe::App for GhostStickiesApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent — only the painted sticky card is opaque; the desktop
        // shows through around it (and under the drop shadow).
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_privacy();
        if !self.shadow_disabled {
            self.disable_os_window_shadow();
            self.shadow_disabled = true;
        }
        self.handle_image_paste();

        // Privacy toggle (Cmd+Shift+P), shuffle color (Cmd+Shift+T).
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P)
        }) {
            self.privacy_mode = !self.privacy_mode;
        }
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::T)
        }) {
            self.paper = self.paper.random_other();
            self.save_config();
        }

        if !self.minimized {
            self.expanded_size = ctx.content_rect().size();
        }
        self.sync_window(ctx);

        // Pump the terminal.
        self.terminal.ensure_started();
        if self.terminal.drain_output() {
            ctx.request_repaint();
        }
        self.terminal.pending_logs.clear();
        ctx.request_repaint_after(Duration::from_millis(16));

        let palette = self.paper.terminal_palette();
        let colors = self.paper.colors();
        let mut style = (*ctx.style()).clone();
        style.visuals.override_text_color = Some(palette.text);
        style.visuals.selection.bg_fill = palette.selection;
        ctx.set_style(style);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
            .show(ctx, |ui| {
                let window_rect = ui.max_rect();
                let frame = sticky::paint(ui, window_rect, colors, self.minimized);

                if frame.close_clicked {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if frame.peel_resize {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(
                        egui::ResizeDirection::SouthEast,
                    ));
                }
                if frame.menu_clicked {
                    self.menu_open = !self.menu_open;
                }

                // The header band is the move handle.
                let drag = ui.interact(
                    frame.drag,
                    ui.id().with("sticky_drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.dragged() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // The terminal fills the rest of the card (hidden when minimized).
                if !self.minimized {
                    let mut content =
                        ui.new_child(egui::UiBuilder::new().max_rect(frame.content));
                    content.set_clip_rect(frame.content);
                    pane::render_pane(
                        &mut self.terminal,
                        &mut content,
                        palette,
                        ctx,
                        ui.id().with("sticky_terminal"),
                        true,
                    );
                }

                // The dropdown menu paints on top of everything.
                if self.menu_open {
                    let (action, menu_rect) = sticky::paint_menu(
                        ui,
                        frame.menu_anchor,
                        colors,
                        self.privacy_mode,
                        self.minimized,
                    );
                    let mut close_menu = false;
                    if let Some(action) = action {
                        match action {
                            MenuAction::Randomize => {
                                self.paper = self.paper.random_other();
                                self.save_config();
                            }
                            MenuAction::ToggleVisibility => {
                                self.privacy_mode = !self.privacy_mode;
                            }
                            MenuAction::ToggleMinimize => {
                                self.toggle_minimize(ctx);
                            }
                        }
                        close_menu = true;
                    }
                    // Click anywhere outside the menu or its chevron dismisses it.
                    if !frame.menu_clicked && ctx.input(|i| i.pointer.any_click()) {
                        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                            if !menu_rect.contains(pos) && !frame.menu_anchor.contains(pos) {
                                close_menu = true;
                            }
                        }
                    }
                    if close_menu {
                        self.menu_open = false;
                    }
                }
            });
    }
}
