//! Keeping the file index current while files change (DECISIONS 2026-10-14).
//!
//! The background indexer holds a [`FileIndex`]: every walked file and folder
//! by path. When a folder changes, it is re-read with
//! [`crate::files::read_folder`] and the fresh listing is applied with
//! [`FileIndex::apply_listing`], which adds, removes, and updates entries so
//! the index matches the disk again.
//!
//! The thread ([`Indexer`]) watches before it walks, so nothing changed during
//! the first walk is missed; then it waits for file-change events, groups each
//! burst ([`next_batch`]), decides what to re-read ([`plan`]), and publishes a
//! new snapshot when anything changed.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::ErrorKind;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::apps;
use crate::engine::{Index, Snapshots, Sources, user_exclusions};
use crate::files::{self, FileEntry};

/// How long to keep gathering events after the first one of a burst. A build
/// or an unzip produces thousands within this window; they cost one pass.
pub const BATCH_WINDOW: Duration = Duration::from_millis(300);

/// Where installed apps live outside the home folder (`~/Applications` is
/// already watched with it). A change here re-discovers apps (~7 ms).
const APP_FOLDERS: &[&str] = &["/Applications"];

/// What the indexer thread receives.
#[derive(Debug)]
pub enum Message {
    /// A file-change event from the watcher, or an error meaning events were
    /// lost.
    Changed(notify::Result<notify::Event>),
    /// Re-discover everything, as at startup.
    Rescan,
    /// Finish and exit.
    Stop,
}

/// The events of one burst, gathered by [`next_batch`].
#[derive(Debug, Default)]
pub struct Batch {
    pub events: Vec<notify::Event>,
    /// The watcher reported an error: some events may be missing.
    pub lost_track: bool,
    /// A rescan was asked for.
    pub rescan: bool,
}

impl Batch {
    /// Add one message; `None` if it says to stop.
    fn add(&mut self, message: Message) -> Option<()> {
        match message {
            Message::Changed(Ok(event)) => self.events.push(event),
            Message::Changed(Err(_)) => self.lost_track = true,
            Message::Rescan => self.rescan = true,
            Message::Stop => return None,
        }
        Some(())
    }
}

/// Wait for the next message, then gather everything else that arrives within
/// `window` of it. `None` when told to stop, or when every sender is gone.
pub fn next_batch(messages: &Receiver<Message>, window: Duration) -> Option<Batch> {
    let mut batch = Batch::default();
    batch.add(messages.recv().ok()?)?;
    let deadline = Instant::now() + window;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Some(batch);
        }
        match messages.recv_timeout(left) {
            Ok(message) => batch.add(message)?,
            Err(RecvTimeoutError::Timeout) => return Some(batch),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// What one batch asks the indexer to do.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Walk everything again: events were lost, a rescan was asked for, or the
    /// exclusions changed (`exclusions_changed` says whether to reload them).
    pub rescan: bool,
    pub exclusions_changed: bool,
    /// Re-discover installed apps.
    pub apps_changed: bool,
    /// Indexed folders whose contents changed, to re-read.
    pub folders: BTreeSet<PathBuf>,
}

/// Decide what a batch means. An event matters only if its folder is one we
/// index (a root or an indexed folder): everything in hidden folders,
/// `~/Library`, build output, and excluded folders falls away here, which on
/// the measured machine was all but one of 37,004 events in 90 s.
pub fn plan(batch: &Batch, index: &FileIndex, exclusions_file: Option<&Path>) -> Plan {
    let mut plan = Plan {
        rescan: batch.lost_track || batch.rescan,
        apps_changed: batch.rescan,
        ..Plan::default()
    };
    for event in &batch.events {
        if event.need_rescan() {
            plan.rescan = true;
        }
        for path in &event.paths {
            if exclusions_file == Some(path.as_path()) {
                plan.exclusions_changed = true;
                plan.rescan = true;
            } else if APP_FOLDERS.iter().any(|folder| path.starts_with(folder)) {
                plan.apps_changed = true;
            } else if let Some(folder) = path.parent().filter(|folder| index.tracks(folder)) {
                plan.folders.insert(folder.to_path_buf());
            }
        }
    }
    plan
}

/// The background indexer: a thread keeping the published index current.
/// Stop it with [`Indexer::stop`] (the engine does, when dropped).
pub struct Indexer {
    messages: Sender<Message>,
    thread: Option<JoinHandle<()>>,
}

impl Indexer {
    /// Start the thread: watch, walk, publish, then follow changes. If the
    /// thread cannot be created, the engine still answers for everything but
    /// files.
    pub fn start(snapshots: Arc<Snapshots>, sources: Sources) -> Self {
        let (sender, receiver) = mpsc::channel();
        let for_watcher = sender.clone();
        let thread = std::thread::Builder::new()
            .name("milky-indexer".to_string())
            .spawn(move || run(&snapshots, sources, &receiver, for_watcher))
            .ok();
        Self { messages: sender, thread }
    }

    /// Ask for everything to be re-discovered, as at startup. Returns at
    /// once; the new snapshot is published when ready.
    pub fn rescan(&self) {
        let _ = self.messages.send(Message::Rescan);
    }

    /// Tell the thread to finish and wait for it. If it is in the middle of a
    /// walk, that walk completes first.
    pub fn stop(mut self) {
        let _ = self.messages.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The indexer thread's whole life.
fn run(snapshots: &Snapshots, mut sources: Sources, messages: &Receiver<Message>, for_watcher: Sender<Message>) {
    let Some(home) = sources.home.clone() else {
        // No home folder: nothing to walk or watch; wait to be stopped.
        snapshots.publish(Index::new(sources.candidates(&[]), true));
        while next_batch(messages, Duration::ZERO).is_some() {}
        return;
    };
    let roots = files::roots(&home);
    let exclusions_file = files::exclusions_path().ok();
    let mut exclusions = user_exclusions(&home);

    // Watch first: changes made during the walk are queued, not lost.
    let mut watched = vec![home.clone()];
    watched.extend(APP_FOLDERS.iter().map(PathBuf::from));
    let _watcher = watch(&watched, for_watcher);

    let walk_all =
        |exclusions: &files::Exclusions| FileIndex::new(roots.clone(), files::discover_files(&home, exclusions).entries);
    let mut index = walk_all(&exclusions);
    publish(snapshots, &sources, &index);

    while let Some(batch) = next_batch(messages, BATCH_WINDOW) {
        let plan = plan(&batch, &index, exclusions_file.as_deref());
        let mut changed = false;
        if plan.apps_changed {
            sources.apps = apps::discover_apps();
            changed = true;
        }
        if plan.exclusions_changed {
            exclusions = user_exclusions(&home);
        }
        if plan.rescan {
            index = walk_all(&exclusions);
            changed = true;
        } else {
            for folder in &plan.folders {
                match files::read_folder(folder, &exclusions) {
                    Ok(listing) => {
                        changed |=
                            index.apply_listing(folder, listing, |inside| files::walk(inside, &exclusions).entries);
                    }
                    // The folder itself is gone; its parent's event says so too.
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        changed |= index.remove_with_contents(folder);
                    }
                    Err(_) => {}
                }
            }
        }
        if changed {
            publish(snapshots, &sources, &index);
        }
    }
}

/// Publish `sources` plus every file in `index`.
fn publish(snapshots: &Snapshots, sources: &Sources, index: &FileIndex) {
    let files: Vec<FileEntry> = index.entries().cloned().collect();
    snapshots.publish(Index::new(sources.candidates(&files), true));
}

/// Start watching `paths` and everything inside them, sending each event to
/// `messages`. `None` if the watcher cannot start: the index then stays as
/// first walked. A path that does not exist (no iCloud Drive) is skipped.
fn watch(paths: &[PathBuf], messages: Sender<Message>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = messages.send(Message::Changed(event));
    })
    .ok()?;
    for path in paths {
        let _ = watcher.watch(path, RecursiveMode::Recursive);
    }
    Some(watcher)
}

/// Every indexed file and folder, by path, plus the roots they were walked
/// from.
///
/// A `BTreeMap` keeps paths sorted, and `Path` sorts component by component:
/// `/a/b`, then everything inside `/a/b` (`/a/b/c`, `/a/b/c/d`, `/a/b/z`),
/// then `/a/b c`. So a folder's contents always sit in one unbroken run right
/// after the folder itself, and removing a folder with everything inside it
/// is one range of the map, not a search through all of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileIndex {
    entries: BTreeMap<PathBuf, FileEntry>,
    roots: Vec<PathBuf>,
}

impl FileIndex {
    /// An index of `entries`, walked from `roots`.
    pub fn new(roots: Vec<PathBuf>, entries: Vec<FileEntry>) -> Self {
        let entries = entries.into_iter().map(|entry| (entry.path.clone(), entry)).collect();
        Self { entries, roots }
    }

    /// Every entry, in path order.
    pub fn entries(&self) -> impl Iterator<Item = &FileEntry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, path: &Path) -> Option<&FileEntry> {
        self.entries.get(path)
    }

    /// Whether changes inside `folder` matter: it is a root, or an indexed
    /// folder (not a package, which is one file).
    pub fn tracks(&self, folder: &Path) -> bool {
        self.roots.iter().any(|root| root == folder)
            || self.entries.get(folder).is_some_and(|entry| entry.is_folder)
    }

    /// The paths of the entries directly inside `folder`, in path order.
    fn children(&self, folder: &Path) -> Vec<PathBuf> {
        self.entries
            .range::<Path, _>((Bound::Excluded(folder), Bound::Unbounded))
            .take_while(|(path, _)| path.starts_with(folder))
            .filter(|(path, _)| path.parent() == Some(folder))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Remove `path` and everything inside it. Returns whether anything was
    /// removed.
    ///
    /// The contents of `path` are the run of keys right after it that start
    /// with it (see the type's documentation). `self.entries.range(...)`
    /// gives that run in order, `take_while` stops at the first key outside
    /// it. Collect the keys first, then remove them: a map cannot be changed
    /// while it is being iterated.
    pub fn remove_with_contents(&mut self, path: &Path) -> bool {
        let doomed: Vec<PathBuf> = self
            .entries
            .range::<Path, _>((Bound::Included(path), Bound::Unbounded))
            .take_while(|(key, _)| key.starts_with(path))
            .map(|(key, _)| key.clone())
            .collect();
        for key in &doomed {
            self.entries.remove(key);
        }
        !doomed.is_empty()
    }

    /// Make the index agree with a fresh reading of `folder`: `listing` is
    /// what [`crate::files::read_folder`] returned for it. Returns whether
    /// anything changed.
    ///
    /// For each entry directly inside `folder`:
    /// - indexed but no longer listed (deleted, renamed away, newly excluded):
    ///   remove it and everything inside it;
    /// - listed but not indexed (created, renamed here, moved in): add it,
    ///   and if it is a folder, add everything `walk_new(&path)` finds inside
    ///   it too (it may have arrived full, by a move or an unzip);
    /// - in both: replace the entry if it differs (a new modified date, or a
    ///   folder that became a package or back). If it stopped being a folder,
    ///   remove its contents; if it became one, add what `walk_new` finds.
    ///
    /// `walk_new` is passed in, not called directly, so tests can supply
    /// contents without touching the disk.
    pub fn apply_listing(
        &mut self,
        folder: &Path,
        listing: Vec<FileEntry>,
        mut walk_new: impl FnMut(&Path) -> Vec<FileEntry>,
    ) -> bool {
        let mut changed = false;

        {
            let listed: HashSet<&Path> = listing.iter().map(|entry| entry.path.as_path()).collect();
            for child in self.children(folder) {
                if !listed.contains(child.as_path()) {
                    changed |= self.remove_with_contents(&child);
                }
            }
        }

        for entry in listing {
            let was_folder = match self.entries.get(&entry.path) {
                Some(old) if *old == entry => continue,
                Some(old) => old.is_folder,
                None => false,
            };
            changed = true;
            if was_folder && !entry.is_folder {
                self.remove_with_contents(&entry.path);
            }
            let walk = entry.is_folder && !was_folder;
            let path = entry.path.clone();
            self.entries.insert(path.clone(), entry);
            if walk {
                for inside in walk_new(&path) {
                    self.entries.insert(inside.path.clone(), inside);
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn at(seconds: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
    }

    fn file(path: &str) -> FileEntry {
        FileEntry { path: PathBuf::from(path), is_folder: false, modified: at(1) }
    }

    fn folder(path: &str) -> FileEntry {
        FileEntry { path: PathBuf::from(path), is_folder: true, modified: at(1) }
    }

    fn index(entries: Vec<FileEntry>) -> FileIndex {
        FileIndex::new(vec![PathBuf::from("/h")], entries)
    }

    fn paths(index: &FileIndex) -> Vec<&str> {
        index.entries().map(|entry| entry.path.to_str().unwrap()).collect()
    }

    /// A `walk_new` for folders that arrive empty, which must not be called
    /// for anything else.
    fn nothing_inside(path: &Path) -> Vec<FileEntry> {
        let _ = path;
        Vec::new()
    }

    #[test]
    fn a_folder_and_its_contents_are_one_run_of_paths() {
        let index = index(vec![
            folder("/h/a b"),
            folder("/h/a"),
            file("/h/a/x.txt"),
            folder("/h/a/sub"),
            file("/h/a/sub/y.txt"),
            file("/h/a.txt"),
        ]);
        assert_eq!(
            paths(&index),
            vec!["/h/a", "/h/a/sub", "/h/a/sub/y.txt", "/h/a/x.txt", "/h/a b", "/h/a.txt"]
        );
        assert_eq!(index.children(Path::new("/h/a")), vec![PathBuf::from("/h/a/sub"), PathBuf::from("/h/a/x.txt")]);
    }

    #[test]
    fn removing_a_folder_removes_its_contents_and_nothing_else() {
        let mut index = index(vec![
            folder("/h/a"),
            file("/h/a/x.txt"),
            folder("/h/a/sub"),
            file("/h/a/sub/y.txt"),
            folder("/h/a b"),
            file("/h/a b/z.txt"),
            file("/h/a.txt"),
        ]);
        assert!(index.remove_with_contents(Path::new("/h/a")));
        assert_eq!(paths(&index), vec!["/h/a b", "/h/a b/z.txt", "/h/a.txt"], "siblings sharing a prefix stay");
        assert!(!index.remove_with_contents(Path::new("/h/a")), "nothing left to remove");
    }

    #[test]
    fn a_created_file_is_added() {
        let mut index = index(vec![file("/h/old.txt")]);
        let changed = index.apply_listing(Path::new("/h"), vec![file("/h/old.txt"), file("/h/new.txt")], nothing_inside);
        assert!(changed);
        assert_eq!(paths(&index), vec!["/h/new.txt", "/h/old.txt"]);
    }

    #[test]
    fn a_deleted_file_is_removed() {
        let mut index = index(vec![file("/h/a.txt"), file("/h/b.txt")]);
        assert!(index.apply_listing(Path::new("/h"), vec![file("/h/b.txt")], nothing_inside));
        assert_eq!(paths(&index), vec!["/h/b.txt"]);
    }

    #[test]
    fn a_deleted_folder_takes_its_contents_with_it() {
        let mut index = index(vec![folder("/h/proj"), file("/h/proj/a.rs"), folder("/h/proj/src"), file("/h/proj/src/main.rs")]);
        assert!(index.apply_listing(Path::new("/h"), vec![], nothing_inside));
        assert!(index.is_empty());
    }

    #[test]
    fn a_folder_that_arrives_full_is_walked() {
        let mut index = index(vec![]);
        let changed = index.apply_listing(Path::new("/h"), vec![folder("/h/unzipped")], |path| {
            assert_eq!(path, Path::new("/h/unzipped"));
            vec![folder("/h/unzipped/docs"), file("/h/unzipped/docs/readme.md")]
        });
        assert!(changed);
        assert_eq!(paths(&index), vec!["/h/unzipped", "/h/unzipped/docs", "/h/unzipped/docs/readme.md"]);
    }

    #[test]
    fn a_rename_is_a_removal_and_an_addition() {
        let mut index = index(vec![folder("/h/draft"), file("/h/draft/a.txt")]);
        let changed = index.apply_listing(Path::new("/h"), vec![folder("/h/final")], |_| vec![file("/h/final/a.txt")]);
        assert!(changed);
        assert_eq!(paths(&index), vec!["/h/final", "/h/final/a.txt"]);
    }

    #[test]
    fn an_edit_updates_the_modified_date() {
        let mut index = index(vec![file("/h/a.txt")]);
        let edited = FileEntry { modified: at(2), ..file("/h/a.txt") };
        assert!(index.apply_listing(Path::new("/h"), vec![edited.clone()], nothing_inside));
        assert_eq!(index.get(Path::new("/h/a.txt")), Some(&edited));
    }

    #[test]
    fn an_unchanged_listing_changes_nothing() {
        let mut index = index(vec![folder("/h/a"), file("/h/a/x.txt"), file("/h/b.txt")]);
        let before = index.clone();
        let changed = index.apply_listing(Path::new("/h"), vec![folder("/h/a"), file("/h/b.txt")], |_| {
            panic!("an existing folder must not be walked again")
        });
        assert!(!changed);
        assert_eq!(index, before, "the folder's contents are left alone");
    }

    #[test]
    fn only_the_named_folder_is_reconciled() {
        let mut index = index(vec![folder("/h/a"), file("/h/a/x.txt"), file("/h/top.txt")]);
        assert!(index.apply_listing(Path::new("/h/a"), vec![], nothing_inside));
        assert_eq!(paths(&index), vec!["/h/a", "/h/top.txt"], "only /h/a's contents went");
    }

    #[test]
    fn a_folder_that_becomes_a_package_loses_its_contents() {
        let mut index = index(vec![folder("/h/Thing.pages"), file("/h/Thing.pages/index.xml")]);
        assert!(index.apply_listing(Path::new("/h"), vec![file("/h/Thing.pages")], nothing_inside));
        assert_eq!(paths(&index), vec!["/h/Thing.pages"]);
        assert!(!index.get(Path::new("/h/Thing.pages")).unwrap().is_folder);
    }

    fn created(path: &str) -> notify::Event {
        notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File)).add_path(PathBuf::from(path))
    }

    fn batch(events: Vec<notify::Event>) -> Batch {
        Batch { events, ..Batch::default() }
    }

    #[test]
    fn a_burst_becomes_one_batch() {
        let (sender, receiver) = mpsc::channel();
        for i in 0..100 {
            sender.send(Message::Changed(Ok(created(&format!("/h/{i}.txt"))))).unwrap();
        }
        let batch = next_batch(&receiver, Duration::from_millis(50)).unwrap();
        assert_eq!(batch.events.len(), 100);
        assert!(!batch.lost_track);
    }

    #[test]
    fn a_batch_ends_when_the_window_does() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Message::Changed(Ok(created("/h/a.txt")))).unwrap();
        let started = Instant::now();
        let batch = next_batch(&receiver, Duration::from_millis(30)).unwrap();
        assert_eq!(batch.events.len(), 1);
        assert!(started.elapsed() < Duration::from_secs(2), "waited for the window, not forever");
        drop(sender);
    }

    #[test]
    fn stop_ends_the_wait_even_mid_burst() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Message::Changed(Ok(created("/h/a.txt")))).unwrap();
        sender.send(Message::Stop).unwrap();
        assert!(next_batch(&receiver, Duration::from_secs(10)).is_none());
        let (sender, receiver) = mpsc::channel::<Message>();
        drop(sender);
        assert!(next_batch(&receiver, Duration::from_secs(10)).is_none(), "no senders left");
    }

    #[test]
    fn a_watcher_error_means_events_were_lost() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Message::Changed(Err(notify::Error::generic("overflow")))).unwrap();
        let batch = next_batch(&receiver, Duration::from_millis(10)).unwrap();
        assert!(batch.lost_track);
        assert!(plan(&batch, &index(vec![]), None).rescan);
    }

    #[test]
    fn only_events_in_indexed_folders_count() {
        let index = index(vec![folder("/h/docs"), file("/h/docs/a.txt")]);
        let plan = plan(
            &batch(vec![
                created("/h/docs/b.txt"),
                created("/h/docs/c.txt"),
                created("/h/new.txt"),
                created("/h/.cache/x"),
                created("/h/Library/Caches/y"),
                created("/h/proj/target/debug/z"),
            ]),
            &index,
            None,
        );
        assert!(!plan.rescan && !plan.apps_changed);
        assert_eq!(
            plan.folders.into_iter().collect::<Vec<_>>(),
            vec![PathBuf::from("/h"), PathBuf::from("/h/docs")],
            "each changed folder once; hidden, Library, and unindexed folders dropped"
        );
    }

    #[test]
    fn app_folder_changes_rediscover_apps() {
        let plan = plan(&batch(vec![created("/Applications/New.app/Contents/Info.plist")]), &index(vec![]), None);
        assert!(plan.apps_changed);
        assert!(plan.folders.is_empty());
    }

    #[test]
    fn editing_the_exclusions_file_rescans_with_new_rules() {
        let exclusions = Path::new("/h/Library/Application Support/Milky/exclusions.txt");
        let plan = plan(&batch(vec![created(exclusions.to_str().unwrap())]), &index(vec![]), Some(exclusions));
        assert!(plan.exclusions_changed && plan.rescan);
    }

    #[test]
    fn a_rescan_request_redoes_everything() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Message::Rescan).unwrap();
        let batch = next_batch(&receiver, Duration::from_millis(10)).unwrap();
        let plan = plan(&batch, &index(vec![]), None);
        assert!(plan.rescan && plan.apps_changed);
    }

    #[test]
    fn roots_and_indexed_folders_are_tracked_and_packages_are_not() {
        let index = index(vec![folder("/h/a"), file("/h/Thing.pages"), file("/h/a.txt")]);
        assert!(index.tracks(Path::new("/h")));
        assert!(index.tracks(Path::new("/h/a")));
        assert!(!index.tracks(Path::new("/h/Thing.pages")));
        assert!(!index.tracks(Path::new("/h/a.txt")));
        assert!(!index.tracks(Path::new("/h/target")));
    }
}
