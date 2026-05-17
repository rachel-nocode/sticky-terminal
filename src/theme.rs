use eframe::egui;

/// Colors the terminal surface is painted with. Derived from the active
/// `PaperColor` so the terminal reads like ink on the sticky's paper.
#[derive(Clone, Copy)]
pub(crate) struct ThemePalette {
    pub(crate) terminal_bg: egui::Color32,
    pub(crate) text: egui::Color32,
    pub(crate) muted_text: egui::Color32,
    pub(crate) selection: egui::Color32,
    pub(crate) accent: egui::Color32,
    pub(crate) surface: egui::Color32,
}
