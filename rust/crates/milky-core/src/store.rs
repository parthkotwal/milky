//! The local event store: everything Milky records to learn from use
//! (DECISIONS 2026-10-15).
//!
//! One SQLite database, `milky.sqlite3`, in Milky's data folder. It holds
//! launcher sessions (what was typed, drawn, highlighted, and done, and the
//! context), the searches those sessions drew with their candidates and
//! serving-time features, and single selections from clients that do not send
//! sessions. Nothing here leaves the Mac.
//!
//! Result IDs are long (file paths), and the same ones recur in every search,
//! so each is stored once in `destinations` and referred to by number.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

use crate::api::{FocusRequest, SelectionEvent, SessionEvent, SessionReport};
use crate::candidate::Kind;
use crate::events::LoggedSelection;
use crate::search::Features;

/// File name of the database inside the data folder.
pub const FILE_NAME: &str = "milky.sqlite3";

/// Schema migrations, in order. Entry `n` upgrades version `n` to `n + 1`;
/// the database's `PRAGMA user_version` says how many have run. Never edit a
/// released entry: add a new one.
const MIGRATIONS: &[&str] = &[r"
    CREATE TABLE destinations (
        id INTEGER PRIMARY KEY,
        result_id TEXT NOT NULL UNIQUE
    );
    CREATE TABLE sessions (
        session_id TEXT PRIMARY KEY,
        started_at_ms INTEGER NOT NULL,
        ended_at_ms INTEGER NOT NULL,
        invocation TEXT NOT NULL,
        end_reason TEXT NOT NULL,
        frontmost_app INTEGER REFERENCES destinations(id),
        timezone_offset_min INTEGER,
        on_battery INTEGER,
        displays INTEGER,
        recorded_at_ms INTEGER NOT NULL
    );
    CREATE TABLE session_running_apps (
        session_id TEXT NOT NULL REFERENCES sessions(session_id),
        app INTEGER NOT NULL REFERENCES destinations(id),
        PRIMARY KEY (session_id, app)
    );
    CREATE TABLE session_events (
        session_id TEXT NOT NULL REFERENCES sessions(session_id),
        seq INTEGER NOT NULL,
        t_ms INTEGER NOT NULL,
        type TEXT NOT NULL,
        query TEXT,
        search_id TEXT,
        result INTEGER REFERENCES destinations(id),
        idx INTEGER,
        visible INTEGER,
        action TEXT,
        outcome TEXT,
        PRIMARY KEY (session_id, seq)
    );
    CREATE TABLE searches (
        search_id TEXT PRIMARY KEY,
        session_id TEXT REFERENCES sessions(session_id),
        served_at_ms INTEGER NOT NULL,
        query TEXT NOT NULL,
        ranker_version INTEGER NOT NULL,
        files_ready INTEGER NOT NULL,
        latency_us INTEGER NOT NULL,
        decisive INTEGER NOT NULL
    );
    CREATE TABLE impressions (
        search_id TEXT NOT NULL REFERENCES searches(search_id),
        destination INTEGER NOT NULL REFERENCES destinations(id),
        served_rank INTEGER,
        pool_rank INTEGER,
        kind TEXT NOT NULL,
        tier TEXT NOT NULL,
        standing INTEGER NOT NULL,
        match_kind TEXT,
        keyword TEXT,
        usage REAL NOT NULL,
        prior TEXT NOT NULL,
        title_chars INTEGER NOT NULL,
        depth INTEGER,
        age_s INTEGER,
        PRIMARY KEY (search_id, destination)
    );
    CREATE TABLE selections (
        id INTEGER PRIMARY KEY,
        at_ms INTEGER NOT NULL,
        query TEXT NOT NULL,
        shown TEXT NOT NULL,
        selected TEXT NOT NULL,
        outcome TEXT NOT NULL,
        log_version INTEGER NOT NULL
    );
    CREATE INDEX session_events_by_search ON session_events (search_id);
", r"
    -- Phase 2 (2026-10-16): when files were last opened and how busy their
    -- folders were, dwell after a pick, app focus, folder activity.
    ALTER TABLE impressions ADD COLUMN last_used_age_s INTEGER;
    ALTER TABLE impressions ADD COLUMN folder_activity REAL;
    ALTER TABLE impressions ADD COLUMN tree_activity REAL;
    ALTER TABLE session_events ADD COLUMN app INTEGER REFERENCES destinations(id);
    ALTER TABLE session_events ADD COLUMN front_ms INTEGER;
    ALTER TABLE session_events ADD COLUMN ended TEXT;
    CREATE TABLE focus_intervals (
        app INTEGER NOT NULL REFERENCES destinations(id),
        started_at_ms INTEGER NOT NULL,
        ended_at_ms INTEGER NOT NULL,
        ended_by TEXT NOT NULL,
        recorded_at_ms INTEGER NOT NULL,
        PRIMARY KEY (app, started_at_ms)
    );
    CREATE INDEX focus_intervals_by_time ON focus_intervals (started_at_ms);
    CREATE TABLE folder_activity (
        folder INTEGER NOT NULL REFERENCES destinations(id),
        at_ms INTEGER NOT NULL,
        changes INTEGER NOT NULL
    );
    CREATE INDEX folder_activity_by_time ON folder_activity (at_ms);
", r"
    -- Phase 3 (2026-10-16): settings such as pausing the history.
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"];

/// One search as served, kept in memory until a session says it was drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedSearch {
    pub search_id: String,
    pub served_at_ms: u64,
    pub query: String,
    pub latency_us: u64,
    pub files_ready: bool,
    pub ranker_version: u32,
    /// What the client was sent, in order.
    pub shown: Vec<ServedResult>,
    /// The best candidates by rank alone, in order (see `search::Served`).
    pub pool: Vec<ServedResult>,
}

/// One result of a served search, with what the ranker knew about it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedResult {
    pub result_id: String,
    pub kind: Kind,
    pub features: Features,
}

/// How much is stored, for the CLI and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub sessions: u64,
    pub events: u64,
    pub searches: u64,
    pub impressions: u64,
    pub selections: u64,
    pub focus_intervals: u64,
    pub folder_activity: u64,
}

/// One session in brief, for the CLI.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub started_at_ms: u64,
    pub end_reason: String,
    pub events: u64,
    pub last_query: Option<String>,
    pub acted_on: Option<String>,
}

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Io(std::io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sql(err) => write!(f, "event store: {err}"),
            StoreError::Io(err) => write!(f, "event store i/o: {err}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        StoreError::Sql(err)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::Io(err)
    }
}

/// The open database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path` and bring its schema
    /// up to date.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::prepare(Connection::open(path)?)
    }

    /// A store that lives only in memory, for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self, StoreError> {
        // WAL: readers (an export, the CLI) never block the writer. NORMAL
        // synchronous: a crash can lose the last moments of writes, never
        // corrupt the file; full sync per write would cost ~5 ms each.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Run every migration the database has not seen yet, each in its own
    /// transaction together with the version bump.
    fn migrate(&mut self) -> Result<(), StoreError> {
        let mut version = self.schema_version()?;
        while let Some(migration) = MIGRATIONS.get(version) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(migration)?;
            tx.pragma_update(None, "user_version", (version + 1) as i64)?;
            tx.commit()?;
            version += 1;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<usize, StoreError> {
        let version: i64 = self.conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok(version as usize)
    }

    /// Store one `record_selection` event, received at `at_ms`.
    pub fn record_selection(&mut self, at_ms: u64, event: &SelectionEvent) -> Result<(), StoreError> {
        let logged = LoggedSelection {
            v: crate::events::LOG_VERSION,
            event: crate::events::EventKind::Selection,
            at_ms,
            query: event.query.clone(),
            shown: event.shown.clone(),
            selected: event.selected.clone(),
            outcome: event.outcome,
        };
        insert_selection(&self.conn, &logged)?;
        Ok(())
    }

    /// Copy the old `events.jsonl` log into `selections`, in one transaction.
    /// Lines that do not parse are skipped (a damaged last line must not hide
    /// the rest). Returns how many were imported.
    pub fn import_selections(&mut self, jsonl: &str) -> Result<usize, StoreError> {
        let tx = self.conn.transaction()?;
        let mut imported = 0;
        for line in jsonl.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(logged) = serde_json::from_str::<LoggedSelection>(line) {
                insert_selection(&tx, &logged)?;
                imported += 1;
            }
        }
        tx.commit()?;
        Ok(imported)
    }

    /// Store one session report, with the searches it drew. `searches` pairs
    /// each drawn search with whether it was decisive (the one acted on, or
    /// the last drawn): a decisive search keeps its whole candidate pool, any
    /// other only what was shown. All in one transaction, so a crash leaves
    /// either the whole report or none of it.
    pub fn record_session(
        &mut self,
        report: &SessionReport,
        searches: &[(&ServedSearch, bool)],
        recorded_at_ms: u64,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let frontmost = report.context.frontmost_app.as_deref().map(|app| destination(&tx, app)).transpose()?;
        tx.execute(
            "INSERT INTO sessions (session_id, started_at_ms, ended_at_ms, invocation, end_reason,
                 frontmost_app, timezone_offset_min, on_battery, displays, recorded_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (session_id) DO UPDATE SET
                 ended_at_ms = excluded.ended_at_ms,
                 end_reason = excluded.end_reason,
                 recorded_at_ms = excluded.recorded_at_ms",
            params![
                report.session_id,
                report.started_at_ms as i64,
                report.ended_at_ms as i64,
                name(&report.invocation),
                name(&report.end),
                frontmost,
                report.context.timezone_offset_min,
                report.context.on_battery,
                report.context.displays,
                recorded_at_ms as i64,
            ],
        )?;
        for app in &report.context.running_apps {
            let app = destination(&tx, app)?;
            tx.execute(
                "INSERT OR IGNORE INTO session_running_apps (session_id, app) VALUES (?1, ?2)",
                params![report.session_id, app],
            )?;
        }

        // A second report for the same session continues its numbering.
        let first_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq) + 1, 0) FROM session_events WHERE session_id = ?1",
            params![report.session_id],
            |row| row.get(0),
        )?;
        for (offset, event) in report.events.iter().enumerate() {
            insert_event(&tx, &report.session_id, first_seq + offset as i64, event)?;
        }

        for (search, decisive) in searches {
            insert_search(&tx, &report.session_id, search, *decisive)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Store a batch of app focus intervals. An interval already stored (same
    /// app and start) is skipped, so a resent batch does no harm.
    pub fn record_focus(&mut self, request: &FocusRequest, recorded_at_ms: u64) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for interval in &request.intervals {
            let app = destination(&tx, &interval.app)?;
            tx.execute(
                "INSERT OR IGNORE INTO focus_intervals (app, started_at_ms, ended_at_ms, ended_by, recorded_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    app,
                    interval.started_at_ms as i64,
                    interval.ended_at_ms as i64,
                    name(&interval.ended_by),
                    recorded_at_ms as i64,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Store how many file changes the indexer saw in each folder at `at_ms`.
    /// `changes` pairs a folder's result ID (`folder:<path>`) with its count.
    pub fn record_folder_activity(&mut self, changes: &[(String, usize)], at_ms: u64) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for (folder, count) in changes {
            let folder = destination(&tx, folder)?;
            tx.execute(
                "INSERT INTO folder_activity (folder, at_ms, changes) VALUES (?1, ?2, ?3)",
                params![folder, at_ms as i64, *count as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether recording is paused (`milky --pause-history`).
    pub fn paused(&self) -> Result<bool, StoreError> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = 'paused'", [], |row| row.get(0))
            .optional()?;
        Ok(value.as_deref() == Some("true"))
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES ('paused', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![if paused { "true" } else { "false" }],
        )?;
        Ok(())
    }

    /// Write a consistent copy of the whole database to `path` (which must
    /// not exist), for analysis that must not read a database being written.
    pub fn export_to(&self, path: &Path) -> Result<(), StoreError> {
        let path = path.to_str().ok_or_else(|| {
            StoreError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "path is not UTF-8"))
        })?;
        self.conn.execute("VACUUM INTO ?1", params![path])?;
        Ok(())
    }

    /// Delete recorded history: everything, or only what happened at or after
    /// `since_ms`. Settings (such as pausing) are kept. Freed space is
    /// returned to the disk.
    pub fn forget(&mut self, since_ms: Option<u64>) -> Result<(), StoreError> {
        let since = since_ms.unwrap_or(0) as i64;
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM impressions WHERE search_id IN
                 (SELECT search_id FROM searches WHERE served_at_ms >= ?1)",
            params![since],
        )?;
        tx.execute("DELETE FROM searches WHERE served_at_ms >= ?1", params![since])?;
        for table in ["session_events", "session_running_apps"] {
            tx.execute(
                &format!(
                    "DELETE FROM {table} WHERE session_id IN (SELECT session_id FROM sessions WHERE started_at_ms >= ?1)"
                ),
                params![since],
            )?;
        }
        tx.execute("DELETE FROM sessions WHERE started_at_ms >= ?1", params![since])?;
        tx.execute("DELETE FROM selections WHERE at_ms >= ?1", params![since])?;
        tx.execute("DELETE FROM focus_intervals WHERE started_at_ms >= ?1", params![since])?;
        tx.execute("DELETE FROM folder_activity WHERE at_ms >= ?1", params![since])?;
        tx.execute(
            "DELETE FROM destinations WHERE id NOT IN (
                 SELECT destination FROM impressions UNION SELECT result FROM session_events WHERE result IS NOT NULL
                 UNION SELECT app FROM session_events WHERE app IS NOT NULL
                 UNION SELECT frontmost_app FROM sessions WHERE frontmost_app IS NOT NULL
                 UNION SELECT app FROM session_running_apps UNION SELECT app FROM focus_intervals
                 UNION SELECT folder FROM folder_activity)",
            [],
        )?;
        tx.commit()?;
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }

    pub fn counts(&self) -> Result<Counts, StoreError> {
        let count = |table: &str| -> Result<u64, StoreError> {
            let n: i64 = self.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))?;
            Ok(n as u64)
        };
        Ok(Counts {
            sessions: count("sessions")?,
            events: count("session_events")?,
            searches: count("searches")?,
            impressions: count("impressions")?,
            selections: count("selections")?,
            focus_intervals: count("focus_intervals")?,
            folder_activity: count("folder_activity")?,
        })
    }

    /// The `limit` most recent sessions, newest first.
    pub fn recent_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT s.session_id, s.started_at_ms, s.end_reason,
                    (SELECT COUNT(*) FROM session_events e WHERE e.session_id = s.session_id),
                    (SELECT query FROM session_events e WHERE e.session_id = s.session_id AND e.type = 'query'
                       ORDER BY seq DESC LIMIT 1),
                    (SELECT d.result_id FROM session_events e JOIN destinations d ON d.id = e.result
                       WHERE e.session_id = s.session_id AND e.type = 'action' ORDER BY seq DESC LIMIT 1)
             FROM sessions s ORDER BY s.started_at_ms DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(SessionSummary {
                started_at_ms: row.get::<_, i64>(1)? as u64,
                end_reason: row.get(2)?,
                events: row.get::<_, i64>(3)? as u64,
                last_query: row.get(4)?,
                acted_on: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The outcomes of stored selections, oldest first, for tests.
    #[cfg(test)]
    pub(crate) fn selection_outcomes(&self) -> Vec<String> {
        let mut statement = self.conn.prepare("SELECT outcome FROM selections ORDER BY id").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    }

    /// The stored last-used age and folder activity of one impression, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn impression_facts(&self, search_id: &str, result_id: &str) -> (Option<i64>, Option<f64>) {
        self.conn
            .query_row(
                "SELECT i.last_used_age_s, i.folder_activity FROM impressions i
                 JOIN destinations d ON d.id = i.destination WHERE i.search_id = ?1 AND d.result_id = ?2",
                params![search_id, result_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    /// The pool and shown ranks stored for `search_id`, by result, for tests.
    #[cfg(test)]
    pub(crate) fn impressions_of(&self, search_id: &str) -> Vec<(String, Option<i64>, Option<i64>)> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT d.result_id, i.served_rank, i.pool_rank FROM impressions i
                 JOIN destinations d ON d.id = i.destination WHERE i.search_id = ?1
                 ORDER BY COALESCE(i.pool_rank, 1000), i.served_rank",
            )
            .unwrap();
        statement
            .query_map(params![search_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

/// The stored number for a result ID, adding it the first time it is seen.
fn destination(tx: &Connection, result_id: &str) -> Result<i64, StoreError> {
    tx.execute("INSERT OR IGNORE INTO destinations (result_id) VALUES (?1)", params![result_id])?;
    Ok(tx.query_row("SELECT id FROM destinations WHERE result_id = ?1", params![result_id], |row| row.get(0))?)
}

fn insert_selection(conn: &Connection, logged: &LoggedSelection) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO selections (at_ms, query, shown, selected, outcome, log_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            logged.at_ms as i64,
            logged.query,
            serde_json::to_string(&logged.shown).unwrap_or_default(),
            logged.selected,
            name(&logged.outcome),
            logged.v,
        ],
    )?;
    Ok(())
}

fn insert_event(tx: &Transaction, session_id: &str, seq: i64, event: &SessionEvent) -> Result<(), StoreError> {
    let result = event.result_id().map(|id| destination(tx, id)).transpose()?;
    let mut row = EventRow::default();
    let kind = match event {
        SessionEvent::Query { query, .. } => {
            row.query = Some(query.as_str());
            "query"
        }
        SessionEvent::Rendered { visible, .. } => {
            row.visible = Some(*visible);
            "rendered"
        }
        SessionEvent::Highlight { index, .. } => {
            row.index = Some(*index);
            "highlight"
        }
        SessionEvent::TabComplete { .. } => "tab_complete",
        SessionEvent::Dwell { app, front_ms, ended, .. } => {
            row.app = app.as_deref().map(|app| destination(tx, app)).transpose()?;
            row.front_ms = Some(*front_ms as i64);
            row.ended = Some(name(ended));
            "dwell"
        }
        SessionEvent::Action { action, index, outcome, .. } => {
            row.index = Some(*index);
            row.action = Some(name(action));
            row.outcome = Some(name(outcome));
            "action"
        }
    };
    tx.execute(
        "INSERT INTO session_events (session_id, seq, t_ms, type, query, search_id, result, idx, visible,
             action, outcome, app, front_ms, ended)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            session_id,
            seq,
            event.t_ms() as i64,
            kind,
            row.query,
            event.search_id(),
            result,
            row.index,
            row.visible,
            row.action,
            row.outcome,
            row.app,
            row.front_ms,
            row.ended,
        ],
    )?;
    Ok(())
}

/// The columns of `session_events` that only some event types use.
#[derive(Default)]
struct EventRow<'a> {
    query: Option<&'a str>,
    visible: Option<u32>,
    index: Option<u32>,
    action: Option<String>,
    outcome: Option<String>,
    app: Option<i64>,
    front_ms: Option<i64>,
    ended: Option<String>,
}

fn insert_search(tx: &Transaction, session_id: &str, search: &ServedSearch, decisive: bool) -> Result<(), StoreError> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO searches (search_id, session_id, served_at_ms, query, ranker_version,
             files_ready, latency_us, decisive)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            search.search_id,
            session_id,
            search.served_at_ms as i64,
            search.query,
            search.ranker_version,
            search.files_ready,
            search.latency_us as i64,
            decisive,
        ],
    )?;
    if inserted == 0 {
        return Ok(()); // already stored by an earlier report
    }
    // One row per result: its place in what was shown, its place in the
    // pool, or both.
    let mut rows: BTreeMap<&str, (Option<usize>, Option<usize>, &ServedResult)> = BTreeMap::new();
    for (rank, result) in search.shown.iter().enumerate() {
        rows.insert(&result.result_id, (Some(rank), None, result));
    }
    if decisive {
        for (rank, result) in search.pool.iter().enumerate() {
            rows.entry(&result.result_id).or_insert((None, None, result)).1 = Some(rank);
        }
    }
    for (result_id, (served_rank, pool_rank, result)) in rows {
        let id = destination(tx, result_id)?;
        let features = &result.features;
        tx.execute(
            "INSERT OR IGNORE INTO impressions (search_id, destination, served_rank, pool_rank, kind, tier,
                 standing, match_kind, keyword, usage, prior, title_chars, depth, age_s,
                 last_used_age_s, folder_activity, tree_activity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                search.search_id,
                id,
                served_rank.map(|rank| rank as i64),
                pool_rank.map(|rank| rank as i64),
                format!("{:?}", result.kind).to_lowercase(),
                format!("{:?}", features.tier),
                features.standing,
                features.match_kind.map(|kind| format!("{kind:?}")),
                features.keyword.map(|keyword| format!("{keyword:?}")),
                features.usage,
                format!("{:?}", features.prior),
                features.title_chars as i64,
                features.depth.map(|depth| depth as i64),
                features.age_s,
                features.last_used_age_s,
                features.folder_activity,
                features.tree_activity,
            ],
        )?;
    }
    Ok(())
}

/// A wire enum's name as it appears in JSON: `click_away`, `copy_path`.
fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_string)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{EndReason, Invocation, Outcome, SessionContext, UserAction};
    use crate::candidate::Prior;
    use crate::search::Tier;

    fn result(id: &str) -> ServedResult {
        ServedResult {
            result_id: id.to_string(),
            kind: Kind::App,
            features: Features {
                tier: Tier::Prefix,
                standing: 7,
                match_kind: None,
                keyword: None,
                usage: 0.5,
                prior: Prior::App,
                title_chars: 6,
                depth: None,
                age_s: None,
                last_used_age_s: None,
                folder_activity: None,
                tree_activity: None,
            },
        }
    }

    fn served(search_id: &str, shown: &[&str], pool: &[&str]) -> ServedSearch {
        ServedSearch {
            search_id: search_id.to_string(),
            served_at_ms: 1_000,
            query: "sa".to_string(),
            latency_us: 300,
            files_ready: true,
            ranker_version: 1,
            shown: shown.iter().map(|id| result(id)).collect(),
            pool: pool.iter().map(|id| result(id)).collect(),
        }
    }

    fn report(session_id: &str, events: Vec<SessionEvent>) -> SessionReport {
        SessionReport {
            session_id: session_id.to_string(),
            started_at_ms: 1_000,
            ended_at_ms: 2_000,
            invocation: Invocation::Hotkey,
            end: EndReason::Action,
            context: SessionContext {
                frontmost_app: Some("app:/Applications/Ghostty.app".into()),
                running_apps: vec!["app:/Applications/Ghostty.app".into(), "app:/Applications/Safari.app".into()],
                timezone_offset_min: Some(-420),
                on_battery: Some(false),
                displays: Some(2),
            },
            events,
        }
    }

    fn picked_safari() -> Vec<SessionEvent> {
        vec![
            SessionEvent::Query { t_ms: 0, query: "sa".into() },
            SessionEvent::Rendered { t_ms: 30, search_id: "s1".into(), visible: 2 },
            SessionEvent::Action {
                t_ms: 400,
                action: UserAction::Open,
                search_id: "s1".into(),
                result_id: "app:/Applications/Safari.app".into(),
                index: 0,
                outcome: Outcome::Opened,
            },
        ]
    }

    #[test]
    fn a_new_database_gets_the_current_schema_and_reopening_keeps_it() {
        let dir = std::env::temp_dir().join(format!("milky-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(FILE_NAME);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
        drop(store);
        let again = Store::open(&path).unwrap();
        assert_eq!(again.schema_version().unwrap(), MIGRATIONS.len(), "no migration runs twice");
    }

    #[test]
    fn a_version_one_database_is_upgraded_in_place() {
        let dir = std::env::temp_dir().join(format!("milky-store-upgrade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute("INSERT INTO destinations (result_id) VALUES ('app:/Old.app')", []).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
        let columns: Vec<String> = store
            .conn
            .prepare("SELECT name FROM pragma_table_info('impressions')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.contains(&"last_used_age_s".to_string()));
        let kept: i64 = store.conn.query_row("SELECT COUNT(*) FROM destinations", [], |row| row.get(0)).unwrap();
        assert_eq!(kept, 1, "existing rows survive the upgrade");
    }

    #[test]
    fn focus_intervals_are_stored_once() {
        let mut store = Store::open_in_memory().unwrap();
        let request: FocusRequest = serde_json::from_str(
            r#"{"intervals":[{"app":"app:/Applications/Ghostty.app","started_at_ms":1,"ended_at_ms":5,"ended_by":"idle"},
                             {"app":"app:/Applications/Safari.app","started_at_ms":5,"ended_at_ms":9,"ended_by":"switch"}]}"#,
        )
        .unwrap();
        store.record_focus(&request, 10).unwrap();
        store.record_focus(&request, 11).unwrap();
        assert_eq!(store.counts().unwrap().focus_intervals, 2, "a resent batch is not stored twice");
    }

    #[test]
    fn folder_activity_is_stored() {
        let mut store = Store::open_in_memory().unwrap();
        store.record_folder_activity(&[("folder:/h/proj".into(), 3), ("folder:/h".into(), 1)], 7).unwrap();
        assert_eq!(store.counts().unwrap().folder_activity, 2);
    }

    #[test]
    fn a_dwell_is_stored_with_the_session_it_follows() {
        let mut store = Store::open_in_memory().unwrap();
        store.record_session(&report("one", picked_safari()), &[], 5_000).unwrap();
        let dwell = vec![SessionEvent::Dwell {
            t_ms: 4_300,
            result_id: "app:/Applications/Safari.app".into(),
            app: Some("app:/Applications/Safari.app".into()),
            front_ms: 3_400,
            ended: crate::api::DwellEnd::Switched,
        }];
        store.record_session(&report("one", dwell), &[], 6_000).unwrap();
        let (front_ms, ended): (i64, String) = store
            .conn
            .query_row("SELECT front_ms, ended FROM session_events WHERE type = 'dwell'", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((front_ms, ended.as_str()), (3_400, "switched"));
        assert_eq!(store.counts().unwrap().events, 4);
    }

    #[test]
    fn pausing_is_remembered() {
        let mut store = Store::open_in_memory().unwrap();
        assert!(!store.paused().unwrap());
        store.set_paused(true).unwrap();
        assert!(store.paused().unwrap());
        store.set_paused(false).unwrap();
        assert!(!store.paused().unwrap());
    }

    #[test]
    fn forgetting_everything_empties_every_table_but_settings() {
        let mut store = Store::open_in_memory().unwrap();
        let search = served("s1", &["app:/Applications/Safari.app"], &["app:/Applications/Slack.app"]);
        store.record_session(&report("one", picked_safari()), &[(&search, true)], 5_000).unwrap();
        store.record_folder_activity(&[("folder:/h".into(), 1)], 7).unwrap();
        store.set_paused(true).unwrap();
        store.forget(None).unwrap();
        assert_eq!(store.counts().unwrap(), Counts::default());
        let destinations: i64 = store.conn.query_row("SELECT COUNT(*) FROM destinations", [], |row| row.get(0)).unwrap();
        assert_eq!(destinations, 0);
        assert!(store.paused().unwrap(), "settings are kept");
    }

    #[test]
    fn forgetting_recent_history_keeps_what_came_before() {
        let mut store = Store::open_in_memory().unwrap();
        let old = SessionReport { started_at_ms: 100, ended_at_ms: 200, ..report("old", picked_safari()) };
        let recent = SessionReport { started_at_ms: 10_000, ended_at_ms: 10_100, ..report("recent", vec![]) };
        store.record_session(&old, &[], 300).unwrap();
        store.record_session(&recent, &[], 10_200).unwrap();
        store.forget(Some(5_000)).unwrap();
        let counts = store.counts().unwrap();
        assert_eq!((counts.sessions, counts.events), (1, 3), "the old session and its events stay");
    }

    #[test]
    fn an_export_is_a_complete_copy() {
        let mut store = Store::open_in_memory().unwrap();
        store.record_session(&report("one", picked_safari()), &[], 5_000).unwrap();
        let dir = std::env::temp_dir().join(format!("milky-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("copy.sqlite3");
        store.export_to(&copy).unwrap();
        let exported = Store::open(&copy).unwrap();
        assert_eq!(exported.counts().unwrap(), store.counts().unwrap());
    }

    #[test]
    fn a_session_is_stored_with_its_events_context_and_searches() {
        let mut store = Store::open_in_memory().unwrap();
        let search = served("s1", &["app:/Applications/Safari.app", "app:/Applications/Slack.app"], &[]);
        store.record_session(&report("one", picked_safari()), &[(&search, true)], 5_000).unwrap();
        let counts = store.counts().unwrap();
        assert_eq!((counts.sessions, counts.events, counts.searches, counts.impressions), (1, 3, 1, 2));
        let summary = &store.recent_sessions(5).unwrap()[0];
        assert_eq!(summary.end_reason, "action");
        assert_eq!(summary.last_query.as_deref(), Some("sa"));
        assert_eq!(summary.acted_on.as_deref(), Some("app:/Applications/Safari.app"));
    }

    #[test]
    fn a_decisive_search_keeps_its_pool_and_others_only_what_was_shown() {
        let mut store = Store::open_in_memory().unwrap();
        let pool = ["app:/A.app", "app:/B.app", "app:/C.app", "app:/D.app"];
        let decisive = served("s1", &["app:/A.app", "app:/C.app"], &pool);
        let earlier = served("s0", &["app:/A.app"], &pool);
        store.record_session(&report("one", vec![]), &[(&earlier, false), (&decisive, true)], 5_000).unwrap();
        assert_eq!(
            store.impressions_of("s1"),
            vec![
                ("app:/A.app".to_string(), Some(0), Some(0)),
                ("app:/B.app".to_string(), None, Some(1)),
                ("app:/C.app".to_string(), Some(1), Some(2)),
                ("app:/D.app".to_string(), None, Some(3)),
            ]
        );
        assert_eq!(store.impressions_of("s0"), vec![("app:/A.app".to_string(), Some(0), None)]);
    }

    #[test]
    fn a_second_report_for_a_session_adds_to_it() {
        let mut store = Store::open_in_memory().unwrap();
        store.record_session(&report("one", picked_safari()), &[], 5_000).unwrap();
        let later = vec![SessionEvent::Query { t_ms: 3_000, query: "more".into() }];
        store.record_session(&report("one", later), &[], 6_000).unwrap();
        let counts = store.counts().unwrap();
        assert_eq!((counts.sessions, counts.events), (1, 4));
    }

    #[test]
    fn result_ids_are_stored_once() {
        let mut store = Store::open_in_memory().unwrap();
        let search = served("s1", &["app:/Applications/Safari.app"], &["app:/Applications/Safari.app"]);
        store.record_session(&report("one", picked_safari()), &[(&search, true)], 5_000).unwrap();
        let distinct: i64 = store.conn.query_row("SELECT COUNT(*) FROM destinations", [], |row| row.get(0)).unwrap();
        assert_eq!(distinct, 2, "Safari and Ghostty, each once, however often they appear");
    }

    #[test]
    fn the_old_selection_log_is_imported_and_damaged_lines_skipped() {
        let mut store = Store::open_in_memory().unwrap();
        let line = r#"{"v":2,"event":"selection","at_ms":1,"query":"find","shown":["app:/A.app"],"selected":"app:/A.app","outcome":"opened"}"#;
        let jsonl = format!("{line}\n{line}\n{{\"v\":2,\"ev");
        assert_eq!(store.import_selections(&jsonl).unwrap(), 2);
        assert_eq!(store.counts().unwrap().selections, 2);
    }

    #[test]
    fn a_selection_is_stored() {
        let mut store = Store::open_in_memory().unwrap();
        let event = SelectionEvent {
            query: "sa".into(),
            shown: vec!["app:/A.app".into()],
            selected: "app:/A.app".into(),
            outcome: Outcome::Opened,
        };
        store.record_selection(42, &event).unwrap();
        assert_eq!(store.counts().unwrap().selections, 1);
    }
}
