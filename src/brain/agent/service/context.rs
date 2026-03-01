use super::builder::AgentService;
use super::types::ProgressEvent;
use crate::brain::agent::context::AgentContext;
use crate::brain::agent::error::{AgentError, Result};
use crate::brain::provider::{LLMRequest, Message};
use crate::services::{MessageService, SessionService};
use uuid::Uuid;

impl AgentService {
    /// Helper to prepare message context for LLM requests
    ///
    /// This extracts the common setup logic shared between send_message() and
    /// send_message_streaming() to reduce code duplication.
    pub(super) async fn prepare_message_context(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<(String, LLMRequest, MessageService, SessionService)> {
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

        let db_messages = Self::trim_messages_to_budget(
            all_db_messages,
            context_window as usize,
            self.tool_registry.count(),
            self.default_system_brain.as_deref(),
        );

        let mut context =
            AgentContext::from_db_messages(session_id, db_messages, context_window as usize);

        // Add system brain if available
        if let Some(brain) = &self.default_system_brain {
            context.system_brain = Some(brain.clone());
        }

        // Add user message
        let user_msg = Message::user(user_message.clone());
        context.add_message(user_msg);

        // Save user message to database
        message_service
            .create_message(session_id, "user".to_string(), user_message)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Build base LLM request
        let request = LLMRequest::new(model_name.clone(), context.messages.clone())
            .with_max_tokens(self.max_tokens);

        let request = if let Some(system) = context.system_brain {
            request.with_system(system)
        } else {
            request
        };

        Ok((model_name, request, message_service, session_service))
    }

    /// Trim DB messages to fit within the context budget.
    ///
    /// Keeps only the most recent messages that fit within ~60% of the context window
    /// after reserving space for tool definitions, brain, and response.
    /// Uses tiktoken cl100k_base for accurate token counting — no more chars/N guessing.
    pub(super) fn trim_messages_to_budget(
        all_messages: Vec<crate::db::models::Message>,
        context_window: usize,
        tool_count: usize,
        brain: Option<&str>,
    ) -> Vec<crate::db::models::Message> {
        use crate::brain::tokenizer;

        let tool_budget = tool_count * 500;
        let brain_budget = brain.map(tokenizer::count_tokens).unwrap_or(0);
        let history_budget = context_window
            .saturating_sub(tool_budget)
            .saturating_sub(brain_budget)
            .saturating_sub(16384) // reserve for response
            * 60
            / 100; // Target 60% to leave headroom for tool results and overhead

        let mut token_acc = 0usize;
        let mut keep_from = 0usize;
        for (i, msg) in all_messages.iter().enumerate().rev() {
            if msg.content.is_empty() {
                continue;
            }
            let msg_tokens = tokenizer::count_message_tokens(&msg.content);
            if token_acc + msg_tokens > history_budget {
                keep_from = i + 1;
                break;
            }
            token_acc += msg_tokens;
        }

        if keep_from > 0 {
            let kept = all_messages.len() - keep_from;
            tracing::info!(
                "Context budget: keeping last {} of {} messages ({} tokens via tiktoken, budget {}, window {})",
                kept,
                all_messages.len(),
                token_acc,
                history_budget,
                context_window
            );
            all_messages[keep_from..].to_vec()
        } else {
            all_messages
        }
    }

    /// Auto-compact the context when usage is too high.
    ///
    /// Before compaction, calculates the remaining context budget and sends
    /// the last portion of the conversation to the LLM with a request for a
    /// structured breakdown. This breakdown serves as a "wake-up" summary so
    /// OpenCrabs can continue working seamlessly after compaction.
    pub(super) async fn compact_context(
        &self,
        session_id: Uuid,
        context: &mut AgentContext,
        model_name: &str,
    ) -> Result<String> {
        // Emit compacting progress
        if let Some(ref cb) = self.progress_callback {
            cb(session_id, ProgressEvent::Compacting);
        }

        let remaining_budget = context.max_tokens.saturating_sub(context.token_count);

        // Build a summarization request with the full conversation
        let mut summary_messages = Vec::new();

        // Include all conversation messages so the LLM sees the full context.
        // Skip any leading user messages that consist only of ToolResult blocks —
        // they are orphaned (their tool_use was removed by a prior trim) and would
        // cause the API to reject the request with a 400.
        let start = context
            .messages
            .iter()
            .position(|m| {
                !(m.role == crate::brain::provider::Role::User
                    && !m.content.is_empty()
                    && m.content.iter().all(|b| {
                        matches!(b, crate::brain::provider::ContentBlock::ToolResult { .. })
                    }))
            })
            .unwrap_or(context.messages.len());

        // Cap the messages sent to the summarizer so the compaction request itself
        // never exceeds the provider's context window. Reserve enough tokens for:
        // - compaction prompt (~1k tokens)
        // - system prompt (~1k tokens)
        // - output budget (8k tokens)
        // - safety margin (6k tokens)
        // Total overhead: 16k tokens. Take the LAST N messages (most recent = most useful).
        let compaction_overhead = 16_000usize;
        // Also cap at 75% of context window to leave headroom — compaction request
        // must itself fit within the provider limit.
        let max_budget = (context.max_tokens as f64 * 0.75) as usize;
        let summary_budget = context
            .max_tokens
            .saturating_sub(compaction_overhead)
            .min(max_budget);
        let mut running_tokens = 0usize;
        let all_msgs = &context.messages[start..];
        // Walk backwards from most-recent until we hit the budget
        let msgs_to_include: Vec<&Message> = all_msgs
            .iter()
            .rev()
            .take_while(|m| {
                let t = AgentContext::estimate_tokens_static(m);
                if running_tokens + t <= summary_budget {
                    running_tokens += t;
                    true
                } else {
                    false
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        tracing::info!(
            "Compaction: sending {} / {} messages to summarizer ({} / {} tokens)",
            msgs_to_include.len(),
            all_msgs.len(),
            running_tokens,
            context.max_tokens,
        );

        for msg in msgs_to_include {
            summary_messages.push(msg.clone());
        }

        // Add the compaction instruction as a user message
        let compaction_prompt = format!(
            "IMPORTANT: The context window is at {:.0}% capacity ({} / {} tokens, {} tokens remaining). \
             The conversation must be compacted to continue.\n\n\
             Please provide a STRUCTURED BREAKDOWN of this entire conversation so far. \
             This will be used as the sole context when the agent wakes up after compaction. \
             Include ALL of the following sections:\n\n\
             ## Current Task\n\
             What is the user currently working on? What was the last request?\n\n\
             ## Key Decisions Made\n\
             List all important decisions, choices, and conclusions reached.\n\n\
             ## Files Modified\n\
             List every file that was created, edited, or discussed, with a brief note on what changed.\n\n\
             ## Current State\n\
             Where did we leave off? What is the next step? Any pending work?\n\n\
             ## Important Context\n\
             Any critical details, constraints, preferences, or gotchas the agent must remember.\n\n\
             ## Errors & Solutions\n\
             Any errors encountered and how they were resolved.\n\n\
             ## Tool Approval Policy\n\
             State whether tool approval is required (auto-approve OFF) or tools run freely (auto-approve ON). \
             This is CRITICAL for the agent to know post-compaction.\n\n\
             Be concise but complete — this summary is the ONLY context the agent will have after compaction.\n\n\
             Tool approval status for this session: {}",
            context.usage_percentage(),
            context.token_count,
            context.max_tokens,
            remaining_budget,
            if self.auto_approve_tools {
                "AUTO-APPROVE ON (tools run freely)"
            } else {
                "AUTO-APPROVE OFF — tool approval is REQUIRED for every tool call"
            },
        );

        summary_messages.push(Message::user(compaction_prompt));

        // Output budget: cap at 8k tokens for the summary itself (plenty for structured output)
        let summary_max_tokens = 8_000u32.min(self.max_tokens);

        let request = LLMRequest::new(model_name.to_string(), summary_messages)
            .with_max_tokens(summary_max_tokens)
            .with_system("You are a precise summarization assistant. Your job is to create a structured breakdown of the conversation that will serve as the complete context for an AI agent continuing this work after context compaction. Be thorough — include every file, decision, and pending task.".to_string());

        // Use streaming so the TUI shows the summary being written in real-time
        // instead of freezing silently for 2-5 minutes on large contexts
        let (response, _reasoning) = self
            .stream_complete(session_id, request, None, None)
            .await
            .map_err(AgentError::Provider)?;

        let summary = Self::extract_text_from_response(&response);

        // Save to daily memory log
        if let Err(e) = self.save_to_memory(&summary).await {
            tracing::warn!("Failed to save compaction summary to daily log: {}", e);
        }

        // Index the updated memory file in the background so memory_search picks it up
        let memory_path = crate::config::opencrabs_home()
            .join("memory")
            .join(format!("{}.md", chrono::Local::now().format("%Y-%m-%d")));
        tokio::spawn(async move {
            if let Ok(store) = crate::memory::get_store() {
                let _ = crate::memory::index_file(store, &memory_path).await;
            }
        });

        // Compact the context: keep last 4 message pairs (8 messages)
        context.compact_with_summary(summary.clone(), 8);

        tracing::info!(
            "Context compacted: now at {:.0}% ({} tokens)",
            context.usage_percentage(),
            context.token_count
        );

        // Show the summary to the user in chat
        if let Some(ref cb) = self.progress_callback {
            cb(
                session_id,
                ProgressEvent::CompactionSummary {
                    summary: summary.clone(),
                },
            );
        }

        Ok(summary)
    }

    /// Save a compaction summary to a daily memory log at `~/.opencrabs/memory/YYYY-MM-DD.md`.
    ///
    /// Multiple compactions per day append to the same file. The brain workspace's
    /// `MEMORY.md` is left untouched — it stays as user-curated durable memory.
    pub(super) async fn save_to_memory(&self, summary: &str) -> std::result::Result<(), String> {
        let memory_dir = crate::config::opencrabs_home().join("memory");

        std::fs::create_dir_all(&memory_dir)
            .map_err(|e| format!("Failed to create memory directory: {}", e))?;

        let date = chrono::Local::now().format("%Y-%m-%d");
        let memory_path = memory_dir.join(format!("{}.md", date));

        // Read existing content (if any — multiple compactions per day stack)
        let existing = std::fs::read_to_string(&memory_path).unwrap_or_default();

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let new_content = format!(
            "{}\n\n---\n\n## Auto-Compaction Summary ({})\n\n{}\n",
            existing.trim(),
            timestamp,
            summary
        );

        std::fs::write(&memory_path, new_content.trim_start())
            .map_err(|e| format!("Failed to write daily memory log: {}", e))?;

        tracing::info!("Saved compaction summary to {}", memory_path.display());
        Ok(())
    }
}
