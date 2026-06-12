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

    /// Return matched entries in ranked order (best first), limited to `limit`.
    pub fn results(&mut self, limit: usize) -> Vec<MatchedEntry> {
        // Ensure any in-flight work is done before snapshotting
        self.nucleo.tick(5);
        let snapshot = self.nucleo.snapshot();
        snapshot
            .matched_items(..snapshot.matched_item_count().min(limit as u32))
            .map(|item| {
                let idx = *item.data;
                MatchedEntry {
                    entry: self.entries[idx].clone(),
                }
            })
            .collect()
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
