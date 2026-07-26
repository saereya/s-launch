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
    notify: Arc<dyn Fn() + Send + Sync>,
    /// How many distinct `Entry::priority` values the entry list contains, i.e.
    /// how many buckets `results` can possibly fill. Lets it stop scanning once
    /// they're all full instead of walking every match.
    distinct_priorities: usize,
}

impl Searcher {
    /// `notify` is invoked from a matcher worker thread whenever new results
    /// have become available and `results()` should be polled again.
    ///
    /// Passing it is not optional for correctness: nucleo matches on background
    /// threads, so a `tick` can return before the workers have finished and
    /// `results()` will then hand back a *partial* match set. This callback is
    /// the only signal that the rest has landed — without it a caller silently
    /// renders truncated results and never corrects them.
    pub fn new(entries: Vec<Entry>, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        let nucleo: Nucleo<usize> = Nucleo::new(
            Config::DEFAULT,
            notify.clone(),
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

        let distinct_priorities = entries
            .iter()
            .map(|e| e.priority)
            .collect::<std::collections::BTreeSet<_>>()
            .len();

        Self {
            nucleo,
            entries,
            notify,
            distinct_priorities,
        }
    }

    /// Update the search pattern. Does not wait for the matcher to settle —
    /// take whatever `results()` has now and let `notify` drive a re-poll when
    /// the workers finish, rather than blocking the UI thread on every
    /// keystroke.
    pub fn update_pattern(&mut self, query: &str) {
        self.nucleo
            .pattern
            .reparse(0, query, CaseMatching::Ignore, Normalization::Smart, false);
        self.nucleo.tick(1);
    }

    /// Return matched entries limited to `limit`, with higher-priority plugins
    /// filling slots before lower-priority ones regardless of fuzzy score.
    ///
    /// Nucleo ranks all entries by score together, so a highly-scored command
    /// would otherwise push a lower-scored app out of the visible window. Instead
    /// we scan all matches, bucket by plugin priority, and fill the result list
    /// from the highest-priority bucket first.
    pub fn results(&mut self, limit: usize) -> Vec<MatchedEntry> {
        self.nucleo.tick(1);
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
            // Once every priority level has `limit` candidates, any further match
            // lands in an already-full bucket and cannot change the output — so
            // stop rather than walking all matches (2000+ on an empty query).
            if buckets.len() == self.distinct_priorities
                && buckets.values().all(|b| b.len() >= limit)
            {
                break;
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

    /// Whether the matcher still has work in flight, i.e. `results()` may still
    /// be incomplete. The UI does not poll this — it re-renders when `notify`
    /// fires — but callers that need the settled set (tests, batch use) can.
    #[allow(dead_code)]
    pub fn is_matching(&mut self) -> bool {
        self.nucleo.tick(1).running
    }

    /// Replace the entire entry list (after a rescan / plugin reload), keeping
    /// the existing notify callback wired up.
    pub fn reload(&mut self, entries: Vec<Entry>) {
        let notify = self.notify.clone();
        *self = Self::new(entries, notify);
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

    fn searcher(entries: Vec<Entry>) -> Searcher {
        Searcher::new(entries, Arc::new(|| {}))
    }

    /// Query and settle the matcher. nucleo's workers are async, so `results()`
    /// straight after `update_pattern` may be partial — wait for the matcher to
    /// report itself idle instead of assuming a fixed delay.
    fn search(entries: Vec<Entry>, query: &str, limit: usize) -> Vec<MatchedEntry> {
        let mut searcher = searcher(entries);
        searcher.update_pattern(query);
        settle(&mut searcher, limit)
    }

    /// Deadline-capped so a regression fails the test rather than hanging it.
    fn settle(searcher: &mut Searcher, limit: usize) -> Vec<MatchedEntry> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while searcher.is_matching() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
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
        let mut searcher = searcher(vec![entry("firefox", 0)]);
        searcher.reload(vec![entry("chromium", 0)]);
        searcher.update_pattern("");
        let results = settle(&mut searcher, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "chromium");
    }

    #[test]
    fn reload_keeps_the_notify_callback_wired_up() {
        // A rescan rebuilds the whole matcher; if reload dropped the callback,
        // async results after a rescan would never reach the UI again.
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woken.clone();
        let mut searcher = Searcher::new(
            vec![entry("firefox", 0)],
            Arc::new(move || {
                w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        );
        searcher.reload(large_entry_set());
        searcher.update_pattern("entry-4242");
        settle(&mut searcher, 10);
        assert!(
            woken.load(std::sync::atomic::Ordering::Relaxed) > 0,
            "notify never fired after reload"
        );
    }

    #[test]
    fn large_entry_set_results_are_not_silently_truncated() {
        // Regression: `update_pattern` used to block for 20ms and `results` for
        // a further 5ms, then return whatever the workers had managed — with no
        // way for the caller to learn that more was coming. Enough entries to
        // guarantee matching outlives a single tick, then assert the full set
        // arrives once the matcher settles.
        let mut searcher = searcher(large_entry_set());
        searcher.update_pattern("entry-4242");
        let results = settle(&mut searcher, 10);
        // Fuzzy matching is by subsequence, so plenty of names match and the
        // limit fills up. What matters is that the set is complete enough for
        // the exact match to be in it — before the notify wiring, a single tick
        // could return with nothing at this entry count.
        assert_eq!(results.len(), 10, "results truncated below the limit");
        assert!(
            results.iter().any(|m| m.entry.name == "entry-4242"),
            "exact match missing from settled results"
        );
    }

    fn large_entry_set() -> Vec<Entry> {
        (0..50_000)
            .map(|i| entry(&format!("entry-{i}"), 0))
            .collect()
    }
}
