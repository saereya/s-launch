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
    pub power: bool,
    /// Order determines result priority: first entry appears before later ones.
    pub priority: Vec<String>,
    /// Terminal emulator to use for Terminal=true apps. None = auto-detect.
    pub terminal: Option<String>,
    /// Shell command run for each power plugin action. An empty string omits
    /// that action from results entirely, so individual actions (e.g. "lock"
    /// on a setup with no lock daemon) can be disabled without touching `power`.
    pub power_commands: PowerCommands,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PowerCommands {
    pub shutdown: String,
    pub reboot: String,
    pub suspend: String,
    pub lock: String,
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
            power: true,
            priority: vec!["apps".into(), "commands".into(), "power".into()],
            terminal: None,
            power_commands: PowerCommands::default(),
        }
    }
}

impl Default for PowerCommands {
    fn default() -> Self {
        Self {
            // systemd/logind defaults, since that's what most distros run;
            // override per-action in config.toml on other init systems.
            shutdown: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            suspend: "systemctl suspend".into(),
            lock: "loginctl lock-session".into(),
        }
    }
}

// ── Loading ──────────────────────────────────────────────────────────────────

pub fn config_path() -> PathBuf {
    xdg::BaseDirectories::with_prefix("slaunch")
        .map(|b| b.get_config_file("config.toml"))
        .unwrap_or_else(|_| fallback_config_dir().join("config.toml"))
}

pub fn style_path() -> PathBuf {
    xdg::BaseDirectories::with_prefix("slaunch")
        .map(|b| b.get_config_file("style.css"))
        .unwrap_or_else(|_| fallback_config_dir().join("style.css"))
}

fn fallback_config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config/slaunch")
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
            eprintln!("slaunch: config parse error in {}: {e}", path.display());
            tracing::error!("Config parse error: {e}");
            Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_documented_values() {
        let cfg = Config::default();
        assert_eq!(cfg.window.width, 640);
        assert_eq!(cfg.window.max_results, 12);
        assert_eq!(cfg.window.monitor, "focused");
        assert_eq!(cfg.window.anchor, "center");
        assert_eq!(cfg.window.margin, 60);
        assert_eq!(cfg.window.item_height, 40);
        assert_eq!(cfg.window.padding, 10);
        assert_eq!(cfg.input.placeholder, "Search...");
        assert!(cfg.plugins.apps);
        assert!(cfg.plugins.commands);
        assert!(cfg.plugins.power);
        assert_eq!(cfg.plugins.priority, vec!["apps", "commands", "power"]);
        assert_eq!(cfg.plugins.terminal, None);
        assert_eq!(cfg.plugins.power_commands.shutdown, "systemctl poweroff");
        assert_eq!(cfg.plugins.power_commands.reboot, "systemctl reboot");
        assert_eq!(cfg.plugins.power_commands.suspend, "systemctl suspend");
        assert_eq!(cfg.plugins.power_commands.lock, "loginctl lock-session");
    }

    #[test]
    fn empty_toml_falls_back_to_all_defaults() {
        let cfg: Config = toml::from_str("").expect("empty document is valid");
        assert_eq!(cfg.window.width, Config::default().window.width);
        assert_eq!(cfg.plugins.priority, Config::default().plugins.priority);
    }

    #[test]
    fn partial_toml_overrides_only_specified_fields() {
        let raw = r#"
            [window]
            width = 800
            anchor = "bottom"
        "#;
        let cfg: Config = toml::from_str(raw).expect("valid partial config");
        assert_eq!(cfg.window.width, 800);
        assert_eq!(cfg.window.anchor, "bottom");
        // Untouched fields keep their defaults.
        assert_eq!(cfg.window.max_results, 12);
        assert_eq!(cfg.window.margin, 60);
        assert!(cfg.plugins.apps);
    }

    #[test]
    fn full_toml_overrides_every_field() {
        let raw = r#"
            [window]
            width = 500
            max_results = 5
            monitor = "eDP-1"
            anchor = "top"
            margin = 20
            item_height = 30
            padding = 4

            [input]
            placeholder = "Type..."

            [plugins]
            apps = false
            commands = false
            power = false
            priority = ["commands", "apps"]
            terminal = "foot"

            [plugins.power_commands]
            shutdown = "doas poweroff"
            reboot = "doas reboot"
            suspend = ""
            lock = "swaylock"
        "#;
        let cfg: Config = toml::from_str(raw).expect("valid full config");
        assert_eq!(cfg.window.width, 500);
        assert_eq!(cfg.window.max_results, 5);
        assert_eq!(cfg.window.monitor, "eDP-1");
        assert_eq!(cfg.window.anchor, "top");
        assert_eq!(cfg.window.margin, 20);
        assert_eq!(cfg.window.item_height, 30);
        assert_eq!(cfg.window.padding, 4);
        assert_eq!(cfg.input.placeholder, "Type...");
        assert!(!cfg.plugins.apps);
        assert!(!cfg.plugins.commands);
        assert!(!cfg.plugins.power);
        assert_eq!(cfg.plugins.priority, vec!["commands", "apps"]);
        assert_eq!(cfg.plugins.terminal.as_deref(), Some("foot"));
        assert_eq!(cfg.plugins.power_commands.shutdown, "doas poweroff");
        assert_eq!(cfg.plugins.power_commands.reboot, "doas reboot");
        assert_eq!(cfg.plugins.power_commands.suspend, "");
        assert_eq!(cfg.plugins.power_commands.lock, "swaylock");
    }

    #[test]
    fn malformed_toml_is_rejected_by_the_parser() {
        // load() falls back to Config::default() on this Err; we assert the
        // parser actually rejects it, since that's the branch load() depends on.
        let raw = "window = not valid toml {{{";
        assert!(toml::from_str::<Config>(raw).is_err());
    }

    #[test]
    fn wrong_field_type_is_rejected_by_the_parser() {
        let raw = r#"
            [window]
            width = "not a number"
        "#;
        assert!(toml::from_str::<Config>(raw).is_err());
    }

    #[test]
    fn load_returns_default_when_config_file_absent() {
        let _guard = crate::test_env::lock();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _xdg = unsafe {
            crate::test_env::EnvVarGuard::set(
                "XDG_CONFIG_HOME",
                tmp.path().to_str().expect("utf8 path"),
            )
        };
        assert!(!config_path().exists());
        let cfg = load();
        assert_eq!(cfg.window.width, Config::default().window.width);
    }

    #[test]
    fn load_reads_and_parses_an_existing_config_file() {
        let _guard = crate::test_env::lock();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _xdg = unsafe {
            crate::test_env::EnvVarGuard::set(
                "XDG_CONFIG_HOME",
                tmp.path().to_str().expect("utf8 path"),
            )
        };
        let dir = config_path().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(config_path(), "[window]\nwidth = 999\n").unwrap();

        let cfg = load();
        assert_eq!(cfg.window.width, 999);
    }

    #[test]
    fn load_falls_back_to_default_on_parse_error() {
        let _guard = crate::test_env::lock();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _xdg = unsafe {
            crate::test_env::EnvVarGuard::set(
                "XDG_CONFIG_HOME",
                tmp.path().to_str().expect("utf8 path"),
            )
        };
        let dir = config_path().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(config_path(), "not valid toml {{{").unwrap();

        let cfg = load();
        assert_eq!(cfg.window.width, Config::default().window.width);
    }
}
