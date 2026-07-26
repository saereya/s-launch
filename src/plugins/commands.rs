use std::collections::HashSet;
use std::path::PathBuf;

use super::{launch_in_terminal, Entry, EntryKind, Plugin};

pub struct CommandsPlugin {
    pub terminal: Option<String>,
    pub terminal_args: Option<Vec<String>>,
}

impl CommandsPlugin {
    pub fn new(terminal: Option<String>, terminal_args: Option<Vec<String>>) -> Self {
        Self {
            terminal,
            terminal_args,
        }
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

            for dir_entry in read.flatten() {
                let path = dir_entry.path();
                // file_type() comes from readdir's d_type on Linux, so the common
                // case (a regular file) costs no stat at all. Symlinks need the
                // target resolved, since one can point at a directory.
                let is_dir = match dir_entry.file_type() {
                    Ok(t) if t.is_symlink() => path.metadata().map(|m| m.is_dir()).unwrap_or(false),
                    Ok(t) => t.is_dir(),
                    Err(_) => path.metadata().map(|m| m.is_dir()).unwrap_or(false),
                };
                if is_dir || !is_executable(&path) {
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
            // Single-element argv: the path is the whole command, and gets quoted
            // for the wrapping shell. to_string_lossy rather than to_str, which
            // used to silently launch "" for a non-UTF-8 path.
            let argv = vec![path.to_string_lossy().into_owned()];
            launch_in_terminal(&argv, self.terminal.as_deref(), self.terminal_args.as_ref());
        }
    }
}

/// Whether *this* user can actually execute `path`.
///
/// `access(X_OK)` rather than testing `mode & 0o111`: that reported any execute
/// bit, so a root-owned 0700 binary in a `$PATH` directory was offered as a
/// launchable command to a normal user who could never run it. One syscall, and
/// it accounts for ownership, ACLs and mount options.
///
/// Returns true for directories (they're searchable), so callers must exclude
/// those separately — `scan` does it from readdir's file type.
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false; // interior NUL; not a real path
    };
    // SAFETY: access(2) with a valid NUL-terminated path and a valid mode flag.
    unsafe { libc::access(c_path.as_ptr(), libc::X_OK) == 0 }
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
    fn scan_skips_directories_on_path() {
        // is_executable() itself reports true for a directory (they're
        // searchable), so this is scan's responsibility — assert it there.
        let _guard = crate::test_env::lock();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("subdir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_file(tmp.path(), "realtool", 0o755);

        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env =
            unsafe { crate::test_env::EnvVarGuard::set("PATH", tmp.path().to_str().unwrap()) };

        let mut out = Vec::new();
        CommandsPlugin::new(None, None).scan(&mut out);
        assert!(out.iter().any(|e| e.name == "realtool"));
        assert!(
            !out.iter().any(|e| e.name == "subdir"),
            "directories must not appear as commands"
        );
    }

    #[test]
    fn scan_skips_a_symlink_pointing_at_a_directory() {
        let _guard = crate::test_env::lock();
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("targetdir");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join("dirlink")).unwrap();
        write_file(tmp.path(), "realtool", 0o755);

        // SAFETY: guarded by ENV_LOCK for the duration of this test.
        let _env =
            unsafe { crate::test_env::EnvVarGuard::set("PATH", tmp.path().to_str().unwrap()) };

        let mut out = Vec::new();
        CommandsPlugin::new(None, None).scan(&mut out);
        assert!(out.iter().any(|e| e.name == "realtool"));
        assert!(
            !out.iter().any(|e| e.name == "dirlink"),
            "a symlink to a directory must not appear as a command"
        );
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
        CommandsPlugin::new(None, None).scan(&mut out);

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
