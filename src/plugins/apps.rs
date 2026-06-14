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
            let Ok(read) = std::fs::read_dir(dir) else { continue };
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path.file_name().and_then(|n| n.to_str()) else { continue };
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
            if *terminal {
                launch_in_terminal(&cmd, self.terminal.as_deref());
            } else {
                launch_detached(&cmd);
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

    let icon = if icon_name.is_empty() { None } else { Some(icon_name) };
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
            // Consume the following code character; `%%` collapses to one `%`.
            match chars.next() {
                Some('%') => out.push('%'),
                _ => {} // drop the field code (or trailing lone `%`)
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

fn launch_detached(exec: &str) {
    let Some(args) = parse_exec(exec) else { return };
    if let Err(e) = std::process::Command::new(&args[0])
        .args(&args[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        tracing::error!("Failed to launch '{exec}': {e}");
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
