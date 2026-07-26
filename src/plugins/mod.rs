pub mod apps;
pub mod commands;
pub mod emoji;
pub mod math;
pub mod power;

/// Terminals we know how to drive, with the arguments that make each one run a
/// command. Used both as the auto-detection order and to pick arguments when
/// `terminal` is configured but `terminal_args` isn't.
///
/// `-e` is not the universal convention it looks like: wezterm needs
/// `start --`, gnome-terminal takes `--`. wezterm was previously in the
/// auto-detect list *and* invoked with `-e`, so it could never have worked.
const KNOWN_TERMINALS: &[(&str, &[&str])] = &[
    ("foot", &["-e"]),
    ("alacritty", &["-e"]),
    ("kitty", &["-e"]),
    ("ghostty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xterm", &["-e"]),
];

/// Arguments to run a command in `terminal`: the configured ones if given,
/// otherwise the known-terminal entry, otherwise `-e` as the best guess.
fn terminal_exec_args(terminal: &str, configured: Option<&Vec<String>>) -> Vec<String> {
    if let Some(args) = configured {
        return args.clone();
    }
    let basename = std::path::Path::new(terminal)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(terminal);
    KNOWN_TERMINALS
        .iter()
        .find(|(name, _)| *name == basename)
        .map(|(_, args)| args.iter().map(|s| s.to_string()).collect())
        .unwrap_or_else(|| vec!["-e".to_string()])
}

/// Put `text` on the clipboard using the configured command.
pub(super) fn copy_to_clipboard(text: &str, command: &[String]) {
    let Some((program, leading)) = command.split_first() else {
        tracing::error!("No clipboard command configured; set [plugins] clipboard");
        return;
    };
    if let Err(e) = detached_command(program)
        .args(leading)
        .arg(text)
        .spawn()
        .map_err(|e| e.to_string())
    {
        tracing::error!(
            "Clipboard command '{}' failed: {e}. Install wl-clipboard, or set \
             [plugins] clipboard (e.g. [\"xclip\", \"-selection\", \"clipboard\"])",
            command.join(" ")
        );
    }
}

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
        .stderr(std::process::Stdio::null())
        // Own process group, so a Ctrl-C aimed at a foreground `slaunch daemon`
        // (the documented dev workflow) doesn't take every app launched from it
        // down as collateral.
        .process_group(0);
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

/// Run `argv` inside a terminal emulator, keeping the terminal open afterwards.
///
/// Takes an already-split argv rather than a command string because the pieces
/// have to be re-quoted for the shell we wrap them in: interpolating a raw
/// string meant a path like `/opt/my apps/tool` ran as two words and failed with
/// 127 — invisibly, since the child's stderr goes to /dev/null. Quoting also
/// stops a filename on `$PATH` from being read as shell syntax.
pub(super) fn launch_in_terminal(
    argv: &[String],
    terminal: Option<&str>,
    terminal_args: Option<&Vec<String>>,
) {
    if argv.is_empty() {
        tracing::error!("Refusing to launch an empty command in a terminal");
        return;
    }
    let quoted = argv
        .iter()
        .map(|arg| shell_words::quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    // Wrap in a shell so the terminal stays open after the command exits.
    // $SHELL can be unset (e.g. under a bare systemd unit), hence the fallback.
    let shell_cmd = format!("{quoted}; exec \"${{SHELL:-/bin/sh}}\"");

    let spawn = |term: &str, args: &[String]| {
        detached_command(term)
            .args(args)
            .args(["sh", "-c", &shell_cmd])
            .spawn()
    };

    if let Some(term) = terminal {
        let args = terminal_exec_args(term, terminal_args);
        if let Err(e) = spawn(term, &args) {
            tracing::error!("Failed to launch terminal '{term}': {e}");
        }
        return;
    }

    for (term, _) in KNOWN_TERMINALS {
        let args = terminal_exec_args(term, None);
        match spawn(term, &args) {
            Ok(_) => return,
            // Not installed — try the next one. Anything else means the terminal
            // is present but wouldn't start, which is worth reporting rather than
            // silently falling through: the old code tested only `is_ok()`, so a
            // terminal that rejected its arguments still counted as success and
            // the user got nothing, with no error either.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::error!("Terminal '{term}' is installed but failed to start: {e}");
                return;
            }
        }
    }
    tracing::error!(
        "No terminal emulator found. Set [plugins] terminal (and terminal_args \
         if it isn't one of: {})",
        KNOWN_TERMINALS
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(", ")
    );
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
    /// Extra text the fuzzy matcher should search but the row shouldn't show —
    /// a `.desktop` file's `Keywords=` and `GenericName`, for instance, so
    /// "browser" finds Firefox.
    ///
    /// Deliberately separate from `description`: for commands that's the `$PATH`
    /// directory, and indexing it would make every one of ~2300 entries match
    /// "usr" or "bin".
    pub keywords: Option<String>,
}

impl Entry {
    /// The haystack the fuzzy matcher indexes for this entry.
    pub fn search_text(&self) -> std::borrow::Cow<'_, str> {
        match &self.keywords {
            Some(extra) if !extra.is_empty() => {
                std::borrow::Cow::Owned(format!("{} {extra}", self.name))
            }
            _ => std::borrow::Cow::Borrowed(&self.name),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_terminals_get_their_own_arguments() {
        // wezterm is the reason this table exists: it was in the auto-detect list
        // and invoked with `-e`, which it doesn't accept, so it never worked.
        assert_eq!(terminal_exec_args("wezterm", None), vec!["start", "--"]);
        assert_eq!(terminal_exec_args("gnome-terminal", None), vec!["--"]);
        assert_eq!(terminal_exec_args("foot", None), vec!["-e"]);
        assert_eq!(terminal_exec_args("alacritty", None), vec!["-e"]);
    }

    #[test]
    fn an_absolute_terminal_path_still_matches_by_basename() {
        assert_eq!(
            terminal_exec_args("/usr/bin/wezterm", None),
            vec!["start", "--"]
        );
    }

    #[test]
    fn an_unknown_terminal_falls_back_to_dash_e() {
        assert_eq!(terminal_exec_args("someterm", None), vec!["-e"]);
    }

    #[test]
    fn configured_arguments_win_over_the_table() {
        let configured = vec!["--command".to_string()];
        assert_eq!(
            terminal_exec_args("wezterm", Some(&configured)),
            vec!["--command"]
        );
    }

    #[test]
    fn configured_empty_arguments_are_respected() {
        // kitty and others accept a bare `kitty <cmd>`; an empty list must mean
        // "pass no flag", not "fall back to the default".
        let configured: Vec<String> = Vec::new();
        assert!(terminal_exec_args("kitty", Some(&configured)).is_empty());
    }

    /// Exercises the real spawn path — `detached_command` with its pre_exec and
    /// process-group setup — by pointing the clipboard command at a script that
    /// records how it was invoked.
    #[test]
    fn clipboard_command_receives_the_text_as_its_last_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("invocation.log");
        let script = tmp.path().join("fake-copy");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", log.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let command = vec![
            script.to_string_lossy().into_owned(),
            "--selection".to_string(),
        ];
        copy_to_clipboard("hello world", &command);

        // The child is detached and never waited on, so poll for its output.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !log.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let recorded = std::fs::read_to_string(&log).expect("clipboard command never ran");
        assert_eq!(recorded, "--selection\nhello world\n");
    }

    #[test]
    fn an_empty_clipboard_command_is_reported_not_panicked() {
        copy_to_clipboard("text", &[]); // logs an error, must not panic
    }

    #[test]
    fn launched_children_do_not_inherit_sigchld_ignored() {
        // The daemon sets SIGCHLD to SIG_IGN so the kernel reaps what it spawns,
        // and that disposition survives execve — which broke system()/popen()
        // inside every launched app. detached_command must undo it in the child.
        // SAFETY: this test process is allowed to set its own disposition; the
        // point is to observe what a child inherits through it.
        unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };

        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("status");
        let script = tmp.path().join("report-sigign");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ngrep '^SigIgn:' /proc/self/status > {}\n",
                log.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut child = detached_command(script.to_str().unwrap()).spawn().unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !log.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // SAFETY: restore the default disposition before reaping, so the wait
        // below can actually collect the child instead of hitting ECHILD.
        unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) };
        // It may already have been auto-reaped while SIGCHLD was ignored; either
        // outcome is fine, this just avoids leaving a zombie behind.
        let _ = child.wait();

        let status = std::fs::read_to_string(&log).expect("child never reported");
        let mask = u64::from_str_radix(status.split_whitespace().nth(1).unwrap(), 16).unwrap();
        // SIGCHLD is signal 17, i.e. bit 16 of the mask.
        assert_eq!(
            mask & (1 << 16),
            0,
            "child inherited SIGCHLD as ignored (SigIgn={status:?})"
        );
    }
}
