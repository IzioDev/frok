use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use frok_common::paths::data_path;

use crate::ui::theme::{Texture, ThemeKind};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UiConfig {
    theme: Option<String>,
    texture: Option<String>,
    quickstart_seen: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AppConfig {
    #[serde(default)]
    ui: UiConfig,
}

pub struct UiPrefs {
    pub theme: ThemeKind,
    pub texture: Texture,
    pub quickstart_seen: bool,
}

pub fn load_ui_prefs() -> UiPrefs {
    let config = load_config();
    let theme = config
        .ui
        .theme
        .as_deref()
        .and_then(ThemeKind::parse)
        .unwrap_or(ThemeKind::NeonSunset);
    let texture = config
        .ui
        .texture
        .as_deref()
        .and_then(Texture::parse)
        .unwrap_or(Texture::None);
    let quickstart_seen = config.ui.quickstart_seen.unwrap_or(false);
    UiPrefs {
        theme,
        texture,
        quickstart_seen,
    }
}

pub fn save_ui_prefs(theme: ThemeKind, texture: Texture) -> Result<()> {
    let mut config = load_config();
    config.ui.theme = Some(theme.as_str().to_string());
    config.ui.texture = Some(texture.as_str().to_string());
    save_config(&config)
}

pub fn save_quickstart_seen(seen: bool) -> Result<()> {
    let mut config = load_config();
    config.ui.quickstart_seen = Some(seen);
    save_config(&config)
}

fn config_path() -> PathBuf {
    data_path("config.toml")
}

fn load_config() -> AppConfig {
    let path = config_path();
    let Ok(contents) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    match toml::from_str::<AppConfig>(&contents) {
        Ok(config) => config,
        Err(err) => {
            warn!("config parse failed: {err}");
            AppConfig::default()
        }
    }
}

fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(config)?;
    fs::write(path, contents)?;
    Ok(())
}
