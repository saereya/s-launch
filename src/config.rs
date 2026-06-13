use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub window: WindowConfig,
    pub input: InputConfig,
    pub plugins: PluginsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Launcher width in pixels
    pub width: u32,
    /// Maximum results to show at once
    pub max_results: usize,
    /// Which monitor to appear on: "focused" | "primary" | output name
    pub monitor: String,
    /// Anchor position: "top" | "bottom" | "center"
    pub anchor: String,
    /// Pixels from the anchored edge
    pub margin: u32,
    /// Height of each result row in pixels
    pub item_height: u16,
    /// Padding around the launcher content in pixels
    pub padding: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct InputConfig {
    pub placeholder: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PluginsConfig {
    pub apps: bool,
    pub commands: bool,
    /// Order determines result priority: first entry appears before later ones.
    pub priority: Vec<String>,
}

// ── Defaults ────────────────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            window: Default::default(),
            input: Default::default(),
            plugins: Default::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 640,
            max_results: 12,
            monitor: "focused".into(),
            anchor: "center".into(),
            margin: 60,
            item_height: 40,
            padding: 10,
        }
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            placeholder: "Search...".into(),
        }
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            apps: true,
            commands: true,
            priority: vec!["apps".into(), "commands".into()],
        }
    }
}

// ── Loading ──────────────────────────────────────────────────────────────────

pub fn config_path() -> PathBuf {
    let base = xdg::BaseDirectories::with_prefix("slaunch")
        .expect("xdg basedirs");
    base.get_config_file("config.toml")
}

pub fn style_path() -> PathBuf {
    let base = xdg::BaseDirectories::with_prefix("slaunch")
        .expect("xdg basedirs");
    base.get_config_file("style.css")
}

pub fn load() -> Config {
    let path = config_path();
    if !path.exists() {
        return Config::default();
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Could not read config at {}: {e}", path.display());
            return Config::default();
        }
    };
    match toml::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Config parse error: {e}");
            Config::default()
        }
    }
}
