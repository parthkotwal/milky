//! The message contract between Milky's core and its clients.
//!
//! These types are the Swift ↔ Rust contract (DECISIONS 2026-09-13). They are
//! kept separate from the engine's own types on purpose: `MatchKind` and
//! `Hit` change for ranking reasons, and a ranking change must never
//! silently change what a client receives. Conversion is explicit, below.

use serde::{Deserialize, Serialize};

use crate::candidate::{Action, Kind};
use crate::matching::MatchKind;
use crate::search::Hit;

/// A request from a client. Tagged by `"op"`: `{"op":"search", ...}`.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Search(SearchRequest),
    /// The user picked a result and the launcher tried to act on it. Kept for
    /// clients that do not send `record_events` yet.
    RecordSelection(SelectionEvent),
    /// One launcher session, from invocation to dismissal (DECISIONS
    /// 2026-10-15; wire format in `.agents/swift/INTEGRATION.md`).
    RecordEvents(EventsRequest),
    /// Which app was in front, and when (phase 2).
    RecordFocus(FocusRequest),
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    /// Required. An explicit contract beats a default that can drift.
    pub limit: usize,
}

/// The engine's reply. Tagged by `"kind"`.
///
/// No variant struct may have its own `kind` field: variant fields share the
/// top level with the tag, so it would produce two `kind` keys.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Search(SearchResponse),
    /// A `record_selection` or `record_events` request was validated and
    /// stored durably.
    Recorded,
    Error(ErrorResponse),
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: String,
    /// Opaque and unique across engine runs. Clients echo it in the events
    /// about these results, which joins what was ranked to what the user did.
    pub search_id: String,
    pub results: Vec<ResultItem>,
    /// Whether files and folders were searched. `false` only in the moments
    /// after the engine starts, before the background walk is published.
    /// Additive in contract v4: clients that do not know it ignore it.
    pub files_ready: bool,
}

/// One search result on the wire (contract v4, DECISIONS 2026-10-08).
#[derive(Debug, Serialize)]
pub struct ResultItem {
    /// Identity of the destination: `app:<path>`, `folder:<path>`,
    /// `file:<path>`, `settings:<pane id>`, or `settings:<pane id>#<anchor>`.
    /// Usage and selections attach to this.
    pub id: String,
    /// Nested inside `results`, so it does not collide with the response's
    /// own `kind` tag.
    pub kind: WireKind,
    /// The name that matched best.
    pub title: String,
    pub subtitle: String,
    pub action: WireAction,
    pub match_kind: WireMatchKind,
}

/// Which kind of thing a result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireKind {
    App,
    Setting,
    Folder,
    /// Not produced yet: files join the engine once file matching is gated
    /// (contract v4 already defines them).
    File,
}

/// What picking a result does, tagged by `"type"`:
/// `{"type":"launch","path":...}`, `{"type":"open_url","url":...}`, or
/// `{"type":"open","path":...}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireAction {
    Launch {
        path: String,
    },
    OpenUrl {
        url: String,
    },
    /// Open a file or folder in its default app (Finder for folders).
    Open {
        path: String,
    },
}

/// Match kind as clients see it: a stable name, never a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireMatchKind {
    Exact,
    Prefix,
    WordPrefix,
    Acronym,
    Substring,
    Subsequence,
    /// Only the destination's keywords matched, not its title.
    Keyword,
    /// A file's name matched part of the query and its folders or extension
    /// matched the rest.
    Path,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub reason: ErrorReason,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorReason {
    BadRequest,
    Internal,
    Panic,
}

/// The most results a selection event may report as shown. Bounds log growth;
/// the launcher displays far fewer.
pub const MAX_SHOWN: usize = 100;

/// One completed interaction: what was shown for a query, which result the user
/// picked, and whether acting on it worked. Sent once, after the action finishes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SelectionEvent {
    /// The query as typed when the user acted.
    pub query: String,
    /// Result IDs in the order they were displayed.
    pub shown: Vec<String>,
    /// The ID the user picked. Must be one of `shown`.
    pub selected: String,
    pub outcome: Outcome,
}

/// Whether the launcher's action on a selected result succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The app opened or was brought forward. Only this counts as a launch.
    Opened,
    /// Acting on it failed, for example the app had been deleted.
    Failed,
}

impl SelectionEvent {
    /// Reject events that cannot be internally consistent, before they touch
    /// stored history. The message becomes a `bad_request` error.
    pub fn validate(&self) -> Result<(), String> {
        if self.shown.is_empty() {
            return Err("shown must list at least the selected result".into());
        }
        if self.shown.len() > MAX_SHOWN {
            return Err(format!(
                "shown lists {} results; at most {MAX_SHOWN}",
                self.shown.len()
            ));
        }
        if let Some(bad) = self.shown.iter().find(|id| !is_result_id(id)) {
            return Err(format!("not a result id: {bad:?}"));
        }
        if !self.shown.contains(&self.selected) {
            return Err("selected must be one of shown".into());
        }
        Ok(())
    }
}

/// The most events one session may report, and the longest query text.
pub const MAX_SESSION_EVENTS: usize = 2_000;
pub const MAX_QUERY_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EventsRequest {
    pub session: SessionReport,
}

/// What happened in one launcher session, as the client saw it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionReport {
    /// Made by the client, unique per session. A second report with the same
    /// ID adds to the session (phase 2: dwell after a pick).
    pub session_id: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub invocation: Invocation,
    pub end: EndReason,
    pub context: SessionContext,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Invocation {
    Hotkey,
    Other,
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// Closed after acting on a result.
    Action,
    Escape,
    ClickAway,
    AppSwitch,
    /// The shortcut was pressed again.
    Hotkey,
    Other,
}

/// What surrounded the session when it started.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SessionContext {
    /// The app in front before invoking, as an `app:` ID.
    pub frontmost_app: Option<String>,
    /// Apps with a regular activation policy (no menu-bar or background
    /// agents), as `app:` IDs. Running is not using: focus time (phase 2)
    /// says what was used.
    #[serde(default)]
    pub running_apps: Vec<String>,
    pub timezone_offset_min: Option<i32>,
    pub on_battery: Option<bool>,
    pub displays: Option<u32>,
}

/// One thing that happened in a session. `t_ms` counts from the session's
/// start.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// The query text after any change.
    Query { t_ms: u64, query: String },
    /// The results of `search_id` were drawn; `visible` rows were on screen.
    Rendered { t_ms: u64, search_id: String, visible: u32 },
    /// The selected row changed.
    Highlight { t_ms: u64, search_id: String, result_id: String, index: u32 },
    /// Tab accepted a completion from this result.
    TabComplete { t_ms: u64, search_id: String, result_id: String },
    /// After an action opened something: how long the app that came
    /// forward stayed in front (phase 2). Sent in a later report for the
    /// same session.
    Dwell {
        t_ms: u64,
        result_id: String,
        app: Option<String>,
        front_ms: u64,
        ended: DwellEnd,
    },
    /// The user acted on a result.
    Action {
        t_ms: u64,
        action: UserAction,
        search_id: String,
        result_id: String,
        index: u32,
        outcome: Outcome,
    },
}

impl SessionEvent {
    pub fn t_ms(&self) -> u64 {
        match self {
            SessionEvent::Query { t_ms, .. }
            | SessionEvent::Rendered { t_ms, .. }
            | SessionEvent::Highlight { t_ms, .. }
            | SessionEvent::TabComplete { t_ms, .. }
            | SessionEvent::Dwell { t_ms, .. }
            | SessionEvent::Action { t_ms, .. } => *t_ms,
        }
    }

    /// The search this event is about, if any.
    pub fn search_id(&self) -> Option<&str> {
        match self {
            SessionEvent::Query { .. } | SessionEvent::Dwell { .. } => None,
            SessionEvent::Rendered { search_id, .. }
            | SessionEvent::Highlight { search_id, .. }
            | SessionEvent::TabComplete { search_id, .. }
            | SessionEvent::Action { search_id, .. } => Some(search_id),
        }
    }

    /// The result this event is about, if any.
    pub fn result_id(&self) -> Option<&str> {
        match self {
            SessionEvent::Query { .. } | SessionEvent::Rendered { .. } => None,
            SessionEvent::Highlight { result_id, .. }
            | SessionEvent::TabComplete { result_id, .. }
            | SessionEvent::Dwell { result_id, .. }
            | SessionEvent::Action { result_id, .. } => Some(result_id),
        }
    }
}

/// How a dwell ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DwellEnd {
    /// Another app came forward.
    Switched,
    Hidden,
    Quit,
    /// No input for two minutes; the dwell ends at the last input.
    Idle,
    /// Screen sleep or lock.
    Sleep,
    /// Thirty minutes reached.
    Limit,
}

/// The most focus intervals one message may carry, and the longest one.
pub const MAX_FOCUS_INTERVALS: usize = 1_000;
pub const MAX_FOCUS_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FocusRequest {
    pub intervals: Vec<FocusInterval>,
}

/// One stretch of an app being in front while the user was there.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FocusInterval {
    pub app: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub ended_by: FocusEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusEnd {
    Switch,
    Idle,
    Sleep,
    Lock,
    Quit,
    Shutdown,
}

impl FocusRequest {
    /// Reject batches that cannot be consistent, before they are stored.
    pub fn validate(&self) -> Result<(), String> {
        if self.intervals.len() > MAX_FOCUS_INTERVALS {
            return Err(format!("{} intervals; at most {MAX_FOCUS_INTERVALS}", self.intervals.len()));
        }
        for interval in &self.intervals {
            if !interval.app.starts_with("app:/") {
                return Err(format!("not an app id: {:?}", interval.app));
            }
            if interval.ended_at_ms < interval.started_at_ms {
                return Err("an interval ends before it starts".into());
            }
            if interval.ended_at_ms - interval.started_at_ms > MAX_FOCUS_MS {
                return Err("an interval is longer than 24 hours".into());
            }
        }
        Ok(())
    }
}

/// What the user did with a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserAction {
    Open,
    Reveal,
    CopyPath,
    Inspect,
    /// The action menu was opened on it.
    Menu,
}

impl UserAction {
    /// Whether acting this way counts as using the result for ranking:
    /// opening it, revealing it, or copying its path, but not just looking.
    pub fn counts_as_use(self) -> bool {
        matches!(self, UserAction::Open | UserAction::Reveal | UserAction::CopyPath)
    }
}

impl SessionReport {
    /// Reject reports that cannot be consistent, before they touch stored
    /// history. The message becomes a `bad_request` error.
    pub fn validate(&self) -> Result<(), String> {
        if self.session_id.is_empty() || self.session_id.len() > 100 {
            return Err("session_id must be 1 to 100 characters".into());
        }
        if self.ended_at_ms < self.started_at_ms {
            return Err("ended_at_ms is before started_at_ms".into());
        }
        if self.events.len() > MAX_SESSION_EVENTS {
            return Err(format!("{} events; at most {MAX_SESSION_EVENTS}", self.events.len()));
        }
        let apps = self.context.frontmost_app.iter().chain(&self.context.running_apps);
        if let Some(bad) = apps.into_iter().find(|id| !id.starts_with("app:/")) {
            return Err(format!("not an app id: {bad:?}"));
        }
        let mut last = 0;
        for event in &self.events {
            if event.t_ms() < last {
                return Err("events are not in time order".into());
            }
            last = event.t_ms();
            if let SessionEvent::Query { query, .. } = event
                && query.chars().count() > MAX_QUERY_CHARS
            {
                return Err(format!("a query is longer than {MAX_QUERY_CHARS} characters"));
            }
            if event.search_id().is_some_and(str::is_empty) {
                return Err("an event has an empty search_id".into());
            }
            if let Some(bad) = event.result_id().filter(|id| !is_result_id(id)) {
                return Err(format!("not a result id: {bad:?}"));
            }
            if let SessionEvent::Dwell { app: Some(app), .. } = event
                && !app.starts_with("app:/")
            {
                return Err(format!("not an app id: {app:?}"));
            }
        }
        Ok(())
    }
}

/// `app:`, `folder:`, or `file:` followed by an absolute path, or
/// `settings:<pane id>` with an optional `#<anchor>`.
fn is_result_id(id: &str) -> bool {
    match id.split_once(':') {
        Some(("app" | "folder" | "file", rest)) => rest.starts_with('/'),
        Some(("settings", rest)) => !rest.is_empty() && !rest.starts_with('#'),
        _ => false,
    }
}

/// Sent if encoding a response ever fails, so a client always gets valid JSON.
pub const ENCODE_FAILURE: &str =
    r#"{"kind":"error","reason":"internal","message":"failed to encode response"}"#;

impl From<MatchKind> for WireMatchKind {
    fn from(kind: MatchKind) -> Self {
        match kind {
            MatchKind::Subsequence => WireMatchKind::Subsequence,
            MatchKind::Substring => WireMatchKind::Substring,
            MatchKind::Acronym => WireMatchKind::Acronym,
            MatchKind::WordPrefix => WireMatchKind::WordPrefix,
            MatchKind::Prefix => WireMatchKind::Prefix,
            MatchKind::Exact => WireMatchKind::Exact,
        }
    }
}

impl From<Kind> for WireKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::App => WireKind::App,
            Kind::Setting => WireKind::Setting,
            Kind::Place | Kind::Folder => WireKind::Folder,
            Kind::File => WireKind::File,
        }
    }
}

/// Convert a search hit into a wire result.
///
/// Returns `None` if an app path is not valid UTF-8: a lossily converted path
/// would be an action clients could not perform. Matching on `Action` here is
/// exhaustive, so a new action kind cannot compile until the wire handles it.
pub fn result_item(hit: &Hit) -> Option<ResultItem> {
    let candidate = hit.candidate;
    let action = match &candidate.action {
        Action::Launch(path) => WireAction::Launch {
            path: path.to_str()?.to_string(),
        },
        Action::OpenUrl(url) => WireAction::OpenUrl { url: url.clone() },
        Action::Open(path) => WireAction::Open {
            path: path.to_str()?.to_string(),
        },
    };
    Some(ResultItem {
        id: candidate.id.clone(),
        kind: candidate.kind.into(),
        title: hit.title.to_string(),
        subtitle: candidate.subtitle.clone(),
        action,
        match_kind: match hit.kind {
            Some(kind) => kind.into(),
            None if hit.candidate.file.is_some() => WireMatchKind::Path,
            None => WireMatchKind::Keyword,
        },
    })
}

/// Serialize an error response. For failures that happen before a request
/// reaches the engine, like a null or non-UTF-8 request at the C boundary.
pub fn error_json(reason: ErrorReason, message: &str) -> String {
    let response = Response::Error(ErrorResponse {
        reason,
        message: message.to_string(),
    });
    serde_json::to_string(&response).unwrap_or_else(|_| ENCODE_FAILURE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use crate::apps::App;
    use crate::candidate::Candidate;
    use crate::settings::{SettingsItem, SettingsPane};

    #[test]
    fn wire_kinds_have_stable_snake_case_names() {
        assert_eq!(
            serde_json::to_value(WireMatchKind::WordPrefix).unwrap(),
            "word_prefix"
        );
        assert_eq!(
            serde_json::to_value(WireMatchKind::Subsequence).unwrap(),
            "subsequence"
        );
    }

    #[test]
    fn match_kinds_convert_by_name_not_position() {
        assert_eq!(WireMatchKind::from(MatchKind::Exact), WireMatchKind::Exact);
        assert_eq!(
            WireMatchKind::from(MatchKind::Acronym),
            WireMatchKind::Acronym
        );
        assert_eq!(
            WireMatchKind::from(MatchKind::Subsequence),
            WireMatchKind::Subsequence
        );
    }

    fn hit_for(candidate: &Candidate) -> Hit<'_> {
        Hit {
            candidate,
            title: candidate.title(),
            kind: Some(MatchKind::Exact),
            keyword: None,
            tier: crate::search::Tier::Exact,
            usage: 0.0,
        }
    }

    #[test]
    fn an_app_hit_becomes_a_launch_result() {
        let app = App::new(
            PathBuf::from("/Applications/Python 3.12/IDLE.app"),
            "IDLE".into(),
        );
        let candidate = crate::candidate::from_app(&app).unwrap();
        let item = result_item(&hit_for(&candidate)).expect("utf-8 path");
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::json!({
                "id": "app:/Applications/Python 3.12/IDLE.app",
                "kind": "app",
                "title": "IDLE",
                "subtitle": "Python 3.12",
                "action": {"type": "launch", "path": "/Applications/Python 3.12/IDLE.app"},
                "match_kind": "exact"
            })
        );
    }

    #[test]
    fn a_settings_hit_becomes_an_open_url_result() {
        let pane = SettingsPane {
            id: "com.apple.wifi-settings-extension".into(),
            name: "Wi-Fi".into(),
            in_sidebar: true,
            items: vec![SettingsItem {
                anchor: "Advanced".into(),
                title: "Wi-Fi MAC Address".into(),
                keywords: vec![],
            }],
        };
        let candidates = crate::candidate::from_settings(&pane);
        let item = result_item(&hit_for(&candidates[1])).unwrap();
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::json!({
                "id": "settings:com.apple.wifi-settings-extension#Advanced",
                "kind": "setting",
                "title": "Wi-Fi MAC Address",
                "subtitle": "Wi-Fi",
                "action": {"type": "open_url", "url": "x-apple.systempreferences:com.apple.wifi-settings-extension?Advanced"},
                "match_kind": "exact"
            })
        );
    }

    #[test]
    fn a_place_hit_becomes_an_open_result() {
        let place = crate::places::Place {
            path: PathBuf::from("/Users/someone/Downloads"),
            name: "Downloads".into(),
            aliases: vec![],
        };
        let candidate =
            crate::candidate::from_place(&place, std::path::Path::new("/Users/someone")).unwrap();
        let item = result_item(&hit_for(&candidate)).unwrap();
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::json!({
                "id": "folder:/Users/someone/Downloads",
                "kind": "folder",
                "title": "Downloads",
                "subtitle": "~/Downloads",
                "action": {"type": "open", "path": "/Users/someone/Downloads"},
                "match_kind": "exact"
            })
        );
    }

    #[test]
    fn a_file_hit_becomes_a_file_result() {
        let entry = crate::files::FileEntry {
            path: PathBuf::from("/Users/someone/notes/todo.txt"),
            is_folder: false,
            modified: None,
            last_used: None,
        };
        let candidate =
            crate::candidate::from_file(&entry, std::path::Path::new("/Users/someone")).unwrap();
        let item = result_item(&hit_for(&candidate)).unwrap();
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::json!({
                "id": "file:/Users/someone/notes/todo.txt",
                "kind": "file",
                "title": "todo.txt",
                "subtitle": "~/notes",
                "action": {"type": "open", "path": "/Users/someone/notes/todo.txt"},
                "match_kind": "exact"
            })
        );
    }

    #[test]
    fn a_non_utf8_app_path_is_skipped_not_mangled() {
        // 0xFF can never appear in UTF-8.
        let path = PathBuf::from(OsStr::from_bytes(b"/Applications/\xff.app"));
        let candidate = Candidate {
            id: "app:/Applications/broken.app".into(),
            kind: Kind::App,
            subtitle: String::new(),
            action: Action::Launch(path),
            names: vec![crate::candidate::Name::new("Broken")],
            keywords: Vec::new(),
            file: None,
        };
        assert!(result_item(&hit_for(&candidate)).is_none());
    }

    fn selection(selected: &str, shown: &[&str]) -> SelectionEvent {
        SelectionEvent {
            query: "c".into(),
            shown: shown.iter().map(|s| s.to_string()).collect(),
            selected: selected.into(),
            outcome: Outcome::Opened,
        }
    }

    #[test]
    fn record_selection_parses_from_the_contract_shape() {
        let json = r#"{"op":"record_selection","query":"c",
            "shown":["app:/System/Applications/Chess.app","app:/System/Applications/Calendar.app"],
            "selected":"app:/System/Applications/Calendar.app","outcome":"opened"}"#;
        match serde_json::from_str::<Request>(json).expect("valid request") {
            Request::RecordSelection(event) => {
                assert_eq!(event.selected, "app:/System/Applications/Calendar.app");
                assert_eq!(event.outcome, Outcome::Opened);
                assert_eq!(event.shown.len(), 2);
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn recorded_serializes_as_a_bare_kind() {
        assert_eq!(
            serde_json::to_string(&Response::Recorded).unwrap(),
            r#"{"kind":"recorded"}"#
        );
    }

    #[test]
    fn a_consistent_selection_validates() {
        let event = selection("app:/A.app", &["app:/B.app", "app:/A.app"]);
        assert_eq!(event.validate(), Ok(()));
        let section = "settings:com.apple.wifi-settings-extension#Advanced";
        assert_eq!(selection(section, &[section]).validate(), Ok(()));
        for id in [
            "folder:/Users/someone/Downloads",
            "file:/Users/someone/notes.txt",
        ] {
            assert_eq!(selection(id, &[id]).validate(), Ok(()), "{id}");
        }
    }

    #[test]
    fn selected_must_have_been_shown() {
        assert!(
            selection("app:/C.app", &["app:/A.app", "app:/B.app"])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn shown_must_not_be_empty() {
        assert!(selection("app:/A.app", &[]).validate().is_err());
    }

    #[test]
    fn only_result_ids_are_accepted() {
        for bad in [
            "/A.app",
            "app:A.app",
            "settings:",
            "settings:#Advanced",
            "files:/A",
            "A.app",
            "folder:Downloads",
        ] {
            assert!(
                selection(bad, &[bad]).validate().is_err(),
                "{bad} should be rejected"
            );
        }
    }

    /// The example in `.agents/swift/INTEGRATION.md` ("Interaction events"),
    /// without its comments: the documented format must parse and validate.
    const DOCUMENTED_SESSION: &str = r#"{"op":"record_events","session":{
      "session_id":"6F1C","started_at_ms":1792040000000,"ended_at_ms":1792040004200,
      "invocation":"hotkey","end":"action",
      "context":{"frontmost_app":"app:/Applications/Ghostty.app",
                 "running_apps":["app:/Applications/Ghostty.app","app:/Applications/Google Chrome.app"],
                 "timezone_offset_min":-420,"on_battery":false,"displays":2},
      "events":[
        {"t_ms":0,   "type":"query", "query":""},
        {"t_ms":180, "type":"query", "query":"s"},
        {"t_ms":215, "type":"rendered", "search_id":"s1f", "visible":8},
        {"t_ms":260, "type":"query", "query":"sa"},
        {"t_ms":290, "type":"rendered", "search_id":"s20", "visible":8},
        {"t_ms":700, "type":"highlight", "search_id":"s20", "result_id":"app:/Applications/Safari.app", "index":1},
        {"t_ms":900, "type":"action", "action":"open", "search_id":"s20",
         "result_id":"app:/Applications/Safari.app", "index":1, "outcome":"opened"}
      ]}}"#;

    fn documented() -> SessionReport {
        match serde_json::from_str::<Request>(DOCUMENTED_SESSION).expect("the documented format parses") {
            Request::RecordEvents(request) => request.session,
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn the_documented_session_parses_and_validates() {
        let report = documented();
        assert_eq!(report.end, EndReason::Action);
        assert_eq!(report.events.len(), 7);
        assert_eq!(report.validate(), Ok(()));
    }

    #[test]
    fn every_end_reason_and_action_has_its_documented_name() {
        for end in ["action", "escape", "click_away", "app_switch", "hotkey", "other"] {
            assert!(serde_json::from_str::<EndReason>(&format!("\"{end}\"")).is_ok(), "{end}");
        }
        for action in ["open", "reveal", "copy_path", "inspect", "menu"] {
            assert!(serde_json::from_str::<UserAction>(&format!("\"{action}\"")).is_ok(), "{action}");
        }
        assert!(serde_json::from_str::<SessionEvent>(r#"{"t_ms":1,"type":"tab_complete","search_id":"s","result_id":"app:/A.app"}"#).is_ok());
    }

    #[test]
    fn inconsistent_sessions_are_rejected() {
        let mut report = documented();
        report.ended_at_ms = report.started_at_ms - 1;
        assert!(report.validate().is_err(), "ends before it starts");

        let mut report = documented();
        report.events.swap(0, 6);
        assert!(report.validate().is_err(), "events out of time order");

        let mut report = documented();
        report.context.running_apps.push("settings:com.apple.wifi".into());
        assert!(report.validate().is_err(), "a running app that is not an app");

        let mut report = documented();
        report.events.push(SessionEvent::Query { t_ms: 5_000, query: "x".repeat(MAX_QUERY_CHARS + 1) });
        assert!(report.validate().is_err(), "an overlong query");

        let mut report = documented();
        if let SessionEvent::Highlight { result_id, .. } = &mut report.events[5] {
            *result_id = "Safari".into();
        }
        assert!(report.validate().is_err(), "a malformed result id");

        let mut report = documented();
        report.session_id.clear();
        assert!(report.validate().is_err(), "no session id");
    }

    #[test]
    fn the_documented_phase_two_messages_parse_and_validate() {
        let dwell: SessionEvent = serde_json::from_str(
            r#"{"t_ms":4300, "type":"dwell", "result_id":"app:/Applications/Safari.app",
                "app":"app:/Applications/Safari.app", "front_ms":3400, "ended":"switched"}"#,
        )
        .unwrap();
        assert!(matches!(dwell, SessionEvent::Dwell { front_ms: 3400, ended: DwellEnd::Switched, .. }));
        let mut report = documented();
        report.events = vec![dwell];
        assert_eq!(report.validate(), Ok(()));

        let focus = r#"{"op":"record_focus","intervals":[
            {"app":"app:/Applications/Ghostty.app","started_at_ms":1792040000000,
             "ended_at_ms":1792040312000,"ended_by":"switch"}]}"#;
        match serde_json::from_str::<Request>(focus).unwrap() {
            Request::RecordFocus(request) => assert_eq!(request.validate(), Ok(())),
            other => panic!("parsed as {other:?}"),
        }
        for end in ["switch", "idle", "sleep", "lock", "quit", "shutdown"] {
            assert!(serde_json::from_str::<FocusEnd>(&format!("\"{end}\"")).is_ok(), "{end}");
        }
    }

    #[test]
    fn bad_focus_intervals_are_rejected() {
        let interval = |app: &str, start: u64, end: u64| FocusInterval {
            app: app.into(),
            started_at_ms: start,
            ended_at_ms: end,
            ended_by: FocusEnd::Switch,
        };
        let check = |intervals| FocusRequest { intervals }.validate();
        assert!(check(vec![interval("Safari", 0, 1)]).is_err(), "not an app id");
        assert!(check(vec![interval("app:/A.app", 10, 1)]).is_err(), "ends before it starts");
        assert!(check(vec![interval("app:/A.app", 0, MAX_FOCUS_MS + 1)]).is_err(), "over a day");
        assert!(check(vec![interval("app:/A.app", 0, 1); MAX_FOCUS_INTERVALS + 1]).is_err(), "too many");
        assert_eq!(check(vec![interval("app:/A.app", 0, 1)]), Ok(()));
    }

    #[test]
    fn only_using_a_result_counts_not_looking_at_it() {
        assert!(UserAction::Open.counts_as_use() && UserAction::Reveal.counts_as_use() && UserAction::CopyPath.counts_as_use());
        assert!(!UserAction::Inspect.counts_as_use() && !UserAction::Menu.counts_as_use());
    }

    #[test]
    fn oversized_shown_lists_are_rejected() {
        let many: Vec<String> = (0..=MAX_SHOWN).map(|i| format!("app:/{i}.app")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(selection("app:/0.app", &refs).validate().is_err());
    }
}
