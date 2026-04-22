//! Messaging — session CRUD, slash commands, message expansion, streaming.

use super::dialogs::ensure_whispercrabs;
use super::events::{AppMode, ToolApprovalResponse, TuiEvent};
use super::onboarding::OnboardingWizard;
use super::*;
use crate::brain::SelfUpdater;
use anyhow::Result;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Return the byte length of a UTF-8 character from its leading byte.
#[inline]
fn utf8_char_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1, // continuation byte — shouldn't happen on valid str, advance 1
    }
}

impl App {
    /// Read the persisted approval policy from config.toml.
    /// Returns `(auto_session, auto_always)` flags.
    pub(crate) fn read_approval_policy_from_config() -> (bool, bool) {
        match crate::config::Config::load() {
            Ok(cfg) => match cfg.agent.approval_policy.as_str() {
                "auto-session" => (true, false),
                "auto-always" => (false, true),
                _ => (false, false),
            },
            Err(_) => (false, false),
        }
    }

    /// Create a new session
    pub(crate) async fn create_new_session(&mut self) -> Result<()> {
        // Inherit provider and model from the current agent service
        let provider_name = Some(self.agent_service.provider_name());
        let model = Some(self.default_model_name.clone());
        let session = self
            .session_service
            .create_session_with_provider(Some("New Chat".to_string()), provider_name, model)
            .await?;

        self.current_session = Some(session.clone());
        // Persist as last active so the next startup resumes this session.
        // Without this, `last_session` kept pointing at whatever chat the
        // user was on BEFORE creating the new one — restarts loaded the
        // old session and the user saw their fresh work as "gone".
        Self::save_last_session_id(session.id);
        self.set_plan_file_for_session(session.id);
        self.is_processing = false; // New session is never processing
        self.messages.clear();
        self.auto_scroll = true;
        self.scroll_offset = 0;
        self.mode = AppMode::Chat;
        // Clear streaming state from any previous session
        self.streaming_response = None;
        self.streaming_reasoning = None;
        self.active_tool_group = None;
        self.streaming_output_tokens = 0;
        self.intermediate_text_received = false;
        // Re-read approval policy from config (persisted by /approve)
        (self.approval_auto_session, self.approval_auto_always) =
            Self::read_approval_policy_from_config();
        // Show the system prompt + tools baseline immediately — new sessions are never 0
        let base = self.agent_service.base_context_tokens();
        self.session_input_tokens.insert(session.id, base);
        self.last_input_tokens = Some(base);

        // Sync shared session ID for channels (Telegram, WhatsApp)
        *self.shared_session_id.lock().await = Some(session.id);

        // Reload sessions list
        self.load_sessions().await?;

        Ok(())
    }

    /// Load a session and its messages
    pub(crate) async fn load_session(&mut self, session_id: Uuid) -> Result<()> {
        let session = self
            .session_service
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let messages = self
            .message_service
            .list_messages_for_session(session_id)
            .await?;

        // TUI scrollback shows the FULL session history. The agent applies
        // its own compaction window when building the LLM prompt — that's
        // independent of what the human sees. Trimming here was wiping the
        // visible chat after every auto-compaction.

        // Stash old session's cancel token before switching so background
        // processing can still be cancelled from the sessions screen.
        if let Some(old_token) = self.cancel_token.take()
            && let Some(ref old_session) = self.current_session
            && self.processing_sessions.contains(&old_session.id)
        {
            self.session_cancel_tokens.insert(old_session.id, old_token);
        }

        // Cache outgoing session's messages for inactive pane rendering
        if self.pane_manager.is_split()
            && let Some(ref old_session) = self.current_session
        {
            self.pane_message_cache
                .insert(old_session.id, self.messages.clone());
        }

        // Clear streaming state from previous session so it doesn't
        // bleed into the newly loaded session's chat view.
        self.streaming_response = None;
        self.streaming_reasoning = None;
        self.active_tool_group = None;
        self.streaming_output_tokens = 0;
        self.intermediate_text_received = false;

        // Restore session's working directory if persisted.
        // Only write to config if actually changed — writing triggers ConfigWatcher
        // which reloads config → calls load_session → writes again → infinite loop.
        if let Some(ref dir_str) = session.working_directory {
            let path = std::path::PathBuf::from(dir_str);
            if path.is_dir() && path != self.working_directory {
                self.working_directory = path.clone();
                self.agent_service.set_working_directory(path.clone());
                let _ = crate::config::Config::write_key(
                    "agent",
                    "working_directory",
                    &path.to_string_lossy(),
                );
            }
        }

        self.current_session = Some(session.clone());
        self.set_plan_file_for_session(session.id);
        // Sync is_processing flag with per-session state
        self.is_processing = self.processing_sessions.contains(&session.id);

        let (display, hidden) = Self::trim_messages_to_display_budget(&messages, 200_000);
        self.hidden_older_messages = hidden;
        self.oldest_displayed_sequence = display.first().map(|m| m.sequence).unwrap_or(0);
        self.display_token_count = display
            .iter()
            .map(|m| crate::brain::tokenizer::count_tokens(&m.content))
            .sum();
        let mut expanded: Vec<DisplayMessage> =
            display.into_iter().flat_map(Self::expand_message).collect();
        if hidden > 0 {
            expanded.insert(0, Self::make_history_marker(hidden));
        }
        self.messages = expanded;
        self.auto_scroll = true;
        self.scroll_offset = 0;
        // Re-read approval policy from config (persisted by /approve)
        (self.approval_auto_session, self.approval_auto_always) =
            Self::read_approval_policy_from_config();

        // Sync shared session ID for channels (Telegram, WhatsApp)
        *self.shared_session_id.lock().await = Some(session.id);

        // Restore last known context size, preferring the in-memory
        // per-session cache over the DB. The cache is fed by live
        // TokenCountUpdated events for this session's turns and is the
        // most accurate "last observed prompt size" — the DB column
        // still holds the cumulative `billable_input` from the
        // multi-iter fix, which over-reports context on session reload.
        let base = self.agent_service.base_context_tokens();
        let cached_tokens = self.session_input_tokens.get(&session_id).copied();
        let restored_tokens = if let Some(c) = cached_tokens {
            Some(c)
        } else {
            self.message_service
                .last_assistant_input_tokens(session_id)
                .await
                .ok()
                .flatten()
                .map(|t| t as u32)
                .or(Some(base))
        };
        if let Some(t) = restored_tokens {
            self.session_input_tokens.insert(session_id, t);
        }
        self.last_input_tokens = restored_tokens;

        // Clear unread indicator for this session
        self.sessions_with_unread.remove(&session_id);

        // Persist as last active session so startup restores it
        Self::save_last_session_id(session_id);

        // Populate the session's provider entry in the agent service so
        // future turns for this session use the right provider. Crucially
        // we do NOT call `swap_provider` (the GLOBAL default) — another
        // pane's session is allowed to sit on a different provider, and
        // mutating global mid-turn is exactly the bug that routed a
        // background qwen-plus turn at localhost:8891 when the other
        // pane switched to qwenlocal (logs 2026-04-17 17:01).
        if let Some(ref saved_provider) = session.provider_name {
            let current_prov = self.agent_service.provider_name_for_session(session.id);
            if saved_provider != &current_prov {
                // Track whether the swap actually succeeded. If it
                // fails we MUST NOT fall through to the canonical-
                // rename sync below — without this, any swap failure
                // (stale base_url, transient config read error, etc.)
                // caused the session to silently adopt the current
                // global provider as its "live name" and overwrite
                // session.provider_name in memory (and, on next save,
                // in DB). 2026-04-18 incident: a session saved on
                // "opencodeiolo" came back as "opencode" after restart
                // because the global default at load time was
                // "opencode" (first custom alphabetically) and this
                // code stamped it over the user's saved choice.
                let mut swap_ok = false;
                if let Some(cached) = self.provider_cache.get(saved_provider).cloned() {
                    tracing::info!(
                        "Restoring cached provider '{}' for session {}",
                        saved_provider,
                        session.id
                    );
                    self.agent_service
                        .swap_provider_for_session(session.id, cached);
                    swap_ok = true;
                } else if let Ok(config) = crate::config::Config::load() {
                    match crate::brain::provider::create_provider_by_name(&config, saved_provider)
                        .await
                    {
                        Ok(new_provider) => {
                            tracing::info!(
                                "Created provider '{}' for session {}",
                                saved_provider,
                                session.id
                            );
                            self.provider_cache
                                .insert(saved_provider.clone(), new_provider.clone());
                            self.agent_service
                                .swap_provider_for_session(session.id, new_provider);
                            swap_ok = true;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to restore provider '{}' for session {}: {e} — keeping saved name; user will see the real error on next turn",
                                saved_provider,
                                session.id,
                            );
                        }
                    }
                }
                // Canonical-rename sync: ONLY after a successful swap.
                // Handles legacy renames like "qwen-code" → "qwen-cli"
                // where the newly-created provider reports a different
                // canonical id than what was stored. Never runs on
                // failure, so a failed swap never mutates the session's
                // saved provider/model to the global default.
                if swap_ok {
                    let live_name = self.agent_service.provider_name_for_session(session.id);
                    if live_name != *saved_provider
                        && let Some(ref mut s) = self.current_session
                    {
                        tracing::info!(
                            "Canonical rename on load: '{}' → '{}' for session {}",
                            saved_provider,
                            live_name,
                            session.id
                        );
                        s.provider_name = Some(live_name);
                        s.model = Some(self.agent_service.provider_model_for_session(session.id));
                    }
                }
            }
        } else {
            // Legacy session with no saved provider — stamp current provider onto it
            if let Some(ref mut s) = self.current_session {
                s.provider_name = Some(self.agent_service.provider_name_for_session(session.id));
                s.model = Some(self.agent_service.provider_model_for_session(session.id));
            }
        }

        // Display model comes from the session record, falling back to the session's provider
        self.default_model_name = session
            .model
            .clone()
            .unwrap_or_else(|| self.agent_service.provider_model_for_session(session.id));
        // context_window is session-specific (different providers have
        // different windows). Cache it per session_id so the indicator
        // is correct even if the focused pane's provider differs from
        // the global default.
        let max_ctx = self
            .agent_service
            .context_window_for_model(&self.default_model_name);
        self.context_max_tokens = max_ctx;
        self.session_context_max.insert(session_id, max_ctx);

        // Keep focused pane in sync with loaded session
        if let Some(pane) = self.pane_manager.focused_pane_mut() {
            pane.session_id = Some(session_id);
        }
        // Persist layout so pane-to-session mapping survives restarts
        if self.pane_manager.is_split() {
            self.pane_manager.save_layout();
        }

        Ok(())
    }

    /// Persist the current session ID to `~/.opencrabs/last_session` so
    /// the next startup can restore the correct session.
    fn save_last_session_id(session_id: Uuid) {
        let base = crate::config::opencrabs_home();
        if let Err(e) = std::fs::write(base.join("last_session"), session_id.to_string()) {
            tracing::warn!("Failed to persist last_session: {}", e);
        }
    }

    /// Read the last active session ID from disk.
    pub(crate) fn read_last_session_id() -> Option<Uuid> {
        let base = crate::config::opencrabs_home();
        let content = std::fs::read_to_string(base.join("last_session")).ok()?;
        Uuid::parse_str(content.trim()).ok()
    }

    /// Pre-load a session's messages into the pane cache (for restored split panes).
    pub(crate) async fn preload_pane_session(&mut self, session_id: Uuid) {
        let messages = self
            .message_service
            .list_messages_for_session(session_id)
            .await
            .unwrap_or_default();
        let (display, _) = Self::trim_messages_to_display_budget(&messages, 200_000);
        let expanded: Vec<DisplayMessage> =
            display.into_iter().flat_map(Self::expand_message).collect();
        self.pane_message_cache.insert(session_id, expanded);
    }

    /// Trim a list of DB messages to fit within a token budget (newest messages kept).
    /// Returns (kept_messages, hidden_count).
    fn trim_messages_to_display_budget(
        msgs: &[crate::db::models::Message],
        budget: usize,
    ) -> (Vec<crate::db::models::Message>, usize) {
        let mut tokens = 0usize;
        let mut keep = 0usize;
        for msg in msgs.iter().rev() {
            let t = crate::brain::tokenizer::count_tokens(&msg.content);
            if tokens + t > budget {
                break;
            }
            tokens += t;
            keep += 1;
        }
        let hidden = msgs.len() - keep;
        (msgs[hidden..].to_vec(), hidden)
    }

    /// Build the dim italic history marker shown at the top of the message list.
    fn make_history_marker(count: usize) -> DisplayMessage {
        DisplayMessage {
            id: Uuid::new_v4(),
            role: "history_marker".to_string(),
            content: format!("↑ {} older messages hidden · Ctrl+O to load more", count),
            timestamp: chrono::Utc::now(),
            token_count: None,
            cost: None,
            approval: None,
            approve_menu: None,
            details: None,
            expanded: false,
            tool_group: None,
        }
    }

    /// Load an older batch of messages (up to 100k tokens) from the DB and prepend
    /// them to the current display list.  Called by Ctrl+O when hidden_older_messages > 0.
    pub(crate) async fn load_more_history(&mut self) -> Result<()> {
        let session_id = match self.current_session.as_ref().map(|s| s.id) {
            Some(id) => id,
            None => return Ok(()),
        };
        let all = self
            .message_service
            .list_messages_for_session(session_id)
            .await?;
        // Messages older than the current oldest displayed
        let older: Vec<_> = all
            .into_iter()
            .filter(|m| m.sequence < self.oldest_displayed_sequence)
            .collect(); // already ordered ASC by sequence

        let budget = 100_000usize;
        let mut tokens = 0usize;
        let mut keep = 0usize;
        for msg in older.iter().rev() {
            let t = crate::brain::tokenizer::count_tokens(&msg.content);
            if tokens + t > budget {
                break;
            }
            tokens += t;
            keep += 1;
        }
        let hidden_still = older.len().saturating_sub(keep);
        let to_add = &older[older.len() - keep..];

        // Remove existing history_marker at front
        if self
            .messages
            .first()
            .map(|m| m.role == "history_marker")
            .unwrap_or(false)
        {
            self.messages.remove(0);
        }

        let mut new_msgs: Vec<DisplayMessage> = to_add
            .iter()
            .cloned()
            .flat_map(Self::expand_message)
            .collect();
        if hidden_still > 0 {
            new_msgs.insert(0, Self::make_history_marker(hidden_still));
        }
        new_msgs.append(&mut self.messages);
        self.messages = new_msgs;
        self.hidden_older_messages = hidden_still;
        self.oldest_displayed_sequence = to_add.first().map(|m| m.sequence).unwrap_or(0);
        self.display_token_count += tokens;
        self.render_cache.clear();
        Ok(())
    }

    /// Load all sessions
    pub(crate) async fn load_sessions(&mut self) -> Result<()> {
        use crate::db::repository::{SessionListOptions, UsageLedgerRepository};

        self.sessions = self
            .session_service
            .list_sessions(SessionListOptions {
                include_archived: false,
                limit: Some(100),
                offset: 0,
            })
            .await?;

        // Load all-time usage from the ledger (survives session deletes)
        let ledger = UsageLedgerRepository::new(self.session_service.pool());
        self.usage_ledger_stats = ledger.stats_by_model().await.unwrap_or_default();

        Ok(())
    }

    /// Clear all messages from the current session
    pub(crate) async fn clear_session(&mut self) -> Result<()> {
        if let Some(session) = &self.current_session {
            // Delete all messages from the database
            self.message_service
                .delete_messages_for_session(session.id)
                .await?;

            // Clear messages from UI
            self.messages.clear();
            self.scroll_offset = 0;
            self.streaming_response = None;
            self.streaming_reasoning = None;
            self.error_message = None;
            self.error_message_shown_at = None;
        }

        Ok(())
    }

    /// Handle slash commands locally (returns true if handled)
    pub(crate) async fn handle_slash_command(&mut self, input: &str) -> bool {
        let cmd = input.split_whitespace().next().unwrap_or("");
        match cmd {
            "/models" => {
                self.open_model_selector().await;
                true
            }
            "/usage" => {
                self.open_usage_dashboard().await;
                self.mode = AppMode::UsageDashboard;
                true
            }
            s if s.starts_with("/onboard") || s == "/doctor" => {
                use crate::tui::onboarding::OnboardingStep;
                let suffix = if s == "/doctor" {
                    "health"
                } else {
                    s.strip_prefix("/onboard")
                        .unwrap_or("")
                        .trim_start_matches(':')
                };
                let step = match suffix {
                    "provider" => OnboardingStep::ProviderAuth,
                    "workspace" => OnboardingStep::Workspace,
                    "channels" => OnboardingStep::Channels,
                    "voice" => OnboardingStep::VoiceSetup,
                    "image" => OnboardingStep::ImageSetup,
                    "daemon" => OnboardingStep::Daemon,
                    "health" => OnboardingStep::HealthCheck,
                    "brain" => OnboardingStep::BrainSetup,
                    _ => OnboardingStep::ModeSelect,
                };
                let config = match crate::config::Config::load() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to load config for onboarding: {}", e);
                        self.push_system_message(format!(
                            "⚠️ Could not load config: {}. Cannot open onboarding.",
                            e
                        ));
                        return false;
                    }
                };
                let mut wizard = OnboardingWizard::from_config(&config);
                wizard.step = step;
                // Deep-link to a specific step: lock to that step only
                // (no progress dots, no navigation, Enter/Esc exit to chat)
                // Only bare /onboard runs the full wizard flow
                if step != OnboardingStep::ModeSelect {
                    wizard.quick_jump = true;
                }
                if step == OnboardingStep::HealthCheck {
                    wizard.start_health_check();
                }
                if step == OnboardingStep::ImageSetup {
                    wizard.detect_existing_image_key();
                }
                self.onboarding = Some(wizard);
                self.mode = AppMode::Onboarding;
                true
            }
            "/new" => {
                let _ = self.event_sender().send(TuiEvent::NewSession);
                true
            }
            "/sessions" => {
                self.mode = AppMode::Sessions;
                let _ = self
                    .event_sender()
                    .send(TuiEvent::SwitchMode(AppMode::Sessions));
                true
            }
            "/approve" => {
                self.messages.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "system".to_string(),
                    content: String::new(),
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: Some(ApproveMenu {
                        selected_option: 0,
                        state: ApproveMenuState::Pending,
                    }),
                    details: None,
                    expanded: false,
                    tool_group: None,
                });
                self.scroll_offset = 0;
                true
            }
            "/compact" => {
                let pct = self.context_usage_percent();
                self.push_system_message(format!(
                    "Compacting context... (currently at {:.0}%)",
                    pct
                ));
                // Trigger compaction by sending a special message to the agent
                let sender = self.event_sender();
                let _ = sender.send(TuiEvent::MessageSubmitted(
                    "[SYSTEM: Compact context now. Summarize this conversation for continuity.]"
                        .to_string(),
                ));
                true
            }
            "/rebuild" => {
                self.push_system_message(
                    "🔨 Building from source... (streaming output below)".to_string(),
                );
                let sender = self.event_sender();
                let sid = self
                    .current_session
                    .as_ref()
                    .map(|s| s.id)
                    .unwrap_or(Uuid::nil());
                tokio::spawn(async move {
                    match SelfUpdater::auto_detect() {
                        Ok(updater) => {
                            let root = updater.project_root().display().to_string();
                            let _ = sender.send(TuiEvent::SystemMessage(format!("📁 {}", root)));
                            let tx = sender.clone();
                            match updater
                                .build_streaming(move |line| {
                                    // Filter to only meaningful cargo lines
                                    let trimmed = line.trim();
                                    if trimmed.starts_with("Compiling")
                                        || trimmed.starts_with("Finished")
                                        || trimmed.starts_with("error")
                                        || trimmed.starts_with("warning[")
                                        || trimmed.starts_with("-->")
                                    {
                                        let _ = tx.send(TuiEvent::BuildLine(line));
                                    }
                                })
                                .await
                            {
                                Ok(_) => {
                                    let _ = sender
                                        .send(TuiEvent::RestartReady("✅ Build complete".into()));
                                }
                                Err(e) => {
                                    let _ = sender.send(TuiEvent::Error {
                                        session_id: sid,
                                        message: format!("Build failed: {}", e),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let _ = sender.send(TuiEvent::Error {
                                session_id: sid,
                                message: format!("Cannot detect project: {}", e),
                            });
                        }
                    }
                });
                true
            }
            "/evolve" => {
                self.push_system_message("Checking for updates...".to_string());
                let sender = self.event_sender();
                let sid = self
                    .current_session
                    .as_ref()
                    .map(|s| s.id)
                    .unwrap_or(Uuid::nil());
                tokio::spawn(async move {
                    run_evolve_directly(sid, sender).await;
                });
                true
            }
            "/whisper" => {
                self.push_system_message("Setting up WhisperCrabs...".to_string());
                let sender = self.event_sender();
                let sid = self
                    .current_session
                    .as_ref()
                    .map(|s| s.id)
                    .unwrap_or(Uuid::nil());
                tokio::spawn(async move {
                    match ensure_whispercrabs().await {
                        Ok(binary_path) => {
                            // Launch the binary (GTK handles if already running)
                            match tokio::process::Command::new(&binary_path)
                                .stdin(std::process::Stdio::null())
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .spawn()
                            {
                                Ok(_) => {
                                    let _ = sender.send(TuiEvent::SystemMessage(
                                        "WhisperCrabs is running! A floating mic button is now on your screen.\n\n\
                                         Speak from any app — transcription is auto-copied to your clipboard. Just paste wherever you need.\n\n\
                                         To change settings, right-click the button or just ask me here.".to_string()
                                    ));
                                }
                                Err(e) => {
                                    let _ = sender.send(TuiEvent::Error {
                                        session_id: sid,
                                        message: format!("Failed to launch WhisperCrabs: {}", e),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let _ = sender.send(TuiEvent::Error {
                                session_id: sid,
                                message: format!("WhisperCrabs setup failed: {}", e),
                            });
                        }
                    }
                });
                true
            }
            "/help" => {
                self.mode = AppMode::Help;
                true
            }
            "/cd" => {
                let _ = self.open_directory_picker().await;
                true
            }
            _ if input.starts_with('/') => {
                // Check user-defined commands
                if let Some(user_cmd) = self.user_commands.iter().find(|c| c.name == cmd) {
                    let prompt = user_cmd.prompt.clone();
                    let action = user_cmd.action.clone();
                    match action.as_str() {
                        "system" => {
                            self.push_system_message(prompt);
                        }
                        _ => {
                            // "prompt" action — send to LLM
                            let sender = self.event_sender();
                            let _ = sender.send(TuiEvent::MessageSubmitted(prompt));
                        }
                    }
                    return true;
                }
                self.push_system_message(format!(
                    "Unknown command: {}. Type /help for available commands.",
                    cmd
                ));
                true
            }
            _ => false,
        }
    }

    /// Format a human-readable description of a tool call from its name and input
    /// Case-insensitive key lookup on a JSON object.
    /// Handles camelCase, snake_case, or whatever the model sends.
    fn get_input_ci<'a>(input: &'a Value, key: &str) -> Option<&'a Value> {
        input.get(key).or_else(|| {
            let lower = key.to_lowercase();
            input
                .as_object()
                .and_then(|obj| obj.iter().find(|(k, _)| k.to_lowercase() == lower))
                .map(|(_, v)| v)
        })
    }

    pub fn format_tool_description(tool_name: &str, tool_input: &Value) -> String {
        let ci = Self::get_input_ci;
        match tool_name {
            "bash" => {
                let cmd = ci(tool_input, "command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("bash: {}", cmd)
            }
            "read_file" | "read" => {
                let path = ci(tool_input, "path")
                    .or_else(|| ci(tool_input, "file_path"))
                    .or_else(|| ci(tool_input, "filePath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Read {}", path)
            }
            "write_file" | "write" => {
                let path = ci(tool_input, "path")
                    .or_else(|| ci(tool_input, "file_path"))
                    .or_else(|| ci(tool_input, "filePath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Write {}", path)
            }
            "edit_file" | "edit" => {
                let path = ci(tool_input, "path")
                    .or_else(|| ci(tool_input, "file_path"))
                    .or_else(|| ci(tool_input, "filePath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Edit {}", path)
            }
            "ls" => {
                let path = ci(tool_input, "path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                format!("ls {}", path)
            }
            "glob" => {
                let pattern = ci(tool_input, "pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Glob {}", pattern)
            }
            "grep" => {
                let pattern = ci(tool_input, "pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let path = ci(tool_input, "path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if path.is_empty() {
                    format!("Grep '{}'", pattern)
                } else {
                    format!("Grep '{}' in {}", pattern, path)
                }
            }
            "lsp" => {
                let op = ci(tool_input, "operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("LSP {}", op)
            }
            "web_search" => {
                let query = ci(tool_input, "query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Search: {}", query)
            }
            "exa_search" => {
                let query = ci(tool_input, "query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("EXA search: {}", query)
            }
            "brave_search" => {
                let query = ci(tool_input, "query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Brave search: {}", query)
            }
            "http_request" => {
                let url = ci(tool_input, "url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let method = ci(tool_input, "method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET");
                format!("{} {}", method, url)
            }
            "execute_code" => {
                let lang = ci(tool_input, "language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let code = ci(tool_input, "code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if code.is_empty() {
                    format!("Execute {}", lang)
                } else {
                    // Show first line of code for context
                    let first_line = code.lines().next().unwrap_or(code);
                    let truncated = if first_line.len() > 80 {
                        format!("{}…", &first_line[..80])
                    } else {
                        first_line.to_string()
                    };
                    format!("{}: {}", lang, truncated)
                }
            }
            "notebook_edit" => {
                let path = ci(tool_input, "notebook_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Notebook {}", path)
            }
            "parse_document" => {
                let path = ci(tool_input, "path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Parse {}", path)
            }
            "task_manager" => {
                let op = ci(tool_input, "operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Task: {}", op)
            }
            "plan" => {
                let op = ci(tool_input, "operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                match op {
                    "create" => {
                        let name = ci(tool_input, "title")
                            .or_else(|| ci(tool_input, "name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("plan");
                        format!("Plan: create '{}'", name)
                    }
                    "add_task" => {
                        let title = ci(tool_input, "title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("task");
                        format!("Plan: add task '{}'", title)
                    }
                    "finalize" => "Plan: finalize — awaiting approval".to_string(),
                    "start_task" => {
                        let id = ci(tool_input, "task_id")
                            .and_then(|v| v.as_u64())
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        format!("Plan: start task #{}", id)
                    }
                    "complete_task" => {
                        let id = ci(tool_input, "task_id")
                            .and_then(|v| v.as_u64())
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        format!("Plan: complete task #{}", id)
                    }
                    "update_task" => {
                        let id = ci(tool_input, "task_id")
                            .and_then(|v| v.as_u64())
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        format!("Plan: update task #{}", id)
                    }
                    "summary" => "Plan: summary".to_string(),
                    "get_status" => "Plan: status".to_string(),
                    _ => format!("Plan: {}", op),
                }
            }
            "session_context" => "Session context".to_string(),
            "Agent" | "agent" => {
                let desc = tool_input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent task");
                format!("Agent: {}", desc)
            }
            other => other.to_string(),
        }
    }

    /// Expand a DB message into one or more DisplayMessages.
    /// Assistant messages may contain tool markers that get reconstructed into ToolCallGroup display messages.
    /// Find the byte length of a balanced JSON array starting at `s[0] == '['`.
    /// Tracks string and escape state so `-->` or `]` tokens inside string
    /// values don't terminate the scan prematurely (e.g. cargo/rustc
    /// diagnostics like `--> src/main.rs:42` embedded in a tool-call output).
    /// Returns `None` if the input doesn't start with `[` or is unbalanced.
    fn find_balanced_json_end(s: &str) -> Option<usize> {
        let bytes = s.as_bytes();
        if bytes.first() != Some(&b'[') {
            return None;
        }
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;
        for (idx, &b) in bytes.iter().enumerate() {
            if escape {
                escape = false;
                continue;
            }
            if in_string {
                match b {
                    b'\\' => escape = true,
                    b'"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Supports both v1 (`<!-- tools: desc1 | desc2 -->`) and v2 (`<!-- tools-v2: [JSON] -->`) formats.
    /// Extract reasoning blocks from text. Handles:
    /// - `<!-- reasoning -->...<!-- /reasoning -->`
    /// - `<antThinking>...</antThinking>` (Qwen, case-insensitive)
    /// - `<think>...</think>` (DeepSeek)
    ///
    /// Returns (reasoning_text, remaining_text_without_markers).
    fn extract_reasoning(text: &str) -> (Option<String>, String) {
        let mut reasoning_parts = Vec::new();
        let mut remaining = text.to_string();

        // Extract <!-- reasoning --> blocks
        let open_tag = "<!-- reasoning -->";
        let close_tag = "<!-- /reasoning -->";
        while let Some(start) = remaining.find(open_tag) {
            let before = remaining[..start].to_string();
            let after_tag = &remaining[start + open_tag.len()..];
            if let Some(end) = after_tag.find(close_tag) {
                let part = after_tag[..end].trim();
                if !part.is_empty() {
                    reasoning_parts.push(part.to_string());
                }
                remaining = format!("{}{}", before, &after_tag[end + close_tag.len()..]);
            } else {
                let part = after_tag.trim();
                if !part.is_empty() {
                    reasoning_parts.push(part.to_string());
                }
                remaining = before;
                break;
            }
        }

        // Extract <antThinking> blocks (case-insensitive, handles <anTthinking> typo)
        remaining =
            Self::extract_tag_case_insensitive(&remaining, "antthinking", &mut reasoning_parts);

        // Extract <think> blocks (DeepSeek)
        remaining = Self::extract_tag_case_insensitive(&remaining, "think", &mut reasoning_parts);

        // Extract self-closing tags: <think/>, <antThinking/>, etc.
        // These appear when the model emits thinking with no content or as a flush marker.
        for tag_name in ["think", "antthinking"] {
            let mut cleaned = String::new();
            let mut rest = remaining.as_str();
            let open_self = format!("<{}", tag_name);
            loop {
                let rest_lower = rest.to_lowercase();
                if let Some(start) = rest_lower.find(&open_self) {
                    cleaned.push_str(&rest[..start]);
                    let after = &rest[start + open_self.len()..];
                    // Find the closing />
                    if let Some(end) = after.find("/>") {
                        let inner = after[..end].trim();
                        // If the self-closing tag has content attributes, capture them
                        if !inner.is_empty() && !inner.starts_with('/') {
                            reasoning_parts.push(inner.to_string());
                        }
                        rest = &after[end + 2..];
                    } else {
                        // Malformed — keep the rest as-is
                        cleaned.push_str(&rest[start..]);
                        rest = "";
                        break;
                    }
                } else {
                    break;
                }
            }
            cleaned.push_str(rest);
            remaining = cleaned;
        }

        // Strip orphan closing tags (</think>, </antThinking>) that leaked
        // when a stream error dropped mid-thinking-block. These have no
        // matching open tag so extract_tag_case_insensitive never catches them.
        for tag_name in ["think", "antthinking"] {
            let close = format!("</{}>", tag_name);
            let mut cleaned = String::new();
            let mut rest = remaining.as_str();
            loop {
                let rest_lower = rest.to_lowercase();
                if let Some(start) = rest_lower.find(&close) {
                    cleaned.push_str(&rest[..start]);
                    rest = &rest[start + close.len()..];
                } else {
                    break;
                }
            }
            cleaned.push_str(rest);
            remaining = cleaned;
        }

        let remaining = remaining.trim().to_string();
        if reasoning_parts.is_empty() {
            (None, remaining)
        } else {
            (Some(reasoning_parts.join("\n\n")), remaining)
        }
    }

    /// Extract content between <tag>...</tag> pairs, case-insensitive.
    fn extract_tag_case_insensitive(
        text: &str,
        tag_name: &str,
        reasoning_parts: &mut Vec<String>,
    ) -> String {
        let mut result = text.to_string();
        loop {
            let lower = result.to_lowercase();
            let open = format!("<{}>", tag_name);
            let close = format!("</{}>", tag_name);
            let start_opt = lower.find(&open);
            if let Some(start) = start_opt {
                let before = result[..start].to_string();
                let after_open = &result[start + open.len()..];
                let after_open_lower = &lower[start + open.len()..];
                if let Some(end) = after_open_lower.find(&close) {
                    let part = after_open[..end].trim();
                    if !part.is_empty() {
                        reasoning_parts.push(part.to_string());
                    }
                    result = format!("{}{}", before, &after_open[end + close.len()..]);
                } else {
                    let part = after_open.trim();
                    if !part.is_empty() {
                        reasoning_parts.push(part.to_string());
                    }
                    result = before;
                    break;
                }
            } else {
                break;
            }
        }
        result
    }

    fn expand_message(msg: crate::db::models::Message) -> Vec<DisplayMessage> {
        // Compaction markers are stored as a user message containing the full
        // structured summary the LLM uses to recover context. They must stay
        // fully transparent in the TUI — don't render them at all on reload.
        if msg.content.starts_with("[CONTEXT COMPACTION") {
            return vec![];
        }

        let content_lower = msg.content.to_lowercase();
        let has_db_thinking = msg.thinking.as_ref().is_some_and(|t| !t.trim().is_empty());
        let has_tool_call_tag =
            msg.content.contains("\u{fe0f}\u{20e3}") || msg.content.contains("<tool_call>");
        if msg.role != "assistant"
            || (!msg.content.contains("<!-- tools")
                && !msg.content.contains("<!-- reasoning -->")
                && !content_lower.contains("<antthinking")
                && !content_lower.contains("<think")
                && !has_db_thinking
                && !has_tool_call_tag)
        {
            return vec![DisplayMessage::from(msg)];
        }

        // Extract owned values before borrowing content
        let id = msg.id;
        let timestamp = msg.created_at;
        let token_count = msg.token_count;
        let cost = msg.cost;
        let content = msg.content;
        let db_thinking = msg.thinking;

        let mut result = Vec::new();

        // Find the next tool marker (either v1 or v2)
        fn find_next_marker(s: &str) -> Option<(usize, bool)> {
            let v2_pos = s.find("<!-- tools-v2:");
            let v1_pos = s.find("<!-- tools:");
            match (v2_pos, v1_pos) {
                (Some(v2), Some(v1)) => {
                    if v2 <= v1 {
                        Some((v2, true))
                    } else {
                        Some((v1, false))
                    }
                }
                (Some(v2), None) => Some((v2, true)),
                (None, Some(v1)) => Some((v1, false)),
                (None, None) => None,
            }
        }

        let mut remaining = content.as_str();
        let mut first_text = true;
        while let Some((marker_start, is_v2)) = find_next_marker(remaining) {
            // Text before marker
            let text_before = remaining[..marker_start].trim();
            if !text_before.is_empty() {
                let (reasoning, clean_text) = Self::extract_reasoning(text_before);
                if !clean_text.is_empty() {
                    result.push(DisplayMessage {
                        id: if first_text { id } else { Uuid::new_v4() },
                        role: "assistant".to_string(),
                        content: clean_text,
                        timestamp,
                        token_count: if first_text { token_count } else { None },
                        cost: if first_text { cost } else { None },
                        approval: None,
                        approve_menu: None,
                        details: reasoning,
                        expanded: false,
                        tool_group: None,
                    });
                    first_text = false;
                } else if let Some(r) = reasoning {
                    // Reasoning-only block (no visible text) — attach to next text segment
                    // For now, create a placeholder so reasoning is not lost
                    result.push(DisplayMessage {
                        id: if first_text { id } else { Uuid::new_v4() },
                        role: "assistant".to_string(),
                        content: String::new(),
                        timestamp,
                        token_count: if first_text { token_count } else { None },
                        cost: if first_text { cost } else { None },
                        approval: None,
                        approve_menu: None,
                        details: Some(r),
                        expanded: false,
                        tool_group: None,
                    });
                    first_text = false;
                }
            }

            let marker_len = if is_v2 {
                "<!-- tools-v2:".len()
            } else {
                "<!-- tools:".len()
            };
            let after_marker = &remaining[marker_start + marker_len..];

            // v2 markers contain a JSON array that may include `-->` inside
            // string values (e.g. cargo/rustc diagnostics like
            // `--> src/main.rs:42`). A naive find("-->") terminates at the
            // first inner arrow, truncates the JSON, and fails to parse.
            // Use balanced JSON scanning to find the true closing `]`, then
            // look for `-->` after it.
            let (tools_str, close_end) = if is_v2 {
                let trimmed = after_marker.trim_start();
                let lead = after_marker.len() - trimmed.len();
                if let Some(array_end) = Self::find_balanced_json_end(trimmed) {
                    let tail = &trimmed[array_end..];
                    let tail_lead = tail.len() - tail.trim_start().len();
                    let post = &tail[tail_lead..];
                    if post.starts_with("-->") {
                        let end = lead + array_end + tail_lead + 3;
                        (after_marker[..end - 3].trim(), end)
                    } else {
                        // No closing `-->` after balanced array — malformed
                        remaining = after_marker;
                        break;
                    }
                } else {
                    // No balanced JSON array found — malformed
                    remaining = after_marker;
                    break;
                }
            } else if let Some(end) = after_marker.find("-->") {
                (after_marker[..end].trim(), end + 3)
            } else {
                remaining = after_marker;
                break;
            };

            let calls: Vec<ToolCallEntry> = if is_v2 {
                // v2: parse JSON array with descriptions, success, output, and tool input
                serde_json::from_str::<Vec<serde_json::Value>>(tools_str)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entry| {
                        let desc = entry["d"].as_str().unwrap_or("?").to_string();
                        let success = entry["s"].as_bool().unwrap_or(true);
                        let output = entry["o"]
                            .as_str()
                            .map(|s| s.to_string())
                            .filter(|s| !s.is_empty());
                        let tool_input = entry.get("i").cloned().unwrap_or(serde_json::Value::Null);
                        ToolCallEntry {
                            description: desc,
                            success,
                            details: output,
                            completed: true,
                            tool_input,
                        }
                    })
                    .collect()
            } else {
                // v1: plain descriptions, no output
                tools_str
                    .split(" | ")
                    .map(|desc| ToolCallEntry {
                        description: desc.to_string(),
                        success: true,
                        details: None,
                        completed: true,
                        tool_input: serde_json::Value::Null,
                    })
                    .collect()
            };

            if !calls.is_empty() {
                let count = calls.len();
                result.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "tool_group".to_string(),
                    content: format!("{} tool call{}", count, if count == 1 { "" } else { "s" }),
                    timestamp,
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: None,
                    details: None,
                    expanded: false,
                    tool_group: Some(ToolCallGroup {
                        calls,
                        expanded: false,
                    }),
                });
            }
            remaining = &after_marker[close_end..];
        }

        // Any remaining text after the last marker
        let trailing = remaining.trim();
        if !trailing.is_empty() {
            let (reasoning, clean_text) = Self::extract_reasoning(trailing);
            if !clean_text.is_empty() {
                result.push(DisplayMessage {
                    id: if first_text { id } else { Uuid::new_v4() },
                    role: "assistant".to_string(),
                    content: clean_text,
                    timestamp,
                    token_count: if first_text { token_count } else { None },
                    cost: if first_text { cost } else { None },
                    approval: None,
                    approve_menu: None,
                    details: reasoning,
                    expanded: false,
                    tool_group: None,
                });
            } else if let Some(r) = reasoning {
                result.push(DisplayMessage {
                    id: if first_text { id } else { Uuid::new_v4() },
                    role: "assistant".to_string(),
                    content: String::new(),
                    timestamp,
                    token_count: if first_text { token_count } else { None },
                    cost: if first_text { cost } else { None },
                    approval: None,
                    approve_menu: None,
                    details: Some(r),
                    expanded: false,
                    tool_group: None,
                });
            }
        }

        if result.is_empty() {
            // Content was only tool markers with no text — show a placeholder
            result.push(DisplayMessage {
                id,
                role: "assistant".to_string(),
                content: String::new(),
                timestamp,
                token_count,
                cost,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
            });
        }

        // Apply thinking from DB `thinking` column (non-CLI providers).
        // If no DisplayMessage has `details` set yet but we have persisted
        // thinking, attach it to the first text-based assistant message.
        if let Some(ref thinking) = db_thinking
            && !thinking.trim().is_empty()
        {
            let has_details = result.iter().any(|dm| dm.details.is_some());
            if !has_details {
                // Find the first assistant text message to attach thinking to
                if let Some(first_text) = result
                    .iter_mut()
                    .find(|dm| dm.role == "assistant" && !dm.content.is_empty())
                {
                    first_text.details = Some(thinking.clone());
                } else if let Some(first) = result.first_mut() {
                    // Even if it's a tool-only message, attach thinking
                    first.details = Some(thinking.clone());
                }
            }
        }

        result
    }

    /// Extract image file paths from text and return (remaining_text, attachments).
    /// Handles paths with spaces (e.g. `/home/user/My Screenshots/photo.png`)
    /// and image URLs.
    ///
    /// Strip terminal escape sequences (CSI, SGR mouse, OSC, etc.) from text.
    ///
    /// These leak into paste events when switching terminal focus while mouse
    /// capture is active (e.g. tmux pane switch, alt-tab with iTerm2).
    pub(crate) fn strip_terminal_escapes(text: &str) -> String {
        // Fast path: no ESC byte means nothing to strip
        if !text.as_bytes().contains(&0x1b) {
            return text.to_string();
        }

        let mut out = String::with_capacity(text.len());
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            if bytes[i] == 0x1b {
                // ESC — start of escape sequence (always single-byte ASCII)
                i += 1;
                if i >= len {
                    break;
                }
                match bytes[i] {
                    b'[' => {
                        // CSI sequence: ESC [ ... (final byte in 0x40-0x7E)
                        i += 1;
                        while i < len && !(0x40..=0x7E).contains(&bytes[i]) {
                            i += 1;
                        }
                        if i < len {
                            i += 1; // skip the final byte
                        }
                    }
                    b']' => {
                        // OSC sequence: ESC ] ... ST (ST = ESC \ or BEL 0x07)
                        i += 1;
                        while i < len {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    _ => {
                        // Two-byte escape (e.g. ESC =, ESC >)
                        i += 1;
                    }
                }
            } else {
                // Normal byte — figure out how many bytes this UTF-8 char is
                // and copy the whole character to preserve multi-byte (emoji, CJK, etc.)
                let ch_len = utf8_char_len(bytes[i]);
                if i + ch_len <= len {
                    // SAFETY: the input is a valid &str so this slice is a valid char
                    out.push_str(&text[i..i + ch_len]);
                }
                i += ch_len;
            }
        }
        out
    }

    /// Normalize invisible/zero-width Unicode characters that web pages use
    /// for table formatting. These paste as invisible bytes in terminals,
    /// gluing columns together.
    pub(crate) fn normalize_unicode_whitespace(text: &str) -> String {
        text.chars()
            .map(|ch| match ch {
                // Non-breaking spaces (common in HTML tables)
                '\u{00A0}' | '\u{202F}' | '\u{2007}' => ' ',
                // Zero-width spaces / joiners — remove entirely
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' => '\0',
                // Various width spaces — normalize to regular space
                '\u{2000}'..='\u{200A}' | '\u{205F}' | '\u{3000}' => ' ',
                // Soft hyphen — remove
                '\u{00AD}' => '\0',
                _ => ch,
            })
            .filter(|&ch| ch != '\0')
            .collect()
    }

    /// Expand tab characters to spaces (8 spaces per tab stop — standard terminal width).
    /// Terminal TUIs can't render tab stops, so pasted table data with `\t`
    /// collapses to zero-width, gluing columns together. This preserves visual
    /// alignment from web pages and spreadsheets.
    pub(crate) fn expand_tabs(text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut col = 0usize;
        for ch in text.chars() {
            if ch == '\t' {
                // Advance to next tab stop (every 8 columns — standard terminal)
                let spaces = 8 - (col % 8);
                for _ in 0..spaces {
                    result.push(' ');
                }
                col += spaces;
            } else {
                result.push(ch);
                if ch == '\n' {
                    col = 0;
                } else {
                    col += 1;
                }
            }
        }
        result
    }

    /// Text file paths (`.txt`, `.md`, `.json`, source code, etc.) are read from
    /// disk and inlined into the returned text — no attachment needed.
    pub(crate) fn extract_image_paths(text: &str) -> (String, Vec<ImageAttachment>) {
        let trimmed = text.trim();
        let lower = trimmed.to_lowercase();

        // Case 1: Entire pasted text is a single image path (handles spaces in path)
        if IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
            // Local path
            let path = std::path::Path::new(trimmed);
            if path.exists() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| trimmed.to_string());
                return (
                    String::new(),
                    vec![ImageAttachment {
                        name,
                        path: trimmed.to_string(),
                    }],
                );
            }
            // URL (no spaces — just check prefix)
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                let name = trimmed.rsplit('/').next().unwrap_or(trimmed).to_string();
                return (
                    String::new(),
                    vec![ImageAttachment {
                        name,
                        path: trimmed.to_string(),
                    }],
                );
            }
        }

        // Case 1b: Entire pasted text is a single text file path (handles spaces in path)
        if TEXT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
            let path = std::path::Path::new(trimmed);
            if path.exists() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| trimmed.to_string());
                if let Ok(content) = std::fs::read_to_string(path) {
                    const LIMIT: usize = 8_000;
                    let truncated = if content.len() > LIMIT {
                        let safe: String = content.chars().take(LIMIT).collect();
                        format!("{}…[truncated]", safe)
                    } else {
                        content
                    };
                    return (format!("[File: {}]\n```\n{}\n```", name, truncated), vec![]);
                }
            }
        }

        // Case 2: Mixed text — scan for image URLs (split by whitespace is fine for URLs)
        // and absolute paths without spaces
        let mut attachments = Vec::new();
        let mut remaining_parts = Vec::new();
        let mut inlined_files: Vec<String> = Vec::new();

        for word in text.split_whitespace() {
            let word_lower = word.to_lowercase();
            let is_image = IMAGE_EXTENSIONS.iter().any(|ext| word_lower.ends_with(ext));

            if is_image {
                let path = std::path::Path::new(word);
                if path.exists() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| word.to_string());
                    attachments.push(ImageAttachment {
                        name,
                        path: word.to_string(),
                    });
                    continue;
                }
                if word.starts_with("http://") || word.starts_with("https://") {
                    let name = word.rsplit('/').next().unwrap_or(word).to_string();
                    attachments.push(ImageAttachment {
                        name,
                        path: word.to_string(),
                    });
                    continue;
                }
            }

            // Check for text file paths (space-free paths only in mixed-text mode)
            let is_text = TEXT_EXTENSIONS.iter().any(|ext| word_lower.ends_with(ext));
            if is_text {
                let path = std::path::Path::new(word);
                if path.exists()
                    && let Ok(content) = std::fs::read_to_string(path)
                {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| word.to_string());
                    const LIMIT: usize = 8_000;
                    let truncated = if content.len() > LIMIT {
                        let safe: String = content.chars().take(LIMIT).collect();
                        format!("{}…[truncated]", safe)
                    } else {
                        content
                    };
                    inlined_files.push(format!("[File: {}]\n```\n{}\n```", name, truncated));
                    continue;
                }
            }

            remaining_parts.push(word);
        }

        let mut result = remaining_parts.join(" ");
        for file_content in inlined_files {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str(&file_content);
        }
        (result, attachments)
    }

    /// Replace `<<IMG:/path/to/file.png>>` markers with readable `[IMG: file.png]` for display.
    pub(crate) fn humanize_image_markers(text: &str) -> String {
        let mut result = text.to_string();
        while let Some(start) = result.find("<<IMG:") {
            if let Some(end) = result[start..].find(">>") {
                let path = &result[start + 6..start + end];
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string());
                let replacement = format!("[IMG: {}]", name);
                result = format!(
                    "{}{}{}",
                    &result[..start],
                    replacement,
                    &result[start + end + 2..]
                );
            } else {
                break;
            }
        }
        result.trim().to_string()
    }

    /// Push a system message into the chat display
    pub(crate) fn push_system_message(&mut self, content: String) {
        self.messages.push(DisplayMessage {
            id: Uuid::new_v4(),
            role: "system".to_string(),
            content,
            timestamp: chrono::Utc::now(),
            token_count: None,
            cost: None,
            approval: None,
            approve_menu: None,
            details: None,
            expanded: false,
            tool_group: None,
        });
        self.scroll_offset = 0;
    }

    /// Send a message to the agent
    pub(crate) async fn send_message(&mut self, content: String) -> Result<()> {
        tracing::info!(
            "[send_message] START is_processing={} has_session={} content_len={}",
            self.is_processing,
            self.current_session.is_some(),
            content.len()
        );

        // On every new user message, decide what to do with any in-memory plan:
        //
        // Plan file lifecycle on user message:
        // • Terminal (Completed/Rejected/Cancelled): delete file + clear memory.
        // • InProgress: plan is actively executing — keep file and widget.
        // • Everything else (Draft/PendingApproval/Approved): user moved on.
        //   Clear widget from memory but keep file on disk so the agent tool can
        //   still read/write it if the exchange continues.
        if let Some(ref plan) = self.plan_document {
            use crate::tui::plan::PlanStatus;
            match plan.status {
                PlanStatus::Completed | PlanStatus::Rejected | PlanStatus::Cancelled => {
                    self.discard_plan_file();
                    self.plan_document = None;
                }
                PlanStatus::InProgress => {
                    // Actively executing — leave widget showing
                }
                _ => {
                    self.plan_document = None;
                }
            }
        }

        // Deny stale pending approvals so they don't block streaming
        let stale_count = self
            .messages
            .iter()
            .filter(|m| {
                m.approval
                    .as_ref()
                    .is_some_and(|a| a.state == ApprovalState::Pending)
            })
            .count();
        if stale_count > 0 {
            tracing::warn!(
                "[send_message] Clearing {} stale pending approvals",
                stale_count
            );
        }
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval
                && approval.state == ApprovalState::Pending
            {
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Superseded".to_string()),
                });
                approval.state = ApprovalState::Denied("Superseded".to_string());
            }
        }

        if let Some(session) = &self.current_session
            && self.processing_sessions.contains(&session.id)
        {
            tracing::warn!(
                "[send_message] QUEUED — session {} still processing",
                session.id
            );

            // Stack queued messages — multiple sends accumulate.
            if let Some(sid) = self.current_session.as_ref().map(|s| s.id) {
                self.queued_messages
                    .entry(sid)
                    .or_default()
                    .push(content.clone());
            }
            self.cursor_position = self.input_buffer.len();

            // Queue for injection between tool calls — join all stacked messages
            {
                let mut lock = self.message_queue.lock().await;
                match lock.as_mut() {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(&content);
                    }
                    None => *lock = Some(content),
                }
            }
            return Ok(());
        }
        if let Some(session) = &self.current_session {
            self.processing_sessions.insert(session.id);
            self.is_processing = true;
            self.processing_started_at = Some(std::time::Instant::now());
            self.streaming_output_tokens = 0;
            self.error_message = None;
            self.error_message_shown_at = None;
            self.intermediate_text_received = false;

            // Drain pending context hints (model changes, /cd, etc.) and prepend to message
            let mut transformed_content = content.clone();
            if !self.pending_context.is_empty() {
                let context = self
                    .pending_context
                    .drain(..)
                    .collect::<Vec<_>>()
                    .join("\n");
                transformed_content = format!("{}\n\n{}", context, transformed_content);
            }

            // Analyze and transform the prompt before sending to agent
            let transformed_content = self
                .prompt_analyzer
                .analyze_and_transform(&transformed_content);

            // Log if the prompt was transformed
            if transformed_content != content {
                tracing::info!("✨ Prompt transformed with tool hints");
            }

            // Add user message to UI — skip internal system triggers (e.g. /compact)
            let is_system_trigger = content.starts_with("[SYSTEM:");
            if !is_system_trigger {
                let display_content = Self::humanize_image_markers(&content);
                let user_msg = DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "user".to_string(),
                    content: display_content,
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: None,
                    details: None,
                    expanded: false,
                    tool_group: None,
                };
                self.messages.push(user_msg);
            }

            // Auto-scroll to show the new user message and re-enable auto-scroll
            self.auto_scroll = true;
            self.scroll_offset = 0;

            // Create cancellation token for this request
            let token = CancellationToken::new();
            self.cancel_token = Some(token.clone());

            // Send transformed content to agent in background
            let agent_service = self.agent_service.clone();
            let session_id = session.id;
            let event_sender = self.event_sender();

            tracing::info!(
                "[send_message] Spawning agent task for session {}",
                session_id
            );
            let abort_event_sender = event_sender.clone();
            let handle = tokio::spawn(async move {
                tracing::info!("[agent_task] START calling send_message_with_tools_and_mode");
                let result = agent_service
                    .send_message_with_tools_and_mode(
                        session_id,
                        transformed_content,
                        None,
                        Some(token),
                    )
                    .await;

                match result {
                    Ok(response) => {
                        tracing::info!("[agent_task] OK — sending ResponseComplete");
                        if let Err(e) = event_sender.send(TuiEvent::ResponseComplete {
                            session_id,
                            response,
                        }) {
                            tracing::error!("[agent_task] FAILED to send ResponseComplete: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("[agent_task] ERROR: {}", e);
                        // Translate cryptic provider errors into human-readable messages
                        let raw = e.to_string();
                        let user_message = if raw.contains("error decoding response body") {
                            "Provider stream broke unexpectedly (connection dropped mid-response). \
                             This can happen when the provider hits a token limit or network issue. \
                             Try again or switch to a different model.".to_string()
                        } else if raw.contains("Repetition detected") {
                            "Provider got stuck in a loop repeating the same content. \
                             The stream was terminated automatically. Try rephrasing your request \
                             or switching models."
                                .to_string()
                        } else {
                            raw
                        };
                        if let Err(e2) = event_sender.send(TuiEvent::Error {
                            session_id,
                            message: user_message,
                        }) {
                            tracing::error!("[agent_task] FAILED to send Error event: {}", e2);
                        }
                    }
                }
            });
            // Store abort handle so double-Escape can hard-kill the task
            self.task_abort_handle = Some(handle.abort_handle());

            // Watch for panics — surface them in the UI instead of silent hang
            tokio::spawn(async move {
                match handle.await {
                    Err(e) if e.is_panic() => {
                        tracing::error!("[agent_task] PANICKED: {}", e);
                        let _ = abort_event_sender.send(TuiEvent::Error {
                            session_id,
                            message: format!(
                                "Agent task crashed unexpectedly: {e}. You can continue chatting."
                            ),
                        });
                    }
                    // Cancelled or completed — no action needed
                    _ => {}
                }
            });
        }

        Ok(())
    }

    /// Append a streaming chunk
    pub(crate) fn append_streaming_chunk(&mut self, chunk: String) {
        if let Some(ref mut response) = self.streaming_response {
            response.push_str(&chunk);
        } else {
            self.streaming_response = Some(chunk);
            // Auto-scroll when response starts streaming (only if user hasn't scrolled up)
            if self.auto_scroll {
                self.scroll_offset = 0;
            }
        }
    }

    /// Complete the streaming response
    pub(crate) async fn complete_response(
        &mut self,
        response: crate::brain::agent::AgentResponse,
    ) -> Result<()> {
        if let Some(ref session) = self.current_session {
            self.processing_sessions.remove(&session.id);
            self.session_cancel_tokens.remove(&session.id);
        }
        self.is_processing = false;
        self.processing_started_at = None;
        tracing::debug!(
            "[TUI] complete_response: clearing streaming_response (was {} chars), intermediate_text_received={}",
            self.streaming_response
                .as_ref()
                .map(|s| s.len())
                .unwrap_or(0),
            self.intermediate_text_received
        );
        self.streaming_response = None;
        self.streaming_output_tokens = 0;
        let reasoning_details = self.streaming_reasoning.take();
        self.cancel_token = None;
        self.task_abort_handle = None;
        self.escape_pending_at = None; // Reset so abort hint doesn't leak to input clear

        // Clean up stale pending approvals — send deny so agent callbacks don't hang
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval
                && approval.state == ApprovalState::Pending
            {
                tracing::warn!(
                    "Cleaning up stale pending approval for tool '{}'",
                    approval.tool_name
                );
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Agent completed without resolution".to_string()),
                });
                approval.state =
                    ApprovalState::Denied("Agent completed without resolution".to_string());
            }
        }

        // Finalize active tool group as a quick_jump message BEFORE the response.
        // Matches DB reload order from expand_message.
        if let Some(group) = self.active_tool_group.take() {
            let count = group.calls.len();
            self.messages.push(DisplayMessage {
                id: Uuid::new_v4(),
                role: "tool_group".to_string(),
                content: format!("{} tool call{}", count, if count == 1 { "" } else { "s" }),
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: Some(group),
            });
        }

        // Clear any unconsumed queued message (tool loop may have already drained it)
        if self.message_queue.lock().await.take().is_some() {
            tracing::info!("[TUI] Discarding unconsumed queued message at response complete");
        }
        if let Some(sid) = self.current_session.as_ref().map(|s| s.id) {
            self.queued_messages.remove(&sid);
        }

        // Reload user commands (agent may have written new ones to commands.json)
        self.reload_user_commands();

        // Track context usage from latest response. Write to the
        // per-session cache first; only mirror into the focused-view
        // fields if THIS response belongs to the focused session,
        // otherwise a background turn's completion would overwrite the
        // pane the user is actually looking at.
        if let Some(ref session) = self.current_session {
            self.session_input_tokens
                .insert(session.id, response.context_tokens);
            if self.is_current_session(session.id) {
                self.last_input_tokens = Some(response.context_tokens);
            }
        } else {
            self.last_input_tokens = Some(response.context_tokens);
        }

        // Strip LLM artifacts (<!-- reasoning -->, </invoke>, XML tool blocks)
        // before displaying in TUI — same sanitization as Telegram/external channels.
        let mut response = response;
        response.content = crate::utils::sanitize::strip_llm_artifacts(&response.content);

        // Debug: log response content length
        tracing::debug!(
            "Response complete: content_len={}, output_tokens={}",
            response.content.len(),
            response.usage.output_tokens
        );

        if self.intermediate_text_received {
            // IntermediateText only contained text BEFORE the last tool block.
            // The full response may have trailing text AFTER the last tool.
            // Update the last intermediate assistant message with the full content
            // so trailing text is not lost, and add cost/token metadata.
            if let Some(last_assistant) = self
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == "assistant")
            {
                let old_len = last_assistant.content.len();
                // Only overwrite if the aggregated response actually has text.
                // CLI providers (qwen-cli, opencode-cli) stream their full text
                // through IntermediateText progress events and return an empty
                // `response.content` at the end — blindly assigning would wipe
                // the streamed text and make the reply vanish from the UI.
                if !response.content.is_empty() {
                    last_assistant.content = response.content.clone();
                }
                last_assistant.token_count = Some(response.usage.output_tokens as i32);
                last_assistant.cost = Some(response.cost);
                if reasoning_details.is_some() {
                    last_assistant.details = reasoning_details;
                }
                tracing::debug!(
                    "Updated intermediate assistant message: old_len={}, new_len={}",
                    old_len,
                    last_assistant.content.len()
                );
            } else {
                // No intermediate message found (shouldn't happen), add as new
                self.messages.push(DisplayMessage {
                    id: response.message_id,
                    role: "assistant".to_string(),
                    content: response.content,
                    timestamp: chrono::Utc::now(),
                    token_count: Some(response.usage.output_tokens as i32),
                    cost: Some(response.cost),
                    approval: None,
                    approve_menu: None,
                    details: reasoning_details,
                    expanded: false,
                    tool_group: None,
                });
            }
        } else {
            // Add assistant message to UI
            let assistant_msg = DisplayMessage {
                id: response.message_id,
                role: "assistant".to_string(),
                content: response.content,
                timestamp: chrono::Utc::now(),
                token_count: Some(response.usage.output_tokens as i32),
                cost: Some(response.cost),
                approval: None,
                approve_menu: None,
                details: reasoning_details,
                expanded: false,
                tool_group: None,
            };
            self.messages.push(assistant_msg);
        }

        // Update session model if not already set
        if let Some(session) = &mut self.current_session
            && session.model.is_none()
        {
            session.model = Some(response.model.clone());
            // Save the updated session to database
            if let Err(e) = self.session_service.update_session(session).await {
                tracing::warn!("Failed to update session model: {}", e);
            }
        }

        // Refresh plan widget: reload from disk, then clear if the exchange is done.
        // This catches plans that completed (status=Completed) or got stuck mid-execution
        // due to tool errors (status still InProgress but agent has moved on).
        self.reload_plan();
        if let Some(ref plan) = self.plan_document {
            use crate::tui::plan::PlanStatus;
            match plan.status {
                PlanStatus::Completed | PlanStatus::Rejected | PlanStatus::Cancelled => {
                    self.discard_plan_file();
                    self.plan_document = None;
                }
                PlanStatus::InProgress => {
                    // Agent finished responding but plan is still "in progress" —
                    // either all tasks completed (tool wrote status wrong) or a tool
                    // call failed silently. Either way, clear the stale widget.
                    let all_done = plan.tasks.iter().all(|t| {
                        matches!(
                            t.status,
                            crate::tui::plan::TaskStatus::Completed
                                | crate::tui::plan::TaskStatus::Skipped
                                | crate::tui::plan::TaskStatus::Failed
                        )
                    });
                    if all_done {
                        self.discard_plan_file();
                        self.plan_document = None;
                    }
                }
                _ => {}
            }
        }

        // Auto-scroll to bottom
        self.scroll_offset = 0;

        // Update pane message cache so inactive panes reflect latest content
        if self.pane_manager.is_split()
            && let Some(ref session) = self.current_session
        {
            self.pane_message_cache
                .insert(session.id, self.messages.clone());
        }

        Ok(())
    }

    /// Persist current in-memory streaming state to DB so cancel never loses visible content.
    ///
    /// Finds the last assistant message (created by tool_loop at start) and appends
    /// any streaming text + tool call markers that are currently displayed on screen.
    pub(crate) async fn persist_streaming_state(&self, session_id: Uuid) {
        // Build content from what's currently visible
        let mut content = String::new();

        // 1. Collect any intermediate text messages that were already added to self.messages
        //    during this response cycle (IntermediateText events create DisplayMessages).
        //    These may not have been persisted if the tool loop was aborted before it could.
        //    However, the tool loop does persist text per-iteration, so these are likely
        //    already in DB. We focus on what's NOT yet persisted:

        // 2. Active tool group (tool calls shown on screen but not yet flushed to a message)
        if let Some(ref group) = self.active_tool_group {
            let entries: Vec<serde_json::Value> = group
                .calls
                .iter()
                .map(|call| {
                    serde_json::json!({
                        "d": call.description,
                        "s": call.success,
                        "i": call.tool_input,
                    })
                })
                .collect();
            if !entries.is_empty() {
                content.push_str(&format!(
                    "\n<!-- tools-v2: {} -->\n",
                    serde_json::to_string(&entries).unwrap_or_default()
                ));
            }
        }

        // 3. Streaming response text (currently being typed out, not yet committed)
        if let Some(ref text) = self.streaming_response
            && !text.trim().is_empty()
        {
            content.push_str(text);
            content.push_str("\n\n");
        }

        if content.is_empty() {
            return;
        }

        // Find the last assistant message in this session and append
        match self.message_service.get_last_message(session_id).await {
            Ok(Some(msg)) if msg.role == "assistant" => {
                if let Err(e) = self.message_service.append_content(msg.id, &content).await {
                    tracing::error!("Failed to persist streaming state on cancel: {}", e);
                }
                tracing::info!(
                    "Persisted {} chars of streaming state to DB on cancel",
                    content.len()
                );
            }
            Ok(_) => {
                // Last message isn't assistant — create one to hold the partial content
                match self
                    .message_service
                    .create_message(session_id, "assistant".to_string(), content.clone())
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Created new assistant message with {} chars of streaming state on cancel",
                            content.len()
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to create assistant message for streaming state: {}",
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to query last message on cancel: {}", e);
            }
        }
    }
}

/// Run the evolve tool directly (no LLM involvement) and pipe progress
/// events into TUI system messages. Used by `/evolve`, the UpdatePrompt
/// dialog, and the auto-update path on startup.
pub(crate) async fn run_evolve_directly(
    session_id: Uuid,
    sender: tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) {
    use crate::brain::agent::ProgressEvent;
    use crate::brain::tools::evolve::EvolveTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use std::sync::Arc;

    // Translate ProgressEvent into TUI events so the user sees live status.
    let tx = sender.clone();
    let progress: crate::brain::agent::ProgressCallback =
        Arc::new(move |_sid, event| match event {
            ProgressEvent::IntermediateText { text, .. } => {
                let _ = tx.send(TuiEvent::SystemMessage(text));
            }
            ProgressEvent::RestartReady { status } => {
                let _ = tx.send(TuiEvent::RestartReady(status));
            }
            _ => {}
        });

    let tool = EvolveTool::new(Some(progress));
    let ctx = ToolExecutionContext::new(session_id);
    match tool.execute(serde_json::json!({}), &ctx).await {
        Ok(result) => {
            if !result.success {
                if let Some(err) = result.error {
                    let _ = sender.send(TuiEvent::SystemMessage(format!("Evolve failed: {}", err)));
                } else {
                    let _ = sender.send(TuiEvent::SystemMessage(
                        "Evolve failed (unknown error)".to_string(),
                    ));
                }
            } else if !result.output.is_empty() {
                // Already on latest, or non-restart success path — surface message.
                let _ = sender.send(TuiEvent::SystemMessage(result.output));
            }
        }
        Err(e) => {
            let _ = sender.send(TuiEvent::SystemMessage(format!("Evolve error: {}", e)));
        }
    }
}
