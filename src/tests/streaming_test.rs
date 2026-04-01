//! Streaming Response Tests
//!
//! Tests for streaming LLM responses and real-time UI updates.

use anyhow::Result;
use async_trait::async_trait;
use opencrabs::llm::provider::{
    error::{ProviderError, Result as ProviderResult},
    types::{
        ContentBlock, ContentDelta, LLMRequest, LLMResponse, MessageDelta, Role, StopReason,
        StreamEvent, StreamMessage, TokenUsage,
    },
    Provider, ProviderStream,
};
use futures::{stream, StreamExt};

/// Mock provider with streaming support
struct StreamingMockProvider {
    events: Vec<StreamEvent>,
}

impl StreamingMockProvider {
    fn new(text_chunks: Vec<&str>) -> Self {
        let mut events = vec![StreamEvent::MessageStart {
            message: StreamMessage {
                id: "msg-test".to_string(),
                model: "mock-model".to_string(),
                role: Role::Assistant,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0, ..Default::default() },
            },
        }];

        // Add content block start
        events.push(StreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Text {
                text: String::new(),
            },
        });

        // Add text deltas for each chunk
        for chunk in text_chunks {
            events.push(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: chunk.to_string(),
                },
            });
        }

        // Add content block stop
        events.push(StreamEvent::ContentBlockStop { index: 0 });

        // Add message delta with final token count
        events.push(StreamEvent::MessageDelta {
            delta: MessageDelta {
                stop_reason: Some(StopReason::EndTurn),
                stop_sequence: None,
            },
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 20, ..Default::default() },
        });

        // Add message stop
        events.push(StreamEvent::MessageStop);

        Self { events }
    }

    fn with_error(error_message: &str) -> Self {
        Self {
            events: vec![StreamEvent::Error {
                error: error_message.to_string(),
            }],
        }
    }
}

#[async_trait]
impl Provider for StreamingMockProvider {
    async fn complete(&self, _request: LLMRequest) -> ProviderResult<LLMResponse> {
        // Not implemented for this test - we focus on streaming
        Err(ProviderError::StreamingNotSupported)
    }

    async fn stream(&self, _request: LLMRequest) -> ProviderResult<ProviderStream> {
        let events = self.events.clone();
        let stream = stream::iter(events.into_iter().map(Ok));
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "streaming-mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(8192)
    }

    fn calculate_cost(&self, _model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        ((input_tokens + output_tokens) as f64 / 1000.0) * 0.001
    }

    fn supports_streaming(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn test_streaming_basic() -> Result<()> {
    let provider = StreamingMockProvider::new(vec!["Hello", " ", "world", "!"]);
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut events = vec![];

    while let Some(event) = stream.next().await {
        events.push(event?);
    }

    // Should have: MessageStart, ContentBlockStart, 4 deltas, ContentBlockStop, MessageDelta, MessageStop
    assert_eq!(events.len(), 9); // 1 start + 1 block start + 4 deltas + 1 block stop + 1 delta + 1 stop

    // Verify first event is MessageStart
    assert!(matches!(events[0], StreamEvent::MessageStart { .. }));

    // Verify text deltas
    let mut text_chunks = vec![];
    for event in &events {
        if let StreamEvent::ContentBlockDelta {
            delta: ContentDelta::TextDelta { text },
            ..
        } = event
        {
            text_chunks.push(text.clone());
        }
    }
    assert_eq!(text_chunks, vec!["Hello", " ", "world", "!"]);

    // Verify final event is MessageStop
    assert!(matches!(events[events.len() - 1], StreamEvent::MessageStop));

    Ok(())
}

#[tokio::test]
async fn test_streaming_single_chunk() -> Result<()> {
    let provider = StreamingMockProvider::new(vec!["Complete response in one go"]);
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut event_count = 0;
    let mut text_received = String::new();

    while let Some(event) = stream.next().await {
        event_count += 1;
        if let StreamEvent::ContentBlockDelta {
            delta: ContentDelta::TextDelta { text },
            ..
        } = event?
        {
            text_received.push_str(&text);
        }
    }

    assert_eq!(text_received, "Complete response in one go");
    assert_eq!(event_count, 6); // MessageStart + BlockStart + Delta + BlockStop + MessageDelta + MessageStop
    Ok(())
}

#[tokio::test]
async fn test_streaming_multiple_chunks() -> Result<()> {
    let chunks = vec![
        "This",
        " is",
        " a",
        " longer",
        " response",
        " with",
        " many",
        " chunks",
    ];
    let provider = StreamingMockProvider::new(chunks.clone());
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut received_chunks = vec![];

    while let Some(event) = stream.next().await {
        if let StreamEvent::ContentBlockDelta {
            delta: ContentDelta::TextDelta { text },
            ..
        } = event?
        {
            received_chunks.push(text);
        }
    }

    assert_eq!(received_chunks, chunks);
    Ok(())
}

#[tokio::test]
async fn test_streaming_token_counting() -> Result<()> {
    let provider = StreamingMockProvider::new(vec!["Test"]);
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut start_tokens = None;
    let mut end_tokens = None;

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::MessageStart { message } => {
                start_tokens = Some(message.usage);
            }
            StreamEvent::MessageDelta { usage, .. } => {
                end_tokens = Some(usage);
            }
            _ => {}
        }
    }

    // Verify we got token counts
    assert!(start_tokens.is_some());
    assert!(end_tokens.is_some());

    let end = end_tokens.unwrap();
    assert_eq!(end.input_tokens, 10);
    assert_eq!(end.output_tokens, 20);
    assert_eq!(end.total(), 30);

    Ok(())
}

#[tokio::test]
async fn test_streaming_stop_reason() -> Result<()> {
    let provider = StreamingMockProvider::new(vec!["Test"]);
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut stop_reason = None;

    while let Some(event) = stream.next().await {
        if let StreamEvent::MessageDelta { delta, .. } = event? {
            stop_reason = delta.stop_reason;
        }
    }

    assert_eq!(stop_reason, Some(StopReason::EndTurn));
    Ok(())
}

#[tokio::test]
async fn test_streaming_error_handling() -> Result<()> {
    let provider = StreamingMockProvider::with_error("Test error message");
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut error_received = None;

    while let Some(event) = stream.next().await {
        if let StreamEvent::Error { error } = event? {
            error_received = Some(error);
            break;
        }
    }

    assert!(error_received.is_some());
    assert_eq!(error_received.unwrap(), "Test error message");
    Ok(())
}

#[tokio::test]
async fn test_streaming_empty_response() -> Result<()> {
    let provider = StreamingMockProvider::new(vec![]);
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut event_count = 0;

    while let Some(event) = stream.next().await {
        event?;
        event_count += 1;
    }

    // Should still have metadata events even with no text
    // MessageStart + ContentBlockStart + ContentBlockStop + MessageDelta + MessageStop = 5
    assert_eq!(event_count, 5);
    Ok(())
}

#[tokio::test]
async fn test_streaming_content_accumulation() -> Result<()> {
    let provider = StreamingMockProvider::new(vec!["Hello", " ", "world", "!"]);
    let request = LLMRequest::new("mock-model", vec![]).with_streaming();

    let mut stream = provider.stream(request).await?;
    let mut accumulated_text = String::new();

    while let Some(event) = stream.next().await {
        if let StreamEvent::ContentBlockDelta {
            delta: ContentDelta::TextDelta { text },
            ..
        } = event?
        {
            accumulated_text.push_str(&text);
        }
    }

    assert_eq!(accumulated_text, "Hello world!");
    Ok(())
}

#[tokio::test]
async fn test_streaming_request_builder() {
    let request = LLMRequest::new("test-model", vec![]).with_streaming();

    assert!(request.stream);
    assert_eq!(request.model, "test-model");
}

#[tokio::test]
async fn test_provider_supports_streaming() {
    let provider = StreamingMockProvider::new(vec!["test"]);
    assert!(provider.supports_streaming());
}
