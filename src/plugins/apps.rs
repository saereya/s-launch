use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{launch_in_terminal, Entry, EntryKind, Plugin};

pub struct AppsPlugin {
    pub terminal: Option<String>,
    pub terminal_args: Option<Vec<String>>,
}

impl AppsPlugin {
    pub fn new(terminal: Option<String>, terminal_args: Option<Vec<String>>) -> Self {
        Self {
            terminal,
            terminal_args,
        }
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
                launch_in_terminal(&argv, self.terminal.as_deref(), self.terminal_args.as_ref());
            } else {
                launch_detached(&argv);
            }
        }
    }
}

// ── .desktop parser ───────────────────────────────────────────────────────────

fn parse_desktop(raw: &str, path: &Path) -> Option<Entry> {
    // Collected into a map rather than matched inline, so locale-suffixed keys
    // (`Name[de]`) can be resolved against the bare key afterwards.
    let mut fields: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    let mut in_desktop_entry = false;

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
            // First occurrence of a key wins.
            fields.entry(k.trim()).or_insert(v.trim());
        }
    }

    if fields.get("Type").is_some_and(|t| *t != "Application") {
        return None;
    }
    if is_true(fields.get("NoDisplay")) || is_true(fields.get("Hidden")) {
        return None;
    }
    if !shown_in_current_desktop(&fields) || !try_exec_available(&fields) {
        return None;
    }

    let name = localized(&fields, "Name").unwrap_or_default();
    let exec = fields.get("Exec").copied().unwrap_or_default();
    if name.is_empty() || exec.is_empty() {
        return None;
    }

    let description = match localized(&fields, "GenericName") {
        Some(generic) if !generic.is_empty() => Some(generic.to_string()),
        _ => path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string()),
    };

    Some(Entry {
        name: name.to_string(),
        description,
        icon: fields
            .get("Icon")
            .filter(|i| !i.is_empty())
            .map(|i| i.to_string()),
        kind: EntryKind::App {
            exec: exec.to_string(),
            terminal: is_true(fields.get("Terminal")),
        },
        priority: 0,
    })
}

fn is_true(value: Option<&&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// Locale suffixes to try for a translatable key, most specific first —
/// `de_DE` then `de` for `LANG=de_DE.UTF-8`.
fn locale_suffixes() -> Vec<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX");
    let Some(raw) = raw else {
        return Vec::new();
    };
    // Strip encoding and modifier: de_DE.UTF-8@euro -> de_DE
    let base = raw
        .split('.')
        .next()
        .unwrap_or(&raw)
        .split('@')
        .next()
        .unwrap_or(&raw);
    let mut out = vec![base.to_string()];
    if let Some((language, _)) = base.split_once('_') {
        out.push(language.to_string());
    }
    out
}

/// Resolve a translatable key, honouring the locale suffixes the desktop spec
/// defines. Without this the launcher showed English names regardless of locale.
fn localized<'a>(
    fields: &std::collections::HashMap<&'a str, &'a str>,
    key: &str,
) -> Option<&'a str> {
    for suffix in locale_suffixes() {
        if let Some(value) = fields.get(format!("{key}[{suffix}]").as_str()) {
            return Some(value);
        }
    }
    fields.get(key).copied()
}

/// Honour `OnlyShowIn` / `NotShowIn` against `$XDG_CURRENT_DESKTOP`.
///
/// Without this, desktop-specific entries — GNOME Control Center, KDE's
/// systemsettings — show up under Sway or Hyprland, where they're useless or
/// won't start at all.
fn shown_in_current_desktop(fields: &std::collections::HashMap<&str, &str>) -> bool {
    let current: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let listed = |list: &str| {
        list.split(';')
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
            .any(|env| current.iter().any(|c| c == &env.to_ascii_lowercase()))
    };

    if let Some(only) = fields.get("OnlyShowIn") {
        if !listed(only) {
            return false;
        }
    }
    if let Some(not) = fields.get("NotShowIn") {
        if listed(not) {
            return false;
        }
    }
    true
}

/// Honour `TryExec`: the spec says an entry whose `TryExec` program is missing
/// should not be shown. Filters out leftovers for uninstalled applications.
fn try_exec_available(fields: &std::collections::HashMap<&str, &str>) -> bool {
    let Some(try_exec) = fields.get("TryExec").map(|t| t.trim()) else {
        return true;
    };
    if try_exec.is_empty() {
        return true;
    }
    if try_exec.contains('/') {
        return Path::new(try_exec).is_file();
    }
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .any(|dir| Path::new(dir).join(try_exec).is_file())
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
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("applications"))
        .collect();

    if let Some(data_home) = user_data_home() {
        dirs.insert(0, data_home.join("applications"));
    }
    dirs
}

/// The user's data directory, per the XDG basedir spec: `$XDG_DATA_HOME` if set,
/// otherwise `~/.local/share`.
///
/// Only `$HOME` used to be consulted, so anyone who relocates `XDG_DATA_HOME`
/// lost every user-level `.desktop` entry — inconsistent with the config paths,
/// which already go through the `xdg` crate.
fn user_data_home() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".local/share"))
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
        let _data_home = unsafe { crate::test_env::EnvVarGuard::remove("XDG_DATA_HOME") };
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
    fn xdg_data_home_overrides_the_home_local_share_default() {
        // Regression: only $HOME was consulted, so relocating XDG_DATA_HOME lost
        // every user-level .desktop entry.
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _home = unsafe { crate::test_env::EnvVarGuard::set("HOME", "/home/test-user") };
        let _data_home =
            unsafe { crate::test_env::EnvVarGuard::set("XDG_DATA_HOME", "/elsewhere/data") };
        let _xdg = unsafe { crate::test_env::EnvVarGuard::set("XDG_DATA_DIRS", "/usr/share") };
        let dirs = xdg_application_dirs();
        assert_eq!(dirs[0], PathBuf::from("/elsewhere/data/applications"));
        assert!(!dirs.iter().any(|d| d.starts_with("/home/test-user/.local")));
    }

    #[test]
    fn empty_xdg_data_home_falls_back_to_home() {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _home = unsafe { crate::test_env::EnvVarGuard::set("HOME", "/home/test-user") };
        let _data_home = unsafe { crate::test_env::EnvVarGuard::set("XDG_DATA_HOME", "") };
        let dirs = xdg_application_dirs();
        assert_eq!(
            dirs[0],
            PathBuf::from("/home/test-user/.local/share/applications")
        );
    }

    // ── OnlyShowIn / NotShowIn ───────────────────────────────────────────────

    fn parse_in_desktop(raw: &str, current_desktop: &str) -> Option<Entry> {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env =
            unsafe { crate::test_env::EnvVarGuard::set("XDG_CURRENT_DESKTOP", current_desktop) };
        parse(raw)
    }

    #[test]
    fn only_show_in_hides_entries_for_other_desktops() {
        // A GNOME-only entry under Hyprland: previously shown, and useless there.
        let raw =
            "[Desktop Entry]\nName=GNOME Settings\nExec=gnome-control-center\nOnlyShowIn=GNOME;\n";
        assert!(parse_in_desktop(raw, "Hyprland").is_none());
        assert!(parse_in_desktop(raw, "GNOME").is_some());
    }

    #[test]
    fn only_show_in_matches_any_entry_in_the_current_desktop_list() {
        let raw = "[Desktop Entry]\nName=App\nExec=app\nOnlyShowIn=wlroots;GNOME;\n";
        assert!(parse_in_desktop(raw, "Hyprland:wlroots").is_some());
    }

    #[test]
    fn only_show_in_comparison_ignores_case() {
        let raw = "[Desktop Entry]\nName=App\nExec=app\nOnlyShowIn=gnome;\n";
        assert!(parse_in_desktop(raw, "GNOME").is_some());
    }

    #[test]
    fn not_show_in_hides_entries_for_the_current_desktop() {
        let raw = "[Desktop Entry]\nName=App\nExec=app\nNotShowIn=Hyprland;\n";
        assert!(parse_in_desktop(raw, "Hyprland").is_none());
        assert!(parse_in_desktop(raw, "sway").is_some());
    }

    #[test]
    fn only_show_in_hides_the_entry_when_no_desktop_is_set() {
        let raw = "[Desktop Entry]\nName=App\nExec=app\nOnlyShowIn=GNOME;\n";
        assert!(parse_in_desktop(raw, "").is_none());
    }

    // ── TryExec ──────────────────────────────────────────────────────────────

    #[test]
    fn try_exec_pointing_at_a_missing_binary_hides_the_entry() {
        let _guard = crate::test_env::lock();
        let raw =
            "[Desktop Entry]\nName=Gone\nExec=gone\nTryExec=/nonexistent/definitely-not-here\n";
        assert!(parse(raw).is_none());
    }

    #[test]
    fn try_exec_pointing_at_an_existing_binary_keeps_the_entry() {
        let _guard = crate::test_env::lock();
        let raw = "[Desktop Entry]\nName=Shell\nExec=sh\nTryExec=/bin/sh\n";
        assert!(parse(raw).is_some());
    }

    #[test]
    fn bare_try_exec_name_is_searched_on_path() {
        let _guard = crate::test_env::lock();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("mytool"), b"#!/bin/sh\n").unwrap();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env =
            unsafe { crate::test_env::EnvVarGuard::set("PATH", tmp.path().to_str().unwrap()) };

        assert!(parse("[Desktop Entry]\nName=T\nExec=t\nTryExec=mytool\n").is_some());
        assert!(parse("[Desktop Entry]\nName=T\nExec=t\nTryExec=absent-tool\n").is_none());
    }

    // ── Localised names ──────────────────────────────────────────────────────

    fn parse_in_locale(raw: &str, lang: &str) -> Option<Entry> {
        let _guard = crate::test_env::lock();
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _lc_all = unsafe { crate::test_env::EnvVarGuard::remove("LC_ALL") };
        let _lc_messages = unsafe { crate::test_env::EnvVarGuard::remove("LC_MESSAGES") };
        let _lang = unsafe { crate::test_env::EnvVarGuard::set("LANG", lang) };
        parse(raw)
    }

    #[test]
    fn localised_name_is_preferred_for_the_current_locale() {
        let raw = "[Desktop Entry]\nName=Files\nName[de]=Dateien\nExec=files\n";
        assert_eq!(parse_in_locale(raw, "de_DE.UTF-8").unwrap().name, "Dateien");
        assert_eq!(parse_in_locale(raw, "en_GB.UTF-8").unwrap().name, "Files");
    }

    #[test]
    fn region_specific_localisation_beats_the_bare_language() {
        let raw = "[Desktop Entry]\nName=Colour\nName[pt]=Cor\nName[pt_BR]=Cor BR\nExec=x\n";
        assert_eq!(parse_in_locale(raw, "pt_BR.UTF-8").unwrap().name, "Cor BR");
        assert_eq!(parse_in_locale(raw, "pt_PT.UTF-8").unwrap().name, "Cor");
    }

    #[test]
    fn locale_modifier_and_encoding_are_stripped() {
        let raw = "[Desktop Entry]\nName=Base\nName[de]=Deutsch\nExec=x\n";
        assert_eq!(
            parse_in_locale(raw, "de_DE.UTF-8@euro").unwrap().name,
            "Deutsch"
        );
    }

    #[test]
    fn c_locale_uses_the_unlocalised_name() {
        let raw = "[Desktop Entry]\nName=Base\nName[de]=Deutsch\nExec=x\n";
        assert_eq!(parse_in_locale(raw, "C").unwrap().name, "Base");
    }

    #[test]
    fn generic_name_is_localised_too() {
        let raw =
            "[Desktop Entry]\nName=Firefox\nGenericName=Web Browser\nGenericName[de]=Webbrowser\nExec=firefox\n";
        assert_eq!(
            parse_in_locale(raw, "de_DE.UTF-8")
                .unwrap()
                .description
                .as_deref(),
            Some("Webbrowser")
        );
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
        AppsPlugin::new(None, None).scan(&mut out);

        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Home Version"));
        assert!(!names.contains(&"System Version"));
        assert!(names.contains(&"Other App"));
    }
}
