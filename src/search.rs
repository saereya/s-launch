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
        self.nucleo.pattern.reparse(
            0,
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            false,
        );
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
                bucket.push(MatchedEntry { entry: entry.clone() });
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
