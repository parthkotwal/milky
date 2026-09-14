//! The long-lived search engine.

use std::path::PathBuf;

use crate::apps;
use crate::normalize_query;
use crate::search::{AppMatch, search_apps};
use crate::api::{Request, Response, SearchResponse, ErrorResponse, ErrorReason, ENCODE_FAILURE, result_item};

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

    /// Handle one serialized request and return a serialized response.
    ///
    /// Returns a `String`, not a `Result`: a malformed request is a normal
    /// error *response*, so a caller across the C boundary has exactly one
    /// thing to check rather than two failure channels.
    pub fn handle_json(&self, request_json: &str) -> String {
        let response = match serde_json::from_str::<Request>(request_json) {
            Ok(Request::Search(req)) => {
                let results = self
                    .search(&req.query, req.limit)
                    .iter()
                    .filter_map(result_item)
                    .collect();
                Response::Search(SearchResponse {
                    query: req.query,
                    results,
                })
            }
            Err(err) => Response::Error(ErrorResponse {
                reason: ErrorReason::BadRequest,
                message: format!("could not parse request: {err}"),
            }),
        };
        serde_json::to_string(&response).unwrap_or_else(|_| ENCODE_FAILURE.to_string())
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

        fn ask(engine: &Engine, request: &str) -> serde_json::Value {
        serde_json::from_str(&engine.handle_json(request))
            .expect("a response must always be valid JSON")
    }

    #[test]
    fn search_round_trips_through_json() {
        let engine = Engine::new();
        let v = ask(&engine, r#"{"op":"search","query":"term","limit":5}"#);
        assert_eq!(v["kind"], "search");
        assert_eq!(v["query"], "term");
        let results = v["results"].as_array().unwrap();
        let terminal = results.iter().find(|r| r["name"] == "Terminal").expect("Terminal");
        assert_eq!(terminal["match_kind"], "prefix");
        assert_eq!(terminal["path"], "/System/Applications/Utilities/Terminal.app");
    }

    #[test]
    fn no_matches_is_an_empty_search_not_an_error() {
        let engine = Engine::new();
        let v = ask(&engine, r#"{"op":"search","query":"zzqqxxnomatch","limit":5}"#);
        assert_eq!(v["kind"], "search");
        assert!(v["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn json_search_respects_the_limit() {
        let engine = Engine::new();
        let v = ask(&engine, r#"{"op":"search","query":"a","limit":2}"#);
        assert!(v["results"].as_array().unwrap().len() <= 2);
    }

    #[test]
    fn malformed_json_is_a_bad_request() {
        let v = ask(&Engine::new(), "definitely not json");
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn unknown_op_is_a_bad_request() {
        let v = ask(&Engine::new(), r#"{"op":"teleport"}"#);
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn missing_limit_is_a_bad_request() {
        let v = ask(&Engine::new(), r#"{"op":"search","query":"term"}"#);
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn error_responses_carry_exactly_one_kind_key() {
        let raw = Engine::new().handle_json("not json");
        assert_eq!(raw.matches(r#""kind""#).count(), 1, "got {raw}");
    }
}