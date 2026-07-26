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
                kind: EntryKind::EmojiResult {
                    emoji: emoji.as_str().to_string(),
                },
                priority: 0,
            });
        }
    }

    fn launch(&self, entry: &Entry) {
        if let EntryKind::EmojiResult { emoji } = &entry.kind {
            if let Err(e) = super::detached_command("wl-copy").arg(emoji).spawn() {
                tracing::error!("Failed to copy emoji to clipboard: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(input: &str) -> Vec<Entry> {
        let mut out = Vec::new();
        EmojiPlugin.query(input, &mut out);
        out
    }

    #[test]
    fn empty_search_after_colon_produces_no_results() {
        assert!(query(":").is_empty());
        assert!(query(":   ").is_empty());
    }

    #[test]
    fn matches_by_name_substring() {
        let out = query(":grinning");
        assert!(out.iter().any(|e| e.name.contains("grinning")));
    }

    #[test]
    fn matches_by_shortcode_substring() {
        // "heart" is a shortcode for several heart emoji even when it isn't a
        // substring of the Unicode name itself (e.g. red heart's name is
        // "red heart" so this also covers the name path; assert at least one
        // hit either way).
        let out = query(":heart");
        assert!(!out.is_empty());
    }

    #[test]
    fn search_is_case_insensitive() {
        let lower = query(":grinning");
        let upper = query(":GRINNING");
        assert_eq!(lower.len(), upper.len());
        assert!(!lower.is_empty());
    }

    #[test]
    fn no_match_produces_no_results() {
        assert!(query(":thisisnotarealemojiname12345").is_empty());
    }

    #[test]
    fn result_entry_carries_shortcode_description_and_emoji_kind() {
        let out = query(":grinning");
        let hit = out.iter().find(|e| e.name.contains("grinning")).unwrap();
        assert!(hit
            .description
            .as_deref()
            .is_some_and(|d| d.starts_with(':') && d.ends_with(':')));
        match &hit.kind {
            EntryKind::EmojiResult { emoji } => assert!(!emoji.is_empty()),
            _ => panic!("expected EmojiResult kind"),
        }
    }
}
