//! Milky's search engine core.
//!
//! This crate is plain Rust. It knows nothing about C, Swift, sockets, or any
//! other transport. Everything that talks to it — the FFI shim, the dev CLI,
//! and any future daemon — goes through the same request/response API in
//! [`api`], usually via [`Engine::handle_json`].
//!
//! Keeping the transport out of this crate is deliberate: it means we can move
//! the engine in-process, out-of-process, or behind a socket later without
//! touching engine code. See `.agents/DECISIONS.md`.

pub mod api;
pub mod engine;

pub use api::{Request, Response};
pub use engine::{Config, Engine, EngineError};

/// Version of this crate, from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
