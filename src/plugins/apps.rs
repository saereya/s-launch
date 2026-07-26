use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{launch_in_terminal, Entry, EntryKind, Plugin};

pub struct AppsPlugin {
    pub terminal: Option<String>,
}

impl AppsPlugin {
    pub fn new(terminal: Option<String>) -> Self {
        Self { terminal }
    }
}

impl Plugin for AppsPlugin {
    fn name(&self) -> &str {
        "apps"
    }

    fn scan(&self, out: &mut Vec<Entry>) {
        let dirs = xdg_application_dirs();
        // Dedup by desktop file ID (the .desktop filename). Dirs are scanned in
        // priority order (~/.local first), so the first occurrence of an ID wins
        // and shadows lower-priority dirs — matching the XDG override semantics.
        let mut seen: HashSet<String> = HashSet::new();

        for dir in &dirs {
            let Ok(read) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                // Mark seen even if it later fails to parse / is hidden, so a
                // higher-priority Hidden entry suppresses lower-priority copies.
                if !seen.insert(id.to_string()) {
                    continue;
                }
                let raw = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if let Some(app_entry) = parse_desktop(&raw, &path) {
                    out.push(app_entry);
                }
            }
        }
    }

    fn launch(&self, entry: &Entry) {
        if let EntryKind::App { exec, terminal } = &entry.kind {
            let cmd = strip_field_codes(exec);
            let Some(argv) = parse_exec(&cmd) else { return };
            if *terminal {
                launch_in_terminal(&argv, self.terminal.as_deref());
            } else {
                launch_detached(&argv);
            }
        }
    }
}

// ── .desktop parser ───────────────────────────────────────────────────────────

fn parse_desktop(raw: &str, path: &Path) -> Option<Entry> {
    let mut in_desktop_entry = false;
    let mut name = String::new();
    let mut exec = String::new();
    let mut icon_name = String::new();
    let mut no_display = false;
    let mut hidden = false;
    let mut terminal = false;
    let mut generic_name = String::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "Name" if name.is_empty() => name = v.trim().to_string(),
                "GenericName" if generic_name.is_empty() => generic_name = v.trim().to_string(),
                "Exec" if exec.is_empty() => exec = v.trim().to_string(),
                "Icon" if icon_name.is_empty() => icon_name = v.trim().to_string(),
                "NoDisplay" => no_display = v.trim().eq_ignore_ascii_case("true"),
                "Hidden" => hidden = v.trim().eq_ignore_ascii_case("true"),
                "Terminal" => terminal = v.trim().eq_ignore_ascii_case("true"),
                "Type" if v.trim() != "Application" => return None,
                _ => {}
            }
        }
    }

    if no_display || hidden || name.is_empty() || exec.is_empty() {
        return None;
    }

    let icon = if icon_name.is_empty() {
        None
    } else {
        Some(icon_name)
    };
    let description = if generic_name.is_empty() {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    } else {
        Some(generic_name)
    };

    Some(Entry {
        name,
        description,
        icon,
        kind: EntryKind::App { exec, terminal },
        priority: 0,
    })
}

/// Strip %u, %U, %f, %F, %i, %c, %k field codes from Exec values.
/// Per the desktop spec, `%%` is an escaped literal percent sign.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            // Consume the following code character; `%%` collapses to one `%`,
            // anything else (or a trailing lone `%`) is a field code we drop.
            if let Some('%') = chars.next() {
                out.push('%');
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn parse_exec(exec: &str) -> Option<Vec<String>> {
    match shell_words::split(exec) {
        Ok(args) if !args.is_empty() => Some(args),
        Ok(_) => {
            tracing::error!("Empty exec string");
            None
        }
        Err(e) => {
            tracing::error!("Could not parse exec '{exec}': {e}");
            None
        }
    }
}

fn launch_detached(argv: &[String]) {
    if let Err(e) = super::detached_command(&argv[0]).args(&argv[1..]).spawn() {
        tracing::error!("Failed to launch '{}': {e}", argv.join(" "));
    }
}

pub(crate) fn xdg_application_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .map(|d| PathBuf::from(d).join("applications"))
        .collect();

    if let Ok(home) = std::env::var("HOME") {
        dirs.insert(0, PathBuf::from(home).join(".local/share/applications"));
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn parse(raw: &str) -> Option<Entry> {
        parse_desktop(raw, Path::new("/usr/share/applications/test.desktop"))
    }

    #[test]
    fn parses_minimal_valid_entry() {
        let entry = parse("[Desktop Entry]\nName=Firefox\nExec=firefox %u\n").unwrap();
        assert_eq!(entry.name, "Firefox");
        match entry.kind {
            EntryKind::App { exec, terminal } => {
                assert_eq!(exec, "firefox %u");
                assert!(!terminal);
            }
            _ => panic!("expected App kind"),
        }
    }

    #[test]
    fn missing_name_is_rejected() {
        assert!(parse("[Desktop Entry]\nExec=firefox\n").is_none());
    }

    #[test]
    fn missing_exec_is_rejected() {
        assert!(parse("[Desktop Entry]\nName=Firefox\n").is_none());
    }

    #[test]
    fn no_display_entries_are_rejected() {
        let raw = "[Desktop Entry]\nName=Hidden App\nExec=foo\nNoDisplay=true\n";
        assert!(parse(raw).is_none());
    }

    #[test]
    fn hidden_entries_are_rejected() {
        let raw = "[Desktop Entry]\nName=Hidden App\nExec=foo\nHidden=true\n";
        assert!(parse(raw).is_none());
    }

    #[test]
    fn non_application_type_is_rejected() {
        let raw = "[Desktop Entry]\nName=Some Link\nExec=foo\nType=Link\n";
        assert!(parse(raw).is_none());
    }

    #[test]
    fn terminal_flag_is_captured() {
        let raw = "[Desktop Entry]\nName=Htop\nExec=htop\nTerminal=true\n";
        let entry = parse(raw).unwrap();
        match entry.kind {
            EntryKind::App { terminal, .. } => assert!(terminal),
            _ => panic!("expected App kind"),
        }
    }

    #[test]
    fn generic_name_is_preferred_as_description() {
        let raw = "[Desktop Entry]\nName=Firefox\nGenericName=Web Browser\nExec=firefox\n";
        let entry = parse(raw).unwrap();
        assert_eq!(entry.description.as_deref(), Some("Web Browser"));
    }

    #[test]
    fn description_falls_back_to_parent_dir_name_without_generic_name() {
        let entry = parse_desktop(
            "[Desktop Entry]\nName=Firefox\nExec=firefox\n",
            Path::new("/usr/share/applications/firefox.desktop"),
        )
        .unwrap();
        assert_eq!(entry.description.as_deref(), Some("applications"));
    }

    #[test]
    fn icon_is_none_when_unset() {
        let entry = parse("[Desktop Entry]\nName=Firefox\nExec=firefox\n").unwrap();
        assert_eq!(entry.icon, None);
    }

    #[test]
    fn icon_is_captured_when_set() {
        let raw = "[Desktop Entry]\nName=Firefox\nExec=firefox\nIcon=firefox\n";
        let entry = parse(raw).unwrap();
        assert_eq!(entry.icon.as_deref(), Some("firefox"));
    }

    #[test]
    fn first_occurrence_of_a_repeated_key_wins() {
        let raw = "[Desktop Entry]\nName=First\nName=Second\nExec=foo\n";
        let entry = parse(raw).unwrap();
        assert_eq!(entry.name, "First");
    }

    #[test]
    fn keys_outside_desktop_entry_section_are_ignored() {
        let raw = "[Desktop Action New]\nName=Wrong\n[Desktop Entry]\nName=Right\nExec=foo\n";
        let entry = parse(raw).unwrap();
        assert_eq!(entry.name, "Right");
    }

    #[test]
    fn comment_lines_are_ignored() {
        let raw = "[Desktop Entry]\n# Name=Commented\nName=Real\nExec=foo\n";
        let entry = parse(raw).unwrap();
        assert_eq!(entry.name, "Real");
    }

    #[test]
    fn strip_field_codes_removes_known_codes() {
        assert_eq!(strip_field_codes("firefox %u"), "firefox");
        assert_eq!(strip_field_codes("app %f %F %U %i %c %k"), "app");
        assert_eq!(strip_field_codes("cmd %u --flag"), "cmd  --flag");
    }

    #[test]
    fn strip_field_codes_collapses_escaped_percent() {
        assert_eq!(strip_field_codes("echo 100%%"), "echo 100%");
    }

    #[test]
    fn strip_field_codes_trims_result() {
        assert_eq!(strip_field_codes("  firefox %u  "), "firefox");
    }

    fn write_desktop_file(dir: &Path, filename: &str, contents: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let mut f = std::fs::File::create(dir.join(filename)).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn xdg_application_dirs_puts_home_local_share_first() {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _home = unsafe { crate::test_env::EnvVarGuard::set("HOME", "/home/test-user") };
        let _xdg = unsafe {
            crate::test_env::EnvVarGuard::set("XDG_DATA_DIRS", "/usr/local/share:/usr/share")
        };
        let dirs = xdg_application_dirs();
        assert_eq!(
            dirs[0],
            PathBuf::from("/home/test-user/.local/share/applications")
        );
        assert!(dirs.contains(&PathBuf::from("/usr/local/share/applications")));
        assert!(dirs.contains(&PathBuf::from("/usr/share/applications")));
    }

    #[test]
    fn scan_dedups_by_desktop_id_preferring_earlier_dir() {
        let _guard = crate::test_env::lock();
        let home_tmp = tempfile::tempdir().unwrap();
        let sys_tmp = tempfile::tempdir().unwrap();

        let home_apps = home_tmp.path().join(".local/share/applications");
        write_desktop_file(
            &home_apps,
            "app.desktop",
            "[Desktop Entry]\nName=Home Version\nExec=foo\n",
        );

        let sys_apps = sys_tmp.path().join("applications");
        write_desktop_file(
            &sys_apps,
            "app.desktop",
            "[Desktop Entry]\nName=System Version\nExec=foo\n",
        );
        // Second, non-conflicting app only present in the system dir.
        write_desktop_file(
            &sys_apps,
            "other.desktop",
            "[Desktop Entry]\nName=Other App\nExec=bar\n",
        );

        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _home =
            unsafe { crate::test_env::EnvVarGuard::set("HOME", home_tmp.path().to_str().unwrap()) };
        let _xdg = unsafe {
            crate::test_env::EnvVarGuard::set("XDG_DATA_DIRS", sys_tmp.path().to_str().unwrap())
        };

        let mut out = Vec::new();
        AppsPlugin::new(None).scan(&mut out);

        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Home Version"));
        assert!(!names.contains(&"System Version"));
        assert!(names.contains(&"Other App"));
    }
}
