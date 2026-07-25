use std::sync::Arc;

use nucleo::{
    pattern::{CaseMatching, Normalization},
    Config, Nucleo,
};

use crate::plugins::Entry;

/// Wraps a nucleo matcher that operates on a cloned Vec of all entries.
/// Re-usable across queries: call `update_pattern` then `results`.
pub struct Searcher {
    nucleo: Nucleo<usize>, // stores entry index into the master list
    entries: Vec<Entry>,
}

impl Searcher {
    pub fn new(entries: Vec<Entry>) -> Self {
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let nucleo: Nucleo<usize> = Nucleo::new(
            Config::DEFAULT,
            notify,
            None, // use default thread count
            1,    // one column: the display name
        );

        {
            let injector = nucleo.injector();
            for (i, _entry) in entries.iter().enumerate() {
                injector.push(i, |idx, cols| {
                    cols[0] = entries[*idx].name.as_str().into();
                });
            }
        }

        Self { nucleo, entries }
    }

    /// Update the search pattern and tick until the matcher is settled.
    pub fn update_pattern(&mut self, query: &str) {
        self.nucleo
            .pattern
            .reparse(0, query, CaseMatching::Ignore, Normalization::Smart, false);
        // Tick up to 20 ms to let parallel workers finish
        self.nucleo.tick(20);
    }

    /// Return matched entries limited to `limit`, with higher-priority plugins
    /// filling slots before lower-priority ones regardless of fuzzy score.
    ///
    /// Nucleo ranks all entries by score together, so a highly-scored command
    /// would otherwise push a lower-scored app out of the visible window. Instead
    /// we scan all matches, bucket by plugin priority, and fill the result list
    /// from the highest-priority bucket first.
    pub fn results(&mut self, limit: usize) -> Vec<MatchedEntry> {
        self.nucleo.tick(5);
        let snapshot = self.nucleo.snapshot();
        let total = snapshot.matched_item_count();

        // Collect up to `limit` best matches per priority bucket (score order).
        let mut buckets: std::collections::BTreeMap<u8, Vec<MatchedEntry>> =
            std::collections::BTreeMap::new();
        for item in snapshot.matched_items(..total) {
            let idx = *item.data;
            let entry = &self.entries[idx];
            let bucket = buckets.entry(entry.priority).or_default();
            if bucket.len() < limit {
                bucket.push(MatchedEntry {
                    entry: entry.clone(),
                });
            }
        }

        // Fill result list from highest-priority (lowest number) bucket first.
        let mut results = Vec::with_capacity(limit);
        for bucket in buckets.values() {
            let remaining = limit.saturating_sub(results.len());
            if remaining == 0 {
                break;
            }
            results.extend_from_slice(&bucket[..bucket.len().min(remaining)]);
        }
        results
    }

    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Replace the entire entry list (after a rescan / plugin reload).
    pub fn reload(&mut self, entries: Vec<Entry>) {
        *self = Self::new(entries);
    }
}

#[derive(Debug, Clone)]
pub struct MatchedEntry {
    pub entry: Entry,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::EntryKind;

    fn entry(name: &str, priority: u8) -> Entry {
        Entry {
            name: name.to_string(),
            description: None,
            icon: None,
            kind: EntryKind::Command { path: name.into() },
            priority,
        }
    }

    /// Query and settle the matcher; nucleo's worker threads are async, so a
    /// freshly-updated pattern needs a moment before results() reflects it.
    fn search(entries: Vec<Entry>, query: &str, limit: usize) -> Vec<MatchedEntry> {
        let mut searcher = Searcher::new(entries);
        searcher.update_pattern(query);
        std::thread::sleep(std::time::Duration::from_millis(50));
        searcher.results(limit)
    }

    #[test]
    fn empty_query_returns_all_entries_up_to_limit() {
        let entries = vec![entry("firefox", 0), entry("alacritty", 0), entry("htop", 0)];
        let results = search(entries, "", 10);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn empty_query_respects_limit() {
        let entries = vec![entry("firefox", 0), entry("alacritty", 0), entry("htop", 0)];
        let results = search(entries, "", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn no_match_returns_empty() {
        let entries = vec![entry("firefox", 0), entry("alacritty", 0)];
        let results = search(entries, "zzzzzznomatch", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn fuzzy_match_finds_subsequence() {
        let entries = vec![entry("firefox", 0), entry("alacritty", 0)];
        let results = search(entries, "ffx", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "firefox");
    }

    #[test]
    fn higher_priority_bucket_fills_before_lower_priority_bucket() {
        // A perfectly-scoring low-priority (numerically larger) match must not
        // crowd out a weaker-scoring but higher-priority (numerically smaller)
        // match — this is the whole point of Searcher::results' bucketing.
        let entries = vec![
            entry("zzz-low-priority-exact", 1), // priority bucket 1 (e.g. "commands")
            entry("zzz-high-priority-fuzzy", 0), // priority bucket 0 (e.g. "apps")
        ];
        let results = search(entries, "zzz", 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "zzz-high-priority-fuzzy");
    }

    #[test]
    fn fills_remaining_slots_from_lower_priority_bucket_once_higher_is_exhausted() {
        let entries = vec![
            entry("app-one", 0),
            entry("app-two", 0),
            entry("cmd-one", 1),
            entry("cmd-two", 1),
        ];
        let results = search(entries, "", 3);
        assert_eq!(results.len(), 3);
        // Both priority-0 entries appear before any priority-1 entry.
        let names: Vec<&str> = results.iter().map(|m| m.entry.name.as_str()).collect();
        assert_eq!(&names[..2], &["app-one", "app-two"]);
        assert_eq!(names[2], "cmd-one");
    }

    #[test]
    fn reload_replaces_entries_entirely() {
        let mut searcher = Searcher::new(vec![entry("firefox", 0)]);
        searcher.reload(vec![entry("chromium", 0)]);
        searcher.update_pattern("");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let results = searcher.results(10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "chromium");
    }
}
