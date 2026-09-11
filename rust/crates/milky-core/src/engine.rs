//! The long-lived search engine.

use std::path::PathBuf;

use crate::apps;
use crate::normalize_query;
use crate::search::{AppMatch, search_apps};

/// A warm search engine.
///
/// Scanning for apps costs about 1.6 ms, far too much per keystroke, so the
/// scan happens once here and the result is held. The field is private: the
/// only way to change the index is [`Engine::reindex`].
pub struct Engine {
    apps: Vec<PathBuf>,
}

// `Default` implies a cheap, obvious value, and `new` does about 1.6 ms of disk
// I/O. It is also heading for `new(config) -> Result<Self, _>`, at which point a
// `Default` impl could not exist. Not worth adding to delete.
#[allow(clippy::new_without_default)]
impl Engine {
    /// Build an engine, scanning for installed apps once.
    pub fn new() -> Self {
        Self { apps: apps::discover(), }
    }

    /// How many apps are currently indexed.
    pub fn app_count(&self) -> usize {
        self.apps.len()
    }

    /// Search the index, best results first.
    ///
    /// Takes raw user input and normalizes it, so callers do not have to know
    /// that the matching layer expects a normalized query.
    pub fn search(&self, query: &str, limit: usize) -> Vec<AppMatch> {
        let normalized = normalize_query(query);
        search_apps(&self.apps, &normalized, limit)
    }

    /// Re-scan for installed apps, replacing the index.
    ///
    /// Takes `&mut self` because it replaces state no reader may be holding.
    pub fn reindex(&mut self) {
        self.apps = apps::discover();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(matches: &[AppMatch]) -> Vec<&str> {
        matches.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn new_indexes_the_machines_apps() {
        let engine = Engine::new();
        assert!(engine.app_count() > 0, "expected to find installed apps");
    }

    #[test]
    fn finds_terminal_by_prefix() {
        let engine = Engine::new();
        let found = engine.search("term", 10);
        assert!(
            found.iter().any(|m| m.name == "Terminal"),
            "got {:?}",
            names(&found)
        );
    }

    #[test]
    fn engine_normalizes_raw_user_input() {
        // Capitals and stray whitespace, as actually typed. `search_apps` alone
        // would return nothing for this.
        let engine = Engine::new();
        let found = engine.search("  TERM  ", 10);
        assert!(
            found.iter().any(|m| m.name == "Terminal"),
            "got {:?}",
            names(&found)
        );
    }

    #[test]
    fn respects_the_limit() {
        let engine = Engine::new();
        assert!(engine.search("a", 3).len() <= 3);
    }

    #[test]
    fn reindexing_leaves_the_engine_usable() {
        let mut engine = Engine::new();
        let before = engine.app_count();
        engine.reindex();
        assert_eq!(engine.app_count(), before);
    }
}