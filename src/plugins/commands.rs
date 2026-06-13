use std::collections::HashSet;
use std::path::PathBuf;

use super::{Entry, EntryKind, Plugin};

pub struct CommandsPlugin;

impl Plugin for CommandsPlugin {
    fn name(&self) -> &str {
        "commands"
    }

    fn scan(&self, out: &mut Vec<Entry>) {
        let mut seen: HashSet<String> = HashSet::new();

        let path_var = std::env::var("PATH").unwrap_or_default();
        for dir in path_var.split(':') {
            let dir_path = PathBuf::from(dir);
            let Ok(read) = std::fs::read_dir(&dir_path) else { continue };

            for entry in read.flatten() {
                let path = entry.path();
                // Skip directories and non-executable files
                if !is_executable(&path) {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
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
            use std::process::Command;
            if let Err(e) = Command::new(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                tracing::error!("Failed to launch '{}': {e}", path.display());
            }
        }
    }
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| !m.is_dir() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
