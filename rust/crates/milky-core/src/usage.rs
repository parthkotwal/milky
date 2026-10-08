//! Learning from what the user actually launches.
//!
//! Each launch is worth 1.0 when it happens and decays exponentially with age,
//! so frequent use accumulates while abandoned use fades. Rather than keeping
//! every timestamp, we keep one decayed score per app and the moment it was last
//! updated, which is equivalent and constant-space.
//!
//! The store persists as JSON under `~/Library/Application Support/Milky/`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::storage::{self, StorageError, sibling, write_atomic};

/// File name of the usage store inside the data directory.
pub const FILE_NAME: &str = "usage.json";

/// How long until a launch counts half as much.
///
/// A hyperparameter, picked by intuition and not yet by evidence. Shorter makes
/// the launcher forget faster and chase recent habits; longer makes it stubborn.
pub const HALF_LIFE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What we know about one app's use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Decayed launch count as of `last_updated`.
    score: f64,
    /// When `score` was last brought up to date.
    last_updated: SystemTime,
    /// Raw launch count, never decayed. For inspection, not ranking.
    pub launches: u32,
}

/// Usage history for every app we have seen launched.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UsageStore {
    entries: HashMap<PathBuf, Usage>,
}

impl UsageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of apps with recorded history.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Record that `path` was launched at `now`.
    pub fn record_launch(&mut self, path: &Path, now: SystemTime) {
        self.entries
            .entry(path.to_path_buf())
            .and_modify(|usage| {
                let elapsed = now
                    .duration_since(usage.last_updated)
                    .unwrap_or(Duration::ZERO);
                usage.score = usage.score * decay_factor(elapsed) + 1.0;
                usage.last_updated = now;
                usage.launches += 1;
            })
            .or_insert_with(|| Usage {
                score: 1.0,
                last_updated: now,
                launches: 1,
            });
    }

    /// Decayed score for `path` as of `now`. Zero if never launched.
    pub fn score(&self, path: &Path, now: SystemTime) -> f64 {
        match self.entries.get(path) {
            Some(usage) => {
                let elapsed = now
                    .duration_since(usage.last_updated)
                    .unwrap_or(Duration::ZERO);

                usage.score * decay_factor(elapsed)
            }
            None => 0.0,
        }
    }
}

impl UsageStore {
    /// Load history from `path`.
    ///
    /// A missing file is an empty store, not an error: the first run is normal.
    /// A file that exists but cannot be parsed *is* an error, because silently
    /// starting over would throw away real history.
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(err) => return Err(err.into()),
        };
        Ok(serde_json::from_str(&text)?)
    }

    /// Load history, or start empty without destroying a damaged file.
    ///
    /// A launcher that refuses to start because its ranking history is corrupt
    /// is worse than one that forgets. So on any load failure the file is moved
    /// aside to `usage.json.corrupt-<unix seconds>`, kept for inspection, and the
    /// store starts empty. The error is returned so the caller can report it.
    pub fn load_or_quarantine(path: &Path, now: SystemTime) -> (Self, Option<StorageError>) {
        match Self::load(path) {
            Ok(store) => (store, None),
            Err(err) => {
                let seconds = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let _ = fs::rename(path, sibling(path, &format!(".corrupt-{seconds}")));
                (Self::new(), Some(err))
            }
        }
    }

    /// Save history to `path` crash-safely (see [`storage::write_atomic`]).
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let json = serde_json::to_vec_pretty(self)?;
        write_atomic(path, &json)
    }
}

/// Where usage history lives: `~/Library/Application Support/Milky/usage.json`.
pub fn default_path() -> Result<PathBuf, StorageError> {
    Ok(storage::data_dir()?.join(FILE_NAME))
}

/// How much a score is worth after `elapsed` time has passed.
///
/// `0.5` at one half-life, `0.25` at two, approaching zero but never negative.
fn decay_factor(elapsed: Duration) -> f64 {
    0.5_f64.powf(elapsed.as_secs_f64() / HALF_LIFE.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn app() -> PathBuf {
        PathBuf::from("/Applications/Safari.app")
    }

    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn unknown_apps_score_zero() {
        let store = UsageStore::new();
        assert_eq!(store.score(&app(), at(0)), 0.0);
    }

    #[test]
    fn one_launch_is_worth_one() {
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        assert!((store.score(&app(), at(0)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_score_halves_after_one_half_life() {
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        let later = at(HALF_LIFE.as_secs());
        assert!((store.score(&app(), later) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn launches_accumulate() {
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        store.record_launch(&app(), at(0));
        assert!((store.score(&app(), at(0)) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn recent_use_beats_stale_use() {
        let mut store = UsageStore::new();
        let daily = PathBuf::from("/Applications/Daily.app");
        let abandoned = PathBuf::from("/Applications/Abandoned.app");

        // Abandoned: twenty launches, a year ago.
        for _ in 0..20 {
            store.record_launch(&abandoned, at(0));
        }
        // Daily: three launches, this week.
        let now = at(365 * DAY);
        for day in 0..3 {
            store.record_launch(&daily, at(365 * DAY - day * DAY));
        }

        assert!(
            store.score(&daily, now) > store.score(&abandoned, now),
            "daily {} should beat abandoned {}",
            store.score(&daily, now),
            store.score(&abandoned, now)
        );
    }

    #[test]
    fn a_backwards_clock_does_not_break_the_score() {
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(1000));
        // Reading at an earlier time than the last update.
        let score = store.score(&app(), at(0));
        assert!(score.is_finite() && score > 0.0, "got {score}");
    }

    #[test]
    fn tracks_how_many_apps_it_knows() {
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        store.record_launch(&app(), at(0));
        assert_eq!(store.len(), 1);
        store.record_launch(Path::new("/Applications/Other.app"), at(0));
        assert_eq!(store.len(), 2);
    }

    /// A fresh directory per test, so parallel tests never share files.
    fn temp_path(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("milky-usage-{label}-{}", std::process::id()));
        path.push("usage.json");
        path
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn saving_then_loading_round_trips() {
        let path = temp_path("roundtrip");
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        store.record_launch(&app(), at(0));
        store.save(&path).expect("save");

        let loaded = UsageStore::load(&path).expect("load");
        assert_eq!(loaded.len(), 1);
        assert!((loaded.score(&app(), at(0)) - 2.0).abs() < 1e-9);
        cleanup(&path);
    }

    #[test]
    fn decay_survives_a_round_trip() {
        let path = temp_path("decay");
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        store.save(&path).expect("save");

        let loaded = UsageStore::load(&path).expect("load");
        let later = at(HALF_LIFE.as_secs());
        assert!((loaded.score(&app(), later) - 0.5).abs() < 1e-9);
        cleanup(&path);
    }

    #[test]
    fn a_missing_file_is_an_empty_store() {
        let path = temp_path("missing");
        let loaded = UsageStore::load(&path).expect("a missing file should not be an error");
        assert!(loaded.is_empty());
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_store() {
        let path = temp_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this is not json").unwrap();
        assert!(matches!(
            UsageStore::load(&path),
            Err(StorageError::Format(_))
        ));
        cleanup(&path);
    }

    #[test]
    fn save_creates_missing_directories() {
        let path = temp_path("nested");
        UsageStore::new()
            .save(&path)
            .expect("save should create the directory");
        assert!(path.exists());
        cleanup(&path);
    }

    #[test]
    fn save_replaces_and_leaves_no_temp_file() {
        let path = temp_path("replace");
        let mut store = UsageStore::new();
        store.record_launch(&app(), at(0));
        store.save(&path).expect("first save");
        store.record_launch(Path::new("/Applications/Other.app"), at(0));
        store.save(&path).expect("second save");

        assert_eq!(UsageStore::load(&path).unwrap().len(), 2);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file left behind"
        );
        cleanup(&path);
    }

    #[test]
    fn a_corrupt_file_is_quarantined_not_destroyed() {
        let path = temp_path("quarantine");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this is not json").unwrap();

        let (store, err) = UsageStore::load_or_quarantine(&path, at(1_700_000_000));
        assert!(store.is_empty());
        assert!(matches!(err, Some(StorageError::Format(_))));
        assert!(
            !path.exists(),
            "the damaged file should have been moved aside"
        );
        let aside = sibling(&path, ".corrupt-1700000000");
        assert_eq!(fs::read_to_string(&aside).unwrap(), "this is not json");
        cleanup(&path);
    }

    #[test]
    fn quarantine_of_a_missing_file_is_just_empty() {
        let path = temp_path("quarantine-missing");
        let (store, err) = UsageStore::load_or_quarantine(&path, at(0));
        assert!(store.is_empty());
        assert!(err.is_none());
    }
}
