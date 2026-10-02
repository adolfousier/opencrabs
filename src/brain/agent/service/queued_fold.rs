//! Queued-message folding (#1784).
//!
//! A follow-up that lands mid-turn is injected into the SAME run instead of
//! starting a second one. When the round that was already running ended
//! text-only, its finished draft was the complete answer to the earlier
//! request, and relaying it as its own message means one queued burst produces
//! two replies. The draft stays in the model's context, and the fold is
//! announced here, in the injected user turn, so the single final reply covers
//! both requests instead of reading like a delta ("as above") about an answer
//! the user never saw.

/// Told to the model in the injected user turn when the round before it
/// produced a draft that was never sent.
pub(crate) const NOT_DELIVERED_NOTE: &str = "[OpenCrabs] The reply you drafted just now was never delivered to the user: a newer message arrived while you were writing it, and this turn continues with that message. Your draft is in the conversation above, but the user has not seen any of it. Answer both requests in the final reply, restating anything from the draft the user still needs. Do not refer to a previous answer, and do not write a delta like \"as above\".";

/// The user text injected for a queued message that folds into a running turn.
///
/// `had_draft` is whether the superseded round wrote any text at all: a round
/// that produced nothing was not hiding an answer, so it gets no note. Prefixing
/// an empty round with a claim about a withheld draft tells the model something
/// false about its own transcript, and models argue with false transcripts.
pub(crate) fn injected_context(had_draft: bool, queued: &str) -> String {
    if had_draft {
        format!("{NOT_DELIVERED_NOTE}\n\n{queued}")
    } else {
        queued.to_string()
    }
}
