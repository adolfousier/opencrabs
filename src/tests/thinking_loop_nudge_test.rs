//! Regression guards for #1690: the thinking-loop guard nudges instead of
//! killing, and it is scopable per provider.
//!
//! `thinking_loop_timeout_secs` is the "nothing is happening" clock. Before
//! this change it did two things it should not have done:
//!
//! * it read only `Config::current().agent.thinking_loop_timeout_secs`, so a
//!   long-reasoning model and a fast chat model shared one number and neither
//!   could be tuned without touching the other;
//! * on expiry it returned `Err(ThinkingLoopTimeout)` unconditionally, which
//!   tore down a stream that was happily writing its tenth paragraph of an
//!   answer that needs no tools, and made the tool loop replay the whole
//!   conversation with phantom enforcement.
//!
//! The contract now: the ceiling resolves `[providers.<name>]` → `[agent]`, and
//! the guard kills only a stream that has delivered NOTHING. A delivering
//! stream stands the clock down and keeps its text.
//!
//! The behavioural tests drive the real streaming layer through
//! `stream_complete` with a mock provider that controls both its delivery
//! timing and the ceiling it advertises, so the assertion is about what the
//! caller actually receives rather than about a log line.

use crate::brain::provider::factory::create_provider;
use crate::brain::provider::r#trait::Provider;
use crate::brain::provider::{
    ContentBlock, ContentDelta, LLMRequest, LLMResponse, Message, MessageDelta, ProviderError,
    ProviderStream, Role, StreamEvent, StreamMessage, TokenUsage,
};
use crate::config::timeout::resolve_thinking_loop;
use crate::config::{Config, ProviderConfig, ProviderConfigs};
use crate::tests::agent_service_mocks::create_test_service_with_provider;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

fn source(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

// ------------------------------------------------------------ the mock

/// A provider that advertises `secs` as its thinking-loop ceiling and streams
/// `chunks` after sitting silent for `pause_before`, then `gap` apart.
///
/// `pause_before` is the interesting knob: it is the window in which the guard
/// expires with nothing delivered, which is the only case the guard may kill.
struct GuardProvider {
    secs: Option<u64>,
    pause_before: Duration,
    chunks: Vec<String>,
    gap: Duration,
}

impl GuardProvider {
    fn new(secs: Option<u64>, pause_before: Duration, chunks: &[&str], gap: Duration) -> Self {
        Self {
            secs,
            pause_before,
            chunks: chunks.iter().map(|s| (*s).to_string()).collect(),
            gap,
        }
    }

    fn delivering(secs: u64) -> Self {
        Self::new(
            Some(secs),
            Duration::ZERO,
            &["one ", "two ", "three ", "four ", "five ", "six"],
            Duration::from_millis(400),
        )
    }

    fn silent_then(secs: Option<u64>, pause: Duration) -> Self {
        Self::new(secs, pause, &["eventually "], Duration::from_millis(50))
    }
}

#[async_trait]
impl Provider for GuardProvider {
    async fn complete(&self, _request: LLMRequest) -> Result<LLMResponse, ProviderError> {
        unreachable!("test exercises stream(), not complete()");
    }

    async fn stream(&self, _request: LLMRequest) -> Result<ProviderStream, ProviderError> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<StreamEvent, ProviderError>>();
        let chunks = self.chunks.clone();
        let pause = self.pause_before;
        let gap = self.gap;
        tokio::spawn(async move {
            let _ = tx.send(Ok(StreamEvent::MessageStart {
                message: StreamMessage {
                    id: "guard-resp".into(),
                    model: "mock-guard".into(),
                    role: Role::Assistant,
                    usage: TokenUsage::default(),
                },
            }));
            let _ = tx.send(Ok(StreamEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Text {
                    text: String::new(),
                },
            }));
            // Silence first: MessageStart deliberately does NOT count as a
            // delivery, so the guard sees "nothing delivered" across this gap.
            if !pause.is_zero() {
                tokio::time::sleep(pause).await;
            }
            for (i, chunk) in chunks.into_iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(gap).await;
                }
                if tx
                    .send(Ok(StreamEvent::ContentBlockDelta {
                        index: 0,
                        delta: ContentDelta::TextDelta { text: chunk },
                    }))
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send(Ok(StreamEvent::ContentBlockStop { index: 0 }));
            let _ = tx.send(Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: Some(crate::brain::provider::StopReason::EndTurn),
                    stop_sequence: None,
                },
                usage: TokenUsage::default(),
            }));
            let _ = tx.send(Ok(StreamEvent::MessageStop));
        });
        let mut rx = rx;
        Ok(Box::pin(futures::stream::poll_fn(move |cx| {
            rx.poll_recv(cx)
        })))
    }

    fn name(&self) -> &str {
        "mock-guard"
    }

    fn default_model(&self) -> &str {
        "mock-guard"
    }

    fn thinking_loop_timeout(&self) -> Option<u64> {
        self.secs
    }

    // Keep the idle clock well clear of the guard so a failure here is
    // unambiguously about the thinking-loop behaviour.
    fn stream_idle_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-guard".into()]
    }

    fn context_window(&self, _: &str) -> Option<u32> {
        Some(4096)
    }

    fn calculate_cost(&self, _: &str, _: u32, _: u32) -> f64 {
        0.0
    }
}

fn text_of(resp: &LLMResponse) -> String {
    resp.content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

async fn stream_with(provider: GuardProvider) -> Result<LLMResponse, ProviderError> {
    let provider = Arc::new(provider) as Arc<dyn Provider>;
    let (svc, _) = create_test_service_with_provider(provider).await;
    let request = LLMRequest::new("mock-guard".to_string(), vec![Message::user("go")]);
    let (response, _stop_reason) = svc
        .stream_complete(Uuid::nil(), request, None, None, None, None, false)
        .await?;
    Ok(response)
}

// ------------------------------------------------------- behavioural
//
// Contract, stated in the names: a stream that is still delivering is never
// discarded by this guard, and a stream that has delivered nothing still is.

#[tokio::test]
async fn a_stream_still_delivering_when_the_guard_expires_keeps_every_chunk() {
    // Ceiling 1s; six chunks 400ms apart, so the deadline lands mid-answer
    // (around chunk 3) with no tool call in sight. Pre-#1690 this returned
    // Err(ThinkingLoopTimeout(1)) and the caller lost three paragraphs.
    let result = stream_with(GuardProvider::delivering(1)).await;
    let resp = result.expect("#1690: the guard must not discard a delivering stream");
    let text = text_of(&resp);
    assert_eq!(
        text, "one two three four five six",
        "chunks after the deadline were dropped — the answer is truncated at \
         the guard boundary, which is the exact regression #1690 removes"
    );
    assert_eq!(
        resp.stop_reason,
        Some(crate::brain::provider::StopReason::EndTurn),
        "the stream did not run to its natural end"
    );
}

#[tokio::test]
async fn a_stream_that_delivered_nothing_is_still_killed_so_the_phantom_retry_runs() {
    // The genuine "thinking forever, saying nothing" signature keeps its old
    // behaviour: the tool loop needs the error to trigger phantom enforcement.
    let result = stream_with(GuardProvider::silent_then(
        Some(1),
        Duration::from_millis(3_000),
    ))
    .await;
    match result {
        Err(ProviderError::ThinkingLoopTimeout(secs)) => assert_eq!(
            secs, 1,
            "the provider-scoped ceiling did not reach the guard (#1690)"
        ),
        other => panic!("expected ThinkingLoopTimeout for a silent stream, got {other:?}"),
    }
}

#[tokio::test]
async fn a_provider_ceiling_of_zero_disarms_the_guard_entirely() {
    // `0` is a VALUE on this clock, not an absent one: it switches the guard off
    // for this provider. The same 2s silence kills at ceiling 1 (test above) and
    // is tolerated here.
    let result = stream_with(GuardProvider::silent_then(
        Some(0),
        Duration::from_millis(2_000),
    ))
    .await;
    let resp = result.expect("`thinking_loop_timeout_secs = 0` must disable the guard");
    assert_eq!(text_of(&resp), "eventually ");
}

// ---------------------------------------------------------- resolution

/// A config with exactly one family configured, keyed, and enabled.
fn scoped(family: &str, secs: Option<u64>) -> Config {
    let mut config = Config::default();
    let provider = ProviderConfig {
        enabled: true,
        api_key: Some("test-key".to_string()),
        thinking_loop_timeout_secs: secs,
        ..Default::default()
    };
    config.providers = ProviderConfigs {
        anthropic: (family == "anthropic").then(|| provider.clone()),
        gemini: (family == "gemini").then(|| provider.clone()),
        openai: (family == "openai").then(|| provider.clone()),
        ..Default::default()
    };
    config
}

#[tokio::test]
async fn every_family_prefers_its_own_ceiling_over_the_agent_tier() {
    for family in ["anthropic", "gemini", "openai"] {
        let mut config = scoped(family, Some(30));
        config.agent.thinking_loop_timeout_secs = 90;
        let provider = create_provider(&config)
            .await
            .unwrap_or_else(|e| panic!("{family} builds: {e}"));
        assert_eq!(
            provider.thinking_loop_timeout(),
            Some(30),
            "`[providers.{family}] thinking_loop_timeout_secs` is not read (#1690)"
        );
    }
}

#[tokio::test]
async fn every_family_falls_back_to_the_agent_ceiling() {
    for family in ["anthropic", "gemini", "openai"] {
        let mut config = scoped(family, None);
        config.agent.thinking_loop_timeout_secs = 90;
        let provider = create_provider(&config)
            .await
            .unwrap_or_else(|e| panic!("{family} builds: {e}"));
        assert_eq!(
            provider.thinking_loop_timeout(),
            Some(90),
            "`[agent] thinking_loop_timeout_secs` does not reach {family} (#1690)"
        );
    }
}

#[test]
fn the_agent_default_stays_six_hundred_seconds() {
    // The issue pinned this: the global default must not move.
    assert_eq!(Config::default().agent.thinking_loop_timeout_secs, 600);
}

#[test]
fn zero_is_a_value_and_not_skipped_the_way_the_transport_clocks_skip_it() {
    assert_eq!(resolve_thinking_loop(Some(0), 600), 0);
    assert_eq!(resolve_thinking_loop(Some(30), 600), 30);
    assert_eq!(resolve_thinking_loop(None, 600), 600);
    assert_eq!(resolve_thinking_loop(None, 0), 0);
}

// ----------------------------------------------------------- wiring

#[test]
fn the_guard_asks_the_provider_before_it_reads_the_global_config() {
    let src = source("src/brain/agent/service/helpers.rs");
    assert!(
        src.contains("provider.thinking_loop_timeout()"),
        "the thinking-loop ceiling is still read straight from \
         `Config::current().agent`, so it cannot be scoped per provider (#1690)"
    );
    assert!(
        src.contains("let thinking_loop_timeout_secs = if is_cli {"),
        "CLI providers run tools internally, so a tool-less stream is normal \
         and must stay exempt (#1690)"
    );
}

#[test]
fn the_kill_branch_is_reachable_only_with_nothing_delivered() {
    let src = source("src/brain/agent/service/helpers.rs");
    assert_eq!(
        src.matches("ProviderError::ThinkingLoopTimeout(").count(),
        1,
        "there must be exactly one place that turns this guard into an error"
    );
    assert!(
        src.contains("last_delta_at.is_some()"),
        "the guard no longer distinguishes a delivering stream from a silent \
         one — it will discard an in-flight answer (#1690)"
    );
    assert!(
        src.contains("thinking_loop_armed = false"),
        "the guard is never stood down, so a long answer is re-killed on \
         every loop iteration (#1690)"
    );
}

#[test]
fn all_three_families_apply_the_resolved_ceiling() {
    let src = source("src/brain/provider/factory.rs");
    assert!(
        src.matches("with_thinking_loop_timeout(").count() >= 3,
        "a family was left out of the thinking-loop wiring (#1690)"
    );
    assert!(
        src.contains("resolve_thinking_loop("),
        "the ceiling is not resolved through the shared chain (#1690)"
    );
}
