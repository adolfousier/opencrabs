//! `suggest_options` tool.
//!
//! Lets the agent surface OPTIONAL next-step suggestions the user may accept or
//! ignore. This is non-blocking: it fires a
//! `ProgressEvent::SuggestedOptions` and returns immediately without awaiting
//! any answer. Each surface renders the options as its own INTERACTIVE UI —
//! tap-to-send buttons under the reply on chat channels (Telegram/Discord/…), a
//! pick-list or gray ghost-text accept in the TUI. The rendering is the tool's
//! job; the model must NOT write the suggestions as plain text in its reply, or
//! they land as dead text with no button to tap.
//!
//! Intended for "here's a likely next thing you might ask" — a convenience, not
//! a question. If the agent genuinely cannot proceed without a choice, it should
//! ask directly in your reply instead.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::agent::ProgressEvent;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

/// Hard cap on options. More than a handful is noise the user won't read;
/// 8 accommodates branchy decisions without truncation (#1178).
pub const MAX_OPTIONS: usize = 8;

pub struct SuggestOptionsTool;

#[derive(Debug, Deserialize)]
struct SuggestInput {
    options: Vec<String>,
}

#[async_trait]
impl Tool for SuggestOptionsTool {
    fn name(&self) -> &str {
        "suggest_options"
    }

    fn description(&self) -> &str {
        "Surface up to 8 short option messages for the user to pick from as their next input. CHANNEL-AGNOSTIC interactive UI: tap-to-send buttons under your reply on chat channels (Telegram/Discord/...), a pick-list or gray ghost-text accept in the TUI. You MUST call this tool to make options interactive: writing them as plain text leaves dead text with no button to tap. If this is your final action of the turn, the turn ends with the options pending the user's pick (#1178 turn-halt); mid-turn calls attach the options to your message without stopping you. Use ONE option for an obvious single next step — a one-tap confirm (\"Go\", \"Confirm\", \"Agreed\") is often easier for the user than typing the word, and single-option sets are always legal; 2-8 for distinct next directions. Each option must be a complete, ready-to-send user message phrased in the user's voice (e.g. \"Add tests for the new endpoint\", not \"I could add tests\"). Keep each under ~60 chars. Do NOT use this to ask a question you need answered to proceed; do not also repeat the options in your prose."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": MAX_OPTIONS,
                    "description": "1 to 8 distinct, ready-to-send option messages in the user's voice. Rendered as interactive UI on every surface (tap-to-send buttons on chat channels; ghost-text/pick-list in the TUI) — never as plain text."
                }
            },
            "required": ["options"]
        })
    }

    fn halts_turn(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        // Pure UI signal. No filesystem, shell, or network.
        vec![]
    }

    fn requires_approval(&self) -> bool {
        // The tool IS a passive UI hint — nothing to approve.
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // #129 belt-and-braces: headless surfaces have no UI to render the
        // options — fail LOUDLY rather than park a verdict nobody sees.
        if context.headless {
            return Err(ToolError::Execution(
                "suggest_options is not available headless — no interactive user \
                 exists on this surface. State your options and recommendation \
                 in your final message instead."
                    .into(),
            ));
        }
        let parsed: SuggestInput = serde_json::from_value(input)?;

        let options = sanitize_options(parsed.options);
        if let Err(msg) = options {
            return Ok(ToolResult::error(msg));
        }
        let options = options.expect("checked Ok above");

        // Non-blocking: fire the event and return. Surfaces without a progress
        // bridge (channels, A2A) simply don't render it — not an error, the
        // suggestions are always optional.
        let count = options.len();
        if let Some(cb) = context.progress_callback.as_ref() {
            cb(context.session_id, ProgressEvent::SuggestedOptions(options));
        }

        Ok(ToolResult::success(format!(
            "Surfaced {count} follow-up suggestion(s)."
        )))
    }
}

/// Trim, drop empties, enforce the 1..=MAX distinct contract. Extracted so the
/// validation is unit-testable without a live progress callback. The mechanics
/// live in `question_common::check_options` (#764 R1); this wrapper keeps the
/// tool's own error wording (pinned by tests).
pub(crate) fn sanitize_options(raw: Vec<String>) -> std::result::Result<Vec<String>, String> {
    use crate::channels::question_common::{OptionsError, check_options};
    match check_options(raw, 1, MAX_OPTIONS) {
        Ok(options) => Ok(options),
        Err(OptionsError::TooFew { .. }) => {
            Err("suggest_options needs at least 1 non-empty option.".into())
        }
        Err(OptionsError::TooMany(n)) => Err(format!(
            "Too many suggestions ({}). Cap is {}.",
            n, MAX_OPTIONS
        )),
        Err(OptionsError::Duplicate(opt)) => Err(format!(
            "Duplicate suggestion '{opt}'. Suggestions must be distinct."
        )),
    }
}
