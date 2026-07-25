pub mod apps;
pub mod commands;
pub mod emoji;
pub mod math;
pub mod power;

const FALLBACK_TERMINALS: &[&str] = &["foot", "alacritty", "kitty", "wezterm", "xterm"];

pub(super) fn launch_in_terminal(cmd: &str, terminal: Option<&str>) {
    if shell_words::split(cmd).map_or(true, |a| a.is_empty()) {
        tracing::error!("Empty or unparseable exec string: '{cmd}'");
        return;
    }
    // Wrap in a shell so the terminal stays open after the command exits.
    let shell_cmd = format!("{cmd}; exec $SHELL");
    let spawn = |term: &str| {
        std::process::Command::new(term)
            .args(["-e", "sh", "-c", &shell_cmd])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    };
    if let Some(term) = terminal {
        if let Err(e) = spawn(term) {
            tracing::error!("Failed to launch terminal '{term}': {e}");
        }
        return;
    }
    for term in FALLBACK_TERMINALS {
        if spawn(term).is_ok() {
            return;
        }
    }
    tracing::error!("No terminal emulator found; set [plugins] terminal in config");
}

/// A single searchable result that a plugin provides.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Display name shown in the results list
    pub name: String,
    /// Secondary description shown below the name (e.g. path, category)
    pub description: Option<String>,
    /// Icon name (XDG theme name like "firefox") or absolute path to PNG/SVG
    pub icon: Option<String>,
    /// How to launch: the plugin's launch() receives the entry back
    pub kind: EntryKind,
    /// Lower value = higher priority in results; set by scan_entries from config
    pub priority: u8,
}

#[derive(Debug, Clone)]
pub enum EntryKind {
    /// XDG .desktop application
    App { exec: String, terminal: bool },
    /// Raw shell command found on $PATH
    Command { path: std::path::PathBuf },
    /// Evaluated math expression; value is copied to clipboard on launch
    MathResult { value: String },
    /// Emoji character; copied to clipboard on launch
    EmojiResult { emoji: String },
    /// System power/session action; command is spawned via a shell-word split on launch
    Power { command: String },
}

/// Static plugin interface — all plugins are compiled in and registered at startup.
pub trait Plugin: Send + Sync {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    /// Populate the provided vec with all entries this plugin knows about.
    fn scan(&self, out: &mut Vec<Entry>);
    /// Called when the user activates an entry produced by this plugin.
    fn launch(&self, entry: &Entry);
    /// Produce dynamic entries from the current query string (called on every keystroke).
    fn query(&self, _input: &str, _out: &mut Vec<Entry>) {}
}
