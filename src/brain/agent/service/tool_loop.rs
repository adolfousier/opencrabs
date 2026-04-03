use super::builder::AgentService;
use super::types::*;
use crate::brain::agent::context::AgentContext;
use crate::brain::agent::error::{AgentError, Result};
use crate::brain::provider::{ContentBlock, LLMRequest, LLMResponse, Message};
use crate::brain::tools::ToolExecutionContext;
use crate::services::{MessageService, SessionService};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl AgentService {
    /// Enforce the 65 % context budget rule.
    ///
    /// - ≥ 65 %: try LLM compaction (up to 3 retries on error).
    /// - After compaction (or if all retries fail): hard-truncate to 65 % if still over.
    /// - Context NEVER exceeds 65 % after this function returns.
    ///
    /// NOTE: 65 % (~130k of 200k) is chosen because MiniMax (and likely other
    /// models) degrade on function-calling quality well before hitting their
    /// theoretical context limit — tool calls stop around ~133k tokens.
    ///
    /// Returns the compaction summary if LLM compaction succeeded.
    async fn enforce_context_budget(
        &self,
        session_id: Uuid,
        context: &mut AgentContext,
        model_name: &str,
        progress_callback: &Option<ProgressCallback>,
    ) -> Option<String> {
        let tool_overhead = self.actual_tool_schema_tokens();
        let effective_max = context.max_tokens.saturating_sub(tool_overhead);
        let usage_pct = if effective_max > 0 {
            (context.token_count as f64 / effective_max as f64) * 100.0
        } else {
            100.0
        };

        tracing::debug!(
            "Context budget: {} msg tokens / {} effective max ({} tool-schema overhead) = {:.1}%",
            context.token_count,
            effective_max,
            tool_overhead,
            usage_pct,
        );

        if usage_pct <= 65.0 {
            return None;
        }

        tracing::warn!(
            "Context at {:.0}% (>65%) — triggering compaction",
            usage_pct
        );

        // Try LLM compaction first (preserves context via summary)
        let mut summary_result = None;
        const MAX_ATTEMPTS: u32 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            match self.compact_context(session_id, context, model_name).await {
                Ok(summary) => {
                    if let Some(cb) = progress_callback {
                        cb(
                            session_id,
                            ProgressEvent::CompactionSummary {
                                summary: summary.clone(),
                            },
                        );
                        cb(session_id, ProgressEvent::TokenCount(context.token_count));
                    }
                    summary_result = Some(summary);
                    break;
                }
                Err(e) => {
                    tracing::error!(
                        "LLM compaction failed (attempt {}/{}): {}",
                        attempt,
                        MAX_ATTEMPTS,
                        e
                    );
                }
            }
        }

        // If still over budget after compaction, re-compact with tighter budget.
        // Never hard-truncate — always use LLM compaction to preserve context.
        let target_tokens = (effective_max as f64 * 0.65) as usize;
        if context.token_count > target_tokens {
            tracing::warn!(
                "Still at {} tokens after compaction (target {}), re-compacting",
                context.token_count,
                target_tokens,
            );
            // Re-run compaction — the 55% keep_budget should get it under 65%
            // on the second pass since there's less context now.
            if let Ok(summary) = self.compact_context(session_id, context, model_name).await {
                if let Some(cb) = progress_callback {
                    cb(
                        session_id,
                        ProgressEvent::CompactionSummary {
                            summary: summary.clone(),
                        },
                    );
                    cb(session_id, ProgressEvent::TokenCount(context.token_count));
                }
                summary_result = Some(summary);
            }
        }

        summary_result
    }

    /// Core tool-execution loop — called by all public shims.
    /// `override_approval_callback` and `override_progress_callback` take
    /// precedence over the service-level callbacks (used by Telegram, etc.)
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_tool_loop(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
        cancel_token: Option<CancellationToken>,
        override_approval_callback: Option<ApprovalCallback>,
        override_progress_callback: Option<ProgressCallback>,
        channel: &str,
        channel_chat_id: Option<&str>,
    ) -> Result<AgentResponse> {
        // Track this request for restart recovery
        let pending_repo = crate::db::PendingRequestRepository::new(self.context.pool());
        let request_id = Uuid::new_v4();
        if let Err(e) = pending_repo
            .insert(
                request_id,
                session_id,
                &user_message,
                channel,
                channel_chat_id,
            )
            .await
        {
            tracing::warn!("Failed to track pending request: {}", e);
        }

        // Per-call effective callbacks (override wins over service-level).
        // Track whether an explicit per-call override was provided so we can honour
        // channel approval callbacks even when the factory set auto_approve_tools=true.
        let has_override_approval = override_approval_callback.is_some();
        let approval_callback: Option<ApprovalCallback> =
            override_approval_callback.or_else(|| self.approval_callback.clone());
        let has_progress_override = override_progress_callback.is_some();
        let progress_callback: Option<ProgressCallback> =
            override_progress_callback.or_else(|| self.progress_callback.clone());

        // Run the actual loop
        let result = self
            .run_tool_loop_inner(
                session_id,
                user_message,
                model,
                cancel_token,
                has_override_approval,
                approval_callback,
                has_progress_override,
                progress_callback,
            )
            .await;

        // Request finished — delete the tracking row. Only PROCESSING rows
        // survive (meaning the process crashed/restarted mid-request).
        if let Err(e) = pending_repo.delete(request_id).await {
            tracing::warn!("Failed to clean up pending request: {}", e);
        }

        result
    }

    /// Inner tool loop — separated so `run_tool_loop` can wrap with request tracking.
    #[allow(clippy::too_many_arguments)]
    async fn run_tool_loop_inner(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
        cancel_token: Option<CancellationToken>,
        has_override_approval: bool,
        approval_callback: Option<ApprovalCallback>,
        has_progress_override: bool,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<AgentResponse> {
        // Get or create session
        let session_service = SessionService::new(self.context.clone());
        let _session = session_service
            .get_session(session_id)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?
            .ok_or(AgentError::SessionNotFound(session_id))?;

        // Load conversation context with budget-aware message trimming
        let message_service = MessageService::new(self.context.clone());
        let all_db_messages = message_service
            .list_messages_for_session(session_id)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        let model_name = model.unwrap_or_else(|| {
            self.provider
                .read()
                .expect("provider lock poisoned")
                .default_model()
                .to_string()
        });
        let context_window = self.context_limit;

        // Load from last compaction point — find the last CONTEXT COMPACTION marker
        // and only load messages from there forward. No arbitrary trimming.
        let db_messages = Self::messages_from_last_compaction(all_db_messages);

        let mut context =
            AgentContext::from_db_messages(session_id, db_messages, context_window as usize);

        // Add system brain if available (count its tokens so context.token_count
        // reflects the full API input from the start — prevents gross undercount
        // that causes the TUI context counter to jump wildly on first calibration)
        if let Some(brain) = &self.default_system_brain {
            context.token_count += AgentContext::estimate_tokens(brain);
            context.system_brain = Some(brain.clone());
        }

        // Check for manual /compact before user_message is consumed
        let is_manual_compact = user_message.contains("[SYSTEM: Compact context now.");

        // Build user message — detect and attach images from paths/URLs
        let user_msg = Self::build_user_message(&user_message).await;
        context.add_message(user_msg);

        // Save user message to database (text only — images are ephemeral).
        // Skip DB persistence for internal system continuations (restart recovery)
        // — they go to context for the LLM but never appear in chat history.
        // Redact secrets so Bearer tokens, API keys etc. from cron prompts
        // never persist to DB or appear in TUI chat history.
        let is_system_continuation = user_message.starts_with("[System:");
        if !is_system_continuation {
            let safe_message = crate::utils::sanitize::redact_secrets(&user_message);
            let _user_db_msg = message_service
                .create_message(session_id, "user".to_string(), safe_message)
                .await
                .map_err(|e| AgentError::Database(e.to_string()))?;
        }

        // Create assistant message placeholder NOW for real-time persistence.
        // We'll append content as we go and update with final tokens at the end.
        let mut assistant_db_msg = message_service
            .create_message(session_id, "assistant".to_string(), String::new())
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Manual /compact: force compaction and return summary directly — no second LLM call.
        // The summary already contains next steps and follow-ups, so it IS the response.
        if is_manual_compact {
            match self
                .compact_context(session_id, &mut context, &model_name)
                .await
            {
                Ok(summary) => {
                    // Persist compaction marker to DB so restarts load from this point
                    let compaction_marker = format!(
                        "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                         Below is a structured summary of everything before this point.]\n\n{}",
                        summary
                    );
                    message_service
                        .create_message(session_id, "user".to_string(), compaction_marker)
                        .await
                        .map_err(|e| AgentError::Database(e.to_string()))?;

                    // Persist summary as the assistant response
                    message_service
                        .append_content(assistant_db_msg.id, &summary)
                        .await
                        .map_err(|e| AgentError::Database(e.to_string()))?;

                    if let Some(ref cb) = progress_callback {
                        cb(session_id, ProgressEvent::TokenCount(context.token_count));
                    }

                    return Ok(AgentResponse {
                        message_id: assistant_db_msg.id,
                        content: summary,
                        stop_reason: Some(crate::brain::provider::StopReason::EndTurn),
                        usage: crate::brain::provider::TokenUsage {
                            input_tokens: 0,
                            output_tokens: 0,
                            ..Default::default()
                        },
                        context_tokens: context.token_count as u32,
                        cost: 0.0,
                        model: model_name,
                    });
                }
                Err(e) => {
                    tracing::error!("Manual compaction failed: {}", e);
                    let error_msg = format!(
                        "Compaction failed: {}\n\nThis can happen if:\n\
                         - The session has too few messages to summarize\n\
                         - The AI provider returned an error\n\
                         - The database is locked or inaccessible\n\n\
                         Try again, or continue the conversation normally — \
                         auto-compaction will trigger at 80% context usage.",
                        e
                    );
                    message_service
                        .append_content(assistant_db_msg.id, &error_msg)
                        .await
                        .map_err(|e2| AgentError::Database(e2.to_string()))?;

                    return Ok(AgentResponse {
                        message_id: assistant_db_msg.id,
                        content: error_msg,
                        stop_reason: Some(crate::brain::provider::StopReason::EndTurn),
                        usage: crate::brain::provider::TokenUsage {
                            input_tokens: 0,
                            output_tokens: 0,
                            ..Default::default()
                        },
                        context_tokens: context.token_count as u32,
                        cost: 0.0,
                        model: model_name,
                    });
                }
            }
        }

        // Detect CLI provider once (doesn't change during the loop)
        let is_cli_provider = self
            .provider
            .read()
            .map(|p| p.cli_handles_tools())
            .unwrap_or(false);

        // CLI providers manage their own context window internally.
        // Keep our tiktoken estimate from DB messages as-is — it's a reasonable
        // approximation. Don't reset to 0, because CLI cache tokens (used for
        // calibration below) are cumulative across internal tool rounds, not
        // the actual context window size.

        // Auto-compact: triggers at >65% usage.
        // CLI providers manage their own context window — skip our compaction entirely.
        let compaction_result = if is_cli_provider {
            None
        } else {
            self.enforce_context_budget(session_id, &mut context, &model_name, &progress_callback)
                .await
        };

        if let Some(ref summary) = compaction_result {
            // Persist compaction marker to DB so restarts load from this point
            let compaction_marker = format!(
                "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                 Below is a structured summary of everything before this point.]\n\n{}",
                summary
            );
            if let Err(e) = message_service
                .create_message(session_id, "user".to_string(), compaction_marker)
                .await
            {
                tracing::error!("Failed to persist compaction marker to DB: {}", e);
            }

            let mut cont_text =
                "[SYSTEM: Context was auto-compacted. The summary above includes a snapshot \
                 of recent messages before compaction.\n\
                 POST-COMPACTION PROTOCOL:\n\
                 1. Read the compaction summary and the recent message snapshot to understand \
                 the current task, tools in use, and what you were doing.\n\
                 2. If you need specific brain context, selectively load ONLY the relevant \
                 brain file (e.g. TOOLS.md, SOUL.md, USER.md). NEVER use name=\"all\".\n\
                 3. Continue the task immediately. Do NOT repeat completed work. \
                 Do NOT ask the user for instructions — you have everything you need.]"
                    .to_string();
            if !self.auto_approve_tools {
                cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
            }
            context.add_message(Message::user(cont_text));
        }

        // Create tool execution context
        let mut tool_context = ToolExecutionContext::new(session_id)
            .with_auto_approve(self.auto_approve_tools)
            .with_working_directory(
                self.working_directory
                    .read()
                    .expect("working_directory lock poisoned")
                    .clone(),
            );
        tool_context.sudo_callback = self.sudo_callback.clone();
        tool_context.shared_working_directory = Some(Arc::clone(&self.working_directory));
        tool_context.service_context = Some(self.context.clone());

        // Tool execution loop
        let mut iteration = 0;
        let mut total_input_tokens = 0u32;
        let mut total_output_tokens = 0u32;
        let mut total_cache_creation = 0u32;
        let mut total_cache_read = 0u32;
        let mut final_response: Option<LLMResponse> = None;
        let mut accumulated_text = String::new(); // Collect text from all iterations (not just final)
        let mut recent_tool_calls: Vec<String> = Vec::new(); // Track tool calls to detect loops
        let mut stream_retry_count = 0u32; // Track consecutive stream drop retries
        const MAX_STREAM_RETRIES: u32 = 2; // Retry up to 2 times on dropped streams

        // Ordered content segments for CLI providers — tracks text and tool markers
        // in the exact order they stream, so DB persistence preserves interleaving.
        #[derive(Clone)]
        enum CliSegment {
            Text(String),
            Tool(serde_json::Value),
        }
        let cli_segments: std::sync::Arc<std::sync::Mutex<Vec<CliSegment>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // Wrap progress_callback for CLI providers to intercept IntermediateText
        // and ToolCompleted events, preserving their streaming order.
        let progress_callback: Option<ProgressCallback> = if is_cli_provider {
            if let Some(ref original_cb) = progress_callback {
                let orig = original_cb.clone();
                let segs = cli_segments.clone();
                Some(std::sync::Arc::new(
                    move |sid: Uuid, event: ProgressEvent| {
                        match event {
                            ProgressEvent::IntermediateText { ref text, .. }
                                if !text.is_empty() =>
                            {
                                if let Ok(mut acc) = segs.lock() {
                                    acc.push(CliSegment::Text(text.clone()));
                                }
                            }
                            ProgressEvent::ToolCompleted {
                                ref tool_name,
                                ref tool_input,
                                success,
                                ref summary,
                                ..
                            } => {
                                let desc = AgentService::format_tool_summary(
                                    &tool_name.to_lowercase(),
                                    tool_input,
                                );
                                let entry = if summary.is_empty() {
                                    serde_json::json!({"d": desc, "s": success})
                                } else {
                                    serde_json::json!({"d": desc, "s": success, "o": summary})
                                };
                                if let Ok(mut acc) = segs.lock() {
                                    acc.push(CliSegment::Tool(entry));
                                }
                            }
                            _ => {}
                        }
                        orig(sid, event);
                    },
                ))
            } else {
                None
            }
        } else {
            progress_callback
        };

        loop {
            // Safety: warn every 50 iterations but never hard-stop
            // Loop detection (below) is the real safety net
            if self.max_tool_iterations > 0 && iteration >= self.max_tool_iterations {
                tracing::warn!(
                    "Tool iteration {} exceeded configured max of {} — continuing (loop detection is active)",
                    iteration,
                    self.max_tool_iterations
                );
            }
            // Check for cancellation
            if let Some(ref token) = cancel_token
                && token.is_cancelled()
            {
                tracing::warn!(
                    "🛑 Tool loop cancelled at iteration {} (cancel_token fired). \
                     Accumulated text: {} chars, tool iterations so far: {}",
                    iteration,
                    accumulated_text.len(),
                    iteration,
                );
                break;
            }

            iteration += 1;

            // Emit thinking progress
            if let Some(ref cb) = progress_callback {
                cb(session_id, ProgressEvent::Thinking);
            }

            // Enforce 65% budget before every API call (skip for CLI — it manages its own context)
            if let Some(ref summary) = if is_cli_provider {
                None
            } else {
                self.enforce_context_budget(
                    session_id,
                    &mut context,
                    &model_name,
                    &progress_callback,
                )
                .await
            } {
                // Persist compaction marker to DB so restarts load from this point
                let compaction_marker = format!(
                    "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                     Below is a structured summary of everything before this point.]\n\n{}",
                    summary
                );
                if let Err(e) = message_service
                    .create_message(session_id, "user".to_string(), compaction_marker)
                    .await
                {
                    tracing::error!("Failed to persist mid-loop compaction marker to DB: {}", e);
                }

                let mut cont_text =
                    "[SYSTEM: Context was auto-compacted mid-loop. The summary above includes \
                     a snapshot of recent messages. Review it and continue the task immediately. \
                     Do NOT repeat completed work. Do NOT ask for instructions.]"
                        .to_string();
                if !self.auto_approve_tools {
                    cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
                }
                context.add_message(Message::user(cont_text));
            }

            // Build LLM request with tools if available
            let mut request = LLMRequest::new(model_name.clone(), context.messages.clone())
                .with_max_tokens(self.max_tokens);
            request.working_directory =
                Some(self.get_working_directory().to_string_lossy().to_string());
            request.session_id = Some(session_id);

            if let Some(system) = &context.system_brain {
                request = request.with_system(system.clone());
            }

            // Add tools if registry has any
            let tool_count = self.tool_registry.count();
            tracing::debug!("Tool registry contains {} tools", tool_count);
            if tool_count > 0 {
                let tool_defs = self.tool_registry.get_tool_definitions();
                tracing::debug!("Adding {} tool definitions to request", tool_defs.len());
                request = request.with_tools(tool_defs);
            } else {
                tracing::warn!("No tools registered in tool registry!");
            }

            // CLI providers: pass queue callback so stream_complete can check
            // for queued user messages at tool boundaries mid-stream.
            let queued_buf = tokio::sync::Mutex::new(None);

            // Send to provider via streaming — retry once after emergency compaction if prompt is too long
            let (mut response, reasoning_text) = match self
                .stream_complete(
                    session_id,
                    request,
                    cancel_token.as_ref(),
                    progress_callback.as_ref(),
                    if is_cli_provider {
                        self.message_queue_callback.as_ref()
                    } else {
                        None
                    },
                    if is_cli_provider {
                        Some(&queued_buf)
                    } else {
                        None
                    },
                )
                .await
            {
                Ok(resp) => resp,
                Err(ref e)
                    if e.to_string().contains("prompt is too long")
                        || e.to_string().contains("too many tokens")
                        || e.to_string().contains("Argument list too long")
                        || matches!(
                            e,
                            crate::brain::provider::ProviderError::ContextLengthExceeded(_)
                        ) =>
                {
                    tracing::warn!("Prompt too long for provider — emergency compaction");

                    // Pre-truncate to 170K so compact_context() can actually run.
                    // Without this, the compaction LLM call also fails with "too long".
                    const PRE_TRUNCATE_TARGET: usize = 170_000;
                    if context.token_count > PRE_TRUNCATE_TARGET {
                        tracing::warn!(
                            "Context too large for compaction ({} tokens) — pre-truncating to {}K",
                            context.token_count,
                            PRE_TRUNCATE_TARGET / 1000
                        );
                        context.hard_truncate_to(PRE_TRUNCATE_TARGET);
                        tracing::info!(
                            "Pre-truncated to {} messages ({} tokens) — now attempting compaction",
                            context.messages.len(),
                            context.token_count
                        );
                    }

                    match self
                        .compact_context(session_id, &mut context, &model_name)
                        .await
                    {
                        Ok(summary) => {
                            // Persist compaction marker to DB so restarts load from this point
                            let compaction_marker = format!(
                                "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                                 Below is a structured summary of everything before this point.]\n\n{}",
                                summary
                            );
                            if let Err(e) = message_service
                                .create_message(session_id, "user".to_string(), compaction_marker)
                                .await
                            {
                                tracing::error!(
                                    "Failed to persist emergency compaction marker to DB: {}",
                                    e
                                );
                            }

                            let mut cont_text =
                                "[SYSTEM: Emergency compaction — provider rejected the prompt as \
                                 too large. Context has been compacted. Acknowledge the compaction \
                                 briefly with a fun/cheeky remark, then resume the task from where \
                                 you left off. Do NOT repeat completed work.]"
                                    .to_string();
                            if !self.auto_approve_tools {
                                cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
                            }
                            context.add_message(Message::user(cont_text));

                            // Notify user about emergency compaction
                            if let Some(ref cb) = progress_callback {
                                cb(
                                    session_id,
                                    ProgressEvent::SelfHealingAlert {
                                        message: "Emergency compaction: context was too large for the provider. Conversation has been compacted automatically.".to_string(),
                                    },
                                );
                            }
                        }
                        Err(compact_err) => {
                            tracing::error!(
                                "Emergency compaction also failed: {} — falling back to hard truncation",
                                compact_err
                            );

                            // Hard truncate: keep last 12 message pairs (24 messages).
                            // Full conversation is in the DB — agent can search_session for older context.
                            const KEEP_MESSAGES: usize = 24;
                            let total = context.messages.len();
                            if total > KEEP_MESSAGES {
                                let dropped = total - KEEP_MESSAGES;
                                context.messages = context.messages.split_off(dropped);
                                tracing::warn!(
                                    "Hard truncated context: dropped {} messages, kept {}",
                                    dropped,
                                    context.messages.len()
                                );
                            }

                            // Insert truncation marker so the agent knows context was lost
                            let truncation_marker = format!(
                                "[CONTEXT TRUNCATION — The conversation was too large for the provider \
                                 and compaction failed. The {} oldest messages were dropped. \
                                 The full conversation history is still in the database — use the \
                                 search_session tool if you need to recall earlier context. \
                                 Continue from where you left off.]",
                                total.saturating_sub(KEEP_MESSAGES)
                            );
                            context
                                .messages
                                .insert(0, Message::user(truncation_marker.clone()));

                            // Persist truncation marker to DB
                            if let Err(e) = message_service
                                .create_message(session_id, "user".to_string(), truncation_marker)
                                .await
                            {
                                tracing::error!("Failed to persist truncation marker: {}", e);
                            }

                            // Notify user about hard truncation
                            if let Some(ref cb) = progress_callback {
                                cb(
                                    session_id,
                                    ProgressEvent::SelfHealingAlert {
                                        message: format!(
                                            "Hard truncation: compaction failed, {} oldest messages were dropped. Full history is still in the database.",
                                            total.saturating_sub(KEEP_MESSAGES)
                                        ),
                                    },
                                );
                            }

                            // Re-estimate token count after truncation
                            context.token_count = context
                                .messages
                                .iter()
                                .map(|m| {
                                    m.content
                                        .iter()
                                        .map(|b| match b {
                                            ContentBlock::Text { text } => {
                                                crate::brain::tokenizer::count_tokens(text)
                                            }
                                            ContentBlock::ToolUse { input, .. } => {
                                                crate::brain::tokenizer::count_tokens(
                                                    &input.to_string(),
                                                )
                                            }
                                            ContentBlock::ToolResult { content, .. } => {
                                                crate::brain::tokenizer::count_tokens(content)
                                            }
                                            ContentBlock::Thinking { thinking, .. } => {
                                                crate::brain::tokenizer::count_tokens(thinking)
                                            }
                                            ContentBlock::Image { .. } => 1000,
                                        })
                                        .sum::<usize>()
                                })
                                .sum();
                        }
                    }

                    // Rebuild request with compacted context
                    let mut retry_req =
                        LLMRequest::new(model_name.clone(), context.messages.clone())
                            .with_max_tokens(self.max_tokens);
                    retry_req.working_directory =
                        Some(self.get_working_directory().to_string_lossy().to_string());
                    retry_req.session_id = Some(session_id);
                    if let Some(system) = &context.system_brain {
                        retry_req = retry_req.with_system(system.clone());
                    }
                    if self.tool_registry.count() > 0 {
                        retry_req = retry_req.with_tools(self.tool_registry.get_tool_definitions());
                    }
                    self.stream_complete(
                        session_id,
                        retry_req,
                        cancel_token.as_ref(),
                        progress_callback.as_ref(),
                        if is_cli_provider {
                            self.message_queue_callback.as_ref()
                        } else {
                            None
                        },
                        if is_cli_provider {
                            Some(&queued_buf)
                        } else {
                            None
                        },
                    )
                    .await
                    .map_err(AgentError::Provider)?
                }
                Err(e) => return Err(AgentError::Provider(e)),
            };

            // CLI providers return "Prompt is too long" as a successful response
            // with is_error=true in the content — detect and re-route to the
            // same emergency compaction path used for Err cases above.
            let is_cli_too_long = is_cli_provider
                && response.content.iter().any(|b| {
                    if let ContentBlock::Text { text } = b {
                        text.trim().starts_with("Prompt is too long")
                            || text.contains("prompt is too long")
                    } else {
                        false
                    }
                });

            if is_cli_too_long {
                tracing::warn!(
                    "CLI returned 'Prompt is too long' as content — triggering emergency compaction"
                );
                const PRE_TRUNCATE_TARGET: usize = 170_000;
                if context.token_count > PRE_TRUNCATE_TARGET {
                    context.hard_truncate_to(PRE_TRUNCATE_TARGET);
                }
                match self
                    .compact_context(session_id, &mut context, &model_name)
                    .await
                {
                    Ok(summary) => {
                        let compaction_marker = format!(
                            "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                             Below is a structured summary of everything before this point.]\n\n{}",
                            summary
                        );
                        let _ = message_service
                            .create_message(session_id, "user".to_string(), compaction_marker)
                            .await;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Emergency compaction also failed: {} — hard truncating",
                            e
                        );
                        const KEEP_MESSAGES: usize = 24;
                        let total = context.messages.len();
                        if total > KEEP_MESSAGES {
                            context.messages.drain(..total - KEEP_MESSAGES);
                        }
                    }
                }
                // Re-run the loop iteration with the compacted context
                continue;
            }

            // Track token usage — fall back to tiktoken estimate when provider
            // doesn't report usage (e.g. MiniMax streaming ignores include_usage)
            let call_input_tokens = if response.usage.input_tokens > 0 {
                response.usage.input_tokens
            } else {
                // Serialize actual tool definitions to count their real token cost,
                // matching how the provider computes it before each request.
                let tool_defs = self.tool_registry.get_tool_definitions();
                let tool_tokens = crate::brain::tokenizer::count_tokens(
                    &serde_json::to_string(&tool_defs).unwrap_or_default(),
                ) as u32;
                let estimate = context.token_count as u32 + tool_tokens;
                tracing::debug!(
                    "Provider reported 0 input tokens, using tiktoken estimate: {} ({} msg + {} tool schemas)",
                    estimate,
                    context.token_count,
                    tool_tokens
                );
                estimate
            };
            total_input_tokens += call_input_tokens;
            total_output_tokens += response.usage.output_tokens;
            // Use billing fields (cumulative across CLI rounds) when available
            total_cache_creation += if response.usage.billing_cache_creation > 0 {
                response.usage.billing_cache_creation
            } else {
                response.usage.cache_creation_tokens
            };
            total_cache_read += if response.usage.billing_cache_read > 0 {
                response.usage.billing_cache_read
            } else {
                response.usage.cache_read_tokens
            };

            // Calibrate context token count from the provider's reported usage.
            //
            // CLI providers: use context_input() which gives the LAST round's
            // per-call cache tokens (actual context window), NOT cumulative billing.
            if is_cli_provider {
                let cli_context = response.usage.context_input() as usize;
                if cli_context > 0 {
                    tracing::info!(
                        "CLI context calibration: {} → {} (from per-call cache tokens)",
                        context.token_count,
                        cli_context,
                    );
                    context.token_count = cli_context;
                }
            } else {
                let api_input = response.usage.input_tokens as usize;
                let tool_overhead = self.actual_tool_schema_tokens();
                let real_message_tokens = api_input.saturating_sub(tool_overhead);
                let min_sane = 100;
                let max_drop_ratio = 0.2;
                let min_after_drop = (context.token_count as f64 * max_drop_ratio) as usize;
                if real_message_tokens >= min_sane && real_message_tokens >= min_after_drop {
                    let drift = (context.token_count as f64 - real_message_tokens as f64).abs();
                    if drift > 5000.0 {
                        tracing::info!(
                            "Token calibration: estimated {} → API actual {} (drift: {:.0})",
                            context.token_count,
                            real_message_tokens,
                            drift,
                        );
                        context.token_count = real_message_tokens;
                    }
                } else if real_message_tokens > 0 && real_message_tokens < min_sane {
                    tracing::warn!(
                        "Token calibration skipped: api_input={}, tool_overhead={}, result={} (below sanity threshold)",
                        api_input,
                        tool_overhead,
                        real_message_tokens,
                    );
                }
            }
            // Fire real-time token count update after every API response
            if let Some(ref cb) = progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
            }
            // When a channel override is active, also fire to the service-level callback
            // so the TUI ctx display stays in sync with channel interactions.
            if has_progress_override && let Some(ref cb) = self.progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
            }

            // Post-calibration compaction check (API providers only).
            // CLI providers handle their own context — no compaction needed from us.
            if let Some(ref summary) = if is_cli_provider {
                None
            } else {
                self.enforce_context_budget(
                    session_id,
                    &mut context,
                    &model_name,
                    &progress_callback,
                )
                .await
            } {
                let compaction_marker = format!(
                    "[CONTEXT COMPACTION — The conversation was automatically compacted \
                     after token calibration revealed high context usage.]\n\n{}",
                    summary
                );
                if let Err(e) = message_service
                    .create_message(session_id, "user".to_string(), compaction_marker)
                    .await
                {
                    tracing::error!(
                        "Failed to persist post-calibration compaction marker: {}",
                        e
                    );
                }
                context.add_message(Message::user(
                    "[SYSTEM: Context was auto-compacted after calibration. \
                     Review the summary above and continue immediately.]"
                        .to_string(),
                ));
            }

            // --- CANCEL CHECK BEFORE STREAM DROP RETRY ---
            // If the user cancelled during streaming, don't retry — save partial text and break.
            if response.stop_reason.is_none()
                && let Some(ref token) = cancel_token
                && token.is_cancelled()
            {
                if is_cli_provider {
                    // CLI providers: persist interleaved text + tool markers from
                    // streaming events. These were accumulated by the wrapped callback.
                    let mut cancel_content = String::new();

                    // Reasoning
                    if let Some(ref reasoning) = reasoning_text
                        && !reasoning.trim().is_empty()
                    {
                        cancel_content.push_str(&format!(
                            "<!-- reasoning -->\n{}\n<!-- /reasoning -->\n\n",
                            reasoning
                        ));
                    }

                    // Build interleaved content from ordered segments
                    let segments: Vec<CliSegment> = cli_segments
                        .lock()
                        .map(|mut s| s.drain(..).collect())
                        .unwrap_or_default();
                    let mut pending_tools: Vec<serde_json::Value> = Vec::new();
                    for seg in segments {
                        match seg {
                            CliSegment::Text(text) => {
                                // Flush pending tools before text
                                if !pending_tools.is_empty() {
                                    let marker = format!(
                                        "\n<!-- tools-v2: {} -->\n",
                                        serde_json::to_string(&pending_tools).unwrap_or_default()
                                    );
                                    cancel_content.push_str(&marker);
                                    accumulated_text.push_str(&marker);
                                    pending_tools.clear();
                                }
                                cancel_content.push_str(&format!("{}\n\n", text));
                                if !accumulated_text.is_empty() {
                                    accumulated_text.push_str("\n\n");
                                }
                                accumulated_text.push_str(&text);
                            }
                            CliSegment::Tool(entry) => {
                                pending_tools.push(entry);
                            }
                        }
                    }
                    // Flush trailing tools
                    if !pending_tools.is_empty() {
                        let marker = format!(
                            "\n<!-- tools-v2: {} -->\n",
                            serde_json::to_string(&pending_tools).unwrap_or_default()
                        );
                        cancel_content.push_str(&marker);
                        accumulated_text.push_str(&marker);
                    }

                    // Also extract any text from the partial response not yet
                    // emitted as IntermediateText (trailing text after last tool)
                    for block in &response.content {
                        if let ContentBlock::Text { text } = block
                            && !text.trim().is_empty()
                        {
                            // Only append if not already covered by segments
                            if cancel_content.is_empty() || !cancel_content.contains(text.trim()) {
                                cancel_content.push_str(&format!("{}\n\n", text));
                                if !accumulated_text.is_empty() {
                                    accumulated_text.push_str("\n\n");
                                }
                                accumulated_text.push_str(text);
                            }
                        }
                    }

                    // Single atomic write
                    if !cancel_content.is_empty() {
                        let _ = message_service
                            .append_content(assistant_db_msg.id, &cancel_content)
                            .await;
                    }
                } else {
                    // Non-CLI: persist partial text from response blocks
                    for block in &response.content {
                        if let ContentBlock::Text { text } = block
                            && !text.trim().is_empty()
                        {
                            if !accumulated_text.is_empty() {
                                accumulated_text.push_str("\n\n");
                            }
                            accumulated_text.push_str(text);
                            let _ = message_service
                                .append_content(assistant_db_msg.id, &format!("{}\n\n", text))
                                .await;
                        }
                    }
                }
                tracing::info!(
                    "Stream cancelled by user — saving partial text ({} chars)",
                    accumulated_text.len()
                );
                break;
            }

            // --- STREAM DROP DETECTION ---
            // If stop_reason is None, the stream ended without [DONE]/MessageStop.
            // This means a network interruption, provider timeout, or dropped connection.
            // The response may contain partial/corrupt data. Retry instead of proceeding
            // with garbage that silently drops the task.
            if response.stop_reason.is_none() {
                if stream_retry_count < MAX_STREAM_RETRIES {
                    stream_retry_count += 1;
                    tracing::warn!(
                        "🔄 Stream dropped without completion (no stop_reason) at iteration {}. \
                         Retrying ({}/{}) — partial content discarded.",
                        iteration,
                        stream_retry_count,
                        MAX_STREAM_RETRIES,
                    );
                    // Subtract the tokens we just counted — they'll be re-counted on retry
                    total_input_tokens -= response.usage.input_tokens;
                    total_output_tokens -= response.usage.output_tokens;
                    total_cache_creation =
                        total_cache_creation.saturating_sub(response.usage.cache_creation_tokens);
                    total_cache_read =
                        total_cache_read.saturating_sub(response.usage.cache_read_tokens);
                    // Don't increment iteration — this is a retry, not a new turn
                    iteration -= 1;
                    continue;
                } else {
                    let drop_msg = format!(
                        "Provider stream dropped {} times consecutively. \
                         The request could not be completed. \
                         Check logs at ~/.opencrabs/logs/ for details.",
                        MAX_STREAM_RETRIES,
                    );
                    tracing::error!(
                        "🚨 {} Content blocks: {}, stop_reason: None",
                        drop_msg,
                        response.content.len(),
                    );
                    // Inject error as visible text so user sees it in the TUI
                    if response.content.iter().all(
                        |b| !matches!(b, ContentBlock::Text { text } if !text.trim().is_empty()),
                    ) {
                        response.content.push(ContentBlock::Text {
                            text: format!("⚠️ {}", drop_msg),
                        });
                    }
                    // Reset retry counter — we're accepting the partial response
                    stream_retry_count = 0;
                }
            } else {
                // Successful stream completion — reset retry counter
                stream_retry_count = 0;
            }

            // Separate text blocks and tool use blocks from the response
            tracing::debug!("Response has {} content blocks", response.content.len());
            let mut iteration_text = String::new();
            let mut tool_uses: Vec<(String, String, Value)> = Vec::new();

            for (i, block) in response.content.iter().enumerate() {
                match block {
                    ContentBlock::Text { text } => {
                        tracing::debug!(
                            "Block {}: Text ({}...)",
                            i,
                            &text.chars().take(50).collect::<String>()
                        );
                        if !text.trim().is_empty() {
                            if !iteration_text.is_empty() {
                                iteration_text.push_str("\n\n");
                            }
                            iteration_text.push_str(text);
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        // GRANULAR LOG: Tool call received from provider
                        let input_keys: Vec<_> = input
                            .as_object()
                            .map(|o| o.keys().cloned().collect())
                            .unwrap_or_default();
                        tracing::info!(
                            "[TOOL_EXEC] 📥 Tool call received: name={}, id={}, input_keys={:?}",
                            name,
                            id,
                            input_keys
                        );

                        // Check for empty/Invalid input
                        if input.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                            tracing::error!(
                                "[TOOL_EXEC] ⚠️ Tool '{}' received empty input — tool call will fail",
                                name
                            );
                        }

                        // Normalize hallucinated tool names: some providers send
                        // "Plan: complete_task" instead of tool="plan" + operation="complete_task".
                        let (norm_name, norm_input) =
                            Self::normalize_tool_call(name.clone(), input.clone());

                        tool_uses.push((id.clone(), norm_name, norm_input));
                    }
                    _ => {
                        tracing::debug!("Block {}: Other content block", i);
                    }
                }
            }

            // ── Strip echoed markup ──────────────────────────────────────
            // The LLM echoes or invents HTML comment markers from context:
            // <!-- tools-v2: ... -->, <!-- lens -->, <!-- /tools-v2>, etc.
            // Strip ALL HTML comments from iteration text to prevent any
            // from leaking into Telegram/channel output or the TUI.
            if iteration_text.contains("<!--") {
                iteration_text = Self::strip_html_comments(&iteration_text);
            }

            // ── XML tool-call recovery ──────────────────────────────────
            // MiniMax (and some other providers) sometimes emit tool calls as
            // XML in the content instead of using the API's tool_calls field.
            // Parse them into real tool_uses AND inject into response.content
            // so the context has matching ToolUse blocks for ToolResult messages.
            //
            // CRITICAL: Only strip XML blocks that were SUCCESSFULLY parsed as
            // valid tool calls. If the model is just talking ABOUT XML tags in
            // prose (e.g. release notes), parsing finds no valid JSON inside
            // the tags and we leave the text untouched.
            if Self::has_xml_tool_block(&iteration_text) {
                let parsed = Self::parse_xml_tool_calls(&iteration_text);
                if !parsed.is_empty() {
                    tracing::info!(
                        "Recovered {} XML tool call(s) from content text",
                        parsed.len()
                    );
                    for (name, input) in parsed {
                        let synthetic_id = format!("xml-{}", uuid::Uuid::new_v4().simple());
                        tool_uses.push((synthetic_id.clone(), name.clone(), input.clone()));
                        response.content.push(ContentBlock::ToolUse {
                            id: synthetic_id,
                            name,
                            input,
                        });
                    }
                    // Only strip after successful parse — prose mentions are left alone
                    iteration_text = Self::strip_xml_tool_calls(&iteration_text);
                }
            }

            // ── DB persistence ──────────────────────────────────────────
            // CLI providers: build interleaved content from ordered segments
            // (text + tool markers in streaming order) for a single atomic write.
            // This preserves the text→tools→text sequence seen during live streaming
            // and survives Esc×2 cancel + restart.
            if is_cli_provider {
                let mut cli_content = String::new();

                // 1. Reasoning
                if let Some(ref reasoning) = reasoning_text
                    && !reasoning.trim().is_empty()
                {
                    cli_content.push_str(&format!(
                        "<!-- reasoning -->\n{}\n<!-- /reasoning -->\n\n",
                        reasoning
                    ));
                }

                // 2. Interleaved text + tool markers from streaming events
                let segments: Vec<CliSegment> = cli_segments
                    .lock()
                    .map(|mut s| s.drain(..).collect())
                    .unwrap_or_default();
                let mut pending_tools: Vec<serde_json::Value> = Vec::new();
                for seg in segments {
                    match seg {
                        CliSegment::Text(text) => {
                            // Flush pending tools before text
                            if !pending_tools.is_empty() {
                                let marker = format!(
                                    "\n<!-- tools-v2: {} -->\n",
                                    serde_json::to_string(&pending_tools).unwrap_or_default()
                                );
                                cli_content.push_str(&marker);
                                accumulated_text.push_str(&marker);
                                pending_tools.clear();
                            }
                            cli_content.push_str(&format!("{}\n\n", text));
                            if !accumulated_text.is_empty() {
                                accumulated_text.push_str("\n\n");
                            }
                            accumulated_text.push_str(&text);
                        }
                        CliSegment::Tool(entry) => {
                            pending_tools.push(entry);
                        }
                    }
                }
                // Flush trailing tools
                if !pending_tools.is_empty() {
                    let marker = format!(
                        "\n<!-- tools-v2: {} -->\n",
                        serde_json::to_string(&pending_tools).unwrap_or_default()
                    );
                    cli_content.push_str(&marker);
                    accumulated_text.push_str(&marker);
                }

                // Single atomic write — no partial state visible to load_session
                if !cli_content.is_empty() {
                    let _ = message_service
                        .append_content(assistant_db_msg.id, &cli_content)
                        .await;
                }
            } else {
                // Non-CLI: separate writes (tool execution happens between iterations)
                if let Some(ref reasoning) = reasoning_text
                    && !reasoning.trim().is_empty()
                {
                    let _ = message_service
                        .append_content(
                            assistant_db_msg.id,
                            &format!("<!-- reasoning -->\n{}\n<!-- /reasoning -->\n\n", reasoning),
                        )
                        .await;
                }

                if !iteration_text.is_empty() {
                    if !accumulated_text.is_empty() {
                        accumulated_text.push_str("\n\n");
                    }
                    accumulated_text.push_str(&iteration_text);

                    let _ = message_service
                        .append_content(assistant_db_msg.id, &format!("{}\n\n", iteration_text))
                        .await;
                }
            }

            tracing::debug!("Found {} tool uses to execute", tool_uses.len());

            // CLI providers handle tools internally — emit progress events for
            // TUI display (expandable tool groups) but don't execute them.
            // Break immediately after — the CLI already completed its full run.
            if is_cli_provider && !tool_uses.is_empty() {
                // Text/tool interleaving and ToolStarted/ToolCompleted events
                // are already emitted during streaming by helpers.rs
                // (cli_unflushed_text flushes at tool boundaries + stream end).
                // Tool markers already persisted atomically above via cli_segments.
                //
                // Do NOT re-emit IntermediateText here — helpers.rs already sent
                // all text blocks during streaming. Emitting again causes the
                // entire conversation text to appear duplicated in the TUI.
                iteration_text.clear();
                tool_uses.clear();
            }

            if tool_uses.is_empty() {
                // Check queued messages — stream_complete may have consumed
                // one mid-stream (stored in queued_buf), or check the queue now.
                let (queued_msg, from_buf) = {
                    let buffered = queued_buf.lock().await.take();
                    if buffered.is_some() {
                        (buffered, true)
                    } else if let Some(ref queue_cb) = self.message_queue_callback {
                        (queue_cb().await, false)
                    } else {
                        (None, false)
                    }
                };
                if let Some(queued_msg) = queued_msg {
                    tracing::info!("Injecting queued user message (from_buf={})", from_buf);
                    // Emit assistant's intermediate text FIRST so it appears
                    // before the queued user message in the TUI
                    if !iteration_text.is_empty()
                        && let Some(ref cb) = progress_callback
                    {
                        cb(
                            session_id,
                            ProgressEvent::IntermediateText {
                                text: iteration_text,
                                reasoning: reasoning_text,
                            },
                        );
                    }
                    // Emit QueuedUserMessage — always here, never in stream_complete
                    if let Some(ref cb) = progress_callback {
                        cb(
                            session_id,
                            ProgressEvent::QueuedUserMessage {
                                text: queued_msg.clone(),
                            },
                        );
                    }
                    // Add assistant response + queued user message to context
                    let assistant_text = response
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    context.add_message(Message::assistant(assistant_text));
                    let injected = Message::user(queued_msg.clone());
                    context.add_message(injected);
                    let _ = message_service
                        .create_message(session_id, "user".to_string(), queued_msg)
                        .await;
                    // Create a NEW assistant placeholder so the next response
                    // gets a sequence number AFTER the queued user message.
                    // Without this, the next LLM response appends to the old
                    // placeholder (created before the user message), causing
                    // the reply to appear ABOVE the user's message in the DB.
                    assistant_db_msg = message_service
                        .create_message(session_id, "assistant".to_string(), String::new())
                        .await
                        .map_err(|e| AgentError::Database(e.to_string()))?;
                    continue;
                }

                if iteration > 0 {
                    tracing::info!("Agent completed after {} tool iterations", iteration);
                    // Emit final text so TUI persists it as a permanent message.
                    // CLI providers: helpers.rs already flushed cli_unflushed_text
                    // as IntermediateText at stream end — skip to avoid duplication.
                    if !is_cli_provider
                        && !iteration_text.is_empty()
                        && let Some(ref cb) = progress_callback
                    {
                        cb(
                            session_id,
                            ProgressEvent::IntermediateText {
                                text: iteration_text,
                                reasoning: reasoning_text,
                            },
                        );
                    }
                } else {
                    tracing::info!("Agent responded with text only (no tool calls)");
                }
                final_response = Some(response);
                break;
            }

            // Emit intermediate text to TUI so it appears before the tool calls
            if !iteration_text.is_empty()
                && let Some(ref cb) = progress_callback
            {
                cb(
                    session_id,
                    ProgressEvent::IntermediateText {
                        text: iteration_text,
                        reasoning: reasoning_text,
                    },
                );
            }

            // Detect tool loops: hash the full input for every tool.
            // Different arguments = different hash = no false loop detection.
            let current_call_signature = tool_uses
                .iter()
                .map(|(_, name, input)| {
                    let input_str = serde_json::to_string(input).unwrap_or_default();
                    let hash: u64 = input_str
                        .bytes()
                        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
                    format!("{}:{:x}", name, hash)
                })
                .collect::<Vec<_>>()
                .join(",");

            recent_tool_calls.push(current_call_signature.clone());

            // Keep last 50 iterations for loop detection.
            // Modern agents legitimately make dozens of tool calls with different args.
            // Signatures include arguments, so only truly identical calls match.
            if recent_tool_calls.len() > 50 {
                recent_tool_calls.remove(0);
            }

            // Check for repeated patterns with tool-specific thresholds.
            // Only triggers for truly identical calls (same tool + same arguments).

            let is_modification_tool = current_call_signature.starts_with("write:")
                || current_call_signature.starts_with("edit:")
                || current_call_signature.starts_with("bash:");

            // Modification tools get a lower threshold (dangerous if looping).
            // Everything else gets a generous threshold since signatures
            // already distinguish different arguments.
            let loop_threshold = if is_modification_tool {
                4 // Same exact write/edit/bash command 4 times = stuck
            } else {
                8 // Same exact call with same exact args 8 times = stuck
            };

            // Check if we have enough calls to detect a loop
            if recent_tool_calls.len() >= loop_threshold {
                let last_n = &recent_tool_calls[recent_tool_calls.len() - loop_threshold..];
                if last_n.iter().all(|call| call == &current_call_signature) {
                    tracing::warn!(
                        "⚠️ Detected tool loop: '{}' called {} times in a row. Breaking loop.",
                        current_call_signature,
                        loop_threshold
                    );

                    if is_modification_tool {
                        tracing::warn!(
                            "⚠️ Modification tool loop detected. \
                             Same command repeated {} times with identical arguments.",
                            loop_threshold
                        );
                    }

                    // Force a final response by breaking the loop
                    final_response = Some(response);
                    break;
                }
            }

            // Execute tools and build response message
            let mut tool_results = Vec::new();
            let mut tool_descriptions: Vec<String> = Vec::new(); // For DB persistence
            let mut tool_outputs: Vec<(bool, String)> = Vec::new(); // (success, output) parallel to descriptions

            for (tool_id, tool_name, tool_input) in tool_uses {
                // Check for cancellation before each tool
                if let Some(ref token) = cancel_token
                    && token.is_cancelled()
                {
                    tracing::warn!(
                        "🛑 Tool execution cancelled before '{}' at iteration {}",
                        tool_name,
                        iteration,
                    );
                    break;
                }

                tracing::info!("Executing tool '{}' (iteration {})", tool_name, iteration,);

                // Save tool input for progress reporting (before it's moved to execute)
                let tool_input_for_progress = tool_input.clone();

                // Build short description for DB persistence
                tool_descriptions.push(Self::format_tool_summary(&tool_name, &tool_input));

                // Emit tool started progress
                if let Some(ref cb) = progress_callback {
                    cb(
                        session_id,
                        ProgressEvent::ToolStarted {
                            tool_name: tool_name.clone(),
                            tool_input: tool_input_for_progress.clone(),
                        },
                    );
                }

                // Check if approval is needed.
                // Each channel's make_approval_callback() already checks
                // check_approval_policy() from config — the tool loop only
                // respects the auto_approve_tools flag and tool-level policy.
                let needs_approval = if let Some(tool) = self.tool_registry.get(&tool_name) {
                    tool.requires_approval_for_input(&tool_input)
                        && (!self.auto_approve_tools || has_override_approval)
                        && !tool_context.auto_approve
                } else {
                    false
                };

                // Request approval if needed
                if needs_approval {
                    if let Some(ref approval_cb) = approval_callback {
                        // Get tool details for approval request
                        let tool_info = if let Some(tool) = self.tool_registry.get(&tool_name) {
                            ToolApprovalInfo {
                                session_id,
                                tool_name: tool_name.clone(),
                                tool_description: tool.description().to_string(),
                                tool_input: tool_input.clone(),
                                capabilities: tool
                                    .capabilities()
                                    .iter()
                                    .map(|c| format!("{:?}", c))
                                    .collect(),
                            }
                        } else {
                            // Tool not found, skip approval
                            let err = format!("Tool not found: {}", tool_name);
                            tool_outputs.push((false, err.clone()));
                            tool_results.push(ContentBlock::ToolResult {
                                tool_use_id: tool_id,
                                content: err,
                                is_error: Some(true),
                            });
                            continue;
                        };

                        // Call approval callback
                        tracing::info!("Requesting user approval for tool '{}'", tool_name);
                        match approval_cb(tool_info).await {
                            Ok((approved, always_approve)) => {
                                if !approved {
                                    tracing::warn!("User denied approval for tool '{}'", tool_name);
                                    tool_outputs
                                        .push((false, "User denied permission".to_string()));
                                    tool_results.push(ContentBlock::ToolResult {
                                        tool_use_id: tool_id,
                                        content: "User denied permission to execute this tool"
                                            .to_string(),
                                        is_error: Some(true),
                                    });
                                    continue;
                                }
                                // Propagate "always approve" to skip callbacks for remaining tools
                                if always_approve {
                                    tool_context.auto_approve = true;
                                    tracing::info!(
                                        "User selected 'Always' — auto-approving remaining tools in this loop"
                                    );
                                }
                                tracing::info!("User approved tool '{}'", tool_name);
                                // Create approved context for this tool execution
                                let approved_tool_context = ToolExecutionContext {
                                    session_id: tool_context.session_id,
                                    working_directory: tool_context.working_directory.clone(),
                                    env_vars: tool_context.env_vars.clone(),
                                    auto_approve: true, // User approved this execution
                                    timeout_secs: tool_context.timeout_secs,
                                    sudo_callback: tool_context.sudo_callback.clone(),
                                    shared_working_directory: tool_context
                                        .shared_working_directory
                                        .clone(),
                                    service_context: tool_context.service_context.clone(),
                                };

                                // Execute the tool with approved context, racing against cancel
                                let exec_result = tokio::select! {
                                    biased;
                                    _ = async {
                                        if let Some(ref t) = cancel_token { t.cancelled().await } else { std::future::pending().await }
                                    } => {
                                        tracing::warn!("🛑 Tool '{}' cancelled mid-execution", tool_name);
                                        break;
                                    }
                                    r = self.tool_registry.execute(&tool_name, tool_input, &approved_tool_context) => r,
                                };
                                match exec_result {
                                    Ok(result) => {
                                        let success = result.success;
                                        let content = if result.success {
                                            result.output
                                        } else {
                                            result.error.unwrap_or_else(|| {
                                                "Tool execution failed".to_string()
                                            })
                                        };

                                        // GRANULAR LOG: Tool execution result
                                        if success {
                                            tracing::info!(
                                                "[TOOL_EXEC] ✅ Tool '{}' executed successfully, output_len={}",
                                                tool_name,
                                                content.len()
                                            );
                                        } else {
                                            tracing::error!(
                                                "[TOOL_EXEC] ❌ Tool '{}' failed: {}",
                                                tool_name,
                                                content.chars().take(200).collect::<String>()
                                            );
                                        }

                                        let output_summary: String =
                                            content.chars().take(2000).collect();
                                        tool_outputs.push((success, output_summary.clone()));
                                        if let Some(ref cb) = progress_callback {
                                            cb(
                                                session_id,
                                                ProgressEvent::ToolCompleted {
                                                    tool_name: tool_name.clone(),
                                                    tool_input: tool_input_for_progress.clone(),
                                                    success,
                                                    summary: output_summary,
                                                },
                                            );
                                        }
                                        tool_results.push(ContentBlock::ToolResult {
                                            tool_use_id: tool_id,
                                            content,
                                            is_error: Some(!success),
                                        });
                                    }
                                    Err(e) => {
                                        let err_msg = format!("Tool execution error: {}", e);
                                        // GRANULAR LOG: Tool execution error
                                        tracing::error!(
                                            "[TOOL_EXEC] 💥 Tool '{}' error: {}",
                                            tool_name,
                                            err_msg
                                        );
                                        let output_summary: String =
                                            err_msg.chars().take(2000).collect();
                                        tool_outputs.push((false, output_summary.clone()));
                                        if let Some(ref cb) = progress_callback {
                                            cb(
                                                session_id,
                                                ProgressEvent::ToolCompleted {
                                                    tool_name: tool_name.clone(),
                                                    tool_input: tool_input_for_progress.clone(),
                                                    success: false,
                                                    summary: output_summary,
                                                },
                                            );
                                        }
                                        tool_results.push(ContentBlock::ToolResult {
                                            tool_use_id: tool_id,
                                            content: err_msg,
                                            is_error: Some(true),
                                        });
                                    }
                                }
                                continue; // Skip the normal execution path below
                            }
                            Err(e) => {
                                tracing::error!("Approval callback error: {}", e);
                                tool_outputs.push((false, format!("Approval failed: {}", e)));
                                tool_results.push(ContentBlock::ToolResult {
                                    tool_use_id: tool_id,
                                    content: format!("Approval request failed: {}", e),
                                    is_error: Some(true),
                                });
                                continue;
                            }
                        }
                    } else {
                        // No approval callback configured, deny execution
                        tracing::warn!(
                            "Tool '{}' requires approval but no approval callback configured",
                            tool_name
                        );
                        tool_outputs.push((false, "No approval mechanism configured".to_string()));
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: tool_id,
                            content: "Tool requires approval but no approval mechanism configured"
                                .to_string(),
                            is_error: Some(true),
                        });
                        continue;
                    }
                }

                // Execute the tool (no approval needed — mark context as approved
                // so the registry's own approval check doesn't block it)
                let mut approved_context = tool_context.clone();
                approved_context.auto_approve = true;
                let exec_result = tokio::select! {
                    biased;
                    _ = async {
                        if let Some(ref t) = cancel_token { t.cancelled().await } else { std::future::pending().await }
                    } => {
                        tracing::warn!("🛑 Tool '{}' cancelled mid-execution", tool_name);
                        break;
                    }
                    r = self.tool_registry.execute(&tool_name, tool_input, &approved_context) => r,
                };
                match exec_result {
                    Ok(result) => {
                        let success = result.success;
                        let content = if result.success {
                            result.output
                        } else {
                            result
                                .error
                                .unwrap_or_else(|| "Tool execution failed".to_string())
                        };

                        // GRANULAR LOG: Direct tool execution result
                        if success {
                            tracing::info!(
                                "[TOOL_EXEC] ✅ Tool '{}' executed successfully, output_len={}",
                                tool_name,
                                content.len()
                            );
                        } else {
                            tracing::error!(
                                "[TOOL_EXEC] ❌ Tool '{}' failed: {}",
                                tool_name,
                                content.chars().take(200).collect::<String>()
                            );
                        }

                        let output_summary: String = content.chars().take(2000).collect();
                        tool_outputs.push((success, output_summary.clone()));
                        if let Some(ref cb) = progress_callback {
                            cb(
                                session_id,
                                ProgressEvent::ToolCompleted {
                                    tool_name: tool_name.clone(),
                                    tool_input: tool_input_for_progress.clone(),
                                    success,
                                    summary: output_summary,
                                },
                            );
                        }
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: tool_id,
                            content,
                            is_error: Some(!success),
                        });
                    }
                    Err(e) => {
                        let err_msg = format!("Tool execution error: {}", e);
                        // GRANULAR LOG: Direct tool execution error
                        tracing::error!("[TOOL_EXEC] 💥 Tool '{}' error: {}", tool_name, err_msg);
                        let output_summary: String = err_msg.chars().take(2000).collect();
                        tool_outputs.push((false, output_summary.clone()));
                        if let Some(ref cb) = progress_callback {
                            cb(
                                session_id,
                                ProgressEvent::ToolCompleted {
                                    tool_name: tool_name.clone(),
                                    tool_input: tool_input_for_progress.clone(),
                                    success: false,
                                    summary: output_summary,
                                },
                            );
                        }
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: tool_id,
                            content: err_msg,
                            is_error: Some(true),
                        });
                    }
                }
            }

            // Append tool call data to accumulated text for DB persistence.
            // v2 format: <!-- tools-v2: [{"d":"desc","s":true,"o":"output..."}] -->
            // Includes tool output so Ctrl+O expansion works after session reload.
            if !tool_descriptions.is_empty() {
                if !accumulated_text.is_empty() {
                    accumulated_text.push('\n');
                }
                let entries: Vec<serde_json::Value> = tool_descriptions.iter()
                    .zip(tool_outputs.iter())
                    .map(|(desc, (success, output))| {
                        serde_json::json!({"d": desc, "s": success, "o": output})
                    })
                    .collect();
                accumulated_text.push_str(&format!(
                    "<!-- tools-v2: {} -->",
                    serde_json::to_string(&entries).unwrap_or_default()
                ));

                // REAL-TIME PERSISTENCE: Save tool results to DB immediately
                let tool_block = format!(
                    "\n<!-- tools-v2: {} -->\n",
                    serde_json::to_string(&entries).unwrap_or_default()
                );
                let _ = message_service
                    .append_content(assistant_db_msg.id, &tool_block)
                    .await;

                // Notify TUI after each tool iteration so it refreshes in real-time,
                // even during long-running channel sessions (Telegram, WhatsApp, etc.)
                if let Some(ref tx) = self.session_updated_tx {
                    let _ = tx.send(session_id);
                }

                tool_descriptions.clear();
                tool_outputs.clear();
            }

            // Add assistant message with tool use to context (filter empty text blocks)
            let clean_content: Vec<ContentBlock> = response
                .content
                .iter()
                .filter(|b| !matches!(b, ContentBlock::Text { text } if text.is_empty()))
                .cloned()
                .collect();
            let assistant_msg = Message {
                role: crate::brain::provider::Role::Assistant,
                content: clean_content,
            };
            context.add_message(assistant_msg);

            // Add user message with tool results to context
            let tool_result_msg = Message {
                role: crate::brain::provider::Role::User,
                content: tool_results,
            };
            context.add_message(tool_result_msg);

            // Fire token count update after tool results are added — keeps TUI in sync.
            if let Some(ref cb) = progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
            }
            if has_progress_override && let Some(ref cb) = self.progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
            }

            // Enforce 65% budget after tool results (skip for CLI — it manages its own context)
            if let Some(ref summary) = if is_cli_provider {
                None
            } else {
                self.enforce_context_budget(
                    session_id,
                    &mut context,
                    &model_name,
                    &progress_callback,
                )
                .await
            } {
                // Persist compaction marker to DB so restarts load from this point
                let compaction_marker = format!(
                    "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                     Below is a structured summary of everything before this point.]\n\n{}",
                    summary
                );
                if let Err(e) = message_service
                    .create_message(session_id, "user".to_string(), compaction_marker)
                    .await
                {
                    tracing::error!("Failed to persist post-tool compaction marker to DB: {}", e);
                }

                let mut cont_text =
                    "[SYSTEM: Mid-loop context compaction complete. The summary above has \
                     full context of everything done so far. Briefly acknowledge the \
                     compaction to the user with a fun/cheeky remark (be creative, surprise \
                     them — cursing allowed), then pick up where you left off. Do NOT re-do \
                     completed work.]"
                        .to_string();
                if !self.auto_approve_tools {
                    cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
                }
                context.add_message(Message::user(cont_text));
            }

            // Check for queued user messages to inject between tool iterations.
            // This lets the user provide follow-up feedback mid-execution (like Claude Code).
            if let Some(ref queue_cb) = self.message_queue_callback
                && let Some(queued_msg) = queue_cb().await
            {
                tracing::info!("Injecting queued user message between tool iterations");

                // Notify TUI so the user message appears inline in the chat flow
                if let Some(ref cb) = progress_callback {
                    cb(
                        session_id,
                        ProgressEvent::QueuedUserMessage {
                            text: queued_msg.clone(),
                        },
                    );
                }

                let injected = Message::user(queued_msg.clone());
                context.add_message(injected);

                // Save to database so conversation history stays consistent
                let _ = message_service
                    .create_message(session_id, "user".to_string(), queued_msg)
                    .await;
                // Create a NEW assistant placeholder so the next response
                // gets a sequence number AFTER the queued user message.
                assistant_db_msg = message_service
                    .create_message(session_id, "assistant".to_string(), String::new())
                    .await
                    .map_err(|e| AgentError::Database(e.to_string()))?;
            }
        }

        // === GRACEFUL SAVE ON CANCEL/LOOP-BREAK ===
        // If we broke out of the loop without a final_response (cancellation, error, etc.)
        // but we have accumulated text/tool results, they're already in the DB from real-time persistence.
        // Usage update is handled below in the unified path after response synthesis —
        // doing it here too would double-count because the synthesized response (line below)
        // still flows through the final update_session_usage call.
        if final_response.is_none() && !accumulated_text.is_empty() {
            tracing::info!(
                "Loop broken without final response but accumulated text ({} chars) already persisted in real-time",
                accumulated_text.len()
            );
        }

        // If the loop broke without a final_response but we have accumulated text,
        // synthesize a partial response instead of erroring — the user already saw the
        // text streamed in real-time, so returning it keeps the TUI consistent.
        let response = match final_response {
            Some(resp) => resp,
            None if !accumulated_text.is_empty() => {
                tracing::warn!(
                    "Synthesizing partial response from {} chars of accumulated text \
                     (loop broke without final LLM response)",
                    accumulated_text.len()
                );
                LLMResponse {
                    id: String::new(),
                    content: vec![ContentBlock::Text {
                        text: accumulated_text.clone(),
                    }],
                    model: model_name.clone(),
                    usage: crate::brain::provider::TokenUsage {
                        input_tokens: total_input_tokens,
                        output_tokens: total_output_tokens,
                        cache_creation_tokens: total_cache_creation,
                        cache_read_tokens: total_cache_read,
                        ..Default::default()
                    },
                    stop_reason: Some(crate::brain::provider::StopReason::EndTurn),
                }
            }
            None => {
                // If the cancel token is set and was triggered, this is a user-initiated
                // cancellation — return Cancelled instead of a noisy Internal error.
                if let Some(ref token) = cancel_token
                    && token.is_cancelled()
                {
                    return Err(AgentError::Cancelled);
                }
                return Err(AgentError::Internal(
                    "Tool loop ended without final response".to_string(),
                ));
            }
        };

        // Extract text from the final response only (for TUI display).
        // Intermediate text was already shown in real-time via IntermediateText events.
        let final_text = Self::extract_text_from_response(&response);

        // The assistant message was already created and updated in real-time.
        // Now update with final token usage.

        // Calculate total cost with full cache breakdown for accurate pricing.
        // input_tokens = non-cached, cache_creation/read tracked separately.
        let billable_input = total_input_tokens + total_cache_creation + total_cache_read;
        let total_tokens = billable_input + total_output_tokens;
        let cost = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .calculate_cost_with_cache(
                &response.model,
                total_input_tokens,
                total_output_tokens,
                total_cache_creation,
                total_cache_read,
            );

        // Update message with usage info
        message_service
            .update_message_usage(assistant_db_msg.id, total_tokens as i32, cost)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Update session token usage
        session_service
            .update_session_usage(session_id, total_tokens as i32, cost)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Notify the TUI that this session was updated (enables live refresh when
        // a remote channel — Telegram, WhatsApp, Discord, Slack — processes a message).
        if let Some(ref tx) = self.session_updated_tx {
            let _ = tx.send(session_id);
        }

        Ok(AgentResponse {
            message_id: assistant_db_msg.id,
            content: final_text,
            stop_reason: response.stop_reason,
            usage: crate::brain::provider::TokenUsage {
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
                cache_creation_tokens: total_cache_creation,
                cache_read_tokens: total_cache_read,
                ..Default::default()
            },
            context_tokens: context.token_count as u32,
            cost,
            model: response.model,
        })
    }
}
