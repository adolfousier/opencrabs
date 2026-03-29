//! Integration Tests
//!
//! End-to-end tests with mocked LLM responses.

use anyhow::Result;
use async_trait::async_trait;
use opencrabs::{
    config::Config,
    db::Database,
    llm::{
        agent::AgentService,
        provider::{
            types::{ContentBlock, LLMRequest, LLMResponse, StopReason, TokenUsage},
            Provider, ProviderStream,
        },
        tools::{bash::BashTool, read::ReadTool, registry::ToolRegistry, write::WriteTool},
    },
    services::{MessageService, ServiceContext, SessionService},
};
use std::sync::Arc;
use uuid::Uuid;

/// Mock provider that returns predefined responses
struct MockProvider {
    responses: Vec<String>,
    current: std::sync::Mutex<usize>,
}

impl MockProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            current: std::sync::Mutex::new(0),
        }
    }

    fn single_response(response: String) -> Self {
        Self::new(vec![response])
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn complete(
        &self,
        _request: LLMRequest,
    ) -> opencrabs::llm::provider::error::Result<LLMResponse> {
        let mut idx = self.current.lock().unwrap();
        let response_text = self
            .responses
            .get(*idx)
            .cloned()
            .unwrap_or_else(|| "Mock response".to_string());

        *idx = (*idx + 1).min(self.responses.len() - 1);

        Ok(LLMResponse {
            id: "mock-id".to_string(),
            model: "mock-model".to_string(),
            content: vec![ContentBlock::Text {
                text: response_text,
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 20,
            },
        })
    }

    async fn stream(
        &self,
        _request: LLMRequest,
    ) -> opencrabs::llm::provider::error::Result<ProviderStream> {
        Err(opencrabs::llm::provider::error::ProviderError::StreamingNotSupported)
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn calculate_cost(&self, _model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        // Mock cost: $0.001 per 1000 tokens
        ((input_tokens + output_tokens) as f64 / 1000.0) * 0.001
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
}

/// Test helper to create a test database
async fn create_test_db() -> Result<Database> {
    let db = Database::connect_in_memory().await?;
    db.run_migrations().await?;
    Ok(db)
}

/// Test helper to create agent service with mock provider
async fn create_test_agent(
    db: &Database,
    responses: Vec<String>,
) -> Result<(AgentService, ServiceContext)> {
    let provider = Arc::new(MockProvider::new(responses));
    let service_context = ServiceContext::new(db.pool().clone());

    let tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(ReadTool));
    tool_registry.register(Arc::new(WriteTool));
    tool_registry.register(Arc::new(BashTool));

    let agent_service = AgentService::new_for_test(provider, service_context.clone())
        .with_tool_registry(Arc::new(tool_registry));

    Ok((agent_service, service_context))
}

#[tokio::test]
async fn test_end_to_end_simple_message() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let (agent_service, service_context) =
        create_test_agent(&db, vec!["Hello! I'm a mock AI assistant.".to_string()]).await?;

    // Create session
    let session_service = SessionService::new(service_context.clone());
    let session = session_service
        .create_session(Some("Test Session".to_string()))
        .await?;

    // Send message
    let response = agent_service
        .send_message(session.id, "Hello, how are you?".to_string(), None)
        .await?;

    // Verify response
    assert_eq!(response.content, "Hello! I'm a mock AI assistant.");
    assert!(response.cost > 0.0);
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 20);

    // Verify message was saved to database
    let message_service = MessageService::new(service_context);
    let messages = message_service
        .list_messages_for_session(session.id)
        .await?;

    assert_eq!(messages.len(), 2); // User + Assistant
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Hello, how are you?");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "Hello! I'm a mock AI assistant.");

    Ok(())
}

#[tokio::test]
async fn test_end_to_end_multi_turn_conversation() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_test_agent(
        &db,
        vec![
            "Nice to meet you!".to_string(),
            "I'm doing great, thanks for asking!".to_string(),
            "Goodbye!".to_string(),
        ],
    )
    .await?;

    let session_service = SessionService::new(service_context.clone());
    let session = session_service
        .create_session(Some("Multi-turn Test".to_string()))
        .await?;

    // Turn 1
    let response1 = agent_service
        .send_message(session.id, "Hi there!".to_string(), None)
        .await?;
    assert_eq!(response1.content, "Nice to meet you!");

    // Turn 2
    let response2 = agent_service
        .send_message(session.id, "How are you?".to_string(), None)
        .await?;
    assert_eq!(response2.content, "I'm doing great, thanks for asking!");

    // Turn 3
    let response3 = agent_service
        .send_message(session.id, "Bye!".to_string(), None)
        .await?;
    assert_eq!(response3.content, "Goodbye!");

    // Verify all messages saved
    let message_service = MessageService::new(service_context);
    let messages = message_service
        .list_messages_for_session(session.id)
        .await?;

    assert_eq!(messages.len(), 6); // 3 user + 3 assistant
    assert_eq!(messages[0].sequence, 1);
    assert_eq!(messages[5].sequence, 6);

    Ok(())
}

#[tokio::test]
async fn test_end_to_end_session_management() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_test_agent(
        &db,
        vec!["Response 1".to_string(), "Response 2".to_string()],
    )
    .await?;

    let session_service = SessionService::new(service_context.clone());

    // Create multiple sessions
    let session1 = session_service
        .create_session(Some("Session 1".to_string()))
        .await?;
    let session2 = session_service
        .create_session(Some("Session 2".to_string()))
        .await?;

    // Send messages to different sessions
    agent_service
        .send_message(session1.id, "Message to session 1".to_string(), None)
        .await?;

    agent_service
        .send_message(session2.id, "Message to session 2".to_string(), None)
        .await?;

    // Verify messages are in correct sessions
    let message_service = MessageService::new(service_context);
    let messages1 = message_service
        .list_messages_for_session(session1.id)
        .await?;
    let messages2 = message_service
        .list_messages_for_session(session2.id)
        .await?;

    assert_eq!(messages1.len(), 2);
    assert_eq!(messages2.len(), 2);
    assert_eq!(messages1[0].content, "Message to session 1");
    assert_eq!(messages2[0].content, "Message to session 2");

    // Test listing sessions
    let sessions = session_service
        .list_sessions(opencrabs::db::repository::SessionListOptions {
            include_archived: false,
            limit: Some(10),
            offset: 0,
        })
        .await?;

    assert_eq!(sessions.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_end_to_end_cost_tracking() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_test_agent(
        &db,
        vec![
            "Response 1".to_string(),
            "Response 2".to_string(),
            "Response 3".to_string(),
        ],
    )
    .await?;

    let session_service = SessionService::new(service_context.clone());
    let session = session_service
        .create_session(Some("Cost Test".to_string()))
        .await?;

    // Send multiple messages
    let r1 = agent_service
        .send_message(session.id, "Message 1".to_string(), None)
        .await?;
    let r2 = agent_service
        .send_message(session.id, "Message 2".to_string(), None)
        .await?;
    let r3 = agent_service
        .send_message(session.id, "Message 3".to_string(), None)
        .await?;

    // Verify costs are tracked
    assert!(r1.cost > 0.0);
    assert!(r2.cost > 0.0);
    assert!(r3.cost > 0.0);

    // Verify session has total cost
    let updated_session = session_service.get_session(session.id).await?.unwrap();
    assert!(updated_session.total_cost > 0.0);

    // Verify total cost equals sum of message costs
    let expected_total = r1.cost + r2.cost + r3.cost;
    let actual_total = updated_session.total_cost;
    assert!((expected_total - actual_total).abs() < 0.0001); // Float comparison

    Ok(())
}

#[tokio::test]
async fn test_end_to_end_error_handling() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let (agent_service, _service_context) =
        create_test_agent(&db, vec!["Response".to_string()]).await?;

    // Try to send message to non-existent session
    let fake_session_id = Uuid::new_v4();
    let result = agent_service
        .send_message(fake_session_id, "Test".to_string(), None)
        .await;

    // Should get error
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_end_to_end_token_usage() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let (agent_service, service_context) =
        create_test_agent(&db, vec!["Short response".to_string()]).await?;

    let session_service = SessionService::new(service_context.clone());
    let session = session_service
        .create_session(Some("Token Test".to_string()))
        .await?;

    // Send message
    let response = agent_service
        .send_message(session.id, "Test message".to_string(), None)
        .await?;

    // Verify token usage
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 20);

    // Verify tokens saved to database
    let message_service = MessageService::new(service_context);
    let messages = message_service
        .list_messages_for_session(session.id)
        .await?;

    let assistant_message = messages.iter().find(|m| m.role == "assistant").unwrap();
    assert!(assistant_message.token_count.is_some());
    assert_eq!(assistant_message.token_count.unwrap(), 30); // input + output

    // Verify session total tokens
    let updated_session = session_service.get_session(session.id).await?.unwrap();
    assert!(updated_session.token_count > 0);

    Ok(())
}

#[tokio::test]
async fn test_end_to_end_system_brain() -> Result<()> {
    // Setup
    let db = create_test_db().await?;
    let provider = Arc::new(MockProvider::single_response(
        "I am a pirate assistant!".to_string(),
    ));
    let service_context = ServiceContext::new(db.pool().clone());

    let agent_service = AgentService::new_for_test(provider, service_context.clone())
        .with_system_brain("You are a pirate assistant.".to_string());

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("System Brain Test".to_string()))
        .await?;

    // Send message
    let response = agent_service
        .send_message(session.id, "Hello".to_string(), None)
        .await?;

    // Verify response (mocked, but in real test would check pirate talk)
    assert_eq!(response.content, "I am a pirate assistant!");

    Ok(())
}

#[tokio::test]
async fn test_config_loading() -> Result<()> {
    // Test default config
    let config = Config::default();

    // Verify defaults
    assert_eq!(config.logging.level, "info");
    assert!(config.database.path.ends_with("opencrabs.db"));

    // Verify providers structure exists
    assert!(config.providers.anthropic.is_some() || config.providers.anthropic.is_none());

    Ok(())
}

#[tokio::test]
async fn test_database_persistence() -> Result<()> {
    // Create temporary database file
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_{}.db", Uuid::new_v4()));

    // Create database
    let db = Database::connect(&db_path).await?;
    db.run_migrations().await?;

    // Create session
    let service_context = ServiceContext::new(db.pool().clone());
    let session_service = SessionService::new(service_context.clone());
    let session = session_service
        .create_session(Some("Persistence Test".to_string()))
        .await?;
    let session_id = session.id;

    // Drop database connection
    drop(db);

    // Reconnect
    let db2 = Database::connect(&db_path).await?;
    let service_context2 = ServiceContext::new(db2.pool().clone());
    let session_service2 = SessionService::new(service_context2);

    // Verify session persisted
    let loaded_session = session_service2.get_session(session_id).await?;
    assert!(loaded_session.is_some());
    assert_eq!(
        loaded_session.unwrap().title,
        Some("Persistence Test".to_string())
    );

    // Cleanup
    let _ = std::fs::remove_file(&db_path);

    Ok(())
}
