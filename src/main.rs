#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub(crate) mod config;
pub(crate) mod notes;
pub(crate) mod terminal;
pub(crate) mod theme;
pub(crate) mod ui;

use eframe::egui;
use ui::GhostStickiesApp;

pub(crate) const WINDOW_WIDTH: f32 = 1180.0;
pub(crate) const WINDOW_HEIGHT: f32 = 760.0;
pub(crate) const TOP_BAR_HEIGHT: f32 = 40.0;
pub(crate) const TAB_BAR_HEIGHT: f32 = 38.0;
pub(crate) const MINIMIZED_HEIGHT: f32 = 40.0;
pub(crate) const SIDEBAR_DEFAULT_WIDTH: f32 = 340.0;
pub(crate) const TERMINAL_SCROLLBACK: usize = 5_000;
pub(crate) const PANE_SEPARATOR_WIDTH: f32 = 1.0;

fn main() -> Result<(), eframe::Error> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-1.png"))
        .expect("app icon should decode");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_title("StickyTerminal")
            .with_transparent(true)
            .with_decorations(false)
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "StickyTerminal",
        options,
        Box::new(|cc| Ok(Box::new(GhostStickiesApp::new(cc)))),
    )
}
