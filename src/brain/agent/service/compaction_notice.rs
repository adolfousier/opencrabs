//! What the user gets told while a compaction walks the fallback chain.
//!
//! `complete_compaction_request` reported every step with `tracing` only:
//! a primary timing out after its full attempt budget, the walk moving to
//! the next provider, the chain running dry. After `/compact` the TUI showed
//! one "requested" line, then silence for as long as the chain took, then
//! an error (#1521). The chat path surfaces the same events as alerts.
//!
//! Manual compaction is the case where a person is watching, so it gets
//! every step. Automatic compaction stays quiet on the way and reports only
//! a failure, because a summariser that quietly succeeds is the point of
//! running it in the background.

use uuid::Uuid;

use super::types::{ProgressCallback, ProgressEvent};

/// One thing that happened inside the summariser walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompactionStep {
    /// A provider was asked and did not deliver: timeout, quota, refusal.
    AttemptFailed { provider: String, reason: String },
    /// The walk is about to ask this provider with this model.
    TryingFallback { provider: String, model: String },
    /// Nothing left to try, or nothing to walk to. `detail` is the ledger.
    Failed { detail: String },
}

/// The line a step renders as. Pure, so the wording is testable on its own.
pub(crate) fn describe(step: &CompactionStep) -> String {
    match step {
        CompactionStep::AttemptFailed { provider, reason } => {
            format!("Compaction: '{provider}' failed ({reason}), walking the fallback chain")
        }
        CompactionStep::TryingFallback { provider, model } => {
            format!("Compaction: trying '{provider}' with model '{model}'")
        }
        CompactionStep::Failed { detail } => format!("Compaction failed: {detail}"),
    }
}

/// Routes compaction steps to a session's progress callback.
#[derive(Clone)]
pub(crate) struct CompactionNotifier {
    session_id: Uuid,
    callback: ProgressCallback,
    /// Report every step (manual compaction) or only a failure (automatic).
    verbose: bool,
}

impl CompactionNotifier {
    /// A person asked for this compaction and is waiting on it.
    pub(crate) fn manual(session_id: Uuid, callback: ProgressCallback) -> Self {
        Self {
            session_id,
            callback,
            verbose: true,
        }
    }

    /// The budget asked for this compaction; only a failure is news.
    pub(crate) fn auto(session_id: Uuid, callback: ProgressCallback) -> Self {
        Self {
            session_id,
            callback,
            verbose: false,
        }
    }

    /// Build from an optional callback, so call sites stay one line.
    pub(crate) fn manual_from(
        session_id: Uuid,
        callback: Option<&ProgressCallback>,
    ) -> Option<Self> {
        callback.map(|cb| Self::manual(session_id, cb.clone()))
    }

    pub(crate) fn auto_from(session_id: Uuid, callback: Option<&ProgressCallback>) -> Option<Self> {
        callback.map(|cb| Self::auto(session_id, cb.clone()))
    }

    /// Whether this step reaches the user under this notifier's policy.
    pub(crate) fn reports(&self, step: &CompactionStep) -> bool {
        self.verbose || matches!(step, CompactionStep::Failed { .. })
    }

    pub(crate) fn step(&self, step: CompactionStep) {
        if !self.reports(&step) {
            return;
        }
        (self.callback)(
            self.session_id,
            ProgressEvent::SelfHealingAlert {
                message: describe(&step),
            },
        );
    }
}
