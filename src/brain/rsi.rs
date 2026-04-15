//! RSI (Recursive Self-Improvement) background engine.
//!
//! Runs as a background task after startup:
//! 1. Writes a digest of feedback_ledger stats to `~/.opencrabs/rsi/digest.md`
//! 2. Periodically analyzes feedback and applies improvements autonomously
//! 3. Emits TUI notifications when improvements are applied
//!
//! Uses the provider/model configured in `[agent].self_improvement_provider`
//! and `[agent].self_improvement_model`, falling back to the active provider.

use crate::config::Config;
use crate::db::repository::FeedbackLedgerRepository;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Interval between RSI cycles (analyze + improve).
const RSI_CYCLE_INTERVAL_SECS: u64 = 3600; // 1 hour

/// Minimum feedback entries before RSI attempts improvements.
const RSI_MIN_ENTRIES: i64 = 50;

/// Max tool iterations for the RSI agent (keep it focused).
const RSI_MAX_TOOL_ITERATIONS: usize = 10;

/// Ensure `~/.opencrabs/rsi/` and `~/.opencrabs/rsi/history/` exist.
fn ensure_rsi_dirs() -> std::io::Result<PathBuf> {
    let home = crate::config::opencrabs_home();
    let rsi_dir = home.join("rsi");
    let history_dir = rsi_dir.join("history");
    std::fs::create_dir_all(&history_dir)?;
    Ok(rsi_dir)
}

/// Write the startup digest to `~/.opencrabs/rsi/digest.md`.
/// Called once at boot after DB is ready.
pub async fn write_startup_digest(pool: crate::db::Pool) {
    let repo = FeedbackLedgerRepository::new(pool);
    let total = match repo.total_count().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("RSI digest: failed to query feedback_ledger: {e}");
            return;
        }
    };

    if total == 0 {
        tracing::debug!("RSI digest: no feedback data yet, skipping");
        return;
    }

    let rsi_dir = match ensure_rsi_dirs() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("RSI digest: failed to create rsi dir: {e}");
            return;
        }
    };

    let mut out = format!(
        "# RSI Digest\n\n**Generated:** {}\n**Total events:** {total}\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    );

    // Event type breakdown
    if let Ok(summary) = repo.summary().await {
        out.push_str("## Event Breakdown\n\n");
        for (event_type, count) in &summary {
            let pct = (*count as f64 / total as f64) * 100.0;
            out.push_str(&format!("- **{event_type}**: {count} ({pct:.1}%)\n"));
        }
        out.push('\n');
    }

    // Tool stats with failure rates
    if let Ok(stats) = repo.stats_by_dimension("tool_").await {
        let failing: Vec<_> = stats.iter().filter(|s| s.failures > 0).collect();
        if !failing.is_empty() {
            out.push_str("## Tool Performance\n\n");
            out.push_str("| Tool | Total | OK | Fail | Rate |\n");
            out.push_str("|------|------:|---:|-----:|-----:|\n");
            for s in &failing {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {:.0}% |\n",
                    s.dimension,
                    s.total_events,
                    s.successes,
                    s.failures,
                    s.success_rate * 100.0
                ));
            }
            out.push('\n');
        }
    }

    // Recent failures
    if let Ok(entries) = repo.by_event_type("tool_failure", 10).await
        && !entries.is_empty()
    {
        out.push_str("## Recent Failures\n\n");
        for e in &entries {
            let meta = e.metadata.as_deref().unwrap_or("(no details)");
            let short: String = meta.chars().take(120).collect();
            out.push_str(&format!(
                "- `{}` — {} — {}\n",
                e.created_at.format("%Y-%m-%d %H:%M"),
                e.dimension,
                short
            ));
        }
        out.push('\n');
    }

    // User corrections
    if let Ok(corrections) = repo.by_event_type("user_correction", 10).await
        && !corrections.is_empty()
    {
        out.push_str("## User Corrections\n\n");
        for c in &corrections {
            let meta = c.metadata.as_deref().unwrap_or("(no details)");
            let short: String = meta.chars().take(120).collect();
            out.push_str(&format!(
                "- `{}` — {} — {}\n",
                c.created_at.format("%Y-%m-%d %H:%M"),
                c.dimension,
                short
            ));
        }
        out.push('\n');
    }

    // Applied improvements
    if let Ok(improvements) = repo.by_event_type("improvement_applied", 10).await
        && !improvements.is_empty()
    {
        out.push_str("## Applied Improvements\n\n");
        for imp in &improvements {
            out.push_str(&format!(
                "- `{}` — {}\n",
                imp.created_at.format("%Y-%m-%d %H:%M"),
                imp.dimension
            ));
        }
        out.push('\n');
    }

    let digest_path = rsi_dir.join("digest.md");
    match std::fs::File::create(&digest_path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(out.as_bytes()) {
                tracing::warn!("RSI digest: failed to write: {e}");
            } else {
                tracing::info!(
                    "RSI digest written to {} ({total} events)",
                    digest_path.display()
                );
            }
        }
        Err(e) => tracing::warn!("RSI digest: failed to create file: {e}"),
    }
}

/// Notification message from the RSI engine to TUI/channels.
#[derive(Debug, Clone)]
pub enum RsiNotification {
    /// RSI cycle started
    CycleStarted,
    /// Digest written at startup
    DigestWritten { total_events: i64 },
    /// An improvement was identified and needs agent execution
    ImprovementOpportunity { description: String },
    /// Autonomous agent completed an improvement cycle
    AgentCycleComplete { summary: String },
    /// Autonomous agent failed
    AgentCycleFailed { error: String },
}

/// Build a minimal tool registry containing only the 3 RSI tools.
fn build_rsi_tool_registry() -> Arc<crate::brain::tools::ToolRegistry> {
    use crate::brain::tools::ToolRegistry;
    use crate::brain::tools::feedback_analyze::FeedbackAnalyzeTool;
    use crate::brain::tools::feedback_record::FeedbackRecordTool;
    use crate::brain::tools::self_improve::SelfImproveTool;

    let registry = ToolRegistry::new();
    registry.register(Arc::new(FeedbackRecordTool));
    registry.register(Arc::new(FeedbackAnalyzeTool));
    registry.register(Arc::new(SelfImproveTool));
    Arc::new(registry)
}

/// The system prompt for the RSI agent.
const RSI_AGENT_PROMPT: &str = "\
You are the RSI (Recursive Self-Improvement) engine for OpenCrabs. \
Your job is to analyze system feedback and autonomously apply improvements to brain files.

## Analysis Steps

1. Call feedback_analyze with query='summary' to see overall system stats.
2. Call feedback_analyze with query='tool_stats' to identify tools with high failure rates.
3. Call feedback_analyze with query='failures' to see recent failure details.
4. Call feedback_analyze with query='recent' to see the latest events (including self-heal triggers).
5. For each actionable problem, call self_improve to apply a targeted fix.
6. Be conservative: only apply improvements when you have clear evidence from the feedback data.
7. Focus on the highest-impact issues first (highest failure rate, most frequent corrections).

## Target File Taxonomy

Each brain file controls a different aspect of the agent. Route improvements to the RIGHT file:

- **SOUL.md** — How the model BEHAVES: response style, reasoning patterns, personality. \
  Fix here when: phantom_tool_call events (model narrates instead of acting), gaslighting \
  preambles, verbose/repetitive responses, wrong tone.
- **TOOLS.md** — How TOOLS are used: argument formats, common pitfalls, usage patterns. \
  Fix here when: tool_failure events show the same tool failing with similar args, or the \
  model consistently misuses a tool parameter.
- **USER.md** — How to interact with THIS USER: preferences, corrections, frustrations. \
  Fix here when: user_correction events show a repeated preference the agent keeps violating.
- **MEMORY.md** — Persistent KNOWLEDGE: facts, context, project state, integrations. \
  Fix here when: the agent repeatedly lacks context it should have retained across sessions.
- **AGENTS.md** — Agent configuration, workspace rules, safety policies. \
  Fix here when: agent-level behavior (approval flow, context handling) needs adjustment.
- **CODE.md** — Coding standards and patterns. \
  Fix here when: code-quality feedback recurs (wrong style, missing tests, bad patterns).
- **SECURITY.md** — Security policies. Fix here when: security-related feedback appears.

## Self-Heal Event Types

These events in the feedback ledger represent behaviors the self-heal layer had to correct at runtime. \
Your job is to write improvements that PREVENT these from recurring:

- **phantom_tool_call** — Model described file changes in prose but executed zero tool calls. \
  Self-heal injected a retry prompt. Write to SOUL.md: reinforce 'execute tools, don't narrate'.
- **user_correction** — User said 'no', 'wrong', 'try again', etc. \
  Analyze the correction content to determine if it's behavioral (SOUL), tool-usage (TOOLS), or preference (USER).
- **context_compaction** — Context exceeded budget, had to be compacted. \
  If frequent, check if the agent is loading too many brain files or being too verbose (SOUL).
- **provider_error** — Provider returned an error. Usually not actionable unless the agent is \
  sending bad requests (TOOLS) or using the wrong model.
- **tool_failure** — A specific tool failed. Check args and usage patterns (TOOLS).

## Workflow — MANDATORY

1. **Read first**: Before ANY modification, call self_improve with action='read' on the target file. \
   You MUST see the current content to judge whether your improvement is new, redundant, or refines something existing.
2. **Decide action**: After reading:
   - If the file has NO existing instruction covering your improvement → use action='apply' to append.
   - If the file ALREADY has an instruction that covers the same topic but needs refinement → use action='update' with the exact old_content copied from what you just read, and your improved content in 'content'.
   - If the file already says what you want to say (even in different words) → SKIP. Do not duplicate.
3. **Never rewrite the whole file**. The 'update' action replaces ONE specific section/paragraph. \
   The 'apply' action appends. Neither should be used to rewrite the entire file. \
   Brain files contain user-written content — you must preserve it and only add/refine specific instructions.

## Rules

Do NOT apply improvements if the data is insufficient or ambiguous. \
Quality over quantity — one well-reasoned improvement is better than many speculative ones. \
Never duplicate an existing instruction in a brain file — you have the 'read' action to check first. \
If an improvement was already applied (check self_improve action='list'), skip it. \
Use 'update' over 'apply' when an existing instruction needs rewording, not a new one added.";

/// Run a single autonomous RSI agent cycle.
///
/// Creates a lightweight AgentService with only RSI tools, sends the improvement
/// prompt, and returns the agent's summary of what it did.
async fn run_rsi_agent_cycle(
    pool: crate::db::Pool,
    config: &Config,
    opportunities: &[String],
) -> anyhow::Result<String> {
    use crate::brain::agent::AgentService;
    use crate::services::{ServiceContext, SessionService};

    // Resolve RSI provider: prefer self_improvement_provider, fall back to user's active provider
    let active_provider = config.providers.active_provider_and_model().0;
    let provider_name = config
        .agent
        .self_improvement_provider
        .as_deref()
        .unwrap_or(&active_provider);

    let provider =
        crate::brain::provider::factory::create_provider_by_name(config, provider_name).await?;

    let service_ctx = ServiceContext::new(pool);
    let tool_registry = build_rsi_tool_registry();
    let brain_path = crate::config::opencrabs_home();

    let agent = AgentService::new(provider, service_ctx.clone(), config)
        .await
        .with_tool_registry(tool_registry)
        .with_auto_approve_tools(true)
        .with_max_tool_iterations(RSI_MAX_TOOL_ITERATIONS)
        .with_system_brain(RSI_AGENT_PROMPT.to_string())
        .with_brain_path(brain_path);

    // Reuse a persistent RSI session — keeps context across cycles so the agent
    // knows what it already improved and doesn't repeat work.
    let session_service = SessionService::new(service_ctx);
    let session = match session_service
        .find_session_by_title("RSI autonomous cycle")
        .await?
    {
        Some(s) => s,
        None => {
            session_service
                .create_session_with_provider(
                    Some("RSI autonomous cycle".to_string()),
                    Some(provider_name.to_string()),
                    config.agent.self_improvement_model.clone(),
                )
                .await?
        }
    };

    // Build the user prompt with detected opportunities
    let mut prompt = "Run an autonomous self-improvement cycle.\n\n".to_string();
    if !opportunities.is_empty() {
        prompt.push_str("Detected opportunities:\n");
        for opp in opportunities {
            prompt.push_str(&format!("- {opp}\n"));
        }
        prompt.push('\n');
    }
    prompt.push_str(
        "Analyze the feedback data, identify the highest-impact issues, and apply improvements.",
    );

    let model = config.agent.self_improvement_model.clone();

    let response = agent
        .send_message_with_tools(session.id, prompt, model)
        .await?;

    tracing::info!(
        "RSI agent cycle complete: {} tokens used, ${:.4} cost",
        response.usage.input_tokens + response.usage.output_tokens,
        response.cost
    );

    Ok(response.content)
}

/// Spawn the background RSI engine.
///
/// - Writes startup digest immediately
/// - Every `RSI_CYCLE_INTERVAL_SECS`, checks if there are actionable patterns
/// - When opportunities are found, spawns an autonomous agent to apply improvements
/// - Emits notifications to TUI via the provided channel
pub fn spawn_rsi_engine(
    pool: crate::db::Pool,
    config: &Config,
    notification_tx: mpsc::UnboundedSender<RsiNotification>,
) {
    let pool_clone = pool.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        // Delay to let the app fully start
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        // 1. Write startup digest
        write_startup_digest(pool_clone.clone()).await;
        let repo = FeedbackLedgerRepository::new(pool_clone.clone());
        if let Ok(total) = repo.total_count().await {
            let _ = notification_tx.send(RsiNotification::DigestWritten {
                total_events: total,
            });
        }

        // 2. Periodic analysis + autonomous improvement cycle
        //
        // On startup, check how long ago the last cycle ran. If the app was
        // restarted before the interval elapsed (e.g. dev recompile every
        // ~20 min), only sleep the remaining time instead of a full hour.
        // Without this, frequent restarts prevent RSI from ever firing.
        let last_cycle_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".opencrabs/rsi/last_cycle");
        let initial_delay = if let Ok(meta) = std::fs::metadata(&last_cycle_path) {
            let elapsed = meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs())
                .unwrap_or(RSI_CYCLE_INTERVAL_SECS);
            if elapsed >= RSI_CYCLE_INTERVAL_SECS {
                // Overdue — run soon (30s grace for app to stabilize)
                30
            } else {
                RSI_CYCLE_INTERVAL_SECS - elapsed
            }
        } else {
            // First run ever — use full interval
            RSI_CYCLE_INTERVAL_SECS
        };
        tracing::info!(
            "RSI engine: first cycle in {}m{}s",
            initial_delay / 60,
            initial_delay % 60
        );

        let mut first_iteration = true;
        let mut last_seen_count: i64 = 0;
        loop {
            let delay = if first_iteration {
                first_iteration = false;
                initial_delay
            } else {
                RSI_CYCLE_INTERVAL_SECS
            };
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;

            let total = match repo.total_count().await {
                Ok(t) => t,
                Err(_) => continue,
            };

            if total < RSI_MIN_ENTRIES {
                tracing::debug!(
                    "RSI cycle: only {total} entries (need {RSI_MIN_ENTRIES}), skipping"
                );
                continue;
            }

            // Skip if no new feedback since last cycle — same data = same analysis
            if total == last_seen_count {
                tracing::debug!("RSI cycle: feedback count unchanged ({total}), skipping");
                // Still stamp the file so restart timer stays accurate
                let _ = std::fs::write(&last_cycle_path, "");
                continue;
            }
            last_seen_count = total;

            let _ = notification_tx.send(RsiNotification::CycleStarted);
            tracing::info!("RSI cycle: analyzing {total} feedback entries");

            // Refresh digest file
            write_startup_digest(repo.pool().clone()).await;

            // Collect actionable opportunities
            let mut opportunities = Vec::new();

            // Tools with >20% failure rate and >5 executions
            if let Ok(stats) = repo.stats_by_dimension("tool_").await {
                for s in stats
                    .iter()
                    .filter(|s| s.total_events >= 5 && s.success_rate < 0.8)
                {
                    // Pull recent failures for this tool to give agent context
                    let mut detail = format!(
                        "Tool '{}' has {:.0}% failure rate ({} failures out of {}). \
                         Consider adding error handling guidance to TOOLS.md.",
                        s.dimension,
                        (1.0 - s.success_rate) * 100.0,
                        s.failures,
                        s.total_events
                    );
                    if let Ok(recent) = repo.by_event_type("tool_failure", 10).await {
                        let relevant: Vec<_> = recent
                            .iter()
                            .filter(|e| e.dimension == s.dimension)
                            .take(3)
                            .collect();
                        if !relevant.is_empty() {
                            detail.push_str("\n  Recent failures:");
                            for e in relevant {
                                detail.push_str(&format!(
                                    "\n  - session={}, time={}, meta={}",
                                    &e.session_id[..8.min(e.session_id.len())],
                                    e.created_at.format("%Y-%m-%d %H:%M"),
                                    e.metadata.as_deref().unwrap_or("none")
                                ));
                            }
                        }
                    }
                    tracing::info!("RSI opportunity: {}", detail);
                    let _ = notification_tx.send(RsiNotification::ImprovementOpportunity {
                        description: detail.clone(),
                    });
                    opportunities.push(detail);
                }
            }

            // Repeated user corrections — include recent examples with session/model
            if let Ok(corrections) = repo.by_event_type("user_correction", 50).await
                && corrections.len() >= 3
            {
                let mut desc = format!(
                    "{} user corrections recorded. Review patterns and update brain files.",
                    corrections.len()
                );
                desc.push_str("\n  Recent corrections:");
                for e in corrections.iter().take(5) {
                    desc.push_str(&format!(
                        "\n  - session={}, model={}, time={}, text={}",
                        &e.session_id[..8.min(e.session_id.len())],
                        e.dimension,
                        e.created_at.format("%Y-%m-%d %H:%M"),
                        e.metadata.as_deref().unwrap_or("none")
                    ));
                }
                tracing::info!("RSI opportunity: {}", desc);
                let _ = notification_tx.send(RsiNotification::ImprovementOpportunity {
                    description: desc.clone(),
                });
                opportunities.push(desc);
            }

            // Provider errors — surface model/provider info so agent knows which
            // provider is failing and can adjust brain files accordingly
            if let Ok(errors) = repo.by_event_type("provider_error", 20).await
                && errors.len() >= 3
            {
                let mut desc = format!("{} provider errors recorded.", errors.len());
                desc.push_str("\n  Recent errors:");
                for e in errors.iter().take(5) {
                    desc.push_str(&format!(
                        "\n  - session={}, provider/model={}, time={}, detail={}",
                        &e.session_id[..8.min(e.session_id.len())],
                        e.dimension,
                        e.created_at.format("%Y-%m-%d %H:%M"),
                        e.metadata.as_deref().unwrap_or("none")
                    ));
                }
                tracing::info!("RSI opportunity: {}", desc);
                let _ = notification_tx.send(RsiNotification::ImprovementOpportunity {
                    description: desc.clone(),
                });
                opportunities.push(desc);
            }

            // 3. If opportunities detected, spawn autonomous agent
            if !opportunities.is_empty() {
                tracing::info!(
                    "RSI cycle: {} opportunities found, spawning autonomous agent",
                    opportunities.len()
                );

                match run_rsi_agent_cycle(repo.pool().clone(), &config_clone, &opportunities).await
                {
                    Ok(summary) => {
                        let short: String = summary.chars().take(200).collect();
                        tracing::info!("RSI agent completed: {short}");
                        let _ =
                            notification_tx.send(RsiNotification::AgentCycleComplete { summary });
                    }
                    Err(e) => {
                        tracing::warn!("RSI agent cycle failed: {e}");
                        let _ = notification_tx.send(RsiNotification::AgentCycleFailed {
                            error: e.to_string(),
                        });
                    }
                }
            }

            // Stamp last_cycle so restarts resume from here, not from scratch
            let _ = std::fs::write(&last_cycle_path, "");
        }
    });
}
