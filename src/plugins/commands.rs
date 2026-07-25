use std::collections::HashSet;
use std::path::PathBuf;

use super::{launch_in_terminal, Entry, EntryKind, Plugin};

pub struct CommandsPlugin {
    pub terminal: Option<String>,
}

impl CommandsPlugin {
    pub fn new(terminal: Option<String>) -> Self {
        Self { terminal }
    }
}

impl Plugin for CommandsPlugin {
    fn name(&self) -> &str {
        "commands"
    }

    fn scan(&self, out: &mut Vec<Entry>) {
        let mut seen: HashSet<String> = HashSet::new();

        let path_var = std::env::var("PATH").unwrap_or_default();
        for dir in path_var.split(':') {
            let dir_path = PathBuf::from(dir);
            let Ok(read) = std::fs::read_dir(&dir_path) else {
                continue;
            };

            for entry in read.flatten() {
                let path = entry.path();
                // Skip directories and non-executable files
                if !is_executable(&path) {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let name = name.to_string();

                if seen.insert(name.clone()) {
                    out.push(Entry {
                        name,
                        description: Some(dir_path.display().to_string()),
                        icon: None,
                        kind: EntryKind::Command { path: path.clone() },
                        priority: 0,
                    });
                }
            }
        }
    }

    fn launch(&self, entry: &Entry) {
        if let EntryKind::Command { path } = &entry.kind {
            launch_in_terminal(path.to_str().unwrap_or_default(), self.terminal.as_deref());
        }
    }
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| !m.is_dir() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_file(dir: &std::path::Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn executable_file_is_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_file(tmp.path(), "runme", 0o755);
        assert!(is_executable(&path));
    }

    #[test]
    fn non_executable_file_is_not_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_file(tmp.path(), "readonly", 0o644);
        assert!(!is_executable(&path));
    }

    #[test]
    fn directory_is_not_executable_even_with_x_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("subdir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_executable(&dir));
    }

    #[test]
    fn nonexistent_path_is_not_executable() {
        assert!(!is_executable(std::path::Path::new(
            "/nonexistent/path/does-not-exist"
        )));
    }

    #[test]
    fn scan_dedups_by_name_preferring_earlier_path_dir() {
        let _guard = crate::test_env::lock();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();

        write_file(first.path(), "mytool", 0o755);
        write_file(second.path(), "mytool", 0o755);
        write_file(second.path(), "othertool", 0o755);
        write_file(first.path(), "notexec.txt", 0o644);

        let path_var = format!("{}:{}", first.path().display(), second.path().display());
        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env = unsafe { crate::test_env::EnvVarGuard::set("PATH", &path_var) };

        let mut out = Vec::new();
        CommandsPlugin::new(None).scan(&mut out);

        let mytool_entries: Vec<_> = out.iter().filter(|e| e.name == "mytool").collect();
        assert_eq!(mytool_entries.len(), 1, "mytool must appear only once");
        match &mytool_entries[0].kind {
            EntryKind::Command { path } => assert_eq!(path, &first.path().join("mytool")),
            _ => panic!("expected Command kind"),
        }
        assert!(out.iter().any(|e| e.name == "othertool"));
        assert!(!out.iter().any(|e| e.name == "notexec.txt"));
    }
}
