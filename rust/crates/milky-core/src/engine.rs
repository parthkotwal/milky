//! The long-lived search engine.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant, SystemTime};

use crate::api::{Outcome, SelectionEvent};
use crate::events::{self, LoggedSelection};
use crate::storage;
use crate::usage::{self, UsageStore};

use crate::api::{
    ENCODE_FAILURE, ErrorReason, ErrorResponse, Request, Response, SearchResponse, result_item,
};
use crate::apps::{self, App};
use crate::candidate::{self, Candidate, Kind};
use crate::normalize_query;
use crate::files;
use crate::files::FileEntry;
use crate::indexer::Indexer;
use crate::places::{self, Place};
use crate::search::{self, Hit};
use crate::settings::{self, SettingsPane};

/// One immutable view of everything searchable.
///
/// Searches share a snapshot instead of locking the engine: a search takes
/// the current `Arc<Index>` and reads it for as long as it likes, while an
/// update builds a whole new `Index` and swaps it in (see
/// [`Engine::publish`]). Nobody ever sees a half-updated index, and a search
/// never waits for an update to finish.
#[derive(Debug)]
pub struct Index {
    candidates: Vec<Candidate>,
    /// Whether the file walk has finished. Until then the index has apps,
    /// places, and settings, but no files.
    pub files_ready: bool,
}

impl Index {
    pub fn new(candidates: Vec<Candidate>, files_ready: bool) -> Self {
        Self { candidates, files_ready }
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// How many candidates of `kind` this snapshot holds.
    pub fn count(&self, kind: Kind) -> usize {
        self.candidates.iter().filter(|candidate| candidate.kind == kind).count()
    }
}

/// The current [`Index`], shared by the engine and its background indexer.
///
/// Both need it, and the indexer thread may outlive any borrow of the engine,
/// so it lives behind its own `Arc` rather than inside `Engine`.
#[derive(Debug)]
pub struct Snapshots {
    /// Read by cloning the `Arc` (a pointer and a counter increment) under a
    /// brief read lock; replaced whole under a brief write lock.
    current: RwLock<Arc<Index>>,
}

impl Snapshots {
    pub fn new(first: Index) -> Self {
        Self { current: RwLock::new(Arc::new(first)) }
    }

    /// The current snapshot.
    ///
    /// Cheap: take the read lock just long enough to clone the `Arc` (which
    /// copies a pointer and bumps a reference count), then let go. The caller
    /// can read the returned snapshot for as long as it likes, even while
    /// [`Snapshots::publish`] replaces it: the old `Index` lives until its
    /// last `Arc` is dropped. A poisoned lock is recovered: a swap is one
    /// assignment, so it cannot be left half-done.
    pub fn current(&self) -> Arc<Index> {
        let guard = self.current.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(&guard)
    }

    /// Make `index` the snapshot every later search sees. The `Arc` is built
    /// before the write lock is taken, and the previous snapshot is dropped
    /// after it is released, so the lock is held only for the swap.
    pub fn publish(&self, index: Index) {
        let fresh = Arc::new(index);
        let mut guard = self.current.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let old = std::mem::replace(&mut *guard, fresh);
        drop(guard);
        drop(old);
    }
}

/// Everything the engine searches besides files, found at startup (~55 ms
/// warm): installed apps, well-known places, System Settings. `home` is
/// `None` on a machine without a home folder, which then has no places and no
/// files.
#[derive(Debug, Clone)]
pub struct Sources {
    pub apps: Vec<App>,
    pub places: Vec<Place>,
    pub panes: Vec<SettingsPane>,
    pub home: Option<PathBuf>,
}

impl Sources {
    /// Every candidate for these sources plus `files`. Walked entries that are
    /// already an app or a place are left out (see [`candidate::build`]).
    pub fn candidates(&self, files: &[FileEntry]) -> Vec<Candidate> {
        candidate::build(
            &self.apps,
            &self.places,
            &self.panes,
            files,
            self.home.as_deref().unwrap_or(Path::new("/")),
        )
    }
}

/// Find apps, places, and System Settings on this Mac.
pub fn discover_sources() -> Sources {
    // The real path, with any symbolic links resolved: file-change events
    // report real paths, and the index must use the same ones to match them.
    let home = std::env::home_dir().map(|home| std::fs::canonicalize(&home).unwrap_or(home));
    Sources {
        apps: apps::discover_apps(),
        places: home.as_deref().map(places::discover_places).unwrap_or_default(),
        panes: settings::discover_settings(),
        home,
    }
}

/// A warm search engine.
///
/// Discovering apps (about 23 ms) and System Settings (about 40 ms) is far too
/// slow to repeat per keystroke, so it happens once and the result is held as
/// an [`Index`] snapshot. Files are walked on a background thread and join
/// the index when ready (`files_ready`), so startup does not wait for them.
/// The only way to change the index is [`Engine::publish`].
pub struct Engine {
    snapshots: Arc<Snapshots>,
    /// The background indexer, stopped when the engine is dropped. An
    /// `Option` so `Drop` can take it out: `stop` consumes it.
    indexer: Option<Indexer>,
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
        Self::with_sources(dir, discover_sources())
    }

    /// An engine over `sources`, keeping its usage history in `dir`. Searches
    /// work at once on everything but files; the indexer adds files.
    pub fn with_sources(dir: &Path, sources: Sources) -> Self {
        let usage_path = dir.join(usage::FILE_NAME);
        let events_path = dir.join(events::FILE_NAME);
        let (store, _damaged) = UsageStore::load_or_quarantine(&usage_path, SystemTime::now());
        let snapshots = Arc::new(Snapshots::new(Index::new(sources.candidates(&[]), false)));
        let indexer = Some(Indexer::start(Arc::clone(&snapshots), sources));
        Self {
            snapshots,
            indexer,
            usage: Mutex::new(store),
            usage_path,
            events_path,
        }
    }

    /// How many apps are currently indexed.
    pub fn app_count(&self) -> usize {
        self.count(Kind::App)
    }

    /// How many well-known places are currently indexed.
    pub fn place_count(&self) -> usize {
        self.count(Kind::Place)
    }

    /// How many files and folders from the file walk are currently indexed.
    pub fn file_count(&self) -> usize {
        self.count(Kind::File) + self.count(Kind::Folder)
    }

    /// How many System Settings panes and sections are currently indexed.
    pub fn settings_count(&self) -> usize {
        self.count(Kind::Setting)
    }

    fn count(&self, kind: Kind) -> usize {
        self.index().count(kind)
    }

    /// The current snapshot: see [`Snapshots::current`].
    pub fn index(&self) -> Arc<Index> {
        self.snapshots.current()
    }

    /// Make `index` the snapshot every later search sees: see
    /// [`Snapshots::publish`].
    pub fn publish(&self, index: Index) {
        self.snapshots.publish(index);
    }

    /// Wait until the background file walk has been published, up to
    /// `timeout`; `true` if it has. For callers that want a complete index
    /// before their first search (the CLI, tests). Checks every few
    /// milliseconds: simple, and the wait is rare.
    pub fn wait_for_files(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.index().files_ready {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Search `index`, best results first. The hits borrow from `index`, so
    /// they stay valid even if a newer snapshot is published meanwhile: take
    /// the snapshot with [`Engine::index`] and keep it while using the hits.
    ///
    /// Takes raw user input and normalizes it, so callers do not have to know
    /// that the matching layer expects a normalized query.
    pub fn search<'a>(&self, index: &'a Index, query: &str, limit: usize) -> Vec<Hit<'a>> {
        let normalized = normalize_query(query);
        let now = SystemTime::now();
        let store = self.lock_usage();
        search::search(index.candidates(), &normalized, limit, |id| {
            store.score(id, now)
        })
    }

    /// Ask the indexer to re-discover apps and walk all files again, as at
    /// startup. Returns at once; the new snapshot is published when ready.
    /// Changes are normally followed live, so this is for recovery and tools.
    pub fn reindex(&self) {
        if let Some(indexer) = &self.indexer {
            indexer.rescan();
        }
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
                let index = self.index();
                let results = self
                    .search(&index, &req.query, req.limit)
                    .iter()
                    .filter_map(result_item)
                    .collect();
                Response::Search(SearchResponse {
                    query: req.query,
                    results,
                    files_ready: index.files_ready,
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

/// Stop the background indexer before the engine goes away and wait for its
/// thread to finish, so no indexing work or file watching outlives the engine.
///
/// `Drop::drop` runs automatically when an `Engine` is dropped; across the C
/// boundary that is `milky_engine_free`. Take the handle out of
/// `self.indexer` (`Option::take` leaves `None` behind; `join` needs to own
/// the handle) and join it. `join` returns `Err` if the thread panicked:
/// ignore that here, since panicking inside `drop` would abort the process.
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(indexer) = self.indexer.take() {
            indexer.stop();
        }
    }
}

/// The user's exclusions (`exclusions.txt`, created with the defaults if
/// missing), or the defaults if that file cannot be read or written: a broken
/// file must not mean indexing everything.
pub fn user_exclusions(home: &Path) -> files::Exclusions {
    files::exclusions_path()
        .and_then(|path| files::load_exclusions(&path, home))
        .map(|(rules, _ignored)| rules)
        .unwrap_or_else(|_| files::Exclusions::defaults(home))
}

/// The file walk with the user's exclusions.
pub fn discover_files(home: &Path) -> files::Walk {
    files::discover_files(home, &user_exclusions(home))
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
        let index = engine.index();
        let found = engine.search(&index, "term", 10);
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
        let index = engine.index();
        let found = engine.search(&index, "  TERM  ", 10);
        assert!(
            found.iter().any(|hit| hit.title == "Terminal"),
            "got {:?}",
            names(&found)
        );
    }

    #[test]
    fn respects_the_limit() {
        let engine = Engine::new();
        assert!(engine.search(&engine.index(), "a", 3).len() <= 3);
    }

    #[test]
    fn reindexing_leaves_the_engine_usable() {
        let engine = Engine::new();
        let before = engine.app_count();
        engine.reindex();
        assert_eq!(engine.app_count(), before);
    }

    fn tiny_index(names: &[&str]) -> Index {
        let apps: Vec<crate::apps::App> = names
            .iter()
            .map(|name| crate::apps::App::new(PathBuf::from(format!("/Applications/{name}.app")), name.to_string()))
            .collect();
        Index::new(candidate::build(&apps, &[], &[], &[], Path::new("/Users/someone")), true)
    }

    #[test]
    fn a_published_index_is_what_later_searches_see() {
        let engine = Engine::with_data_dir(&temp_data_dir("publish"));
        engine.publish(tiny_index(&["Alpha", "Beta"]));
        let index = engine.index();
        assert_eq!(index.count(Kind::App), 2);
        assert_eq!(names(&engine.search(&index, "alp", 10)), vec!["Alpha"]);
    }

    #[test]
    fn a_held_snapshot_survives_a_publish() {
        let engine = Engine::with_data_dir(&temp_data_dir("snapshot"));
        engine.publish(tiny_index(&["Alpha"]));
        let before = engine.index();
        let hits = engine.search(&before, "alpha", 10);
        engine.publish(tiny_index(&["Gamma"]));
        // The old snapshot, and hits borrowed from it, are still intact.
        assert_eq!(names(&hits), vec!["Alpha"]);
        assert_eq!(before.count(Kind::App), 1);
        assert_eq!(names(&engine.search(&engine.index(), "gamma", 10)), vec!["Gamma"]);
        assert!(engine.search(&engine.index(), "alpha", 10).is_empty());
    }

    #[test]
    fn index_hands_out_the_same_snapshot_until_a_publish() {
        let engine = Engine::with_data_dir(&temp_data_dir("same"));
        let a = engine.index();
        let b = engine.index();
        assert!(Arc::ptr_eq(&a, &b), "cloning the Arc, not the index");
        engine.publish(tiny_index(&["Alpha"]));
        assert!(!Arc::ptr_eq(&a, &engine.index()));
    }

    #[test]
    fn searches_on_many_threads_run_while_snapshots_are_published() {
        let engine = Engine::with_data_dir(&temp_data_dir("concurrent-publish"));
        engine.publish(tiny_index(&["Alpha", "Albatross"]));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..500 {
                        let index = engine.index();
                        let found = names(&engine.search(&index, "al", 10));
                        // Every snapshot ever published has both or neither.
                        assert!(found.is_empty() || found.len() == 2, "torn snapshot: {found:?}");
                    }
                });
            }
            for i in 0..200 {
                engine.publish(if i % 2 == 0 { tiny_index(&["Alpha", "Albatross"]) } else { tiny_index(&[]) });
            }
        });
    }

    #[test]
    fn a_panic_while_publishing_does_not_break_search() {
        let engine = Engine::with_data_dir(&temp_data_dir("poisoned-index"));
        engine.publish(tiny_index(&["Alpha"]));
        let crashed = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = engine.snapshots.current.write().unwrap();
                    panic!("simulated crash while holding the index lock");
                })
                .join()
        });
        assert!(crashed.is_err());
        assert!(engine.snapshots.current.is_poisoned());
        assert_eq!(names(&engine.search(&engine.index(), "alpha", 10)), vec!["Alpha"]);
        engine.publish(tiny_index(&["Beta"]));
        assert_eq!(engine.index().count(Kind::App), 1);
    }

    /// A made-up home folder with `files` in it (paths relative to it).
    fn temp_home(label: &str, files: &[&str]) -> PathBuf {
        let home = std::env::temp_dir().join(format!("milky-home-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for file in files {
            let path = home.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        }
        std::fs::create_dir_all(&home).unwrap();
        // The real path (`/var` is a link to `/private/var`): file-change
        // events report real paths, as `discover_sources` does for the home
        // folder.
        std::fs::canonicalize(&home).unwrap()
    }

    /// Wait up to 10 s for a snapshot where `found` holds. File events take
    /// a moment: FSEvents' own delay, then the indexer's batch window.
    fn eventually(engine: &Engine, found: impl Fn(&Engine, &Index) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if found(engine, &engine.index()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn finds(engine: &Engine, index: &Index, query: &str) -> Vec<String> {
        engine.search(index, query, 10).iter().map(|hit| hit.title.to_string()).collect()
    }

    #[test]
    fn a_new_file_is_found_without_a_restart() {
        let home = temp_home("live-create", &["notes/a.txt"]);
        let engine = Engine::with_sources(&temp_data_dir("live-create"), files_only(Some(home.clone())));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        assert!(finds(&engine, &engine.index(), "invoice").is_empty());
        std::fs::write(home.join("notes/invoice-march.pdf"), b"").unwrap();
        assert!(
            eventually(&engine, |engine, index| finds(engine, index, "invoice") == vec!["invoice-march.pdf"]),
            "the new file never appeared"
        );
    }

    #[test]
    fn a_deleted_file_disappears() {
        let home = temp_home("live-delete", &["old-draft.txt", "keep.txt"]);
        let engine = Engine::with_sources(&temp_data_dir("live-delete"), files_only(Some(home.clone())));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        assert_eq!(finds(&engine, &engine.index(), "draft"), vec!["old-draft.txt"]);
        std::fs::remove_file(home.join("old-draft.txt")).unwrap();
        assert!(eventually(&engine, |engine, index| finds(engine, index, "draft").is_empty()));
        assert_eq!(finds(&engine, &engine.index(), "keep"), vec!["keep.txt"]);
    }

    #[test]
    fn a_renamed_file_is_found_by_its_new_name() {
        let home = temp_home("live-rename", &["docs/draft.md"]);
        let engine = Engine::with_sources(&temp_data_dir("live-rename"), files_only(Some(home.clone())));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        std::fs::rename(home.join("docs/draft.md"), home.join("docs/final-report.md")).unwrap();
        assert!(eventually(&engine, |engine, index| {
            finds(engine, index, "final report") == vec!["final-report.md"] && finds(engine, index, "draft").is_empty()
        }));
    }

    #[test]
    fn a_folder_moved_in_arrives_with_its_contents() {
        let home = temp_home("live-move", &["inbox.txt"]);
        let outside = std::env::temp_dir().join(format!("milky-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(outside.join("trip/photos")).unwrap();
        std::fs::write(outside.join("trip/photos/itinerary.pdf"), b"").unwrap();
        let engine = Engine::with_sources(&temp_data_dir("live-move"), files_only(Some(home.clone())));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        std::fs::rename(outside.join("trip"), home.join("trip")).unwrap();
        assert!(
            eventually(&engine, |engine, index| finds(engine, index, "itinerary") == vec!["itinerary.pdf"]),
            "a file inside the moved folder never appeared"
        );
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_deleted_folder_takes_its_files_out_of_search() {
        let home = temp_home("live-rmdir", &["project/readme.md", "project/src/main.rs"]);
        let engine = Engine::with_sources(&temp_data_dir("live-rmdir"), files_only(Some(home.clone())));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        assert_eq!(engine.file_count(), 4);
        std::fs::remove_dir_all(home.join("project")).unwrap();
        assert!(eventually(&engine, |_, index| index.count(Kind::File) + index.count(Kind::Folder) == 0));
    }

    /// Sources with no apps, places, or settings: only files under `home`.
    fn files_only(home: Option<PathBuf>) -> Sources {
        Sources { apps: vec![], places: vec![], panes: vec![], home }
    }

    #[test]
    fn files_arrive_from_the_background_indexer() {
        let home = temp_home("arrive", &["notes/groceries.txt", "taxes.pdf"]);
        let engine = Engine::with_sources(&temp_data_dir("arrive"), files_only(Some(home)));
        assert!(engine.wait_for_files(Duration::from_secs(10)), "files never arrived");
        let index = engine.index();
        assert!(index.files_ready);
        assert_eq!(engine.file_count(), 3, "notes/, groceries.txt, taxes.pdf");
        assert_eq!(names(&engine.search(&index, "groceries", 10)), vec!["groceries.txt"]);
    }

    #[test]
    fn everything_but_files_is_searchable_at_once() {
        let home = temp_home("at-once", &["a.txt"]);
        let sources = Sources {
            apps: vec![App::new(PathBuf::from("/Applications/Alpha.app"), "Alpha".to_string())],
            ..files_only(Some(home))
        };
        let engine = Engine::with_sources(&temp_data_dir("at-once"), sources);
        // No waiting: the first snapshot already has the app.
        assert_eq!(names(&engine.search(&engine.index(), "alpha", 10)), vec!["Alpha"]);
    }

    #[test]
    fn a_snapshot_without_files_ready_has_no_files() {
        let home = temp_home("invariant", &["a.txt", "b.txt"]);
        let engine = Engine::with_sources(&temp_data_dir("invariant"), files_only(Some(home)));
        let index = engine.index();
        if !index.files_ready {
            assert_eq!(index.count(Kind::File) + index.count(Kind::Folder), 0);
        }
    }

    #[test]
    fn without_a_home_folder_files_are_ready_and_empty() {
        let engine = Engine::with_sources(&temp_data_dir("no-home"), files_only(None));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        assert_eq!(engine.file_count(), 0);
    }

    #[test]
    fn dropping_the_engine_waits_for_its_indexer() {
        let home = temp_home("drop", &["a.txt"]);
        let engine = Engine::with_sources(&temp_data_dir("drop"), files_only(Some(home)));
        let snapshots = Arc::clone(&engine.snapshots);
        drop(engine);
        // Only this test's handle is left: the indexer thread has finished and
        // released its own.
        assert_eq!(Arc::strong_count(&snapshots), 1);
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
        assert_eq!(
            terminal["id"],
            "app:/System/Applications/Utilities/Terminal.app"
        );
        assert_eq!(terminal["kind"], "app");
        assert_eq!(terminal["subtitle"], "Utilities");
        assert_eq!(terminal["action"]["type"], "launch");
        assert_eq!(
            terminal["action"]["path"],
            "/System/Applications/Utilities/Terminal.app"
        );
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
        assert_eq!(engine.usage_score(SAFARI, SystemTime::now()), 0.0);
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
                .search(&engine.index(), "c", 50)
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
        assert!(
            engine.settings_count() > 500,
            "only {} settings",
            engine.settings_count()
        );
        let v = ask(
            &engine,
            r#"{"op":"search","query":"night shift","limit":5}"#,
        );
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
