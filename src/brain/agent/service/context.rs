use super::builder::AgentService;
use crate::brain::agent::context::AgentContext;
use crate::brain::agent::error::{AgentError, Result};
use crate::brain::provider::{ContentBlock, LLMRequest, Message, Provider};
use crate::services::{MessageService, SessionService};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl AgentService {
    /// The per-turn plan reminder pinned at the end of the prompt, keyed to
    /// the session's plan-mode state: an Active checklist gets the task
    /// nudge (`format_plan_reminder`), an Editing session (pre-init or
    /// design prose) gets the Editing rules (`format_editing_reminder`),
    /// and NoPlan gets nothing. Loaded through the shared plan store so
    /// legacy statuses map (and terminal ones resolve) first.
    /// The user message as the model should see it: the raw text plus the
    /// per-turn plan reminder and any relevant MEMORY.md recall.
    ///
    /// Both additions are context-only. The DB always stores the clean
    /// `user_message`, so neither can pollute chat history, and neither piles
    /// up across turns.
    ///
    /// Shared because the two message paths had drifted (#995): the tool loop
    /// appended plan reminder AND recall, `prepare_message_context` appended
    /// only the reminder, and the two blocks were otherwise identical. Whether
    /// memory surfaced therefore depended on which code path a session took,
    /// which is not a property of the memory.
    pub(super) async fn augment_user_message(session_id: Uuid, user_message: &str) -> String {
        let mut out = match Self::active_plan_reminder(session_id).await {
            Some(reminder) => format!("{user_message}\n\n{reminder}"),
            None => user_message.to_string(),
        };
        // Ride relevant memory along with the message (#799). MEMORY.md was
        // written constantly and read almost never; #800 made reading cheap,
        // but a cheap read still has to be chosen, and the model cannot decide
        // to recall a correction it has forgotten exists.
        if let Some(recall) = crate::brain::memory_recall::recall_for(user_message).await {
            tracing::info!(
                "Recalled {} chars from MEMORY.md for session {session_id}",
                recall.len()
            );
            out.push_str("\n\n");
            out.push_str(&recall);
        }
        out
    }

    pub(super) async fn active_plan_reminder(session_id: Uuid) -> Option<String> {
        use crate::utils::plan_files::{self, PlanModeState};
        match plan_files::plan_mode_state(session_id).await {
            PlanModeState::NoPlan => None,
            PlanModeState::PreInitEditing => Some(format_editing_reminder(None)),
            PlanModeState::PostInitEditing => {
                let md = plan_files::plan_md_path(session_id).await;
                // Checklist plans have no design document (#1145): don't
                // point the reminder at a file that does not exist.
                Some(if md.exists() {
                    format_editing_reminder(Some(md))
                } else {
                    format_checklist_editing_reminder().to_string()
                })
            }
            PlanModeState::Active => {
                let plan = plan_files::load_plan(session_id).await?;
                format_plan_reminder(&plan)
            }
        }
    }

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
        self.prepare_message_context_with_display(session_id, user_message, None, model)
            .await
    }

    /// Like [`prepare_message_context`](Self::prepare_message_context) but
    /// persists `display_text` (when set) to the DB instead of the full
    /// `user_message`. The LLM context still receives `user_message`, so
    /// turn-scoped scaffolding (reaction guidance, steering prefaces) reaches
    /// the model without polluting session history or future context.
    pub(super) async fn prepare_message_context_with_display(
        &self,
        session_id: Uuid,
        user_message: String,
        display_text: Option<String>,
        model: Option<String>,
    ) -> Result<(String, LLMRequest, MessageService, SessionService)> {
        // Get or create session
        let session_service = SessionService::new(self.context.clone());
        let _session = session_service
            .get_session(session_id)
            .await
            .map_err(AgentError::db)?
            .ok_or(AgentError::SessionNotFound(session_id))?;

        // Load conversation context with budget-aware message trimming
        let message_service = MessageService::new(self.context.clone());
        let all_db_messages = message_service
            .list_messages_for_session(session_id)
            .await
            .map_err(AgentError::db)?;

        let model_name = model.unwrap_or_else(|| {
            self.provider_for_session(session_id)
                .default_model()
                .to_string()
        });
        let context_window = self.context_limit_for_session(session_id);

        // Load from last compaction point — no arbitrary trimming
        let db_messages = Self::messages_from_last_compaction(all_db_messages);

        let mut context =
            AgentContext::from_db_messages(session_id, db_messages, context_window as usize);

        // Add system brain if available (count its tokens for accurate tracking).
        // `live_system_brain` rebuilds from disk when a brain file changed so
        // edits take effect on the next turn without a restart (#213). The
        // session-aware variant patches the Runtime Info Model/Provider lines
        // to the session's resolved pair, not the startup default.
        if let Some(brain) = self.live_system_brain_for_session(session_id) {
            context.token_count += AgentContext::estimate_tokens(&brain);
            context.system_brain = Some(brain);
        }

        // Add user message. If a plan is actively executing, append a compact
        // reminder of its incomplete tasks so it rides at the END of the prompt
        // (best recall) every turn — without it the plan scrolls out of the
        // recency window in a long conversation and the model forgets it was
        // mid-plan (discussion #177). Regenerated each turn from the plan file;
        // the DB only ever stores the clean user message, so it never piles up.
        let context_user_message = Self::augment_user_message(session_id, &user_message).await;
        let user_msg = Message::user(context_user_message);
        context.add_message(user_msg);

        // Save user message to database. When a display override is set,
        // history records the compact form; the full text was context-only.
        message_service
            .create_message(
                session_id,
                "user".to_string(),
                display_text.unwrap_or(user_message),
            )
            .await
            .map_err(AgentError::db)?;

        // Build base LLM request. The output reservation is bounded by this
        // session's active context window so a 200K route keeps input headroom.
        let request = LLMRequest::new(model_name.clone(), context.messages.clone())
            .with_max_tokens(self.request_max_tokens_for_session(session_id));

        // Surface a small "Recently accessed" anchor section so the
        // agent re-uses real paths from prior sessions / pre-compaction
        // turns instead of hallucinating directory layouts. Filtered
        // against the live messages so we don't double-list paths the
        // agent just touched in this same session.
        let working_directory = self.get_working_directory_for_session(session_id);
        let recent_paths = self.recent_paths_for_dir(&working_directory).await;
        let augmented_system = Self::augment_system_with_recent_paths(
            context.system_brain,
            &recent_paths,
            &context.messages,
        );

        let mut request = if let Some(system) = augmented_system {
            request.with_system(system)
        } else {
            request
        };

        // Pass working directory so proxy-aware providers can forward it
        request.working_directory = Some(working_directory.to_string_lossy().to_string());
        request.session_id = Some(session_id);

        Ok((model_name, request, message_service, session_service))
    }

    /// Append a "Recently accessed" anchor section to `system_brain`,
    /// listing only the paths from the persistent recent-paths store
    /// that don't already appear verbatim in any of the live messages.
    /// Returns `base` unchanged when there's nothing to surface — keeps
    /// the prompt clean during normal uncompacted runs where the literal
    /// tool_call/tool_result blocks already mention the path.
    pub(crate) fn augment_system_with_recent_paths(
        base: Option<String>,
        recent_paths: &[String],
        messages: &[Message],
    ) -> Option<String> {
        if recent_paths.is_empty() {
            return base;
        }
        let context_blob: String = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                ContentBlock::ToolUse { input, .. } => Some(input.to_string()),
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let context_for_match = context_blob.to_lowercase();

        let surviving: Vec<&String> = recent_paths
            .iter()
            .filter(|p| !context_for_match.contains(&p.to_lowercase()))
            .collect();
        if surviving.is_empty() {
            return base;
        }
        let mut out = base.unwrap_or_default();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(
            "\n--- Recently accessed in this project ---\n\
             (Real paths previously confirmed by read/edit/grep/ls. Prefer these as anchors \
             over guessing from naming conventions.)\n",
        );
        for p in surviving {
            out.push_str("  - ");
            out.push_str(p);
            out.push('\n');
        }
        Some(out)
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

    /// Build a "recovered brain" context string from key brain files.
    ///
    /// After compaction wipes the conversation history, this restores the agent's
    /// core identity, user context, tool documentation, and coding standards so it
    /// doesn't wake up with only a lossy LLM summary.
    ///
    /// Full files injected (~1-2k tokens total) — identity + always-enforced
    /// rules ONLY:
    /// - SOUL.md — personality / voice
    /// - USER.md — who the human is, preferences
    /// - AGENTS.md — workspace governance + the always-enforced hard rules
    ///
    /// Everything else is contextual and loaded ON DEMAND via `load_brain_file`,
    /// exactly as during a normal turn: CODE.md (before code tasks), TOOLS.md
    /// (environment/tool specifics), SECURITY.md, MEMORY.md, BOOT/HEARTBEAT.
    /// They are NOT pre-injected here — the system prompt's "Available Context
    /// Files" index (reassembled fresh every turn) keeps them discoverable, so
    /// re-injecting them after compaction would just burn tokens on context the
    /// task may not need. A one-line pointer below reminds the agent to fetch
    /// them when relevant.
    fn build_recovered_brain_context() -> String {
        use std::path::PathBuf;

        let full_files = [
            ("SOUL.md", "personality / voice"),
            ("USER.md", "user profile"),
            ("AGENTS.md", "workspace governance + enforced hard rules"),
        ];

        let opencrabs_home = crate::config::opencrabs_home();
        let mut files_block = String::new();

        for (filename, label) in full_files {
            let path: PathBuf = opencrabs_home.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    files_block.push_str(&format!(
                        "--- {} ({}) ---\n{}\n\n",
                        filename, label, trimmed
                    ));
                }
            }
        }

        if files_block.is_empty() {
            return String::from("[No brain files found — agent context limited]\n\n");
        }

        // Contextual files (CODE.md before code work, TOOLS.md for tool specifics,
        // SECURITY/MEMORY/BOOT/HEARTBEAT) are NOT re-injected here — they load on
        // demand like any normal turn. We don't repeat that directive: AGENTS.md
        // (re-injected above) already owns it ("If writing code: Read CODE.md"),
        // and the system prompt's always-present "Available Context Files" index
        // keeps the full set discoverable. One source, no duplication.
        format!(
            "[RECOVERED BRAIN CONTEXT — these files define your identity, the user, and your \
             always-enforced rules. They take priority over any contradictory inference from the \
             summary.]\n\n{files_block}\n"
        )
    }

    /// Synchronous compaction: compute a summary and apply it to `context` in place.
    /// Used by the manual `/compact` command and the two emergency callsites that
    /// recover from "context too large" provider errors. The async path used by
    /// `enforce_context_budget` does NOT go through here — it spawns
    /// `compute_compaction_summary` directly and applies the result via
    /// `apply_compaction_summary` once the LLM call finishes.
    pub(super) async fn compact_context(
        &self,
        session_id: Uuid,
        context: &mut AgentContext,
        model_name: &str,
        cancel_token: Option<&CancellationToken>,
    ) -> Result<String> {
        // This call replaces the whole context, so a background summariser
        // still describing the pre-call conversation is describing something
        // that will not exist by the time it returns. Applying its result
        // later would overwrite the compaction happening right here.
        if let Some(superseded) = self.take_pending_compaction(session_id) {
            tracing::info!("Background compaction superseded by a synchronous one — aborting it");
            superseded.abort();
        }

        let provider = self.provider_for_session(session_id);
        let cancel = cancel_token.cloned().unwrap_or_default();

        let summary = Self::compute_compaction_summary(
            provider,
            self.fallback_chain_snapshot(),
            session_id,
            context.messages.clone(),
            context.token_count,
            context.max_tokens,
            context.usage_percentage(),
            model_name.to_string(),
            self.request_max_tokens_for_session(session_id),
            self.get_working_directory_for_session(session_id),
            self.auto_approve_tools,
            cancel,
            self.compaction_attempt_deadline(session_id),
        )
        .await?;

        let summary =
            Self::decorate_compaction_summary(summary, session_id, self.subagent_manager.clone())
                .await;

        Self::apply_compaction_summary(context, &summary);
        Ok(summary)
    }

    /// Attach the state a model's prose summary cannot be trusted to carry.
    ///
    /// Both blocks are harness-written so they survive a summary that forgot
    /// them, and both ride INSIDE the persisted marker rather than arriving as
    /// separate messages that can scroll away:
    ///
    /// - session plan artifacts, so the post-compaction agent resumes the plan
    ///   instead of rediscovering it;
    /// - live sub-agent IDs, so `wait_agent` / `send_input` / `resume_agent` /
    ///   `close_agent` still have something to address (#936).
    ///
    /// Owns no `&self` and takes the manager by value: the background
    /// summariser calls it from a spawned task with nothing but a snapshot.
    pub(super) async fn decorate_compaction_summary(
        summary: String,
        session_id: Uuid,
        subagents: Option<Arc<crate::brain::tools::subagent::SubAgentManager>>,
    ) -> String {
        let summary = match plan_state_block(session_id).await {
            Some(block) => format!("{summary}\n\n{block}"),
            None => summary,
        };
        match subagents.and_then(|m| m.format_running_for_compaction()) {
            Some(block) => format!("{summary}\n\n{block}"),
            None => summary,
        }
    }

    /// Send the summariser request, walking `[providers.fallback]` when the
    /// session's provider fails (#1247).
    ///
    /// Compaction used to call `provider.complete()` once and surface whatever
    /// came back. Every other request path in the process walks the chain, so
    /// a session whose primary was rate-limited or out of credit kept chatting
    /// happily via a fallback while `/compact` died on the dead primary — and
    /// with the context window full, a session that cannot compact cannot
    /// recover at all.
    ///
    /// Mirrors the tool loop's walk deliberately: skip the primary's own name,
    /// try every remaining entry in configured order (#1251 — no provider is
    /// ever dropped from the walk for having failed before), remap the model
    /// to each fallback's default when it doesn't carry the requested one,
    /// and report the whole ledger if everything dies.
    /// `pub(crate)` for the regression tests in `src/tests` — no caller outside
    /// Bound on a single summariser attempt for a session with no compaction
    /// history to scale from. Not a guess: it is the 300s every HTTP provider
    /// in this codebase already enforces per request
    /// (`anthropic.rs::DEFAULT_TIMEOUT`), extended to the CLI providers that
    /// ship no timeout at all.
    pub(crate) const COMPACTION_ATTEMPT_FLOOR: std::time::Duration =
        std::time::Duration::from_secs(300);

    /// One summariser attempt, bounded.
    ///
    /// HTTP providers already cap a single request at
    /// `anthropic.rs::DEFAULT_TIMEOUT` (300s). CLI providers carry no timeout
    /// at all, so a summariser that stopped answering held the session for as
    /// long as the process felt like living, and the fallback chain below was
    /// never reached because the first attempt never returned (#1255). This
    /// extends the bound every HTTP provider already honours to the ones that
    /// do not, and a provider that blows it is handed on rather than waited
    /// out: `Timeout` is retryable, so `should_try_next_provider` walks.
    async fn compaction_attempt(
        provider: &Arc<dyn Provider>,
        request: LLMRequest,
        deadline: std::time::Duration,
    ) -> std::result::Result<
        crate::brain::provider::LLMResponse,
        crate::brain::provider::ProviderError,
    > {
        match tokio::time::timeout(deadline, provider.complete(request)).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    "Compaction: provider '{}' produced nothing in {:?} — moving on",
                    provider.name(),
                    deadline,
                );
                Err(crate::brain::provider::ProviderError::Timeout(
                    deadline.as_secs(),
                ))
            }
        }
    }

    /// compaction should send a summariser request.
    pub(crate) async fn complete_compaction_request(
        primary: &Arc<dyn Provider>,
        fallbacks: &[Arc<dyn Provider>],
        request: LLMRequest,
        cancel: &CancellationToken,
        attempt_deadline: std::time::Duration,
    ) -> Result<crate::brain::provider::LLMResponse> {
        use crate::brain::provider::error as provider_error;

        let primary_name = primary.name().to_string();
        let first_err = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::info!("Compaction cancelled before completion");
                return Err(AgentError::Cancelled);
            }
            r = Self::compaction_attempt(primary, request.clone(), attempt_deadline) => match r {
                Ok(response) => return Ok(response),
                Err(e) => e,
            },
        };

        if fallbacks.is_empty() || !provider_error::should_try_next_provider(&first_err) {
            return Err(AgentError::Provider(first_err));
        }

        tracing::warn!(
            "Compaction: primary '{}' failed ({}) — walking fallback chain",
            primary_name,
            provider_error::short_error_reason(&first_err),
        );

        // Size is the one failure where the order matters: a narrower window
        // than the one that just refused is a refusal spent before the
        // answer, so an overflow walks widest first (#1379). Everything else
        // keeps the configured order.
        let ordered: Vec<Arc<dyn Provider>> = if matches!(
            first_err,
            crate::brain::provider::ProviderError::ContextLengthExceeded(_)
        ) {
            crate::brain::provider::chain_order::widest_first(fallbacks)
        } else {
            fallbacks.to_vec()
        };

        let mut tried: Vec<String> = Vec::new();
        let mut last_err = first_err;

        for fallback in &ordered {
            let name = fallback.name().to_string();
            if name == primary_name {
                continue;
            }
            // A substitute runs its own configured model, not the one the
            // failed request carried; the same rule the chat chain applies,
            // through the same function (#1374).
            let fb_request =
                crate::brain::provider::fallback::substitute_request(fallback.as_ref(), &request);
            tracing::info!(
                "Compaction: trying fallback provider '{}' (model '{}')",
                name,
                fb_request.model
            );

            let err = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::info!("Compaction cancelled while walking fallback chain");
                    return Err(AgentError::Cancelled);
                }
                r = Self::compaction_attempt(fallback, fb_request, attempt_deadline) => match r {
                    Ok(response) => {
                        tracing::info!("Compaction served by fallback '{}'", name);
                        return Ok(response);
                    }
                    Err(e) => e,
                },
            };

            tried.push(format!(
                "{}: {}",
                name,
                provider_error::short_error_reason(&err)
            ));
            last_err = err;
        }

        let summary = provider_error::chain_exhausted_summary(
            &primary_name,
            &provider_error::short_error_reason(&last_err),
            &tried,
        );
        tracing::error!("Compaction: fallback chain exhausted: {summary}");
        Err(AgentError::Provider(provider_error::with_chain_summary(
            last_err, summary,
        )))
    }

    /// Compute a compaction summary from a snapshot of messages.
    ///
    /// This is the LLM-facing half of compaction. It does not touch any live
    /// session state — it operates entirely on the cloned snapshot — so it
    /// is safe to call from a background `tokio::spawn` task while the agent
    /// keeps appending new messages to the live context.
    ///
    /// Returns the raw summary text. Callers that want to apply it to a live
    /// context should call `apply_compaction_summary` once the future resolves.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn compute_compaction_summary(
        provider: Arc<dyn Provider>,
        fallbacks: Vec<Arc<dyn Provider>>,
        session_id: Uuid,
        snapshot_messages: Vec<Message>,
        snapshot_token_count: usize,
        snapshot_max_tokens: usize,
        snapshot_usage_pct: f64,
        model_name: String,
        max_output_tokens: u32,
        working_directory: PathBuf,
        auto_approve_tools: bool,
        cancel: CancellationToken,
        attempt_deadline: std::time::Duration,
    ) -> Result<String> {
        let remaining_budget = snapshot_max_tokens.saturating_sub(snapshot_token_count);

        // Skip any leading user messages that consist only of ToolResult blocks —
        // they are orphaned (their tool_use was removed by a prior trim) and would
        // cause the API to reject the request with a 400.
        let start = snapshot_messages
            .iter()
            .position(|m| {
                !(m.role == crate::brain::provider::Role::User
                    && !m.content.is_empty()
                    && m.content.iter().all(|b| {
                        matches!(b, crate::brain::provider::ContentBlock::ToolResult { .. })
                    }))
            })
            .unwrap_or(snapshot_messages.len());

        // Reserve room for the summarizer's OUTPUT budget (8k) + prompt (~1k).
        let output_reserve = 8_000usize + 1_000usize;
        let max_input_budget = snapshot_max_tokens.saturating_sub(output_reserve);
        let all_msgs = &snapshot_messages[start..];
        let mut running_tokens = 0usize;
        let msgs_to_include: Vec<&Message> = all_msgs
            .iter()
            .rev()
            .take_while(|m| {
                let t = AgentContext::estimate_tokens_static(m);
                if running_tokens + t <= max_input_budget {
                    running_tokens += t;
                    true
                } else {
                    tracing::warn!(
                        "Compaction: dropping oldest messages to fit input budget ({}/{} tokens used)",
                        running_tokens,
                        max_input_budget,
                    );
                    false
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        tracing::info!(
            "Compaction: sending {} / {} messages to summarizer ({} / {} input tokens, reserving {} for output)",
            msgs_to_include.len(),
            all_msgs.len(),
            running_tokens,
            snapshot_max_tokens,
            output_reserve,
        );

        let mut summary_messages: Vec<Message> = msgs_to_include.into_iter().cloned().collect();

        let compaction_prompt = format!(
            "CRITICAL: The context window is at {:.0}% capacity ({} / {} tokens, {} tokens remaining). \
             The conversation must be compacted NOW.\n\n\
             You are creating a COMPREHENSIVE CONTINUATION DOCUMENT. After compaction, a fresh agent \
             instance will wake up with ONLY this summary as context. It must be able to continue \
             working immediately without asking the user what to do.\n\n\
             Analyze the ENTIRE conversation chronologically and produce the following:\n\n\
             ## 0. IMMEDIATE TASK (CRITICAL — MOST IMPORTANT SECTION)\n\
             Look at the LAST 6-8 message pairs in the conversation. Extract EXACTLY:\n\
             - What was the user's LAST instruction or request? (quote their exact words)\n\
             - What was the agent doing in response? (exact tool calls, file edits, investigations in progress)\n\
             - What is the EXACT next action the agent should take?\n\n\
             Write this as a DIRECTIVE, not a description. Use this format:\n\
             \"CONTINUE THIS TASK: The user asked you to [exact instruction]. \
             You were [exact action in progress — e.g. 'editing file X at line Y', 'running command Z']. \
             Your next step is [specific next action]. \
             Do NOT deviate to any other topic.\"\n\n\
             This is the MOST IMPORTANT section. If nothing else survives compaction, this must. \
             The fresh agent will read this section FIRST and act on it IMMEDIATELY.\n\n\
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
             ## 7. Recovery Playbook\n\
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
             ## 8. Next Step\n\
             State the single most important thing the agent should do when it wakes up. \
             If the task is clear, continue immediately. If ambiguous, ask the user ONE focused \
             follow-up question.\n\n\
             ## 9. Continuation Message\n\
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
            snapshot_usage_pct,
            snapshot_token_count,
            snapshot_max_tokens,
            remaining_budget,
            if auto_approve_tools {
                "AUTO-APPROVE ON (tools run freely)"
            } else {
                "AUTO-APPROVE OFF — tool approval is REQUIRED for every tool call"
            },
        );

        summary_messages.push(Message::user(compaction_prompt));

        // Never send a {provider, model} pair the user didn't configure.
        // If the requested model isn't supported by this provider, remap to
        // the provider's own default — same invariant `stream_complete` enforces.
        let mut effective_model = model_name;
        let supported = provider.supported_models();
        if !supported.is_empty() && !supported.iter().any(|m| m == &effective_model) {
            let remapped = provider.default_model().to_string();
            tracing::warn!(
                "compute_compaction_summary: provider '{}' does not support model '{}' — remapping to '{}'",
                provider.name(),
                effective_model,
                remapped,
            );
            effective_model = remapped;
        }

        let mut request = LLMRequest::new(effective_model, summary_messages)
            .with_max_tokens(max_output_tokens)
            .with_system(
                "You are a continuation document generator. Your job is to create an exhaustive, \
                 detailed knowledge transfer document from a conversation so that a fresh AI agent can \
                 continue the work seamlessly. You must capture every file path, code snippet, user preference, \
                 error, and pending task. The agent reading your output will have ZERO prior context — \
                 your document is its entire memory. Be thorough to the point of being verbose. \
                 Missing a single detail could cause the agent to repeat mistakes or violate user preferences."
                    .to_string(),
            );
        request.working_directory = Some(working_directory.to_string_lossy().to_string());
        request.session_id = Some(session_id);

        // Non-streaming call so no compaction text leaks to the TUI in the
        // background-spawn case. `cancel` aborts the request mid-flight if the
        // caller signals (e.g. 90% hard-truncate firing on the same session).
        let response = Self::complete_compaction_request(
            &provider,
            &fallbacks,
            request,
            &cancel,
            attempt_deadline,
        )
        .await?;

        let summary = Self::extract_text_from_response(&response);

        if let Err(e) = Self::save_compaction_summary_to_memory(&summary).await {
            tracing::warn!("Failed to save compaction summary to daily log: {}", e);
        }

        // Index the updated memory file in the background so memory_search picks it up.
        let memory_path = crate::config::opencrabs_home()
            .join("memory")
            .join(format!("{}.md", chrono::Local::now().format("%Y-%m-%d")));
        tokio::spawn(async move {
            match crate::memory::get_store() {
                Ok(store) => {
                    if let Err(e) = crate::memory::index_file(store, &memory_path).await {
                        tracing::warn!("Failed to index daily note after compaction: {e}");
                    }
                }
                Err(e) => tracing::warn!("Memory store unavailable, daily note not indexed: {e}"),
            }
        });

        Ok(summary)
    }

    /// Apply a previously-computed compaction summary to a live `AgentContext`.
    ///
    /// Builds the recovered-brain preamble plus a snapshot of the most recent
    /// messages, then calls `AgentContext::compact_with_summary` to do the
    /// in-place swap (replace older messages with the summary, keep the recent
    /// tail within 55% of the window).
    /// Apply a summary that was computed in the background, keeping whatever
    /// the turn appended while the summariser was still thinking.
    ///
    /// `apply_compaction_summary` clears the whole message vector, which is
    /// right when the summariser blocked the turn: nothing could arrive in the
    /// meantime. A background summariser leaves a gap, and everything in that
    /// gap (the tool calls and results of the turn still running) is work the
    /// summary never saw and cannot describe. Clearing it would silently
    /// delete the most recent thing the agent did.
    pub(crate) fn apply_compaction_summary_after(
        context: &mut AgentContext,
        summary: &str,
        snapshot_len: usize,
    ) {
        // The index addresses the message vector this turn is appending to.
        // A vector shorter than the snapshot cannot be that one: the context
        // is rebuilt from the database at the start of every turn, so this
        // means the pending entry outlived its turn. There is no delta to
        // keep, only a summary to apply.
        if snapshot_len > context.messages.len() {
            tracing::warn!(
                "Compaction snapshot ({snapshot_len} messages) outlived its context ({}) — \
                 applying the summary without a delta",
                context.messages.len(),
            );
            Self::apply_compaction_summary(context, summary);
            return;
        }

        let mut delta = context.messages.split_off(snapshot_len);
        // The summary replaces everything the snapshot covered, so it is
        // computed against exactly what it summarises.
        Self::apply_compaction_summary(context, summary);

        // The summary lands as a user message. A delta opening with tool
        // results has lost the assistant tool_use that authorised them, and
        // the provider rejects that shape outright.
        let orphans = delta
            .iter()
            .position(|m| !AgentContext::is_orphaned_tool_result_msg(m))
            .unwrap_or(delta.len());
        if orphans > 0 {
            tracing::debug!("Compaction delta: dropping {orphans} orphaned tool results");
            delta.drain(..orphans);
        }

        let kept = delta.len();
        for msg in delta {
            context.add_message(msg);
        }
        tracing::info!("Compaction: kept {kept} messages appended during the summariser call");
    }

    pub(super) fn apply_compaction_summary(context: &mut AgentContext, summary: &str) {
        let recent_snapshot = Self::format_recent_messages(&context.messages, 8);
        let brain_context = Self::build_recovered_brain_context();
        let summary_with_context = if recent_snapshot.is_empty() {
            format!("{}\n\n{}", brain_context, summary)
        } else {
            format!(
                "{}\n\n{}\n\n## Recent Message Pairs (pre-compaction snapshot)\n\
                 CRITICAL: These are the messages from RIGHT BEFORE compaction. You MUST \
                 continue from where you left off. Read these messages, identify what was \
                 in progress, and continue that exact work. Do NOT start a new topic. \
                 Do NOT ask the user what to do — the answer is in these messages.\n\n{}",
                brain_context, summary, recent_snapshot
            )
        };

        // After compaction, the summary IS the conversation — it's prepended
        // as a single user message and the agent picks up from there. We do
        // NOT preserve a raw pre-compaction tail: the summary already embeds
        // the recent-snapshot prose, so keeping the raw tail on top would
        // just duplicate ~half the window and defeat the whole purpose of
        // compacting. Pass 0 so `compact_with_summary` clears everything
        // and prepends just the summary.
        context.compact_with_summary(summary_with_context, 0);

        tracing::info!(
            "Context compacted: now at {:.0}% ({} tokens)",
            context.usage_percentage(),
            context.token_count
        );
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
                        // Truncate very long text blocks to ~500 bytes
                        let display = if text.len() > 500 {
                            let end = text.floor_char_boundary(500);
                            format!("{}… [truncated]", &text[..end])
                        } else {
                            text.clone()
                        };
                        lines.push(format!("{}: {}", role_label, display));
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        let input_preview = {
                            let s = input.to_string();
                            if s.len() > 200 {
                                let end = s.floor_char_boundary(200);
                                format!("{}…", &s[..end])
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
                            let end = content.floor_char_boundary(300);
                            format!("{}… [truncated]", &content[..end])
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
                                let end = thinking.floor_char_boundary(300);
                                format!("{}… [truncated]", &thinking[..end])
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
    pub(super) async fn save_compaction_summary_to_memory(
        summary: &str,
    ) -> std::result::Result<(), String> {
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

/// Harness-written plan-state block appended to every compaction summary
/// while session plan artifacts exist: state (Editing / Active), the
/// absolute `.md` path when present, and one line on what to do next.
/// `None` when the session is NoPlan (no plan chatter in the summary).
pub(crate) async fn plan_state_block(session_id: Uuid) -> Option<String> {
    use crate::utils::plan_files::{self, PlanModeState};
    let md = plan_files::plan_md_path(session_id).await;
    match plan_files::plan_mode_state(session_id).await {
        PlanModeState::NoPlan => None,
        PlanModeState::PreInitEditing => Some(
            "[PLAN MODE — injected by the harness, not from the user]\n\
             \n\
             **State:** Pre-init (no plan yet)\n\
             \n\
             **Now:**\n\
             1. Explore with reads/search/bash\n\
             2. Call: `plan init mode='design' title='<3-8 words>'`\n\
             \n\
             **Next:** Harness creates .md with template for you to edit.\n\
             \n\
             **Constraints:**\n\
             - No project file edits\n\
             - No pasting plan in chat"
                .to_string(),
        ),
        PlanModeState::PostInitEditing => {
            // PostInitEditing either has a design `.md` to refine (design track)
            // or no `.md` at all — a checklist plan waiting on the user's
            // Approve (#1145: checklist init no longer writes the scaffold).
            // Don't point at a file that does not exist.
            if md.exists() {
                Some(format!(
                    "[PLAN MODE — injected by the harness, not from the user]\n\
                     \n\
                     **State:** Editing design prose\n\
                     **File:** {}\n\
                     \n\
                     **Now:**\n\
                     1. Re-read .md\n\
                     2. Refine design\n\
                     \n\
                     **Next:** Wait for `/execute` approval.\n\
                     \n\
                     **Constraints:**\n\
                     - No `plan start`/`complete`\n\
                     - No project file edits\n\
                     - No pasting plan in chat",
                    md.display()
                ))
            } else {
                Some(format_checklist_editing_reminder().to_string())
            }
        }
        PlanModeState::Active => {
            let (title, done, total) = plan_files::load_plan(session_id)
                .await
                .map(|p| {
                    let done = p
                        .tasks
                        .iter()
                        .filter(|t| {
                            matches!(
                                t.status,
                                crate::tui::plan::TaskStatus::Completed
                                    | crate::tui::plan::TaskStatus::Skipped
                            )
                        })
                        .count();
                    (p.title, done, p.tasks.len())
                })
                .unwrap_or_default();
            let md_line = if md.exists() {
                format!("\nPlan document (frozen): {}", md.display())
            } else {
                String::new()
            };
            Some(format!(
                "## PLAN STATE (harness)\n\
                 State: Active checklist \"{title}\" ({done}/{total} done).{md_line}\n\
                 Next: call plan start (no args) to resurface the in-progress task, \
                 then continue executing the checklist."
            ))
        }
    }
}

/// The Editing reminder for a CHECKLIST plan: the checklist is the
/// deliverable and there is no design document to refine (#1145 — checklist
/// init no longer writes the scaffold `.md`). Pure, like
/// [`format_editing_reminder`].
pub(crate) fn format_checklist_editing_reminder() -> &'static str {
    "[PLAN MODE — injected by the harness, not from the user]\n\
     \n\
     **State:** Checklist plan in Editing (no design document)\n\
     \n\
     **Now:**\n\
     1. The task list is already visible to the user in the plan card.\n\
     2. Do NOT restate the tasks in chat.\n\
     \n\
     **Next:** Wait for the user to approve (Approve button or `/execute`).\n\
     \n\
     **Constraints:**\n\
     - No `plan start`/`complete`\n\
     - No project file edits"
}

/// Build the pinned Editing reminder: the session is in Plan mode, so the
/// turn refines the SESSION PLAN instead of executing. `md_path` is the
/// design document once `plan init` created it (post-init); `None` means
/// pre-init (no document yet). Pure (no IO) so it's unit-testable.
pub(crate) fn format_editing_reminder(md_path: Option<std::path::PathBuf>) -> String {
    match md_path {
        Some(md) => format!(
            "[PLAN MODE — injected by the harness, not from the user]\n\
             \n\
             **State:** Editing design prose\n\
             **File:** {}\n\
             \n\
             **Now:**\n\
             1. Re-read .md\n\
             2. Refine design\n\
             \n\
             **Next:** Wait for `/execute` approval.\n\
             \n\
             **Constraints:**\n\
             - No `plan start`/`complete`\n\
             - No project file edits\n\
             - No pasting plan in chat",
            md.display()
        ),
        None => "[PLAN MODE — injected by the harness, not from the user]\n\
                 \n\
                 **State:** Pre-init (no plan yet)\n\
                 \n\
                 **Now:**\n\
                 1. Explore with reads/search/bash\n\
                 2. Call: `plan init mode='design' title='<3-8 words>'`\n\
                 \n\
                 **Next:** Harness creates .md with template for you to edit.\n\
                 \n\
                 **Constraints:**\n\
                 - No project file edits\n\
                 - No pasting plan in chat"
            .to_string(),
    }
}

/// Build the pinned plan reminder from a plan document. `None` unless the
/// checklist is live (Active) with unresolved tasks — Editing (pre-init flag
/// or design prose) gets no execution nagging. Pure (no IO) so it's
/// unit-testable; `active_plan_reminder` does the file load.
pub(crate) fn format_plan_reminder(plan: &crate::tui::plan::PlanDocument) -> Option<String> {
    use crate::tui::plan::{PlanStatus, TaskStatus};

    if plan.status != PlanStatus::Active || plan.pre_init_editing {
        return None;
    }
    let total = plan.tasks.len();
    if total == 0 {
        return None;
    }
    let done = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed))
        .count();
    // Skipped tasks are intentionally resolved; once every task is done or
    // skipped there's nothing left to nag about.
    let resolved = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Skipped))
        .count();
    if resolved == total {
        return None;
    }

    let mut out = format!(
        "[ACTIVE PLAN REMINDER — injected by the harness, not from the user]\n\
         📋 Plan: \"{}\" ({done}/{total} done). Keep executing it; do not abandon it. \
         Use the plan tool's `complete` as you finish each task (it auto-starts the next), and \
         `start` to (re)surface a task's full details.\n\
         Do NOT repeat the plan title or this reminder text in your response — the \
         plan is already displayed to the user by the surface they are reading.\n",
        plan.title
    );
    let mut tasks: Vec<&crate::tui::plan::PlanTask> = plan.tasks.iter().collect();
    tasks.sort_by_key(|t| t.order);
    // Resolve the set of completed/skipped task ids once, so we can name a
    // blocked task's specific unmet dependencies.
    let resolved_ids: std::collections::HashSet<uuid::Uuid> = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Skipped))
        .map(|t| t.id)
        .collect();
    for t in &tasks {
        // [Type, N★] suffix gives the model the task shape at a glance.
        let meta = format!("[{}, {}★]", t.task_type, t.complexity.clamp(1, 5));
        match &t.status {
            TaskStatus::InProgress => {
                out.push_str(&format!("→ Task {}: {} {meta}\n", t.order, t.title));
                if !t.description.is_empty() {
                    let desc: String = t.description.chars().take(160).collect();
                    out.push_str(&format!("  {desc}\n"));
                }
                if !t.acceptance_criteria.is_empty() {
                    let crit = t.acceptance_criteria.join(" • ");
                    out.push_str(&format!("  Criteria: • {crit}\n"));
                }
            }
            TaskStatus::Pending => {
                // Pending but with unmet deps → call it out as blocked, naming them.
                let unmet: Vec<usize> = t
                    .dependencies
                    .iter()
                    .filter_map(|d| d.as_uuid())
                    .filter(|id| !resolved_ids.contains(id))
                    .filter_map(|id| plan.tasks.iter().find(|x| x.id == id).map(|x| x.order))
                    .collect();
                if unmet.is_empty() {
                    out.push_str(&format!("☐ Task {}: {} {meta}\n", t.order, t.title));
                } else {
                    let blockers = unmet
                        .iter()
                        .map(|o| o.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!(
                        "⊘ Task {}: {} {meta} (blocked: {blockers})\n",
                        t.order, t.title
                    ));
                }
            }
            TaskStatus::Failed => out.push_str(&format!(
                "✗ Task {}: {} {meta} (failed — retry/fix)\n",
                t.order, t.title
            )),
            TaskStatus::Blocked(reason) => out.push_str(&format!(
                "⊘ Task {}: {} {meta} (blocked: {reason})\n",
                t.order, t.title
            )),
            TaskStatus::Completed | TaskStatus::Skipped => {}
        }
    }
    Some(out)
}
