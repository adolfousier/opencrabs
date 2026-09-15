//! `/clear`: start the agent fresh at the current point, at no cost.
//!
//! A failed manual `/compact` on a large context is expensive: the whole
//! post-marker snapshot goes to the summariser, and on failure the walk
//! sends it again to every provider in the chain. When the user only wants
//! to move on there was no LLM-free way to do that; `/new` starts a
//! different session row and takes the visible history and title with it
//! (#1585).
//!
//! Clearing is one user row starting with the compaction-marker prefix. The
//! context loader already cuts at the last such row and the TUI already
//! hides it on reload, so history stays on screen and in the database, the
//! session keeps its id and title, and the next turn loads only the marker
//! body. That body names the session and points the agent at
//! `session_search`, so it is not blind, just unburdened.

use uuid::Uuid;

use super::builder::AgentService;
use crate::brain::agent::error::{AgentError, Result};
use crate::services::{MessageService, SessionService};

/// Prefix the context loader and the TUI reload both key on. Must start
/// with the compaction marker text, which is the only thing either checks.
pub(crate) const CLEAR_MARKER_PREFIX: &str = "[CONTEXT COMPACTION: cleared by the user]";

/// What a failed manual compaction tells the user about the way out.
pub(crate) const CLEAR_HINT: &str = "To continue fresh without a summary: cancel if it is \
     still running, then run /clear. The history stays in the session and the agent \
     starts from that point at no cost.";

/// What the agent reads as its whole context after a clear.
pub(crate) fn clear_marker(session_title: Option<&str>) -> String {
    let title = session_title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("untitled");
    format!(
        "{CLEAR_MARKER_PREFIX}\n\n\
         The user cleared this session's context at this point. Nothing said or done \
         before it is in your context. The full history is still stored in this session, \
         titled '{title}'. If you need something from before, use the session_search tool: \
         operation 'tail' with session '{title}' reads the last messages, operation \
         'search' with a query and session '{title}' finds specific content. Fetch only \
         what the task needs. Continue from the user's next message."
    )
}

/// What happened, for the surface that asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearReceipt {
    pub session_title: Option<String>,
    /// A background summariser was in flight and has been aborted, so its
    /// summary of the old context can never land over the cleared one.
    pub aborted_background_compaction: bool,
}

impl ClearReceipt {
    /// One line for the user, the same on every surface.
    pub fn user_line(&self) -> String {
        let title = self
            .session_title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| format!(" (title '{t}')"))
            .unwrap_or_default();
        let aborted = if self.aborted_background_compaction {
            " A background compaction that was running has been cancelled."
        } else {
            ""
        };
        format!(
            "Context cleared. The history above stays in this session{title}; the agent \
             starts fresh from here and can search it with session_search when it needs \
             something. No summariser call was made.{aborted}"
        )
    }
}

impl AgentService {
    /// Cut the agent's context at the current point without a provider call.
    ///
    /// Aborts an in-flight background summariser first, exactly as
    /// `compact_context` does: its result is applied on a later visit and
    /// would otherwise overwrite the cleared context with a summary of the
    /// old one. Then appends the marker row the loader cuts at.
    pub async fn clear_context(&self, session_id: Uuid) -> Result<ClearReceipt> {
        let aborted_background_compaction = match self.take_pending_compaction(session_id) {
            Some(pending) => {
                tracing::info!("/clear: aborting the background compaction in flight");
                pending.abort();
                true
            }
            None => false,
        };

        let session_title = SessionService::new(self.context.clone())
            .get_session(session_id)
            .await
            .map_err(AgentError::db)?
            .and_then(|s| s.title);

        MessageService::new(self.context.clone())
            .create_message(
                session_id,
                "user".to_string(),
                clear_marker(session_title.as_deref()),
            )
            .await
            .map_err(AgentError::db)?;

        tracing::info!(
            "/clear: context cut for session {session_id} (title={:?}, aborted_background={})",
            session_title,
            aborted_background_compaction
        );
        Ok(ClearReceipt {
            session_title,
            aborted_background_compaction,
        })
    }
}
