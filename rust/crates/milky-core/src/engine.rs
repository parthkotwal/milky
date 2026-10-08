//! The long-lived search engine.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use crate::api::{Outcome, SelectionEvent};
use crate::events::{self, LoggedSelection};
use crate::storage;
use crate::usage::{self, UsageStore};

use crate::api::{
    ENCODE_FAILURE, ErrorReason, ErrorResponse, Request, Response, SearchResponse, result_item,
};
use crate::apps;
use crate::normalize_query;
use crate::candidate::{self, Candidate, Kind};
use crate::search::{self, Hit};
use crate::settings;

/// A warm search engine.
///
/// Discovering apps (about 23 ms) and System Settings (about 40 ms) is far too
/// slow to repeat per keystroke, so it happens once here and the candidates are
/// held. The field is private: the only way to change the index is
/// [`Engine::reindex`].
pub struct Engine {
    candidates: Vec<Candidate>,
    usage: Mutex<UsageStore>,
    usage_path: PathBuf,
    events_path: PathBuf,
}

// `Default` implies a cheap, obvious value, and `new` does about 60 ms of disk
// I/O. It is also heading for `new(config) -> Result<Self, _>`, at which point a
// `Default` impl could not exist. Not worth adding to delete.
#[allow(clippy::new_without_default)]
impl Engine {
    /// Build an engine, discovering installed apps and System Settings once.
    pub fn new() -> Self {
        let dir = storage::data_dir().unwrap_or_else(|_| std::env::temp_dir().join("Milky"));
        Self::with_data_dir(&dir)
    }

    pub fn with_data_dir(dir: &Path) -> Self {
        let usage_path = dir.join(usage::FILE_NAME);
        let events_path = dir.join(events::FILE_NAME);
        let (store, _damaged) = UsageStore::load_or_quarantine(&usage_path, SystemTime::now());
        Self {
            candidates: discover_candidates(),
            usage: Mutex::new(store),
            usage_path,
            events_path,
        }
    }

    /// How many apps are currently indexed.
    pub fn app_count(&self) -> usize {
        self.count(Kind::App)
    }

    /// How many System Settings panes and sections are currently indexed.
    pub fn settings_count(&self) -> usize {
        self.count(Kind::Setting)
    }

    fn count(&self, kind: Kind) -> usize {
        self.candidates.iter().filter(|candidate| candidate.kind == kind).count()
    }

    /// Search the index, best results first.
    ///
    /// Takes raw user input and normalizes it, so callers do not have to know
    /// that the matching layer expects a normalized query.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit<'_>> {
        let normalized = normalize_query(query);
        let now = SystemTime::now();
        let store = self.lock_usage();
        search::search(&self.candidates, &normalized, limit, |id| store.score(id, now))
    }

    /// Re-discover apps and System Settings, replacing the index.
    ///
    /// Takes `&mut self` because it replaces state no reader may be holding.
    pub fn reindex(&mut self) {
        self.candidates = discover_candidates();
    }

    fn lock_usage(&self) -> MutexGuard<'_, UsageStore> {
        self.usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Current usage score for the result `id`.
    pub fn usage_score(&self, id: &str, now: SystemTime) -> f64 {
        self.lock_usage().score(id, now)
    }

    fn record_selection(&self, event: SelectionEvent, now: SystemTime) -> Response {
        if let Err(message) = event.validate() {
            return Response::Error(ErrorResponse {
                reason: ErrorReason::BadRequest,
                message,
            });
        }
        let mut store = self.lock_usage();
        if let Err(err) = events::append(&self.events_path, &LoggedSelection::new(&event, now)) {
            return Response::Error(ErrorResponse {
                reason: ErrorReason::Internal,
                message: format!("could not record selection: {err}"),
            });
        }
        if event.outcome == Outcome::Opened {
            store.record_launch(&event.selected, now);
        }
        if let Err(err) = store.save(&self.usage_path) {
            return Response::Error(ErrorResponse {
                reason: ErrorReason::Internal,
                message: format!("could not save usage history: {err}"),
            });
        }
        Response::Recorded
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
            Ok(Request::RecordSelection(event)) => self.record_selection(event, SystemTime::now()),
            Err(err) => Response::Error(ErrorResponse {
                reason: ErrorReason::BadRequest,
                message: format!("could not parse request: {err}"),
            }),
        };
        serde_json::to_string(&response).unwrap_or_else(|_| ENCODE_FAILURE.to_string())
    }
}

/// Every destination search can return: installed apps, then System Settings
/// panes and sections.
fn discover_candidates() -> Vec<Candidate> {
    candidate::build(&apps::discover_apps(), &settings::discover_settings())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(hits: &[Hit<'a>]) -> Vec<&'a str> {
        hits.iter().map(|hit| hit.title).collect()
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
            found.iter().any(|hit| hit.title == "Terminal"),
            "got {:?}",
            names(&found)
        );
    }

    #[test]
    fn engine_normalizes_raw_user_input() {
        // Capitals and stray whitespace, as actually typed. `search::search` alone
        // would return nothing for this.
        let engine = Engine::new();
        let found = engine.search("  TERM  ", 10);
        assert!(
            found.iter().any(|hit| hit.title == "Terminal"),
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
        let terminal = results
            .iter()
            .find(|r| r["title"] == "Terminal")
            .expect("Terminal");
        assert_eq!(terminal["match_kind"], "prefix");
        assert_eq!(terminal["id"], "app:/System/Applications/Utilities/Terminal.app");
        assert_eq!(terminal["kind"], "app");
        assert_eq!(terminal["subtitle"], "Utilities");
        assert_eq!(terminal["action"]["type"], "launch");
        assert_eq!(terminal["action"]["path"], "/System/Applications/Utilities/Terminal.app");
    }

    #[test]
    fn no_matches_is_an_empty_search_not_an_error() {
        let engine = Engine::new();
        let v = ask(
            &engine,
            r#"{"op":"search","query":"zzqqxxnomatch","limit":5}"#,
        );
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

    const SAFARI: &str = "app:/Applications/Safari.app";

    fn temp_data_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("milky-engine-{label}-{}", std::process::id()))
    }

    fn selection_json(outcome: &str) -> String {
        format!(
            r#"{{"op":"record_selection","query":"saf","shown":["{SAFARI}","app:/System/Applications/Calendar.app"],"selected":"{SAFARI}","outcome":"{outcome}"}}"#
        )
    }

    fn log_lines(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join(events::FILE_NAME))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    #[test]
    fn an_opened_selection_counts_as_a_launch() {
        let dir = temp_data_dir("opened");
        let engine = Engine::with_data_dir(&dir);
        assert_eq!(ask(&engine, &selection_json("opened"))["kind"], "recorded");
        assert!(engine.usage_score(SAFARI, SystemTime::now()) > 0.99);
        assert_eq!(log_lines(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_selection_is_logged_but_not_counted() {
        let dir = temp_data_dir("failed");
        let engine = Engine::with_data_dir(&dir);
        assert_eq!(ask(&engine, &selection_json("failed"))["kind"], "recorded");
        assert_eq!(
            engine.usage_score(SAFARI, SystemTime::now()),
            0.0
        );
        let lines = log_lines(&dir);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(r#""outcome":"failed""#), "{}", lines[0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn history_survives_a_restart() {
        let dir = temp_data_dir("restart");
        {
            let engine = Engine::with_data_dir(&dir);
            assert_eq!(ask(&engine, &selection_json("opened"))["kind"], "recorded");
        }
        let reopened = Engine::with_data_dir(&dir);
        assert!(reopened.usage_score(SAFARI, SystemTime::now()) > 0.99);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_inconsistent_selection_stores_nothing() {
        let dir = temp_data_dir("inconsistent");
        let engine = Engine::with_data_dir(&dir);
        let v = ask(
            &engine,
            r#"{"op":"record_selection","query":"x","shown":["app:/A.app"],"selected":"app:/B.app","outcome":"opened"}"#,
        );
        assert_eq!(v["reason"], "bad_request");
        assert!(!dir.join(usage::FILE_NAME).exists());
        assert!(!dir.join(events::FILE_NAME).exists());
    }

    #[test]
    fn concurrent_recordings_all_land() {
        let dir = temp_data_dir("concurrent");
        let engine = Engine::with_data_dir(&dir);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10 {
                        assert_eq!(ask(&engine, &selection_json("opened"))["kind"], "recorded");
                    }
                });
            }
        });
        assert_eq!(log_lines(&dir).len(), 40);
        let score = engine.usage_score(SAFARI, SystemTime::now());
        assert!(score > 39.9 && score <= 40.0, "score {score}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_poisoned_lock_does_not_stop_recording() {
        let dir = temp_data_dir("poison");
        let engine = Engine::with_data_dir(&dir);
        let crashed = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = engine.usage.lock().unwrap();
                    panic!("simulated bug while holding the usage lock");
                })
                .join()
        });
        assert!(crashed.is_err());
        assert!(engine.usage.is_poisoned());
        assert_eq!(ask(&engine, &selection_json("opened"))["kind"], "recorded");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn usage_reorders_real_results() {
        let dir = temp_data_dir("rerank");
        let engine = Engine::with_data_dir(&dir);
        let position = |name: &str| {
            engine
                .search("c", 50)
                .iter()
                .position(|hit| hit.title == name)
                .expect(name)
        };
        // With no history, the shorter name wins the tie.
        assert!(position("Chess") < position("Calendar"));

        let pick_calendar = r#"{"op":"record_selection","query":"c",
            "shown":["app:/System/Applications/Chess.app","app:/System/Applications/Calendar.app"],
            "selected":"app:/System/Applications/Calendar.app","outcome":"opened"}"#;
        assert_eq!(ask(&engine, pick_calendar)["kind"], "recorded");

        assert!(position("Calendar") < position("Chess"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_are_indexed_and_searchable() {
        let engine = Engine::new();
        assert!(engine.settings_count() > 500, "only {} settings", engine.settings_count());
        let v = ask(&engine, r#"{"op":"search","query":"night shift","limit":5}"#);
        let night_shift = v["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "settings:com.apple.Displays-Settings.extension#nightShiftSection")
            .expect("night shift result");
        assert_eq!(night_shift["kind"], "setting");
        assert_eq!(night_shift["subtitle"], "Displays");
        assert_eq!(night_shift["action"]["type"], "open_url");
        assert_eq!(
            night_shift["action"]["url"],
            "x-apple.systempreferences:com.apple.Displays-Settings.extension?nightShiftSection"
        );
    }

    #[test]
    fn a_settings_selection_counts_by_its_id() {
        let dir = temp_data_dir("settings-selection");
        let engine = Engine::with_data_dir(&dir);
        let id = "settings:com.apple.Displays-Settings.extension#nightShiftSection";
        let event = format!(
            r#"{{"op":"record_selection","query":"night","shown":["{id}"],"selected":"{id}","outcome":"opened"}}"#
        );
        assert_eq!(ask(&engine, &event)["kind"], "recorded");
        assert!(engine.usage_score(id, SystemTime::now()) > 0.99);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
