use std::sync::OnceLock;

use super::{Entry, EntryKind, Plugin};

/// One searchable emoji: the emoji itself plus case-folded name and shortcode.
struct IndexedEmoji {
    emoji: &'static emojis::Emoji,
    name_lower: String,
    shortcode_lower: String,
}

/// Case-folded emoji index, built once on first use.
///
/// The bundled Unicode names are mixed-case — 770 of them contain capitals,
/// including every country flag ("flag: Germany"), the zodiac signs and things
/// like "ATM sign". Matching a lowercased query against the raw name therefore
/// missed all of them, and the shortcodes don't cover the gap (flags use ISO
/// codes like `de`, so `:germany` found nothing). Fold both sides, once, rather
/// than allocating ~1900 lowercased strings on every keystroke.
fn index() -> &'static [IndexedEmoji] {
    static INDEX: OnceLock<Vec<IndexedEmoji>> = OnceLock::new();
    INDEX.get_or_init(|| {
        emojis::iter()
            .map(|emoji| IndexedEmoji {
                emoji,
                name_lower: emoji.name().to_lowercase(),
                shortcode_lower: emoji.shortcode().unwrap_or("").to_lowercase(),
            })
            .collect()
    })
}

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

        for indexed in index() {
            if !indexed.name_lower.contains(search.as_str())
                && !indexed.shortcode_lower.contains(search.as_str())
            {
                continue;
            }

            let description = if indexed.shortcode_lower.is_empty() {
                None
            } else {
                Some(format!(":{}:", indexed.shortcode_lower))
            };

            out.push(Entry {
                name: format!("{} {}", indexed.emoji.as_str(), indexed.emoji.name()),
                description,
                icon: None,
                kind: EntryKind::EmojiResult {
                    emoji: indexed.emoji.as_str().to_string(),
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
    fn matches_names_that_contain_capitals() {
        // Regression: the query was lowercased but the Unicode name was not, so
        // every mixed-case name was unreachable. Country flags are the worst
        // case — their shortcodes are ISO codes, so there was no other way in.
        for term in [":germany", ":Germany", ":united kingdom", ":aries", ":atm"] {
            assert!(
                !query(term).is_empty(),
                "'{term}' should match at least one emoji"
            );
        }
    }

    #[test]
    fn capitalised_name_matches_regardless_of_query_case() {
        let lower = query(":germany");
        let mixed = query(":GeRmAnY");
        assert_eq!(lower.len(), mixed.len());
        assert!(lower.iter().any(|e| e.name.contains("Germany")));
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
