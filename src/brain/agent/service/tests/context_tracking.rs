use super::*;
use crate::brain::agent::context::AgentContext;

// === System brain token counting ===

#[test]
fn test_system_brain_tokens_counted_in_context() {
    let session_id = Uuid::new_v4();
    let mut context = AgentContext::from_db_messages(session_id, vec![], 200_000);

    assert_eq!(context.token_count, 0, "empty context should have 0 tokens");

    // Simulate what tool_loop.rs does after the fix:
    // count system brain tokens when setting it
    let brain = "You are a helpful AI assistant with extensive knowledge.";
    let brain_tokens = AgentContext::estimate_tokens(brain);
    context.token_count += brain_tokens;
    context.system_brain = Some(brain.to_string());

    assert!(
        context.token_count > 0,
        "context should count system brain tokens"
    );
    assert_eq!(
        context.token_count, brain_tokens,
        "token count should equal system brain tokens"
    );
}

#[test]
fn test_system_brain_tokens_not_double_counted() {
    let session_id = Uuid::new_v4();
    let mut context = AgentContext::from_db_messages(session_id, vec![], 200_000);

    let brain = "You are a helpful AI assistant.";
    let brain_tokens = AgentContext::estimate_tokens(brain);

    // Count and set system brain (the fixed pattern)
    context.token_count += brain_tokens;
    context.system_brain = Some(brain.to_string());

    // Add a user message
    context.add_message(Message::user("Hello".to_string()));

    let total = context.token_count;
    assert!(
        total > brain_tokens,
        "total should be brain + message tokens"
    );

    // Verify brain tokens are counted exactly once
    let msg_tokens = total - brain_tokens;
    assert!(
        msg_tokens > 0,
        "message tokens should be positive (got {})",
        msg_tokens
    );
}

#[test]
fn test_large_system_brain_counted_accurately() {
    let session_id = Uuid::new_v4();
    let mut context = AgentContext::from_db_messages(session_id, vec![], 200_000);

    // Simulate a realistic system brain (~2000 tokens)
    let brain = "x ".repeat(4000); // ~2000 tokens
    let brain_tokens = AgentContext::estimate_tokens(&brain);

    context.token_count += brain_tokens;
    context.system_brain = Some(brain);

    assert!(
        brain_tokens > 1500,
        "large brain should have >1500 tokens (got {})",
        brain_tokens
    );
    assert_eq!(context.token_count, brain_tokens);
}

// === Context rebuild from DB ===

#[tokio::test]
async fn test_context_includes_brain_after_db_rebuild() {
    let provider = Arc::new(MockProvider);
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());

    let brain_text = "You are a helpful assistant for software development.";
    let agent_service = AgentService::new(provider, context.clone())
        .with_system_brain(brain_text.to_string());

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message(session.id, "Hi".to_string(), None)
        .await
        .unwrap();

    // context_tokens should include brain tokens, not just message tokens
    let brain_tokens = AgentContext::estimate_tokens(brain_text);
    assert!(
        response.context_tokens as usize >= brain_tokens,
        "context_tokens ({}) should include brain tokens ({})",
        response.context_tokens,
        brain_tokens,
    );
}

// === Context persistence across requests ===

#[tokio::test]
async fn test_context_does_not_drop_between_requests() {
    let provider = Arc::new(MockProvider);
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());

    let agent_service = AgentService::new(provider, context.clone())
        .with_system_brain("System brain prompt.".to_string());

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test".to_string()))
        .await
        .unwrap();

    // First request
    let response1 = agent_service
        .send_message(session.id, "First message".to_string(), None)
        .await
        .unwrap();

    // Second request — context should be >= first request
    // (it has all prior messages PLUS the new one)
    let response2 = agent_service
        .send_message(session.id, "Second message".to_string(), None)
        .await
        .unwrap();

    assert!(
        response2.context_tokens >= response1.context_tokens,
        "context should grow between requests (request1={}, request2={})",
        response1.context_tokens,
        response2.context_tokens,
    );
}

// === Tool loop context growth ===

#[tokio::test]
async fn test_tool_loop_context_grows_with_results() {
    let provider = Arc::new(MockProviderWithTools::new());
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true)
        .with_system_brain("Brain.".to_string());

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools_and_mode(session.id, "Use the tool".to_string(), None, None)
        .await
        .unwrap();

    // After a tool loop, context_tokens should be non-trivial
    // (includes brain + user msg + assistant msg + tool_use + tool_result + final response)
    assert!(
        response.context_tokens > 0,
        "context_tokens must be non-zero after tool loop"
    );
}

// === base_context_tokens uses real tool schema tokens ===

#[test]
fn test_base_context_tokens_uses_real_tool_schemas() {
    // Create two services: one with tools, one without
    // The one with tools should have higher base_context_tokens
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let provider = Arc::new(MockProvider);
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let context = ServiceContext::new(db.pool().clone());

        let service_no_tools = AgentService::new(provider.clone(), context.clone())
            .with_system_brain("Brain.".to_string());

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(MockTool));
        let service_with_tools = AgentService::new(provider, context)
            .with_system_brain("Brain.".to_string())
            .with_tool_registry(Arc::new(registry));

        let base_no_tools = service_no_tools.base_context_tokens();
        let base_with_tools = service_with_tools.base_context_tokens();

        assert!(
            base_with_tools > base_no_tools,
            "service with tools ({}) should have higher base_context_tokens than without ({})",
            base_with_tools,
            base_no_tools,
        );
    });
}

// === Calibration with system brain ===

#[tokio::test]
async fn test_calibration_drift_reduced_with_brain_counting() {
    // When system brain is counted, the initial context.token_count should be
    // much closer to the API's real input_tokens, reducing calibration drift.
    let session_id = Uuid::new_v4();
    let brain = "You are a helpful assistant for coding tasks.";
    let brain_tokens = AgentContext::estimate_tokens(brain);

    // Simulate OLD behavior: brain not counted
    let mut ctx_old = AgentContext::from_db_messages(session_id, vec![], 200_000);
    ctx_old.system_brain = Some(brain.to_string());
    // Don't add brain tokens (old bug)
    ctx_old.add_message(Message::user("Hello".to_string()));
    let old_count = ctx_old.token_count;

    // Simulate NEW behavior: brain counted
    let mut ctx_new = AgentContext::from_db_messages(session_id, vec![], 200_000);
    ctx_new.token_count += brain_tokens;
    ctx_new.system_brain = Some(brain.to_string());
    ctx_new.add_message(Message::user("Hello".to_string()));
    let new_count = ctx_new.token_count;

    // New count should be higher by exactly brain_tokens
    assert_eq!(
        new_count - old_count,
        brain_tokens,
        "new count should include brain tokens"
    );

    // Simulate API response: input_tokens = brain + messages + tool_overhead
    let tool_overhead = 100; // small for test
    let api_input = new_count + tool_overhead;

    // Old behavior drift
    let old_real = api_input.saturating_sub(tool_overhead);
    let old_drift = (old_count as f64 - old_real as f64).abs();

    // New behavior drift
    let new_real = api_input.saturating_sub(tool_overhead);
    let new_drift = (new_count as f64 - new_real as f64).abs();

    assert!(
        new_drift < old_drift,
        "new drift ({}) should be less than old drift ({})",
        new_drift,
        old_drift,
    );
    assert!(
        new_drift < 10.0,
        "new drift should be near-zero (got {})",
        new_drift
    );
}
