use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::theme::ThemePreset;

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct AppConfig {
    pub(crate) notes_root: Option<PathBuf>,
    pub(crate) current_note_file: Option<PathBuf>,
    pub(crate) theme: ThemePreset,
    #[serde(default)]
    pub(crate) recent_notes: Vec<PathBuf>,
}
