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
    /// Enforce the 80 % context budget rule.
    ///
    /// - ≥ 80 %: try LLM compaction (up to 3 retries on error).
    /// - After compaction (or if all retries fail): hard-truncate to 80 % if still over.
    /// - Context NEVER exceeds 80 % after this function returns.
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

        if usage_pct <= 80.0 {
            return None;
        }

        tracing::warn!("Context at {:.0}% — triggering compaction", usage_pct);

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

        // Hard-truncate guarantee: NEVER proceed with context > 80%.
        // This fires if LLM compaction failed entirely, or if the compacted
        // context (8 recent messages + summary) is still too large.
        let target_tokens = (effective_max as f64 * 0.80) as usize;
        if context.token_count > target_tokens {
            let before = context.token_count;
            let before_msgs = context.messages.len();
            context.hard_truncate_to(target_tokens);
            tracing::warn!(
                "Hard-truncated context: {} → {} tokens, {} → {} messages (target {})",
                before,
                context.token_count,
                before_msgs,
                context.messages.len(),
                target_tokens,
            );
            if let Some(cb) = progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
                cb(
                    session_id,
                    ProgressEvent::IntermediateText {
                        text: format!(
                            "⚠️ Context hard-truncated from {} to {} tokens to stay within budget.",
                            before, context.token_count
                        ),
                        reasoning: None,
                    },
                );
            }
        }

        summary_result
    }

    /// Core tool-execution loop — called by all public shims.
    /// `override_approval_callback` and `override_progress_callback` take
    /// precedence over the service-level callbacks (used by Telegram, etc.)
    pub(super) async fn run_tool_loop(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
        cancel_token: Option<CancellationToken>,
        override_approval_callback: Option<ApprovalCallback>,
        override_progress_callback: Option<ProgressCallback>,
    ) -> Result<AgentResponse> {
        // Track this request for restart recovery
        let pending_repo = crate::db::PendingRequestRepository::new(self.context.pool());
        let request_id = Uuid::new_v4();
        if let Err(e) = pending_repo
            .insert(request_id, session_id, &user_message, "tui")
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
        let is_system_continuation = user_message.starts_with("[System:");
        if !is_system_continuation {
            let _user_db_msg = message_service
                .create_message(session_id, "user".to_string(), user_message)
                .await
                .map_err(|e| AgentError::Database(e.to_string()))?;
        }

        // Create assistant message placeholder NOW for real-time persistence.
        // We'll append content as we go and update with final tokens at the end.
        let assistant_db_msg = message_service
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
                        },
                        context_tokens: context.token_count as u32,
                        cost: 0.0,
                        model: model_name,
                    });
                }
            }
        }

        // Auto-compact: triggers at >80% usage
        let compaction_result = self
            .enforce_context_budget(session_id, &mut context, &model_name, &progress_callback)
            .await;

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
        let mut final_response: Option<LLMResponse> = None;
        let mut accumulated_text = String::new(); // Collect text from all iterations (not just final)
        let mut recent_tool_calls: Vec<String> = Vec::new(); // Track tool calls to detect loops
        let mut stream_retry_count = 0u32; // Track consecutive stream drop retries
        const MAX_STREAM_RETRIES: u32 = 2; // Retry up to 2 times on dropped streams

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

            // Enforce 80% budget before every API call
            if let Some(ref summary) = self
                .enforce_context_budget(session_id, &mut context, &model_name, &progress_callback)
                .await
            {
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

            // Send to provider via streaming — retry once after emergency compaction if prompt is too long
            let (response, reasoning_text) = match self
                .stream_complete(
                    session_id,
                    request,
                    cancel_token.as_ref(),
                    progress_callback.as_ref(),
                )
                .await
            {
                Ok(resp) => resp,
                Err(ref e)
                    if e.to_string().contains("prompt is too long")
                        || e.to_string().contains("too many tokens") =>
                {
                    tracing::warn!("Prompt too long for provider — emergency compaction");
                    let err_msg = e.to_string();
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
                        }
                        Err(compact_err) => {
                            tracing::error!("Emergency compaction also failed: {}", compact_err);
                            return Err(AgentError::Internal(format!(
                                "Provider rejected prompt ({}) and emergency compaction failed: {}",
                                err_msg, compact_err
                            )));
                        }
                    }

                    // Rebuild request with compacted context
                    let mut retry_req =
                        LLMRequest::new(model_name.clone(), context.messages.clone())
                            .with_max_tokens(self.max_tokens);
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
                    )
                    .await
                    .map_err(AgentError::Provider)?
                }
                Err(e) => return Err(AgentError::Provider(e)),
            };

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

            // Calibrate context token count with the API's real input_tokens.
            // Even with tiktoken, there's some drift since Anthropic's tokenizer differs slightly.
            // The API knows the exact count — use it to keep our tracking honest.
            let api_input = response.usage.input_tokens as usize;
            let tool_overhead = self.actual_tool_schema_tokens();
            let real_message_tokens = api_input.saturating_sub(tool_overhead);
            if real_message_tokens > 0 {
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

            // --- CANCEL CHECK BEFORE STREAM DROP RETRY ---
            // If the user cancelled during streaming, don't retry — save partial text and break.
            if response.stop_reason.is_none()
                && let Some(ref token) = cancel_token
                && token.is_cancelled()
            {
                // Extract any text from the partial response for persistence
                for block in &response.content {
                    if let ContentBlock::Text { text } = block
                        && !text.trim().is_empty()
                    {
                        if !accumulated_text.is_empty() {
                            accumulated_text.push_str("\n\n");
                        }
                        accumulated_text.push_str(text);
                        // Persist partial text to DB
                        let _ = message_service
                            .append_content(assistant_db_msg.id, &format!("{}\n\n", text))
                            .await;
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
                    // Don't increment iteration — this is a retry, not a new turn
                    iteration -= 1;
                    continue;
                } else {
                    tracing::error!(
                        "🚨 Stream dropped {} times consecutively at iteration {}. \
                         Proceeding with partial response to avoid infinite retry loop. \
                         Content blocks: {}, stop_reason: None",
                        MAX_STREAM_RETRIES,
                        iteration,
                        response.content.len(),
                    );
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
            // The LLM sometimes echoes back <!-- tools-v2: ... --> or
            // <!-- reasoning -->...<!-- /reasoning --> blocks from its
            // conversation context. Strip them so they don't leak into
            // Telegram/channel output or the TUI.
            if iteration_text.contains("<!-- tools") {
                iteration_text = Self::strip_tools_v2_markers(&iteration_text);
            }

            // ── XML tool-call stripping ──────────────────────────────────
            // Some providers (e.g. MiniMax) occasionally emit tool calls as
            // XML text instead of structured tool_calls. We used to try
            // extracting and executing these, but the synthetic IDs caused
            // providers to reject tool results ("tool id not found"),
            // triggering infinite retry loops. Just strip the XML and let
            // the model respond with text instead.
            if iteration_text.contains("<tool_call>") {
                iteration_text = Self::strip_xml_tool_calls(&iteration_text);
            }

            // Persist reasoning content to DB (before iteration text)
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

            // Accumulate text from every iteration
            if !iteration_text.is_empty() {
                if !accumulated_text.is_empty() {
                    accumulated_text.push_str("\n\n");
                }
                accumulated_text.push_str(&iteration_text);

                // REAL-TIME PERSISTENCE: Save to DB immediately after each iteration's text
                let _ = message_service
                    .append_content(assistant_db_msg.id, &format!("{}\n\n", iteration_text))
                    .await;
            }

            tracing::debug!("Found {} tool uses to execute", tool_uses.len());

            if tool_uses.is_empty() {
                if iteration > 0 {
                    tracing::info!("Agent completed after {} tool iterations", iteration);
                    // Emit final text so TUI persists it as a permanent message
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

            // Fire token count update after tool results are added — keeps TUI in sync
            if let Some(ref cb) = progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
            }
            if has_progress_override && let Some(ref cb) = self.progress_callback {
                cb(session_id, ProgressEvent::TokenCount(context.token_count));
            }

            // Enforce 80% budget after tool results (results can be massive)
            if let Some(ref summary) = self
                .enforce_context_budget(session_id, &mut context, &model_name, &progress_callback)
                .await
            {
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
            }
        }

        // === GRACEFUL SAVE ON CANCEL/LOOP-BREAK ===
        // If we broke out of the loop without a final_response (cancellation, error, etc.)
        // but we have accumulated text/tool results, they're already in the DB from real-time persistence.
        // Just ensure session usage is updated.
        if final_response.is_none() && !accumulated_text.is_empty() {
            tracing::info!(
                "Loop broken without final response but accumulated text ({} chars) already persisted in real-time",
                accumulated_text.len()
            );
            // Also save token usage for what we consumed
            let partial_tokens = total_input_tokens + total_output_tokens;
            if partial_tokens > 0 {
                let partial_cost = self
                    .provider
                    .read()
                    .expect("provider lock poisoned")
                    .calculate_cost(&model_name, total_input_tokens, total_output_tokens);
                let _ = session_service
                    .update_session_usage(session_id, partial_tokens as i32, partial_cost)
                    .await;
            }
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

        // Calculate total cost
        let total_tokens = total_input_tokens + total_output_tokens;
        let cost = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .calculate_cost(&response.model, total_input_tokens, total_output_tokens);

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
            },
            context_tokens: context.token_count as u32,
            cost,
            model: response.model,
        })
    }
}
