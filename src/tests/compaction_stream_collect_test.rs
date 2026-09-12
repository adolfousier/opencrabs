//! Regression for #1519: the summariser asks HTTP providers through the
//! stream path and folds the stream into one response.
//!
//! A manual compaction of a ~140k-token snapshot sat for 15 minutes and then
//! failed with the chain exhausted while a chat session on the same provider
//! and model kept working. The summariser used the non-streaming request, the
//! gateway held that connection open until the whole summary existed, and its
//! idle limit of about two minutes returned 504 on every retry. The chat path
//! streams, so bytes flow within seconds and the limit never fires.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::brain::agent::service::AgentService;
use crate::brain::agent::service::compaction_stream::collect_stream;
use crate::brain::provider::error::should_try_next_provider;
use crate::brain::provider::{
    ContentBlock, ContentDelta, LLMRequest, LLMResponse, Message, MessageDelta, Provider,
    ProviderError, ProviderStream, Role, StopReason, StreamEvent, StreamMessage, TokenUsage,
};

fn start(id: &str, model: &str, input_tokens: u32) -> StreamEvent {
    StreamEvent::MessageStart {
        message: StreamMessage {
            id: id.to_string(),
            model: model.to_string(),
            role: Role::Assistant,
            usage: TokenUsage {
                input_tokens,
                ..Default::default()
            },
        },
    }
}

fn text(index: usize, t: &str) -> StreamEvent {
    StreamEvent::ContentBlockDelta {
        index,
        delta: ContentDelta::TextDelta {
            text: t.to_string(),
        },
    }
}

fn reasoning(t: &str) -> StreamEvent {
    StreamEvent::ContentBlockDelta {
        index: 0,
        delta: ContentDelta::ReasoningDelta {
            text: t.to_string(),
        },
    }
}

fn stream_of(events: Vec<Result<StreamEvent, ProviderError>>) -> ProviderStream {
    Box::pin(futures::stream::iter(events))
}

fn summary_text(response: &LLMResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn text_deltas_fold_into_one_response_with_identity_and_usage() {
    let response = collect_stream(stream_of(vec![
        Ok(start("resp-1", "m", 120)),
        Ok(StreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Text {
                text: String::new(),
            },
        }),
        Ok(text(0, "## Summary\n")),
        Ok(text(0, "Work continues.")),
        Ok(StreamEvent::ContentBlockStop { index: 0 }),
        Ok(StreamEvent::MessageDelta {
            delta: MessageDelta {
                stop_reason: Some(StopReason::EndTurn),
                stop_sequence: None,
            },
            usage: TokenUsage {
                output_tokens: 9,
                ..Default::default()
            },
        }),
        Ok(StreamEvent::MessageStop),
    ]))
    .await
    .expect("collects");

    assert_eq!(response.id, "resp-1");
    assert_eq!(response.model, "m");
    assert_eq!(summary_text(&response), "## Summary\nWork continues.");
    assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(response.usage.input_tokens, 120);
    assert_eq!(response.usage.output_tokens, 9);
}

#[tokio::test]
async fn reasoning_deltas_are_not_part_of_the_summary() {
    let response = collect_stream(stream_of(vec![
        Ok(start("r", "m", 0)),
        Ok(reasoning("let me think about the session")),
        Ok(text(1, "The summary.")),
        Ok(StreamEvent::MessageStop),
    ]))
    .await
    .expect("collects");
    assert_eq!(summary_text(&response), "The summary.");
}

#[tokio::test]
async fn a_stream_error_event_is_a_provider_error_the_chain_walks() {
    let err = collect_stream(stream_of(vec![
        Ok(start("r", "m", 0)),
        Ok(text(0, "partial")),
        Ok(StreamEvent::Error {
            error: "upstream reset".to_string(),
        }),
    ]))
    .await
    .expect_err("an error event fails the attempt");
    assert!(matches!(err, ProviderError::StreamError(ref e) if e == "upstream reset"));
    assert!(
        should_try_next_provider(&err),
        "a broken summariser stream hands the attempt to the next provider"
    );
}

#[tokio::test]
async fn a_failed_stream_item_propagates_as_is() {
    let err = collect_stream(stream_of(vec![
        Ok(start("r", "m", 0)),
        Err(ProviderError::ContextLengthExceeded(0)),
    ]))
    .await
    .expect_err("a failed item fails the attempt");
    assert!(matches!(err, ProviderError::ContextLengthExceeded(0)));
}

#[tokio::test]
async fn a_stream_that_ends_without_message_stop_keeps_its_text() {
    let response = collect_stream(stream_of(vec![Ok(start("r", "m", 0)), Ok(text(0, "kept"))]))
        .await
        .expect("collects");
    assert_eq!(summary_text(&response), "kept");
}

/// Counts which entry point compaction uses for a provider.
struct PathMock {
    name: String,
    cli: bool,
    complete_calls: Arc<AtomicUsize>,
    stream_calls: Arc<AtomicUsize>,
}

impl PathMock {
    fn new(name: &str, cli: bool) -> Self {
        Self {
            name: name.to_string(),
            cli,
            complete_calls: Arc::new(AtomicUsize::new(0)),
            stream_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Provider for PathMock {
    async fn complete(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<LLMResponse> {
        self.complete_calls.fetch_add(1, Ordering::SeqCst);
        Ok(LLMResponse {
            id: format!("{}-complete", self.name),
            model: request.model,
            content: vec![ContentBlock::Text {
                text: "via complete".to_string(),
            }],
            stop_reason: None,
            usage: TokenUsage::default(),
            streaming_active_secs: None,
            tool_text_leak: false,
        })
    }

    async fn stream(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<ProviderStream> {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        Ok(stream_of(vec![
            Ok(start(&format!("{}-stream", self.name), &request.model, 0)),
            Ok(text(0, "via stream")),
            Ok(StreamEvent::MessageStop),
        ]))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        "mock-default"
    }

    fn supported_models(&self) -> Vec<String> {
        Vec::new()
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(200_000)
    }

    fn calculate_cost(&self, _model: &str, _input_tokens: u32, _output_tokens: u32) -> f64 {
        0.0
    }

    fn cli_handles_tools(&self) -> bool {
        self.cli
    }
}

fn request() -> LLMRequest {
    LLMRequest::new("mock-default", vec![Message::user("summarise this")])
}

#[tokio::test]
async fn an_http_provider_is_summarised_through_the_stream_path() {
    let mock = PathMock::new("http-mock", false);
    let (complete_calls, stream_calls) = (mock.complete_calls.clone(), mock.stream_calls.clone());
    let primary: Arc<dyn Provider> = Arc::new(mock);

    let response = AgentService::complete_compaction_request(
        &primary,
        &[],
        request(),
        &CancellationToken::new(),
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("served");

    assert_eq!(response.id, "http-mock-stream");
    assert_eq!(summary_text(&response), "via stream");
    assert_eq!(stream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(complete_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_cli_provider_keeps_the_non_streaming_request() {
    let mock = PathMock::new("cli-mock", true);
    let (complete_calls, stream_calls) = (mock.complete_calls.clone(), mock.stream_calls.clone());
    let primary: Arc<dyn Provider> = Arc::new(mock);

    let response = AgentService::complete_compaction_request(
        &primary,
        &[],
        request(),
        &CancellationToken::new(),
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("served");

    assert_eq!(response.id, "cli-mock-complete");
    assert_eq!(complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(stream_calls.load(Ordering::SeqCst), 0);
}
