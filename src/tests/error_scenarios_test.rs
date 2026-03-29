//! Error Scenario Tests
//!
//! Comprehensive error handling tests for various failure modes.

use anyhow::Result;
use async_trait::async_trait;
use opencrabs::{
    db::Database,
    llm::{
        agent::AgentService,
        provider::{
            error::{ProviderError, Result as ProviderResult},
            types::{ContentBlock, LLMRequest, LLMResponse, StopReason, TokenUsage},
            Provider, ProviderStream,
        },
        tools::{bash::BashTool, read::ReadTool, registry::ToolRegistry, write::WriteTool},
    },
    services::{ServiceContext, SessionService},
};
use std::sync::Arc;
use uuid::Uuid;

/// Mock provider that always returns errors
struct ErrorMockProvider {
    error_type: ErrorType,
}

#[derive(Clone)]
enum ErrorType {
    ApiError,
    RateLimit,
    Timeout,
    InvalidResponse,
    AuthenticationError,
}

impl ErrorMockProvider {
    fn new(error_type: ErrorType) -> Self {
        Self { error_type }
    }
}

#[async_trait]
impl Provider for ErrorMockProvider {
    async fn complete(&self, _request: LLMRequest) -> ProviderResult<LLMResponse> {
        match self.error_type {
            ErrorType::ApiError => Err(ProviderError::ApiError {
                status: 500,
                message: "Internal server error".to_string(),
                error_type: None,
            }),
            ErrorType::RateLimit => Err(ProviderError::RateLimitExceeded(
                "Rate limit exceeded, retry after 60 seconds".to_string(),
            )),
            ErrorType::Timeout => Err(ProviderError::Timeout(30)),
            ErrorType::InvalidResponse => Err(ProviderError::InvalidRequest(
                "Malformed JSON response".to_string(),
            )),
            ErrorType::AuthenticationError => Err(ProviderError::InvalidApiKey),
        }
    }

    async fn stream(&self, _request: LLMRequest) -> ProviderResult<ProviderStream> {
        Err(ProviderError::StreamingNotSupported)
    }

    fn name(&self) -> &str {
        "error-mock"
    }

    fn default_model(&self) -> &str {
        "error-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["error-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(8192)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.0
    }
}

async fn create_test_db() -> Result<Database> {
    let db = Database::connect_in_memory().await?;
    db.run_migrations().await?;
    Ok(db)
}

async fn create_error_agent(
    db: &Database,
    error_type: ErrorType,
) -> Result<(AgentService, ServiceContext)> {
    let provider = Arc::new(ErrorMockProvider::new(error_type));
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
async fn test_error_api_error() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_error_agent(&db, ErrorType::ApiError).await?;

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Error Test".to_string()))
        .await?;

    let result = agent_service
        .send_message(session.id, "Test".to_string(), None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Internal server error") || err.to_string().contains("500"));
    Ok(())
}

#[tokio::test]
async fn test_error_rate_limit() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_error_agent(&db, ErrorType::RateLimit).await?;

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Rate Limit Test".to_string()))
        .await?;

    let result = agent_service
        .send_message(session.id, "Test".to_string(), None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("rate limit") || err.to_string().contains("Rate limit"));
    Ok(())
}

#[tokio::test]
async fn test_error_timeout() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_error_agent(&db, ErrorType::Timeout).await?;

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Timeout Test".to_string()))
        .await?;

    let result = agent_service
        .send_message(session.id, "Test".to_string(), None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_str = err.to_string().to_lowercase();
    assert!(err_str.contains("timeout") || err_str.contains("timed out"));
    Ok(())
}

#[tokio::test]
async fn test_error_invalid_response() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, service_context) =
        create_error_agent(&db, ErrorType::InvalidResponse).await?;

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Invalid Response Test".to_string()))
        .await?;

    let result = agent_service
        .send_message(session.id, "Test".to_string(), None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Invalid") || err.to_string().contains("Malformed"));
    Ok(())
}

#[tokio::test]
async fn test_error_authentication() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, service_context) =
        create_error_agent(&db, ErrorType::AuthenticationError).await?;

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Auth Test".to_string()))
        .await?;

    let result = agent_service
        .send_message(session.id, "Test".to_string(), None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("Invalid API key")
            || err.to_string().contains("Authentication")
            || err.to_string().contains("auth")
    );
    Ok(())
}

#[tokio::test]
async fn test_error_session_not_found() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, _service_context) = create_error_agent(&db, ErrorType::ApiError).await?;

    // Try to send message to non-existent session
    let fake_session_id = Uuid::new_v4();
    let result = agent_service
        .send_message(fake_session_id, "Test".to_string(), None)
        .await;

    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn test_error_empty_message() -> Result<()> {
    let db = create_test_db().await?;

    // Use a working provider for this test (not error provider)
    let provider = Arc::new(WorkingMockProvider);
    let service_context = ServiceContext::new(db.pool().clone());

    let agent_service = AgentService::new_for_test(provider, service_context.clone());

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Empty Message Test".to_string()))
        .await?;

    // Try to send empty message
    let result = agent_service
        .send_message(session.id, "".to_string(), None)
        .await;

    // Should succeed but with empty or default response
    // The provider should handle empty input gracefully
    assert!(result.is_ok());
    Ok(())
}

#[tokio::test]
async fn test_error_database_concurrent_access() -> Result<()> {
    let db = create_test_db().await?;
    let service_context = ServiceContext::new(db.pool().clone());
    let session_service = SessionService::new(service_context.clone());

    // Create session
    let session = session_service
        .create_session(Some("Concurrent Test".to_string()))
        .await?;

    // Try to access from multiple "threads" (tasks)
    let session_id = session.id;
    let service1 = SessionService::new(service_context.clone());
    let service2 = SessionService::new(service_context);

    let handle1 = tokio::spawn(async move { service1.get_session(session_id).await });

    let handle2 = tokio::spawn(async move { service2.get_session(session_id).await });

    let result1 = handle1.await??;
    let result2 = handle2.await??;

    // Both should succeed
    assert!(result1.is_some());
    assert!(result2.is_some());
    Ok(())
}

#[tokio::test]
async fn test_error_recovery_after_failure() -> Result<()> {
    let db = create_test_db().await?;
    let (agent_service, service_context) = create_error_agent(&db, ErrorType::Timeout).await?;

    let session_service = SessionService::new(service_context);
    let session = session_service
        .create_session(Some("Recovery Test".to_string()))
        .await?;

    // First attempt should fail
    let result1 = agent_service
        .send_message(session.id, "Test 1".to_string(), None)
        .await;
    assert!(result1.is_err());

    // Second attempt should also fail (same provider)
    let result2 = agent_service
        .send_message(session.id, "Test 2".to_string(), None)
        .await;
    assert!(result2.is_err());

    // Session should still exist
    let loaded_session = session_service.get_session(session.id).await?;
    assert!(loaded_session.is_some());
    Ok(())
}

/// Working mock provider for testing non-error scenarios
struct WorkingMockProvider;

#[async_trait]
impl Provider for WorkingMockProvider {
    async fn complete(&self, _request: LLMRequest) -> ProviderResult<LLMResponse> {
        Ok(LLMResponse {
            id: "test-id".to_string(),
            model: "test-model".to_string(),
            content: vec![ContentBlock::Text {
                text: "Test response".to_string(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 20,
            },
        })
    }

    async fn stream(&self, _request: LLMRequest) -> ProviderResult<ProviderStream> {
        Err(ProviderError::StreamingNotSupported)
    }

    fn name(&self) -> &str {
        "working-mock"
    }

    fn default_model(&self) -> &str {
        "test-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["test-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(8192)
    }

    fn calculate_cost(&self, _model: &str, input: u32, output: u32) -> f64 {
        ((input + output) as f64 / 1000.0) * 0.001
    }
}
