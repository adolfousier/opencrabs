//! Non-streaming response → stream event synthesizer.
//!
//! Some OpenRouter upstreams (e.g. Venice) don't support streaming.
//! OpenRouter returns the full response as a single JSON blob with
//! `"object":"chat.completion"` and `"message"` instead of SSE `data:`
//! lines with `"delta"`. This module detects that case and synthesizes
//! the same stream events the SSE parser would have produced.

use super::error::ProviderError;
use super::types::*;

/// Deserialize structs — mirrors the OpenAI types in custom_openai_compatible
/// but with full (non-streaming) `message` fields.

#[derive(Debug, Deserialize)]
struct NonStreamResponse {
    id: String,
    #[serde(default)]
    model: Option<String>,
    choices: Vec<NonStreamChoice>,
    #[serde(default)]
    usage: Option<NonStreamUsage>,
}

#[derive(Debug, Deserialize)]
struct NonStreamChoice {
    #[allow(dead_code)]
    index: u32,
    message: Option<NonStreamMessage>,
    /// Some providers put the message under `delta` even in non-streaming.
    delta: Option<NonStreamMessage>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NonStreamMessage {
    #[allow(dead_code)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<NonStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct NonStreamToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<NonStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct NonStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NonStreamUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default, alias = "prompt_tokens_details")]
    prompt_details: Option<NonStreamPromptDetails>,
}

#[derive(Debug, Deserialize)]
struct NonStreamPromptDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

use serde::Deserialize;

/// Check if the buffer looks like a non-streaming response.
pub(crate) fn is_nonstream_response(buf: &str) -> bool {
    let trimmed = buf.trim();
    trimmed.starts_with('{') && trimmed.contains("\"chat.completion\"")
}

/// Parse a non-streaming JSON response and synthesize the same stream
/// events that the SSE parser would have produced. Returns `None` if
/// the buffer doesn't parse as a valid response.
pub(crate) fn synthesize_stream_events(
    buf: &str,
) -> Option<Vec<std::result::Result<StreamEvent, ProviderError>>> {
    let resp: NonStreamResponse = serde_json::from_str(buf.trim()).ok()?;
    let mut events = Vec::new();

    tracing::info!(
        "[OR_NONSTREAM] Synthesizing stream events from non-streaming response (id={})",
        resp.id,
    );

    // ── MessageStart ──
    if !resp.id.is_empty() {
        events.push(Ok(StreamEvent::MessageStart {
            message: StreamMessage {
                id: resp.id,
                model: resp.model.unwrap_or_default(),
                role: Role::Assistant,
                usage: TokenUsage::default(),
            },
        }));
    }

    let choice = resp.choices.first()?;
    let msg = choice.message.as_ref().or(choice.delta.as_ref())?;

    // ── Reasoning / thinking content ──
    if let Some(ref reasoning) = msg.reasoning_content
        && !reasoning.is_empty()
    {
        events.push(Ok(StreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::ReasoningDelta {
                text: reasoning.clone(),
            },
        }));
    }

    // ── Text content ──
    let content = msg.content.as_deref().unwrap_or("");
    // Strip leading newline that some models prepend after reasoning
    let content = content.strip_prefix('\n').unwrap_or(content);
    if !content.is_empty() {
        events.push(Ok(StreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Text {
                text: content.to_string(),
            },
        }));
        events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
    }

    // ── Tool calls ──
    if let Some(ref tc_list) = msg.tool_calls {
        for tc in tc_list {
            let id = tc.id.clone().unwrap_or_default();
            let name = tc
                .function
                .as_ref()
                .and_then(|f| f.name.clone())
                .unwrap_or_default();
            let args = tc
                .function
                .as_ref()
                .and_then(|f| f.arguments.clone())
                .unwrap_or_default();
            let input = serde_json::from_str(&args).unwrap_or_else(|_| serde_json::json!({}));
            let tool_index = tc.index + 1; // offset by 1 to avoid collision with text at 0

            tracing::info!(
                "[OR_NONSTREAM] tool_call: id={}, name={}, args_len={}",
                id,
                name,
                args.len(),
            );

            events.push(Ok(StreamEvent::ContentBlockStart {
                index: tool_index,
                content_block: ContentBlock::ToolUse { id, name, input },
            }));
            events.push(Ok(StreamEvent::ContentBlockStop { index: tool_index }));
        }
    }

    // ── Stop reason ──
    let stop_reason = choice.finish_reason.as_deref().map(|fr| match fr {
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        _ => StopReason::EndTurn,
    });

    // ── Usage ──
    let mut token_usage = TokenUsage::default();
    if let Some(ref usage) = resp.usage {
        token_usage.input_tokens = usage.prompt_tokens.unwrap_or(0);
        token_usage.output_tokens = usage.completion_tokens.unwrap_or(0);
        if let Some(cache_create) = usage.cache_creation_input_tokens {
            token_usage.cache_creation_tokens = cache_create;
        }
        if let Some(ref details) = usage.prompt_details
            && let Some(cached) = details.cached_tokens
        {
            token_usage.cache_read_tokens = cached;
        }
    }

    events.push(Ok(StreamEvent::MessageDelta {
        delta: MessageDelta {
            stop_reason,
            stop_sequence: None,
        },
        usage: token_usage,
    }));
    events.push(Ok(StreamEvent::MessageStop));

    Some(events)
}
