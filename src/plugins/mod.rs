pub mod apps;
pub mod commands;

/// A single searchable result that a plugin provides.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Display name shown in the results list
    pub name: String,
    /// Secondary description shown below the name (e.g. path, category)
    pub description: Option<String>,
    /// Filesystem path to a PNG/SVG icon, resolved at scan time (used by icon rendering)
    #[allow(dead_code)]
    pub icon: Option<std::path::PathBuf>,
    /// How to launch: the plugin's launch() receives the entry back
    pub kind: EntryKind,
}

#[derive(Debug, Clone)]
pub enum EntryKind {
    /// XDG .desktop application
    App { exec: String, terminal: bool },
    /// Raw shell command found on $PATH
    Command { path: std::path::PathBuf },
}

/// Static plugin interface — all plugins are compiled in and registered at startup.
pub trait Plugin: Send + Sync {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    /// Populate the provided vec with all entries this plugin knows about.
    fn scan(&self, out: &mut Vec<Entry>);
    /// Called when the user activates an entry produced by this plugin.
    fn launch(&self, entry: &Entry);
}
