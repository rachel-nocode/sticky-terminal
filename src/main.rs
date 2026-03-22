#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

mod config;
mod notes;
mod theme;
mod watcher;
mod terminal;
mod ui;

fn main() -> Result<(), eframe::Error> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-1.png"))
        .expect("app icon should decode");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_title("StickyTerminal")
            .with_transparent(true)
            .with_decorations(false)
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "StickyTerminal",
        options,
        Box::new(|cc| Ok(Box::new(ui::GhostStickiesApp::new(cc)))),
    )
}
