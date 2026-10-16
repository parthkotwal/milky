//! The long-lived search engine.

use std::path::{Path, PathBuf};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant, SystemTime};

use crate::activity::FolderActivity;
use crate::api::{FocusRequest, Outcome, SelectionEvent, SessionEvent, SessionReport};
use crate::events;
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
use crate::store::{ServedResult, ServedSearch, Store};
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
    /// The event store; `None` if it could not be opened, in which case
    /// searching still works and recording requests answer with an error.
    /// Shared with the indexer, which records folder activity.
    store: Arc<Mutex<Option<Store>>>,
    /// How busy each folder has been lately, counted by the indexer and read
    /// into the features of served files.
    activity: Arc<Mutex<FolderActivity>>,
    /// Recently served searches, kept until a session says they were drawn.
    recent: Mutex<VecDeque<ServedSearch>>,
    /// Makes search IDs unique across runs: this engine's start time, then a
    /// counter.
    started_ms: u64,
    next_search: AtomicU64,
}

/// Environment variable that moves the engine's history (usage, event store)
/// to another folder.
pub const DATA_DIR_VARIABLE: &str = "MILKY_DATA_DIR";

/// How many candidates beyond the visible results each search remembers for
/// learning: the alternatives a learned ranker chooses between.
pub const POOL_SIZE: usize = 50;

/// How many served searches are remembered until a session reports them.
/// A session draws a few per keystroke; this covers many sessions.
const RECENT_SEARCHES: usize = 512;

fn now_ms(now: SystemTime) -> u64 {
    now.duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

// `Default` implies a cheap, obvious value, and `new` does about 60 ms of disk
// I/O. It is also heading for `new(config) -> Result<Self, _>`, at which point a
// `Default` impl could not exist. Not worth adding to delete.
#[allow(clippy::new_without_default)]
impl Engine {
    /// Build an engine, discovering installed apps and System Settings once.
    ///
    /// History lives in `~/Library/Application Support/Milky`, or in
    /// `$MILKY_DATA_DIR` if set: tests and experiments must never write to the
    /// real store. The launcher never sets it.
    pub fn new() -> Self {
        let dir = std::env::var_os(DATA_DIR_VARIABLE)
            .map(PathBuf::from)
            .or_else(|| storage::data_dir().ok())
            .unwrap_or_else(|| std::env::temp_dir().join("Milky"));
        Self::with_data_dir(&dir)
    }

    pub fn with_data_dir(dir: &Path) -> Self {
        Self::with_sources(dir, discover_sources())
    }

    /// An engine over `sources`, keeping its usage history in `dir`. Searches
    /// work at once on everything but files; the indexer adds files.
    pub fn with_sources(dir: &Path, sources: Sources) -> Self {
        let usage_path = dir.join(usage::FILE_NAME);
        let (usage, _damaged) = UsageStore::load_or_quarantine(&usage_path, SystemTime::now());
        let store = Arc::new(Mutex::new(open_store(dir)));
        let activity = Arc::new(Mutex::new(FolderActivity::default()));
        let snapshots = Arc::new(Snapshots::new(Index::new(sources.candidates(&[]), false)));
        let indexer = Some(Indexer::start(
            Arc::clone(&snapshots),
            sources,
            Arc::clone(&store),
            Arc::clone(&activity),
        ));
        Self {
            snapshots,
            indexer,
            usage: Mutex::new(usage),
            usage_path,
            store,
            activity,
            recent: Mutex::new(VecDeque::with_capacity(RECENT_SEARCHES)),
            started_ms: now_ms(SystemTime::now()),
            next_search: AtomicU64::new(0),
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

    /// Search and remember what was served (results, pool, features) under a
    /// new search ID, so a later `record_events` can store what was drawn.
    fn serve(&self, query: &str, limit: usize) -> (String, Vec<crate::api::ResultItem>, bool) {
        let started = Instant::now();
        let index = self.index();
        let now = SystemTime::now();
        let served = {
            let store = self.lock_usage();
            search::search_with_pool(index.candidates(), &normalize_query(query), limit, POOL_SIZE, |id| {
                store.score(id, now)
            })
        };
        let results = served.shown.iter().filter_map(result_item).collect();
        let latency_us = started.elapsed().as_micros() as u64;
        let search_id = format!("{:x}-{:x}", self.started_ms, self.next_search.fetch_add(1, Ordering::Relaxed));
        let activity = self.activity.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = |hits: &[Hit]| -> Vec<ServedResult> {
            hits.iter()
                .map(|hit| ServedResult {
                    result_id: hit.candidate.id.clone(),
                    kind: hit.candidate.kind,
                    features: search::features(hit, now, &activity),
                })
                .collect()
        };
        let remembered = ServedSearch {
            search_id: search_id.clone(),
            served_at_ms: now_ms(now),
            query: query.to_string(),
            latency_us,
            files_ready: index.files_ready,
            ranker_version: search::RANKER_VERSION,
            shown: record(&served.shown),
            pool: record(&served.pool),
        };
        drop(activity);
        let mut recent = self.recent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if recent.len() == RECENT_SEARCHES {
            recent.pop_front();
        }
        recent.push_back(remembered);
        (search_id, results, index.files_ready)
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

    /// Whether recording is paused: nothing about use is stored, usage
    /// scores included.
    fn paused(&self) -> bool {
        let store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        store.as_ref().is_some_and(|store| store.paused().unwrap_or(false))
    }

    /// Pause or resume recording.
    pub fn set_paused(&self, paused: bool) -> Result<(), String> {
        let mut store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = store.as_mut().ok_or("the event store could not be opened")?;
        store.set_paused(paused).map_err(|err| err.to_string())
    }

    /// Write a consistent copy of the event store to `path`.
    pub fn export_history(&self, path: &Path) -> Result<(), String> {
        let store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = store.as_ref().ok_or("the event store could not be opened")?;
        store.export_to(path).map_err(|err| err.to_string())
    }

    /// Delete recorded history, everything or since `since`, and with all of
    /// it the usage scores ranking learned from it.
    pub fn forget_history(&self, since: Option<SystemTime>) -> Result<(), String> {
        {
            let mut store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let store = store.as_mut().ok_or("the event store could not be opened")?;
            store.forget(since.map(now_ms)).map_err(|err| err.to_string())?;
        }
        if since.is_none() {
            let mut usage = self.lock_usage();
            *usage = UsageStore::new();
            usage.save(&self.usage_path).map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    fn record_selection(&self, event: SelectionEvent, now: SystemTime) -> Response {
        if let Err(message) = event.validate() {
            return bad_request(message);
        }
        if self.paused() {
            return Response::Recorded;
        }
        if let Err(response) = self.with_store(|store| store.record_selection(now_ms(now), &event)) {
            return response;
        }
        if event.outcome == Outcome::Opened {
            return self.record_uses(&[event.selected.as_str()], now);
        }
        Response::Recorded
    }

    /// Store one session and the searches it drew, then count what the user
    /// opened, revealed, or copied as use.
    fn record_events(&self, report: SessionReport, now: SystemTime) -> Response {
        if let Err(message) = report.validate() {
            return bad_request(message);
        }
        if self.paused() {
            return Response::Recorded;
        }
        let drawn = self.drawn_searches(&report);
        let pairs: Vec<(&ServedSearch, bool)> = drawn.iter().map(|(search, decisive)| (search, *decisive)).collect();
        if let Err(response) = self.with_store(|store| store.record_session(&report, &pairs, now_ms(now))) {
            return response;
        }
        let used: Vec<&str> = report
            .events
            .iter()
            .filter_map(|event| match event {
                SessionEvent::Action { action, outcome: Outcome::Opened, result_id, .. } if action.counts_as_use() => {
                    Some(result_id.as_str())
                }
                _ => None,
            })
            .collect();
        if used.is_empty() {
            return Response::Recorded;
        }
        self.record_uses(&used, now)
    }

    /// Store a batch of app focus intervals.
    fn record_focus(&self, request: FocusRequest, now: SystemTime) -> Response {
        if let Err(message) = request.validate() {
            return bad_request(message);
        }
        if self.paused() {
            return Response::Recorded;
        }
        match self.with_store(|store| store.record_focus(&request, now_ms(now))) {
            Ok(()) => Response::Recorded,
            Err(response) => response,
        }
    }

    /// The remembered searches a session drew, each with whether it was
    /// decisive: the search of the last action, or else the last one drawn.
    /// Searches no longer remembered (served too long ago) are left out.
    fn drawn_searches(&self, report: &SessionReport) -> Vec<(ServedSearch, bool)> {
        let mut drawn: Vec<&str> = Vec::new();
        for event in &report.events {
            if let SessionEvent::Rendered { search_id, .. } = event
                && !drawn.contains(&search_id.as_str())
            {
                drawn.push(search_id);
            }
        }
        let decisive = report
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                SessionEvent::Action { search_id, .. } => Some(search_id.as_str()),
                _ => None,
            })
            .or(drawn.last().copied());
        let recent = self.recent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        drawn
            .iter()
            .filter_map(|id| recent.iter().find(|search| search.search_id == *id))
            .map(|search| (search.clone(), Some(search.search_id.as_str()) == decisive))
            .collect()
    }

    /// Count each of `ids` as one use now, and save the usage history.
    fn record_uses(&self, ids: &[&str], now: SystemTime) -> Response {
        let mut usage = self.lock_usage();
        for id in ids {
            usage.record_launch(id, now);
        }
        match usage.save(&self.usage_path) {
            Ok(()) => Response::Recorded,
            Err(err) => internal(format!("could not save usage history: {err}")),
        }
    }

    /// Run `write` on the event store, turning failure into an error response.
    fn with_store(
        &self,
        write: impl FnOnce(&mut Store) -> Result<(), crate::store::StoreError>,
    ) -> Result<(), Response> {
        let mut store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match store.as_mut() {
            Some(store) => write(store).map_err(|err| internal(format!("could not record: {err}"))),
            None => Err(internal("the event store could not be opened".to_string())),
        }
    }

    /// How much the event store holds, and its most recent sessions.
    pub fn log_summary(&self, sessions: usize) -> Option<(crate::store::Counts, Vec<crate::store::SessionSummary>)> {
        let store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = store.as_ref()?;
        Some((store.counts().ok()?, store.recent_sessions(sessions).ok()?))
    }

    /// Handle one serialized request and return a serialized response.
    ///
    /// Returns a `String`, not a `Result`: a malformed request is a normal
    /// error *response*, so a caller across the C boundary has exactly one
    /// thing to check rather than two failure channels.
    pub fn handle_json(&self, request_json: &str) -> String {
        let response = match serde_json::from_str::<Request>(request_json) {
            Ok(Request::Search(req)) => {
                let (search_id, results, files_ready) = self.serve(&req.query, req.limit);
                Response::Search(SearchResponse { query: req.query, search_id, results, files_ready })
            }
            Ok(Request::RecordSelection(event)) => self.record_selection(event, SystemTime::now()),
            Ok(Request::RecordEvents(request)) => self.record_events(request.session, SystemTime::now()),
            Ok(Request::RecordFocus(request)) => self.record_focus(request, SystemTime::now()),
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

fn bad_request(message: String) -> Response {
    Response::Error(ErrorResponse { reason: ErrorReason::BadRequest, message })
}

fn internal(message: String) -> Response {
    Response::Error(ErrorResponse { reason: ErrorReason::Internal, message })
}

/// Open the event store in `dir`, importing the old `events.jsonl` log the
/// first time (it is then renamed `events.jsonl.imported`, so it is imported
/// once and kept). `None` if the store cannot be opened.
fn open_store(dir: &Path) -> Option<Store> {
    let mut store = Store::open(&dir.join(crate::store::FILE_NAME)).ok()?;
    let old_log = dir.join(events::FILE_NAME);
    if let Ok(text) = std::fs::read_to_string(&old_log)
        && store.import_selections(&text).is_ok()
    {
        let _ = std::fs::rename(&old_log, dir.join(format!("{}.imported", events::FILE_NAME)));
    }
    Some(store)
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
        let engine = isolated_engine();
        assert!(engine.app_count() > 0, "expected to find installed apps");
    }

    #[test]
    fn finds_terminal_by_prefix() {
        let engine = isolated_engine();
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
        let engine = isolated_engine();
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
        let engine = isolated_engine();
        assert!(engine.search(&engine.index(), "a", 3).len() <= 3);
    }

    #[test]
    fn reindexing_leaves_the_engine_usable() {
        let engine = isolated_engine();
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
        let engine = isolated_engine();
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
        let engine = isolated_engine();
        let v = ask(
            &engine,
            r#"{"op":"search","query":"zzqqxxnomatch","limit":5}"#,
        );
        assert_eq!(v["kind"], "search");
        assert!(v["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn json_search_respects_the_limit() {
        let engine = isolated_engine();
        let v = ask(&engine, r#"{"op":"search","query":"a","limit":2}"#);
        assert!(v["results"].as_array().unwrap().len() <= 2);
    }

    #[test]
    fn malformed_json_is_a_bad_request() {
        let v = ask(&isolated_engine(), "definitely not json");
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn unknown_op_is_a_bad_request() {
        let v = ask(&isolated_engine(), r#"{"op":"teleport"}"#);
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn missing_limit_is_a_bad_request() {
        let v = ask(&isolated_engine(), r#"{"op":"search","query":"term"}"#);
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn error_responses_carry_exactly_one_kind_key() {
        let raw = isolated_engine().handle_json("not json");
        assert_eq!(raw.matches(r#""kind""#).count(), 1, "got {raw}");
    }

    const SAFARI: &str = "app:/Applications/Safari.app";

    /// An engine over this Mac's real apps, settings, and files, keeping its
    /// history in a fresh temporary folder, never the user's.
    fn isolated_engine() -> Engine {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Engine::with_data_dir(&temp_data_dir(&format!("isolated-{n}")))
    }

    fn temp_data_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("milky-engine-{label}-{}", std::process::id()))
    }

    fn selection_json(outcome: &str) -> String {
        format!(
            r#"{{"op":"record_selection","query":"saf","shown":["{SAFARI}","app:/System/Applications/Calendar.app"],"selected":"{SAFARI}","outcome":"{outcome}"}}"#
        )
    }

    /// The outcomes of the selections the engine has stored, oldest first.
    fn stored_selections(engine: &Engine) -> Vec<String> {
        engine.store.lock().unwrap().as_ref().expect("store open").selection_outcomes()
    }

    #[test]
    fn an_opened_selection_counts_as_a_launch() {
        let dir = temp_data_dir("opened");
        let engine = Engine::with_data_dir(&dir);
        assert_eq!(ask(&engine, &selection_json("opened"))["kind"], "recorded");
        assert!(engine.usage_score(SAFARI, SystemTime::now()) > 0.99);
        assert_eq!(stored_selections(&engine), vec!["opened"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_selection_is_logged_but_not_counted() {
        let dir = temp_data_dir("failed");
        let engine = Engine::with_data_dir(&dir);
        assert_eq!(ask(&engine, &selection_json("failed"))["kind"], "recorded");
        assert_eq!(engine.usage_score(SAFARI, SystemTime::now()), 0.0);
        assert_eq!(stored_selections(&engine), vec!["failed"]);
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
        assert!(stored_selections(&engine).is_empty());
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
        assert_eq!(stored_selections(&engine).len(), 40);
        let score = engine.usage_score(SAFARI, SystemTime::now());
        assert!(score > 39.9 && score <= 40.0, "score {score}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An engine over one made-up app, Safari, with no files.
    fn safari_engine(label: &str) -> (Engine, PathBuf) {
        let dir = temp_data_dir(label);
        let _ = std::fs::remove_dir_all(&dir);
        let sources = Sources {
            apps: vec![App::new(PathBuf::from("/Applications/Safari.app"), "Safari".to_string())],
            ..files_only(None)
        };
        (Engine::with_sources(&dir, sources), dir)
    }

    fn search_id_for(engine: &Engine, query: &str) -> String {
        let reply = ask(engine, &format!(r#"{{"op":"search","query":"{query}","limit":10}}"#));
        reply["search_id"].as_str().expect("a search_id").to_string()
    }

    fn session_json(session_id: &str, search_id: &str, action: &str) -> String {
        format!(
            r#"{{"op":"record_events","session":{{"session_id":"{session_id}","started_at_ms":1000,"ended_at_ms":2000,
               "invocation":"hotkey","end":"action",
               "context":{{"frontmost_app":"app:/Applications/Ghostty.app","running_apps":[],"timezone_offset_min":-420,"on_battery":false,"displays":1}},
               "events":[
                 {{"t_ms":0,"type":"query","query":"saf"}},
                 {{"t_ms":20,"type":"rendered","search_id":"{search_id}","visible":1}},
                 {{"t_ms":300,"type":"action","action":"{action}","search_id":"{search_id}","result_id":"{SAFARI}","index":0,"outcome":"opened"}}
               ]}}}}"#
        )
    }

    fn counts(engine: &Engine) -> crate::store::Counts {
        engine.log_summary(1).expect("store open").0
    }

    #[test]
    fn every_search_reply_has_its_own_id() {
        let (engine, _dir) = safari_engine("search-ids");
        let first = search_id_for(&engine, "saf");
        let second = search_id_for(&engine, "saf");
        assert_ne!(first, second);
    }

    #[test]
    fn a_session_stores_what_it_drew_and_counts_the_pick() {
        let (engine, _dir) = safari_engine("session");
        let search_id = search_id_for(&engine, "saf");
        assert_eq!(ask(&engine, &session_json("one", &search_id, "open"))["kind"], "recorded");
        let counts = counts(&engine);
        assert_eq!((counts.sessions, counts.events, counts.searches), (1, 3, 1));
        assert_eq!(counts.impressions, 1, "Safari, shown and in the pool");
        let stored = engine.store.lock().unwrap().as_ref().unwrap().impressions_of(&search_id);
        assert_eq!(stored, vec![(SAFARI.to_string(), Some(0), Some(0))]);
        assert!(engine.usage_score(SAFARI, SystemTime::now()) > 0.99);
    }

    #[test]
    fn revealing_counts_as_use_and_inspecting_does_not() {
        let (engine, _dir) = safari_engine("reveal");
        let search_id = search_id_for(&engine, "saf");
        assert_eq!(ask(&engine, &session_json("a", &search_id, "inspect"))["kind"], "recorded");
        assert_eq!(engine.usage_score(SAFARI, SystemTime::now()), 0.0);
        assert_eq!(ask(&engine, &session_json("b", &search_id, "reveal"))["kind"], "recorded");
        assert!(engine.usage_score(SAFARI, SystemTime::now()) > 0.99);
    }

    #[test]
    fn a_search_no_longer_remembered_still_lets_the_session_be_stored() {
        let (engine, _dir) = safari_engine("forgotten");
        assert_eq!(ask(&engine, &session_json("one", "never-served", "open"))["kind"], "recorded");
        let counts = counts(&engine);
        assert_eq!((counts.sessions, counts.searches), (1, 0));
    }

    #[test]
    fn a_malformed_session_stores_nothing() {
        let (engine, _dir) = safari_engine("bad-session");
        let bad = session_json("one", "x", "open").replace(r#""ended_at_ms":2000"#, r#""ended_at_ms":10"#);
        assert_eq!(ask(&engine, &bad)["reason"], "bad_request");
        assert_eq!(counts(&engine).sessions, 0);
    }

    #[test]
    fn the_old_selection_log_is_imported_once() {
        let dir = temp_data_dir("import");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let line = r#"{"v":2,"event":"selection","at_ms":1,"query":"saf","shown":["app:/Applications/Safari.app"],"selected":"app:/Applications/Safari.app","outcome":"opened"}"#;
        std::fs::write(dir.join(events::FILE_NAME), format!("{line}\n{line}\n")).unwrap();
        {
            let engine = Engine::with_sources(&dir, files_only(None));
            assert_eq!(stored_selections(&engine).len(), 2);
        }
        assert!(!dir.join(events::FILE_NAME).exists());
        assert!(dir.join("events.jsonl.imported").exists());
        let reopened = Engine::with_sources(&dir, files_only(None));
        assert_eq!(stored_selections(&reopened).len(), 2, "not imported twice");
    }

    #[test]
    fn focus_intervals_are_recorded_and_bad_ones_rejected() {
        let (engine, _dir) = safari_engine("focus");
        let good = r#"{"op":"record_focus","intervals":[{"app":"app:/Applications/Ghostty.app","started_at_ms":1,"ended_at_ms":9,"ended_by":"idle"}]}"#;
        assert_eq!(ask(&engine, good)["kind"], "recorded");
        let bad = r#"{"op":"record_focus","intervals":[{"app":"Ghostty","started_at_ms":1,"ended_at_ms":9,"ended_by":"idle"}]}"#;
        assert_eq!(ask(&engine, bad)["reason"], "bad_request");
        assert_eq!(counts(&engine).focus_intervals, 1);
    }

    #[test]
    fn served_files_carry_last_use_and_folder_activity() {
        let home = temp_home("phase-two", &["notes/old.txt"]);
        let engine = Engine::with_sources(&temp_data_dir("phase-two"), files_only(Some(home.clone())));
        assert!(engine.wait_for_files(Duration::from_secs(10)));
        // A new file, marked as opened in 2026-04: the live index sees the
        // change, counts activity in `notes`, and reads the date.
        let report = home.join("notes/quarterly-report.pdf");
        std::fs::write(&report, b"").unwrap();
        let status = std::process::Command::new("xattr")
            .args(["-wx", "com.apple.lastuseddate#PS", "F0E0E76900000000 0000000000000000"])
            .arg(&report)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(eventually(&engine, |engine, index| {
            engine.search(index, "quarterly", 10).first().is_some_and(|hit| {
                hit.candidate.file.as_ref().is_some_and(|facts| facts.last_used.is_some())
            })
        }));

        let reply = ask(&engine, r#"{"op":"search","query":"quarterly","limit":10}"#);
        let search_id = reply["search_id"].as_str().unwrap().to_string();
        let result_id = reply["results"][0]["id"].as_str().unwrap().to_string();
        let session = format!(
            r#"{{"op":"record_events","session":{{"session_id":"p2","started_at_ms":1,"ended_at_ms":2,
               "invocation":"hotkey","end":"escape","context":{{}},
               "events":[{{"t_ms":0,"type":"rendered","search_id":"{search_id}","visible":1}}]}}}}"#
        );
        assert_eq!(ask(&engine, &session)["kind"], "recorded");
        let (last_used_age, folder_activity): (Option<i64>, Option<f64>) = {
            let store = engine.store.lock().unwrap();
            let store = store.as_ref().unwrap();
            store.impression_facts(&search_id, &result_id)
        };
        assert!(last_used_age.is_some_and(|age| age > 0), "last opened months ago: {last_used_age:?}");
        assert!(folder_activity.is_some_and(|activity| activity > 0.0), "notes just changed: {folder_activity:?}");
        assert!(counts(&engine).folder_activity > 0, "the raw counts are stored too");
    }

    #[test]
    fn while_paused_nothing_about_use_is_recorded() {
        let (engine, _dir) = safari_engine("paused");
        engine.set_paused(true).unwrap();
        let search_id = search_id_for(&engine, "saf");
        assert_eq!(ask(&engine, &session_json("one", &search_id, "open"))["kind"], "recorded");
        assert_eq!(ask(&engine, &selection_json("opened"))["kind"], "recorded");
        assert_eq!(counts(&engine).sessions, 0);
        assert!(stored_selections(&engine).is_empty());
        assert_eq!(engine.usage_score(SAFARI, SystemTime::now()), 0.0, "usage is not learned either");
        engine.set_paused(false).unwrap();
        assert_eq!(ask(&engine, &session_json("two", &search_id, "open"))["kind"], "recorded");
        assert_eq!(counts(&engine).sessions, 1);
    }

    #[test]
    fn forgetting_everything_also_forgets_usage() {
        let (engine, _dir) = safari_engine("forget");
        let search_id = search_id_for(&engine, "saf");
        assert_eq!(ask(&engine, &session_json("one", &search_id, "open"))["kind"], "recorded");
        assert!(engine.usage_score(SAFARI, SystemTime::now()) > 0.99);
        engine.forget_history(None).unwrap();
        assert_eq!(counts(&engine).sessions, 0);
        assert_eq!(engine.usage_score(SAFARI, SystemTime::now()), 0.0);
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
        let engine = isolated_engine();
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
