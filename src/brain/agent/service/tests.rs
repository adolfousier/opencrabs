use super::*;
use crate::brain::provider::{
    ContentBlock, LLMRequest, LLMResponse, Message, ProviderStream, Role, StopReason, TokenUsage,
};
use crate::brain::tools::ToolRegistry;
use crate::db::Database;
use crate::services::{MessageService, ServiceContext, SessionService};
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

use crate::brain::provider::Provider;

/// Mock provider for testing
struct MockProvider;

#[async_trait]
impl Provider for MockProvider {
    async fn complete(&self, _request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        Ok(LLMResponse {
            id: "test-response-1".to_string(),
            model: "mock-model".to_string(),
            content: vec![ContentBlock::Text {
                text: "This is a test response".to_string(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 20,
            },
        })
    }

    async fn stream(&self, request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

        let response = self.complete(request).await?;
        let mut events = vec![Ok(StreamEvent::MessageStart {
            message: StreamMessage {
                id: response.id.clone(),
                model: response.model.clone(),
                role: Role::Assistant,
                usage: response.usage,
            },
        })];
        for (i, block) in response.content.iter().enumerate() {
            if let ContentBlock::Text { text } = block {
                events.push(Ok(StreamEvent::ContentBlockStart {
                    index: i,
                    content_block: ContentBlock::Text {
                        text: String::new(),
                    },
                }));
                events.push(Ok(StreamEvent::ContentBlockDelta {
                    index: i,
                    delta: ContentDelta::TextDelta { text: text.clone() },
                }));
                events.push(Ok(StreamEvent::ContentBlockStop { index: i }));
            }
        }
        events.push(Ok(StreamEvent::MessageDelta {
            delta: MessageDelta {
                stop_reason: response.stop_reason,
                stop_sequence: None,
            },
            usage: response.usage,
        }));
        events.push(Ok(StreamEvent::MessageStop));
        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(4096)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.001 // Mock cost
    }
}

async fn create_test_service() -> (AgentService, Uuid) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();

    let context = ServiceContext::new(pool);
    let provider = Arc::new(MockProvider);

    let agent_service = AgentService::new(provider, context.clone());

    // Create a test session
    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test Session".to_string()))
        .await
        .unwrap();

    (agent_service, session.id)
}

#[tokio::test]
async fn test_agent_service_creation() {
    let (agent_service, _) = create_test_service().await;
    assert_eq!(agent_service.max_tool_iterations, 0); // 0 = unlimited
}

#[tokio::test]
async fn test_send_message() {
    let (agent_service, session_id) = create_test_service().await;

    let response = agent_service
        .send_message(session_id, "Hello, world!".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());
    assert_eq!(response.model, "mock-model");
    assert!(response.cost > 0.0);
}

#[tokio::test]
async fn test_send_message_with_system_brain() {
    let (agent_service, session_id) = create_test_service().await;

    let agent_service = agent_service.with_system_brain("You are a helpful assistant.".to_string());

    let response = agent_service
        .send_message(session_id, "Hello!".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());
}

/// Mock provider that simulates tool use
struct MockProviderWithTools {
    call_count: std::sync::Mutex<usize>,
}

impl MockProviderWithTools {
    fn new() -> Self {
        Self {
            call_count: std::sync::Mutex::new(0),
        }
    }
}

#[async_trait]
impl Provider for MockProviderWithTools {
    async fn complete(&self, _request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        let call_num = *count;

        if call_num == 1 {
            // First call: request tool use
            Ok(LLMResponse {
                id: "test-response-1".to_string(),
                model: "mock-model".to_string(),
                content: vec![
                    ContentBlock::Text {
                        text: "I'll use the test tool.".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "tool-1".to_string(),
                        name: "test_tool".to_string(),
                        input: serde_json::json!({"message": "test"}),
                    },
                ],
                stop_reason: Some(StopReason::ToolUse),
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                },
            })
        } else {
            // Second call: final response after tool execution
            Ok(LLMResponse {
                id: "test-response-2".to_string(),
                model: "mock-model".to_string(),
                content: vec![ContentBlock::Text {
                    text: "Tool execution completed successfully.".to_string(),
                }],
                stop_reason: Some(StopReason::EndTurn),
                usage: TokenUsage {
                    input_tokens: 15,
                    output_tokens: 25,
                },
            })
        }
    }

    async fn stream(&self, request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

        // Get the response that complete() would return, then convert to stream events
        let response = self.complete(request).await?;
        let mut events = vec![Ok(StreamEvent::MessageStart {
            message: StreamMessage {
                id: response.id.clone(),
                model: response.model.clone(),
                role: Role::Assistant,
                usage: response.usage,
            },
        })];

        for (i, block) in response.content.iter().enumerate() {
            // ContentBlockStart sends empty shells; actual content comes via deltas
            match block {
                ContentBlock::Text { text } => {
                    events.push(Ok(StreamEvent::ContentBlockStart {
                        index: i,
                        content_block: ContentBlock::Text {
                            text: String::new(),
                        },
                    }));
                    events.push(Ok(StreamEvent::ContentBlockDelta {
                        index: i,
                        delta: ContentDelta::TextDelta { text: text.clone() },
                    }));
                }
                ContentBlock::ToolUse { id, name, input } => {
                    events.push(Ok(StreamEvent::ContentBlockStart {
                        index: i,
                        content_block: ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: serde_json::Value::Object(Default::default()),
                        },
                    }));
                    events.push(Ok(StreamEvent::ContentBlockDelta {
                        index: i,
                        delta: ContentDelta::InputJsonDelta {
                            partial_json: serde_json::to_string(input).unwrap_or_default(),
                        },
                    }));
                }
                _ => {
                    events.push(Ok(StreamEvent::ContentBlockStart {
                        index: i,
                        content_block: block.clone(),
                    }));
                }
            }
            events.push(Ok(StreamEvent::ContentBlockStop { index: i }));
        }

        events.push(Ok(StreamEvent::MessageDelta {
            delta: MessageDelta {
                stop_reason: response.stop_reason,
                stop_sequence: None,
            },
            usage: response.usage,
        }));
        events.push(Ok(StreamEvent::MessageStop));

        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        "mock-with-tools"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(4096)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.001
    }
}

/// Mock tool for testing
struct MockTool;

#[async_trait]
impl crate::brain::tools::Tool for MockTool {
    fn name(&self) -> &str {
        "test_tool"
    }

    fn description(&self) -> &str {
        "A test tool"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {"type": "string"}
            }
        })
    }

    fn capabilities(&self) -> Vec<crate::brain::tools::ToolCapability> {
        vec![]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _context: &crate::brain::tools::ToolExecutionContext,
    ) -> crate::brain::tools::Result<crate::brain::tools::ToolResult> {
        Ok(crate::brain::tools::ToolResult::success(
            "Tool executed successfully".to_string(),
        ))
    }
}

#[tokio::test]
async fn test_send_message_with_tool_execution() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();

    let context = ServiceContext::new(pool);
    let provider = Arc::new(MockProviderWithTools::new());

    // Create tool registry and register our test tool
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true);

    // Create a test session
    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test Session".to_string()))
        .await
        .unwrap();

    // Send message with tool execution
    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());
    assert!(response.content.contains("completed successfully"));
    assert_eq!(response.model, "mock-model");
    // Should have tokens from both calls
    assert!(response.usage.input_tokens >= 25); // 10 + 15
    assert!(response.usage.output_tokens >= 45); // 20 + 25
}

#[tokio::test]
async fn test_message_queue_injection_between_tool_calls() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();

    let context = ServiceContext::new(pool);
    let provider = Arc::new(MockProviderWithTools::new());

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    // Set up a message queue with a queued message
    let queue: Arc<tokio::sync::Mutex<Option<String>>> =
        Arc::new(tokio::sync::Mutex::new(Some("user follow-up".to_string())));

    let queue_clone = queue.clone();
    let message_queue_callback: MessageQueueCallback = Arc::new(move || {
        let q = queue_clone.clone();
        Box::pin(async move { q.lock().await.take() })
    });

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true)
        .with_message_queue_callback(Some(message_queue_callback));

    let session_service = SessionService::new(context.clone());
    let session = session_service
        .create_session(Some("Queue Test".to_string()))
        .await
        .unwrap();

    // Send message — the mock provider will do a tool call on first LLM call,
    // then the queue callback will inject "user follow-up" between iterations
    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());

    // Verify the queue was drained
    assert!(queue.lock().await.is_none());

    // Verify the injected message was saved to database
    let message_service = MessageService::new(context);
    let messages = message_service
        .list_messages_for_session(session.id)
        .await
        .unwrap();

    let user_messages: Vec<_> = messages.iter().filter(|m| m.role == "user").collect();

    // Should have original message + injected follow-up
    assert!(
        user_messages.len() >= 2,
        "expected at least 2 user messages (original + injected), got {}",
        user_messages.len()
    );

    let has_followup = user_messages.iter().any(|m| m.content == "user follow-up");
    assert!(
        has_followup,
        "injected follow-up message not found in database"
    );
}

#[tokio::test]
async fn test_message_queue_empty_no_injection() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();

    let context = ServiceContext::new(pool);
    let provider = Arc::new(MockProviderWithTools::new());

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    // Empty queue — should not inject anything
    let queue: Arc<tokio::sync::Mutex<Option<String>>> = Arc::new(tokio::sync::Mutex::new(None));

    let queue_clone = queue.clone();
    let message_queue_callback: MessageQueueCallback = Arc::new(move || {
        let q = queue_clone.clone();
        Box::pin(async move { q.lock().await.take() })
    });

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true)
        .with_message_queue_callback(Some(message_queue_callback));

    let session_service = SessionService::new(context.clone());
    let session = session_service
        .create_session(Some("Empty Queue Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());

    // Only 1 user message (the original), no injected messages
    let message_service = MessageService::new(context);
    let messages = message_service
        .list_messages_for_session(session.id)
        .await
        .unwrap();

    let user_messages: Vec<_> = messages.iter().filter(|m| m.role == "user").collect();

    assert_eq!(
        user_messages.len(),
        1,
        "should only have original user message"
    );
}

#[tokio::test]
async fn test_stream_complete_text_only() {
    // Verify stream_complete reconstructs a text-only response correctly
    let (agent_service, _) = create_test_service().await;

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);

    let (response, reasoning) = agent_service
        .stream_complete(Uuid::nil(), request, None)
        .await
        .unwrap();
    assert!(
        reasoning.is_none(),
        "mock provider should not produce reasoning"
    );
    assert_eq!(response.model, "mock-model");
    assert!(!response.content.is_empty());

    // Should have a text block
    let has_text = response
        .content
        .iter()
        .any(|b| matches!(b, ContentBlock::Text { text } if !text.is_empty()));
    assert!(has_text, "response should contain non-empty text");
    assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
    assert!(response.usage.input_tokens > 0 || response.usage.output_tokens > 0);
}

#[tokio::test]
async fn test_stream_complete_with_tool_use() {
    // Verify stream_complete reconstructs tool use blocks from stream events
    let provider = Arc::new(MockProviderWithTools::new());
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let agent_service = AgentService::new(provider, context);

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Use a tool")]);

    let (response, reasoning) = agent_service
        .stream_complete(Uuid::nil(), request, None)
        .await
        .unwrap();
    assert!(
        reasoning.is_none(),
        "mock provider should not produce reasoning"
    );

    // First call to MockProviderWithTools returns text + tool_use
    let text_blocks: Vec<_> = response
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::Text { .. }))
        .collect();
    let tool_blocks: Vec<_> = response
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
        .collect();

    assert!(!text_blocks.is_empty(), "should have text block");
    assert!(!tool_blocks.is_empty(), "should have tool_use block");
    assert_eq!(response.stop_reason, Some(StopReason::ToolUse));

    // Verify tool use has correct name and parsed input
    if let ContentBlock::ToolUse { name, input, .. } = &tool_blocks[0] {
        assert_eq!(name, "test_tool");
        assert_eq!(input.get("message").and_then(|v| v.as_str()), Some("test"));
    }
}

#[tokio::test]
async fn test_streaming_chunks_emitted() {
    // Verify StreamingChunk progress events are emitted during streaming
    use std::sync::Mutex;

    let provider = Arc::new(MockProvider);
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());

    let chunks_received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = chunks_received.clone();

    let progress_cb: ProgressCallback = Arc::new(move |_session_id, event| {
        if let ProgressEvent::StreamingChunk { text } = event {
            chunks_clone.lock().unwrap().push(text);
        }
    });

    let agent_service =
        AgentService::new(provider, context).with_progress_callback(Some(progress_cb));

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);

    let (response, reasoning) = agent_service
        .stream_complete(Uuid::nil(), request, None)
        .await
        .unwrap();
    assert!(
        reasoning.is_none(),
        "mock provider should not produce reasoning"
    );
    assert!(!response.content.is_empty(), "response should have content");

    let chunks = chunks_received.lock().unwrap();
    assert!(!chunks.is_empty(), "should have received streaming chunks");
    let combined: String = chunks.iter().cloned().collect();
    assert!(!combined.is_empty(), "combined chunks should have content");
}

#[tokio::test]
async fn test_context_tokens_is_last_iteration_not_accumulated() {
    // When tool loop runs 2 iterations (10 + 15 input tokens),
    // context_tokens should be 15 (last iteration), not 25 (sum).
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let provider = Arc::new(MockProviderWithTools::new());

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true);

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Context Tokens Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    // usage.input_tokens = accumulated (10 + 15 = 25) — for billing
    assert_eq!(response.usage.input_tokens, 25);
    // context_tokens = last iteration only (15) — for display
    assert_eq!(response.context_tokens, 15);
}

#[tokio::test]
async fn test_context_tokens_equals_input_tokens_without_tools() {
    // Without tool use, context_tokens should equal usage.input_tokens
    // (single API call, no accumulation).
    let (agent_service, session_id) = create_test_service().await;

    let response = agent_service
        .send_message(session_id, "Hello".to_string(), None)
        .await
        .unwrap();

    assert_eq!(response.context_tokens, response.usage.input_tokens);
    assert_eq!(response.context_tokens, 10); // MockProvider returns 10
}
