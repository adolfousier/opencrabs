//! ACP (Agent Client Protocol) server mode: `opencrabs acp`.
//!
//! Speaks JSON-RPC 2.0 over stdio, newline-delimited, so agent-aware editors
//! (MonoCode, Zed, anything ACP) can drive OpenCrabs as a harness: one child
//! process per conversation thread, `session/prompt` runs the full tool loop,
//! progress streams back as `session/update` notifications, and approvals
//! round-trip to the client as `session/request_permission` requests.
//!
//! stdout is the protocol channel — nothing but JSON-RPC frames may be
//! written to it. Logs go to the daily log file (or stderr in debug), never
//! stdout.
//!
//! - [`protocol`]: JSON-RPC framing types and the ACP method/vocabulary.
//! - [`transport`]: the NDJSON stdio pump with outbound request correlation.
//! - [`server`]: method dispatch and the ACP-session state map.
//! - [`turn`]: the prompt bridge — tool-loop events/approvals -> ACP frames.

pub mod protocol;
pub mod server;
pub mod transport;
pub mod turn;

pub use server::{AcpServer, ServerState, SteerMap, new_steer_map};
