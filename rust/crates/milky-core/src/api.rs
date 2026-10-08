//! The message contract between Milky's core and its clients.
//!
//! These types are the Swift ↔ Rust contract (DECISIONS 2026-09-13). They are
//! kept separate from the engine's own types on purpose: `MatchKind` and
//! `AppMatch` change for ranking reasons, and a ranking change must never
//! silently change what a client receives. Conversion is explicit, below.

use serde::{Deserialize, Serialize};

use crate::matching::MatchKind;
use crate::search::AppMatch;

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

#[derive(Debug, Serialize)]
pub struct ResultItem {
    /// Identity. App names are not unique.
    pub path: String,
    pub name: String,
    pub match_kind: WireMatchKind,
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
    /// Result paths in the order they were displayed.
    pub shown: Vec<String>,
    /// The path the user picked. Must be one of `shown`.
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
        if !self.shown.iter().all(|path| path.starts_with('/')) {
            return Err("every shown path must be absolute".into());
        }
        if !self.shown.contains(&self.selected) {
            return Err("selected must be one of shown".into());
        }
        Ok(())
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

/// Convert an engine match into a wire result.
///
/// Returns `None` if the path is not valid UTF-8: a lossily converted path
/// would be a broken identity that clients could not open.
pub fn result_item(m: &AppMatch) -> Option<ResultItem> {
    let path = m.path.to_str()?.to_string();
    Some(ResultItem {
        path,
        name: m.name.clone(),
        match_kind: m.kind.into(),
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

    #[test]
    fn a_utf8_path_becomes_a_result() {
        let m = AppMatch {
            name: "Safari".into(),
            path: PathBuf::from("/Applications/Safari.app"),
            kind: MatchKind::Exact,
            usage: 0.0,
        };
        let item = result_item(&m).expect("utf-8 path should convert");
        assert_eq!(item.path, "/Applications/Safari.app");
        assert_eq!(item.match_kind, WireMatchKind::Exact);
    }

    #[test]
    fn a_non_utf8_path_is_skipped_not_mangled() {
        // 0xFF can never appear in UTF-8.
        let path = PathBuf::from(OsStr::from_bytes(b"/Applications/\xff.app"));
        let m = AppMatch {
            name: "Broken".into(),
            path,
            kind: MatchKind::Prefix,
            usage: 0.0,
        };
        assert!(result_item(&m).is_none());
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
            "shown":["/System/Applications/Chess.app","/System/Applications/Calendar.app"],
            "selected":"/System/Applications/Calendar.app","outcome":"opened"}"#;
        match serde_json::from_str::<Request>(json).expect("valid request") {
            Request::RecordSelection(event) => {
                assert_eq!(event.selected, "/System/Applications/Calendar.app");
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
        let event = selection("/A.app", &["/B.app", "/A.app"]);
        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn selected_must_have_been_shown() {
        assert!(
            selection("/C.app", &["/A.app", "/B.app"])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn shown_must_not_be_empty() {
        assert!(selection("/A.app", &[]).validate().is_err());
    }

    #[test]
    fn relative_paths_are_rejected() {
        assert!(selection("A.app", &["A.app"]).validate().is_err());
    }

    #[test]
    fn oversized_shown_lists_are_rejected() {
        let many: Vec<String> = (0..=MAX_SHOWN).map(|i| format!("/{i}.app")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(selection("/0.app", &refs).validate().is_err());
    }
}
