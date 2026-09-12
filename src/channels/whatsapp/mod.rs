//! WhatsApp Integration
//!
//! Runs a WhatsApp Web client alongside the TUI, forwarding messages from
//! allowlisted phone numbers to the AgentService and replying with responses.
//!
//! Layout: [`state`] holds the shared `WhatsAppState` struct and its
//! constructor; each concern has its own impl module beside it
//! (`approval`, `cancel`, `connection`, `followups`, `onboarding_events`,
//! `pairing`, `photos`, `sessions`); `agent` runs the client event loop,
//! `handler` routes inbound messages, `resume` re-delivers background
//! results and `store` persists the session. This file is declarations
//! only — no function definitions live here (CONTRIBUTING.md).

mod agent;
mod approval;
pub(crate) mod blocklist;
mod cancel;
mod connection;
mod followups;
pub(crate) mod ephemeral;
pub(crate) mod handler;
mod onboarding_events;
pub(crate) mod inbound;
pub(crate) mod media_retry;
mod pairing;
pub(crate) mod outbox;
mod photos;
pub(crate) mod rate_limit;
pub(crate) mod poll;
pub(crate) mod resume;
pub(crate) mod reaction;
mod sessions;
mod state;
pub(crate) mod store;

pub use agent::WhatsAppAgent;
pub(crate) mod stream;
pub(crate) mod voice_note;
pub use approval::WaApproval;
pub use state::WhatsAppState;
