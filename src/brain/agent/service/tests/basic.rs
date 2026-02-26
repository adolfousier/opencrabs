use super::*;

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

#[tokio::test]
async fn test_send_message_with_tool_execution() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();

    let context = ServiceContext::new(pool);
    let provider = Arc::new(MockProviderWithTools::new());

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true);

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test Session".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());
    assert!(response.content.contains("completed successfully"));
    assert_eq!(response.model, "mock-model");
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

    if let ContentBlock::ToolUse { name, input, .. } = &tool_blocks[0] {
        assert_eq!(name, "test_tool");
        assert_eq!(input.get("message").and_then(|v| v.as_str()), Some("test"));
    }
}

#[tokio::test]
async fn test_streaming_chunks_emitted() {
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
    let (agent_service, session_id) = create_test_service().await;

    let response = agent_service
        .send_message(session_id, "Hello".to_string(), None)
        .await
        .unwrap();

    assert_eq!(response.context_tokens, response.usage.input_tokens);
    assert_eq!(response.context_tokens, 10); // MockProvider returns 10
}
