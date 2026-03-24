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

        // Load from last compaction point — no arbitrary trimming
        let db_messages = Self::messages_from_last_compaction(all_db_messages);

        let mut context =
            AgentContext::from_db_messages(session_id, db_messages, context_window as usize);

        // Add system brain if available (count its tokens for accurate tracking)
        if let Some(brain) = &self.default_system_brain {
            context.token_count += AgentContext::estimate_tokens(brain);
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

        let mut request = if let Some(system) = context.system_brain {
            request.with_system(system)
        } else {
            request
        };

        // Pass working directory so proxy-aware providers can forward it
        request.working_directory =
            Some(self.get_working_directory().to_string_lossy().to_string());

        Ok((model_name, request, message_service, session_service))
    }

    /// Load messages from the last compaction point forward.
    ///
    /// Finds the last message containing the `[CONTEXT COMPACTION` marker and
    /// returns only messages from that point onward. If no compaction marker
    /// exists, returns all messages. This ensures restarts pick up exactly
    /// where compaction left off — no arbitrary trimming.
    pub fn messages_from_last_compaction(
        all_messages: Vec<crate::db::models::Message>,
    ) -> Vec<crate::db::models::Message> {
        const COMPACTION_MARKER: &str = "[CONTEXT COMPACTION";

        // Walk backward to find the last compaction marker
        let compaction_idx = all_messages
            .iter()
            .rposition(|msg| msg.content.contains(COMPACTION_MARKER));

        if let Some(idx) = compaction_idx {
            let kept = all_messages.len() - idx;
            tracing::info!(
                "Found compaction marker at message {}/{} — loading {} messages from compaction point",
                idx,
                all_messages.len(),
                kept,
            );
            all_messages[idx..].to_vec()
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
            "CRITICAL: The context window is at {:.0}% capacity ({} / {} tokens, {} tokens remaining). \
             The conversation must be compacted NOW.\n\n\
             You are creating a COMPREHENSIVE CONTINUATION DOCUMENT. After compaction, a fresh agent \
             instance will wake up with ONLY this summary as context. It must be able to continue \
             working immediately without asking the user what to do.\n\n\
             Analyze the ENTIRE conversation chronologically and produce the following:\n\n\
             ## 1. Chronological Analysis\n\
             Walk through every task the user requested, in order. For each task include:\n\
             - What was requested\n\
             - What was done (with exact file paths and line numbers where relevant)\n\
             - Exact code snippets for any changes made (show before/after when applicable)\n\
             - Whether it was completed, committed, pushed, or still pending\n\n\
             ## 2. Files Modified\n\
             List EVERY file that was created, edited, read, or discussed. For each file include:\n\
             - Full file path\n\
             - What was changed and why\n\
             - Key code snippets showing the current state of changes\n\
             - Whether the change is committed or uncommitted\n\n\
             ## 3. User Preferences & Constraints\n\
             List EVERY preference, constraint, or strong reaction from the user. Include:\n\
             - Things the user explicitly said to NEVER do (with their exact words if they were emphatic)\n\
             - Workflow preferences (commit style, release process, tool choices)\n\
             - Technical constraints or architectural decisions\n\
             - Any corrections the user made to your work\n\n\
             ## 4. Errors & Corrections\n\
             Every error encountered, every mistake made, and how each was resolved. Include:\n\
             - Exact error messages when available\n\
             - What caused the error\n\
             - The fix applied\n\
             - User reactions to mistakes (so the agent avoids repeating them)\n\n\
             ## 5. All User Messages\n\
             Summarize every user message in order, capturing their intent and exact wording \
             for important instructions. This is critical for understanding the user's communication \
             style and expectations.\n\n\
             ## 6. Pending Tasks\n\
             List everything that is NOT yet done:\n\
             - Uncommitted changes\n\
             - Tasks mentioned but not started\n\
             - Investigations in progress\n\
             - Next steps the user expects\n\n\
             ## 7. Current Work\n\
             What was the agent doing RIGHT BEFORE this compaction? What is the immediate next action? \
             The fresh agent must pick up exactly where this left off.\n\n\
             ## 8. Recovery Playbook\n\
             The fresh agent has these tools available to recover any missing context:\n\
             - `session_search` — search past conversation messages in this session by keyword\n\
             - `memory_search` — search daily memory logs and indexed knowledge\n\
             - `load_brain_file` — reload brain files (SOUL.md, TOOLS.md, USER.md, etc.) for identity/preferences\n\
             - `read_file` / `glob` / `grep` — read any file, search by pattern, search file contents\n\
             - `bash` — run shell commands (git status, git log, git diff, etc.)\n\
             - `ls` — list directory contents\n\
             - `gh` — GitHub CLI for ALL GitHub operations (repos, releases, issues, PRs). \
             NEVER use HTTP requests to GitHub — always use `gh` CLI.\n\n\
             Write a SPECIFIC recovery plan: which tools to call with which arguments to get back \
             up to speed. Example: \"Run `git status` and `git diff` to see uncommitted changes, \
             then `read_file src/main.rs` to verify the current state of the fix, then \
             `session_search 'vision fallback'` to recover details from the investigation.\"\n\
             Be concrete — include actual file paths, search queries, and commands.\n\n\
             ## 9. Next Step\n\
             State the single most important thing the agent should do when it wakes up. \
             If the task is clear, continue immediately. If ambiguous, ask the user ONE focused \
             follow-up question.\n\n\
             ## 10. Continuation Message\n\
             Write a SHORT, punchy message (2-4 sentences) that the agent will say to the user \
             right after waking up from compaction. This message MUST:\n\
             - Reference SPECIFIC things from the conversation (file names, user quotes, inside jokes, \
             frustrations, wins) — prove the agent remembers everything\n\
             - Mention what was just accomplished and what's next in a way that feels alive and engaged\n\
             - Match the user's energy and communication style from the conversation\n\
             - Be creative, surprising, maybe funny — make the user think \"holy shit it remembers\"\n\
             - End with a clear action: what the agent is about to do next or a specific question\n\
             DO NOT be generic. DO NOT say \"I'm ready to continue.\" Reference actual conversation details \
             that only someone who was there would know.\n\n\
             Tool approval status: {}\n\n\
             BE EXHAUSTIVE. This is not a summary — it is a complete knowledge transfer. \
             Include code snippets, exact paths, user quotes, error messages. \
             The fresh agent has ZERO context beyond what you write here.",
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

        let mut request = LLMRequest::new(model_name.to_string(), summary_messages)
            .with_max_tokens(self.max_tokens)
            .with_system("You are a continuation document generator. Your job is to create an exhaustive, \
             detailed knowledge transfer document from a conversation so that a fresh AI agent can \
             continue the work seamlessly. You must capture every file path, code snippet, user preference, \
             error, and pending task. The agent reading your output will have ZERO prior context — \
             your document is its entire memory. Be thorough to the point of being verbose. \
             Missing a single detail could cause the agent to repeat mistakes or violate user preferences.".to_string());
        request.working_directory =
            Some(self.get_working_directory().to_string_lossy().to_string());

        // Use streaming so the TUI shows the summary being written in real-time
        // instead of freezing silently for 2-5 minutes on large contexts
        let (response, _reasoning) = self
            .stream_complete(session_id, request, None, None, None, None)
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

        // Snapshot the last 8 messages as formatted text before compaction.
        // This gives the agent immediate access to recent context without needing
        // an extra session_search call after waking up.
        let recent_snapshot = Self::format_recent_messages(&context.messages, 8);
        let summary_with_context = if recent_snapshot.is_empty() {
            summary.clone()
        } else {
            format!(
                "{}\n\n## Recent Message Pairs (pre-compaction snapshot)\n\
                 The following are the last messages before compaction — use them to \
                 understand the current task state and decide what context to reload.\n\n{}",
                summary, recent_snapshot
            )
        };

        // Compact the context: keep recent messages within 55% of max_tokens
        // (below the 65% budget threshold so hard-truncation never fires after compaction)
        let keep_budget = (context.max_tokens as f64 * 0.55) as usize;
        context.compact_with_summary(summary_with_context, keep_budget);

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

    /// Format the last N messages into a human-readable snapshot for post-compaction context.
    /// Truncates long tool results to keep the snapshot concise.
    pub(crate) fn format_recent_messages(messages: &[Message], n: usize) -> String {
        use crate::brain::provider::{ContentBlock, Role};

        let start = messages.len().saturating_sub(n);
        let mut lines = Vec::new();

        for msg in &messages[start..] {
            let role_label = match msg.role {
                Role::User => "**User**",
                Role::Assistant => "**Assistant**",
                Role::System => "**System**",
            };

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        // Truncate very long text blocks to 500 chars
                        let display = if text.len() > 500 {
                            format!("{}… [truncated]", &text[..500])
                        } else {
                            text.clone()
                        };
                        lines.push(format!("{}: {}", role_label, display));
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        let input_preview = {
                            let s = input.to_string();
                            if s.len() > 200 {
                                format!("{}…", &s[..200])
                            } else {
                                s
                            }
                        };
                        lines.push(format!(
                            "{}: [tool_use: {}({})]",
                            role_label, name, input_preview
                        ));
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        let display = if content.len() > 300 {
                            format!("{}… [truncated]", &content[..300])
                        } else {
                            content.clone()
                        };
                        lines.push(format!("{}: [tool_result: {}]", role_label, display));
                    }
                    ContentBlock::Image { .. } => {
                        lines.push(format!("{}: [image]", role_label));
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        if !thinking.is_empty() {
                            let display = if thinking.len() > 300 {
                                format!("{}… [truncated]", &thinking[..300])
                            } else {
                                thinking.clone()
                            };
                            lines.push(format!("{}: [thinking: {}]", role_label, display));
                        }
                    }
                }
            }
        }

        lines.join("\n")
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
