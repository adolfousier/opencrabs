use super::*;

#[tokio::test]
async fn test_concurrent_sessions_independent() {
    // Two sessions send messages via tokio::join!, both get correct responses
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProvider);
    let agent_service = Arc::new(AgentService::new(provider, context.clone()));

    let session_service = SessionService::new(context);
    let session_a = session_service
        .create_session(Some("Session A".to_string()))
        .await
        .unwrap();
    let session_b = session_service
        .create_session(Some("Session B".to_string()))
        .await
        .unwrap();

    let svc_a = Arc::clone(&agent_service);
    let svc_b = Arc::clone(&agent_service);
    let id_a = session_a.id;
    let id_b = session_b.id;

    let (resp_a, resp_b) = tokio::join!(
        svc_a.send_message(id_a, "Hello from A".to_string(), None),
        svc_b.send_message(id_b, "Hello from B".to_string(), None),
    );

    let resp_a = resp_a.unwrap();
    let resp_b = resp_b.unwrap();

    assert!(!resp_a.content.is_empty(), "session A should have a response");
    assert!(!resp_b.content.is_empty(), "session B should have a response");
    assert_eq!(resp_a.model, "mock-model");
    assert_eq!(resp_b.model, "mock-model");
}

#[tokio::test]
async fn test_concurrent_sessions_different_providers() {
    // Two AgentServices with distinct providers, each gets its own response
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider_alpha = Arc::new(MockProviderWithModel::new("alpha", "alpha-model"));
    let provider_beta = Arc::new(MockProviderWithModel::new("beta", "beta-model"));

    let svc_alpha = Arc::new(AgentService::new(provider_alpha, context.clone()));
    let svc_beta = Arc::new(AgentService::new(provider_beta, context.clone()));

    let session_service = SessionService::new(context);
    let session_a = session_service
        .create_session(Some("Alpha Session".to_string()))
        .await
        .unwrap();
    let session_b = session_service
        .create_session(Some("Beta Session".to_string()))
        .await
        .unwrap();

    let id_a = session_a.id;
    let id_b = session_b.id;

    let (resp_a, resp_b) = tokio::join!(
        svc_alpha.send_message(id_a, "Hello alpha".to_string(), None),
        svc_beta.send_message(id_b, "Hello beta".to_string(), None),
    );

    let resp_a = resp_a.unwrap();
    let resp_b = resp_b.unwrap();

    assert!(
        resp_a.content.contains("alpha"),
        "session A should get response from alpha provider, got: {}",
        resp_a.content
    );
    assert!(
        resp_b.content.contains("beta"),
        "session B should get response from beta provider, got: {}",
        resp_b.content
    );
}

#[tokio::test]
async fn test_cancel_one_session_other_continues() {
    // Cancel token on session A, session B completes normally
    use tokio_util::sync::CancellationToken;

    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProviderWithTools::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let svc_a = Arc::new(
        AgentService::new(Arc::new(MockProviderWithTools::new()), context.clone())
            .with_tool_registry(Arc::new({
                let mut r = ToolRegistry::new();
                r.register(Arc::new(MockTool));
                r
            }))
            .with_auto_approve_tools(true),
    );

    let svc_b = Arc::new(
        AgentService::new(provider, context.clone())
            .with_tool_registry(Arc::new(registry))
            .with_auto_approve_tools(true),
    );

    let session_service = SessionService::new(context);
    let session_a = session_service
        .create_session(Some("Session A".to_string()))
        .await
        .unwrap();
    let session_b = session_service
        .create_session(Some("Session B".to_string()))
        .await
        .unwrap();

    let cancel_a = CancellationToken::new();
    cancel_a.cancel(); // Cancel immediately

    let id_a = session_a.id;
    let id_b = session_b.id;

    let svc_a_clone = Arc::clone(&svc_a);
    let svc_b_clone = Arc::clone(&svc_b);
    let cancel_a_clone = cancel_a.clone();

    let (result_a, result_b) = tokio::join!(
        svc_a_clone.send_message_with_tools_and_mode(
            id_a,
            "Use the tool".to_string(),
            None,
            false,
            Some(cancel_a_clone),
        ),
        svc_b_clone.send_message_with_tools(id_b, "Use the tool".to_string(), None),
    );

    // Session A was cancelled — it may succeed with partial content or succeed normally
    // The important thing is that it doesn't panic and session B completes
    let _ = result_a; // cancelled session result is fine either way

    let resp_b = result_b.unwrap();
    assert!(!resp_b.content.is_empty(), "session B should complete normally");
}

#[tokio::test]
async fn test_message_isolation_between_sessions() {
    // Messages in session A don't appear in session B's list
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProvider);
    let agent_service = Arc::new(AgentService::new(provider, context.clone()));

    let session_service = SessionService::new(context.clone());
    let session_a = session_service
        .create_session(Some("Session A".to_string()))
        .await
        .unwrap();
    let session_b = session_service
        .create_session(Some("Session B".to_string()))
        .await
        .unwrap();

    // Send distinct messages to each session
    agent_service
        .send_message(
            session_a.id,
            "Message only in session A".to_string(),
            None,
        )
        .await
        .unwrap();

    agent_service
        .send_message(
            session_b.id,
            "Message only in session B".to_string(),
            None,
        )
        .await
        .unwrap();

    let message_service = MessageService::new(context);
    let msgs_a = message_service
        .list_messages_for_session(session_a.id)
        .await
        .unwrap();
    let msgs_b = message_service
        .list_messages_for_session(session_b.id)
        .await
        .unwrap();

    let text_a: Vec<&str> = msgs_a.iter().map(|m| m.content.as_str()).collect();
    let text_b: Vec<&str> = msgs_b.iter().map(|m| m.content.as_str()).collect();

    assert!(
        text_a.iter().any(|t| t.contains("only in session A")),
        "session A messages should contain session-A message"
    );
    assert!(
        !text_a.iter().any(|t| t.contains("only in session B")),
        "session A messages should NOT contain session-B message"
    );
    assert!(
        text_b.iter().any(|t| t.contains("only in session B")),
        "session B messages should contain session-B message"
    );
    assert!(
        !text_b.iter().any(|t| t.contains("only in session A")),
        "session B messages should NOT contain session-A message"
    );
}

#[tokio::test]
async fn test_session_usage_tracked_independently() {
    // Token count and cost accumulate per-session, not globally
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProvider);
    let agent_service = Arc::new(AgentService::new(provider, context.clone()));

    let session_service = SessionService::new(context.clone());
    let session_a = session_service
        .create_session(Some("Session A".to_string()))
        .await
        .unwrap();
    let session_b = session_service
        .create_session(Some("Session B".to_string()))
        .await
        .unwrap();

    // Send two messages to session A, one to session B
    agent_service
        .send_message(session_a.id, "First message to A".to_string(), None)
        .await
        .unwrap();
    agent_service
        .send_message(session_a.id, "Second message to A".to_string(), None)
        .await
        .unwrap();
    agent_service
        .send_message(session_b.id, "Only message to B".to_string(), None)
        .await
        .unwrap();

    let updated_a = session_service
        .get_session_required(session_a.id)
        .await
        .unwrap();
    let updated_b = session_service
        .get_session_required(session_b.id)
        .await
        .unwrap();

    // Session A got 2 messages (2 × 30 tokens each), session B got 1 message (30 tokens)
    // MockProvider: 10 input + 20 output = 30 tokens per call
    assert_eq!(
        updated_a.token_count, 60,
        "session A should have 60 tokens (2 messages × 30)"
    );
    assert_eq!(
        updated_b.token_count, 30,
        "session B should have 30 tokens (1 message × 30)"
    );
    assert!(
        updated_a.total_cost > updated_b.total_cost,
        "session A (2 msgs) should have higher cost than session B (1 msg)"
    );
}
