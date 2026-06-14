use super::{Entry, EntryKind, Plugin};

pub struct EmojiPlugin;

impl Plugin for EmojiPlugin {
    fn name(&self) -> &str {
        "emoji"
    }

    fn scan(&self, _out: &mut Vec<Entry>) {}

    fn query(&self, input: &str, out: &mut Vec<Entry>) {
        let search = input.trim_start_matches(':').trim().to_lowercase();
        if search.is_empty() {
            return;
        }

        for emoji in emojis::iter() {
            let name = emoji.name();
            let shortcode = emoji.shortcode().unwrap_or("");
            if !name.contains(search.as_str()) && !shortcode.contains(search.as_str()) {
                continue;
            }

            let description = if shortcode.is_empty() {
                None
            } else {
                Some(format!(":{shortcode}:"))
            };

            out.push(Entry {
                name: format!("{} {name}", emoji.as_str()),
                description,
                icon: None,
                kind: EntryKind::EmojiResult { emoji: emoji.as_str().to_string() },
                priority: 0,
            });
        }
    }

    fn launch(&self, entry: &Entry) {
        if let EntryKind::EmojiResult { emoji } = &entry.kind {
            if let Err(e) = std::process::Command::new("wl-copy").arg(emoji).spawn() {
                tracing::error!("Failed to copy emoji to clipboard: {e}");
            }
        }
    }
}
