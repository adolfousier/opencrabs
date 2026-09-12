//! Compaction walks `[providers.fallback]` like every other request path
//! (#1247).
//!
//! `compact_context` called `provider.complete()` exactly once and surfaced
//! whatever came back. A session whose primary was out of credit therefore kept
//! chatting fine (the tool loop walks the chain) while `/compact` failed every
//! time on the dead primary. That is the worst possible pairing: the session
//! that most needs compacting is the one whose window is full, and a session
//! that cannot compact cannot recover.
//!
//! Also pins the shared fall-through policy: a hard quota / 402 billing error
//! MUST advance the chain. `FallbackProvider::should_try_next` used to gate on
//! `is_retryable()` alone, and `is_retryable()` deliberately returns false for
//! hard quota (#952: don't burn backoff on a wall) — so the one error class a
//! chain exists to survive was the one that aborted it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::brain::agent::service::AgentService;
use crate::brain::provider::chain_order::widest_first;
use crate::brain::provider::error::{should_try_next_provider, with_chain_summary};
use crate::brain::provider::fallback::substitute_model;
use crate::brain::provider::{
    ContentDelta, LLMRequest, LLMResponse, Message, Provider, ProviderError, ProviderStream, Role,
    StreamEvent, StreamMessage, TokenUsage,
};

/// Long enough that no mock in this file can reach it: these tests are about
/// which provider gets asked, not about how long one is given to answer.
const ATTEMPT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// How a mock answers `complete`.
enum Behaviour {
    Ok,
    /// Hard quota / no balance — the exact shape z.ai and modelscope return.
    QuotaExhausted,
    /// Not fall-through-able: nothing downstream should be tried.
    Fatal,
    /// The payload does not fit this provider's window (#1379).
    TooLong,
    /// Never answers. CLI providers ship no request timeout, so this is what
    /// a wedged summariser actually looks like (#1255) — not an error, just
    /// silence for as long as the process lives.
    Hangs,
}

struct CountingMock {
    name: String,
    behaviour: Behaviour,
    models: Vec<String>,
    /// What `default_model()` answers; the model a substitute runs (#1374).
    default: String,
    /// What `context_window()` answers; decides the walk order on an
    /// overflow (#1379). `None` models a provider that publishes no window.
    window: Option<u32>,
    calls: Arc<AtomicUsize>,
    /// Model string of the last request this mock received.
    last_model: Arc<std::sync::Mutex<Option<String>>>,
}

impl CountingMock {
    fn new(name: &str, behaviour: Behaviour) -> Self {
        Self {
            name: name.to_string(),
            behaviour,
            models: Vec::new(),
            default: "mock-default".to_string(),
            window: Some(200_000),
            calls: Arc::new(AtomicUsize::new(0)),
            last_model: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Restrict this provider's catalogue, so a request for anything else has
    /// to be remapped to `default_model()` before it is sent.
    fn with_models(mut self, models: &[&str]) -> Self {
        self.models = models.iter().map(|m| m.to_string()).collect();
        self
    }

    /// What this provider is configured to run. Empty models a provider that
    /// publishes no default.
    fn with_default(mut self, default: &str) -> Self {
        self.default = default.to_string();
        self
    }

    fn with_window(mut self, window: Option<u32>) -> Self {
        self.window = window;
        self
    }

    fn call_counter(&self) -> Arc<AtomicUsize> {
        self.calls.clone()
    }

    fn model_spy(&self) -> Arc<std::sync::Mutex<Option<String>>> {
        self.last_model.clone()
    }
}

#[async_trait]
impl Provider for CountingMock {
    async fn complete(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<LLMResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_model.lock().unwrap() = Some(request.model.clone());
        match self.behaviour {
            Behaviour::Ok => Ok(LLMResponse {
                id: format!("{}-response", self.name),
                model: request.model,
                content: vec![crate::brain::provider::ContentBlock::Text {
                    text: format!("summary from {}", self.name),
                }],
                stop_reason: None,
                usage: TokenUsage::default(),
                streaming_active_secs: None,
                tool_text_leak: false,
            }),
            Behaviour::QuotaExhausted => Err(ProviderError::RateLimitExceeded(
                "Insufficient balance or no resource package. Please recharge.".to_string(),
            )),
            Behaviour::Fatal => Err(ProviderError::Internal("mock fatal".to_string())),
            Behaviour::TooLong => Err(ProviderError::ContextLengthExceeded(0)),
            Behaviour::Hangs => {
                futures::future::pending::<()>().await;
                unreachable!("a hanging provider never returns")
            }
        }
    }

    /// Same behaviour as `complete()`: compaction asks an HTTP provider
    /// through the stream path and folds the events into one response
    /// (#1519), so the mock answers both the same way.
    async fn stream(
        &self,
        request: LLMRequest,
    ) -> crate::brain::provider::error::Result<ProviderStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_model.lock().unwrap() = Some(request.model.clone());
        match self.behaviour {
            Behaviour::Ok => Ok(Box::pin(futures::stream::iter(vec![
                Ok(StreamEvent::MessageStart {
                    message: StreamMessage {
                        id: format!("{}-response", self.name),
                        model: request.model,
                        role: Role::Assistant,
                        usage: TokenUsage::default(),
                    },
                }),
                Ok(StreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: ContentDelta::TextDelta {
                        text: format!("summary from {}", self.name),
                    },
                }),
                Ok(StreamEvent::MessageStop),
            ]))),
            Behaviour::QuotaExhausted => Err(ProviderError::RateLimitExceeded(
                "Insufficient balance or no resource package. Please recharge.".to_string(),
            )),
            Behaviour::Fatal => Err(ProviderError::Internal("mock fatal".to_string())),
            Behaviour::TooLong => Err(ProviderError::ContextLengthExceeded(0)),
            Behaviour::Hangs => {
                futures::future::pending::<()>().await;
                unreachable!("a hanging provider never returns")
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        &self.default
    }

    fn supported_models(&self) -> Vec<String> {
        self.models.clone()
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        self.window
    }

    fn calculate_cost(&self, _model: &str, _input_tokens: u32, _output_tokens: u32) -> f64 {
        0.0
    }
}

fn request(model: &str) -> LLMRequest {
    LLMRequest::new(model.to_string(), vec![Message::user("compact me")])
}

/// The headline regression: primary out of credit, chain healthy, compaction
/// must succeed instead of surfacing the primary's billing error.
#[tokio::test]
async fn quota_on_primary_falls_through_to_the_chain() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-primary-quota",
        Behaviour::QuotaExhausted,
    ));
    let healthy = CountingMock::new("cfc-healthy", Behaviour::Ok);
    let healthy_calls = healthy.call_counter();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(healthy)];

    let response = AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("primary-model"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("#1247: the chain must serve compaction when the primary is dead");

    assert_eq!(response.id, "cfc-healthy-response");
    assert_eq!(
        healthy_calls.load(Ordering::SeqCst),
        1,
        "the fallback must actually be called, once"
    );
}

/// A provider that doesn't publish the requested model gets the request
/// remapped to its own default — the same invariant the chat path enforces.
#[tokio::test]
async fn fallback_model_is_remapped_when_unsupported() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-primary-remap",
        Behaviour::QuotaExhausted,
    ));
    let healthy =
        CountingMock::new("cfc-remap-target", Behaviour::Ok).with_models(&["mock-default"]);
    let seen = healthy.model_spy();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(healthy)];

    AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("a-model-only-the-primary-has"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("remapped request must succeed");

    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("mock-default"),
        "a cross-provider model must never be sent to a fallback"
    );
}

/// A substitute runs its own configured model even when it lists the one the
/// failed request carried (#1374): `models[]` is capability, `default_model`
/// is intent, and carrying the model along meant the chain never tried
/// anything different.
#[tokio::test]
async fn a_fallback_listing_the_requested_model_still_runs_its_own_default() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-primary-shared",
        Behaviour::QuotaExhausted,
    ));
    let substitute = CountingMock::new("cfc-substitute", Behaviour::Ok)
        .with_models(&["shared-model", "substitute-default"])
        .with_default("substitute-default");
    let seen = substitute.model_spy();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(substitute)];

    AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("shared-model"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("the substitute answers");

    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("substitute-default"),
        "the carried model must not ride along just because the substitute lists it"
    );
}

/// Two chain entries on the same endpoint, each configured for a different
/// model, produce two different requests instead of one request twice
/// (#1374: the observed 3 x 300s of byte-identical retries).
#[tokio::test]
async fn two_providers_sharing_an_endpoint_get_different_requests() {
    let primary: Arc<dyn Provider> = Arc::new(
        CountingMock::new("cfc-host-a", Behaviour::QuotaExhausted)
            .with_models(&["big", "mid", "small"])
            .with_default("big"),
    );
    let mid = CountingMock::new("cfc-host-b", Behaviour::QuotaExhausted)
        .with_models(&["big", "mid", "small"])
        .with_default("mid");
    let small = CountingMock::new("cfc-host-c", Behaviour::Ok)
        .with_models(&["big", "mid", "small"])
        .with_default("small");
    let seen_mid = mid.model_spy();
    let seen_small = small.model_spy();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(mid), Arc::new(small)];

    AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("big"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("the last substitute answers");

    assert_eq!(seen_mid.lock().unwrap().as_deref(), Some("mid"));
    assert_eq!(seen_small.lock().unwrap().as_deref(), Some("small"));
}

/// A substitute with no usable default keeps the requested model rather than
/// being sent an empty model id.
#[test]
fn a_substitute_without_a_default_keeps_the_requested_model() {
    let bare = CountingMock::new("cfc-no-default", Behaviour::Ok).with_default("  ");
    assert_eq!(
        substitute_model(&bare, "whatever-was-asked"),
        "whatever-was-asked"
    );

    let configured = CountingMock::new("cfc-with-default", Behaviour::Ok).with_default("mine");
    assert_eq!(
        substitute_model(&configured, "whatever-was-asked"),
        "mine",
        "the configured default wins whenever there is one"
    );
}

/// A context-length rejection walks the chain (#1379): a summariser the
/// primary refused on size is one a wider-window fallback can accept, and the
/// primary that just refused is not asked again.
#[tokio::test]
async fn context_overflow_on_primary_reaches_the_first_fallback() {
    let primary_mock = CountingMock::new("cfc-primary-overflow", Behaviour::TooLong);
    let primary_calls = primary_mock.call_counter();
    let primary: Arc<dyn Provider> = Arc::new(primary_mock);
    let wide = CountingMock::new("cfc-wide", Behaviour::Ok);
    let wide_calls = wide.call_counter();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(wide)];

    let response = AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("mock-default"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("the wider fallback summarises");

    assert_eq!(response.id, "cfc-wide-response");
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        1,
        "refused once, never re-asked"
    );
    assert_eq!(wide_calls.load(Ordering::SeqCst), 1);
}

/// On an overflow the widest window is asked first, whatever the configured
/// order says (#1379): the narrow entry configured first would only refuse.
#[tokio::test]
async fn an_overflow_walks_the_widest_window_first() {
    let primary: Arc<dyn Provider> = Arc::new(
        CountingMock::new("cfc-overflow-primary", Behaviour::TooLong).with_window(Some(200_000)),
    );
    let narrow = CountingMock::new("cfc-narrow", Behaviour::Ok).with_window(Some(128_000));
    let wide = CountingMock::new("cfc-wide", Behaviour::Ok).with_window(Some(1_000_000));
    let narrow_calls = narrow.call_counter();
    let wide_calls = wide.call_counter();
    // Configured narrow first.
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(narrow), Arc::new(wide)];

    let response = AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("mock-default"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("the wide fallback summarises");

    assert_eq!(response.id, "cfc-wide-response");
    assert_eq!(wide_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        narrow_calls.load(Ordering::SeqCst),
        0,
        "the narrower entry is not spent before the one that can answer"
    );
}

/// Any other failure keeps the configured order: the reorder is for size
/// only, so this does not widen into a general re-ranking of the chain.
#[tokio::test]
async fn a_quota_failure_keeps_the_configured_order() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-quota-primary",
        Behaviour::QuotaExhausted,
    ));
    let narrow = CountingMock::new("cfc-narrow-first", Behaviour::Ok).with_window(Some(128_000));
    let wide = CountingMock::new("cfc-wide-second", Behaviour::Ok).with_window(Some(1_000_000));
    let narrow_calls = narrow.call_counter();
    let wide_calls = wide.call_counter();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(narrow), Arc::new(wide)];

    let response = AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("mock-default"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect("the first configured fallback answers");

    assert_eq!(response.id, "cfc-narrow-first-response");
    assert_eq!(narrow_calls.load(Ordering::SeqCst), 1);
    assert_eq!(wide_calls.load(Ordering::SeqCst), 0);
}

/// The ordering rule on its own: widest first, stable for ties, unknown last.
#[test]
fn widest_first_is_stable_and_puts_unknown_windows_last() {
    let mk = |name: &str, w: Option<u32>| -> Arc<dyn Provider> {
        Arc::new(CountingMock::new(name, Behaviour::Ok).with_window(w))
    };
    let chain = vec![
        mk("unknown-a", None),
        mk("small", Some(128_000)),
        mk("big-first", Some(1_000_000)),
        mk("mid", Some(200_000)),
        mk("big-second", Some(1_000_000)),
        mk("unknown-b", None),
    ];
    let names: Vec<String> = widest_first(&chain)
        .iter()
        .map(|p| p.name().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "big-first",
            "big-second",
            "mid",
            "small",
            "unknown-a",
            "unknown-b"
        ]
    );
}

/// The policy itself, and the property the chat path relies on: an
/// exhausted chain hands the tool loop the same variant, so its emergency
/// compaction still recognises the overflow.
#[test]
fn context_length_exceeded_advances_the_chain_and_survives_the_summary() {
    let err = ProviderError::ContextLengthExceeded(0);
    assert!(
        !err.is_retryable(),
        "sanity: re-sending to the same provider is pointless"
    );
    assert!(
        should_try_next_provider(&err),
        "#1379: but the next provider may have the room"
    );

    let wrapped = with_chain_summary(err, "tried: a, b".to_string());
    assert!(
        matches!(wrapped, ProviderError::ContextLengthExceeded(0)),
        "the chain summary must not change the variant the tool loop matches on"
    );
}

/// No chain configured: the primary's error is surfaced verbatim, not masked
/// behind a "chain exhausted" summary that would name providers that don't
/// exist.
#[tokio::test]
async fn empty_chain_surfaces_the_primary_error() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-primary-alone",
        Behaviour::QuotaExhausted,
    ));

    let err = AgentService::complete_compaction_request(
        &primary,
        &[],
        request("primary-model"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect_err("nothing to fall back to");

    assert!(
        err.to_string().contains("Insufficient balance"),
        "expected the raw provider error, got: {err}"
    );
}

/// An error that is not fall-through-able must not spend the chain.
#[tokio::test]
async fn fatal_error_does_not_walk_the_chain() {
    let primary: Arc<dyn Provider> =
        Arc::new(CountingMock::new("cfc-primary-fatal", Behaviour::Fatal));
    let untouched = CountingMock::new("cfc-untouched", Behaviour::Ok);
    let untouched_calls = untouched.call_counter();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(untouched)];

    AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("primary-model"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect_err("a fatal error stays fatal");

    assert_eq!(
        untouched_calls.load(Ordering::SeqCst),
        0,
        "a non-retryable, non-quota failure must not be retried elsewhere"
    );
}

/// Everything dead: the caller gets a ledger naming what was tried, not one
/// provider's raw failure.
#[tokio::test]
async fn exhausted_chain_reports_what_was_tried() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-primary-exhaust",
        Behaviour::QuotaExhausted,
    ));
    let chain: Vec<Arc<dyn Provider>> = vec![
        Arc::new(CountingMock::new("cfc-dead-one", Behaviour::QuotaExhausted)),
        Arc::new(CountingMock::new("cfc-dead-two", Behaviour::QuotaExhausted)),
    ];

    let err = AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("primary-model"),
        &CancellationToken::new(),
        ATTEMPT_DEADLINE,
    )
    .await
    .expect_err("every provider failed");

    let text = err.to_string();
    assert!(
        text.contains("cfc-dead-one") && text.contains("cfc-dead-two"),
        "the failure ledger must name each provider tried, got: {text}"
    );
}

/// A cancelled compaction stops immediately rather than walking the chain.
#[tokio::test]
async fn cancellation_short_circuits_the_walk() {
    let primary: Arc<dyn Provider> = Arc::new(CountingMock::new(
        "cfc-primary-cancel",
        Behaviour::QuotaExhausted,
    ));
    let untouched = CountingMock::new("cfc-cancel-target", Behaviour::Ok);
    let untouched_calls = untouched.call_counter();
    let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(untouched)];

    let cancel = CancellationToken::new();
    cancel.cancel();

    AgentService::complete_compaction_request(
        &primary,
        &chain,
        request("m"),
        &cancel,
        ATTEMPT_DEADLINE,
    )
    .await
    .expect_err("cancelled");

    assert_eq!(untouched_calls.load(Ordering::SeqCst), 0);
}

/// The policy itself: hard quota and 402 must advance a chain even though they
/// are (correctly) not retryable in place.
#[test]
fn quota_and_billing_errors_advance_the_chain() {
    let monthly = ProviderError::RateLimitExceeded(
        "You have exceeded this month's quota for model X, please try again next month".to_string(),
    );
    assert!(
        monthly.is_quota_exhausted(),
        "sanity: this wording is a hard quota"
    );
    assert!(
        !monthly.is_retryable(),
        "sanity: hard quota is not retryable in place (#952)"
    );
    assert!(
        should_try_next_provider(&monthly),
        "#1247: but it MUST advance the chain"
    );

    let no_balance = ProviderError::RateLimitExceeded(
        "Insufficient balance or no resource package. Please recharge.".to_string(),
    );
    assert!(should_try_next_provider(&no_balance));

    let payment_required = ProviderError::ApiError {
        status: 402,
        message: "payment required".to_string(),
        error_type: None,
    };
    assert!(
        should_try_next_provider(&payment_required),
        "#1247: 402 billing caps are per-account, the next provider bills elsewhere"
    );
}

/// Transient and credential failures keep falling through; genuinely internal
/// errors do not.
#[test]
fn fall_through_policy_covers_transient_and_auth_but_not_internal() {
    assert!(should_try_next_provider(&ProviderError::RateLimitExceeded(
        "slow down".to_string()
    )));
    assert!(should_try_next_provider(&ProviderError::InvalidApiKey));
    assert!(should_try_next_provider(&ProviderError::ApiError {
        status: 401,
        message: "unauthorized".to_string(),
        error_type: None,
    }));
    assert!(should_try_next_provider(&ProviderError::ModelNotFound(
        "nope".to_string()
    )));
    assert!(!should_try_next_provider(&ProviderError::Internal(
        "bug".to_string()
    )));
}

/// A summariser that stops answering must not stop the session (#1255).
///
/// HTTP providers cap a single request at 300s. CLI providers carry no
/// timeout at all, so a wedged one held compaction for as long as it felt
/// like living and the chain below it was never reached, because the first
/// attempt never returned to fail.
mod watchdog {
    use super::*;

    /// Short enough to keep the test instant. The bound that ships is scaled
    /// from the session's own observed compactions; what is asserted here is
    /// that a bound exists and that blowing it hands the work on.
    const SHORT: std::time::Duration = std::time::Duration::from_millis(50);

    #[tokio::test]
    async fn a_wedged_primary_is_handed_to_the_chain() {
        let primary: Arc<dyn Provider> =
            Arc::new(CountingMock::new("cfc-wedged", Behaviour::Hangs));
        let healthy = CountingMock::new("cfc-rescue", Behaviour::Ok);
        let healthy_calls = healthy.call_counter();
        let chain: Vec<Arc<dyn Provider>> = vec![Arc::new(healthy)];

        let response = AgentService::complete_compaction_request(
            &primary,
            &chain,
            request("primary-model"),
            &CancellationToken::new(),
            SHORT,
        )
        .await
        .expect("the chain should have rescued a wedged primary");

        assert_eq!(
            healthy_calls.load(Ordering::SeqCst),
            1,
            "the fallback was never reached: the wedged primary was waited out"
        );
        assert!(matches!(
            &response.content[0],
            crate::brain::provider::ContentBlock::Text { text } if text.contains("cfc-rescue")
        ));
    }

    #[tokio::test]
    async fn a_wedged_fallback_does_not_end_the_walk() {
        let primary: Arc<dyn Provider> =
            Arc::new(CountingMock::new("cfc-dead", Behaviour::QuotaExhausted));
        let healthy = CountingMock::new("cfc-last", Behaviour::Ok);
        let healthy_calls = healthy.call_counter();
        let chain: Vec<Arc<dyn Provider>> = vec![
            Arc::new(CountingMock::new("cfc-wedged-fb", Behaviour::Hangs)),
            Arc::new(healthy),
        ];

        AgentService::complete_compaction_request(
            &primary,
            &chain,
            request("primary-model"),
            &CancellationToken::new(),
            SHORT,
        )
        .await
        .expect("a hang mid-chain must not strand the entries behind it");

        assert_eq!(healthy_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_hang_with_nowhere_to_go_still_returns() {
        let primary: Arc<dyn Provider> =
            Arc::new(CountingMock::new("cfc-alone-wedged", Behaviour::Hangs));

        let err = AgentService::complete_compaction_request(
            &primary,
            &[],
            request("primary-model"),
            &CancellationToken::new(),
            SHORT,
        )
        .await
        .expect_err("a wedged provider with no chain is a failure, not a wait");

        // The caller retries and eventually truncates with a marker. What it
        // must never do is block on this call forever.
        assert!(format!("{err}").to_lowercase().contains("time"), "{err}");
    }
}
