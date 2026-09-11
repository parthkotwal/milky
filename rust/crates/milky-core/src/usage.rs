//! Learning from what the user actually launches.
//!
//! Each launch is worth 1.0 when it happens and decays exponentially with age,
//! so frequent use accumulates while abandoned use fades. Rather than keeping
//! every timestamp, we keep one decayed score per app and the moment it was last
//! updated, which is equivalent and constant-space.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long until a launch counts half as much.
///
/// A hyperparameter, picked by intuition and not yet by evidence. Shorter makes
/// the launcher forget faster and chase recent habits; longer makes it stubborn.
pub const HALF_LIFE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What we know about one app's use.
#[derive(Debug, Clone)]
pub struct Usage {
    /// Decayed launch count as of `last_updated`.
    score: f64,
    /// When `score` was last brought up to date.
    last_updated: SystemTime,
    /// Raw launch count, never decayed. For inspection, not ranking.
    pub launches: u32,
}

/// Usage history for every app we have seen launched.
#[derive(Debug, Default)]
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
                let elapsed = now.duration_since(usage.last_updated).unwrap_or(Duration::ZERO);
                usage.score = usage.score * decay_factor(elapsed) + 1.0;
                usage.last_updated = now;
                usage.launches += 1;
            })
            .or_insert_with(|| {
                Usage {
                    score: 1.0,
                    last_updated: now,
                    launches: 1,
                }
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
}