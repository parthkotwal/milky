//! The message contract between Milky's core and its clients.
//!
//! These types are the Swift ↔ Rust contract (DECISIONS 2026-09-13). They are
//! kept separate from the engine's own types on purpose: `MatchKind` and
//! `Hit` change for ranking reasons, and a ranking change must never
//! silently change what a client receives. Conversion is explicit, below.

use serde::{Deserialize, Serialize};

use crate::matching::MatchKind;
use crate::candidate::{Action, Kind};
use crate::search::Hit;

/// A request from a client. Tagged by `"op"`: `{"op":"search", ...}`.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Search(SearchRequest),
    /// The user picked a result and the launcher tried to act on it.
    RecordSelection(SelectionEvent),
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
    /// A `record_selection` event was validated and stored durably.
    Recorded,
    Error(ErrorResponse),
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<ResultItem>,
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
    /// Not produced yet; file search arrives in the same contract (v4).
    File,
}

/// What picking a result does, tagged by `"type"`:
/// `{"type":"launch","path":...}`, `{"type":"open_url","url":...}`, or
/// `{"type":"open","path":...}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireAction {
    Launch { path: String },
    OpenUrl { url: String },
    /// Open a file or folder in its default app (Finder for folders).
    Open { path: String },
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
            Kind::Place => WireKind::Folder,
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
        Action::Launch(path) => WireAction::Launch { path: path.to_str()?.to_string() },
        Action::OpenUrl(url) => WireAction::OpenUrl { url: url.clone() },
        Action::Open(path) => WireAction::Open { path: path.to_str()?.to_string() },
    };
    Some(ResultItem {
        id: candidate.id.clone(),
        kind: candidate.kind.into(),
        title: hit.title.to_string(),
        subtitle: candidate.subtitle.clone(),
        action,
        match_kind: hit.kind.map_or(WireMatchKind::Keyword, WireMatchKind::from),
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
        let app = App::new(PathBuf::from("/Applications/Python 3.12/IDLE.app"), "IDLE".into());
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
        let candidate = crate::candidate::from_place(&place, std::path::Path::new("/Users/someone")).unwrap();
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
        for id in ["folder:/Users/someone/Downloads", "file:/Users/someone/notes.txt"] {
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
        for bad in ["/A.app", "app:A.app", "settings:", "settings:#Advanced", "files:/A", "A.app", "folder:Downloads"] {
            assert!(selection(bad, &[bad]).validate().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn oversized_shown_lists_are_rejected() {
        let many: Vec<String> = (0..=MAX_SHOWN).map(|i| format!("app:/{i}.app")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(selection("app:/0.app", &refs).validate().is_err());
    }
}
