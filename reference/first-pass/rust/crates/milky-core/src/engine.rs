//! The engine: long-lived state plus one entry point per request.

use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::api::{
    Candidate, CandidateKind, ErrorKind, ErrorResponse, HealthResponse, Request, Response,
    SearchRequest, SearchResponse, API_VERSION,
};

/// Engine configuration, supplied once at construction.
///
/// Deliberately small. Options get added when something actually needs them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Where indexes and usage history will live. `None` means "not yet chosen",
    /// which is fine while the engine has nothing to persist.
    pub data_dir: Option<PathBuf>,
    /// Default result cap when a request does not specify one.
    pub default_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: None,
            default_limit: 20,
        }
    }
}

/// Failure while constructing the engine. Distinct from a per-request error:
/// this means we do not have a usable engine at all.
#[derive(Debug)]
pub enum EngineError {
    InvalidConfig(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::InvalidConfig(msg) => write!(f, "invalid config: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Long-lived search engine.
///
/// One of these is created when the app launches and kept warm for the whole
/// session: a launcher cannot afford to build state per keystroke.
///
/// `handle` takes `&self`, not `&mut self`, so concurrent queries stay possible
/// without redesigning the API. Anything mutable inside will use interior
/// mutability with its own locking.
pub struct Engine {
    config: Config,
    started: Instant,
}

impl Engine {
    pub fn new(config: Config) -> Result<Self, EngineError> {
        if config.default_limit == 0 {
            return Err(EngineError::InvalidConfig(
                "default_limit must be greater than 0".into(),
            ));
        }
        Ok(Self {
            config,
            started: Instant::now(),
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// How long this engine has been alive. Useful for confirming a client is
    /// reusing a warm engine rather than building a new one per query.
    pub fn uptime_micros(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    /// Handle one typed request.
    pub fn handle(&self, request: Request) -> Response {
        match request {
            Request::Health => Response::Health(self.health()),
            Request::Search(req) => match self.search(req) {
                Ok(resp) => Response::Search(resp),
                Err(err) => Response::internal(err.to_string()),
            },
        }
    }

    /// Handle one request in its serialized form.
    ///
    /// This is the single funnel every transport uses, so parsing rules and
    /// error shapes cannot drift between the FFI shim, the CLI, and any future
    /// daemon. It returns a `String` rather than a `Result` on purpose: a
    /// malformed request is a normal `Response::Error`, and callers across a C
    /// boundary should never have to distinguish two failure channels.
    pub fn handle_json(&self, request_json: &str) -> String {
        let response = match serde_json::from_str::<Request>(request_json) {
            Ok(request) => self.handle(request),
            Err(err) => Response::bad_request(format!("could not parse request: {err}")),
        };
        encode_response(&response)
    }

    fn health(&self) -> HealthResponse {
        HealthResponse {
            engine_version: crate::VERSION.to_string(),
            api_version: API_VERSION,
            ready: false,
            note: Some(
                "skeleton engine: no sources are indexed yet, search returns placeholders".into(),
            ),
        }
    }

    fn search(&self, req: SearchRequest) -> Result<SearchResponse, EngineError> {
        let started = Instant::now();
        let limit = req.limit.unwrap_or(self.config.default_limit);
        let query = req.query.trim().to_string();

        // PROVISIONAL. Real candidate generation (apps, files, lexical,
        // semantic) lands with the source implementations. For now this exists
        // so the whole path — Swift, C ABI, JSON, engine, and back — can be
        // exercised and measured.
        let candidates = if query.is_empty() {
            Vec::new()
        } else {
            vec![
                Candidate {
                    id: format!("placeholder:echo:{query}"),
                    title: query.clone(),
                    subtitle: Some("placeholder result from milky-core".into()),
                    kind: CandidateKind::Placeholder,
                    score: 1.0,
                },
                Candidate {
                    id: format!("web:search:{query}"),
                    title: format!("Search the web for \"{query}\""),
                    subtitle: Some("web-search fallback".into()),
                    kind: CandidateKind::WebSearch,
                    score: 0.1,
                },
            ]
            .into_iter()
            .take(limit)
            .collect()
        };

        Ok(SearchResponse {
            query: req.query,
            took_micros: started.elapsed().as_micros() as u64,
            candidates,
        })
    }
}

/// Parse a [`Config`] from JSON, flattening serde's error into a plain string.
///
/// Exposed so transports that must not depend on `serde_json` — the C ABI shim,
/// for one — can still accept a config payload.
pub fn parse_config(json: &str) -> Result<Config, String> {
    serde_json::from_str(json).map_err(|err| format!("invalid config json: {err}"))
}

/// Serialize a response, falling back to a hand-built error payload.
///
/// Encoding our own types should never fail, but the fallback means a client
/// always receives valid JSON rather than an empty string it has to guess at.
fn encode_response(response: &Response) -> String {
    serde_json::to_string(response).unwrap_or_else(|err| {
        let fallback = Response::Error(ErrorResponse {
            reason: ErrorKind::Internal,
            message: format!("failed to encode response: {err}"),
        });
        serde_json::to_string(&fallback).unwrap_or_else(|_| {
            r#"{"kind":"error","reason":"internal","message":"failed to encode response"}"#
                .to_string()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        Engine::new(Config::default()).expect("default config is valid")
    }

    #[test]
    fn rejects_zero_limit_config() {
        let config = Config {
            default_limit: 0,
            ..Config::default()
        };
        assert!(Engine::new(config).is_err());
    }

    #[test]
    fn health_reports_api_version() {
        let out = engine().handle_json(r#"{"op":"health"}"#);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["kind"], "health");
        assert_eq!(value["api_version"], API_VERSION);
    }

    #[test]
    fn search_echoes_query_and_respects_limit() {
        let out = engine().handle_json(r#"{"op":"search","query":"terminal","limit":1}"#);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["kind"], "search");
        assert_eq!(value["query"], "terminal");
        assert_eq!(value["candidates"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn blank_query_returns_no_candidates() {
        let out = engine().handle_json(r#"{"op":"search","query":"   "}"#);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(value["candidates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn malformed_request_is_a_bad_request_response() {
        let out = engine().handle_json("not json at all");
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["kind"], "error");
        assert_eq!(value["reason"], "bad_request");
    }

    #[test]
    fn unknown_op_is_a_bad_request_response() {
        let out = engine().handle_json(r#"{"op":"teleport"}"#);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["kind"], "error");
        assert_eq!(value["reason"], "bad_request");
    }
}
