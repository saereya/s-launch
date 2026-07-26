use super::{Entry, EntryKind, Plugin};
use crate::config::PowerCommands;

pub struct PowerPlugin {
    commands: PowerCommands,
}

impl PowerPlugin {
    pub fn new(commands: PowerCommands) -> Self {
        Self { commands }
    }
}

impl Plugin for PowerPlugin {
    fn name(&self) -> &str {
        "power"
    }

    fn scan(&self, out: &mut Vec<Entry>) {
        let actions: [(&str, &str, &str); 4] = [
            ("Shutdown", "system-shutdown", &self.commands.shutdown),
            ("Reboot", "system-reboot", &self.commands.reboot),
            ("Suspend", "system-suspend", &self.commands.suspend),
            ("Lock Screen", "system-lock-screen", &self.commands.lock),
        ];

        for (name, icon, command) in actions {
            if command.trim().is_empty() {
                continue;
            }
            out.push(Entry {
                name: name.to_string(),
                description: Some(command.to_string()),
                icon: Some(icon.to_string()),
                kind: EntryKind::Power {
                    command: command.to_string(),
                },
                priority: 0,
            });
        }
    }

    fn launch(&self, entry: &Entry) {
        if let EntryKind::Power { command } = &entry.kind {
            spawn_command(command);
        }
    }
}

/// Run a configured power command through a shell.
///
/// The config documents these as shell commands and the useful values need to
/// be: `pgrep swaylock || swaylock`, `loginctl lock-session || swaylock`, a
/// pipeline, `$VAR`. They were previously split with shell-words and exec'd
/// directly, so every one of those was passed through as a literal argument and
/// silently did the wrong thing.
fn spawn_command(command: &str) {
    if command.trim().is_empty() {
        tracing::error!("Empty power command");
        return;
    }
    if let Err(e) = super::detached_command("sh").args(["-c", command]).spawn() {
        tracing::error!("Failed to run power command '{command}': {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(commands: PowerCommands) -> Vec<Entry> {
        let mut out = Vec::new();
        PowerPlugin::new(commands).scan(&mut out);
        out
    }

    #[test]
    fn default_commands_produce_four_entries() {
        let out = scan(PowerCommands::default());
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Shutdown", "Reboot", "Suspend", "Lock Screen"]);
    }

    #[test]
    fn empty_command_omits_that_action() {
        let out = scan(PowerCommands {
            suspend: String::new(),
            ..PowerCommands::default()
        });
        assert!(!out.iter().any(|e| e.name == "Suspend"));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn whitespace_only_command_is_treated_as_empty() {
        let out = scan(PowerCommands {
            lock: "   ".into(),
            ..PowerCommands::default()
        });
        assert!(!out.iter().any(|e| e.name == "Lock Screen"));
    }

    #[test]
    fn all_commands_empty_yields_no_entries() {
        let out = scan(PowerCommands {
            shutdown: String::new(),
            reboot: String::new(),
            suspend: String::new(),
            lock: String::new(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn entry_kind_carries_the_configured_command() {
        let out = scan(PowerCommands {
            shutdown: "doas poweroff".into(),
            ..PowerCommands::default()
        });
        let entry = out.iter().find(|e| e.name == "Shutdown").unwrap();
        match &entry.kind {
            EntryKind::Power { command } => assert_eq!(command, "doas poweroff"),
            _ => panic!("expected Power kind"),
        }
        assert_eq!(entry.description.as_deref(), Some("doas poweroff"));
    }
}
