//! The append-only interaction log: raw history for evaluating and, later,
//! learning ranking.
//!
//! JSON Lines: one object per line. The Python workspace can load it with one
//! call, and a damaged final line never hides the lines before it. Records the
//! query, everything that was shown, and what was picked, because what the user
//! passed over is the training signal and cannot be reconstructed later.
//!
//! Stays on this Mac. Roughly 2 KB per selection.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::api::{Outcome, SelectionEvent};
use crate::storage::{self, StorageError, append_line};

/// File name of the log inside the data directory.
pub const FILE_NAME: &str = "events.jsonl";

/// Bump when the meaning or shape of a logged record changes.
///
/// 1: `shown` and `selected` were app paths. 2: they are result IDs
/// (`app:/...`, `settings:...`; contract v3). Readers must accept both.
pub const LOG_VERSION: u32 = 2;

/// Which kind of interaction a log line records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Selection,
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggedSelection {
    pub v: u32,
    pub event: EventKind,
    /// When the engine received it, in milliseconds since the Unix epoch. The
    /// engine stamps time rather than trusting the client's clock.
    pub at_ms: u64,
    pub query: String,
    /// Result IDs in the order they were displayed.
    pub shown: Vec<String>,
    pub selected: String,
    pub outcome: Outcome,
}

impl LoggedSelection {
    pub fn new(event: &SelectionEvent, now: SystemTime) -> Self {
        Self {
            v: LOG_VERSION,
            event: EventKind::Selection,
            at_ms: now
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            query: event.query.clone(),
            shown: event.shown.clone(),
            selected: event.selected.clone(),
            outcome: event.outcome,
        }
    }
}

/// Append one record to the log at `path`, durably.
pub fn append(path: &Path, record: &LoggedSelection) -> Result<(), StorageError> {
    append_line(path, &serde_json::to_vec(record)?)
}

/// Where the log lives: `~/Library/Application Support/Milky/events.jsonl`.
pub fn default_path() -> Result<PathBuf, StorageError> {
    Ok(storage::data_dir()?.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn event() -> SelectionEvent {
        SelectionEvent {
            query: "c".into(),
            shown: vec![
                "app:/System/Applications/Chess.app".into(),
                "app:/System/Applications/Calendar.app".into(),
            ],
            selected: "app:/System/Applications/Calendar.app".into(),
            outcome: Outcome::Opened,
        }
    }

    #[test]
    fn records_are_versioned_and_stamped_by_the_engine() {
        let now = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        let record = LoggedSelection::new(&event(), now);
        assert_eq!(record.v, LOG_VERSION);
        assert_eq!(record.at_ms, 1_700_000_000_123);
        assert_eq!(record.event, EventKind::Selection);
    }

    #[test]
    fn appended_lines_round_trip_in_order() {
        let dir = std::env::temp_dir().join(format!("milky-events-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let first = LoggedSelection::new(&event(), UNIX_EPOCH);
        let mut second_event = event();
        second_event.outcome = Outcome::Failed;
        let second = LoggedSelection::new(&second_event, UNIX_EPOCH);

        append(&path, &first).unwrap();
        append(&path, &second).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<LoggedSelection> = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
            .collect();
        assert_eq!(lines, vec![first, second]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_log_line_uses_contract_names() {
        let line = serde_json::to_string(&LoggedSelection::new(&event(), UNIX_EPOCH)).unwrap();
        assert!(line.contains(r#""event":"selection""#), "{line}");
        assert!(line.contains(r#""outcome":"opened""#), "{line}");
    }
}
