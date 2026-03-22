use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct ThemePalette {
    pub(crate) bg: egui::Color32,
    pub(crate) bar_bg: egui::Color32,
    pub(crate) border: egui::Color32,
    pub(crate) text: egui::Color32,
    pub(crate) muted_text: egui::Color32,
    pub(crate) selection: egui::Color32,
    pub(crate) terminal_bg: egui::Color32,
    pub(crate) sidebar_bg: egui::Color32,
    pub(crate) sidebar_soft_bg: egui::Color32,
    pub(crate) accent: egui::Color32,
    pub(crate) accent_dim: egui::Color32,
    pub(crate) tab_bg: egui::Color32,
    pub(crate) active_tab_bg: egui::Color32,
    pub(crate) tab_text: egui::Color32,
    pub(crate) active_tab_text: egui::Color32,
    pub(crate) input_bg: egui::Color32,
    pub(crate) surface: egui::Color32,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub(crate) enum ThemePreset {
    Warp,
    WarpLight,
    Terminal,
    Midnight,
}

impl Default for ThemePreset {
    fn default() -> Self {
        Self::Warp
    }
}

impl ThemePreset {
    pub(crate) const ALL: [Self; 4] = [Self::Warp, Self::WarpLight, Self::Terminal, Self::Midnight];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Warp => "Warp Dark",
            Self::WarpLight => "Warp Blue",
            Self::Terminal => "Terminal",
            Self::Midnight => "Midnight",
        }
    }

    pub(crate) fn palette(self) -> ThemePalette {
        match self {
            Self::Warp => ThemePalette {
                bg: egui::Color32::from_rgb(0, 0, 0),
                terminal_bg: egui::Color32::from_rgb(0, 0, 0),
                sidebar_bg: egui::Color32::from_rgb(6, 6, 8),
                sidebar_soft_bg: egui::Color32::from_rgb(10, 10, 12),
                bar_bg: egui::Color32::from_rgb(0, 0, 0),
                border: egui::Color32::from_rgba_premultiplied(255, 255, 255, 18),
                text: egui::Color32::from_rgb(250, 250, 252),
                muted_text: egui::Color32::from_rgb(170, 170, 178),
                selection: egui::Color32::from_rgba_premultiplied(74, 98, 176, 132),
                accent: egui::Color32::from_rgb(98, 224, 192),
                accent_dim: egui::Color32::from_rgb(60, 140, 118),
                tab_bg: egui::Color32::from_rgb(12, 12, 14),
                active_tab_bg: egui::Color32::from_rgb(20, 20, 24),
                tab_text: egui::Color32::from_rgb(180, 180, 188),
                active_tab_text: egui::Color32::from_rgb(250, 250, 252),
                input_bg: egui::Color32::from_rgb(8, 8, 10),
                surface: egui::Color32::from_rgb(14, 14, 18),
            },
            Self::WarpLight => ThemePalette {
                bg: egui::Color32::from_rgb(14, 24, 42),
                terminal_bg: egui::Color32::from_rgb(12, 20, 36),
                sidebar_bg: egui::Color32::from_rgb(18, 30, 52),
                sidebar_soft_bg: egui::Color32::from_rgb(24, 38, 64),
                bar_bg: egui::Color32::from_rgb(20, 32, 54),
                border: egui::Color32::from_rgba_premultiplied(120, 175, 255, 18),
                text: egui::Color32::from_rgb(215, 234, 255),
                muted_text: egui::Color32::from_rgb(100, 140, 190),
                selection: egui::Color32::from_rgba_premultiplied(66, 110, 185, 148),
                accent: egui::Color32::from_rgb(100, 200, 255),
                accent_dim: egui::Color32::from_rgb(60, 130, 180),
                tab_bg: egui::Color32::from_rgb(24, 38, 60),
                active_tab_bg: egui::Color32::from_rgb(36, 52, 80),
                tab_text: egui::Color32::from_rgb(120, 160, 210),
                active_tab_text: egui::Color32::from_rgb(215, 234, 255),
                input_bg: egui::Color32::from_rgb(16, 26, 46),
                surface: egui::Color32::from_rgb(26, 40, 66),
            },
            Self::Terminal => ThemePalette {
                bg: egui::Color32::from_rgb(9, 13, 10),
                terminal_bg: egui::Color32::from_rgb(10, 13, 11),
                sidebar_bg: egui::Color32::from_rgb(16, 22, 18),
                sidebar_soft_bg: egui::Color32::from_rgb(20, 28, 23),
                bar_bg: egui::Color32::from_rgb(17, 22, 18),
                border: egui::Color32::from_rgba_premultiplied(90, 220, 150, 20),
                text: egui::Color32::from_rgb(168, 255, 196),
                muted_text: egui::Color32::from_rgb(80, 130, 96),
                selection: egui::Color32::from_rgba_premultiplied(44, 104, 70, 150),
                accent: egui::Color32::from_rgb(90, 220, 150),
                accent_dim: egui::Color32::from_rgb(50, 140, 90),
                tab_bg: egui::Color32::from_rgb(14, 20, 16),
                active_tab_bg: egui::Color32::from_rgb(24, 36, 28),
                tab_text: egui::Color32::from_rgb(80, 130, 96),
                active_tab_text: egui::Color32::from_rgb(168, 255, 196),
                input_bg: egui::Color32::from_rgb(12, 16, 13),
                surface: egui::Color32::from_rgb(22, 30, 24),
            },
            Self::Midnight => ThemePalette {
                bg: egui::Color32::from_rgb(12, 12, 16),
                terminal_bg: egui::Color32::from_rgb(8, 8, 12),
                sidebar_bg: egui::Color32::from_rgb(16, 16, 22),
                sidebar_soft_bg: egui::Color32::from_rgb(22, 22, 30),
                bar_bg: egui::Color32::from_rgb(18, 18, 24),
                border: egui::Color32::from_rgba_premultiplied(255, 255, 255, 6),
                text: egui::Color32::from_rgb(220, 220, 228),
                muted_text: egui::Color32::from_rgb(90, 90, 108),
                selection: egui::Color32::from_rgba_premultiplied(80, 60, 140, 140),
                accent: egui::Color32::from_rgb(200, 140, 255),
                accent_dim: egui::Color32::from_rgb(130, 90, 180),
                tab_bg: egui::Color32::from_rgb(20, 20, 28),
                active_tab_bg: egui::Color32::from_rgb(34, 34, 46),
                tab_text: egui::Color32::from_rgb(100, 100, 118),
                active_tab_text: egui::Color32::from_rgb(220, 220, 228),
                input_bg: egui::Color32::from_rgb(14, 14, 20),
                surface: egui::Color32::from_rgb(24, 24, 34),
            },
        }
    }
}
