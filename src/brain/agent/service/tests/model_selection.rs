use super::*;

#[tokio::test]
async fn test_explicit_model_override() {
    // send_message with Some("custom-model") → response.model = "custom-model"
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    // MockProviderWithModel echoes back the requested model name
    let provider = Arc::new(MockProviderWithModel::new("test-provider", "default-model"));
    let agent_service = AgentService::new(provider, context.clone());

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Model Override Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message(
            session.id,
            "Hello".to_string(),
            Some("custom-model".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(
        response.model, "custom-model",
        "response.model should reflect the requested override"
    );
}

#[tokio::test]
async fn test_default_model_fallback() {
    // send_message with None → uses provider's default_model()
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProviderWithModel::new("test-provider", "provider-default"));
    let agent_service = AgentService::new(provider, context.clone());

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Default Model Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message(session.id, "Hello".to_string(), None)
        .await
        .unwrap();

    assert_eq!(
        response.model, "provider-default",
        "response.model should be provider's default_model() when None passed"
    );
}

#[tokio::test]
async fn test_swap_provider_changes_default_model() {
    // After swap_provider(), provider_model() returns new default
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let initial_provider = Arc::new(MockProviderWithModel::new("provider-1", "model-1"));
    let agent_service = AgentService::new(initial_provider, context);

    assert_eq!(
        agent_service.provider_model(),
        "model-1",
        "initial provider_model() should be model-1"
    );
    assert_eq!(agent_service.provider_name(), "provider-1");

    let new_provider = Arc::new(MockProviderWithModel::new("provider-2", "model-2"));
    agent_service.swap_provider(new_provider);

    assert_eq!(
        agent_service.provider_model(),
        "model-2",
        "after swap_provider(), provider_model() should be model-2"
    );
    assert_eq!(agent_service.provider_name(), "provider-2");
}

#[tokio::test]
async fn test_create_session_with_provider_stores_metadata() {
    // create_session_with_provider() persists provider_name and model fields
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session_with_provider(
            Some("Provider Metadata Test".to_string()),
            Some("my-provider".to_string()),
            Some("my-model".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(session.provider_name, Some("my-provider".to_string()));
    assert_eq!(session.model, Some("my-model".to_string()));

    // Retrieve from DB and verify persistence
    let retrieved = session_service
        .get_session_required(session.id)
        .await
        .unwrap();
    assert_eq!(retrieved.provider_name, Some("my-provider".to_string()));
    assert_eq!(retrieved.model, Some("my-model".to_string()));
}

#[tokio::test]
async fn test_model_in_response_matches_request() {
    // AgentResponse.model equals the model name used in the request
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProviderWithModel::new("echo-provider", "echo-default"));
    let agent_service = AgentService::new(provider, context.clone());

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Model Response Test".to_string()))
        .await
        .unwrap();

    let requested_model = "requested-model-xyz".to_string();
    let response = agent_service
        .send_message(session.id, "Hello".to_string(), Some(requested_model.clone()))
        .await
        .unwrap();

    assert_eq!(
        response.model, requested_model,
        "AgentResponse.model should equal the requested model"
    );
}

#[tokio::test]
async fn test_different_sessions_different_models() {
    // Session A uses model-a, session B uses model-b, correct model in each response
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProviderWithModel::new("shared-provider", "default"));
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

    let svc = Arc::clone(&agent_service);
    let (resp_a, resp_b) = tokio::join!(
        svc.send_message(
            session_a.id,
            "Hello".to_string(),
            Some("model-a".to_string())
        ),
        agent_service.send_message(
            session_b.id,
            "Hello".to_string(),
            Some("model-b".to_string())
        ),
    );

    let resp_a = resp_a.unwrap();
    let resp_b = resp_b.unwrap();

    assert_eq!(resp_a.model, "model-a", "session A should use model-a");
    assert_eq!(resp_b.model, "model-b", "session B should use model-b");
}
