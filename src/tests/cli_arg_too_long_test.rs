//! Test: emergency compaction triggers when CLI provider hits "Argument list too long" (E2BIG).
//!
//! Simulates the scenario where opencode CLI spawn fails because the conversation
//! context exceeds OS ARG_MAX. The tool loop should catch this, auto-compact, and retry.

use crate::{
    brain::{
        agent::service::AgentService,
        provider::{
            Provider, ProviderStream,
            error::{ProviderError, Result as ProviderResult},
            types::{
                ContentBlock, ContentDelta, LLMRequest, LLMResponse, MessageDelta, Role,
                StopReason, StreamEvent, StreamMessage, TokenUsage,
            },
        },
    },
    db::Database,
    services::{ServiceContext, SessionService},
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

/// Mock provider that fails with "Argument list too long" on the first stream() call,
/// then succeeds on subsequent calls (simulating post-compaction retry).
struct ArgTooLongMockProvider {
    call_count: AtomicU32,
}

impl ArgTooLongMockProvider {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl Provider for ArgTooLongMockProvider {
    async fn complete(&self, _request: LLMRequest) -> ProviderResult<LLMResponse> {
        Ok(LLMResponse {
            id: "mock-complete".into(),
            model: "mock-model".into(),
            content: vec![ContentBlock::Text {
                text: "Compaction summary: previous conversation was about testing.".into(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        })
    }

    async fn stream(&self, _request: LLMRequest) -> ProviderResult<ProviderStream> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);

        if n == 0 {
            // First call: simulate E2BIG — the exact error string from opencode_cli.rs
            return Err(ProviderError::Internal(
                "failed to spawn opencode CLI: Argument list too long (os error 7)".into(),
            ));
        }

        // Subsequent calls: return a normal response stream
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(StreamEvent::MessageStart {
                    message: StreamMessage {
                        id: "msg-retry".into(),
                        model: "mock-model".into(),
                        role: Role::Assistant,
                        usage: TokenUsage {
                            input_tokens: 50,
                            output_tokens: 0,
                            ..Default::default()
                        },
                    },
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: ContentBlock::Text {
                        text: String::new(),
                    },
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: ContentDelta::TextDelta {
                        text: "Recovery successful after compaction!".into(),
                    },
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockStop { index: 0 }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::MessageDelta {
                    delta: MessageDelta {
                        stop_reason: Some(StopReason::EndTurn),
                        stop_sequence: None,
                    },
                    usage: TokenUsage {
                        input_tokens: 50,
                        output_tokens: 10,
                        ..Default::default()
                    },
                }))
                .await;
            let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
        });

        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "arg-too-long-mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".into()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(128_000)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.0
    }
}

/// Same but using ContextLengthExceeded variant directly.
struct ContextLengthMockProvider {
    call_count: AtomicU32,
}

impl ContextLengthMockProvider {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl Provider for ContextLengthMockProvider {
    async fn complete(&self, _request: LLMRequest) -> ProviderResult<LLMResponse> {
        Ok(LLMResponse {
            id: "mock-complete".into(),
            model: "mock-model".into(),
            content: vec![ContentBlock::Text {
                text: "Compaction summary.".into(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        })
    }

    async fn stream(&self, _request: LLMRequest) -> ProviderResult<ProviderStream> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);

        if n == 0 {
            return Err(ProviderError::ContextLengthExceeded(500_000));
        }

        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(StreamEvent::MessageStart {
                    message: StreamMessage {
                        id: "msg-retry".into(),
                        model: "mock-model".into(),
                        role: Role::Assistant,
                        usage: TokenUsage {
                            input_tokens: 50,
                            output_tokens: 0,
                            ..Default::default()
                        },
                    },
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: ContentBlock::Text {
                        text: String::new(),
                    },
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: ContentDelta::TextDelta {
                        text: "Recovered after ContextLengthExceeded!".into(),
                    },
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockStop { index: 0 }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::MessageDelta {
                    delta: MessageDelta {
                        stop_reason: Some(StopReason::EndTurn),
                        stop_sequence: None,
                    },
                    usage: TokenUsage {
                        input_tokens: 50,
                        output_tokens: 10,
                        ..Default::default()
                    },
                }))
                .await;
            let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
        });

        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "context-length-mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".into()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(128_000)
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

/// When stream() fails with "Argument list too long", the tool loop should
/// emergency-compact and retry instead of surfacing a raw error.
#[tokio::test]
async fn arg_too_long_triggers_emergency_compaction() -> Result<()> {
    let db = create_test_db().await?;
    let provider = Arc::new(ArgTooLongMockProvider::new());
    let service_context = ServiceContext::new(db.pool().clone());

    let agent = AgentService::new_for_test(provider.clone(), service_context.clone());

    let session_svc = SessionService::new(service_context);
    let session = session_svc
        .create_session(Some("ARG_MAX Test".into()))
        .await?;

    // Send a message — first stream() call will fail with "Argument list too long",
    // tool loop should compact and retry, second call succeeds.
    let result = agent
        .send_message_with_tools(session.id, "hello".into(), None)
        .await;

    // Should succeed (recovered via compaction + retry), not error out
    assert!(
        result.is_ok(),
        "Expected successful recovery after ARG_MAX compaction, got: {:?}",
        result.err()
    );

    let resp = result.unwrap();
    assert!(
        resp.content.contains("Recovery successful"),
        "Response should come from the retry: {}",
        resp.content
    );

    // Provider should have been called at least twice (first fail, then retry after compaction).
    // May be called more due to context budget enforcement or calibration.
    let calls = provider.call_count.load(Ordering::SeqCst);
    assert!(
        calls >= 2,
        "stream() should be called at least twice (fail + retry), got {}",
        calls
    );

    Ok(())
}

/// When stream() fails with ContextLengthExceeded, same compaction + retry should happen.
#[tokio::test]
async fn context_length_exceeded_triggers_emergency_compaction() -> Result<()> {
    let db = create_test_db().await?;
    let provider = Arc::new(ContextLengthMockProvider::new());
    let service_context = ServiceContext::new(db.pool().clone());

    let agent = AgentService::new_for_test(provider.clone(), service_context.clone());

    let session_svc = SessionService::new(service_context);
    let session = session_svc
        .create_session(Some("ContextLength Test".into()))
        .await?;

    let result = agent
        .send_message_with_tools(session.id, "hello".into(), None)
        .await;

    assert!(
        result.is_ok(),
        "Expected successful recovery after ContextLengthExceeded compaction, got: {:?}",
        result.err()
    );

    let resp = result.unwrap();
    assert!(
        resp.content
            .contains("Recovered after ContextLengthExceeded"),
        "Response should come from the retry: {}",
        resp.content
    );

    let calls = provider.call_count.load(Ordering::SeqCst);
    assert!(
        calls >= 2,
        "stream() should be called at least twice (fail + retry), got {}",
        calls
    );

    Ok(())
}
