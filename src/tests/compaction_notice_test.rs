//! Regression for #1521: the compaction walk tells the user what it is doing.
//!
//! After `/compact` the TUI showed one "requested" line, then nothing for
//! three full attempt budgets, then an error. Every provider timeout, every
//! move to the next provider and the exhausted chain were `tracing` lines
//! only. Manual compaction now reports each step; automatic compaction
//! reports only a failure, and a background summariser that dies is
//! announced where its result would have been applied.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::brain::agent::service::AgentService;
use crate::brain::agent::service::compaction_notice::{
    CompactionNotifier, CompactionStep, describe,
};
use crate::brain::agent::service::{ProgressCallback, ProgressEvent};
use crate::brain::provider::{
    ContentBlock, LLMRequest, LLMResponse, Message, Provider, ProviderError, ProviderStream,
    TokenUsage,
};

/// Collects the alert lines a callback received.
fn recorder() -> (ProgressCallback, Arc<Mutex<Vec<String>>>) {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let cb: ProgressCallback = Arc::new(move |_session, event| {
        if let ProgressEvent::SelfHealingAlert { message } = event {
            sink.lock().unwrap().push(message);
        }
    });
    (cb, seen)
}

fn failed(provider: &str) -> CompactionStep {
    CompactionStep::AttemptFailed {
        provider: provider.to_string(),
        reason: "timeout".to_string(),
    }
}

#[test]
fn every_step_has_a_line_that_names_the_provider() {
    assert_eq!(
        describe(&failed("qwen")),
        "Compaction: 'qwen' failed (timeout), walking the fallback chain"
    );
    assert_eq!(
        describe(&CompactionStep::TryingFallback {
            provider: "zai".to_string(),
            model: "glm-5.3-flash".to_string(),
        }),
        "Compaction: trying 'zai' with model 'glm-5.3-flash'"
    );
    assert_eq!(
        describe(&CompactionStep::Failed {
            detail: "All providers in the fallback chain failed.".to_string(),
        }),
        "Compaction failed: All providers in the fallback chain failed."
    );
}

#[test]
fn a_manual_notifier_reports_every_step() {
    let (cb, seen) = recorder();
    let n = CompactionNotifier::manual(Uuid::new_v4(), cb);
    n.step(failed("qwen"));
    n.step(CompactionStep::TryingFallback {
        provider: "zai".to_string(),
        model: "glm".to_string(),
    });
    n.step(CompactionStep::Failed {
        detail: "chain exhausted".to_string(),
    });
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[test]
fn an_auto_notifier_reports_only_a_failure() {
    let (cb, seen) = recorder();
    let n = CompactionNotifier::auto(Uuid::new_v4(), cb);
    n.step(failed("qwen"));
    n.step(CompactionStep::TryingFallback {
        provider: "zai".to_string(),
        model: "glm".to_string(),
    });
    assert!(
        seen.lock().unwrap().is_empty(),
        "automatic compaction stays quiet while it walks"
    );
    n.step(CompactionStep::Failed {
        detail: "chain exhausted".to_string(),
    });
    let lines = seen.lock().unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("Compaction failed:"));
}

#[test]
fn no_callback_means_no_notifier() {
    let id = Uuid::new_v4();
    assert!(CompactionNotifier::manual_from(id, None).is_none());
    assert!(CompactionNotifier::auto_from(id, None).is_none());
    let (cb, _) = recorder();
    assert!(CompactionNotifier::manual_from(id, Some(&cb)).is_some());
}

/// A summariser mock that either answers or times out the walk's way.
struct Mock {
    name: String,
    ok: bool,
}

#[async_trait]
impl Provider for Mock {
    async fn complete(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<LLMResponse> {
        self.answer(request)
    }

    async fn stream(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<ProviderStream> {
        let response = self.answer(request)?;
        let text = match response.content.first() {
            Some(ContentBlock::Text { text }) => text.clone(),
            _ => String::new(),
        };
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(crate::brain::provider::StreamEvent::MessageStart {
                message: crate::brain::provider::StreamMessage {
                    id: response.id,
                    model: response.model,
                    role: crate::brain::provider::Role::Assistant,
                    usage: TokenUsage::default(),
                },
            }),
            Ok(crate::brain::provider::StreamEvent::ContentBlockDelta {
                index: 0,
                delta: crate::brain::provider::ContentDelta::TextDelta { text },
            }),
            Ok(crate::brain::provider::StreamEvent::MessageStop),
        ])))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        "m"
    }

    fn supported_models(&self) -> Vec<String> {
        Vec::new()
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(200_000)
    }

    fn calculate_cost(&self, _model: &str, _input_tokens: u32, _output_tokens: u32) -> f64 {
        0.0
    }
}

impl Mock {
    fn answer(&self, request: LLMRequest) -> crate::brain::provider::error::Result<LLMResponse> {
        if self.ok {
            Ok(LLMResponse {
                id: format!("{}-response", self.name),
                model: request.model,
                content: vec![ContentBlock::Text {
                    text: format!("summary from {}", self.name),
                }],
                stop_reason: None,
                usage: TokenUsage::default(),
                streaming_active_secs: None,
                tool_text_leak: false,
            })
        } else {
            Err(ProviderError::Timeout(300))
        }
    }
}

fn provider(name: &str, ok: bool) -> Arc<dyn Provider> {
    Arc::new(Mock {
        name: name.to_string(),
        ok,
    })
}

fn request() -> LLMRequest {
    LLMRequest::new("m", vec![Message::user("summarise")])
}

#[tokio::test]
async fn a_manual_walk_narrates_the_failure_and_the_fallback() {
    let (cb, seen) = recorder();
    let session = Uuid::new_v4();
    let notifier = CompactionNotifier::manual(session, cb);
    let primary = provider("primary", false);
    let fallback = provider("second", true);

    let response = AgentService::complete_compaction_request(
        &primary,
        &[fallback],
        request(),
        &CancellationToken::new(),
        std::time::Duration::from_secs(30),
        Some(&notifier),
    )
    .await
    .expect("served by the fallback");
    assert_eq!(response.id, "second-response");

    let lines = seen.lock().unwrap();
    assert_eq!(
        *lines,
        vec![
            "Compaction: 'primary' failed (timeout), walking the fallback chain".to_string(),
            "Compaction: trying 'second' with model 'm'".to_string(),
        ]
    );
}

#[tokio::test]
async fn an_exhausted_walk_ends_with_one_failure_line_under_auto() {
    let (cb, seen) = recorder();
    let notifier = CompactionNotifier::auto(Uuid::new_v4(), cb);
    let primary = provider("primary", false);
    let fallback = provider("second", false);

    AgentService::complete_compaction_request(
        &primary,
        &[fallback],
        request(),
        &CancellationToken::new(),
        std::time::Duration::from_secs(30),
        Some(&notifier),
    )
    .await
    .expect_err("nothing answered");

    let lines = seen.lock().unwrap();
    assert_eq!(lines.len(), 1, "auto reports the failure only: {lines:?}");
    assert!(lines[0].starts_with("Compaction failed: All providers in the fallback chain failed."));
}

#[tokio::test]
async fn a_primary_with_no_chain_still_reports_its_failure() {
    let (cb, seen) = recorder();
    let notifier = CompactionNotifier::auto(Uuid::new_v4(), cb);
    let primary = provider("only", false);

    AgentService::complete_compaction_request(
        &primary,
        &[],
        request(),
        &CancellationToken::new(),
        std::time::Duration::from_secs(30),
        Some(&notifier),
    )
    .await
    .expect_err("nothing answered");

    let lines = seen.lock().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "Compaction failed: 'only': timeout");
}

/// The manual `/compact` site hands a verbose notifier in, the automatic
/// sites hand a quiet one, and a finished background summariser that failed
/// is announced on the callback that would have applied it.
#[test]
fn call_sites_pick_the_right_notifier() {
    const TOOL_LOOP: &str = include_str!("../brain/agent/service/tool_loop.rs");
    const COMPACTION: &str = include_str!("../brain/agent/service/compaction.rs");
    let manual = TOOL_LOOP
        .find("if is_manual_compact {")
        .expect("manual site");
    assert!(TOOL_LOOP[manual..manual + 700].contains("CompactionNotifier::manual_from("));
    assert_eq!(
        TOOL_LOOP.matches("CompactionNotifier::auto_from(").count(),
        2,
        "emergency and hard-trigger compactions are automatic"
    );
    assert_eq!(
        COMPACTION.matches("CompactionNotifier::auto_from(").count(),
        2
    );
    let failed = COMPACTION
        .find("Background compaction failed after")
        .expect("background failure arm");
    assert!(COMPACTION[failed..failed + 700].contains("SelfHealingAlert"));
}
