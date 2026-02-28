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

impl App {
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
        self.is_processing = false; // New session is never processing
        self.messages.clear();
        self.auto_scroll = true;
        self.scroll_offset = 0;
        self.mode = AppMode::Chat;
        self.approval_auto_session = false;
        self.approval_auto_always = false;

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

        self.current_session = Some(session.clone());
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
        self.approval_auto_session = false;
        self.approval_auto_always = false;

        // Sync shared session ID for channels (Telegram, WhatsApp)
        *self.shared_session_id.lock().await = Some(session.id);

        // Use the displayed message token count as an initial estimate so the ctx
        // indicator shows something immediately on load instead of "–".
        // display_token_count only covers non-compacted (visible) messages, making it
        // a reasonable baseline. The real value from the API will replace it on the
        // next response.
        self.last_input_tokens = if self.display_token_count > 0 {
            Some(self.display_token_count as u32)
        } else {
            None
        };

        // Clear unread indicator for this session
        self.sessions_with_unread.remove(&session_id);

        // Auto-restore provider if session has a different one than current
        if let Some(ref saved_provider) = session.provider_name {
            let current_provider = self.agent_service.provider_name();
            if *saved_provider != current_provider {
                // Try cache first
                if let Some(cached) = self.provider_cache.get(saved_provider) {
                    self.agent_service.swap_provider(cached.clone());
                    tracing::info!(
                        "Restored provider '{}' from cache for session {}",
                        saved_provider,
                        session_id
                    );
                } else {
                    // Cache miss — create from config
                    match crate::config::Config::load() {
                        Ok(config) => {
                            match crate::brain::provider::factory::create_provider_by_name(
                                &config,
                                saved_provider,
                            ) {
                                Ok(provider) => {
                                    self.provider_cache
                                        .insert(saved_provider.clone(), provider.clone());
                                    self.agent_service.swap_provider(provider);
                                    tracing::info!(
                                        "Created and cached provider '{}' for session {}",
                                        saved_provider,
                                        session_id
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to restore provider '{}': {}. Using current.",
                                        saved_provider,
                                        e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load config for provider restore: {}", e);
                        }
                    }
                }
                // Update display model name from restored provider
                self.default_model_name = session
                    .model
                    .clone()
                    .unwrap_or_else(|| self.agent_service.provider_model());
                self.context_max_tokens = self
                    .agent_service
                    .context_window_for_model(&self.default_model_name);
            }
        }

        Ok(())
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
        use crate::db::repository::SessionListOptions;

        self.sessions = self
            .session_service
            .list_sessions(SessionListOptions {
                include_archived: false,
                limit: Some(100),
                offset: 0,
            })
            .await?;

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
                self.mode = AppMode::UsageDialog;
                true
            }
            "/onboard" => {
                let config = crate::config::Config::load().unwrap_or_default();
                self.onboarding = Some(OnboardingWizard::from_config(&config));
                self.mode = AppMode::Onboarding;
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
                                        let _ = tx.send(TuiEvent::SystemMessage(line));
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
    pub fn format_tool_description(tool_name: &str, tool_input: &Value) -> String {
        match tool_name {
            "bash" => {
                let cmd = tool_input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short: String = cmd.chars().take(80).collect();
                if cmd.len() > 80 {
                    format!("bash: {}...", short)
                } else {
                    format!("bash: {}", short)
                }
            }
            "read_file" | "read" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Read {}", path)
            }
            "write_file" | "write" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Write {}", path)
            }
            "edit_file" | "edit" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Edit {}", path)
            }
            "ls" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                format!("ls {}", path)
            }
            "glob" => {
                let pattern = tool_input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Glob {}", pattern)
            }
            "grep" => {
                let pattern = tool_input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if path.is_empty() {
                    format!("Grep '{}'", pattern)
                } else {
                    format!("Grep '{}' in {}", pattern, path)
                }
            }
            "web_search" => {
                let query = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Search: {}", query)
            }
            "exa_search" => {
                let query = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("EXA search: {}", query)
            }
            "brave_search" => {
                let query = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Brave search: {}", query)
            }
            "http_request" => {
                let url = tool_input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let method = tool_input
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET");
                format!("{} {}", method, url)
            }
            "execute_code" => {
                let lang = tool_input
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Execute {}", lang)
            }
            "notebook_edit" => {
                let path = tool_input
                    .get("notebook_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Notebook {}", path)
            }
            "parse_document" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Parse {}", path)
            }
            "task_manager" => {
                let op = tool_input
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Task: {}", op)
            }
            "plan" => {
                let op = tool_input
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Plan: {}", op)
            }
            "session_context" => "Session context".to_string(),
            other => other.to_string(),
        }
    }

    /// Expand a DB message into one or more DisplayMessages.
    /// Assistant messages may contain tool markers that get reconstructed into ToolCallGroup display messages.
    /// Supports both v1 (`<!-- tools: desc1 | desc2 -->`) and v2 (`<!-- tools-v2: [JSON] -->`) formats.
    /// Extract `<!-- reasoning -->...<!-- /reasoning -->` blocks from text.
    /// Returns (reasoning_text, remaining_text_without_markers).
    fn extract_reasoning(text: &str) -> (Option<String>, String) {
        let open_tag = "<!-- reasoning -->";
        let close_tag = "<!-- /reasoning -->";
        let mut reasoning_parts = Vec::new();
        let mut remaining = String::new();
        let mut rest = text;

        loop {
            if let Some(start) = rest.find(open_tag) {
                remaining.push_str(&rest[..start]);
                let after_tag = &rest[start + open_tag.len()..];
                if let Some(end) = after_tag.find(close_tag) {
                    let part = after_tag[..end].trim();
                    if !part.is_empty() {
                        reasoning_parts.push(part.to_string());
                    }
                    rest = &after_tag[end + close_tag.len()..];
                } else {
                    // Unclosed tag — treat rest as reasoning
                    let part = after_tag.trim();
                    if !part.is_empty() {
                        reasoning_parts.push(part.to_string());
                    }
                    break;
                }
            } else {
                remaining.push_str(rest);
                break;
            }
        }

        let remaining = remaining.trim().to_string();
        if reasoning_parts.is_empty() {
            (None, remaining)
        } else {
            (Some(reasoning_parts.join("\n\n")), remaining)
        }
    }

    fn expand_message(msg: crate::db::models::Message) -> Vec<DisplayMessage> {
        if msg.role != "assistant"
            || (!msg.content.contains("<!-- tools") && !msg.content.contains("<!-- reasoning -->"))
        {
            return vec![DisplayMessage::from(msg)];
        }

        // Extract owned values before borrowing content
        let id = msg.id;
        let timestamp = msg.created_at;
        let token_count = msg.token_count;
        let cost = msg.cost;
        let content = msg.content;

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
            if let Some(end) = after_marker.find("-->") {
                let tools_str = after_marker[..end].trim();

                let calls: Vec<ToolCallEntry> = if is_v2 {
                    // v2: parse JSON array with descriptions, success, and output
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
                            ToolCallEntry {
                                description: desc,
                                success,
                                details: output,
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
                        })
                        .collect()
                };

                if !calls.is_empty() {
                    let count = calls.len();
                    result.push(DisplayMessage {
                        id: Uuid::new_v4(),
                        role: "tool_group".to_string(),
                        content: format!(
                            "{} tool call{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        ),
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
                remaining = &after_marker[end + 3..];
            } else {
                remaining = after_marker;
                break;
            }
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

        result
    }

    /// Extract image file paths from text and return (remaining_text, attachments).
    /// Handles paths with spaces (e.g. `/home/user/My Screenshots/photo.png`)
    /// and image URLs.
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

        // Case 2: Mixed text — scan for image URLs (split by whitespace is fine for URLs)
        // and absolute paths without spaces
        let mut attachments = Vec::new();
        let mut remaining_parts = Vec::new();

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
            remaining_parts.push(word);
        }

        (remaining_parts.join(" "), attachments)
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
            // DON'T add to messages yet - wait until agent processes it
            // It will be added at the end after all assistant messages

            // Queue for injection between tool calls
            *self.message_queue.lock().await = Some(content);
            return Ok(());
        }
        if let Some(session) = &self.current_session {
            self.processing_sessions.insert(session.id);
            self.is_processing = true;
            self.processing_started_at = Some(std::time::Instant::now());
            self.error_message = None;
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

            // Add user message to UI — replace <<IMG:...>> markers with readable names
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
            let read_only_mode = false;

            tracing::info!(
                "[send_message] Spawning agent task for session {}",
                session_id
            );
            let panic_sender = event_sender.clone();
            let handle = tokio::spawn(async move {
                tracing::info!("[agent_task] START calling send_message_with_tools_and_mode");
                let result = agent_service
                    .send_message_with_tools_and_mode(
                        session_id,
                        transformed_content,
                        None,
                        read_only_mode,
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
                        if let Err(e2) = event_sender.send(TuiEvent::Error {
                            session_id,
                            message: e.to_string(),
                        }) {
                            tracing::error!("[agent_task] FAILED to send Error event: {}", e2);
                        }
                    }
                }
            });
            // Watch for panics — surface them in the UI instead of silent hang
            tokio::spawn(async move {
                if let Err(e) = handle.await {
                    tracing::error!("[agent_task] PANICKED: {}", e);
                    let _ = panic_sender.send(TuiEvent::Error {
                        session_id,
                        message: format!(
                            "Agent task crashed unexpectedly: {e}. You can continue chatting."
                        ),
                    });
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
        let reasoning_details = self.streaming_reasoning.take();
        self.cancel_token = None;
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

        // Finalize any remaining queued message at the END (if agent didn't process it via IntermediateText)
        if let Some(queued_content) = self.message_queue.lock().await.take() {
            let queued_msg = DisplayMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: Self::humanize_image_markers(&queued_content),
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: Some("queued".to_string()),
                expanded: false,
                tool_group: None,
            };
            self.messages.push(queued_msg);
            tracing::info!("[TUI] Added queued message at response complete");
        }

        // Finalize active tool group as a standalone message BEFORE the response.
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

        // Reload user commands (agent may have written new ones to commands.json)
        self.reload_user_commands();

        // Track context usage from latest response
        self.last_input_tokens = Some(response.context_tokens);

        // Debug: log response content length
        tracing::debug!(
            "Response complete: content_len={}, output_tokens={}",
            response.content.len(),
            response.usage.output_tokens
        );

        // Check if we already added assistant messages via IntermediateText this cycle.
        // Uses a per-cycle flag (not a history search) so prior turns don't cause false positives.
        if self.intermediate_text_received {
            tracing::debug!(
                "Skipping duplicate assistant message - already shown via IntermediateText"
            );
        } else {
            // Add assistant message to UI only if not already added
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

        // Auto-scroll to bottom
        self.scroll_offset = 0;

        Ok(())
    }
}
