//! The message-shaped API between Milky's core and whatever is driving it.
//!
//! Every interaction is one [`Request`] in, one [`Response`] out. There is no
//! shared mutable state across the boundary and no callbacks, which is what
//! lets the same API work in-process today and over a socket later.
//!
//! Wire format is JSON for now. It is easy to read while debugging and easy to
//! hand-write in tests. If serialization ever shows up in a latency profile we
//! can swap the encoding without changing these types.

use serde::{Deserialize, Serialize};

/// Version of the request/response contract itself.
///
/// Bump this whenever an existing message changes shape in a way an older
/// client would misread. Adding a new variant or a new optional field does not
/// require a bump.
pub const API_VERSION: u32 = 1;

/// Something a client asks the engine to do.
///
/// Serializes with an `"op"` discriminator, e.g. `{"op":"search","query":"ter"}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Cheap liveness/version probe. Used by clients at startup.
    Health,
    /// Run a search.
    Search(SearchRequest),
}

/// The engine's answer.
///
/// Serializes with a `"kind"` discriminator. Note that a failed request is a
/// normal `Response::Error`, not a transport-level failure: the boundary itself
/// only fails if the engine is gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Health(HealthResponse),
    Search(SearchResponse),
    Error(ErrorResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    /// Raw text as typed by the user. Not normalized yet; that is the engine's job.
    pub query: String,
    /// Maximum results wanted. Falls back to the engine's configured default.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Echoed back so a client can discard responses for stale keystrokes.
    pub query: String,
    /// Engine-side time for this request. Our first, crudest latency signal.
    pub took_micros: u64,
    pub candidates: Vec<Candidate>,
}

/// One search result.
///
/// PROVISIONAL. The real candidate/result/action model is a separate design
/// task (see `.agents/TASKS.md`); this is the minimum needed to prove the
/// Swift/Rust boundary end to end. Expect it to be replaced wholesale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// Stable identifier for this candidate, used later to log selections.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub kind: CandidateKind,
    pub score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Application,
    File,
    Action,
    WebSearch,
    /// Synthetic result that exists only while the engine is a skeleton.
    Placeholder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// `milky-core` crate version.
    pub engine_version: String,
    /// [`API_VERSION`] as compiled into the engine.
    pub api_version: u32,
    /// False while the engine is up but not yet able to serve real results.
    pub ready: bool,
    /// Human-readable note about what is missing when `ready` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Why it failed. Named `reason` rather than `kind` because [`Response`] is
    /// internally tagged on `"kind"`, and two `kind` keys in one JSON object is
    /// a silent footgun.
    pub reason: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The request could not be parsed or was semantically invalid.
    BadRequest,
    /// The engine failed while handling a valid request.
    Internal,
    /// A Rust panic was caught at the boundary. Always a bug in Milky.
    Panic,
}

impl Response {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Response::Error(ErrorResponse {
            reason: ErrorKind::BadRequest,
            message: message.into(),
        })
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Response::Error(ErrorResponse {
            reason: ErrorKind::Internal,
            message: message.into(),
        })
    }
}
