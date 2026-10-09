//! #2009: a provider rotation must not re-persist the turn's user row.
//!
//! `run_tool_loop` runs the whole agent loop inside a retry loop: when a
//! provider is killed by the loop detector the same `user_message` is handed to
//! the next provider on the chain (#1023). Turn-start persistence lived inside
//! `run_tool_loop_inner`, so every re-entry wrote the user's message again. On
//! Telegram that reads as the model answering one message twice, and it grows
//! the history the next prompt replays.
//!
//! Before this file, nothing in the suite entered the rotation path, which is
//! why the duplicate shipped.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::brain::agent::service::AgentService;
use crate::brain::provider::{
    ContentBlock, LLMRequest, LLMResponse, Provider, ProviderError, ProviderStream, StopReason,
    TokenUsage,
};
use crate::db::Database;
use crate::db::repository::MessageRepository;
use crate::services::{ServiceContext, SessionService};
use crate::tests::agent_service_mocks::MockProvider;
use async_trait::async_trait;

/// Kills the first provider call with `AnnouncementLoop` (exactly what the loop
/// detector raises) and answers every call after that. `force_next_fallback`
/// reports one hop, so the rotation loop runs a second attempt and stops.
struct LoopKillThenOk {
    calls: AtomicUsize,
    hops: AtomicUsize,
}

impl LoopKillThenOk {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            hops: AtomicUsize::new(0),
        }
    }
}

fn text_response() -> LLMResponse {
    LLMResponse {
        id: "rotation-test-1".to_string(),
        model: "rotation-mock".to_string(),
        content: vec![ContentBlock::Text {
            text: "answer from the rescued provider".to_string(),
        }],
        stop_reason: Some(StopReason::EndTurn),
        usage: TokenUsage {
            input_tokens: 5,
            output_tokens: 7,
            ..Default::default()
        },
        streaming_active_secs: None,
        tool_text_leak: false,
    }
}

#[async_trait]
impl Provider for LoopKillThenOk {
    async fn complete(&self, _request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            return Err(ProviderError::AnnouncementLoop(
                "test: killed by the loop detector".to_string(),
            ));
        }
        Ok(text_response())
    }

    async fn stream(&self, request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            return Err(ProviderError::AnnouncementLoop(
                "test: killed by the loop detector".to_string(),
            ));
        }
        MockProvider.stream(request).await
    }

    fn force_next_fallback(&self, _reason: &str, _current_model: &str) -> bool {
        // One hop only: the chain is exhausted afterwards, so the loop cannot
        // spin and a second rotation attempt is impossible in this test.
        self.hops.fetch_add(1, Ordering::SeqCst) + 1 == 1
    }

    fn name(&self) -> &str {
        "rotation-mock"
    }

    fn default_model(&self) -> &str {
        "rotation-mock"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["rotation-mock".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(4096)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.0
    }
}

/// One turn, one rotation: the user's message is written exactly once even
/// though the loop re-entered with the same message.
#[tokio::test]
async fn a_rotated_turn_writes_the_user_row_once() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let session = SessionService::new(context.clone())
        .create_session(Some("rotation test".into()))
        .await
        .unwrap();

    let provider = Arc::new(LoopKillThenOk::new());
    let service = AgentService::new_for_test(provider.clone(), context.clone()).await;

    let result = service
        .send_message_with_tools(session.id, "the user asked once".into(), None)
        .await
        .expect("the rotation must rescue the turn instead of dropping it (#1023)");
    assert!(
        !result.content.is_empty(),
        "the rescued provider's answer must come back"
    );

    let rows = MessageRepository::new(db.pool().clone())
        .list_by_session(session.id)
        .await
        .unwrap();
    let user_rows: Vec<_> = rows.iter().filter(|m| m.role == "user").collect();
    let assistant_rows: Vec<_> = rows.iter().filter(|m| m.role == "assistant").collect();

    assert_eq!(
        user_rows.len(),
        1,
        "#2009: the loop-detector rotation re-entered the turn and wrote the same \
         user message {} times; Telegram shows that as a duplicate",
        user_rows.len()
    );
    assert_eq!(user_rows[0].content, "the user asked once");
    assert!(
        !assistant_rows.is_empty(),
        "the turn still needs its assistant row"
    );

    // The vacuity guard. Without a real re-entry this test would only prove that
    // one happy-path turn writes one row, and the fix would be untested.
    // `force_next_fallback` is consulted only by the rotation loop in
    // `run_tool_loop`, so exactly one hop plus a second provider call is the
    // receipt that the same message went through the loop twice.
    assert_eq!(
        provider.hops.load(Ordering::SeqCst),
        1,
        "the rotation loop never asked for a hop, so the turn never re-entered and the \
         one-row assertion above proves nothing (#2009)"
    );
    assert!(
        provider.calls.load(Ordering::SeqCst) >= 2,
        "the second provider was never called; calls={}",
        provider.calls.load(Ordering::SeqCst)
    );
}
