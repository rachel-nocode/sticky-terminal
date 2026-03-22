use std::path::PathBuf;
use crate::theme::ThemePreset;

pub(crate) struct CommandPaletteState {
    pub(crate) open: bool,
    pub(crate) query: String,
    pub(crate) selected: usize,
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
        }
    }
}

#[derive(Clone)]
pub(crate) enum PaletteAction {
    SwitchTab(usize, String),
    OpenNote(PathBuf),
    NewTab,
    SplitPane,
    ToggleSidebar,
    SetTheme(ThemePreset),
    TogglePrivacy,
}

impl PaletteAction {
    pub(crate) fn label(&self) -> String {
        match self {
            PaletteAction::SwitchTab(_, name) => format!("Tab: {}", name),
            PaletteAction::OpenNote(p) => format!(
                "Note: {}",
                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ),
            PaletteAction::NewTab => "New Tab  Cmd+T".to_owned(),
            PaletteAction::SplitPane => "Split Pane  Cmd+D".to_owned(),
            PaletteAction::ToggleSidebar => "Toggle Sidebar".to_owned(),
            PaletteAction::SetTheme(t) => format!("Theme: {:?}", t),
            PaletteAction::TogglePrivacy => "Toggle Privacy Mode".to_owned(),
        }
    }
}
