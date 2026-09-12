//! A configured provider is never removed from consideration (#1251).
//!
//! #952 shipped a process-global quarantine table: a provider whose API
//! reported a hard quota was marked exhausted for an hour, and every fallback
//! walk skipped it. The table was keyed by provider name with no model
//! dimension, so one dead model blacked out every model on that provider, for
//! every session in the process, on the strength of a single error. It also
//! marked the user's own active pick.
//!
//! The rule these tests lock: the system reports failures, the human decides.
//! Nothing is skipped, and a failure on one turn is never remembered as a
//! reason to not try again on the next.

use crate::brain::agent::service::AgentService;
use crate::brain::provider::{
    FallbackProvider, LLMRequest, LLMResponse, Provider, ProviderError, ProviderStream, TokenUsage,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Long enough that no mock in this file can reach it: these tests are about
/// which provider gets asked, not about how long one is given to answer.
const ATTEMPT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// What a mock does when called.
#[derive(Clone, Copy)]
enum Outcome {
    /// The exact modelscope wording that #1084 had to teach the old breaker.
    HardQuota,
    Ok,
}

/// Mock provider that counts every call it receives.
struct CountingMock {
    name: String,
    calls: Arc<AtomicUsize>,
    outcome: Outcome,
}

impl CountingMock {
    fn new(name: &str, outcome: Outcome) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(Self {
            name: name.to_string(),
            calls: calls.clone(),
            outcome,
        });
        (mock, calls)
    }

    fn err(&self) -> ProviderError {
        ProviderError::RateLimitExceeded(format!(
            "Rate limit exceeded: You have exceeded this month's quota for model \
             {}, please try again next month",
            self.name
        ))
    }
}

#[async_trait]
impl Provider for CountingMock {
    async fn complete(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<LLMResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.outcome {
            Outcome::HardQuota => Err(self.err()),
            Outcome::Ok => Ok(LLMResponse {
                id: format!("{}-response", self.name),
                model: request.model,
                content: vec![],
                stop_reason: None,
                usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    ..Default::default()
                },
                streaming_active_secs: None,
                tool_text_leak: false,
            }),
        }
    }

    async fn stream(
        &self,
        _request: LLMRequest,
    ) -> crate::brain::provider::error::Result<ProviderStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.outcome {
            Outcome::HardQuota => Err(self.err()),
            Outcome::Ok => Ok(Box::pin(futures::stream::empty())),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        "mock-default"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-default".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(4096)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.0
    }
}

fn request() -> LLMRequest {
    LLMRequest {
        model: "mock-default".into(),
        messages: vec![],
        system: None,
        system_suffix: None,
        max_tokens: None,
        temperature: None,
        tools: None,
        stream: false,
        metadata: None,
        working_directory: None,
        session_id: None,
    }
}

#[tokio::test]
async fn hard_quota_does_not_stop_the_chain_from_trying_every_provider() {
    // Primary and the first fallback both report a hard monthly quota. Under
    // the old breaker the second walk in the same turn skipped whatever the
    // first walk had just marked. Every configured entry must be attempted.
    let (primary, primary_calls) = CountingMock::new("primary", Outcome::HardQuota);
    let (fb1, fb1_calls) = CountingMock::new("fb1", Outcome::HardQuota);
    let (fb2, fb2_calls) = CountingMock::new("fb2", Outcome::Ok);

    let chain = FallbackProvider::new(primary, vec![fb1, fb2]);
    let resp = chain.complete(request()).await;

    assert!(resp.is_ok(), "the healthy third entry must serve the turn");
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1, "primary attempted");
    assert_eq!(
        fb1_calls.load(Ordering::SeqCst),
        1,
        "quota-dead fb1 still attempted"
    );
    assert_eq!(fb2_calls.load(Ordering::SeqCst), 1, "fb2 served");
}

#[tokio::test]
async fn a_quota_failure_is_not_remembered_on_the_next_turn() {
    // The core regression. The breaker marked a quota-failed provider for an
    // hour, so turn 2 never reached it. With no quarantine memory, a provider
    // that failed on turn 1 is attempted again on turn 2 — most of these
    // errors are transient and clear on their own.
    let (primary, primary_calls) = CountingMock::new("primary", Outcome::HardQuota);
    let (fb1, fb1_calls) = CountingMock::new("fb1", Outcome::HardQuota);

    let chain = FallbackProvider::new(primary, vec![fb1]);

    assert!(chain.complete(request()).await.is_err(), "turn 1 exhausts");
    assert!(chain.complete(request()).await.is_err(), "turn 2 exhausts");

    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        2,
        "the user's primary is re-attempted every turn, never quarantined"
    );
    assert_eq!(
        fb1_calls.load(Ordering::SeqCst),
        2,
        "a quota-failed fallback is re-attempted every turn too"
    );
}

#[tokio::test]
async fn exhaustion_error_names_every_provider_tried() {
    // A user who cannot be told what happened cannot make the call that the
    // system is no longer making for them.
    let (primary, _) = CountingMock::new("primary", Outcome::HardQuota);
    let (fb1, _) = CountingMock::new("fb1", Outcome::HardQuota);

    let chain = FallbackProvider::new(primary, vec![fb1]);
    let err = chain
        .complete(request())
        .await
        .expect_err("every entry failed");
    let text = err.to_string();

    assert!(text.contains("primary"), "names the primary: {text}");
    assert!(text.contains("fb1"), "names the fallback tried: {text}");
    assert!(
        !text.contains("Skipped"),
        "nothing is ever skipped, so nothing can be reported as skipped: {text}"
    );
}

#[tokio::test]
async fn a_healthy_primary_is_never_bypassed_at_startup() {
    // `compute_health_start_index` used to read persisted failure counts and
    // start the chain on a fallback, so the user's configured primary was
    // never even attempted. Construction now always starts at the primary.
    let (primary, primary_calls) = CountingMock::new("primary", Outcome::Ok);
    let (fb1, fb1_calls) = CountingMock::new("fb1", Outcome::Ok);

    let chain = FallbackProvider::new(primary, vec![fb1]);
    assert!(chain.complete(request()).await.is_ok());

    assert_eq!(primary_calls.load(Ordering::SeqCst), 1, "primary served");
    assert_eq!(
        fb1_calls.load(Ordering::SeqCst),
        0,
        "no fallback is consulted while the primary works"
    );
}

#[tokio::test]
async fn compaction_re_attempts_a_quota_failed_provider_on_the_next_run() {
    // The compaction walk carried its own copy of the skip (`context.rs`), so
    // a provider marked exhausted by one compaction was invisible to the next
    // one for an hour. Compaction is exactly the path a session in trouble
    // depends on, so a stale quarantine there could strand a full window with
    // no way back. Both runs must attempt every configured entry.
    let (primary, primary_calls) = CountingMock::new("pni-compact-primary", Outcome::HardQuota);
    let (fb1, fb1_calls) = CountingMock::new("pni-compact-fb1", Outcome::HardQuota);
    let primary_dyn: Arc<dyn Provider> = primary;
    let chain: Vec<Arc<dyn Provider>> = vec![fb1];

    for run in 1..=2 {
        let outcome = AgentService::complete_compaction_request(
            &primary_dyn,
            &chain,
            request(),
            &tokio_util::sync::CancellationToken::new(),
            ATTEMPT_DEADLINE,
            None,
        )
        .await;
        assert!(outcome.is_err(), "run {run}: every entry is quota-dead");
    }

    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        2,
        "compaction re-attempts the primary on the second run"
    );
    assert_eq!(
        fb1_calls.load(Ordering::SeqCst),
        2,
        "compaction re-attempts a quota-failed fallback on the second run"
    );
}
