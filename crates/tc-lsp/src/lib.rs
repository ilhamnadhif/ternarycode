//! TernaryCode's language server client (Language Server Protocol), for the editor's IDE
//! features: go to definition, references, rename, code actions, completion, hover, signature
//! help, diagnostics, formatting and symbols.
//!
//! - `transport`: the base protocol framing (`Content-Length` headers around JSON).
//! - `client`: one server process, requests and notifications over its stdio, run on the app's
//!   tokio runtime; everything it receives goes to a callback (the app's message channel).
//! - `servers`: the built-in list of servers per language, where a project's root is, and the
//!   user's `PATH` (an app opened from Finder does not get it).
//! - `pos`: char indices ↔ LSP positions (UTF-16 or UTF-8 columns).
//! - `edit`: text and workspace edits, applied to strings.
//! - `assist`: completion filtering, snippets as plain text, hover markdown as text.
//!
//! Pure Rust (PRD section 16): `lsp-types` for the protocol's types, `serde_json`, `tokio`.

pub mod assist;
pub mod client;
pub mod edit;
pub mod pos;
pub mod servers;
pub mod transport;

pub use lsp_types as types;
