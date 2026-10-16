//! How busy each folder has been lately: a ranking feature (TASKS, folder
//! activity; DECISIONS 2026-10-15).
//!
//! The live indexer counts the file changes it sees in each indexed folder.
//! Each count decays with a two-hour half-life, so a folder's score says how
//! much is happening there now, not how much ever happened. Raw counts are
//! also stored (`folder_activity` in the event store), so other measures can
//! be computed later from the same facts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How quickly activity fades: half every two hours. A project worked on this
/// morning still counts this afternoon; one from last week does not.
pub const HALF_LIFE: Duration = Duration::from_secs(2 * 60 * 60);

/// Decayed change counts per folder.
#[derive(Debug, Default)]
pub struct FolderActivity {
    folders: HashMap<PathBuf, (f64, SystemTime)>,
}

impl FolderActivity {
    /// Count `changes` file changes in `folder` at `now`.
    pub fn record(&mut self, folder: &Path, changes: usize, now: SystemTime) {
        let entry = self.folders.entry(folder.to_path_buf()).or_insert((0.0, now));
        entry.0 = decayed(entry.0, entry.1, now) + changes as f64;
        entry.1 = now;
    }

    /// `folder`'s activity at `now`: 0 for a folder with no recorded changes.
    pub fn score(&self, folder: &Path, now: SystemTime) -> f64 {
        self.folders.get(folder).map_or(0.0, |&(score, at)| decayed(score, at, now))
    }

    /// The activity of `path`'s folder, and of every folder above it summed
    /// (how busy the whole area is, the project included).
    pub fn around(&self, path: &Path, now: SystemTime) -> (f64, f64) {
        let parent = path.parent().map_or(0.0, |folder| self.score(folder, now));
        let tree = path.ancestors().skip(1).map(|folder| self.score(folder, now)).sum();
        (parent, tree)
    }
}

fn decayed(score: f64, at: SystemTime, now: SystemTime) -> f64 {
    let elapsed = now.duration_since(at).unwrap_or_default();
    score * 0.5_f64.powf(elapsed.as_secs_f64() / HALF_LIFE.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn changes_add_up_and_fade_by_half_each_half_life() {
        let mut activity = FolderActivity::default();
        activity.record(Path::new("/h/proj"), 3, at(0));
        activity.record(Path::new("/h/proj"), 1, at(0));
        assert_eq!(activity.score(Path::new("/h/proj"), at(0)), 4.0);
        let later = at(HALF_LIFE.as_secs());
        assert!((activity.score(Path::new("/h/proj"), later) - 2.0).abs() < 1e-9);
        assert_eq!(activity.score(Path::new("/h/other"), later), 0.0);
    }

    #[test]
    fn a_file_sees_its_folder_and_the_whole_area_above_it() {
        let mut activity = FolderActivity::default();
        activity.record(Path::new("/h/proj/src"), 5, at(0));
        activity.record(Path::new("/h/proj"), 2, at(0));
        assert_eq!(activity.around(Path::new("/h/proj/src/main.rs"), at(0)), (5.0, 7.0));
        assert_eq!(activity.around(Path::new("/h/notes.txt"), at(0)), (0.0, 0.0));
    }
}
