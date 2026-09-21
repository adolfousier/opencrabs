//! Tests for the `decide_cached` tool (#1648, PR2).
//!
//! The two contracts this suite exists to defend:
//! 1. Behavior identity: shadow mode ALWAYS calls the model. The
//!    sentinel test is the whole point of the kill-switch design — if
//!    shadow ever short-circuited, the release-day evaluation would be
//!    measuring a feature nobody opted into.
//! 2. Load-time strictness: a tier without policy_version, or with the
//!    unimplemented similarity flag, stops the config load by name.
//!
//! Plus the reuse round-trip under mode=live and the counters the
//! evaluation reads.

use crate::brain::provider::{
    ContentBlock, LLMRequest, LLMResponse, Provider, ProviderError, ProviderStream, StopReason,
    TokenUsage,
};
use crate::brain::tools::decide_cached::DecideCachedTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use crate::config::Config;
use crate::db::Database;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Provider stub: counts every `complete` and replays a canned JSON answer.
struct StubProvider {
    calls: AtomicUsize,
    answer: String,
}

impl StubProvider {
    fn new(answer: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            answer: answer.to_string(),
        }
    }
}

#[async_trait]
impl Provider for StubProvider {
    async fn complete(&self, _request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LLMResponse {
            id: "stub-1".to_string(),
            model: "stub-model".to_string(),
            content: vec![ContentBlock::Text {
                text: self.answer.clone(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 10,
                ..Default::default()
            },
            streaming_active_secs: None,
            tool_text_leak: false,
        })
    }

    async fn stream(&self, _request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        Err(ProviderError::ApiError {
            status: 501,
            message: "stub does not stream".to_string(),
            error_type: None,
        })
    }

    fn name(&self) -> &str {
        "stub"
    }

    fn default_model(&self) -> &str {
        "stub-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["stub-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(8192)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.0
    }
}

/// Minimal valid config: everything defaulted except the decisions tiers
/// under test. `toml::from_str::<Config>` with a section body is the
/// established pattern (config_dotted_caps_test).
fn config_with(tiers_toml: &str) -> Arc<Config> {
    let toml = format!(
        r#"
[decisions]
{tiers_toml}
"#
    );
    Arc::new(toml::from_str::<Config>(&toml).expect("test TOML must parse"))
}

fn shadow_config() -> Arc<Config> {
    config_with("[decisions.tiers.triage]\npolicy_version = \"p1\"\n")
}

async fn setup(
    config: Arc<Config>,
    answer: &str,
) -> (
    DecideCachedTool,
    Arc<StubProvider>,
    Database,
    ToolExecutionContext,
) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory database");
    db.run_migrations().await.expect("migrations");
    let stub = Arc::new(StubProvider::new(answer));
    let tool = DecideCachedTool::with_stub(db.pool().clone(), config, stub.clone());
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    (tool, stub, db, ctx)
}

fn ask_input(payload: serde_json::Value) -> serde_json::Value {
    json!({
        "tier": "triage",
        "input": payload,
        "ask": "Is this change a bugfix or a feature?",
    })
}

fn text_of(result: crate::brain::tools::ToolResult) -> String {
    result.error.unwrap_or(result.output)
}

/// Round-trip under mode=live: first call is a miss (model asked, stored),
/// second identical call is answered WITHOUT touching the model (#1648
/// acceptance: "cached=true only under mode=live").
#[tokio::test]
async fn live_first_miss_then_cached_hit() {
    let cfg = config_with("[decisions.tiers.triage]\npolicy_version = \"p1\"\nmode = \"live\"\n");
    let (tool, stub, _db, ctx) = setup(cfg, r#"{"decision":"bugfix","p":0.9,"margin":0.4}"#).await;

    let first = text_of(
        tool.execute(ask_input(json!({"file": "cron.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["cached"], json!(false));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);

    let second = text_of(
        tool.execute(ask_input(json!({"file": "cron.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let second: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_eq!(second["cached"], json!(true));
    assert_eq!(second["decision"]["decision"], json!("bugfix"));
    assert_eq!(
        stub.calls.load(Ordering::SeqCst),
        1,
        "live hit must not call the model"
    );
}

/// Sentinel (#1648 acceptance, behavior identity): shadow mode with a
/// row ALREADY cached still calls the model on every ask, and records
/// the would_hit. This is the test that must fail loudly if anyone
/// ever "optimizes" shadow into a real read.
#[tokio::test]
async fn shadow_always_calls_model_and_counts_would_hit() {
    let (tool, stub, db, ctx) = setup(
        shadow_config(),
        r#"{"decision":"bugfix","p":0.9,"margin":0.4}"#,
    )
    .await;

    let first = text_of(
        tool.execute(ask_input(json!({"file": "cron.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["cached"], json!(false));
    assert_eq!(first["would_hit"], json!(false));

    let second = text_of(
        tool.execute(ask_input(json!({"file": "cron.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let second: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_eq!(second["cached"], json!(false));
    assert_eq!(
        second["would_hit"],
        json!(true),
        "row exists from the first ask; shadow must see it"
    );
    assert_eq!(
        stub.calls.load(Ordering::SeqCst),
        2,
        "shadow must ask the model every time"
    );

    // Counters persisted: 2 calls, 1 would-hit.
    let stats = crate::db::DecisionStatsRepository::new(db.pool().clone())
        .get("triage")
        .await
        .unwrap()
        .expect("stats row persisted");
    assert_eq!(stats.calls, 2);
    assert_eq!(stats.would_hit, 1);
    assert_eq!(stats.live_hit, 0);
}

/// mode=off takes zero cache code paths: no row, no stats, just the model.
#[tokio::test]
async fn off_mode_touches_nothing() {
    let cfg = config_with("[decisions.tiers.triage]\npolicy_version = \"p1\"\nmode = \"off\"\n");
    let (tool, stub, db, ctx) = setup(cfg, r#"{"decision":"feature","p":0.7,"margin":0.3}"#).await;

    let out = text_of(
        tool.execute(ask_input(json!({"file": "ui.rs"})), &ctx)
            .await
            .unwrap(),
    );
    assert!(out.contains("\"mode\":\"off\""));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);

    let cache_rows: i64 = db
        .pool()
        .get()
        .await
        .unwrap()
        .interact(|conn| conn.query_row("SELECT COUNT(*) FROM decision_cache", [], |r| r.get(0)))
        .await
        .unwrap()
        .unwrap();
    let stats_rows: i64 = db
        .pool()
        .get()
        .await
        .unwrap()
        .interact(|conn| conn.query_row("SELECT COUNT(*) FROM decision_stats", [], |r| r.get(0)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cache_rows, 0, "off must not write the cache");
    assert_eq!(stats_rows, 0, "off must not write counters");
}

/// Unknown tier: named error, never an implicit default.
#[tokio::test]
async fn unknown_tier_is_a_named_error() {
    let (tool, stub, _db, ctx) = setup(shadow_config(), "{}").await;
    let out = tool
        .execute(json!({"tier": "nope", "input": {}, "ask": "x?"}), &ctx)
        .await
        .unwrap();
    let text = text_of(out);
    assert!(
        text.contains("unknown decision tier 'nope'"),
        "error must name the tier: {text}"
    );
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

/// Load validation (#1648 acceptance): missing policy_version and
/// similarity=true are named config-load errors.
#[test]
fn decisions_tiers_rejected_at_load_without_policy_version() {
    let cfg: Config = toml::from_str("[decisions.tiers.triage]\nmode = \"shadow\"\n")
        .expect("parses (policy_version defaults to empty)");
    let err = cfg
        .validate()
        .expect_err("empty policy_version must stop the load");
    assert!(
        err.to_string()
            .contains("decisions.tiers.triage: policy_version is required"),
        "named error expected, got: {err}"
    );
}

#[test]
fn decisions_similarity_true_is_a_named_load_error() {
    let cfg: Config =
        toml::from_str("[decisions.tiers.triage]\npolicy_version = \"p1\"\nsimilarity = true\n")
            .expect("parses");
    let err = cfg.validate().expect_err("similarity is not implemented");
    assert!(
        err.to_string()
            .contains("decisions.tiers.triage: similarity reuse is not implemented"),
        "named error expected, got: {err}"
    );
}

#[test]
fn decisions_empty_section_loads_clean() {
    let cfg: Config = toml::from_str("").expect("no section at all");
    assert!(cfg.validate().is_ok(), "empty config = today's behavior");
}

/// The margin-floor write gate survives the tool path: a borderline
/// decision is answered but never frozen.
#[tokio::test]
async fn sub_floor_margin_is_answered_but_not_stored() {
    let cfg =
        config_with("[decisions.tiers.triage]\npolicy_version = \"p1\"\nmargin_floor = 0.5\n");
    let (tool, stub, _db, ctx) = setup(cfg, r#"{"decision":"maybe","p":0.55,"margin":0.05}"#).await;

    let out = text_of(
        tool.execute(ask_input(json!({"file": "x.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let out: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(out["stored"], json!(false), "sub-floor margin must refuse");
    assert_eq!(out["decision"]["decision"], json!("maybe"));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);

    // Second ask: no row to would-hit, proving nothing was written.
    let again = text_of(
        tool.execute(ask_input(json!({"file": "x.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let again: serde_json::Value = serde_json::from_str(&again).unwrap();
    assert_eq!(again["would_hit"], json!(false));
}

/// A reply that ignores the JSON contract is returned raw and cached
/// never: the ring only stores what it can prove it can re-serve.
#[tokio::test]
async fn contract_violation_is_not_cached() {
    let (tool, _stub, _db, ctx) = setup(shadow_config(), "It's clearly a bugfix, trust me.").await;
    let out = text_of(
        tool.execute(ask_input(json!({"file": "y.rs"})), &ctx)
            .await
            .unwrap(),
    );
    let out: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(out["stored"], serde_json::Value::Null);
    assert!(
        out["not_cached_reason"].is_string(),
        "reason must be reported: {out}"
    );
    assert!(out["text"].as_str().unwrap().contains("trust me"));
}
