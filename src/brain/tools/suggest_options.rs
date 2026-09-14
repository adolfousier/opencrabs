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
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hard cap on options. More than a handful is noise the user won't read;
/// 8 accommodates branchy decisions without truncation (#1178).
pub const MAX_OPTIONS: usize = 8;

/// Visual button style hint for platforms supporting rich button styling (e.g. Telegram Bot API 10.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStyle {
    #[default]
    Default,
    Primary,
    Danger,
}

/// Raw suggestion item deserialized from polymorphic input:
/// either a bare string or an object `{ "label": "...", "style": "..." }`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RawSuggestionItem {
    Bare(String),
    Styled {
        label: String,
        #[serde(default)]
        style: Option<SuggestionStyle>,
    },
}

impl RawSuggestionItem {
    pub fn into_suggestion_item(self) -> SuggestionItem {
        match self {
            RawSuggestionItem::Bare(label) => SuggestionItem {
                label,
                style: SuggestionStyle::Default,
            },
            RawSuggestionItem::Styled { label, style } => SuggestionItem {
                label,
                style: style.unwrap_or_default(),
            },
        }
    }
}

/// A parsed, validated suggestion item with a message label and button style hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionItem {
    pub label: String,
    pub style: SuggestionStyle,
}

impl SuggestionItem {
    pub fn new(label: impl Into<String>, style: SuggestionStyle) -> Self {
        Self {
            label: label.into(),
            style,
        }
    }

    pub fn default_styled(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: SuggestionStyle::Default,
        }
    }
}

pub struct SuggestOptionsTool;

#[derive(Debug, Deserialize)]
struct SuggestInput {
    options: Vec<RawSuggestionItem>,
}

#[async_trait]
impl Tool for SuggestOptionsTool {
    fn name(&self) -> &str {
        "suggest_options"
    }

    fn description(&self) -> &str {
        "Surface up to 8 short option messages for the user to pick from as their next input. CHANNEL-AGNOSTIC interactive UI: tap-to-send buttons under your reply on chat channels (Telegram/Discord/...), a pick-list or gray ghost-text accept in the TUI. You MUST call this tool to make options interactive: writing them as plain text leaves dead text with no button to tap. If this is your final action of the turn, the turn ends with the options pending the user's pick. Use ONE option for an obvious single next step — a one-tap confirm (\"Go\", \"Confirm\", \"Agreed\") is often easier for the user than typing the word, and single-option sets are always legal; 2-8 for distinct next directions. Each option must be a complete, ready-to-send user message phrased in the user's voice (e.g. \"Add tests for the new endpoint\", not \"I could add tests\"). Keep labels concise: under 20 chars for multi-option sets, under 30 chars for a solo option (longer options collapse into numbered text or confirmation lines). Ask any open questions in your reply text; provide the candidate answers in options. Do NOT also repeat the options in your prose."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "options": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {
                                "type": "string",
                                "description": "Option label string (default neutral button style)"
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "label": {
                                        "type": "string",
                                        "description": "Ready-to-send option message in the user's voice"
                                    },
                                    "style": {
                                        "type": "string",
                                        "enum": ["default", "primary", "danger"],
                                        "description": "Button visual style: 'primary' (recommended/action), 'danger' (destructive/cancel), or 'default' (neutral). Omit for default."
                                    }
                                },
                                "required": ["label"]
                            }
                        ]
                    },
                    "minItems": 1,
                    "maxItems": MAX_OPTIONS,
                    "description": "1 to 8 distinct, ready-to-send option messages in the user's voice. Each item can be a plain string or an object {label, style}. Prefer concise labels (<=20 chars for multi-option sets, <=30 chars for a solo option) so they render as interactive buttons. Rendered on Telegram/Discord/Slack and pick-list in the TUI — never as plain text."
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

/// Trim, drop empties, enforce the 1..=MAX distinct contract, and apply
/// single-option promotion rules:
/// - A single unmarked option (`SuggestionStyle::Default`) is promoted to `SuggestionStyle::Primary`.
/// - A single option explicitly marked `SuggestionStyle::Danger` stays `Danger`.
pub(crate) fn sanitize_options(
    raw: Vec<RawSuggestionItem>,
) -> std::result::Result<Vec<SuggestionItem>, String> {
    use crate::channels::question_common::{OptionsError, check_options};

    let converted: Vec<SuggestionItem> = raw
        .into_iter()
        .map(RawSuggestionItem::into_suggestion_item)
        .collect();

    let raw_labels: Vec<String> = converted.iter().map(|item| item.label.clone()).collect();

    let valid_labels = match check_options(raw_labels, 1, MAX_OPTIONS) {
        Ok(labels) => labels,
        Err(OptionsError::TooFew { .. }) => {
            return Err("suggest_options needs at least 1 non-empty option.".into());
        }
        Err(OptionsError::TooMany(n)) => {
            return Err(format!(
                "Too many suggestions ({}). Cap is {}.",
                n, MAX_OPTIONS
            ));
        }
        Err(OptionsError::Duplicate(opt)) => {
            return Err(format!(
                "Duplicate suggestion '{opt}'. Suggestions must be distinct."
            ));
        }
    };

    // Filter and align trimmed labels with their styles
    let mut items: Vec<SuggestionItem> = Vec::with_capacity(valid_labels.len());
    let mut converted_iter = converted.into_iter();
    for label in valid_labels {
        for item in converted_iter.by_ref() {
            if item.label.trim() == label {
                items.push(SuggestionItem {
                    label,
                    style: item.style,
                });
                break;
            }
        }
    }

    // Single unmarked option promotion:
    // If there is only 1 option and its style is Default, promote to Primary.
    // If it is single and Danger, it remains Danger.
    if items.len() == 1 && items[0].style == SuggestionStyle::Default {
        items[0].style = SuggestionStyle::Primary;
    }

    Ok(items)
}
