use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub window: WindowConfig,
    pub input: InputConfig,
    pub plugins: PluginsConfig,
    pub style: StyleConfig,
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
}

/// Top-level style block — all sub-blocks are optional overrides.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct StyleConfig {
    pub background: String,
    pub foreground: String,
    pub font_family: String,
    pub font_size: f32,
    pub border_radius: f32,
    pub border_width: f32,
    pub border_color: String,
    pub padding: u16,
    pub item_height: u16,
    pub input: InputStyleConfig,
    pub item: ItemStyleConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct InputStyleConfig {
    pub background: Option<String>,
    pub foreground: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ItemStyleConfig {
    pub background: Option<String>,
    pub foreground: Option<String>,
    /// [style.item.selected] overrides
    pub selected: SelectedStyleConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SelectedStyleConfig {
    pub background: Option<String>,
    pub foreground: Option<String>,
}

// ── Defaults ────────────────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            window: Default::default(),
            input: Default::default(),
            plugins: Default::default(),
            style: Default::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 640,
            max_results: 12,
            monitor: "focused".into(),
            anchor: "top".into(),
            margin: 60,
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
        }
    }
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            background: "#1e1e2e".into(),
            foreground: "#cdd6f4".into(),
            font_family: "monospace".into(),
            font_size: 14.0,
            border_radius: 10.0,
            border_width: 1.0,
            border_color: "#313244".into(),
            padding: 10,
            item_height: 40,
            input: Default::default(),
            item: Default::default(),
        }
    }
}

impl Default for InputStyleConfig {
    fn default() -> Self {
        Self {
            background: None,
            foreground: None,
        }
    }
}

impl Default for ItemStyleConfig {
    fn default() -> Self {
        Self {
            background: None,
            foreground: None,
            selected: Default::default(),
        }
    }
}

impl Default for SelectedStyleConfig {
    fn default() -> Self {
        Self {
            background: Some("#313244".into()),
            foreground: None,
        }
    }
}

// ── Loading ──────────────────────────────────────────────────────────────────

pub fn config_path() -> PathBuf {
    let base = xdg::BaseDirectories::with_prefix("slaunch")
        .expect("xdg basedirs");
    base.get_config_file("config.toml")
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

/// Parse a hex color string like "#rrggbb" or "#rrggbbaa" into [u8; 4] RGBA.
pub fn parse_color(hex: &str) -> [u8; 4] {
    let s = hex.trim_start_matches('#');
    let parse2 = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
    match s.len() {
        6 => [parse2(0), parse2(2), parse2(4), 255],
        8 => [parse2(0), parse2(2), parse2(4), parse2(6)],
        _ => [0, 0, 0, 255],
    }
}

pub fn parse_color_iced(hex: &str) -> iced::Color {
    let [r, g, b, a] = parse_color(hex);
    iced::Color::from_rgba8(r, g, b, a as f32 / 255.0)
}
