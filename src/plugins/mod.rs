pub mod apps;
pub mod commands;
pub mod emoji;
pub mod math;
pub mod power;

const FALLBACK_TERMINALS: &[&str] = &["foot", "alacritty", "kitty", "wezterm", "xterm"];

/// Build a `Command` detached from the daemon: no inherited stdio, and SIGCHLD
/// restored to its default disposition in the child.
///
/// `run_daemon` sets SIGCHLD to `SIG_IGN` so the kernel auto-reaps the
/// processes we spawn and never wait on. That disposition is *inherited across
/// `execve`* — POSIX resets caught signals to default but leaves ignored ones
/// ignored — so without this reset every app we launch would start with SIGCHLD
/// ignored, which makes `system()`, `popen()` and bare `waitpid()` calls *inside
/// that app* fail with ECHILD. Undo it between fork and exec.
pub(super) fn detached_command(program: &str) -> std::process::Command {
    use std::os::unix::process::CommandExt;

    let mut cmd = std::process::Command::new(program);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // SAFETY: pre_exec runs in the forked child before exec, where only
    // async-signal-safe calls are permitted. signal(2) is on the POSIX
    // async-signal-safe list, and we touch no other process state.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
            Ok(())
        });
    }
    cmd
}

pub(super) fn launch_in_terminal(cmd: &str, terminal: Option<&str>) {
    if shell_words::split(cmd).map_or(true, |a| a.is_empty()) {
        tracing::error!("Empty or unparseable exec string: '{cmd}'");
        return;
    }
    // Wrap in a shell so the terminal stays open after the command exits.
    let shell_cmd = format!("{cmd}; exec $SHELL");
    let spawn = |term: &str| {
        detached_command(term)
            .args(["-e", "sh", "-c", &shell_cmd])
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
///
/// `PartialEq` is how the UI recognises an entry across a result-set refresh —
/// both to keep the highlight on the same row and to skip rebuilding the list
/// when a rescan produced an identical set.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
