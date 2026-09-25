//! Truncated-mid-sentence response detection and bounded continuation.
//!
//! Local reasoning models (notably Qwen3.6-35B on MLX) periodically emit
//! an EOS token mid-sentence — the response looks complete from a
//! protocol standpoint (proper `finish_reason=stop` + usage chunk) but
//! the visible text ends mid-word. This module owns the decision to
//! ask the model to continue, up to `MAX_CONTINUATION_ATTEMPTS` times
//! per turn (#1737): the original attempt plus one anchored retry when
//! a continuation degenerates into echoing the tail.
//!
//! Extracted from `tool_loop.rs` (was lines 2800-2866) as part of the
//! 2026-05-04 Linor-flagged refactor: tool_loop.rs was 4,047 lines.
//! Behaviour is unchanged from the pre-extraction version. The
//! detection heuristic itself lives in `phantom::looks_truncated_mid_
//! sentence` — that boundary already existed.
//!
//! Coupling with bug-B (commit 03d0524e):
//! `current_iter_is_truncation_continue` is set by the caller AFTER
//! this returns true, so the stream-error path on the NEXT iteration
//! skips cross-provider fallback. We don't try to bundle that flag
//! into this module because it lives on the loop's own state and only
//! has meaning relative to the next stream attempt.

use super::phantom::looks_truncated_mid_sentence;
use super::types::{ProgressCallback, ProgressEvent};
use crate::brain::agent::context::AgentContext;
use crate::brain::provider::Message;
use uuid::Uuid;

/// Usage-gap signal (#36): the visible text is far shorter than the billed
/// non-reasoning output tokens would allow. `usage.output_tokens` includes
/// reasoning, so reasoning is subtracted first — thinking-heavy turns would
/// otherwise fake a gap on every reply. The floor is deliberately
/// conservative at 2 chars per visible token (English prose runs ~4): the
/// gap CORROBORATES a structural truncation signal and journals standalone
/// deficits, but never triggers a retry by itself — tokenizer variance and
/// terse/CJK styles make a ratio-only verdict ambiguous. Skipped entirely
/// when the provider reported no output usage.
pub(crate) fn has_usage_gap(text: &str, usage: &crate::brain::provider::TokenUsage) -> bool {
    if usage.output_tokens == 0 {
        return false;
    }
    let visible_tokens = usage.output_tokens.saturating_sub(usage.reasoning_tokens) as u64;
    if visible_tokens == 0 {
        return false;
    }
    let chars = text.trim_end().chars().count() as u64;
    chars < visible_tokens * 2
}

/// Maximum continuation requests per turn (#1737): the original
/// mid-sentence retry plus one anchored retry when a continuation
/// degenerates (echoed the tail / only closed dangling markup).
pub(crate) const MAX_CONTINUATION_ATTEMPTS: u32 = 2;

/// How many trailing characters of the partial are embedded verbatim in
/// the continuation nudge as the anchor (#1737).
pub(crate) const TAIL_ANCHOR_CHARS: usize = 120;

/// Build the continuation nudge for a truncated partial (#1737).
///
/// The nudge embeds the verbatim tail of the partial between explicit
/// delimiter lines. The unanchored predecessor ("continue from exactly
/// where you left off", #36) left the model to guess where that was from
/// its own last assistant message — and when that message ended inside
/// an unclosed code span, the minimal legal "continuation" was the lone
/// closing backtick: 1 visible char, 774 billed tokens, verdict
/// unrecovered. The anchor removes the guess; the anti-markup clause
/// removes the degenerate minimal move. `degenerate` carries the
/// discarded continuation text on a retry so the model knows its previous
/// attempt was rejected and why.
pub(crate) fn continuation_nudge(partial: &str, degenerate: Option<&str>) -> String {
    let tail: String = partial
        .trim_end()
        .chars()
        .rev()
        .take(TAIL_ANCHOR_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let head = match degenerate {
        Some(d) => format!(
            "[System: Your previous continuation attempt returned only {d:?} — it added \
             no content, so it was discarded. "
        ),
        None => "[System: Your previous reply was cut off mid-sentence. ".to_string(),
    };
    format!(
        "{head}Your reply so far ends with these exact characters:\n---\n{tail}\n---\n\
         Continue from that exact point onward: the very next characters must be NEW \
         content that carries the answer forward. Do NOT repeat any earlier text, do \
         NOT restart the answer, do NOT re-plan. If the tail ends inside unclosed \
         markup (a code span, fence, bracket or quote), closing it alone does NOT \
         count as continuing — carry the content forward and close the markup only \
         when the content is actually done. Just keep writing.]"
    )
}

/// Detect a mid-sentence cut-off and inject the continuation prompt
/// into the context.
///
/// Returns `true` when the text was detected as truncated AND the
/// caller should `continue;` to the next loop iteration. Returns
/// `false` when the text reads as complete — the caller proceeds with
/// normal end-of-turn handling.
///
/// Side effects when returning `true`:
///   * `[TRUNCATION] verdict=retry` warn journal with the signal set
///     (structural, +usage_gap when the billed non-reasoning output
///     dwarfs the arrived text — #36), the last-60-char preview, and
///     the usage numbers
///   * `progress_callback` fires `SelfHealingAlert` + `IntermediateText`
///     so the user sees the partial reply AND a nudge that we're
///     asking for continuation
///   * Two messages appended to `context`: the partial assistant reply
///     (so it's visible AND part of context), then a system-style user
///     message instructing the model to continue from where it left off
///     without restarting or re-planning
///
/// Caller is responsible for the gating preconditions (attempt budget,
/// CLI-provider exclusion, `iteration > 0`, `StopReason::EndTurn`)
/// because those depend on the surrounding loop state and would just
/// be more parameters here without making the function clearer.
pub(super) fn try_emit_truncation_continue(
    iteration_text: &str,
    reasoning_text: Option<&String>,
    usage: &crate::brain::provider::TokenUsage,
    context: &mut AgentContext,
    session_id: Uuid,
    attempt: u32,
    progress_callback: &Option<ProgressCallback>,
) -> bool {
    let structural = looks_truncated_mid_sentence(iteration_text.trim_end());
    let usage_gap = has_usage_gap(iteration_text, usage);

    if !structural {
        // Text reads as complete. A usage gap alone never triggers a retry
        // — tokenizer variance and terse styles make it ambiguous — but the
        // journal line accumulates evidence for future tuning (#36).
        if usage_gap {
            tracing::info!(
                "[TRUNCATION] verdict=usage-gap-watch text_chars={}, usage_output={}, usage_reasoning={} — complete-looking text with a token deficit; no retry",
                iteration_text.trim_end().chars().count(),
                usage.output_tokens,
                usage.reasoning_tokens
            );
        }
        return false;
    }

    let preview: String = iteration_text
        .chars()
        .rev()
        .take(60)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    tracing::warn!(
        "[TRUNCATION] verdict=retry signals=structural{} tail={:?} text_chars={}, usage_output={}, usage_reasoning={}, usage_gap={}, attempt={}/{} — asking model to continue",
        if usage_gap { "+usage_gap" } else { "" },
        preview,
        iteration_text.trim_end().chars().count(),
        usage.output_tokens,
        usage.reasoning_tokens,
        usage_gap,
        attempt,
        MAX_CONTINUATION_ATTEMPTS
    );

    if let Some(cb) = progress_callback {
        cb(
            session_id,
            ProgressEvent::SelfHealingAlert {
                message: "Response was cut off mid-sentence — asking model to continue".into(),
            },
        );
    }
    // Keep the partial as a real intermediate message so the user sees
    // what DID arrive, then nudge continuation.
    if !iteration_text.is_empty()
        && let Some(cb) = progress_callback
    {
        cb(
            session_id,
            ProgressEvent::IntermediateText {
                text: iteration_text.to_string(),
                reasoning: reasoning_text.cloned(),
            },
        );
    }

    context.add_message(Message::assistant(iteration_text.to_string()));
    context.add_message(Message::user(continuation_nudge(iteration_text, None)));

    true
}

/// Join a truncated partial with the continuation that was asked for (#859).
///
/// `final_text` is built from the LAST response only, on the assumption that
/// earlier text already reached the user as `IntermediateText`. That holds for
/// the TUI and stopped holding for Telegram once intermediates were gated to
/// deliverable rich reports (#838): a plain-prose partial is emitted, dropped
/// by the gate, and the continuation alone becomes the answer.
///
/// Observed cost: a 551-token answer was replaced by a 60-character provider
/// refusal, because the refusal was the tail of the partial AND the whole of
/// the continuation. The user saw only the refusal.
/// What a continuation attempt actually achieved.
///
/// The two outcomes used to be one `String`, which meant a continuation that
/// recovered nothing was indistinguishable from one that worked — so a turn cut
/// off mid-sentence was delivered as a finished answer (#956).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Continuation {
    /// The continuation carried the answer forward.
    Extended(String),
    /// It added nothing: the model echoed the tail it was asked to continue
    /// from, or returned nothing at all. The text is STILL truncated.
    Echoed(String),
}

/// Note appended to an answer that is still cut off after the continuation
/// failed. Telling the user is the only honest option left: the alternative is
/// presenting a sentence that stops at a colon as a completed reply.
pub(crate) const INCOMPLETE_MARKER: &str =
    "\n\n_(cut off here — the model did not continue. Ask it to finish this.)_";

pub(crate) fn join_continuation(partial: &str, continuation: &str) -> Continuation {
    let p = partial.trim_end();
    let c = continuation.trim();
    if p.is_empty() {
        return Continuation::Extended(c.to_string());
    }
    // Nothing came back, so nothing was recovered.
    if c.is_empty() {
        return Continuation::Echoed(p.to_string());
    }
    // The continuation repeated ground the partial already covers. This is the
    // reported case: the model echoed the tail it was asked to continue from.
    // Not appending it is right — but the answer is still truncated, and
    // saying so is the caller's job.
    if p.contains(c) {
        return Continuation::Echoed(p.to_string());
    }
    // The model restarted and reproduced the partial in full. That IS forward
    // progress: the restart carries the whole answer, not just the tail.
    if c.contains(p) {
        return Continuation::Extended(c.to_string());
    }
    // A genuine continuation. A separator is inserted only when neither side
    // supplies one: the cut can land mid-word, where joining with a space would
    // corrupt the word, but two clauses run together are worse to read than one
    // stray space. Same trade already made for command labels.
    let needs_space = !p.ends_with(char::is_whitespace)
        && !c.starts_with(char::is_whitespace)
        && !p.ends_with(char::is_alphanumeric);
    Continuation::Extended(if needs_space {
        format!("{p} {c}")
    } else {
        format!("{p}{c}")
    })
}
