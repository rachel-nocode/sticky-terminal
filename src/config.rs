use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::theme::ThemePreset;

pub(crate) const CONFIG_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct AppConfig {
    pub(crate) notes_root: Option<PathBuf>,
    pub(crate) current_note_file: Option<PathBuf>,
    pub(crate) theme: ThemePreset,
    #[serde(default)]
    pub(crate) recent_notes: Vec<PathBuf>,
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) sidebar_open: bool,
}

pub(crate) fn migrate_config(mut config: AppConfig) -> AppConfig {
    // v0 -> v1: no structural changes, just stamp version
    config.version = CONFIG_VERSION;
    config
}

pub(crate) fn app_support_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("StickyTerminal")
    } else {
        PathBuf::from(".stickyterminal")
    }
}

pub(crate) fn config_path() -> PathBuf {
    app_support_dir().join("config.json")
}

pub(crate) fn save_config_inner(config: &AppConfig) -> anyhow::Result<()> {
    let support_dir = app_support_dir();
    fs::create_dir_all(&support_dir).context("Could not create app settings folder")?;

    let contents = serde_json::to_string_pretty(config).context("Could not encode settings")?;

    fs::write(config_path(), contents).context("Could not save settings")?;

    Ok(())
}
