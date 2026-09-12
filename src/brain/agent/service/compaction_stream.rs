//! Collect a provider stream into one [`LLMResponse`] for the summariser.
//!
//! Compaction used to call the non-streaming `complete()` so no summary text
//! could leak to a UI. On an HTTP gateway that holds a non-streaming request
//! open until the whole answer exists, a summary of a full window does not
//! finish before the gateway's idle limit, and the request dies as a 504 no
//! matter how healthy the model is (#1519). The chat path never hits that
//! limit because it streams and bytes flow within seconds.
//!
//! This keeps the "nothing reaches a UI" property another way: the stream is
//! consumed here, never attached to a progress callback, and the caller gets
//! the same `LLMResponse` shape `complete()` produced.

use futures::StreamExt;

use crate::brain::provider::{
    ContentBlock, ContentDelta, LLMResponse, ProviderError, ProviderStream, StreamEvent, TokenUsage,
};

/// Drain `stream` and fold it into a single response.
///
/// Only text deltas form the summary: reasoning deltas are the model's
/// deliberation, and the summariser request carries no tools, so tool-input
/// JSON has nowhere to go. A stream `Error` event or a failed item surfaces
/// as the provider's own error so the fallback walk can decide what to do
/// with it. A stream that ends without `MessageStop` still yields whatever
/// text arrived.
pub(crate) async fn collect_stream(
    mut stream: ProviderStream,
) -> std::result::Result<LLMResponse, ProviderError> {
    let mut id = String::new();
    let mut model = String::new();
    let mut text = String::new();
    let mut usage = TokenUsage::default();
    let mut stop_reason = None;

    while let Some(item) = stream.next().await {
        match item? {
            StreamEvent::MessageStart { message } => {
                id = message.id;
                model = message.model;
                usage = message.usage;
            }
            StreamEvent::ContentBlockStart {
                content_block: ContentBlock::Text { text: opening },
                ..
            } => text.push_str(&opening),
            StreamEvent::ContentBlockStart { .. } => {}
            StreamEvent::ContentBlockDelta {
                delta: ContentDelta::TextDelta { text: delta },
                ..
            } => text.push_str(&delta),
            StreamEvent::ContentBlockDelta { .. } => {}
            StreamEvent::ContentBlockStop { .. } | StreamEvent::Ping => {}
            StreamEvent::MessageDelta {
                delta,
                usage: final_usage,
            } => {
                if delta.stop_reason.is_some() {
                    stop_reason = delta.stop_reason;
                }
                if final_usage.input_tokens > 0 {
                    usage.input_tokens = final_usage.input_tokens;
                }
                if final_usage.output_tokens > 0 {
                    usage.output_tokens = final_usage.output_tokens;
                }
            }
            StreamEvent::MessageStop => break,
            StreamEvent::Error { error } => return Err(ProviderError::StreamError(error)),
        }
    }

    Ok(LLMResponse {
        id,
        model,
        content: vec![ContentBlock::Text { text }],
        stop_reason,
        usage,
        streaming_active_secs: None,
        tool_text_leak: false,
    })
}
