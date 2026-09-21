//! `decide_cached` — the L1 exact-decision-reuse ring (#1648, PR2).
//!
//! One tool, one question shape: the agent asks a decision-shaped question
//! with its structured input; the tool keys it (tier, policy_version,
//! normalizer_version, canonical input → sha256), and depending on the
//! tier's mode either reuses a cached decision (live) or always calls the
//! model while recording what WOULD have been reused (shadow, the default).
//!
//! The shadow path is the release-day evaluation instrument: it changes
//! nothing the agent experiences (behavior identity, the sentinel test
//! pins this) while persisting `decision_shadow` counters in
//! `decision_stats`. Per the kill rule (Adolfo 2026-09-21), numbers or
//! removal — so every mode decision stays in `[decisions]` config, never
//! hardcoded, and `mode = "off"` takes zero cache code paths.
//!
//! Failure direction, everywhere in this file: miss. A DB error, a bad
//! parse, a provider hiccup — the answer still comes from the model, and
//! the worst outcome is a cache that does nothing, never a cache that
//! lies. The write gate (`margin_floor`) is enforced by the repository,
//! not here, so no caller can skip it.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::provider::{ContentBlock, LLMRequest, Message, Provider};
use crate::config::{Config, DecisionMode, DecisionTierConfig};
use crate::db::Pool;
use crate::db::repository::decision_cache::{DecisionCacheRepository, DecisionPut};
use crate::db::repository::decision_stats::DecisionStatsRepository;
use crate::decisions::normalize::{NORMALIZER_VERSION, canonical_json, decision_key};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct DecideCachedTool {
    cache: DecisionCacheRepository,
    stats: DecisionStatsRepository,
    /// Test seams: production registers `None` for both, so config comes
    /// from the live mirror and the provider from the active chain — hot
    /// reload works without restarting anything. Tests inject a fixed
    /// config and a counting stub provider.
    config_override: Option<Arc<Config>>,
    provider_override: Option<Arc<dyn Provider>>,
}

impl DecideCachedTool {
    pub fn new(pool: Pool) -> Self {
        Self {
            cache: DecisionCacheRepository::new(pool.clone()),
            stats: DecisionStatsRepository::new(pool),
            config_override: None,
            provider_override: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_stub(pool: Pool, config: Arc<Config>, provider: Arc<dyn Provider>) -> Self {
        Self {
            cache: DecisionCacheRepository::new(pool.clone()),
            stats: DecisionStatsRepository::new(pool),
            config_override: Some(config),
            provider_override: Some(provider),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DecideInput {
    /// Configured tier name (`[decisions.tiers.<tier>]`).
    tier: String,
    /// The structured context the decision depends on. This — not `ask` —
    /// is what gets canonicalized into the cache key: equal objects under
    /// canonicalization mean the same decision.
    input: Value,
    /// The question to put to the model when the ring does not answer.
    ask: String,
}

/// The tiny contract every cached decision answers under. Kept out of the
/// key (it is part of the tier's `policy_version`: changing this string
/// changes what a decision means, so the operator bumps the version).
const ANSWER_CONTRACT: &str = "Answer with JSON only, no prose, matching: {\"decision\": <concise answer>, \"p\": <confidence 0..1 or null>, \"margin\": <distance to runner-up 0..1 or null>}.";

fn text_of(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Pull the `{...}` decision object out of a model reply; `None` when the
/// reply does not honor the contract. A miss, never a guess.
fn parse_decision(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

#[async_trait]
impl Tool for DecideCachedTool {
    fn name(&self) -> &str {
        "decide_cached"
    }

    fn description(&self) -> &str {
        "Ask a decision-shaped question through the L1 reuse ring (#1648). \
         `tier` names a configured [decisions.tiers.<name>] ring, `input` is the \
         structured context the decision depends on (this is what is keyed), \
         `ask` is the question to put to the model on a miss. Under mode=shadow \
         (the default) the model is ALWAYS asked — identical behavior to a plain \
         call — while would-hits are counted for the release-day evaluation. \
         Under mode=live a fresh cached decision may be returned without a model \
         call (the result then carries \"cached\":true). An unconfigured tier is a \
         named error, never an implicit default."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tier": {
                    "type": "string",
                    "description": "Configured tier name ([decisions.tiers.<tier>])."
                },
                "input": {
                    "description": "The decision context: any JSON value. Canonicalized (sorted keys, masked timestamps/UUIDs/IPs/paths) into the cache key.",
                    "type": "object"
                },
                "ask": {
                    "type": "string",
                    "description": "The decision question to put to the model when the ring does not answer."
                }
            },
            "required": ["tier", "input", "ask"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        // A shadow ring costs exactly what the agent would spend anyway
        // (one model call); live reuse only ever spends LESS. Approval
        // would add friction to the instrument the evaluation needs.
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let parsed: DecideInput = serde_json::from_value(input)?;
        let config = match &self.config_override {
            Some(c) => Arc::clone(c),
            None => Config::current(),
        };

        let Some(tier_cfg) = config.decisions.tiers.get(&parsed.tier) else {
            return Ok(ToolResult::error(format!(
                "unknown decision tier '{}': add [decisions.tiers.{}] with policy_version to \
                 enable it (#1648)",
                parsed.tier, parsed.tier
            )));
        };

        // Kill switch: mode=off touches no cache, no counters, no shadow
        // logs — a plain model call, the same path as before this tool
        // existed. Removal must stay this cheap.
        if tier_cfg.mode == DecisionMode::Off {
            let answer = self
                .ask_model(&config, context, &parsed.ask)
                .await
                .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;
            return Ok(ToolResult::success(
                json!({
                    "cached": false,
                    "mode": "off",
                    "tier": parsed.tier,
                    "text": answer,
                })
                .to_string(),
            ));
        }

        let key = decision_key(
            &parsed.tier,
            &tier_cfg.policy_version,
            NORMALIZER_VERSION,
            &canonical_json(&parsed.input),
        );
        let key8 = &key[..8.min(key.len())];

        let existing = match self.cache.get(&key).await {
            Ok(row) => row.filter(|row| fresh_enough(row, tier_cfg)),
            // A DB read error is a miss, not a failure of the decision.
            Err(e) => {
                tracing::warn!("decide_cached: get failed (treating as miss): {e:#}");
                None
            }
        };

        if tier_cfg.mode == DecisionMode::Live
            && let Some(row) = existing.clone()
        {
            if let Err(e) = self.cache.bump_hits(&key).await {
                tracing::warn!("decide_cached: hit bump failed (under-counts, harmless): {e:#}");
            }
            if let Err(e) = self.stats.bump(&parsed.tier, false, true).await {
                tracing::warn!("decide_cached: stats bump failed (under-counts): {e:#}");
            }
            return Ok(ToolResult::success(
                json!({
                    "cached": true,
                    "mode": "live",
                    "tier": parsed.tier,
                    "key8": key8,
                    "decision": serde_json::from_str::<Value>(&row.result_json)
                        .unwrap_or(Value::String(row.result_json.clone())),
                })
                .to_string(),
            ));
        }

        // Shadow (default) and live-miss share this path: the model is
        // always asked. Shadow additionally records would_hit before the
        // call so the counter cannot be poisoned by the call's outcome.
        let would_hit = existing.is_some();
        if tier_cfg.mode == DecisionMode::Shadow {
            tracing::info!(
                "decision_shadow would_hit={} tier={} key8={}",
                would_hit,
                parsed.tier,
                key8
            );
            if let Err(e) = self.stats.bump(&parsed.tier, would_hit, false).await {
                tracing::warn!("decide_cached: stats bump failed (under-counts): {e:#}");
            }
        } else if let Err(e) = self.stats.bump(&parsed.tier, false, false).await {
            tracing::warn!("decide_cached: stats bump failed (under-counts): {e:#}");
        }

        let answer = self
            .ask_model(&config, context, &parsed.ask)
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;
        let decision = match parse_decision(&answer) {
            Some(d) => d,
            None => {
                // Contract not honored: return the raw text, store nothing.
                // A ring that caches what it cannot parse is a ring that lies.
                return Ok(ToolResult::success(
                    json!({
                        "cached": false,
                        "mode": tier_name(tier_cfg.mode),
                        "tier": parsed.tier,
                        "not_cached_reason": "model reply did not honor the decision JSON contract",
                        "text": answer,
                    })
                    .to_string(),
                ));
            }
        };

        let put = DecisionPut {
            key: key.clone(),
            tier_id: parsed.tier.clone(),
            result_json: decision.to_string(),
            p: decision.get("p").and_then(Value::as_f64),
            margin: decision.get("margin").and_then(Value::as_f64),
            policy_version: tier_cfg.policy_version.clone(),
            normalizer_version: NORMALIZER_VERSION.to_string(),
        };
        let stored = match self.cache.put(put, tier_cfg.margin_floor).await {
            Ok(stored) => stored,
            Err(e) => {
                tracing::warn!("decide_cached: put failed (next ask re-derives): {e:#}");
                false
            }
        };

        Ok(ToolResult::success(
            json!({
                "cached": false,
                "mode": tier_name(tier_cfg.mode),
                "tier": parsed.tier,
                "key8": key8,
                "would_hit": would_hit,
                "stored": stored,
                "decision": decision,
            })
            .to_string(),
        ))
    }
}

impl DecideCachedTool {
    /// One-shot model call on the active provider chain (or the injected
    /// test stub). Deliberately temperature 0 + a small budget: this is a
    /// decision, not a conversation; the session's own model does the
    /// talking when the agent wants prose.
    async fn ask_model(
        &self,
        config: &Config,
        context: &ToolExecutionContext,
        ask: &str,
    ) -> anyhow::Result<String> {
        let provider: Arc<dyn Provider> = match &self.provider_override {
            Some(p) => Arc::clone(p),
            None => {
                use crate::brain::provider::factory;
                match context
                    .session_provider
                    .as_deref()
                    .filter(|name| factory::is_known_provider_name(config, name))
                {
                    Some(name) => factory::create_provider_by_name(config, name).await?,
                    None => factory::create_provider(config).await?,
                }
            }
        };
        let mut request = LLMRequest::new(
            provider.default_model().to_string(),
            vec![Message::user(format!("{ask}\n\n{ANSWER_CONTRACT}"))],
        );
        request.temperature = Some(0.0);
        request.max_tokens = Some(256);
        let response = provider.complete(request).await?;
        Ok(text_of(&response.content))
    }
}

fn tier_name(mode: DecisionMode) -> &'static str {
    match mode {
        DecisionMode::Shadow => "shadow",
        DecisionMode::Live => "live",
        DecisionMode::Off => "off",
    }
}

/// TTL gate on reads: `ttl_hours` unset means the row never expires here
/// (the PR3 sweeper bounds it); set means an older row is a miss.
fn fresh_enough(
    row: &crate::db::repository::decision_cache::DecisionCacheRow,
    tier: &DecisionTierConfig,
) -> bool {
    match tier.ttl_hours {
        Some(hours) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(i64::MAX);
            now.saturating_sub(row.created_at) <= hours.saturating_mul(3600)
        }
        None => true,
    }
}
